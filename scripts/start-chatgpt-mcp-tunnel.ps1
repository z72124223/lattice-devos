[CmdletBinding()]
param(
    [ValidateSet('Init', 'Doctor', 'Run', 'ManagedRun')]
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

$nativeIdentityHelperPath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot 'windows-native-path-identity.ps1'))
$nativeIdentityHelperItem = Get-Item -LiteralPath $nativeIdentityHelperPath -Force -ErrorAction SilentlyContinue
if (
    $null -eq $nativeIdentityHelperItem -or
    $nativeIdentityHelperItem.PSIsContainer -or
    -not ($nativeIdentityHelperItem -is [IO.FileInfo]) -or
    ($nativeIdentityHelperItem.Attributes -band [IO.FileAttributes]::ReparsePoint)
) {
    throw 'TASK038_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}
try {
    $nativeIdentityHelperBytes = [IO.File]::ReadAllBytes($nativeIdentityHelperPath)
    if (
        $nativeIdentityHelperBytes.Length -ge 3 -and
        $nativeIdentityHelperBytes[0] -eq 0xef -and
        $nativeIdentityHelperBytes[1] -eq 0xbb -and
        $nativeIdentityHelperBytes[2] -eq 0xbf
    ) {
        throw 'TASK038_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
    }
    $nativeIdentityHelperSource = [Text.UTF8Encoding]::new($false, $true).GetString(
        $nativeIdentityHelperBytes
    )
    . ([scriptblock]::Create($nativeIdentityHelperSource))
    Initialize-LatticeWindowsNativePathIdentity
}
catch {
    throw 'TASK038_WINDOWS_NATIVE_IDENTITY_HELPER_REJECTED'
}

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
    Assert-NoReparsePath -Path $resolved -FailureCode $FailureCode
    return $resolved
}

function Get-CanonicalPath {
    param([Parameter(Mandatory = $true)][string]$Path)

    return [IO.Path]::GetFullPath($Path).TrimEnd([char[]]@(
        [IO.Path]::DirectorySeparatorChar,
        [IO.Path]::AltDirectorySeparatorChar
    ))
}

function Assert-NoReparsePath {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    $current = Get-CanonicalPath -Path $Path
    while (-not [string]::IsNullOrWhiteSpace($current)) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force -ErrorAction Stop
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
        [Parameter(Mandatory = $true)][string]$TunnelClient,
        [switch]$PassThru
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
        $profileNativeIdentity = Get-LatticeWindowsNativePathIdentityToken `
            -Path $profilePath `
            -Directory $false
        $tunnelClientNativeIdentity = Get-LatticeWindowsNativePathIdentityToken `
            -Path $TunnelClient `
            -Directory $false
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
    try {
        $latticedNativeIdentity = Get-LatticeWindowsNativePathIdentityToken `
            -Path $latticed `
            -Directory $false
    }
    catch {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $tunnelClientSha256 = Get-FileSha256 `
        -Path $TunnelClient `
        -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED'
    $latticedSha256 = Get-FileSha256 `
        -Path $latticed `
        -FailureCode 'TASK038_TUNNEL_PROFILE_REJECTED'
    $profileSha256 = Get-ByteArraySha256 -Bytes $profileBytes
    if (
        -not (Test-LatticeWindowsNativePathIdentity -Path $profilePath -Directory $false -ExpectedToken $profileNativeIdentity) -or
        -not (Test-LatticeWindowsNativePathIdentity -Path $TunnelClient -Directory $false -ExpectedToken $tunnelClientNativeIdentity) -or
        -not (Test-LatticeWindowsNativePathIdentity -Path $latticed -Directory $false -ExpectedToken $latticedNativeIdentity)
    ) {
        throw 'TASK038_TUNNEL_PROFILE_REJECTED'
    }
    $commitment = @(
        'lattice.task-ingress-profile.v2',
        ('profile_name=' + $ProfileName),
        ('profile_sha256=' + $profileSha256),
        ('tunnel_id=' + $tunnelMatch.Groups['value'].Value),
        'channel=main',
        ('tunnel_client_sha256=' + $tunnelClientSha256),
        ('latticed_sha256=' + $latticedSha256)
    ) -join "`n"
    $digest = Get-StringSha256 -Value $commitment
    if ($PassThru) {
        return [pscustomobject][ordered]@{
            schema = 'lattice.task038.profile-provenance.v1'
            digest = $digest
            profile_path = $profilePath
            profile_raw_sha256 = $profileSha256
            profile_byte_count = [long]$profileBytes.Length
            profile_strict_utf8 = $true
            profile_native_identity = $profileNativeIdentity
            tunnel_id = $tunnelMatch.Groups['value'].Value
            latticed_executable = $latticed
            latticed_sha256 = $latticedSha256
            latticed_native_identity = $latticedNativeIdentity
            tunnel_client_sha256 = $tunnelClientSha256
            tunnel_client_native_identity = $tunnelClientNativeIdentity
        }
    }
    return $digest
}

function Get-Task038HmacSha256 {
    param(
        [Parameter(Mandatory = $true)][byte[]]$Key,
        [Parameter(Mandatory = $true)][string]$Value
    )

    $algorithm = [Security.Cryptography.HMACSHA256]::new($Key)
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Value)
        return ([BitConverter]::ToString($algorithm.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
    }
}

function New-Task038PrivateNonce {
    $bytes = [byte[]]::new(32)
    $algorithm = [Security.Cryptography.RandomNumberGenerator]::Create()
    try { $algorithm.GetBytes($bytes) }
    finally { $algorithm.Dispose() }
    return $bytes
}

function ConvertTo-Task038WindowsCommandLineArgument {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$Value)

    if ($Value.Length -eq 0) { return '""' }
    if ($Value.IndexOfAny([char[]]@(' ', "`t", '"')) -lt 0) { return $Value }
    $escaped = [regex]::Replace($Value, '(\\*)"', '$1$1\"')
    $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
    return '"' + $escaped + '"'
}

