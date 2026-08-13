//! Windows-only owned Job Object launcher for the Hermes WSL boundary.
//!
//! The child is created suspended, assigned to a kill-on-close Job Object, and
//! only then resumed. This module owns the trusted Windows launcher lifecycle;
//! the untrusted Hermes process remains confined by the fixed WSL sandbox.

#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::FileExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_CREATE, FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT, NtCreateFile,
};
#[cfg(test)]
use windows_sys::Win32::Foundation::GetHandleInformation;
use windows_sys::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    OBJ_CASE_INSENSITIVE, SetHandleInformation, UNICODE_STRING, WAIT_FAILED, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfo, GetFileAttributesW,
    GetFileInformationByHandle, GetFinalPathNameByHandleW, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
    SYNCHRONIZE, SetFileInformationByHandle,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
    GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::{HermesAdapterError, HermesAdapterErrorKind, HermesAdapterResult};

const PROCESS_TEARDOWN_EXIT_CODE: u32 = 0xC0DE_0340;
const MAX_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const HANDLE_DELETE_RETRY_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) struct WindowsPinnedDirectory {
    handle: Option<OwnedHandle>,
    final_path: PathBuf,
    delete_on_drop: bool,
}

impl WindowsPinnedDirectory {
    pub(crate) fn create_new(parent: &Self, leaf: &OsStr) -> HermesAdapterResult<Self> {
        let parent_handle = parent
            .handle
            .as_ref()
            .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_DIRECTORY_HANDLE_REJECTED"))?;
        let mut name = leaf.encode_wide().collect::<Vec<_>>();
        let name_bytes = name
            .len()
            .checked_mul(size_of::<u16>())
            .and_then(|length| u16::try_from(length).ok())
            .filter(|_| !name.is_empty() && !name.contains(&0))
            .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_DIRECTORY_NAME_REJECTED"))?;
        let object_name = UNICODE_STRING {
            Length: name_bytes,
            MaximumLength: name_bytes,
            Buffer: name.as_mut_ptr(),
        };
        let object_attributes = OBJECT_ATTRIBUTES {
            Length: u32::try_from(size_of::<OBJECT_ATTRIBUTES>())
                .map_err(|_| ambiguous_error("HERMES_WINDOWS_DIRECTORY_HANDLE_REJECTED"))?,
            RootDirectory: parent_handle.raw(),
            ObjectName: &raw const object_name,
            Attributes: OBJ_CASE_INSENSITIVE,
            SecurityDescriptor: null(),
            SecurityQualityOfService: null(),
        };
        let mut io_status = IO_STATUS_BLOCK::default();
        let mut raw = null_mut();
        // SAFETY: every pointer references a live fixed-size value for this
        // call. RootDirectory pins the verified parent, FILE_CREATE makes the
        // leaf exclusive. A provisional RAII guard below owns every
        // pre-return failure without relying on irrevocable delete-on-close.
        let status = unsafe {
            NtCreateFile(
                &raw mut raw,
                FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE,
                &raw const object_attributes,
                &raw mut io_status,
                null(),
                FILE_ATTRIBUTE_NORMAL,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                FILE_CREATE,
                FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT,
                null(),
                0,
            )
        };
        if status < 0 {
            return Err(spawn_error("HERMES_WINDOWS_DIRECTORY_CREATE_REJECTED"));
        }
        let handle = owned_handle(raw)
            .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_DIRECTORY_HANDLE_REJECTED"))?;
        let mut directory = Self {
            handle: Some(handle),
            final_path: PathBuf::new(),
            delete_on_drop: true,
        };
        let validation = (|| {
            let handle = directory
                .handle
                .as_ref()
                .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_DIRECTORY_HANDLE_REJECTED"))?;
            let information = file_information(handle)?;
            if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                return Err(ambiguous_error("HERMES_WINDOWS_DIRECTORY_REPARSE_REJECTED"));
            }
            final_path_by_handle(handle)
        })();
        match validation {
            Ok(final_path) => {
                directory.final_path = final_path;
                directory.delete_on_drop = false;
                Ok(directory)
            }
            Err(failure) => match directory.delete() {
                Ok(()) => Err(failure),
                Err(cleanup) => Err(cleanup),
            },
        }
    }

    pub(crate) fn open(
        path: &Path,
        exclusive_writes: bool,
        delete_access: bool,
        share_delete: bool,
    ) -> HermesAdapterResult<Self> {
        let path_wide = wide_null(path.as_os_str())?;
        let mut share_mode = if exclusive_writes {
            FILE_SHARE_READ
        } else {
            FILE_SHARE_READ | FILE_SHARE_WRITE
        };
        if share_delete {
            share_mode |= FILE_SHARE_DELETE;
        }
        // SAFETY: `path_wide` remains live for this call. OPEN_REPARSE_POINT
        // makes the returned handle identify the named object rather than a
        // reparse target; BACKUP_SEMANTICS is required for directories.
        let desired_access =
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | if delete_access { DELETE } else { 0 };
        let raw = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                desired_access,
                share_mode,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        let handle = owned_handle(raw)
            .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_DIRECTORY_HANDLE_REJECTED"))?;
        let information = file_information(&handle)?;
        if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(ambiguous_error("HERMES_WINDOWS_DIRECTORY_REPARSE_REJECTED"));
        }
        let final_path = final_path_by_handle(&handle)?;
        Ok(Self {
            handle: Some(handle),
            final_path,
            delete_on_drop: false,
        })
    }

    pub(crate) fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub(crate) fn delete(mut self) -> HermesAdapterResult<()> {
        let handle = self
            .handle
            .as_ref()
            .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_DIRECTORY_HANDLE_REJECTED"))?;
        mark_handle_for_deletion(handle.raw())?;
        self.delete_on_drop = false;
        drop(self.handle.take());
        Ok(())
    }
}

impl Drop for WindowsPinnedDirectory {
    fn drop(&mut self) {
        if self.delete_on_drop {
            if let Some(handle) = &self.handle {
                let _ = mark_handle_for_deletion(handle.raw());
            }
        }
    }
}

pub(crate) struct WindowsPinnedFile {
    file: Option<File>,
}

