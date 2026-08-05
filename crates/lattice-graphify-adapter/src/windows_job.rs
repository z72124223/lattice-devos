//! Windows-only owned Job Object launcher.
//!
//! The production executor enters through the crate-private `run` function;
//! direct Win32 FFI and its unsafe blocks are confined to this module. This
//! layer owns the trusted `wsl.exe` launcher lifecycle only. The untrusted
//! Graphify process is confined by the fixed bubblewrap plan inside WSL.

#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::FileExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileAttributesW,
    INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

use crate::{GraphifyAdapterError, GraphifyAdapterErrorKind, GraphifyAdapterResult};

const PROCESS_TEARDOWN_EXIT_CODE: u32 = 0xC0DE_0330;
const MAX_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(30);

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

pub(crate) fn run(plan: &WindowsJobCommandPlan) -> GraphifyAdapterResult<WindowsJobProcessExit> {
    let paths = validate_plan(plan)?;
    let redirects = RedirectHandles::create(&paths.stdout_path, &paths.stderr_path)?;
    let job = create_kill_on_close_job()?;
    let attributes = ProcThreadAttributes::for_handles(redirects.as_slice())?;
    let mut command_line = command_line(&paths.executable, &plan.arguments)?;
    let environment = environment_block(&plan.environment)?;
    let current_dir = wide_null(&paths.current_dir)?;
    let executable = wide_null(&paths.executable)?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>())
        .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_STARTUP_INFO_SIZE"))?;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = redirects.stdin.raw();
    startup.StartupInfo.hStdOutput = redirects.stdout.as_raw_handle().cast();
    startup.StartupInfo.hStdError = redirects.stderr.as_raw_handle().cast();
    startup.lpAttributeList = attributes.raw();

    // SAFETY: every pointer references live storage through this call; the
    // mutable command line and double-NUL environment block satisfy Win32's
    // process-creation contracts. The explicit handle list contains only the
    // three inheritable standard handles owned by `redirects`.
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
        return Err(spawn_error("GRAPHIFY_WINDOWS_CREATE_PROCESS"));
    }

    let process = OwnedHandle::new(process_info.hProcess)
        .ok_or_else(|| spawn_error("GRAPHIFY_WINDOWS_PROCESS_HANDLE"))?;
    let Some(thread_handle) = OwnedHandle::new(process_info.hThread) else {
        terminate_unassigned_process(&process, plan.teardown_timeout)?;
        return Err(spawn_error("GRAPHIFY_WINDOWS_THREAD_HANDLE"));
    };
    drop(attributes);

    // SAFETY: `job` and `process` are valid handles owned for this scope. The
    // child is still suspended, so assignment precedes its first instruction.
    if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
        terminate_unassigned_process(&process, plan.teardown_timeout)?;
        return Err(spawn_error("GRAPHIFY_WINDOWS_JOB_ASSIGNMENT"));
    }

    // SAFETY: `thread_handle` is the suspended primary thread returned for the
    // process above and has not been resumed or closed.
    if unsafe { ResumeThread(thread_handle.raw()) } == u32::MAX {
        terminate_job_and_reap(&job, &process, plan.teardown_timeout)?;
        return Err(spawn_error("GRAPHIFY_WINDOWS_RESUME_THREAD"));
    }
    drop(thread_handle);

    let exit_code = wait_for_exit(plan, &job, &process)?;
    let stdout = read_locked_capture(&redirects.stdout, plan.stdout_limit)?;
    let stderr = read_locked_capture(&redirects.stderr, plan.stderr_limit)?;
    Ok(WindowsJobProcessExit {
        exit_code,
        stdout,
        stderr,
    })
}