function Initialize-Task038TunnelOwnedProcessType {
    if ($null -ne ('Lattice.Task038.TunnelOwnedProcess' -as [type])) { return }

    Add-Type -Language CSharp -ErrorAction Stop -TypeDefinition @'
using System;
using System.Collections;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

namespace Lattice.Task038
{
    public sealed class TunnelOwnedProcess : IDisposable
    {
        private const UInt32 CREATE_SUSPENDED = 0x00000004;
        private const UInt32 CREATE_NO_WINDOW = 0x08000000;
        private const UInt32 CREATE_UNICODE_ENVIRONMENT = 0x00000400;
        private const UInt32 JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
        private const UInt32 JOB_OBJECT_EXTENDED_LIMIT_INFORMATION = 9;
        private const UInt32 WAIT_OBJECT_0 = 0;
        private const UInt32 WAIT_FAILED = 0xffffffff;
        private const UInt32 INFINITE = 0xffffffff;
        private IntPtr jobHandle;
        private IntPtr rootProcessHandle;
        private Int32 rootProcessId;
        private UInt64 rootProcessCreationFileTime;
        private string rootProcessImagePath;
        private bool closed;

        [StructLayout(LayoutKind.Sequential)]
        private struct FILETIME
        {
            public UInt32 Low;
            public UInt32 High;
            public UInt64 Value { get { return ((UInt64)High << 32) | Low; } }
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct STARTUPINFO
        {
            public UInt32 cb;
            public IntPtr lpReserved;
            public IntPtr lpDesktop;
            public IntPtr lpTitle;
            public UInt32 dwX;
            public UInt32 dwY;
            public UInt32 dwXSize;
            public UInt32 dwYSize;
            public UInt32 dwXCountChars;
            public UInt32 dwYCountChars;
            public UInt32 dwFillAttribute;
            public UInt32 dwFlags;
            public UInt16 wShowWindow;
            public UInt16 cbReserved2;
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
            public UInt32 dwProcessId;
            public UInt32 dwThreadId;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_LIMIT_INFORMATION
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
        private struct IO_COUNTERS
        {
            public UInt64 ReadOperationCount;
            public UInt64 WriteOperationCount;
            public UInt64 OtherOperationCount;
            public UInt64 ReadTransferCount;
            public UInt64 WriteTransferCount;
            public UInt64 OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
            public IO_COUNTERS IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
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
        private static extern bool SetInformationJobObject(
            IntPtr job,
            UInt32 informationClass,
            ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION information,
            UInt32 informationLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool QueryInformationJobObject(
            IntPtr job,
            UInt32 informationClass,
            out JOBOBJECT_BASIC_ACCOUNTING_INFORMATION information,
            UInt32 informationLength,
            IntPtr returnLength);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool CreateProcessW(
            string applicationName,
            StringBuilder commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            bool inheritHandles,
            UInt32 creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref STARTUPINFO startupInfo,
            out PROCESS_INFORMATION processInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern UInt32 ResumeThread(IntPtr thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateProcess(IntPtr process, UInt32 exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool TerminateJobObject(IntPtr job, UInt32 exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool CloseHandle(IntPtr handle);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern UInt32 WaitForSingleObject(IntPtr handle, UInt32 milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetExitCodeProcess(IntPtr process, out UInt32 exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern bool GetProcessTimes(
            IntPtr process,
            out FILETIME creation,
            out FILETIME exit,
            out FILETIME kernel,
            out FILETIME user);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool QueryFullProcessImageName(
            IntPtr process,
            UInt32 flags,
            StringBuilder imagePath,
            ref UInt32 size);

        private TunnelOwnedProcess(
            IntPtr job,
            IntPtr process,
            Int32 processId,
            UInt64 creationFileTime,
            string imagePath)
        {
            jobHandle = job;
            rootProcessHandle = process;
            rootProcessId = processId;
            rootProcessCreationFileTime = creationFileTime;
            rootProcessImagePath = imagePath;
        }

        public Int32 ProcessId { get { return rootProcessId; } }
        public string CreationFileTime { get { return rootProcessCreationFileTime.ToString(); } }
        public string ImagePath { get { return rootProcessImagePath; } }

        public Int32 WaitForExitAndGetCode()
        {
            UInt32 waitResult = WaitForSingleObject(rootProcessHandle, INFINITE);
            if (waitResult == WAIT_FAILED)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_CLIENT_WAIT_REJECTED");
            if (waitResult != WAIT_OBJECT_0)
                throw new InvalidOperationException("TASK038_TUNNEL_CLIENT_WAIT_REJECTED");
            UInt32 exitCode;
            if (!GetExitCodeProcess(rootProcessHandle, out exitCode))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_CLIENT_EXIT_CODE_REJECTED");
            return unchecked((Int32)exitCode);
        }

        private static IntPtr BuildEnvironmentBlock(StringDictionary environment)
        {
            var entries = new List<string>();
            foreach (DictionaryEntry entry in environment)
            {
                string name = Convert.ToString(entry.Key);
                string value = Convert.ToString(entry.Value);
                if (String.IsNullOrEmpty(name) || name.IndexOf('=') >= 0 ||
                    name.IndexOf('\0') >= 0 || value.IndexOf('\0') >= 0)
                {
                    throw new InvalidOperationException("TASK038_TUNNEL_CHILD_ENVIRONMENT_REJECTED");
                }
                entries.Add(name + "=" + value);
            }
            entries.Sort(StringComparer.OrdinalIgnoreCase);
            return Marshal.StringToHGlobalUni(String.Join("\0", entries.ToArray()) + "\0\0");
        }

        public static TunnelOwnedProcess Start(
            string executable,
            string arguments,
            string workingDirectory,
            StringDictionary environment)
        {
            if (String.IsNullOrWhiteSpace(executable) || executable.IndexOf('"') >= 0 ||
                String.IsNullOrWhiteSpace(workingDirectory) || environment == null)
            {
                throw new ArgumentException("TASK038_TUNNEL_CLIENT_START_REJECTED");
            }
            IntPtr job = IntPtr.Zero;
            IntPtr environmentBlock = IntPtr.Zero;
            var created = new PROCESS_INFORMATION();
            bool processCreated = false;
            try
            {
                job = CreateJobObject(IntPtr.Zero, null);
                if (job == IntPtr.Zero)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_JOB_CREATE_REJECTED");
                var limits = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if (!SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    ref limits,
                    (UInt32)Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION))))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_JOB_LIMIT_REJECTED");
                }
                environmentBlock = BuildEnvironmentBlock(environment);
                var startup = new STARTUPINFO { cb = (UInt32)Marshal.SizeOf(typeof(STARTUPINFO)) };
                var commandLine = new StringBuilder("\"" + executable + "\" " + (arguments ?? String.Empty));
                if (!CreateProcessW(
                    executable,
                    commandLine,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    false,
                    CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                    environmentBlock,
                    workingDirectory,
                    ref startup,
                    out created))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_CLIENT_START_REJECTED");
                }
                processCreated = true;
                if (!AssignProcessToJobObject(job, created.hProcess))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_JOB_ASSIGN_REJECTED");
                FILETIME creation;
                FILETIME exit;
                FILETIME kernel;
                FILETIME user;
                if (!GetProcessTimes(created.hProcess, out creation, out exit, out kernel, out user))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_PROCESS_IDENTITY_REJECTED");
                UInt32 imagePathCapacity = 32768;
                var imagePath = new StringBuilder((Int32)imagePathCapacity);
                if (!QueryFullProcessImageName(created.hProcess, 0, imagePath, ref imagePathCapacity))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_PROCESS_IDENTITY_REJECTED");
                if (ResumeThread(created.hThread) == UInt32.MaxValue)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_CLIENT_RESUME_REJECTED");
                var result = new TunnelOwnedProcess(
                    job,
                    created.hProcess,
                    (Int32)created.dwProcessId,
                    creation.Value,
                    imagePath.ToString());
                job = IntPtr.Zero;
                created.hProcess = IntPtr.Zero;
                return result;
            }
            catch
            {
                if (processCreated && created.hProcess != IntPtr.Zero) TerminateProcess(created.hProcess, 1);
                throw;
            }
            finally
            {
                if (environmentBlock != IntPtr.Zero) Marshal.FreeHGlobal(environmentBlock);
                if (created.hThread != IntPtr.Zero) CloseHandle(created.hThread);
                if (created.hProcess != IntPtr.Zero) CloseHandle(created.hProcess);
                if (job != IntPtr.Zero) CloseHandle(job);
            }
        }

        public UInt32 ActiveProcessCount()
        {
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION information;
            if (jobHandle == IntPtr.Zero || !QueryInformationJobObject(
                jobHandle,
                1,
                out information,
                (UInt32)Marshal.SizeOf(typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION)),
                IntPtr.Zero))
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "TASK038_TUNNEL_JOB_QUERY_REJECTED");
            }
            return information.ActiveProcesses;
        }

        public bool TerminateAndWait(Int32 milliseconds)
        {
            if (closed) return true;
            if (ActiveProcessCount() > 0 && !TerminateJobObject(jobHandle, 1)) return false;
            var stopwatch = Stopwatch.StartNew();
            while (stopwatch.ElapsedMilliseconds < milliseconds)
            {
                if (ActiveProcessCount() == 0)
                {
                    CloseHandle(jobHandle);
                    jobHandle = IntPtr.Zero;
                    closed = true;
                    return true;
                }
                System.Threading.Thread.Sleep(10);
            }
            return false;
        }

        public void Dispose()
        {
            try { if (!closed) TerminateAndWait(15000); }
            finally
            {
                if (jobHandle != IntPtr.Zero) CloseHandle(jobHandle);
                jobHandle = IntPtr.Zero;
                if (rootProcessHandle != IntPtr.Zero) CloseHandle(rootProcessHandle);
                rootProcessHandle = IntPtr.Zero;
                closed = true;
            }
        }
    }
}
'@
}

function New-Task038TunnelChildStartInfo {
    param(
        [Parameter(Mandatory = $true)][Collections.IDictionary]$EnvironmentValues
    )

    $safeNames = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    @(
        'ALLUSERSPROFILE', 'APPDATA', 'CommonProgramFiles', 'CommonProgramFiles(x86)',
        'CommonProgramW6432', 'ComSpec', 'DriverData', 'HOMEDRIVE', 'HOMEPATH',
        'LOCALAPPDATA', 'NUMBER_OF_PROCESSORS', 'OS', 'Path', 'PATHEXT',
        'PROCESSOR_ARCHITECTURE', 'PROCESSOR_IDENTIFIER', 'PROCESSOR_LEVEL',
        'PROCESSOR_REVISION', 'ProgramData', 'ProgramFiles', 'ProgramFiles(x86)',
        'ProgramW6432', 'SystemDrive', 'SystemRoot', 'TEMP', 'TMP', 'USERDOMAIN',
        'USERDOMAIN_ROAMINGPROFILE', 'USERNAME', 'USERPROFILE', 'windir'
    ) | ForEach-Object { [void]$safeNames.Add($_) }

    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.UseShellExecute = $false
    $startInfo.EnvironmentVariables.Clear()
    foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
        $name = [string]$entry.Key
        if ($safeNames.Contains($name)) {
            $startInfo.EnvironmentVariables[$name] = [string]$entry.Value
        }
    }
    foreach ($entry in $EnvironmentValues.GetEnumerator()) {
        $name = [string]$entry.Key
        $value = [string]$entry.Value
        if (
            $name -cnotmatch '^[A-Z][A-Z0-9_]{0,127}$' -or
            [string]::IsNullOrWhiteSpace($value) -or
            $value.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0
        ) {
            throw 'TASK038_TUNNEL_CHILD_ENVIRONMENT_REJECTED'
        }
        $startInfo.EnvironmentVariables[$name] = $value
    }
    $startInfo.EnvironmentVariables['NO_COLOR'] = '1'
    return $startInfo
}

function Invoke-Task038OwnedTunnelClient {
    param(
        [Parameter(Mandatory = $true)][string]$TunnelClient,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][Diagnostics.ProcessStartInfo]$StartInfo,
        $AuthoritySink
    )

    if ([IO.Path]::GetExtension($TunnelClient) -cne '.exe') {
        throw 'TASK038_TUNNEL_CLIENT_EXECUTABLE_TYPE_REJECTED'
    }
    Initialize-Task038TunnelOwnedProcessType
    $commandLineArguments = @($Arguments | ForEach-Object {
        ConvertTo-Task038WindowsCommandLineArgument -Value ([string]$_)
    }) -join ' '
    $ownedTree = $null
    $exitCode = $null
    $processId = $null
    $processCreationTime = $null
    $processImagePath = $null
    $startedAt = [DateTime]::UtcNow
    try {
        $ownedTree = [Lattice.Task038.TunnelOwnedProcess]::Start(
            $TunnelClient,
            $commandLineArguments,
            $WorkingDirectory,
            $StartInfo.EnvironmentVariables
        )
        $processId = [int]$ownedTree.ProcessId
        $processCreationTime = [string]$ownedTree.CreationFileTime
        $processImagePath = Get-CanonicalPath -Path ([string]$ownedTree.ImagePath)
        if ($null -ne $AuthoritySink) {
            if (
                $processCreationTime -cnotmatch '\A[0-9]{1,32}\z' -or
                $processImagePath -cne [string]$AuthoritySink.tunnel_client_path -or
                (Get-FileSha256 -Path $processImagePath -FailureCode 'TASK038_TUNNEL_PROCESS_IDENTITY_REJECTED') -cne [string]$AuthoritySink.tunnel_client_sha256 -or
                (Get-LatticeWindowsNativePathIdentityToken -Path $processImagePath -Directory $false) -cne [string]$AuthoritySink.tunnel_client_native_identity
            ) { throw 'TASK038_TUNNEL_PROCESS_IDENTITY_REJECTED' }
            Add-Task038LaunchAuthorityEvent -Sink $AuthoritySink -EventType 'PROCESS_SPAWN_BOUND' -Payload ([ordered]@{
                process_id = [long]$processId
                process_creation_time = $processCreationTime
                process_creation_time_source = 'WINDOWS_PROCESS_TIMES'
                process_executable_path = $processImagePath
                process_executable_sha256 = [string]$AuthoritySink.tunnel_client_sha256
                process_executable_native_identity = [string]$AuthoritySink.tunnel_client_native_identity
                create_suspended = $true
                job_assigned_before_resume = $true
                job_owner_process_id = [long]$PID
            })
        }
        $exitCode = [int]$ownedTree.WaitForExitAndGetCode()
    }
    finally {
        if ($null -ne $ownedTree) {
            try {
                if (-not $ownedTree.TerminateAndWait(15000)) {
                    throw 'TASK038_TUNNEL_PROCESS_TREE_NOT_REAPED'
                }
                if ($null -ne $AuthoritySink -and $null -ne $exitCode) {
                    Add-Task038LaunchAuthorityEvent -Sink $AuthoritySink -EventType 'PROCESS_REAPED' -Payload ([ordered]@{
                        process_id = [long]$processId
                        process_creation_time = [string]$processCreationTime
                        process_executable_sha256 = [string]$AuthoritySink.tunnel_client_sha256
                        exit_code = [int]$exitCode
                        job_active_process_count = 0L
                        descendant_processes_after_cleanup = 0L
                    })
                }
            }
            finally {
                $ownedTree.Dispose()
            }
        }
    }
    return [pscustomobject][ordered]@{
        exit_code = $exitCode
        process_id = $processId
        process_creation_time = $processCreationTime
        process_creation_time_source = 'WINDOWS_PROCESS_TIMES'
        process_executable_path = $processImagePath
        started_at_utc = $startedAt.ToString('o')
        exited_at_utc = [DateTime]::UtcNow.ToString('o')
        create_suspended = $true
        job_assigned_before_resume = $true
        descendant_processes_after_cleanup = 0
    }
}