impl WindowsPinnedFile {
    pub(crate) fn create_new(path: &Path, share_delete: bool) -> HermesAdapterResult<Self> {
        let path_wide = wide_null(path.as_os_str())?;
        let mut share_mode = FILE_SHARE_READ;
        if share_delete {
            share_mode |= FILE_SHARE_DELETE;
        }
        // SAFETY: `path_wide` remains live for this call. CREATE_NEW prevents
        // adopting a foreign file, and OPEN_REPARSE_POINT prevents traversal.
        let raw = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE | DELETE,
                share_mode,
                null(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE || raw.is_null() {
            return Err(spawn_error("HERMES_WINDOWS_OWNED_FILE_CREATE_REJECTED"));
        }
        // SAFETY: successful CreateFileW transfers one unique file handle.
        let file = unsafe { File::from_raw_handle(raw.cast()) };
        Ok(Self { file: Some(file) })
    }

    pub(crate) fn write_all_sync(&mut self, bytes: &[u8]) -> HermesAdapterResult<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_OWNED_FILE_HANDLE_REJECTED"))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| spawn_error("HERMES_WINDOWS_OWNED_FILE_WRITE_REJECTED"))
    }

    pub(crate) fn delete(mut self) -> HermesAdapterResult<()> {
        let file = self
            .file
            .take()
            .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_OWNED_FILE_HANDLE_REJECTED"))?;
        mark_handle_for_deletion(file.as_raw_handle().cast())?;
        drop(file);
        Ok(())
    }

    pub(crate) fn close(mut self) {
        drop(self.file.take());
    }
}

pub(crate) struct WindowsJobCommandPlan {
    pub(crate) executable: PathBuf,
    pub(crate) arguments: Vec<OsString>,
    pub(crate) current_dir: PathBuf,
    pub(crate) environment: BTreeMap<OsString, OsString>,
    pub(crate) run_root: PathBuf,
    pub(crate) stdout_path: PathBuf,
    pub(crate) stderr_path: PathBuf,
    pub(crate) stdout_limit: u64,
    pub(crate) stderr_limit: u64,
    pub(crate) deadline: Instant,
    pub(crate) teardown_timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WindowsJobProcessExit {
    pub(crate) exit_code: u32,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// One live process tree assigned before first instruction to a private
/// kill-on-close Job Object.
pub(crate) struct WindowsJobChild {
    job: OwnedHandle,
    process: OwnedHandle,
    process_id: u32,
    stdin: Option<File>,
    stdout: WindowsJobStdout,
    stderr: WindowsJobStderr,
    deadline: Instant,
    teardown_timeout: Duration,
    terminated: bool,
}

enum WindowsJobStdout {
    Capture(File),
    Pipe(Option<File>),
}

enum WindowsJobStderr {
    Capture(File),
    Pipe(Option<File>),
}

pub(crate) fn run(plan: &WindowsJobCommandPlan) -> HermesAdapterResult<WindowsJobProcessExit> {
    let mut child = spawn(plan)?;
    let exit_code = child.wait_for_exit()?;
    let stdout = child.read_stdout(plan.stdout_limit)?;
    let stderr = child.read_stderr(plan.stderr_limit)?;
    Ok(WindowsJobProcessExit {
        exit_code,
        stdout,
        stderr,
    })
}

/// Starts a long-lived owned child without releasing its Job Object.
pub(crate) fn spawn(plan: &WindowsJobCommandPlan) -> HermesAdapterResult<WindowsJobChild> {
    spawn_inner(plan, false, false, false)
}

/// Starts a Job-owned child with parent-only stdin/stdout pipe ends while
/// retaining the normal bounded stderr capture.
pub(crate) fn spawn_with_parent_stdio(
    plan: &WindowsJobCommandPlan,
) -> HermesAdapterResult<WindowsJobChild> {
    spawn_inner(plan, true, true, false)
}

/// Starts a Job-owned child with parent-only pipe ends for all three standard
/// streams. The caller must drain stderr concurrently into bounded evidence.
pub(crate) fn spawn_duplex(plan: &WindowsJobCommandPlan) -> HermesAdapterResult<WindowsJobChild> {
    spawn_inner(plan, true, true, true)
}

fn spawn_inner(
    plan: &WindowsJobCommandPlan,
    pipe_stdin: bool,
    pipe_stdout: bool,
    pipe_stderr: bool,
) -> HermesAdapterResult<WindowsJobChild> {
    let paths = validate_plan(plan)?;
    let redirects = RedirectHandles::create(
        &paths.stdout_path,
        &paths.stderr_path,
        pipe_stdin,
        pipe_stdout,
        pipe_stderr,
    )?;
    let job = create_kill_on_close_job()?;
    let attributes = ProcThreadAttributes::for_handles(redirects.as_slice())?;
    let mut command_line = command_line(&paths.executable, &plan.arguments)?;
    let environment = environment_block(&plan.environment)?;
    let current_dir = wide_null(&paths.current_dir)?;
    let executable = wide_null(&paths.executable)?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>())
        .map_err(|_| spawn_error("HERMES_WINDOWS_STARTUP_INFO_SIZE"))?;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = redirects.child_stdin.raw();
    startup.StartupInfo.hStdOutput = redirects.child_stdout.raw();
    startup.StartupInfo.hStdError = redirects.child_stderr.raw();
    startup.lpAttributeList = attributes.raw();

    // SAFETY: all pointers reference live storage through this call. The
    // mutable command line and double-NUL environment block satisfy Win32's
    // process creation contracts, and the explicit handle list contains only
    // the three standard handles owned by `redirects`.
    let mut process_info: PROCESS_INFORMATION = unsafe { zeroed() };
    // SAFETY: see the pointer and handle invariants immediately above.
    let created = unsafe {
        CreateProcessW(
            executable.as_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            1,
            CREATE_NO_WINDOW
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            environment.as_ptr().cast(),
            current_dir.as_ptr(),
            &raw const startup.StartupInfo,
            &raw mut process_info,
        )
    };
    if created == 0 {
        return Err(spawn_error("HERMES_WINDOWS_CREATE_PROCESS"));
    }

    let process = owned_handle(process_info.hProcess)
        .ok_or_else(|| spawn_error("HERMES_WINDOWS_PROCESS_HANDLE"))?;
    let Some(thread_handle) = owned_handle(process_info.hThread) else {
        terminate_unassigned_process(&process, plan.teardown_timeout)?;
        return Err(spawn_error("HERMES_WINDOWS_THREAD_HANDLE"));
    };
    if redirects.clear_parent_capture_inheritance().is_err() {
        terminate_unassigned_process(&process, plan.teardown_timeout)?;
        return Err(spawn_error("HERMES_WINDOWS_CAPTURE_INHERITANCE_REJECTED"));
    }
    drop(attributes);

    // SAFETY: `job` and `process` are valid handles. The child is suspended,
    // so assignment occurs before its first instruction can execute.
    if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
        terminate_unassigned_process(&process, plan.teardown_timeout)?;
        return Err(spawn_error("HERMES_WINDOWS_JOB_ASSIGNMENT"));
    }
    if redirects.retain_capture_files().is_err() {
        terminate_job_and_reap(&job, &process, plan.teardown_timeout)?;
        redirects.delete_capture_files_best_effort();
        return Err(spawn_error("HERMES_WINDOWS_CAPTURE_RETENTION_REJECTED"));
    }