struct ValidatedPaths {
    executable: PathBuf,
    current_dir: PathBuf,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

fn validate_plan(plan: &WindowsJobCommandPlan) -> GraphifyAdapterResult<ValidatedPaths> {
    if plan.deadline <= Instant::now()
        || plan.teardown_timeout.is_zero()
        || plan.teardown_timeout > MAX_TEARDOWN_TIMEOUT
    {
        return Err(spawn_error("GRAPHIFY_WINDOWS_INVALID_DEADLINE"));
    }

    let executable = std::fs::canonicalize(&plan.executable)
        .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_EXECUTABLE_PATH"))?;
    if !executable.is_file() {
        return Err(spawn_error("GRAPHIFY_WINDOWS_EXECUTABLE_NOT_FILE"));
    }
    let run_root = std::fs::canonicalize(&plan.run_root)
        .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_RUN_ROOT_PATH"))?;
    if !run_root.is_dir() || is_reparse_point(&run_root)? {
        return Err(spawn_error("GRAPHIFY_WINDOWS_RUN_ROOT_UNSAFE"));
    }
    let canonical_current_dir = std::fs::canonicalize(&plan.current_dir)
        .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_CURRENT_DIR_PATH"))?;
    if !canonical_current_dir.is_dir() || !canonical_current_dir.starts_with(&run_root) {
        return Err(spawn_error("GRAPHIFY_WINDOWS_CURRENT_DIR_OUTSIDE_ROOT"));
    }
    let current_dir = non_verbatim_drive_path(&canonical_current_dir)?;
    let stdout_path = validate_new_capture_path(&plan.stdout_path, &run_root)?;
    let stderr_path = validate_new_capture_path(&plan.stderr_path, &run_root)?;
    if stdout_path == stderr_path {
        return Err(spawn_error("GRAPHIFY_WINDOWS_CAPTURE_PATH_COLLISION"));
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

fn non_verbatim_drive_path(path: &Path) -> GraphifyAdapterResult<PathBuf> {
    let text = path.as_os_str().to_string_lossy();
    if let Some(without_prefix) = text.strip_prefix(r"\\?\") {
        if without_prefix.starts_with("UNC\\") {
            return Err(spawn_error("GRAPHIFY_WINDOWS_NETWORK_PATH_DENIED"));
        }
        return Ok(PathBuf::from(without_prefix));
    }
    Ok(path.to_path_buf())
}

fn validate_new_capture_path(path: &Path, run_root: &Path) -> GraphifyAdapterResult<PathBuf> {
    if path.exists() {
        return Err(spawn_error("GRAPHIFY_WINDOWS_CAPTURE_ALREADY_EXISTS"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| spawn_error("GRAPHIFY_WINDOWS_CAPTURE_NO_PARENT"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| spawn_error("GRAPHIFY_WINDOWS_CAPTURE_NO_FILE_NAME"))?;
    let parent = std::fs::canonicalize(parent)
        .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_CAPTURE_PARENT"))?;
    if !parent.starts_with(run_root) {
        return Err(spawn_error("GRAPHIFY_WINDOWS_CAPTURE_OUTSIDE_ROOT"));
    }
    Ok(parent.join(file_name))
}

fn is_reparse_point(path: &Path) -> GraphifyAdapterResult<bool> {
    let path = wide_null(path.as_os_str())?;
    // SAFETY: `path` is NUL-terminated and remains live for this call.
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(spawn_error("GRAPHIFY_WINDOWS_RUN_ROOT_ATTRIBUTES"));
    }
    Ok(attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

struct RedirectHandles {
    stdin: OwnedHandle,
    stdout: File,
    stderr: File,
}

impl RedirectHandles {
    fn create(stdout_path: &Path, stderr_path: &Path) -> GraphifyAdapterResult<Self> {
        Ok(Self {
            stdin: open_inheritable_file(OsStr::new("NUL"), GENERIC_READ, OPEN_EXISTING)?,
            stdout: open_inheritable_capture(stdout_path.as_os_str())?,
            stderr: open_inheritable_capture(stderr_path.as_os_str())?,
        })
    }

    fn as_slice(&self) -> [HANDLE; 3] {
        [
            self.stdin.raw(),
            self.stdout.as_raw_handle().cast(),
            self.stderr.as_raw_handle().cast(),
        ]
    }
}

fn open_inheritable_capture(path: &OsStr) -> GraphifyAdapterResult<File> {
    let path = wide_null(path)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_SECURITY_ATTRIBUTES_SIZE"))?,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: the path and attributes remain live through the call. Share mode
    // zero prevents path replacement while the parent retains this same handle.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            &raw const attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(spawn_error("GRAPHIFY_WINDOWS_REDIRECT_FILE"));
    }
    // SAFETY: `handle` is a unique successful CreateFileW result and ownership
    // moves to `File`, which closes it exactly once.
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}

fn read_locked_capture(file: &File, limit: u64) -> GraphifyAdapterResult<Vec<u8>> {
    let length = file
        .metadata()
        .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_CAPTURE_METADATA"))?
        .len();
    if length > limit {
        return Err(GraphifyAdapterError::new(
            GraphifyAdapterErrorKind::OutputLimit,
            "GRAPHIFY_WINDOWS_CAPTURE_LIMIT",
        ));
    }
    let capacity = usize::try_from(length)
        .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_CAPTURE_SIZE_OVERFLOW"))?;
    let mut bytes = vec![0_u8; capacity];
    let mut offset = 0_usize;
    while offset < bytes.len() {
        let read = file
            .seek_read(&mut bytes[offset..], offset as u64)
            .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_CAPTURE_READ"))?;
        if read == 0 {
            return Err(GraphifyAdapterError::new(
                GraphifyAdapterErrorKind::PartialOutput,
                "GRAPHIFY_WINDOWS_CAPTURE_TRUNCATED",
            ));
        }
        offset += read;
    }
    Ok(bytes)
}

fn open_inheritable_file(
    path: &OsStr,
    desired_access: u32,
    disposition: u32,
) -> GraphifyAdapterResult<OwnedHandle> {
    let path = wide_null(path)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_SECURITY_ATTRIBUTES_SIZE"))?,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    // SAFETY: `path` and `attributes` are valid for this call; the returned
    // handle is owned and closed by `OwnedHandle`.
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
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(spawn_error("GRAPHIFY_WINDOWS_REDIRECT_FILE"));
    }
    Ok(OwnedHandle(handle))
}

fn create_kill_on_close_job() -> GraphifyAdapterResult<OwnedHandle> {
    // SAFETY: unnamed job creation uses no caller pointers.
    let job = unsafe { CreateJobObjectW(null(), null()) };
    let job = OwnedHandle::new(job).ok_or_else(|| spawn_error("GRAPHIFY_WINDOWS_CREATE_JOB"))?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
        .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_JOB_LIMIT_SIZE"))?;
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
        return Err(spawn_error("GRAPHIFY_WINDOWS_SET_JOB_LIMIT"));
    }
    Ok(job)
}

struct ProcThreadAttributes {
    storage: Vec<usize>,
}

impl ProcThreadAttributes {
    fn for_handles(handles: [HANDLE; 3]) -> GraphifyAdapterResult<Self> {
        let mut bytes = 0_usize;
        // SAFETY: null is the documented sizing probe for one attribute.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &raw mut bytes);
        }
        if bytes == 0 {
            return Err(spawn_error("GRAPHIFY_WINDOWS_ATTRIBUTE_LIST_SIZE"));
        }
        let words = bytes
            .checked_add(size_of::<usize>() - 1)
            .map(|value| value / size_of::<usize>())
            .ok_or_else(|| spawn_error("GRAPHIFY_WINDOWS_ATTRIBUTE_LIST_SIZE"))?;
        let mut storage = vec![0_usize; words];
        let raw = storage.as_mut_ptr().cast();
        // SAFETY: `storage` is aligned and at least the probed byte size.
        if unsafe { InitializeProcThreadAttributeList(raw, 1, 0, &raw mut bytes) } == 0 {
            return Err(spawn_error("GRAPHIFY_WINDOWS_ATTRIBUTE_LIST_INIT"));
        }
        let result = Self { storage };
        // SAFETY: the initialized list and three live handle values remain
        // valid through this call; the kernel copies the attribute payload.
        let handle_list_attribute = usize::try_from(PROC_THREAD_ATTRIBUTE_HANDLE_LIST)
            .map_err(|_| spawn_error("GRAPHIFY_WINDOWS_ATTRIBUTE_ID"))?;
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
            return Err(spawn_error("GRAPHIFY_WINDOWS_ATTRIBUTE_HANDLE_LIST"));
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
            // SAFETY: successful construction initializes this list exactly
            // once and it remains live until this drop.
            unsafe { DeleteProcThreadAttributeList(self.raw()) };
        }
    }
}

fn wait_for_exit(
    plan: &WindowsJobCommandPlan,
    job: &OwnedHandle,
    process: &OwnedHandle,
) -> GraphifyAdapterResult<u32> {
    let wait_millis = millis_until(plan.deadline);
    // SAFETY: `process` is a valid process handle and the timeout is bounded.
    let wait = unsafe { WaitForSingleObject(process.raw(), wait_millis) };
    match wait {
        WAIT_OBJECT_0 => {
            let mut exit_code = 0_u32;
            // SAFETY: the signaled process handle is valid and `exit_code` is
            // a valid output pointer.
            if unsafe { GetExitCodeProcess(process.raw(), &raw mut exit_code) } == 0 {
                terminate_job_and_reap(job, process, plan.teardown_timeout)?;
                return Err(spawn_error("GRAPHIFY_WINDOWS_EXIT_CODE"));
            }
            ensure_job_empty(job, process, plan.teardown_timeout)?;
            Ok(exit_code)
        }
        WAIT_TIMEOUT => {
            terminate_job_and_reap(job, process, plan.teardown_timeout)?;
            Err(GraphifyAdapterError::new(
                GraphifyAdapterErrorKind::Timeout,
                "GRAPHIFY_WINDOWS_DEADLINE_EXCEEDED",
            ))
        }
        WAIT_FAILED => {
            terminate_job_and_reap(job, process, plan.teardown_timeout)?;
            Err(spawn_error("GRAPHIFY_WINDOWS_WAIT_FAILED"))
        }
        _ => {
            terminate_job_and_reap(job, process, plan.teardown_timeout)?;
            Err(spawn_error("GRAPHIFY_WINDOWS_WAIT_UNKNOWN"))
        }
    }
}

fn ensure_job_empty(
    job: &OwnedHandle,
    process: &OwnedHandle,
    timeout: Duration,
) -> GraphifyAdapterResult<()> {
    match job_active_processes(job) {
        Ok(0) => Ok(()),
        Ok(_) | Err(()) => terminate_job_and_reap(job, process, timeout),
    }
}

fn terminate_job_and_reap(
    job: &OwnedHandle,
    process: &OwnedHandle,
    timeout: Duration,
) -> GraphifyAdapterResult<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(teardown_ambiguous)?;
    // SAFETY: `job` is a live owned Job Object handle. The return value is
    // checked; no kill failure is ignored.
    if unsafe { TerminateJobObject(job.raw(), PROCESS_TEARDOWN_EXIT_CODE) } == 0 {
        return Err(teardown_ambiguous());
    }
    // SAFETY: `process` is a live handle and the timeout is bounded.
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
) -> GraphifyAdapterResult<()> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(teardown_ambiguous)?;
    // SAFETY: the child is a live, still-suspended owned process. The result is
    // checked before the bounded reap.
    if unsafe { TerminateProcess(process.raw(), PROCESS_TEARDOWN_EXIT_CODE) } == 0 {
        return Err(teardown_ambiguous());
    }
    // SAFETY: `process` remains valid through this bounded wait.
    if unsafe { WaitForSingleObject(process.raw(), millis_until(deadline)) } != WAIT_OBJECT_0 {
        return Err(teardown_ambiguous());
    }
    Ok(())
}

