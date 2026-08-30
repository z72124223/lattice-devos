#requires -Version 7.0

function Initialize-Phase4OwnedProcessInterop {
    if ($null -ne ('Lattice.Phase4.JobOwnedProcess' -as [type])) { return }

    Add-Type -Language CSharp -ErrorAction Stop -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.Win32.SafeHandles;

namespace Lattice.Phase4
{
    public sealed class JobOwnedProcess : IDisposable
    {
        private const UInt32 CREATE_SUSPENDED = 0x00000004;
        private const UInt32 CREATE_UNICODE_ENVIRONMENT = 0x00000400;
        private const UInt32 EXTENDED_STARTUPINFO_PRESENT = 0x00080000;
        private const UInt32 CREATE_NO_WINDOW = 0x08000000;
        private const UInt32 STARTF_USESTDHANDLES = 0x00000100;
        private const UInt32 HANDLE_FLAG_INHERIT = 0x00000001;
        private const UInt32 JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
        private const UInt32 JOB_OBJECT_EXTENDED_LIMIT_INFORMATION = 9;
        private const UInt32 JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION = 1;
        private const UInt32 WAIT_OBJECT_0 = 0;
        private const UInt32 WAIT_TIMEOUT = 258;
        private const UInt32 WAIT_FAILED = 0xffffffff;
        private const UInt32 PROC_THREAD_ATTRIBUTE_HANDLE_LIST = 0x00020002;

        [StructLayout(LayoutKind.Sequential)]
        private struct SecurityAttributes
        {
            public Int32 Length;
            public IntPtr SecurityDescriptor;
            [MarshalAs(UnmanagedType.Bool)] public bool InheritHandle;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct StartupInfo
        {
            public UInt32 Size;
            public IntPtr Reserved;
            public IntPtr Desktop;
            public IntPtr Title;
            public UInt32 X;
            public UInt32 Y;
            public UInt32 XSize;
            public UInt32 YSize;
            public UInt32 XCountChars;
            public UInt32 YCountChars;
            public UInt32 FillAttribute;
            public UInt32 Flags;
            public UInt16 ShowWindow;
            public UInt16 Reserved2;
            public IntPtr Reserved2Pointer;
            public IntPtr StandardInput;
            public IntPtr StandardOutput;
            public IntPtr StandardError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct StartupInfoEx
        {
            public StartupInfo StartupInfo;
            public IntPtr AttributeList;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct ProcessInformation
        {
            public IntPtr Process;
            public IntPtr Thread;
            public UInt32 ProcessId;
            public UInt32 ThreadId;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct FileTime
        {
            public UInt32 Low;
            public UInt32 High;
            public UInt64 Value { get { return ((UInt64)High << 32) | Low; } }
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

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CreatePipe(
            out IntPtr readPipe,
            out IntPtr writePipe,
            ref SecurityAttributes pipeAttributes,
            UInt32 size);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SetHandleInformation(IntPtr handle, UInt32 mask, UInt32 flags);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr CreateJobObject(IntPtr attributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool SetInformationJobObject(
            IntPtr job,
            UInt32 informationClass,
            ref ExtendedLimitInformation information,
            UInt32 informationLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool QueryInformationJobObject(
            IntPtr job,
            UInt32 informationClass,
            out BasicAccountingInformation information,
            UInt32 informationLength,
            IntPtr returnLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool InitializeProcThreadAttributeList(
            IntPtr attributeList,
            Int32 attributeCount,
            UInt32 flags,
            ref IntPtr size);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool UpdateProcThreadAttribute(
            IntPtr attributeList,
            UInt32 flags,
            IntPtr attribute,
            IntPtr value,
            IntPtr size,
            IntPtr previousValue,
            IntPtr returnSize);

        [DllImport("kernel32.dll")]
        private static extern void DeleteProcThreadAttributeList(IntPtr attributeList);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CreateProcessW(
            string applicationName,
            StringBuilder commandLine,
            IntPtr processAttributes,
            IntPtr threadAttributes,
            [MarshalAs(UnmanagedType.Bool)] bool inheritHandles,
            UInt32 creationFlags,
            IntPtr environment,
            string currentDirectory,
            ref StartupInfoEx startupInfo,
            out ProcessInformation processInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool IsProcessInJob(
            IntPtr process,
            IntPtr job,
            [MarshalAs(UnmanagedType.Bool)] out bool result);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern UInt32 ResumeThread(IntPtr thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool TerminateProcess(IntPtr process, UInt32 exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool TerminateJobObject(IntPtr job, UInt32 exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        private static extern UInt32 WaitForSingleObject(IntPtr handle, UInt32 milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetExitCodeProcess(IntPtr process, out UInt32 exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GetProcessTimes(
            IntPtr process,
            out FileTime creation,
            out FileTime exit,
            out FileTime kernel,
            out FileTime user);

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool QueryFullProcessImageName(
            IntPtr process,
            UInt32 flags,
            StringBuilder imagePath,
            ref UInt32 size);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool CloseHandle(IntPtr handle);

        private IntPtr jobHandle;
        private IntPtr rootProcessHandle;
        private bool jobClosed;
        private bool disposed;
        private readonly Int32 processId;
        private readonly UInt64 creationFileTime;
        private readonly string imagePath;
        private readonly object jobGate = new object();

        private JobOwnedProcess(
            IntPtr job,
            IntPtr process,
            Int32 id,
            UInt64 creation,
            string image,
            StreamWriter standardInput,
            StreamReader standardOutput,
            StreamReader standardError)
        {
            jobHandle = job;
            rootProcessHandle = process;
            processId = id;
            creationFileTime = creation;
            imagePath = image;
            StandardInput = standardInput;
            StandardOutput = standardOutput;
            StandardError = standardError;
        }

        public StreamWriter StandardInput { get; private set; }
        public StreamReader StandardOutput { get; private set; }
        public StreamReader StandardError { get; private set; }
        public Int32 Id { get { return processId; } }
        public string Path { get { return imagePath; } }
        public DateTime StartTime { get { return DateTime.FromFileTimeUtc((Int64)creationFileTime).ToLocalTime(); } }
        public bool HasExited { get { return WaitForExit(0); } }
        public Int32 ExitCode
        {
            get
            {
                if (!HasExited) throw new InvalidOperationException("PHASE4_OWNED_PROCESS_STILL_ACTIVE");
                UInt32 code;
                if (!GetExitCodeProcess(rootProcessHandle, out code))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_EXIT_CODE_REJECTED");
                return unchecked((Int32)code);
            }
        }

        public bool ContainsProcessHandle(IntPtr processHandle)
        {
            lock (jobGate)
            {
                if (processHandle == IntPtr.Zero || jobClosed || jobHandle == IntPtr.Zero) return false;
                bool result;
                if (!IsProcessInJob(processHandle, jobHandle, out result))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_JOB_MEMBERSHIP_REJECTED");
                return result;
            }
        }

        public bool WriteStandardInput(string value, bool appendLine, Int32 timeoutMilliseconds)
        {
            if (value == null || timeoutMilliseconds < 1)
                throw new ArgumentException("PHASE4_OWNED_PROCESS_INPUT_REJECTED");
            Task write = appendLine ? StandardInput.WriteLineAsync(value) : StandardInput.WriteAsync(value);
            if (!write.Wait(timeoutMilliseconds))
            {
                TerminateForOutputLimit();
                try { write.Wait(15000); } catch { }
                return false;
            }
            write.GetAwaiter().GetResult();
            Task flush = StandardInput.FlushAsync();
            if (!flush.Wait(timeoutMilliseconds))
            {
                TerminateForOutputLimit();
                try { flush.Wait(15000); } catch { }
                return false;
            }
            flush.GetAwaiter().GetResult();
            return true;
        }

        private void TerminateForOutputLimit()
        {
            lock (jobGate)
            {
                if (!jobClosed && jobHandle != IntPtr.Zero && ActiveProcessCountUnlocked() > 0)
                    TerminateJobObject(jobHandle, 1);
            }
        }

        private async Task<string> ReadToEndBounded(StreamReader reader, Int32 maxUtf8Bytes)
        {
            if (reader == null || maxUtf8Bytes < 1)
                throw new ArgumentException("PHASE4_OWNED_PROCESS_OUTPUT_LIMIT_REJECTED");
            var result = new StringBuilder(Math.Min(maxUtf8Bytes, 8192));
            var encoder = new UTF8Encoding(false, true).GetEncoder();
            var buffer = new char[4096];
            Int64 byteCount = 0;
            while (true)
            {
                Int32 count = await reader.ReadAsync(buffer, 0, buffer.Length).ConfigureAwait(false);
                if (count == 0) break;
                byteCount += encoder.GetByteCount(buffer, 0, count, false);
                if (byteCount > maxUtf8Bytes)
                {
                    TerminateForOutputLimit();
                    throw new InvalidDataException("PHASE4_OWNED_PROCESS_OUTPUT_LIMIT_REJECTED");
                }
                result.Append(buffer, 0, count);
            }
            byteCount += encoder.GetByteCount(Array.Empty<char>(), 0, 0, true);
            if (byteCount > maxUtf8Bytes)
            {
                TerminateForOutputLimit();
                throw new InvalidDataException("PHASE4_OWNED_PROCESS_OUTPUT_LIMIT_REJECTED");
            }
            return result.ToString();
        }

        private async Task<string> ReadLineBounded(StreamReader reader, Int32 maxUtf8Bytes)
        {
            if (reader == null || maxUtf8Bytes < 1)
                throw new ArgumentException("PHASE4_OWNED_PROCESS_OUTPUT_LIMIT_REJECTED");
            var result = new StringBuilder(Math.Min(maxUtf8Bytes, 8192));
            var encoder = new UTF8Encoding(false, true).GetEncoder();
            var buffer = new char[1];
            Int64 byteCount = 0;
            while (true)
            {
                Int32 count = await reader.ReadAsync(buffer, 0, 1).ConfigureAwait(false);
                if (count == 0)
                {
                    if (result.Length == 0) return null;
                    break;
                }
                if (buffer[0] == '\n') break;
                if (buffer[0] == '\r') continue;
                byteCount += encoder.GetByteCount(buffer, 0, 1, false);
                if (byteCount > maxUtf8Bytes)
                {
                    TerminateForOutputLimit();
                    throw new InvalidDataException("PHASE4_OWNED_PROCESS_OUTPUT_LIMIT_REJECTED");
                }
                result.Append(buffer[0]);
            }
            byteCount += encoder.GetByteCount(Array.Empty<char>(), 0, 0, true);
            if (byteCount > maxUtf8Bytes)
            {
                TerminateForOutputLimit();
                throw new InvalidDataException("PHASE4_OWNED_PROCESS_OUTPUT_LIMIT_REJECTED");
            }
            return result.ToString();
        }

        public Task<string> ReadStandardOutputToEndBounded(Int32 maxUtf8Bytes)
        {
            return ReadToEndBounded(StandardOutput, maxUtf8Bytes);
        }

        public Task<string> ReadStandardErrorToEndBounded(Int32 maxUtf8Bytes)
        {
            return ReadToEndBounded(StandardError, maxUtf8Bytes);
        }

        public Task<string> ReadStandardOutputLineBounded(Int32 maxUtf8Bytes)
        {
            return ReadLineBounded(StandardOutput, maxUtf8Bytes);
        }

        private static void CloseIfPresent(ref IntPtr handle)
        {
            if (handle == IntPtr.Zero) return;
            CloseHandle(handle);
            handle = IntPtr.Zero;
        }

        private static string QuoteArgument(string value)
        {
            if (value == null || value.IndexOf('\0') >= 0)
                throw new ArgumentException("PHASE4_OWNED_PROCESS_ARGUMENT_REJECTED");
            if (value.Length == 0) return "\"\"";
            bool quote = value.IndexOfAny(new[] { ' ', '\t', '\n', '\v', '"' }) >= 0;
            if (!quote) return value;
            var result = new StringBuilder("\"");
            Int32 backslashes = 0;
            foreach (char character in value)
            {
                if (character == '\\')
                {
                    backslashes += 1;
                    continue;
                }
                if (character == '"')
                {
                    result.Append('\\', backslashes * 2 + 1);
                    result.Append('"');
                    backslashes = 0;
                    continue;
                }
                result.Append('\\', backslashes);
                backslashes = 0;
                result.Append(character);
            }
            result.Append('\\', backslashes * 2);
            result.Append('"');
            return result.ToString();
        }

        private static IntPtr BuildEnvironmentBlock(string[] entries)
        {
            if (entries == null || entries.Length > 256)
                throw new ArgumentException("PHASE4_OWNED_PROCESS_ENVIRONMENT_REJECTED");
            var values = new List<KeyValuePair<string, string>>();
            Int32 total = 1;
            foreach (string entry in entries)
            {
                Int32 separator = entry == null ? -1 : entry.IndexOf('=');
                if (separator <= 0 || entry.IndexOf('\0') >= 0)
                    throw new ArgumentException("PHASE4_OWNED_PROCESS_ENVIRONMENT_REJECTED");
                string name = entry.Substring(0, separator);
                string value = entry.Substring(separator + 1);
                if (name.IndexOf('=') >= 0)
                    throw new ArgumentException("PHASE4_OWNED_PROCESS_ENVIRONMENT_REJECTED");
                values.Add(new KeyValuePair<string, string>(name, value));
                total += entry.Length + 1;
            }
            values.Sort((left, right) => StringComparer.OrdinalIgnoreCase.Compare(left.Key, right.Key));
            for (Int32 index = 1; index < values.Count; index += 1)
            {
                if (StringComparer.OrdinalIgnoreCase.Equals(values[index - 1].Key, values[index].Key))
                    throw new ArgumentException("PHASE4_OWNED_PROCESS_ENVIRONMENT_REJECTED");
            }
            if (total > 32767)
                throw new ArgumentException("PHASE4_OWNED_PROCESS_ENVIRONMENT_REJECTED");
            var block = new StringBuilder(total);
            foreach (KeyValuePair<string, string> value in values)
            {
                block.Append(value.Key).Append('=').Append(value.Value).Append('\0');
            }
            block.Append('\0');
            return Marshal.StringToHGlobalUni(block.ToString());
        }

        public static JobOwnedProcess Start(
            string executable,
            string[] arguments,
            string workingDirectory,
            string[] environmentEntries,
            Int32 outputCodePage)
        {
            if (String.IsNullOrWhiteSpace(executable) || executable.IndexOf('\0') >= 0 ||
                String.IsNullOrWhiteSpace(workingDirectory) || workingDirectory.IndexOf('\0') >= 0 ||
                arguments == null || arguments.Length > 4096 ||
                outputCodePage <= 0 || outputCodePage > 65535)
            {
                throw new ArgumentException("PHASE4_OWNED_PROCESS_START_REJECTED");
            }

            IntPtr childStdinRead = IntPtr.Zero;
            IntPtr parentStdinWrite = IntPtr.Zero;
            IntPtr parentStdoutRead = IntPtr.Zero;
            IntPtr childStdoutWrite = IntPtr.Zero;
            IntPtr parentStderrRead = IntPtr.Zero;
            IntPtr childStderrWrite = IntPtr.Zero;
            IntPtr environmentBlock = IntPtr.Zero;
            IntPtr attributeList = IntPtr.Zero;
            IntPtr inheritedHandles = IntPtr.Zero;
            IntPtr job = IntPtr.Zero;
            var created = new ProcessInformation();
            bool processCreated = false;
            StreamWriter standardInput = null;
            StreamReader standardOutput = null;
            StreamReader standardError = null;
            try
            {
                var attributes = new SecurityAttributes {
                    Length = Marshal.SizeOf(typeof(SecurityAttributes)),
                    SecurityDescriptor = IntPtr.Zero,
                    InheritHandle = true
                };
                if (!CreatePipe(out childStdinRead, out parentStdinWrite, ref attributes, 0) ||
                    !SetHandleInformation(parentStdinWrite, HANDLE_FLAG_INHERIT, 0) ||
                    !CreatePipe(out parentStdoutRead, out childStdoutWrite, ref attributes, 0) ||
                    !SetHandleInformation(parentStdoutRead, HANDLE_FLAG_INHERIT, 0) ||
                    !CreatePipe(out parentStderrRead, out childStderrWrite, ref attributes, 0) ||
                    !SetHandleInformation(parentStderrRead, HANDLE_FLAG_INHERIT, 0))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_PIPE_REJECTED");
                }

                job = CreateJobObject(IntPtr.Zero, null);
                if (job == IntPtr.Zero)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_JOB_REJECTED");
                var limits = new ExtendedLimitInformation();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if (!SetInformationJobObject(
                    job,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    ref limits,
                    (UInt32)Marshal.SizeOf(typeof(ExtendedLimitInformation))))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_JOB_REJECTED");
                }

                IntPtr attributeBytes = IntPtr.Zero;
                InitializeProcThreadAttributeList(IntPtr.Zero, 1, 0, ref attributeBytes);
                if (attributeBytes == IntPtr.Zero)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_HANDLE_LIST_REJECTED");
                attributeList = Marshal.AllocHGlobal(attributeBytes);
                if (!InitializeProcThreadAttributeList(attributeList, 1, 0, ref attributeBytes))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_HANDLE_LIST_REJECTED");
                inheritedHandles = Marshal.AllocHGlobal(IntPtr.Size * 3);
                Marshal.WriteIntPtr(inheritedHandles, 0, childStdinRead);
                Marshal.WriteIntPtr(inheritedHandles, IntPtr.Size, childStdoutWrite);
                Marshal.WriteIntPtr(inheritedHandles, IntPtr.Size * 2, childStderrWrite);
                if (!UpdateProcThreadAttribute(
                    attributeList,
                    0,
                    new IntPtr(PROC_THREAD_ATTRIBUTE_HANDLE_LIST),
                    inheritedHandles,
                    new IntPtr(IntPtr.Size * 3),
                    IntPtr.Zero,
                    IntPtr.Zero))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_HANDLE_LIST_REJECTED");
                }

                environmentBlock = BuildEnvironmentBlock(environmentEntries);
                var commandLine = new StringBuilder(QuoteArgument(executable));
                foreach (string argument in arguments)
                    commandLine.Append(' ').Append(QuoteArgument(argument));
                if (commandLine.Length > 32766)
                    throw new ArgumentException("PHASE4_OWNED_PROCESS_ARGUMENT_REJECTED");

                var startup = new StartupInfoEx();
                startup.StartupInfo.Size = (UInt32)Marshal.SizeOf(typeof(StartupInfoEx));
                startup.StartupInfo.Flags = STARTF_USESTDHANDLES;
                startup.StartupInfo.StandardInput = childStdinRead;
                startup.StartupInfo.StandardOutput = childStdoutWrite;
                startup.StartupInfo.StandardError = childStderrWrite;
                startup.AttributeList = attributeList;
                if (!CreateProcessW(
                    executable,
                    commandLine,
                    IntPtr.Zero,
                    IntPtr.Zero,
                    true,
                    CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT |
                        EXTENDED_STARTUPINFO_PRESENT,
                    environmentBlock,
                    workingDirectory,
                    ref startup,
                    out created))
                {
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_START_REJECTED");
                }
                processCreated = true;
                if (!AssignProcessToJobObject(job, created.Process))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_JOB_ASSIGN_REJECTED");

                FileTime creation;
                FileTime exit;
                FileTime kernel;
                FileTime user;
                if (!GetProcessTimes(created.Process, out creation, out exit, out kernel, out user))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_IDENTITY_REJECTED");
                UInt32 imageCapacity = 32768;
                var image = new StringBuilder((Int32)imageCapacity);
                if (!QueryFullProcessImageName(created.Process, 0, image, ref imageCapacity))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_IDENTITY_REJECTED");

                CloseIfPresent(ref childStdinRead);
                CloseIfPresent(ref childStdoutWrite);
                CloseIfPresent(ref childStderrWrite);
                var strictUtf8 = new UTF8Encoding(false, true);
                var strictOutputEncoding = Encoding.GetEncoding(
                    outputCodePage,
                    EncoderFallback.ExceptionFallback,
                    DecoderFallback.ExceptionFallback);
                var inputHandle = new SafeFileHandle(parentStdinWrite, true);
                parentStdinWrite = IntPtr.Zero;
                FileStream inputStream = null;
                try
                {
                    inputStream = new FileStream(inputHandle, FileAccess.Write, 4096, false);
                    standardInput = new StreamWriter(inputStream, strictUtf8);
                    inputStream = null;
                }
                finally
                {
                    if (inputStream != null) inputStream.Dispose();
                    else if (standardInput == null) inputHandle.Dispose();
                }
                standardInput.AutoFlush = true;
                var outputHandle = new SafeFileHandle(parentStdoutRead, true);
                parentStdoutRead = IntPtr.Zero;
                FileStream outputStream = null;
                try
                {
                    outputStream = new FileStream(outputHandle, FileAccess.Read, 4096, false);
                    standardOutput = new StreamReader(
                        outputStream, strictOutputEncoding, false, 4096, false);
                    outputStream = null;
                }
                finally
                {
                    if (outputStream != null) outputStream.Dispose();
                    else if (standardOutput == null) outputHandle.Dispose();
                }
                var errorHandle = new SafeFileHandle(parentStderrRead, true);
                parentStderrRead = IntPtr.Zero;
                FileStream errorStream = null;
                try
                {
                    errorStream = new FileStream(errorHandle, FileAccess.Read, 4096, false);
                    standardError = new StreamReader(
                        errorStream, strictOutputEncoding, false, 4096, false);
                    errorStream = null;
                }
                finally
                {
                    if (errorStream != null) errorStream.Dispose();
                    else if (standardError == null) errorHandle.Dispose();
                }

                if (ResumeThread(created.Thread) != 1)
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_RESUME_REJECTED");
                CloseIfPresent(ref created.Thread);
                var result = new JobOwnedProcess(
                    job,
                    created.Process,
                    (Int32)created.ProcessId,
                    creation.Value,
                    image.ToString(),
                    standardInput,
                    standardOutput,
                    standardError);
                standardInput = null;
                standardOutput = null;
                standardError = null;
                job = IntPtr.Zero;
                created.Process = IntPtr.Zero;
                return result;
            }
            catch
            {
                if (processCreated && created.Process != IntPtr.Zero)
                    TerminateProcess(created.Process, 1);
                if (standardInput != null) standardInput.Dispose();
                if (standardOutput != null) standardOutput.Dispose();
                if (standardError != null) standardError.Dispose();
                throw;
            }
            finally
            {
                if (attributeList != IntPtr.Zero)
                {
                    DeleteProcThreadAttributeList(attributeList);
                    Marshal.FreeHGlobal(attributeList);
                }
                if (inheritedHandles != IntPtr.Zero) Marshal.FreeHGlobal(inheritedHandles);
                if (environmentBlock != IntPtr.Zero) Marshal.FreeHGlobal(environmentBlock);
                CloseIfPresent(ref childStdinRead);
                CloseIfPresent(ref parentStdinWrite);
                CloseIfPresent(ref parentStdoutRead);
                CloseIfPresent(ref childStdoutWrite);
                CloseIfPresent(ref parentStderrRead);
                CloseIfPresent(ref childStderrWrite);
                CloseIfPresent(ref created.Thread);
                CloseIfPresent(ref created.Process);
                CloseIfPresent(ref job);
            }
        }

        public bool WaitForExit(Int32 milliseconds)
        {
            if (milliseconds < 0) throw new ArgumentOutOfRangeException("milliseconds");
            UInt32 result = WaitForSingleObject(rootProcessHandle, (UInt32)milliseconds);
            if (result == WAIT_OBJECT_0) return true;
            if (result == WAIT_TIMEOUT) return false;
            if (result == WAIT_FAILED)
                throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_WAIT_REJECTED");
            throw new InvalidOperationException("PHASE4_OWNED_PROCESS_WAIT_REJECTED");
        }

        public void Refresh() { }

        private UInt32 ActiveProcessCountUnlocked()
        {
            if (jobClosed || jobHandle == IntPtr.Zero) return 0;
            BasicAccountingInformation information;
            if (!QueryInformationJobObject(
                jobHandle,
                JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION,
                out information,
                (UInt32)Marshal.SizeOf(typeof(BasicAccountingInformation)),
                IntPtr.Zero))
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(), "PHASE4_OWNED_PROCESS_JOB_QUERY_REJECTED");
            }
            return information.ActiveProcesses;
        }

        public UInt32 ActiveProcessCount()
        {
            lock (jobGate) { return ActiveProcessCountUnlocked(); }
        }

        public bool WaitForEmpty(Int32 milliseconds)
        {
            if (milliseconds < 0) throw new ArgumentOutOfRangeException("milliseconds");
            var watch = Stopwatch.StartNew();
            do
            {
                if (ActiveProcessCount() == 0) return true;
                Thread.Sleep(10);
            }
            while (watch.ElapsedMilliseconds < milliseconds);
            return ActiveProcessCount() == 0;
        }

        public bool TerminateAndWait(Int32 milliseconds)
        {
            if (milliseconds < 0) throw new ArgumentOutOfRangeException("milliseconds");
            lock (jobGate)
            {
                if (jobClosed || jobHandle == IntPtr.Zero) return true;
                if (ActiveProcessCountUnlocked() > 0 && !TerminateJobObject(jobHandle, 1)) return false;
            }
            return WaitForEmpty(milliseconds);
        }

        public bool CloseWhenEmpty()
        {
            lock (jobGate)
            {
                if (jobClosed) return true;
                if (ActiveProcessCountUnlocked() != 0) return false;
                if (!CloseHandle(jobHandle)) return false;
                jobHandle = IntPtr.Zero;
                jobClosed = true;
                return true;
            }
        }

        public void Kill(bool entireProcessTree)
        {
            if (!TerminateAndWait(15000))
                throw new InvalidOperationException("PHASE4_OWNED_PROCESS_TERMINATION_REJECTED");
        }

        public void Dispose()
        {
            if (disposed) return;
            try
            {
                TerminateAndWait(15000);
            }
            finally
            {
                lock (jobGate)
                {
                    if (jobHandle != IntPtr.Zero) CloseHandle(jobHandle);
                    jobHandle = IntPtr.Zero;
                    jobClosed = true;
                }
                if (rootProcessHandle != IntPtr.Zero) CloseHandle(rootProcessHandle);
                rootProcessHandle = IntPtr.Zero;
                if (StandardInput != null) StandardInput.Dispose();
                if (StandardOutput != null) StandardOutput.Dispose();
                if (StandardError != null) StandardError.Dispose();
                StandardInput = null;
                StandardOutput = null;
                StandardError = null;
                disposed = true;
            }
        }
    }
}
'@
}

function Start-Phase4OwnedProcessJob {
    param(
        [Parameter(Mandatory = $true)][string]$Executable,
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][string[]]$Argument,
        [Parameter(Mandatory = $true)][Collections.IDictionary]$Environment,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory,
        [Parameter(Mandatory = $true)][string]$Failure,
        [ValidateRange(1, 65535)][int]$OutputEncodingCodePage = 65001
    )

    Initialize-Phase4OwnedProcessInterop
    $entries = [Collections.Generic.List[string]]::new()
    foreach ($entry in $Environment.GetEnumerator()) {
        $entries.Add(([string]$entry.Key + '=' + [string]$entry.Value))
    }
    try {
        return [Lattice.Phase4.JobOwnedProcess]::Start(
            $Executable,
            [string[]]$Argument,
            $WorkingDirectory,
            [string[]]$entries.ToArray(),
            $OutputEncodingCodePage
        )
    }
    catch {
        throw $Failure
    }
}

function Stop-Phase4OwnedProcessJob {
    param(
        [Parameter(Mandatory = $true)]$OwnedProcess,
        [Parameter(Mandatory = $true)][string]$Failure,
        [ValidateRange(100, 30000)][int]$TimeoutMilliseconds = 15000
    )

    try {
        if (-not $OwnedProcess.TerminateAndWait($TimeoutMilliseconds) -or
            [long]$OwnedProcess.ActiveProcessCount() -ne 0) {
            throw $Failure
        }
    }
    catch { throw $Failure }
}

function Close-Phase4OwnedProcessJob {
    param(
        [Parameter(Mandatory = $true)]$OwnedProcess,
        [Parameter(Mandatory = $true)][string]$Failure,
        [ValidateRange(100, 30000)][int]$TimeoutMilliseconds = 5000
    )

    try {
        if (-not $OwnedProcess.WaitForEmpty($TimeoutMilliseconds)) {
            $null = Stop-Phase4OwnedProcessJob -OwnedProcess $OwnedProcess -Failure $Failure
            throw $Failure
        }
        if (-not $OwnedProcess.CloseWhenEmpty() -or
            [long]$OwnedProcess.ActiveProcessCount() -ne 0) {
            throw $Failure
        }
    }
    catch { throw $Failure }
}