    // SAFETY: this is the still-suspended primary thread returned above.
    if unsafe { ResumeThread(thread_handle.raw()) } == u32::MAX {
        terminate_job_and_reap(&job, &process, plan.teardown_timeout)?;
        redirects.delete_capture_files_best_effort();
        return Err(spawn_error("HERMES_WINDOWS_RESUME_THREAD"));
    }
    drop(thread_handle);

    let RedirectHandles {
        child_stdin,
        child_stdout,
        parent_stdin,
        parent_stdout,
        parent_stderr,
        child_stderr,
    } = redirects;
    drop(child_stdin);
    let stdout = match parent_stdout {
        Some(reader) => {
            drop(child_stdout);
            WindowsJobStdout::Pipe(Some(reader))
        }
        None => WindowsJobStdout::Capture(File::from(child_stdout)),
    };
    let stderr = match parent_stderr {
        Some(reader) => {
            drop(child_stderr);
            WindowsJobStderr::Pipe(Some(reader))
        }
        None => WindowsJobStderr::Capture(File::from(child_stderr)),
    };
    Ok(WindowsJobChild {
        job,
        process,
        process_id: process_info.dwProcessId,
        stdin: parent_stdin,
        stdout,
        stderr,
        deadline: plan.deadline,
        teardown_timeout: plan.teardown_timeout,
        terminated: false,
    })
}

impl WindowsJobChild {
    pub(crate) const fn process_id(&self) -> u32 {
        self.process_id
    }

    pub(crate) fn ensure_running(&mut self) -> HermesAdapterResult<()> {
        if Instant::now() >= self.deadline {
            self.terminate()?;
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Timeout,
                "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
            ));
        }
        // SAFETY: the process handle remains owned by `self`; a zero timeout
        // only observes state and never blocks.
        match unsafe { WaitForSingleObject(self.process.raw(), 0) } {
            WAIT_TIMEOUT => match job_active_processes(&self.job) {
                Ok(active) if active > 0 => Ok(()),
                _ => {
                    self.terminate()?;
                    Err(ambiguous_error("HERMES_PRODUCTION_JOB_STATE_UNKNOWN"))
                }
            },
            WAIT_OBJECT_0 => {
                ensure_job_empty(&self.job, &self.process, self.teardown_timeout)?;
                self.terminated = true;
                Err(HermesAdapterError::new(
                    HermesAdapterErrorKind::Failed,
                    "HERMES_PRODUCTION_CHILD_EXITED",
                ))
            }
            _ => {
                self.terminate()?;
                Err(ambiguous_error("HERMES_PRODUCTION_CHILD_STATUS_UNKNOWN"))
            }
        }
    }

    pub(crate) fn read_stdout(&self, limit: u64) -> HermesAdapterResult<Vec<u8>> {
        match &self.stdout {
            WindowsJobStdout::Capture(stdout) => read_locked_capture(stdout, limit),
            WindowsJobStdout::Pipe(_) => Err(spawn_error("HERMES_WINDOWS_STDOUT_NOT_CAPTURED")),
        }
    }

    pub(crate) fn read_stderr(&self, limit: u64) -> HermesAdapterResult<Vec<u8>> {
        match &self.stderr {
            WindowsJobStderr::Capture(stderr) => read_locked_capture(stderr, limit),
            WindowsJobStderr::Pipe(_) => Err(spawn_error("HERMES_WINDOWS_STDERR_NOT_CAPTURED")),
        }
    }

    pub(crate) fn take_stdin_writer(&mut self) -> HermesAdapterResult<File> {
        self.stdin
            .take()
            .ok_or_else(|| spawn_error("HERMES_WINDOWS_STDIN_PIPE_UNAVAILABLE"))
    }

    pub(crate) fn take_stdout_reader(&mut self) -> HermesAdapterResult<File> {
        match &mut self.stdout {
            WindowsJobStdout::Pipe(reader) => reader
                .take()
                .ok_or_else(|| spawn_error("HERMES_WINDOWS_STDOUT_PIPE_UNAVAILABLE")),
            WindowsJobStdout::Capture(_) => {
                Err(spawn_error("HERMES_WINDOWS_STDOUT_PIPE_UNAVAILABLE"))
            }
        }
    }

    pub(crate) fn take_stderr_reader(&mut self) -> HermesAdapterResult<File> {
        match &mut self.stderr {
            WindowsJobStderr::Pipe(reader) => reader
                .take()
                .ok_or_else(|| spawn_error("HERMES_WINDOWS_STDERR_PIPE_UNAVAILABLE")),
            WindowsJobStderr::Capture(_) => {
                Err(spawn_error("HERMES_WINDOWS_STDERR_PIPE_UNAVAILABLE"))
            }
        }
    }

    pub(crate) fn terminate(&mut self) -> HermesAdapterResult<()> {
        if self.terminated {
            return Ok(());
        }
        terminate_job_and_reap(&self.job, &self.process, self.teardown_timeout)?;
        self.terminated = true;
        Ok(())
    }

    pub(crate) fn close_parent_stdio_and_delete_captures(&mut self) -> HermesAdapterResult<()> {
        self.stdin = None;
        let stdout = std::mem::replace(&mut self.stdout, WindowsJobStdout::Pipe(None));
        let stderr = std::mem::replace(&mut self.stderr, WindowsJobStderr::Pipe(None));
        let stdout_result = match stdout {
            WindowsJobStdout::Capture(file) => {
                let result = mark_handle_for_deletion(file.as_raw_handle().cast());
                drop(file);
                result
            }
            WindowsJobStdout::Pipe(reader) => {
                drop(reader);
                Ok(())
            }
        };
        let stderr_result = match stderr {
            WindowsJobStderr::Capture(file) => {
                let result = mark_handle_for_deletion(file.as_raw_handle().cast());
                drop(file);
                result
            }
            WindowsJobStderr::Pipe(reader) => {
                drop(reader);
                Ok(())
            }
        };
        match (stdout_result, stderr_result) {
            (Err(failure), _) => Err(failure),
            (Ok(()), result) => result,
        }
    }

    fn wait_for_exit(&mut self) -> HermesAdapterResult<u32> {
        let exit_code = wait_for_exit(
            self.deadline,
            self.teardown_timeout,
            &self.job,
            &self.process,
        )?;
        self.terminated = true;
        Ok(exit_code)
    }
}