fn job_active_processes(job: &OwnedHandle) -> Result<u32, ()> {
    let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    let size =
        u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>()).map_err(|_| ())?;
    // SAFETY: `accounting` is writable storage of the exact queried type.
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

fn command_line(executable: &Path, arguments: &[OsString]) -> GraphifyAdapterResult<Vec<u16>> {
    let mut result = Vec::new();
    append_quoted_argument(&mut result, executable.as_os_str())?;
    for argument in arguments {
        result.push(u16::from(b' '));
        append_quoted_argument(&mut result, argument)?;
    }
    result.push(0);
    Ok(result)
}

fn append_quoted_argument(output: &mut Vec<u16>, argument: &OsStr) -> GraphifyAdapterResult<()> {
    let encoded: Vec<u16> = argument.encode_wide().collect();
    if encoded.contains(&0) {
        return Err(spawn_error("GRAPHIFY_WINDOWS_ARGUMENT_NUL"));
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

fn environment_block(
    environment: &BTreeMap<OsString, OsString>,
) -> GraphifyAdapterResult<Vec<u16>> {
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
            return Err(spawn_error("GRAPHIFY_WINDOWS_DUPLICATE_ENVIRONMENT"));
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
            return Err(spawn_error("GRAPHIFY_WINDOWS_INVALID_ENVIRONMENT"));
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

fn wide_null(value: impl AsRef<OsStr>) -> GraphifyAdapterResult<Vec<u16>> {
    let mut wide: Vec<u16> = value.as_ref().encode_wide().collect();
    if wide.contains(&0) {
        return Err(spawn_error("GRAPHIFY_WINDOWS_PATH_NUL"));
    }
    wide.push(0);
    Ok(wide)
}

fn spawn_error(code: &'static str) -> GraphifyAdapterError {
    GraphifyAdapterError::new(GraphifyAdapterErrorKind::Spawn, code)
}

fn teardown_ambiguous() -> GraphifyAdapterError {
    GraphifyAdapterError::new(
        GraphifyAdapterErrorKind::TeardownAmbiguous,
        "GRAPHIFY_WINDOWS_TEARDOWN_AMBIGUOUS",
    )
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE) -> Option<Self> {
        (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(Self(handle))
    }

    const fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: `OwnedHandle` is constructed only from a unique owned handle
        // and closes it exactly once. Closing a configured job also supplies
        // the KILL_ON_JOB_CLOSE unwind/drop backstop.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{WindowsJobCommandPlan, run};
    use crate::GraphifyAdapterErrorKind;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "lattice-graphify-wr-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create isolated test root");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn helper_plan(
        run_root: &Path,
        environment: BTreeMap<OsString, OsString>,
        deadline: Instant,
    ) -> WindowsJobCommandPlan {
        WindowsJobCommandPlan {
            executable: system32().join("cmd.exe"),
            arguments: Vec::new(),
            current_dir: run_root.to_path_buf(),
            environment,
            run_root: run_root.to_path_buf(),
            stdout_path: run_root.join("child.stdout"),
            stderr_path: run_root.join("child.stderr"),
            stdout_limit: 1024 * 1024,
            stderr_limit: 1024 * 1024,
            deadline,
            teardown_timeout: Duration::from_secs(2),
        }
    }

    fn base_environment() -> BTreeMap<OsString, OsString> {
        let mut environment = BTreeMap::new();
        if let Some(system_root) = env::var_os("SystemRoot") {
            environment.insert(OsString::from("SystemRoot"), system_root);
        }
        environment
    }

    fn system32() -> PathBuf {
        PathBuf::from(env::var_os("SystemRoot").expect("SystemRoot")).join("System32")
    }

    #[test]
    fn owned_job_launches_and_captures_trusted_child() {
        let base = TempRoot::new("capture");
        let run_root = base.path().join("run");
        fs::create_dir(&run_root).expect("create run root");
        let mut plan = helper_plan(
            &run_root,
            base_environment(),
            Instant::now() + Duration::from_secs(10),
        );
        plan.arguments = vec![
            OsString::from("/d"),
            OsString::from("/c"),
            OsString::from("echo"),
            OsString::from("owned-job-ok"),
        ];

        let outcome = run(&plan).expect("owned job helper should complete");

        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.stdout, b"owned-job-ok\r\n");
        assert!(outcome.stderr.is_empty());
    }

    #[test]
    fn capture_path_cannot_replace_the_parent_held_handle() {
        let base = TempRoot::new("capture-race");
        let run_root = base.path().join("run");
        fs::create_dir(&run_root).expect("create run root");
        let mut plan = helper_plan(
            &run_root,
            base_environment(),
            Instant::now() + Duration::from_secs(10),
        );
        plan.arguments = vec![
            OsString::from("/d"),
            OsString::from("/c"),
            system32().join("ping.exe").into_os_string(),
            OsString::from("127.0.0.1"),
            OsString::from("-n"),
            OsString::from("3"),
            OsString::from(">nul"),
            OsString::from("&"),
            OsString::from("echo"),
            OsString::from("trusted-bytes"),
        ];
        let capture = plan.stdout_path.clone();
        let attacker = thread::spawn(move || {
            for _ in 0..200 {
                if capture.exists() {
                    return fs::OpenOptions::new().write(true).open(&capture).is_err();
                }
                thread::sleep(Duration::from_millis(10));
            }
            false
        });

        let outcome = run(&plan).expect("owned job helper should complete");

        assert!(attacker.join().expect("attacker probe"));
        assert_eq!(outcome.stdout, b"trusted-bytes\r\n");
    }

    #[test]
    fn timeout_is_known_only_after_the_job_and_descendant_are_reaped() {
        let base = TempRoot::new("timeout");
        let run_root = base.path().join("run");
        fs::create_dir(&run_root).expect("create run root");
        let marker = run_root.join("late-marker.txt");
        let mut plan = helper_plan(
            &run_root,
            base_environment(),
            Instant::now() + Duration::from_millis(1500),
        );
        plan.arguments = vec![
            OsString::from("/d"),
            OsString::from("/c"),
            system32().join("ping.exe").into_os_string(),
            OsString::from("127.0.0.1"),
            OsString::from("-n"),
            OsString::from("4"),
            OsString::from("&"),
            OsString::from("echo"),
            OsString::from(format!("survived>{}", marker.display())),
        ];

        let error = run(&plan).expect_err("the real helper must time out");

        assert_eq!(error.kind(), GraphifyAdapterErrorKind::Timeout);
        thread::sleep(Duration::from_secs(2));
        assert!(
            !marker.exists(),
            "terminated descendant wrote a late marker"
        );
    }
}