function Get-Task038TunnelRuntimeEnvironment {
    $requiredNames = @(
        'CONTROL_PLANE_API_KEY',
        'LATTICE_FULL_CHAIN_RUN_MODE',
        'LATTICE_DELIVERY_CODEX_MODE',
        'LATTICE_DELIVERY_TIMEOUT_SECONDS',
        'LATTICE_TASK019_HOST',
        'LATTICE_TASK019_PORT',
        'LATTICE_TASK019_RUN_ID',
        'LATTICE_TASK019_PASSWORD',
        'LATTICE_STORE_DAEMON_INSTANCE_ID',
        'LATTICE_STORE_DAEMON_EPOCH',
        'LATTICE_STORE_AUTHORITY_REVISION',
        'LATTICE_STORE_OBSERVATION_DIGEST',
        'LATTICE_STORE_AUTHORITY_HEAD_DIGEST',
        'LATTICE_DELIVERY_LAUNCHER',
        'LATTICE_DELIVERY_LAUNCHER_VERSION',
        'LATTICE_DELIVERY_LAUNCHER_SHA256',
        'LATTICE_DELIVERY_SCHEMA_DIR',
        'LATTICE_DELIVERY_CODEX_HOME',
        'LATTICE_DELIVERY_ROOT',
        'LATTICE_DELIVERY_GIT_EXE'
    )
    $values = [ordered]@{}
    foreach ($name in $requiredNames) {
        $value = [Environment]::GetEnvironmentVariable($name, 'Process')
        if (
            [string]::IsNullOrWhiteSpace($value) -or
            $value.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0
        ) {
            throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        }
        $values[$name] = $value
    }

    $port = 0
    $timeout = 0
    $daemonEpoch = 0L
    $authorityRevision = 0L
    if (
        $values.CONTROL_PLANE_API_KEY.Length -lt 20 -or
        $values.LATTICE_FULL_CHAIN_RUN_MODE -cne 'FRESH' -or
        $values.LATTICE_DELIVERY_CODEX_MODE -cne 'OFFICIAL_CODEX_APP_SERVER' -or
        -not [int]::TryParse($values.LATTICE_DELIVERY_TIMEOUT_SECONDS, [ref]$timeout) -or
        $timeout -lt 60 -or $timeout -gt 300 -or
        $values.LATTICE_TASK019_HOST -cne '127.0.0.1' -or
        -not [int]::TryParse($values.LATTICE_TASK019_PORT, [ref]$port) -or
        $port -lt 1 -or $port -gt 65535 -or $port -in @(5432, 64272, 55432) -or
        $values.LATTICE_TASK019_RUN_ID -cnotmatch '\A[0-9a-f]{32}\z' -or
        $values.LATTICE_TASK019_PASSWORD.Length -lt 16 -or
        $values.LATTICE_STORE_DAEMON_INSTANCE_ID -cnotmatch '\Atask038-(?:local|tunnel)-[0-9a-f]{32}\z' -or
        -not [long]::TryParse($values.LATTICE_STORE_DAEMON_EPOCH, [ref]$daemonEpoch) -or
        -not [long]::TryParse($values.LATTICE_STORE_AUTHORITY_REVISION, [ref]$authorityRevision) -or
        $daemonEpoch -lt 1 -or $authorityRevision -lt 1 -or
        $values.LATTICE_STORE_OBSERVATION_DIGEST -cnotmatch '\A[0-9a-f]{64}\z' -or
        $values.LATTICE_STORE_AUTHORITY_HEAD_DIGEST -cnotmatch '\A[0-9a-f]{64}\z' -or
        $values.LATTICE_DELIVERY_LAUNCHER_SHA256 -cnotmatch '\A[0-9a-f]{64}\z'
    ) {
        throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }
    if (
        [string]$values.LATTICE_DELIVERY_TIMEOUT_SECONDS -cne [string]$timeout -or
        [string]$values.LATTICE_TASK019_PORT -cne [string]$port -or
        [string]$values.LATTICE_STORE_DAEMON_EPOCH -cne [string]$daemonEpoch -or
        [string]$values.LATTICE_STORE_AUTHORITY_REVISION -cne [string]$authorityRevision
    ) {
        throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }

    $resolvedRuntimeLeafPaths = [ordered]@{}
    foreach ($name in @('LATTICE_DELIVERY_LAUNCHER', 'LATTICE_DELIVERY_GIT_EXE')) {
        $resolvedRuntimeLeafPaths[$name] = Resolve-RequiredLeafPath `
            -Path ([string]$values[$name]) `
            -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }
    foreach ($name in $resolvedRuntimeLeafPaths.Keys) {
        $values[$name] = [string]$resolvedRuntimeLeafPaths[$name]
    }
    $launcherSha256 = Get-FileSha256 `
        -Path ([string]$resolvedRuntimeLeafPaths.LATTICE_DELIVERY_LAUNCHER) `
        -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    if ($launcherSha256 -cne [string]$values.LATTICE_DELIVERY_LAUNCHER_SHA256) {
        throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }
    foreach ($name in @('LATTICE_DELIVERY_SCHEMA_DIR', 'LATTICE_DELIVERY_ROOT')) {
        if (-not [IO.Path]::IsPathRooted([string]$values[$name])) {
            throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        }
        $path = Get-CanonicalPath -Path ([string]$values[$name])
        $parent = Get-Item -LiteralPath (Split-Path -Parent $path) -Force -ErrorAction SilentlyContinue
        if (
            $null -eq $parent -or
            -not $parent.PSIsContainer -or
            ($parent.Attributes -band [IO.FileAttributes]::ReparsePoint)
        ) {
            throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        }
        Assert-NoReparsePath -Path $path -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
        $values[$name] = $path
    }
    $codexHome = Get-Item -LiteralPath ([string]$values.LATTICE_DELIVERY_CODEX_HOME) -Force -ErrorAction SilentlyContinue
    if (
        $null -eq $codexHome -or
        -not $codexHome.PSIsContainer -or
        ($codexHome.Attributes -band [IO.FileAttributes]::ReparsePoint)
    ) {
        throw 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    }
    Assert-NoReparsePath `
        -Path ([string]$values.LATTICE_DELIVERY_CODEX_HOME) `
        -FailureCode 'TASK038_TUNNEL_RUNTIME_ENVIRONMENT_REJECTED'
    $values.LATTICE_DELIVERY_CODEX_HOME = Get-CanonicalPath -Path ([string]$values.LATTICE_DELIVERY_CODEX_HOME)
    return $values
}

function Get-Task038SafeConfigProjection {
    param(
        [Parameter(Mandatory = $true)][Collections.IDictionary]$RuntimeEnvironment,
        [Parameter(Mandatory = $true)][string]$IngressProfileSha256
    )

    if ($IngressProfileSha256 -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'TASK038_TUNNEL_SAFE_CONFIG_REJECTED'
    }
    $safeNames = @(
        'LATTICE_FULL_CHAIN_RUN_MODE',
        'LATTICE_DELIVERY_CODEX_MODE',
        'LATTICE_DELIVERY_TIMEOUT_SECONDS',
        'LATTICE_TASK019_HOST',
        'LATTICE_TASK019_PORT',
        'LATTICE_TASK019_RUN_ID',
        'LATTICE_STORE_DAEMON_INSTANCE_ID',
        'LATTICE_STORE_DAEMON_EPOCH',
        'LATTICE_STORE_AUTHORITY_REVISION',
        'LATTICE_STORE_OBSERVATION_DIGEST',
        'LATTICE_STORE_AUTHORITY_HEAD_DIGEST',
        'LATTICE_DELIVERY_LAUNCHER',
        'LATTICE_DELIVERY_LAUNCHER_VERSION',
        'LATTICE_DELIVERY_LAUNCHER_SHA256',
        'LATTICE_DELIVERY_SCHEMA_DIR',
        'LATTICE_DELIVERY_CODEX_HOME',
        'LATTICE_DELIVERY_ROOT',
        'LATTICE_DELIVERY_GIT_EXE'
    )
    $projection = [Collections.Generic.List[string]]::new()
    [void]$projection.Add('lattice.task038.tunnel-safe-config.v1')
    foreach ($name in $safeNames) {
        $value = [string]$RuntimeEnvironment[$name]
        if (
            [string]::IsNullOrWhiteSpace($value) -or
            $value.IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0
        ) {
            throw 'TASK038_TUNNEL_SAFE_CONFIG_REJECTED'
        }
        [void]$projection.Add($name + '=' + $value)
    }
    [void]$projection.Add('LATTICE_TASK_INGRESS_KIND=CHATGPT_SECURE_MCP_TUNNEL')
    [void]$projection.Add('LATTICE_TASK_INGRESS_PROFILE_SHA256=' + $IngressProfileSha256)
    $canonical = $projection -join "`n"
    return [pscustomobject][ordered]@{
        schema = 'lattice.task038.tunnel-safe-config.v1'
        digest = Get-StringSha256 -Value $canonical
        byte_count = [Text.UTF8Encoding]::new($false).GetByteCount($canonical)
    }
}