impl Drop for WindowsJobChild {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = terminate_job_and_reap(&self.job, &self.process, self.teardown_timeout);
            self.terminated = true;
        }
    }
}

/// Confirms the broker helper was itself created inside an owned Job Object.
/// A helper launched directly is never allowed to spawn the official Codex
/// bundle because its descendants would not have a kill-on-close owner.
pub(crate) fn current_process_is_in_job() -> bool {
    let mut in_job = 0_i32;
    // SAFETY: the pseudo-handle is always valid for the current process and
    // `in_job` is writable storage for the documented BOOL result.
    unsafe { IsProcessInJob(GetCurrentProcess(), null_mut(), &raw mut in_job) != 0 && in_job != 0 }
}

struct ValidatedPaths {
    executable: PathBuf,
    current_dir: PathBuf,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

fn validate_plan(plan: &WindowsJobCommandPlan) -> HermesAdapterResult<ValidatedPaths> {
    if plan.deadline <= Instant::now()
        || plan.teardown_timeout.is_zero()
        || plan.teardown_timeout > MAX_TEARDOWN_TIMEOUT
    {
        return Err(spawn_error("HERMES_WINDOWS_INVALID_DEADLINE"));
    }

    let executable = std::fs::canonicalize(&plan.executable)
        .map_err(|_| spawn_error("HERMES_WINDOWS_EXECUTABLE_PATH"))?;
    if !executable.is_file() {
        return Err(spawn_error("HERMES_WINDOWS_EXECUTABLE_NOT_FILE"));
    }
    let run_root = std::fs::canonicalize(&plan.run_root)
        .map_err(|_| spawn_error("HERMES_WINDOWS_RUN_ROOT_PATH"))?;
    if !run_root.is_dir() || is_reparse_point(&run_root)? {
        return Err(spawn_error("HERMES_WINDOWS_RUN_ROOT_UNSAFE"));
    }
    let canonical_current_dir = std::fs::canonicalize(&plan.current_dir)
        .map_err(|_| spawn_error("HERMES_WINDOWS_CURRENT_DIR_PATH"))?;
    if !canonical_current_dir.is_dir() || !canonical_current_dir.starts_with(&run_root) {
        return Err(spawn_error("HERMES_WINDOWS_CURRENT_DIR_OUTSIDE_ROOT"));
    }
    let current_dir = non_verbatim_drive_path(&canonical_current_dir)?;
    let stdout_path = validate_new_capture_path(&plan.stdout_path, &run_root)?;
    let stderr_path = validate_new_capture_path(&plan.stderr_path, &run_root)?;
    if stdout_path == stderr_path {
        return Err(spawn_error("HERMES_WINDOWS_CAPTURE_PATH_COLLISION"));
    }
    command_line(&executable, &plan.arguments)?;
    environment_block(&plan.environment)?;

    Ok(ValidatedPaths {
        executable,
        current_dir,
        stdout_path,
        stderr_path,
    })
}

fn non_verbatim_drive_path(path: &Path) -> HermesAdapterResult<PathBuf> {
    let text = path.as_os_str().to_string_lossy();
    if let Some(without_prefix) = text.strip_prefix(r"\\?\") {
        if without_prefix.starts_with("UNC\\") {
            return Err(spawn_error("HERMES_WINDOWS_NETWORK_PATH_DENIED"));
        }
        return Ok(PathBuf::from(without_prefix));
    }
    Ok(path.to_path_buf())
}

fn validate_new_capture_path(path: &Path, run_root: &Path) -> HermesAdapterResult<PathBuf> {
    if path.exists() {
        return Err(spawn_error("HERMES_WINDOWS_CAPTURE_ALREADY_EXISTS"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| spawn_error("HERMES_WINDOWS_CAPTURE_NO_PARENT"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| spawn_error("HERMES_WINDOWS_CAPTURE_NO_FILE_NAME"))?;
    let parent =
        std::fs::canonicalize(parent).map_err(|_| spawn_error("HERMES_WINDOWS_CAPTURE_PARENT"))?;
    if !parent.starts_with(run_root) {
        return Err(spawn_error("HERMES_WINDOWS_CAPTURE_OUTSIDE_ROOT"));
    }
    Ok(parent.join(file_name))
}

fn is_reparse_point(path: &Path) -> HermesAdapterResult<bool> {
    let path = wide_null(path.as_os_str())?;
    // SAFETY: `path` is NUL-terminated and remains live for this call.
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(spawn_error("HERMES_WINDOWS_RUN_ROOT_ATTRIBUTES"));
    }
    Ok(attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

struct RedirectHandles {
    child_stdin: OwnedHandle,
    child_stdout: OwnedHandle,
    child_stderr: OwnedHandle,
    parent_stdin: Option<File>,
    parent_stdout: Option<File>,
    parent_stderr: Option<File>,
}

impl RedirectHandles {
    fn create(
        stdout_path: &Path,
        stderr_path: &Path,
        pipe_stdin: bool,
        pipe_stdout: bool,
        pipe_stderr: bool,
    ) -> HermesAdapterResult<Self> {
        let (child_stdin, parent_stdin) = if pipe_stdin {
            let (child_reader, parent_writer) = create_anonymous_pipe(true)?;
            (child_reader, Some(File::from(parent_writer)))
        } else {
            (
                open_inheritable_file(OsStr::new("NUL"), GENERIC_READ, OPEN_EXISTING)?,
                None,
            )
        };
        let (child_stdout, parent_stdout) = if pipe_stdout {
            let (parent_reader, child_writer) = create_anonymous_pipe(false)?;
            (child_writer, Some(File::from(parent_reader)))
        } else {
            (open_inheritable_capture(stdout_path.as_os_str())?, None)
        };
        let (child_stderr, parent_stderr) = if pipe_stderr {
            let (parent_reader, child_writer) = create_anonymous_pipe(false)?;
            (child_writer, Some(File::from(parent_reader)))
        } else {
            (open_inheritable_capture(stderr_path.as_os_str())?, None)
        };
        Ok(Self {
            child_stdin,
            child_stdout,
            child_stderr,
            parent_stdin,
            parent_stdout,
            parent_stderr,
        })
    }

    fn as_slice(&self) -> [HANDLE; 3] {
        [
            self.child_stdin.raw(),
            self.child_stdout.raw(),
            self.child_stderr.raw(),
        ]
    }

    fn clear_parent_capture_inheritance(&self) -> Result<(), ()> {
        if self.parent_stdout.is_none() {
            clear_handle_inheritance(&self.child_stdout)?;
        }
        if self.parent_stderr.is_none() {
            clear_handle_inheritance(&self.child_stderr)?;
        }
        Ok(())
    }

    fn retain_capture_files(&self) -> HermesAdapterResult<()> {
        if self.parent_stdout.is_none() {
            set_handle_deletion(self.child_stdout.raw(), false)?;
        }
        if self.parent_stderr.is_none() {
            set_handle_deletion(self.child_stderr.raw(), false)?;
        }
        Ok(())
    }

    fn delete_capture_files_best_effort(&self) {
        if self.parent_stdout.is_none() {
            let _ = mark_handle_for_deletion(self.child_stdout.raw());
        }
        if self.parent_stderr.is_none() {
            let _ = mark_handle_for_deletion(self.child_stderr.raw());
        }
    }
}

fn create_anonymous_pipe(
    parent_is_writer: bool,
) -> HermesAdapterResult<(OwnedHandle, OwnedHandle)> {
    let attributes = inheritable_security_attributes()?;
    let mut read_handle: HANDLE = null_mut();
    let mut write_handle: HANDLE = null_mut();
    // SAFETY: both output pointers and the security attributes are valid for
    // the duration of this call. Successful handles are each owned once below.
    if unsafe {
        CreatePipe(
            &raw mut read_handle,
            &raw mut write_handle,
            &raw const attributes,
            0,
        )
    } == 0
    {
        return Err(spawn_error("HERMES_WINDOWS_PIPE_CREATE_FAILED"));
    }
    let reader = owned_handle(read_handle)
        .ok_or_else(|| spawn_error("HERMES_WINDOWS_PIPE_HANDLE_INVALID"))?;
    let writer = owned_handle(write_handle)
        .ok_or_else(|| spawn_error("HERMES_WINDOWS_PIPE_HANDLE_INVALID"))?;
    let parent = if parent_is_writer {
        writer.raw()
    } else {
        reader.raw()
    };
    // SAFETY: `parent` is the live parent-owned pipe end. Clearing inheritance
    // ensures the explicit child handle list cannot leak the peer end.
    if unsafe { SetHandleInformation(parent, HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(spawn_error("HERMES_WINDOWS_PIPE_INHERITANCE_REJECTED"));
    }
    Ok((reader, writer))
}

fn clear_handle_inheritance(handle: &OwnedHandle) -> Result<(), ()> {
    // SAFETY: `handle` is live and owned by the caller. Clearing the inherit
    // bit does not change its access rights or lifetime.
    if unsafe { SetHandleInformation(handle.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        Err(())
    } else {
        Ok(())
    }
}

fn open_inheritable_capture(path: &OsStr) -> HermesAdapterResult<OwnedHandle> {
    let path = wide_null(path)?;
    let attributes = inheritable_security_attributes()?;
    // SAFETY: the path and attributes remain live. Share mode zero prevents
    // replacement while the parent retains this same handle.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | DELETE,
            0,
            &raw const attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(spawn_error("HERMES_WINDOWS_REDIRECT_FILE"));
    }
    let handle = owned_handle(handle).ok_or_else(|| spawn_error("HERMES_WINDOWS_REDIRECT_FILE"))?;
    mark_handle_for_deletion(handle.raw())?;
    Ok(handle)
}

fn inheritable_security_attributes() -> HermesAdapterResult<SECURITY_ATTRIBUTES> {
    Ok(SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| spawn_error("HERMES_WINDOWS_SECURITY_ATTRIBUTES_SIZE"))?,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    })
}

fn read_locked_capture(file: &File, limit: u64) -> HermesAdapterResult<Vec<u8>> {
    read_locked_capture_since(file, 0, limit)
}

fn read_locked_capture_since(file: &File, offset: u64, limit: u64) -> HermesAdapterResult<Vec<u8>> {
    let length = file
        .metadata()
        .map_err(|_| spawn_error("HERMES_WINDOWS_CAPTURE_METADATA"))?
        .len();
    let remaining = length
        .checked_sub(offset)
        .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_CAPTURE_OFFSET_REJECTED"))?;
    if remaining > limit {
        return Err(HermesAdapterError::new(
            HermesAdapterErrorKind::Malformed,
            "HERMES_WINDOWS_CAPTURE_LIMIT",
        ));
    }
    let capacity = usize::try_from(remaining)
        .map_err(|_| spawn_error("HERMES_WINDOWS_CAPTURE_SIZE_OVERFLOW"))?;
    let mut bytes = vec![0_u8; capacity];
    let mut read_offset = 0_usize;
    while read_offset < bytes.len() {
        let file_offset = offset
            .checked_add(read_offset as u64)
            .ok_or_else(|| spawn_error("HERMES_WINDOWS_CAPTURE_SIZE_OVERFLOW"))?;
        let read = file
            .seek_read(&mut bytes[read_offset..], file_offset)
            .map_err(|_| ambiguous_error("HERMES_WINDOWS_CAPTURE_READ"))?;
        if read == 0 {
            return Err(ambiguous_error("HERMES_WINDOWS_CAPTURE_TRUNCATED"));
        }
        read_offset += read;
    }
    Ok(bytes)
}

fn open_inheritable_file(
    path: &OsStr,
    desired_access: u32,
    disposition: u32,
) -> HermesAdapterResult<OwnedHandle> {
    let path = wide_null(path)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| spawn_error("HERMES_WINDOWS_SECURITY_ATTRIBUTES_SIZE"))?,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: the path and attributes are valid and the returned handle is
    // uniquely owned by `OwnedHandle`.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &raw const attributes,
            disposition,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    owned_handle(handle).ok_or_else(|| spawn_error("HERMES_WINDOWS_REDIRECT_FILE"))
}

fn create_kill_on_close_job() -> HermesAdapterResult<OwnedHandle> {
    // SAFETY: unnamed Job Object creation uses no caller pointers.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    let job = owned_handle(job).ok_or_else(|| spawn_error("HERMES_WINDOWS_CREATE_JOB"))?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
        .map_err(|_| spawn_error("HERMES_WINDOWS_JOB_LIMIT_SIZE"))?;
    // SAFETY: the job and immutable limit structure are valid for this call.
    let set = unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size,
        )
    };
    if set == 0 {
        return Err(spawn_error("HERMES_WINDOWS_SET_JOB_LIMIT"));
    }
    Ok(job)
}

struct ProcThreadAttributes {
    storage: Vec<usize>,
}

impl ProcThreadAttributes {
    fn for_handles(handles: [HANDLE; 3]) -> HermesAdapterResult<Self> {
        let mut bytes = 0_usize;
        // SAFETY: null is the documented sizing probe for one attribute.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &raw mut bytes);
        }
        if bytes == 0 {
            return Err(spawn_error("HERMES_WINDOWS_ATTRIBUTE_LIST_SIZE"));
        }
        let words = bytes
            .checked_add(size_of::<usize>() - 1)
            .map(|value| value / size_of::<usize>())
            .ok_or_else(|| spawn_error("HERMES_WINDOWS_ATTRIBUTE_LIST_SIZE"))?;
        let mut storage = vec![0_usize; words];
        let raw = storage.as_mut_ptr().cast();
        // SAFETY: `storage` is aligned and at least the probed byte size.
        if unsafe { InitializeProcThreadAttributeList(raw, 1, 0, &raw mut bytes) } == 0 {
            return Err(spawn_error("HERMES_WINDOWS_ATTRIBUTE_LIST_INIT"));
        }
        let result = Self { storage };
        let handle_list_attribute = usize::try_from(PROC_THREAD_ATTRIBUTE_HANDLE_LIST)
            .map_err(|_| spawn_error("HERMES_WINDOWS_ATTRIBUTE_ID"))?;
        // SAFETY: the initialized list and live handle values remain valid;
        // the kernel copies the attribute payload during this call.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                result.raw(),
                0,
                handle_list_attribute,
                handles.as_ptr().cast(),
                size_of::<[HANDLE; 3]>(),
                null_mut(),
                null(),
            )
        };
        if updated == 0 {
            return Err(spawn_error("HERMES_WINDOWS_ATTRIBUTE_HANDLE_LIST"));
        }
        Ok(result)
    }

    fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_ptr().cast_mut().cast()
    }
}

impl Drop for ProcThreadAttributes {
    fn drop(&mut self) {
        if !self.storage.is_empty() {
            // SAFETY: successful construction initialized this list once.
            unsafe { DeleteProcThreadAttributeList(self.raw()) };
        }
    }
}

fn wait_for_exit(
    deadline: Instant,
    teardown_timeout: Duration,
    job: &OwnedHandle,
    process: &OwnedHandle,
) -> HermesAdapterResult<u32> {
    let wait_millis = millis_until(deadline);
    // SAFETY: `process` is valid and the timeout is bounded.
    let wait = unsafe { WaitForSingleObject(process.raw(), wait_millis) };
    match wait {
        WAIT_OBJECT_0 => {
            let mut exit_code = 0_u32;
            // SAFETY: the signaled process handle and output pointer are valid.
            if unsafe { GetExitCodeProcess(process.raw(), &raw mut exit_code) } == 0 {
                terminate_job_and_reap(job, process, teardown_timeout)?;
                return Err(spawn_error("HERMES_WINDOWS_EXIT_CODE"));
            }
            ensure_job_empty(job, process, teardown_timeout)?;
            Ok(exit_code)
        }
        WAIT_TIMEOUT => {
            terminate_job_and_reap(job, process, teardown_timeout)?;
            Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Timeout,
                "HERMES_WINDOWS_DEADLINE_EXCEEDED",
            ))
        }
        WAIT_FAILED => {
            terminate_job_and_reap(job, process, teardown_timeout)?;
            Err(spawn_error("HERMES_WINDOWS_WAIT_FAILED"))
        }
        _ => {
            terminate_job_and_reap(job, process, teardown_timeout)?;
            Err(spawn_error("HERMES_WINDOWS_WAIT_UNKNOWN"))
        }
    }
}