function Get-Task038DelimitedSha256 {
    param([Parameter(Mandatory = $true)][string[]]$Parts)

    return Get-StringSha256 -Value ($Parts -join "`n")
}

function Assert-Task038JsonObjectKeys {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string[]]$Expected,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    if ($null -eq $Object -or $Object -isnot [pscustomobject]) {
        throw $FailureCode
    }
    $actual = @($Object.PSObject.Properties.Name | Sort-Object -CaseSensitive)
    $wanted = @($Expected | Sort-Object -CaseSensitive)
    if ($actual.Count -ne $wanted.Count) {
        throw $FailureCode
    }
    for ($index = 0; $index -lt $wanted.Count; $index++) {
        if ($actual[$index] -cne $wanted[$index]) {
            throw $FailureCode
        }
    }
}

function Test-Task038JsonInteger {
    param($Value)

    return $Value -is [int] -or $Value -is [long]
}

function Get-Task038ProcessIdentityParts {
    param(
        [Parameter(Mandatory = $true)]$Identity,
        [Parameter(Mandatory = $true)][string]$FailureCode
    )

    Assert-Task038JsonObjectKeys `
        -Object $Identity `
        -Expected @('pid', 'creation_time', 'creation_time_source', 'exe_sha256') `
        -FailureCode $FailureCode
    $processIdValue = 0L
    if (
        -not (Test-Task038JsonInteger -Value $Identity.pid) -or
        -not [long]::TryParse([string]$Identity.pid, [ref]$processIdValue) -or
        $processIdValue -lt 1 -or
        $Identity.creation_time -isnot [string] -or
        [string]::IsNullOrWhiteSpace([string]$Identity.creation_time) -or
        ([string]$Identity.creation_time).IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0 -or
        $Identity.creation_time_source -isnot [string] -or
        [string]::IsNullOrWhiteSpace([string]$Identity.creation_time_source) -or
        ([string]$Identity.creation_time_source).IndexOfAny([char[]]@("`r", "`n", [char]0)) -ge 0 -or
        [string]$Identity.creation_time_source -cnotmatch '\A(?:WINDOWS_PROCESS_TIMES|LINUX_PROC_STAT_START_TICKS|DARWIN_KINFO_PROC_START_TIME)\z' -or
        $Identity.exe_sha256 -isnot [string] -or
        [string]$Identity.exe_sha256 -cnotmatch '\A[0-9a-f]{64}\z'
    ) {
        throw $FailureCode
    }
    return @(
        [string]$processIdValue,
        [string]$Identity.creation_time,
        [string]$Identity.creation_time_source,
        [string]$Identity.exe_sha256
    )
}

function Set-Task038OwnerOnlyAcl {
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
    catch {
        throw 'TASK038_TUNNEL_LIFECYCLE_ACL_REJECTED'
    }
}

function New-Task038LaunchAuthoritySink {
    param(
        [Parameter(Mandatory = $true)][string]$DeliveryRoot,
        [Parameter(Mandatory = $true)][string]$ConsumerSessionId,
        [Parameter(Mandatory = $true)][long]$ConfigGeneration,
        [Parameter(Mandatory = $true)][string]$SafeConfigSha256,
        [Parameter(Mandatory = $true)]$ProfileEvidence,
        [Parameter(Mandatory = $true)][string]$TunnelClient
    )

    if (
        $ConsumerSessionId -cnotmatch '\A[0-9a-f]{32}\z' -or
        $ConfigGeneration -lt 1 -or
        $SafeConfigSha256 -cnotmatch '\A[0-9a-f]{64}\z'
    ) { throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_CONFIG_REJECTED' }
    $root = Get-CanonicalPath -Path $DeliveryRoot
    $authorityRoot = Join-Path $root 'tunnel-launch-authority'
    [IO.Directory]::CreateDirectory($authorityRoot) | Out-Null
    Assert-NoReparsePath -Path $authorityRoot -FailureCode 'TASK038_TUNNEL_LAUNCH_AUTHORITY_PATH_REJECTED'
    Set-Task038OwnerOnlyAcl -Path $authorityRoot -Directory $true
    $sessionId = [Guid]::NewGuid().ToString('N')
    $path = Join-Path $authorityRoot ($sessionId + '.jsonl')
    $stream = $null
    try {
        $stream = [IO.File]::Open(
            $path,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::Read
        )
        Set-Task038OwnerOnlyAcl -Path $path -Directory $false
        $nativeIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $path -Directory $false
        $nonce = New-Task038PrivateNonce
        $nonceCommitment = Get-StringSha256 -Value (@(
            'lattice.task038.tunnel-launch-authority-nonce.v1',
            $sessionId,
            $ConsumerSessionId,
            $SafeConfigSha256,
            [Convert]::ToBase64String($nonce)
        ) -join "`n")
        $owner = Get-Process -Id $PID -ErrorAction Stop
        $ownerPath = Get-CanonicalPath -Path ([string]$owner.Path)
        $sink = [pscustomobject][ordered]@{
            stream = $stream
            path = $path
            native_identity = $nativeIdentity
            nonce = $nonce
            nonce_commitment = $nonceCommitment
            session_id = $sessionId
            consumer_session_id = $ConsumerSessionId
            config_generation = $ConfigGeneration
            safe_config_sha256 = $SafeConfigSha256
            ordinal = 0L
            previous_hmac_sha256 = ('0' * 64)
            owner_process_id = [long]$PID
            owner_process_creation_time = $owner.StartTime.ToUniversalTime().ToFileTimeUtc().ToString()
            owner_process_executable_path = $ownerPath
            owner_process_executable_sha256 = Get-FileSha256 -Path $ownerPath -FailureCode 'TASK038_TUNNEL_LAUNCH_AUTHORITY_OWNER_REJECTED'
            owner_process_executable_native_identity = Get-LatticeWindowsNativePathIdentityToken -Path $ownerPath -Directory $false
            profile_path = [string]$ProfileEvidence.profile_path
            profile_raw_sha256 = [string]$ProfileEvidence.profile_raw_sha256
            profile_native_identity = [string]$ProfileEvidence.profile_native_identity
            tunnel_client_path = $TunnelClient
            tunnel_client_sha256 = [string]$ProfileEvidence.tunnel_client_sha256
            tunnel_client_native_identity = [string]$ProfileEvidence.tunnel_client_native_identity
            latticed_path = [string]$ProfileEvidence.latticed_executable
            latticed_sha256 = [string]$ProfileEvidence.latticed_sha256
            latticed_native_identity = [string]$ProfileEvidence.latticed_native_identity
        }
        Add-Task038LaunchAuthorityEvent -Sink $sink -EventType 'AUTHORITY_OPEN' -Payload ([ordered]@{
            owner_process_id = [long]$sink.owner_process_id
            owner_process_creation_time = [string]$sink.owner_process_creation_time
            owner_process_creation_time_source = 'WINDOWS_PROCESS_TIMES'
            owner_process_executable_path = [string]$sink.owner_process_executable_path
            owner_process_executable_sha256 = [string]$sink.owner_process_executable_sha256
            owner_process_executable_native_identity = [string]$sink.owner_process_executable_native_identity
            profile_path = [string]$sink.profile_path
            profile_raw_sha256 = [string]$sink.profile_raw_sha256
            profile_native_identity = [string]$sink.profile_native_identity
            tunnel_client_path = [string]$sink.tunnel_client_path
            tunnel_client_sha256 = [string]$sink.tunnel_client_sha256
            tunnel_client_native_identity = [string]$sink.tunnel_client_native_identity
            latticed_path = [string]$sink.latticed_path
            latticed_sha256 = [string]$sink.latticed_sha256
            latticed_native_identity = [string]$sink.latticed_native_identity
            authority_sink_native_identity = [string]$sink.native_identity
        })
        return $sink
    }
    catch {
        if ($null -ne $stream) { $stream.Dispose() }
        throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED'
    }
}