fn ensure_job_empty(
    job: &OwnedHandle,
    process: &OwnedHandle,
    timeout: Duration,
) -> HermesAdapterResult<()> {
    match job_active_processes(job) {
        Ok(0) => Ok(()),
        Ok(_) | Err(()) => terminate_job_and_reap(job, process, timeout),
    }
}

fn terminate_job_and_reap(
    job: &OwnedHandle,
    process: &OwnedHandle,
    timeout: Duration,
) -> HermesAdapterResult<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(teardown_ambiguous)?;
    // SAFETY: `job` is live and the termination result is checked.
    if unsafe { TerminateJobObject(job.raw(), PROCESS_TEARDOWN_EXIT_CODE) } == 0 {
        return Err(teardown_ambiguous());
    }
    // SAFETY: `process` remains live and the wait is bounded.
    if unsafe { WaitForSingleObject(process.raw(), millis_until(deadline)) } != WAIT_OBJECT_0 {
        return Err(teardown_ambiguous());
    }
    loop {
        match job_active_processes(job) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(()) => return Err(teardown_ambiguous()),
        }
        if Instant::now() >= deadline {
            return Err(teardown_ambiguous());
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn terminate_unassigned_process(
    process: &OwnedHandle,
    timeout: Duration,
) -> HermesAdapterResult<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(teardown_ambiguous)?;
    // SAFETY: the process is live and still suspended; the result is checked.
    if unsafe { TerminateProcess(process.raw(), PROCESS_TEARDOWN_EXIT_CODE) } == 0 {
        return Err(teardown_ambiguous());
    }
    // SAFETY: the process remains live through this bounded wait.
    if unsafe { WaitForSingleObject(process.raw(), millis_until(deadline)) } != WAIT_OBJECT_0 {
        return Err(teardown_ambiguous());
    }
    Ok(())
}

fn job_active_processes(job: &OwnedHandle) -> Result<u32, ()> {
    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    let size =
        u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>()).map_err(|_| ())?;
    // SAFETY: `accounting` is writable storage of the queried type.
    let queried = unsafe {
        QueryInformationJobObject(
            job.raw(),
            JobObjectBasicAccountingInformation,
            (&raw mut accounting).cast(),
            size,
            null_mut(),
        )
    };
    if queried == 0 {
        Err(())
    } else {
        Ok(accounting.ActiveProcesses)
    }
}

fn millis_until(deadline: Instant) -> u32 {
    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
        return 0;
    };
    let millis = remaining.as_millis();
    if millis == 0 && !remaining.is_zero() {
        1
    } else {
        u32::try_from(millis).unwrap_or(u32::MAX - 1)
    }
}

fn command_line(executable: &Path, arguments: &[OsString]) -> HermesAdapterResult<Vec<u16>> {
    let mut result = Vec::new();
    append_quoted_argument(&mut result, executable.as_os_str())?;
    for argument in arguments {
        result.push(u16::from(b' '));
        append_quoted_argument(&mut result, argument)?;
    }
    result.push(0);
    Ok(result)
}

fn append_quoted_argument(output: &mut Vec<u16>, argument: &OsStr) -> HermesAdapterResult<()> {
    let encoded: Vec<u16> = argument.encode_wide().collect();
    if encoded.contains(&0) {
        return Err(spawn_error("HERMES_WINDOWS_ARGUMENT_NUL"));
    }
    let needs_quotes = encoded.is_empty()
        || encoded
            .iter()
            .any(|unit| matches!(*unit, 0x20 | 0x09 | 0x22));
    if !needs_quotes {
        output.extend(encoded);
        return Ok(());
    }

    output.push(u16::from(b'"'));
    let mut backslashes = 0_usize;
    for unit in encoded {
        match unit {
            0x5c => backslashes += 1,
            0x22 => {
                output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
                output.push(unit);
                backslashes = 0;
            }
            _ => {
                output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
                output.push(unit);
                backslashes = 0;
            }
        }
    }
    output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
    output.push(u16::from(b'"'));
    Ok(())
}

fn environment_block(environment: &BTreeMap<OsString, OsString>) -> HermesAdapterResult<Vec<u16>> {
    let mut entries: Vec<_> = environment.iter().collect();
    entries.sort_by(|(left, _), (right, _)| {
        left.to_string_lossy()
            .to_ascii_uppercase()
            .cmp(&right.to_string_lossy().to_ascii_uppercase())
    });
    for pair in entries.windows(2) {
        if pair[0]
            .0
            .to_string_lossy()
            .eq_ignore_ascii_case(&pair[1].0.to_string_lossy())
        {
            return Err(spawn_error("HERMES_WINDOWS_DUPLICATE_ENVIRONMENT"));
        }
    }

    let mut block = Vec::new();
    for (name, value) in entries {
        let name: Vec<u16> = name.encode_wide().collect();
        let value: Vec<u16> = value.encode_wide().collect();
        if name.is_empty()
            || name.contains(&0)
            || value.contains(&0)
            || name.contains(&u16::from(b'='))
        {
            return Err(spawn_error("HERMES_WINDOWS_INVALID_ENVIRONMENT"));
        }
        block.extend(name);
        block.push(u16::from(b'='));
        block.extend(value);
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Ok(block)
}

fn wide_null(value: impl AsRef<OsStr>) -> HermesAdapterResult<Vec<u16>> {
    let mut wide: Vec<u16> = value.as_ref().encode_wide().collect();
    if wide.contains(&0) {
        return Err(spawn_error("HERMES_WINDOWS_PATH_NUL"));
    }
    wide.push(0);
    Ok(wide)
}

fn spawn_error(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::Spawn, code)
}

fn ambiguous_error(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::Ambiguous, code)
}

fn teardown_ambiguous() -> HermesAdapterError {
    ambiguous_error("HERMES_WINDOWS_TEARDOWN_AMBIGUOUS")
}

fn owned_handle(handle: HANDLE) -> Option<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return None;
    }
    // SAFETY: every successful Win32 creation call above transfers one unique
    // owned handle to this function. `OwnedHandle` supplies the matching close
    // and is `Send` without a local unsafe marker implementation.
    Some(unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
}

fn file_information(handle: &OwnedHandle) -> HermesAdapterResult<BY_HANDLE_FILE_INFORMATION> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `handle` remains live and `information` is writable output
    // storage of the exact documented type.
    if unsafe { GetFileInformationByHandle(handle.raw(), &raw mut information) } == 0 {
        return Err(ambiguous_error("HERMES_WINDOWS_HANDLE_IDENTITY_REJECTED"));
    }
    Ok(information)
}

fn final_path_by_handle(handle: &OwnedHandle) -> HermesAdapterResult<PathBuf> {
    // Query the required UTF-16 length first; the handle remains pinned across
    // both calls, so a path rename/reparse cannot substitute another object.
    let required = unsafe { GetFinalPathNameByHandleW(handle.raw(), null_mut(), 0, 0) };
    if required == 0 {
        return Err(ambiguous_error("HERMES_WINDOWS_HANDLE_PATH_REJECTED"));
    }
    let capacity = usize::try_from(required)
        .map_err(|_| ambiguous_error("HERMES_WINDOWS_HANDLE_PATH_REJECTED"))?
        .checked_add(1)
        .ok_or_else(|| ambiguous_error("HERMES_WINDOWS_HANDLE_PATH_REJECTED"))?;
    let mut buffer = vec![0_u16; capacity];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            handle.raw(),
            buffer.as_mut_ptr(),
            u32::try_from(buffer.len())
                .map_err(|_| ambiguous_error("HERMES_WINDOWS_HANDLE_PATH_REJECTED"))?,
            0,
        )
    };
    if length == 0
        || usize::try_from(length)
            .ok()
            .is_none_or(|value| value >= buffer.len())
    {
        return Err(ambiguous_error("HERMES_WINDOWS_HANDLE_PATH_REJECTED"));
    }
    buffer.truncate(usize::try_from(length).expect("validated path length"));
    Ok(PathBuf::from(OsString::from_wide(&buffer)))
}