function Add-Task038LaunchAuthorityEvent {
    param(
        [Parameter(Mandatory = $true)]$Sink,
        [Parameter(Mandatory = $true)][ValidateSet('AUTHORITY_OPEN', 'PROCESS_SPAWN_BOUND', 'PROCESS_REAPED', 'INNER_CHAIN_BOUND', 'AUTHORITY_CLOSED')][string]$EventType,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Payload
    )

    if (
        $null -eq $Sink.stream -or
        -not $Sink.stream.CanWrite -or
        -not (Test-LatticeWindowsNativePathIdentity -Path ([string]$Sink.path) -Directory $false -ExpectedToken ([string]$Sink.native_identity))
    ) { throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED' }
    $Sink.ordinal = [long]$Sink.ordinal + 1L
    $observedAtUtc = [DateTimeOffset]::UtcNow.ToString('o')
    $payloadJson = $Payload | ConvertTo-Json -Compress -Depth 16
    $payloadSha256 = Get-StringSha256 -Value $payloadJson
    $hmacInput = @(
        'lattice.task038.tunnel-launch-authority-hmac.v1',
        [string]$Sink.previous_hmac_sha256,
        [string]$Sink.session_id,
        [string]$Sink.consumer_session_id,
        [string]$Sink.config_generation,
        [string]$Sink.safe_config_sha256,
        [string]$Sink.nonce_commitment,
        [string]$Sink.ordinal,
        $EventType,
        $observedAtUtc,
        $payloadSha256
    ) -join "`n"
    $eventHmac = Get-Task038HmacSha256 -Key ([byte[]]$Sink.nonce) -Value $hmacInput
    $record = [ordered]@{
        schema = 'lattice.task038.tunnel-launch-authority.v1'
        event_type = $EventType
        session_id = [string]$Sink.session_id
        consumer_session_id = [string]$Sink.consumer_session_id
        config_generation = [long]$Sink.config_generation
        safe_config_sha256 = [string]$Sink.safe_config_sha256
        nonce_commitment = [string]$Sink.nonce_commitment
        ordinal = [long]$Sink.ordinal
        observed_at_utc = $observedAtUtc
        payload = $Payload
        payload_sha256 = $payloadSha256
        previous_hmac_sha256 = [string]$Sink.previous_hmac_sha256
        event_hmac_sha256 = $eventHmac
    }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(
        (($record | ConvertTo-Json -Compress -Depth 20) + "`n")
    )
    $Sink.stream.Write($bytes, 0, $bytes.Length)
    $Sink.stream.Flush($true)
    $Sink.previous_hmac_sha256 = $eventHmac
}

function Complete-Task038LaunchAuthoritySink {
    param([Parameter(Mandatory = $true)]$Sink)

    Add-Task038LaunchAuthorityEvent -Sink $Sink -EventType 'AUTHORITY_CLOSED' -Payload ([ordered]@{
        authority_event_count_before_close = [long]$Sink.ordinal
        current_owner_process_id = [long]$PID
        authority_sink_launch_owned = $true
    })
    try {
        $Sink.stream.Flush($true)
        if ($Sink.stream.Length -lt 1 -or $Sink.stream.Length -gt 1048576) {
            throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED'
        }
        $bytes = [byte[]]::new([int]$Sink.stream.Length)
        $Sink.stream.Position = 0
        $offset = 0
        while ($offset -lt $bytes.Length) {
            $read = $Sink.stream.Read($bytes, $offset, $bytes.Length - $offset)
            if ($read -lt 1) { throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED' }
            $offset += $read
        }
        $Sink.stream.Position = $Sink.stream.Length
        if (
            $bytes.Length -lt 1 -or $bytes.Length -gt 1048576 -or
            ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf)
        ) { throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED' }
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
        if (-not $text.EndsWith("`n", [StringComparison]::Ordinal) -or $text.Contains("`r")) {
            throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED'
        }
        $parts = @($text.Split([string[]]@("`n"), [StringSplitOptions]::None))
        if ($parts.Count -lt 2 -or $parts[-1] -cne '') {
            throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED'
        }
        $lines = @($parts[0..($parts.Count - 2)])
        if (@($lines | Where-Object { $_ -ceq '' }).Count -ne 0) {
            throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED'
        }
        $expectedTypes = @('AUTHORITY_OPEN', 'PROCESS_SPAWN_BOUND', 'PROCESS_REAPED', 'INNER_CHAIN_BOUND', 'AUTHORITY_CLOSED')
        if ($lines.Count -ne $expectedTypes.Count) { throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED' }
        $previous = '0' * 64
        for ($index = 0; $index -lt $lines.Count; $index++) {
            $record = $lines[$index] | ConvertFrom-Json -ErrorAction Stop
            $payloadJson = $record.payload | ConvertTo-Json -Compress -Depth 16
            $payloadSha256 = Get-StringSha256 -Value $payloadJson
            $hmacInput = @(
                'lattice.task038.tunnel-launch-authority-hmac.v1', $previous,
                [string]$Sink.session_id, [string]$Sink.consumer_session_id,
                [string]$Sink.config_generation, [string]$Sink.safe_config_sha256,
                [string]$Sink.nonce_commitment, [string]($index + 1), $expectedTypes[$index],
                [string]$record.observed_at_utc, $payloadSha256
            ) -join "`n"
            $expectedHmac = Get-Task038HmacSha256 -Key ([byte[]]$Sink.nonce) -Value $hmacInput
            if (
                [string]$record.schema -cne 'lattice.task038.tunnel-launch-authority.v1' -or
                [string]$record.event_type -cne $expectedTypes[$index] -or
                [string]$record.session_id -cne [string]$Sink.session_id -or
                [string]$record.consumer_session_id -cne [string]$Sink.consumer_session_id -or
                [long]$record.config_generation -ne [long]$Sink.config_generation -or
                [string]$record.safe_config_sha256 -cne [string]$Sink.safe_config_sha256 -or
                [string]$record.nonce_commitment -cne [string]$Sink.nonce_commitment -or
                [long]$record.ordinal -ne ($index + 1) -or
                [string]$record.payload_sha256 -cne $payloadSha256 -or
                [string]$record.previous_hmac_sha256 -cne $previous -or
                [string]$record.event_hmac_sha256 -cne $expectedHmac
            ) { throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED' }
            $previous = $expectedHmac
        }
        if (-not (Test-LatticeWindowsNativePathIdentity -Path ([string]$Sink.path) -Directory $false -ExpectedToken ([string]$Sink.native_identity))) {
            throw 'TASK038_TUNNEL_LAUNCH_AUTHORITY_REJECTED'
        }
        return [pscustomobject][ordered]@{
            scope = 'LAUNCH_OWNED_PROCESS_EVIDENCE'
            path = [string]$Sink.path
            native_identity = [string]$Sink.native_identity
            raw_sha256 = Get-ByteArraySha256 -Bytes $bytes
            byte_count = [long]$bytes.Length
            strict_utf8 = $true
            event_count = [long]$lines.Count
            final_hmac_sha256 = $previous
            nonce_commitment = [string]$Sink.nonce_commitment
            session_id = [string]$Sink.session_id
            consumer_session_id = [string]$Sink.consumer_session_id
        }
    }
    finally {
        $Sink.stream.Dispose()
        [Array]::Clear([byte[]]$Sink.nonce, 0, ([byte[]]$Sink.nonce).Length)
    }
}

function New-Task038LifecycleSink {
    param(
        [Parameter(Mandatory = $true)][string]$DeliveryRoot,
        [Parameter(Mandatory = $true)][long]$ConfigGeneration,
        [Parameter(Mandatory = $true)][string]$SafeConfigSha256
    )

    if ($ConfigGeneration -lt 1 -or $SafeConfigSha256 -cnotmatch '\A[0-9a-f]{64}\z') {
        throw 'TASK038_TUNNEL_LIFECYCLE_CONFIG_REJECTED'
    }
    $root = Get-CanonicalPath -Path $DeliveryRoot
    [IO.Directory]::CreateDirectory($root) | Out-Null
    Assert-NoReparsePath -Path $root -FailureCode 'TASK038_TUNNEL_LIFECYCLE_PATH_REJECTED'
    $lifecycleRoot = Join-Path $root 'tunnel-lifecycle'
    [IO.Directory]::CreateDirectory($lifecycleRoot) | Out-Null
    Assert-NoReparsePath -Path $lifecycleRoot -FailureCode 'TASK038_TUNNEL_LIFECYCLE_PATH_REJECTED'
    Set-Task038OwnerOnlyAcl -Path $lifecycleRoot -Directory $true
    $sessionId = [Guid]::NewGuid().ToString('N')
    if ($sessionId -cnotmatch '\A[0-9a-f]{32}\z') {
        throw 'TASK038_TUNNEL_LIFECYCLE_SESSION_REJECTED'
    }
    $eventPath = Join-Path $lifecycleRoot ($sessionId + '.jsonl')
    $stream = $null
    try {
        $stream = [IO.File]::Open(
            $eventPath,
            [IO.FileMode]::CreateNew,
            [IO.FileAccess]::ReadWrite,
            [IO.FileShare]::ReadWrite
        )
    }
    catch {
        throw 'TASK038_TUNNEL_LIFECYCLE_PATH_REJECTED'
    }
    finally {
        if ($null -ne $stream) { $stream.Dispose() }
    }
    Set-Task038OwnerOnlyAcl -Path $eventPath -Directory $false
    $eventIdentity = Get-LatticeWindowsNativePathIdentityToken -Path $eventPath -Directory $false
    return [pscustomobject][ordered]@{
        session_id = $sessionId
        config_generation = $ConfigGeneration
        safe_config_sha256 = $SafeConfigSha256
        event_path = $eventPath
        event_native_identity = $eventIdentity
    }
}

function Get-Task038LifecycleEvidence {
    param(
        [Parameter(Mandatory = $true)]$Sink,
        [Parameter(Mandatory = $true)][string]$ExpectedInnerExeSha256,
        [Parameter(Mandatory = $true)][string]$ControlPlaneApiKey,
        [Parameter(Mandatory = $true)][string]$PostgresPassword
    )

    $failureCode = 'TASK038_TUNNEL_LIFECYCLE_EVIDENCE_REJECTED'
    if (-not (Test-LatticeWindowsNativePathIdentity `
        -Path ([string]$Sink.event_path) `
        -Directory $false `
        -ExpectedToken ([string]$Sink.event_native_identity))) {
        throw $failureCode
    }
    $eventItem = Get-Item -LiteralPath ([string]$Sink.event_path) -Force -ErrorAction SilentlyContinue
    if (
        $ExpectedInnerExeSha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
        $null -eq $eventItem -or
        $eventItem.PSIsContainer -or
        $eventItem.Length -lt 1 -or
        $eventItem.Length -gt 1048576
    ) {
        throw $failureCode
    }
    try {
        $bytes = [IO.File]::ReadAllBytes([string]$Sink.event_path)
        if (
            $bytes.Length -lt 1 -or
            ($bytes.Length -ge 3 -and $bytes[0] -eq 0xef -and $bytes[1] -eq 0xbb -and $bytes[2] -eq 0xbf)
        ) {
            throw $failureCode
        }
        $text = [Text.UTF8Encoding]::new($false, $true).GetString($bytes)
    }
    catch {
        throw $failureCode
    }
    if (
        -not $text.EndsWith("`n", [StringComparison]::Ordinal) -or
        $text.IndexOf("`r", [StringComparison]::Ordinal) -ge 0 -or
        $text.IndexOf($ControlPlaneApiKey, [StringComparison]::Ordinal) -ge 0 -or
        $text.IndexOf($PostgresPassword, [StringComparison]::Ordinal) -ge 0
    ) {
        throw $failureCode
    }
    $lines = @($text.Split([string[]]@("`n"), [StringSplitOptions]::None))
    if ($lines[-1] -cne '') { throw $failureCode }
    $lines = @($lines[0..($lines.Count - 2)])
    if ($lines.Count -lt 1) { throw $failureCode }

    $eventTypes = @('SPAWN', 'OPEN', 'CLOSE_REQUESTED', 'PIPE_CLOSED', 'EXITED', 'REAPED')
    $previousEventSha256 = ('0' * 64) -join ''
    $eventIndex = 0
    $anomalyCount = 0
    $previousObservedAt = [DateTimeOffset]::MinValue
    $stableIdentityParts = $null
    $stableCommandSha256 = $null
    $stableEndpointRef = $null
    $exitCode = $null
    $anomalyCodes = [Collections.Generic.List[string]]::new()

    foreach ($line in $lines) {
        try { $record = $line | ConvertFrom-Json -ErrorAction Stop }
        catch { throw $failureCode }
        if ([string]$record.record_type -ceq 'LIFECYCLE') {
            Assert-Task038JsonObjectKeys -Object $record -Expected @(
                'schema', 'record_type', 'component', 'event_type', 'session_id',
                'process_identity', 'config_generation', 'safe_config_sha256',
                'session_command_sha256', 'endpoint_ref', 'lifecycle_strategy', 'ordinal',
                'observed_at_utc', 'exit_code', 'previous_event_sha256', 'idempotency_key',
                'event_sha256', 'lifecycle_classification', 'threshold_profile_version', 'thresholds'
            ) -FailureCode $failureCode
            if ($eventIndex -ge $eventTypes.Count) { throw $failureCode }
            $identityParts = Get-Task038ProcessIdentityParts -Identity $record.process_identity -FailureCode $failureCode
            Assert-Task038JsonObjectKeys -Object $record.lifecycle_strategy -Expected @(
                'transport', 'endpoint_kind', 'spawn_mode', 'create_suspended_owned', 'job_assignment_ownership'
            ) -FailureCode $failureCode
            Assert-Task038JsonObjectKeys -Object $record.thresholds -Expected @(
                'pipe_milliseconds', 'exit_milliseconds', 'reap_milliseconds', 'confirm_milliseconds'
            ) -FailureCode $failureCode
            $observedAt = [DateTimeOffset]::MinValue
            if (
                $record.schema -isnot [string] -or
                $record.record_type -isnot [string] -or
                $record.component -isnot [string] -or
                $record.event_type -isnot [string] -or
                $record.session_id -isnot [string] -or
                -not (Test-Task038JsonInteger -Value $record.config_generation) -or
                $record.safe_config_sha256 -isnot [string] -or
                $record.session_command_sha256 -isnot [string] -or
                $record.endpoint_ref -isnot [string] -or
                $record.lifecycle_strategy.transport -isnot [string] -or
                $record.lifecycle_strategy.endpoint_kind -isnot [string] -or
                $record.lifecycle_strategy.spawn_mode -isnot [string] -or
                $record.lifecycle_strategy.create_suspended_owned -isnot [bool] -or
                $record.lifecycle_strategy.job_assignment_ownership -isnot [string] -or
                -not (Test-Task038JsonInteger -Value $record.ordinal) -or
                $record.observed_at_utc -isnot [string] -or
                $record.previous_event_sha256 -isnot [string] -or
                $record.idempotency_key -isnot [string] -or
                $record.event_sha256 -isnot [string] -or
                $record.lifecycle_classification -isnot [string] -or
                [string]$record.schema -cne 'lattice.tunnel-client.lifecycle-event.v1' -or
                [string]$record.component -cne 'mcpclient' -or
                [string]$record.event_type -cne $eventTypes[$eventIndex] -or
                [string]$record.session_id -cne [string]$Sink.session_id -or
                [long]$record.config_generation -ne [long]$Sink.config_generation -or
                [string]$record.safe_config_sha256 -cne [string]$Sink.safe_config_sha256 -or
                [string]$record.session_command_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
                [string]$record.endpoint_ref -cnotmatch '\Ahmac-sha256:[0-9a-f]{64}\z' -or
                [string]$record.lifecycle_strategy.transport -cne 'STDIO' -or
                [string]$record.lifecycle_strategy.endpoint_kind -cne 'ANONYMOUS_PIPE' -or
                [string]$record.lifecycle_strategy.spawn_mode -cne 'DIRECT' -or
                [bool]$record.lifecycle_strategy.create_suspended_owned -or
                [string]$record.lifecycle_strategy.job_assignment_ownership -cne 'EXTERNAL_OWNER' -or
                [long]$record.ordinal -ne ($eventIndex + 1) -or
                [string]$record.observed_at_utc -cnotmatch '\A\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z\z' -or
                -not [DateTimeOffset]::TryParse(
                    [string]$record.observed_at_utc,
                    [Globalization.CultureInfo]::InvariantCulture,
                    [Globalization.DateTimeStyles]::RoundtripKind,
                    [ref]$observedAt
                ) -or
                $observedAt -lt $previousObservedAt -or
                [string]$record.previous_event_sha256 -cne $previousEventSha256 -or
                [string]$record.idempotency_key -cnotmatch '\A[0-9a-f]{64}\z' -or
                [string]$record.event_sha256 -cnotmatch '\A[0-9a-f]{64}\z' -or
                [string]$record.lifecycle_classification -cne 'UNKNOWN' -or
                $null -ne $record.threshold_profile_version -or
                $null -ne $record.thresholds.pipe_milliseconds -or
                $null -ne $record.thresholds.exit_milliseconds -or
                $null -ne $record.thresholds.reap_milliseconds -or
                $null -ne $record.thresholds.confirm_milliseconds
            ) {
                throw $failureCode
            }
            if ($eventIndex -lt 4) {
                if ($null -ne $record.exit_code) { throw $failureCode }
                $exitCodeText = 'null'
            }
            else {
                $parsedExitCode = 0
                if (
                    -not (Test-Task038JsonInteger -Value $record.exit_code) -or
                    -not [int]::TryParse([string]$record.exit_code, [ref]$parsedExitCode)
                ) { throw $failureCode }
                if ($null -eq $exitCode) { $exitCode = $parsedExitCode }
                elseif ($parsedExitCode -ne $exitCode) { throw $failureCode }
                $exitCodeText = [string]$parsedExitCode
            }
            if ($null -eq $stableIdentityParts) {
                if ($identityParts[3] -cne $ExpectedInnerExeSha256) {
                    throw $failureCode
                }
                $stableIdentityParts = $identityParts
                $stableCommandSha256 = [string]$record.session_command_sha256
                $stableEndpointRef = [string]$record.endpoint_ref
            }
            elseif (
                ($identityParts -join "`n") -cne ($stableIdentityParts -join "`n") -or
                [string]$record.session_command_sha256 -cne $stableCommandSha256 -or
                [string]$record.endpoint_ref -cne $stableEndpointRef
            ) {
                throw $failureCode
            }
            $expectedIdempotency = Get-Task038DelimitedSha256 -Parts @(
                'lattice.tunnel-client.lifecycle-idempotency.v1',
                [string]$Sink.session_id,
                [string]$Sink.config_generation,
                [string]$Sink.safe_config_sha256,
                $stableCommandSha256,
                $stableEndpointRef,
                [string]$record.event_type,
                $identityParts[0], $identityParts[1], $identityParts[2], $identityParts[3],
                $exitCodeText
            )
            $expectedEventSha256 = Get-Task038DelimitedSha256 -Parts @(
                'lattice.tunnel-client.lifecycle-event-hash.v1',
                $previousEventSha256,
                $expectedIdempotency,
                [string]($eventIndex + 1),
                [string]$record.observed_at_utc
            )
            if (
                [string]$record.idempotency_key -cne $expectedIdempotency -or
                [string]$record.event_sha256 -cne $expectedEventSha256
            ) {
                throw $failureCode
            }
            $previousObservedAt = $observedAt
            $previousEventSha256 = $expectedEventSha256
            $eventIndex++
        }
        elseif ([string]$record.record_type -ceq 'ANOMALY') {
            Assert-Task038JsonObjectKeys -Object $record -Expected @(
                'schema', 'record_type', 'component', 'anomaly_code', 'session_id',
                'expected_process_identity', 'observed_process_identity', 'config_generation',
                'safe_config_sha256', 'session_command_sha256', 'endpoint_ref', 'anomaly_ordinal',
                'observed_at_utc', 'related_event_sha256', 'idempotency_key', 'anomaly_sha256',
                'lifecycle_classification', 'threshold_profile_version', 'thresholds'
            ) -FailureCode $failureCode
            $expectedIdentityParts = Get-Task038ProcessIdentityParts -Identity $record.expected_process_identity -FailureCode $failureCode
            $observedIdentityParts = @('null', 'null', 'null', 'null')
            if ($null -ne $record.observed_process_identity) {
                $observedIdentityParts = Get-Task038ProcessIdentityParts -Identity $record.observed_process_identity -FailureCode $failureCode
            }
            Assert-Task038JsonObjectKeys -Object $record.thresholds -Expected @(
                'pipe_milliseconds', 'exit_milliseconds', 'reap_milliseconds', 'confirm_milliseconds'
            ) -FailureCode $failureCode
            $observedAt = [DateTimeOffset]::MinValue
            if (
                $eventIndex -lt 1 -or
                $record.schema -isnot [string] -or
                $record.record_type -isnot [string] -or
                $record.component -isnot [string] -or
                $record.anomaly_code -isnot [string] -or
                $record.session_id -isnot [string] -or
                -not (Test-Task038JsonInteger -Value $record.config_generation) -or
                $record.safe_config_sha256 -isnot [string] -or
                $record.session_command_sha256 -isnot [string] -or
                $record.endpoint_ref -isnot [string] -or
                -not (Test-Task038JsonInteger -Value $record.anomaly_ordinal) -or
                $record.observed_at_utc -isnot [string] -or
                $record.related_event_sha256 -isnot [string] -or
                $record.idempotency_key -isnot [string] -or
                $record.anomaly_sha256 -isnot [string] -or
                $record.lifecycle_classification -isnot [string] -or
                [string]$record.schema -cne 'lattice.tunnel-client.lifecycle-anomaly.v1' -or
                [string]$record.component -cne 'mcpclient' -or
                [string]$record.anomaly_code -cnotmatch '\A(?:PROCESS_IDENTITY_CONFLICT|PROCESS_IDENTITY_UNAVAILABLE|PROCESS_PRESENT_AFTER_WAIT|UNEXPECTED_EXIT_BEFORE_CLOSE)\z' -or
                [string]$record.session_id -cne [string]$Sink.session_id -or
                [long]$record.config_generation -ne [long]$Sink.config_generation -or
                [string]$record.safe_config_sha256 -cne [string]$Sink.safe_config_sha256 -or
                [string]$record.session_command_sha256 -cne $stableCommandSha256 -or
                [string]$record.endpoint_ref -cne $stableEndpointRef -or
                [long]$record.anomaly_ordinal -ne ($anomalyCount + 1) -or
                [string]$record.related_event_sha256 -cne $previousEventSha256 -or
                [string]$record.observed_at_utc -cnotmatch '\A\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z\z' -or
                -not [DateTimeOffset]::TryParse(
                    [string]$record.observed_at_utc,
                    [Globalization.CultureInfo]::InvariantCulture,
                    [Globalization.DateTimeStyles]::RoundtripKind,
                    [ref]$observedAt
                ) -or
                $observedAt -lt $previousObservedAt -or
                [string]$record.lifecycle_classification -cne 'UNKNOWN' -or
                $null -ne $record.threshold_profile_version -or
                $null -ne $record.thresholds.pipe_milliseconds -or
                $null -ne $record.thresholds.exit_milliseconds -or
                $null -ne $record.thresholds.reap_milliseconds -or
                $null -ne $record.thresholds.confirm_milliseconds -or
                ($expectedIdentityParts -join "`n") -cne ($stableIdentityParts -join "`n")
            ) {
                throw $failureCode
            }
            $expectedAnomalyIdempotency = Get-Task038DelimitedSha256 -Parts @(
                'lattice.tunnel-client.lifecycle-anomaly-idempotency.v1',
                [string]$Sink.session_id,
                [string]$Sink.config_generation,
                [string]$Sink.safe_config_sha256,
                $stableCommandSha256,
                $stableEndpointRef,
                [string]$record.anomaly_code,
                $expectedIdentityParts[0], $expectedIdentityParts[1], $expectedIdentityParts[2], $expectedIdentityParts[3],
                $observedIdentityParts[0], $observedIdentityParts[1], $observedIdentityParts[2], $observedIdentityParts[3]
            )
            $expectedAnomalySha256 = Get-Task038DelimitedSha256 -Parts @(
                'lattice.tunnel-client.lifecycle-anomaly-hash.v1',
                $previousEventSha256,
                $expectedAnomalyIdempotency,
                [string]($anomalyCount + 1),
                [string]$record.observed_at_utc
            )
            if (
                [string]$record.idempotency_key -cne $expectedAnomalyIdempotency -or
                [string]$record.anomaly_sha256 -cne $expectedAnomalySha256
            ) {
                throw $failureCode
            }
            $previousObservedAt = $observedAt
            [void]$anomalyCodes.Add([string]$record.anomaly_code)
            $anomalyCount++
        }
        else {
            throw $failureCode
        }
    }
    if (-not (Test-LatticeWindowsNativePathIdentity `
        -Path ([string]$Sink.event_path) `
        -Directory $false `
        -ExpectedToken ([string]$Sink.event_native_identity))) {
        throw $failureCode
    }
    return [pscustomobject][ordered]@{
        schema = 'lattice.task038.tunnel-lifecycle-evidence.v1'
        event_path = [string]$Sink.event_path
        event_raw_sha256 = Get-ByteArraySha256 -Bytes $bytes
        event_byte_count = [long]$bytes.Length
        event_strict_utf8 = $true
        event_native_identity = [string]$Sink.event_native_identity
        session_id = [string]$Sink.session_id
        config_generation = [long]$Sink.config_generation
        safe_config_sha256 = [string]$Sink.safe_config_sha256
        event_count = [long]$eventIndex
        anomaly_count = [long]$anomalyCount
        anomaly_codes = @($anomalyCodes)
        chain_complete = ($eventIndex -eq $eventTypes.Count)
        normal_close_complete = ($eventIndex -eq $eventTypes.Count -and $anomalyCount -eq 0)
        final_event_sha256 = $previousEventSha256
        inner_process_id = [long]$stableIdentityParts[0]
        inner_process_creation_time = $stableIdentityParts[1]
        inner_process_creation_time_source = $stableIdentityParts[2]
        inner_process_exe_sha256 = $stableIdentityParts[3]
        inner_session_command_sha256 = $stableCommandSha256
        inner_endpoint_ref = $stableEndpointRef
        exit_code = if ($null -eq $exitCode) { $null } else { [int]$exitCode }
        lifecycle_threshold_decision = 'C_CALIBRATION_FIRST'
        lifecycle_threshold_profile = $null
        lifecycle_classification = 'UNKNOWN'
        leak_claimed = $false
    }
}

$tunnelClient = Resolve-RequiredLeafPath -Path $TunnelClientExecutable -FailureCode 'TASK038_TUNNEL_CLIENT_REJECTED'
if (-not [IO.Path]::IsPathRooted($ProfileDirectory)) {
    throw 'TASK038_TUNNEL_PROFILE_DIRECTORY_REJECTED'
}
$profileRoot = [IO.Path]::GetFullPath($ProfileDirectory)
Assert-NoReparsePath -Path $profileRoot -FailureCode 'TASK038_TUNNEL_PROFILE_DIRECTORY_REJECTED'
$taskIngressKind = $null
$taskIngressProfileDigest = $null
$profileEvidence = $null
$runtimeEnvironment = $null
$lifecycleSink = $null
$lifecycleEvidence = $null
$lifecycleSafeConfig = $null
$launchAuthoritySink = $null
$launchAuthorityEvidence = $null

trap {
    if ($null -ne $launchAuthoritySink -and $null -ne $launchAuthoritySink.stream) {
        try { $launchAuthoritySink.stream.Dispose() } catch {}
        if ($null -ne $launchAuthoritySink.nonce) {
            try {
                [Array]::Clear(
                    [byte[]]$launchAuthoritySink.nonce,
                    0,
                    ([byte[]]$launchAuthoritySink.nonce).Length
                )
            }
            catch {}
        }
    }
    throw $_.Exception
}

$arguments = switch ($Mode) {
    'Init' {
        if ($TunnelId -cnotmatch '\Atunnel_[0-9a-f]{32}\z') {
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
    $runtimeEnvironment = Get-Task038TunnelRuntimeEnvironment
        $taskIngressKind = 'CHATGPT_SECURE_MCP_TUNNEL'
        $profileEvidence = Get-LiveTaskIngressProfileDigest `
            -ProfileRoot $profileRoot `
            -ProfileName $ProfileName `
            -TunnelClient $tunnelClient `
            -PassThru
        $taskIngressProfileDigest = [string]$profileEvidence.digest
        $lifecycleSafeConfig = Get-Task038SafeConfigProjection `
            -RuntimeEnvironment $runtimeEnvironment `
            -IngressProfileSha256 $taskIngressProfileDigest
        $lifecycleSink = New-Task038LifecycleSink `
            -DeliveryRoot ([string]$runtimeEnvironment.LATTICE_DELIVERY_ROOT) `
            -ConfigGeneration ([long]$runtimeEnvironment.LATTICE_STORE_AUTHORITY_REVISION) `
            -SafeConfigSha256 ([string]$lifecycleSafeConfig.digest)
        @('run', '--profile', $ProfileName, '--profile-dir', $profileRoot)
        break
    }
    'ManagedRun' {
        if ([string]::IsNullOrWhiteSpace($env:CONTROL_PLANE_API_KEY)) {
            throw 'TASK038_TUNNEL_RUNTIME_KEY_REQUIRED'
        }
        $runtimeEnvironment = Get-Task038TunnelRuntimeEnvironment
        $taskIngressKind = 'CHATGPT_SECURE_MCP_TUNNEL'
        $profileEvidence = Get-LiveTaskIngressProfileDigest `
            -ProfileRoot $profileRoot `
            -ProfileName $ProfileName `
            -TunnelClient $tunnelClient `
            -PassThru
        $taskIngressProfileDigest = [string]$profileEvidence.digest
        $lifecycleSafeConfig = Get-Task038SafeConfigProjection `
            -RuntimeEnvironment $runtimeEnvironment `
            -IngressProfileSha256 $taskIngressProfileDigest
        $lifecycleSink = New-Task038LifecycleSink `
            -DeliveryRoot ([string]$runtimeEnvironment.LATTICE_DELIVERY_ROOT) `
            -ConfigGeneration ([long]$runtimeEnvironment.LATTICE_STORE_AUTHORITY_REVISION) `
            -SafeConfigSha256 ([string]$lifecycleSafeConfig.digest)
        @('run', '--profile', $ProfileName, '--profile-dir', $profileRoot)
        break
    }
}

$clientExitCode = 1
if ($Mode -in @('Run', 'ManagedRun')) {
    $consumerSessionId = [Environment]::GetEnvironmentVariable(
        'LATTICE_P0_CONSUMER_SESSION_ID',
        'Process'
    )
    if ($consumerSessionId -cnotmatch '\A[0-9a-f]{32}\z') {
        throw 'TASK038_TUNNEL_CONSUMER_SESSION_REQUIRED'
    }
    $launchAuthoritySink = New-Task038LaunchAuthoritySink `
        -DeliveryRoot ([string]$runtimeEnvironment.LATTICE_DELIVERY_ROOT) `
        -ConsumerSessionId $consumerSessionId `
        -ConfigGeneration ([long]$lifecycleSink.config_generation) `
        -SafeConfigSha256 ([string]$lifecycleSink.safe_config_sha256) `
        -ProfileEvidence $profileEvidence `
        -TunnelClient $tunnelClient
    $runtimeEnvironment['LATTICE_TASK_INGRESS_KIND'] = $taskIngressKind
    $runtimeEnvironment['LATTICE_TASK_INGRESS_PROFILE_SHA256'] = $taskIngressProfileDigest
    $runtimeEnvironment['TUNNEL_CLIENT_LIFECYCLE_EVENT_PATH'] = [string]$lifecycleSink.event_path
    $runtimeEnvironment['TUNNEL_CLIENT_LIFECYCLE_SESSION_ID'] = [string]$lifecycleSink.session_id
    $runtimeEnvironment['TUNNEL_CLIENT_LIFECYCLE_CONFIG_GENERATION'] = [string]$lifecycleSink.config_generation
    $runtimeEnvironment['TUNNEL_CLIENT_LIFECYCLE_SAFE_CONFIG_SHA256'] = [string]$lifecycleSink.safe_config_sha256
    if (
        -not (Test-LatticeWindowsNativePathIdentity -Path $profileEvidence.profile_path -Directory $false -ExpectedToken $profileEvidence.profile_native_identity) -or
        -not (Test-LatticeWindowsNativePathIdentity -Path $profileEvidence.latticed_executable -Directory $false -ExpectedToken $profileEvidence.latticed_native_identity) -or
        -not (Test-LatticeWindowsNativePathIdentity -Path $tunnelClient -Directory $false -ExpectedToken $profileEvidence.tunnel_client_native_identity) -or
        (Get-FileSha256 -Path $profileEvidence.profile_path -FailureCode 'TASK038_TUNNEL_PROFILE_IDENTITY_CHANGED') -cne [string]$profileEvidence.profile_raw_sha256 -or
        (Get-FileSha256 -Path $profileEvidence.latticed_executable -FailureCode 'TASK038_TUNNEL_PROFILE_IDENTITY_CHANGED') -cne [string]$profileEvidence.latticed_sha256 -or
        (Get-FileSha256 -Path $tunnelClient -FailureCode 'TASK038_TUNNEL_PROFILE_IDENTITY_CHANGED') -cne [string]$profileEvidence.tunnel_client_sha256 -or
        (Get-FileSha256 -Path ([string]$runtimeEnvironment.LATTICE_DELIVERY_LAUNCHER) -FailureCode 'TASK038_TUNNEL_PROFILE_IDENTITY_CHANGED') -cne [string]$runtimeEnvironment.LATTICE_DELIVERY_LAUNCHER_SHA256
    ) {
        throw 'TASK038_TUNNEL_PROFILE_IDENTITY_CHANGED'
    }
    $startInfo = New-Task038TunnelChildStartInfo -EnvironmentValues $runtimeEnvironment
    $runResult = Invoke-Task038OwnedTunnelClient `
        -TunnelClient $tunnelClient `
        -Arguments $arguments `
        -WorkingDirectory $profileRoot `
        -StartInfo $startInfo `
        -AuthoritySink $launchAuthoritySink
    $clientExitCode = [int]$runResult.exit_code
    $lifecycleEvidence = Get-Task038LifecycleEvidence `
        -Sink $lifecycleSink `
        -ExpectedInnerExeSha256 ([string]$profileEvidence.latticed_sha256) `
        -ControlPlaneApiKey ([string]$runtimeEnvironment.CONTROL_PLANE_API_KEY) `
        -PostgresPassword ([string]$runtimeEnvironment.LATTICE_TASK019_PASSWORD)
    Add-Task038LaunchAuthorityEvent -Sink $launchAuthoritySink -EventType 'INNER_CHAIN_BOUND' -Payload ([ordered]@{
        lifecycle_session_id = [string]$lifecycleEvidence.session_id
        lifecycle_config_generation = [long]$lifecycleEvidence.config_generation
        lifecycle_safe_config_sha256 = [string]$lifecycleEvidence.safe_config_sha256
        lifecycle_event_native_identity = [string]$lifecycleEvidence.event_native_identity
        lifecycle_event_raw_sha256 = [string]$lifecycleEvidence.event_raw_sha256
        lifecycle_final_event_sha256 = [string]$lifecycleEvidence.final_event_sha256
        lifecycle_event_sequence = @('SPAWN', 'OPEN', 'CLOSE_REQUESTED', 'PIPE_CLOSED', 'EXITED', 'REAPED')
        inner_process_id = [long]$lifecycleEvidence.inner_process_id
        inner_process_creation_time = [string]$lifecycleEvidence.inner_process_creation_time
        inner_process_creation_time_source = [string]$lifecycleEvidence.inner_process_creation_time_source
        inner_process_exe_sha256 = [string]$lifecycleEvidence.inner_process_exe_sha256
        inner_session_command_sha256 = [string]$lifecycleEvidence.inner_session_command_sha256
        inner_endpoint_ref = [string]$lifecycleEvidence.inner_endpoint_ref
        inner_exit_code = $lifecycleEvidence.exit_code
        normal_close_complete = [bool]$lifecycleEvidence.normal_close_complete
        lifecycle_classification = 'UNKNOWN'
        threshold_profile = $null
    })
    $launchAuthorityEvidence = Complete-Task038LaunchAuthoritySink -Sink $launchAuthoritySink
    Write-Output (([ordered]@{
        schema = 'lattice.task038.tunnel-outer-lifecycle.v1'
        mode = $Mode
        process_id = [int]$runResult.process_id
        tunnel_client_exit_code = [int]$runResult.exit_code
        started_at_utc = [string]$runResult.started_at_utc
        exited_at_utc = [string]$runResult.exited_at_utc
        create_suspended = $true
        job_assigned_before_resume = $true
        descendant_processes_after_cleanup = 0
        profile_raw_sha256 = [string]$profileEvidence.profile_raw_sha256
        profile_byte_count = [long]$profileEvidence.profile_byte_count
        profile_strict_utf8 = $true
        profile_native_identity = [string]$profileEvidence.profile_native_identity
        latticed_native_identity = [string]$profileEvidence.latticed_native_identity
        tunnel_client_native_identity = [string]$profileEvidence.tunnel_client_native_identity
        lifecycle_event_path = [string]$lifecycleEvidence.event_path
        lifecycle_event_raw_sha256 = [string]$lifecycleEvidence.event_raw_sha256
        lifecycle_event_byte_count = [long]$lifecycleEvidence.event_byte_count
        lifecycle_event_strict_utf8 = $true
        lifecycle_event_native_identity = [string]$lifecycleEvidence.event_native_identity
        lifecycle_session_id = [string]$lifecycleEvidence.session_id
        lifecycle_config_generation = [long]$lifecycleEvidence.config_generation
        lifecycle_safe_config_schema = [string]$lifecycleSafeConfig.schema
        lifecycle_safe_config_sha256 = [string]$lifecycleEvidence.safe_config_sha256
        lifecycle_safe_config_byte_count = [long]$lifecycleSafeConfig.byte_count
        lifecycle_event_count = [long]$lifecycleEvidence.event_count
        lifecycle_anomaly_count = [long]$lifecycleEvidence.anomaly_count
        lifecycle_anomaly_codes = @($lifecycleEvidence.anomaly_codes)
        lifecycle_chain_complete = [bool]$lifecycleEvidence.chain_complete
        lifecycle_normal_close_complete = [bool]$lifecycleEvidence.normal_close_complete
        lifecycle_final_event_sha256 = [string]$lifecycleEvidence.final_event_sha256
        lifecycle_inner_process_id = [long]$lifecycleEvidence.inner_process_id
        lifecycle_inner_process_creation_time = [string]$lifecycleEvidence.inner_process_creation_time
        lifecycle_inner_process_creation_time_source = [string]$lifecycleEvidence.inner_process_creation_time_source
        lifecycle_inner_process_exe_sha256 = [string]$lifecycleEvidence.inner_process_exe_sha256
        lifecycle_inner_exit_code = $lifecycleEvidence.exit_code
        lifecycle_threshold_decision = 'C_CALIBRATION_FIRST'
        lifecycle_threshold_profile = $null
        lifecycle_thresholds = [ordered]@{
            pipe_milliseconds = $null
            exit_milliseconds = $null
            reap_milliseconds = $null
            confirm_milliseconds = $null
        }
        lifecycle_classification = 'UNKNOWN'
        leak_claimed = $false
        authority_scope = [string]$launchAuthorityEvidence.scope
        authority_consumer_session_id = [string]$launchAuthorityEvidence.consumer_session_id
        authority_session_id = [string]$launchAuthorityEvidence.session_id
        authority_nonce_commitment = [string]$launchAuthorityEvidence.nonce_commitment
        authority_receipt_path = [string]$launchAuthorityEvidence.path
        authority_receipt_native_identity = [string]$launchAuthorityEvidence.native_identity
        authority_receipt_raw_sha256 = [string]$launchAuthorityEvidence.raw_sha256
        authority_receipt_byte_count = [long]$launchAuthorityEvidence.byte_count
        authority_receipt_strict_utf8 = [bool]$launchAuthorityEvidence.strict_utf8
        authority_event_count = [long]$launchAuthorityEvidence.event_count
        authority_final_hmac_sha256 = [string]$launchAuthorityEvidence.final_hmac_sha256
        authority_private_nonce = $null
        authority_sink_launch_owned = $true
        authority_job_identity_bound = $true
        authority_pipe_identity_bound = $true
        authority_current_os_observation_bound = $true
        tunnel_client_process_id = [long]$runResult.process_id
        tunnel_client_process_creation_time = [string]$runResult.process_creation_time
        tunnel_client_process_creation_time_source = [string]$runResult.process_creation_time_source
        tunnel_client_process_executable_path = [string]$runResult.process_executable_path
        tunnel_client_process_executable_sha256 = [string]$profileEvidence.tunnel_client_sha256
    }) | ConvertTo-Json -Compress)
}
else {
    $startInfo = New-Task038TunnelChildStartInfo -EnvironmentValues ([ordered]@{})
    $shortCommandResult = Invoke-Task038OwnedTunnelClient `
        -TunnelClient $tunnelClient `
        -Arguments $arguments `
        -WorkingDirectory $profileRoot `
        -StartInfo $startInfo
    $clientExitCode = [int]$shortCommandResult.exit_code
}
if ($clientExitCode -ne 0) {
    throw ('TASK038_TUNNEL_CLIENT_FAILED_' + $Mode.ToUpperInvariant())
}