fn mark_handle_for_deletion(handle: HANDLE) -> HermesAdapterResult<()> {
    let deadline = Instant::now() + HANDLE_DELETE_RETRY_TIMEOUT;
    loop {
        match set_handle_deletion(handle, true) {
            Ok(()) => return Ok(()),
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Err(failure) => return Err(failure),
        }
    }
}

fn set_handle_deletion(handle: HANDLE, delete: bool) -> HermesAdapterResult<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: delete };
    // SAFETY: `handle` is live with DELETE access and `disposition` is the
    // exact fixed-size structure required by FileDispositionInfo.
    if unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfo,
            (&raw const disposition).cast(),
            u32::try_from(size_of::<FILE_DISPOSITION_INFO>())
                .map_err(|_| ambiguous_error("HERMES_WINDOWS_DELETE_HANDLE_REJECTED"))?,
        )
    } == 0
    {
        return Err(ambiguous_error("HERMES_WINDOWS_DELETE_HANDLE_REJECTED"));
    }
    Ok(())
}

trait OwnedHandleExt {
    fn raw(&self) -> HANDLE;
}

impl OwnedHandleExt for OwnedHandle {
    fn raw(&self) -> HANDLE {
        self.as_raw_handle().cast()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static PIPE_TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn remove_capture_with_retry(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match fs::remove_file(path) {
                Ok(()) => return,
                Err(_) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("remove exact capture {path:?}: {error}"),
            }
        }
    }

    #[test]
    fn pinned_directory_create_new_returns_the_created_object_and_preserves_collision() {
        let sequence = PIPE_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "lattice-hermes-pinned-directory-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&parent).expect("fresh pinned-directory parent");
        let parent = fs::canonicalize(parent).expect("canonical pinned-directory parent");
        let parent_guard =
            WindowsPinnedDirectory::open(&parent, false, false, false).expect("pin parent");
        let child = parent.join("owned");

        let child_guard = WindowsPinnedDirectory::create_new(&parent_guard, OsStr::new("owned"))
            .expect("atomically create and own child directory");
        assert!(crate::same_path(child_guard.final_path(), &child));
        assert!(WindowsPinnedDirectory::create_new(&parent_guard, OsStr::new("owned")).is_err());
        assert!(child.is_dir(), "collision must preserve the owned child");

        child_guard.delete().expect("delete exact pinned child");
        drop(parent_guard);
        fs::remove_dir(parent).expect("remove empty pinned-directory parent");
    }

    #[test]
    fn duplex_parent_ends_relay_and_child_exits_on_input_close() {
        fn assert_send<T: Send>() {}
        assert_send::<WindowsJobChild>();
        let sequence = PIPE_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lattice-hermes-windows-job-duplex-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create exact duplex root");
        let system_root = std::env::var_os("SystemRoot").expect("Windows system root");
        let plan = WindowsJobCommandPlan {
            executable: PathBuf::from(system_root).join("System32").join("more.com"),
            arguments: Vec::new(),
            current_dir: root.clone(),
            environment: BTreeMap::new(),
            run_root: root.clone(),
            stdout_path: root.join("unused.stdout"),
            stderr_path: root.join("unused.stderr"),
            stdout_limit: 4096,
            stderr_limit: 4096,
            deadline: Instant::now() + Duration::from_secs(5),
            teardown_timeout: Duration::from_secs(2),
        };
        let mut child = spawn_duplex(&plan).expect("spawn owned duplex child");
        let mut stdin = child.take_stdin_writer().expect("parent stdin writer");
        let mut stdout = child.take_stdout_reader().expect("parent stdout reader");
        let mut stderr = child.take_stderr_reader().expect("parent stderr reader");
        stdin
            .write_all(b"lattice-pipe-probe\r\n")
            .and_then(|()| stdin.flush())
            .expect("write probe");
        drop(stdin);

        let mut output = Vec::new();
        stdout.read_to_end(&mut output).expect("read child stdout");
        let mut diagnostic = Vec::new();
        stderr
            .read_to_end(&mut diagnostic)
            .expect("read child stderr");
        assert_eq!(child.wait_for_exit().expect("child exit"), 0);
        assert!(
            output
                .windows(b"lattice-pipe-probe".len())
                .any(|window| window == b"lattice-pipe-probe")
        );
        assert!(diagnostic.is_empty());
        drop(child);
        fs::remove_dir(&root).expect("remove exact duplex root");
    }

    #[test]
    fn retained_capture_handles_are_not_inheritable_after_spawn() {
        let sequence = PIPE_TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lattice-hermes-windows-job-capture-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create exact capture root");
        let system_root = std::env::var_os("SystemRoot").expect("Windows system root");
        let plan = WindowsJobCommandPlan {
            executable: PathBuf::from(system_root).join("System32").join("more.com"),
            arguments: Vec::new(),
            current_dir: root.clone(),
            environment: BTreeMap::new(),
            run_root: root.clone(),
            stdout_path: root.join("capture.stdout"),
            stderr_path: root.join("capture.stderr"),
            stdout_limit: 4096,
            stderr_limit: 4096,
            deadline: Instant::now() + Duration::from_secs(5),
            teardown_timeout: Duration::from_secs(2),
        };
        let mut child = spawn(&plan).expect("spawn owned capture child");

        let stdout_handle = match &child.stdout {
            WindowsJobStdout::Capture(file) => file.as_raw_handle().cast(),
            WindowsJobStdout::Pipe(_) => unreachable!("stdout must be captured"),
        };
        let stderr_handle = match &child.stderr {
            WindowsJobStderr::Capture(file) => file.as_raw_handle().cast(),
            WindowsJobStderr::Pipe(_) => unreachable!("stderr must be captured"),
        };
        for handle in [stdout_handle, stderr_handle] {
            let mut flags = 0_u32;
            // SAFETY: each handle is still owned by `child` for this call.
            assert_ne!(unsafe { GetHandleInformation(handle, &raw mut flags) }, 0);
            assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
        }

        assert_eq!(child.wait_for_exit().expect("child exit"), 0);
        drop(child);
        remove_capture_with_retry(&plan.stdout_path);
        remove_capture_with_retry(&plan.stderr_path);
        fs::remove_dir(&root).expect("remove exact capture root");
    }
}
