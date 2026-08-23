//! Owned-process transport for one bounded Codex app-server turn.

use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[cfg(not(windows))]
use std::io;
#[cfg(not(windows))]
use std::process::ExitStatus;
#[cfg(not(windows))]
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout};

use crate::{
    AppServerProtocol, AppServerSession, InitializeEvidence, ProtocolError, SessionError,
    SessionRequest, TurnOutcome,
};

const MAX_STDOUT_LINE_BYTES: usize = 8 * 1024 * 1024;
const CHILD_CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(windows)]
const FS_SANDBOX_PREFLIGHT_TIMEOUT: Duration = Duration::from_mins(1);
#[cfg(windows)]
const FS_SANDBOX_READINESS_TIMEOUT: Duration = Duration::from_secs(30);
const FS_SANDBOX_TEMP_DIRECTORY_NAME: &str = ".lattice-fs-sandbox-temp-v1";
const WINDOWS_SANDBOX_READINESS_REQUEST_ID: i64 = 3;
const INTERRUPT_REQUEST_ID: i64 = 4;
const PROTOCOL_QUIET_WINDOW: Duration = Duration::from_millis(10);
/// Marker required before a directory can be admitted as LATTICE-owned Codex state.
pub const CODEX_HOME_OWNERSHIP_MARKER_NAME: &str = ".lattice-codex-home-v1";
/// Exact marker contents written only by LATTICE workspace provisioning.
pub const CODEX_HOME_OWNERSHIP_MARKER_BYTES: &[u8] = b"lattice.codex-home.v1\n";
const CODEX_HOME_CONFIG_BYTES: &[u8] = b"approval_policy = \"never\"\n\
sandbox_mode = \"workspace-write\"\n\
model = \"gpt-5.6-sol\"\n\
model_reasoning_effort = \"low\"\n\
\n\
[windows]\n\
sandbox = \"unelevated\"\n\
\n\
[features]\n\
plugins = false\n";

/// Exact managed package and helper identities admitted for one official Codex child.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedCodexResources {
    managed_package_root: PathBuf,
    resources_directory: PathBuf,
    digests: PinnedCodexResourceDigests,
}

/// Exact SHA-256 identities for every executable and manifest in one Codex bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedCodexResourceDigests {
    sandbox_setup: String,
    command_runner: String,
    code_mode_host: String,
    rg: String,
    package_manifest: String,
    managed_package_manifest: String,
}

impl PinnedCodexResourceDigests {
    /// Binds all admitted bundle resources to canonical lowercase SHA-256 text.
    ///
    /// # Errors
    ///
    /// Rejects any missing, uppercase, or malformed digest.
    pub fn new(
        sandbox_setup_sha256: impl Into<String>,
        command_runner_sha256: impl Into<String>,
        code_mode_host_sha256: impl Into<String>,
        rg_sha256: impl Into<String>,
        package_manifest_sha256: impl Into<String>,
        managed_package_manifest_sha256: impl Into<String>,
    ) -> Result<Self, AppServerRunError> {
        let digests = Self {
            sandbox_setup: sandbox_setup_sha256.into(),
            command_runner: command_runner_sha256.into(),
            code_mode_host: code_mode_host_sha256.into(),
            rg: rg_sha256.into(),
            package_manifest: package_manifest_sha256.into(),
            managed_package_manifest: managed_package_manifest_sha256.into(),
        };
        if [
            digests.sandbox_setup.as_str(),
            digests.command_runner.as_str(),
            digests.code_mode_host.as_str(),
            digests.rg.as_str(),
            digests.package_manifest.as_str(),
            digests.managed_package_manifest.as_str(),
        ]
        .into_iter()
        .any(|digest| !is_lowercase_sha256(digest))
        {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidPinnedResources,
            ));
        }
        Ok(digests)
    }
}

pub(crate) struct OwnedSandboxTemp {
    path: PathBuf,
}

impl OwnedSandboxTemp {
    pub(crate) fn prepare(codex_home: &Path) -> Result<Self, AppServerRunError> {
        let home_metadata = std::fs::symlink_metadata(codex_home)
            .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidCodexHome))?;
        if !home_metadata.file_type().is_dir() || metadata_is_reparse(&home_metadata) {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidCodexHome,
            ));
        }
        let path = codex_home.join(FS_SANDBOX_TEMP_DIRECTORY_NAME);
        std::fs::create_dir(&path)
            .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidCodexHome))?;
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidCodexHome))?;
        let is_empty = std::fs::read_dir(&path)
            .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidCodexHome))?
            .next()
            .is_none();
        if !metadata.file_type().is_dir() || metadata_is_reparse(&metadata) || !is_empty {
            let _ = std::fs::remove_dir(&path);
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidCodexHome,
            ));
        }
        Ok(Self { path })
    }

    pub(crate) fn configure(&self, command: &mut Command) {
        command
            .env("TMP", &self.path)
            .env("TEMP", &self.path)
            .env("TMPDIR", &self.path);
    }

    pub(crate) fn cleanup(self) -> Result<(), AppServerRunError> {
        let metadata = std::fs::symlink_metadata(&self.path)
            .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::ChildCleanupFailed))?;
        if !metadata.file_type().is_dir() || metadata_is_reparse(&metadata) {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::ChildCleanupFailed,
            ));
        }
        std::fs::remove_dir_all(&self.path)
            .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::ChildCleanupFailed))?;
        if self.path.exists() {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::ChildCleanupFailed,
            ));
        }
        Ok(())
    }
}

impl PinnedCodexResources {
    /// Binds an absolute managed package root to exact helper and manifest digests.
    ///
    /// # Errors
    ///
    /// Rejects a relative root or non-canonical SHA-256 text.
    pub fn new(
        managed_package_root: PathBuf,
        resources_directory: PathBuf,
        digests: PinnedCodexResourceDigests,
    ) -> Result<Self, AppServerRunError> {
        if !managed_package_root.is_absolute() || !resources_directory.is_absolute() {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidPinnedResources,
            ));
        }
        Ok(Self {
            managed_package_root,
            resources_directory,
            digests,
        })
    }

    #[must_use]
    pub fn managed_package_root(&self) -> &Path {
        &self.managed_package_root
    }

    #[must_use]
    pub fn resources_directory(&self) -> &Path {
        &self.resources_directory
    }
}

/// Exact inputs for one supervised app-server turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppServerRunConfig {
    launcher: PathBuf,
    expected_launcher_sha256: String,
    codex_home: PathBuf,
    working_directory: PathBuf,
    prompt: String,
    timeout: Duration,
    pinned_resources: Option<PinnedCodexResources>,
}

impl AppServerRunConfig {
    /// Creates one bounded process request after validating its immutable inputs.
    ///
    /// # Errors
    ///
    /// Rejects relative paths, a blank prompt, or a zero timeout.
    pub fn new(
        launcher: PathBuf,
        expected_launcher_sha256: impl Into<String>,
        codex_home: PathBuf,
        working_directory: PathBuf,
        prompt: impl Into<String>,
        timeout: Duration,
        pinned_resources: Option<PinnedCodexResources>,
    ) -> Result<Self, AppServerRunError> {
        let expected_launcher_sha256 = expected_launcher_sha256.into();
        let prompt = prompt.into();
        if !launcher.is_absolute() {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidLauncher,
            ));
        }
        if !is_lowercase_sha256(&expected_launcher_sha256) {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidLauncherSha256,
            ));
        }
        if !codex_home.is_absolute() {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidCodexHome,
            ));
        }
        if !working_directory.is_absolute() {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidWorkingDirectory,
            ));
        }
        if prompt.trim().is_empty() {
            return Err(AppServerRunError::new(AppServerRunErrorKind::InvalidPrompt));
        }
        if timeout.is_zero() {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidTimeout,
            ));
        }
        Ok(Self {
            launcher,
            expected_launcher_sha256,
            codex_home,
            working_directory,
            prompt,
            timeout,
            pinned_resources,
        })
    }

    #[must_use]
    pub fn launcher(&self) -> &Path {
        &self.launcher
    }

    #[must_use]
    pub fn expected_launcher_sha256(&self) -> &str {
        &self.expected_launcher_sha256
    }

    #[must_use]
    pub fn codex_home(&self) -> &Path {
        &self.codex_home
    }

    #[must_use]
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    #[must_use]
    pub const fn pinned_resources(&self) -> Option<&PinnedCodexResources> {
        self.pinned_resources.as_ref()
    }
}

/// Stable fail-closed process failure classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppServerRunErrorKind {
    InvalidLauncher,
    InvalidLauncherSha256,
    InvalidCodexHome,
    CodexHomeOwnershipMissing,
    CodexHomeOverlap,
    AmbientCodexHomeDenied,
    InvalidWorkingDirectory,
    InvalidPrompt,
    InvalidTimeout,
    InvalidPinnedResources,
    PinnedResourcesMissing,
    PinnedResourcesDigestMismatch,
    PinnedResourcesChanged,
    PinnedResourcePathInvalid,
    LauncherReadFailed,
    LauncherDigestMismatch,
    LauncherChanged,
    SpawnFailed,
    PipeUnavailable,
    WriteFailed,
    StdoutFailed,
    StdoutLineTooLarge,
    ProtocolFailed,
    IncompleteToolExecution,
    CodexHomeMismatch,
    FsSandboxBootstrapFailed,
    FsSandboxHelperTimeout,
    Timeout,
    AmbiguousEof,
    ChildCleanupFailed,
    JobObjectFailed,
}

/// Payload-free process failure safe for durable diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppServerRunError {
    kind: AppServerRunErrorKind,
}

impl AppServerRunError {
    pub(crate) const fn new(kind: AppServerRunErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> AppServerRunErrorKind {
        self.kind
    }
}

impl fmt::Display for AppServerRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CODEX_APP_SERVER_{:?}", self.kind)
    }
}

impl Error for AppServerRunError {}

/// Unambiguous evidence returned after the owned child reaches a terminal turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppServerRunEvidence {
    initialize: InitializeEvidence,
    thread_id: String,
    turn_id: String,
    outcome: TurnOutcome,
}

impl AppServerRunEvidence {
    #[must_use]
    pub const fn initialize(&self) -> &InitializeEvidence {
        &self.initialize
    }

    #[must_use]
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    #[must_use]
    pub fn turn_id(&self) -> &str {
        &self.turn_id
    }

    #[must_use]
    pub const fn outcome(&self) -> &TurnOutcome {
        &self.outcome
    }
}

enum ReaderEvent {
    Line(String),
    Eof,
    Failed,
    LineTooLarge,
}

/// Runs one real Codex app-server child and owns its complete lifecycle.
///
/// # Errors
///
/// Fails closed for invalid paths, process/pipe failures, malformed protocol,
/// timeout, EOF before a terminal turn, or inability to stop the owned child.
pub fn run_codex_app_server(
    config: &AppServerRunConfig,
) -> Result<AppServerRunEvidence, AppServerRunError> {
    let deadline = Instant::now()
        .checked_add(config.timeout())
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::InvalidTimeout))?;
    run_codex_app_server_until(config, deadline)
}

/// Runs one supervised app-server child under a caller-owned absolute deadline.
///
/// # Errors
///
/// Fails before spawn when the deadline has expired and preserves that same
/// deadline through validation, process setup, and protocol driving. Timeout
/// interruption and owned-process teardown use their own fixed cleanup bounds.
pub fn run_codex_app_server_until(
    config: &AppServerRunConfig,
    deadline: Instant,
) -> Result<AppServerRunEvidence, AppServerRunError> {
    ensure_before_deadline(deadline)?;
    validate_live_paths(config)?;
    ensure_before_deadline(deadline)?;
    let before_spawn = launcher_sha256(config.launcher())?;
    if before_spawn != config.expected_launcher_sha256() {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::LauncherDigestMismatch,
        ));
    }
    ensure_before_deadline(deadline)?;
    let sandbox_temp = config
        .pinned_resources()
        .map(|_| OwnedSandboxTemp::prepare(config.codex_home()))
        .transpose()?;
    let result =
        run_app_server_with_sandbox_temp(config, sandbox_temp.as_ref(), &before_spawn, deadline);
    let cleanup = cleanup_sandbox_temp(sandbox_temp);
    cleanup?;
    result
}

fn run_app_server_with_sandbox_temp(
    config: &AppServerRunConfig,
    sandbox_temp: Option<&OwnedSandboxTemp>,
    before_spawn: &str,
    deadline: Instant,
) -> Result<AppServerRunEvidence, AppServerRunError> {
    #[cfg(windows)]
    let preflight = sandbox_temp.map_or(Ok(()), |sandbox_temp| {
        run_windows_sandbox_preflight(config, sandbox_temp, before_spawn, deadline)
    });
    #[cfg(windows)]
    preflight?;
    ensure_before_deadline(deadline)?;
    verify_app_server_identity(config, before_spawn)?;
    ensure_before_deadline(deadline)?;
    let mut command = Command::new(config.launcher());
    crate::scrub_protected_environment(&mut command);
    configure_pinned_child_environment(&mut command, config.pinned_resources())?;
    command
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(config.working_directory())
        .env("CODEX_HOME", config.codex_home())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ensure_before_deadline(deadline)?;
    if let Some(sandbox_temp) = sandbox_temp {
        sandbox_temp.configure(&mut command);
    }
    let mut child = spawn_owned_child(&mut command, OwnedChildStdio::Duplex)?;
    let result = ensure_before_deadline(deadline)
        .and_then(|()| verify_app_server_identity(config, before_spawn))
        .and_then(|()| ensure_before_deadline(deadline))
        .and_then(|()| drive_child(&mut child, config, deadline));
    let cleanup = stop_owned_child(&mut child);
    cleanup?;
    result
}

fn verify_app_server_identity(
    config: &AppServerRunConfig,
    before_spawn: &str,
) -> Result<(), AppServerRunError> {
    let after_spawn = launcher_sha256(config.launcher())?;
    if after_spawn != before_spawn {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::LauncherChanged,
        ));
    }
    validate_pinned_resources(config)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::PinnedResourcesChanged))
}

#[cfg(windows)]
fn run_windows_sandbox_preflight(
    config: &AppServerRunConfig,
    sandbox_temp: &OwnedSandboxTemp,
    expected_launcher_digest: &str,
    caller_deadline: Instant,
) -> Result<(), AppServerRunError> {
    let bootstrap_deadline = Instant::now()
        .checked_add(FS_SANDBOX_PREFLIGHT_TIMEOUT)
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::InvalidTimeout))?;
    ensure_before_sandbox_deadline(caller_deadline, bootstrap_deadline)?;
    validate_pinned_resources(config)?;
    let mut command = Command::new(config.launcher());
    crate::scrub_protected_environment(&mut command);
    configure_pinned_child_environment(&mut command, config.pinned_resources())?;
    sandbox_temp.configure(&mut command);
    command
        .args(["sandbox", "-P", ":workspace", "-C"])
        .arg(config.working_directory())
        .args(["cmd.exe", "/d", "/c", "exit 0"])
        .current_dir(config.working_directory())
        .env("CODEX_HOME", config.codex_home())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    ensure_before_sandbox_deadline(caller_deadline, bootstrap_deadline)?;
    let mut child = spawn_owned_child(&mut command, OwnedChildStdio::Null).map_err(|error| {
        if error.kind() == AppServerRunErrorKind::SpawnFailed {
            AppServerRunError::new(AppServerRunErrorKind::FsSandboxBootstrapFailed)
        } else {
            error
        }
    })?;
    let post_attach = ensure_before_sandbox_deadline(caller_deadline, bootstrap_deadline)
        .and_then(|()| verify_preflight_identity(config, expected_launcher_digest))
        .and_then(|()| ensure_before_sandbox_deadline(caller_deadline, bootstrap_deadline));
    let result = post_attach
        .and_then(|()| wait_for_sandbox_preflight(&mut child, caller_deadline, bootstrap_deadline));
    let cleanup = stop_owned_child(&mut child);
    cleanup?;
    result
}

#[cfg(windows)]
fn verify_preflight_identity(
    config: &AppServerRunConfig,
    expected_launcher_digest: &str,
) -> Result<(), AppServerRunError> {
    let after_spawn = launcher_sha256(config.launcher())?;
    if after_spawn != expected_launcher_digest {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::LauncherChanged,
        ));
    }
    validate_pinned_resources(config)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::PinnedResourcesChanged))
}

#[cfg(windows)]
fn wait_for_sandbox_preflight(
    child: &mut OwnedChild,
    caller_deadline: Instant,
    bootstrap_deadline: Instant,
) -> Result<(), AppServerRunError> {
    let deadline = bootstrap_deadline.min(caller_deadline);
    loop {
        ensure_before_sandbox_deadline(caller_deadline, bootstrap_deadline)?;
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) | Err(_) => {
                return Err(AppServerRunError::new(
                    AppServerRunErrorKind::FsSandboxBootstrapFailed,
                ));
            }
            Ok(None) => {}
        }
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| readiness_timeout(caller_deadline, bootstrap_deadline))?;
        std::thread::sleep(Duration::from_millis(10).min(remaining));
    }
}

#[cfg(windows)]
fn ensure_before_sandbox_deadline(
    caller_deadline: Instant,
    bootstrap_deadline: Instant,
) -> Result<(), AppServerRunError> {
    if Instant::now() < caller_deadline.min(bootstrap_deadline) {
        Ok(())
    } else {
        Err(readiness_timeout(caller_deadline, bootstrap_deadline))
    }
}

pub(crate) fn cleanup_sandbox_temp(
    sandbox_temp: Option<OwnedSandboxTemp>,
) -> Result<(), AppServerRunError> {
    sandbox_temp.map_or(Ok(()), OwnedSandboxTemp::cleanup)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedChildStdio {
    Duplex,
    Stdout,
    Null,
}

#[cfg(windows)]
pub(crate) use windows_job::OwnedChild;

#[cfg(windows)]
pub(crate) fn spawn_owned_child(
    command: &mut Command,
    stdio: OwnedChildStdio,
) -> Result<OwnedChild, AppServerRunError> {
    windows_job::spawn(command, stdio)
}

#[cfg(all(test, windows))]
fn spawn_windows_owned_command_with_pre_resume(
    command: &Command,
    stdio: OwnedChildStdio,
    pre_resume: impl FnOnce() -> Result<(), AppServerRunError>,
) -> Result<OwnedChild, AppServerRunError> {
    windows_job::spawn_with_pre_resume(command, stdio, pre_resume)
}

#[cfg(not(windows))]
type OwnedChildStdin = ChildStdin;
#[cfg(not(windows))]
type OwnedChildStdout = ChildStdout;
#[cfg(not(windows))]
type OwnedChildStderr = ChildStderr;

#[cfg(not(windows))]
#[derive(Debug)]
pub(crate) struct OwnedChild {
    child: Child,
    terminated: bool,
}

#[cfg(not(windows))]
impl OwnedChild {
    pub(crate) fn take_stdin(&mut self) -> Option<OwnedChildStdin> {
        self.child.stdin.take()
    }

    pub(crate) fn take_stdout(&mut self) -> Option<OwnedChildStdout> {
        self.child.stdout.take()
    }

    pub(crate) fn take_stderr(&mut self) -> Option<OwnedChildStderr> {
        self.child.stderr.take()
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    fn terminate_and_reap(&mut self) -> Result<(), AppServerRunError> {
        if self.terminated {
            return Ok(());
        }
        terminate_native_child_bounded(&mut self.child)?;
        self.terminated = true;
        Ok(())
    }
}

#[cfg(not(windows))]
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !self.terminated {
            let _ = terminate_native_child_bounded(&mut self.child);
            self.terminated = true;
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn spawn_owned_child(
    command: &mut Command,
    stdio: OwnedChildStdio,
) -> Result<OwnedChild, AppServerRunError> {
    let _ = stdio;
    command
        .spawn()
        .map(|child| OwnedChild {
            child,
            terminated: false,
        })
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::SpawnFailed))
}

pub(crate) fn stop_owned_child(child: &mut OwnedChild) -> Result<(), AppServerRunError> {
    child.terminate_and_reap()
}

#[cfg(not(windows))]
fn terminate_native_child_bounded(child: &mut Child) -> Result<(), AppServerRunError> {
    match child.try_wait() {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(_) => {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::ChildCleanupFailed,
            ));
        }
    }
    child
        .kill()
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::ChildCleanupFailed))?;
    let deadline = Instant::now()
        .checked_add(CHILD_CLEANUP_TIMEOUT)
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::ChildCleanupFailed))?;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(_) => {
                return Err(AppServerRunError::new(
                    AppServerRunErrorKind::ChildCleanupFailed,
                ));
            }
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::ChildCleanupFailed,
            ));
        };
        std::thread::sleep(Duration::from_millis(10).min(remaining));
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod windows_job {
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::os::windows::process::ExitStatusExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, ExitStatus};
    use std::ptr::{null, null_mut};
    use std::thread;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
        SetHandleInformation, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
        QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread, STARTF_USESTDHANDLES,
        STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
    };

    use super::{AppServerRunError, AppServerRunErrorKind, CHILD_CLEANUP_TIMEOUT, OwnedChildStdio};

    const PROCESS_TEARDOWN_EXIT_CODE: u32 = 0xC0DE_0380;

    /// The child is created suspended, assigned to its private kill-on-close
    /// Job, and resumed only after assignment. The Job handle stays live until
    /// bounded accounting proves `ActiveProcesses == 0`.
    pub(crate) struct OwnedChild {
        job: OwnedHandle,
        process: OwnedHandle,
        stdin: Option<File>,
        stdout: Option<File>,
        stderr: Option<File>,
        terminated: bool,
    }

    impl std::fmt::Debug for OwnedChild {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("OwnedChild")
                .field("terminated", &self.terminated)
                .finish_non_exhaustive()
        }
    }

    impl OwnedChild {
        pub(crate) fn take_stdin(&mut self) -> Option<File> {
            self.stdin.take()
        }

        pub(crate) fn take_stdout(&mut self) -> Option<File> {
            self.stdout.take()
        }

        pub(crate) fn take_stderr(&mut self) -> Option<File> {
            self.stderr.take()
        }

        pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
            // SAFETY: process is a live owned process handle; zero timeout is
            // observation-only.
            match unsafe { WaitForSingleObject(self.process.raw(), 0) } {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut exit_code = 0_u32;
                    // SAFETY: the signaled handle and output pointer are valid.
                    if unsafe { GetExitCodeProcess(self.process.raw(), &raw mut exit_code) } == 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(Some(ExitStatus::from_raw(exit_code)))
                    }
                }
                WAIT_FAILED => Err(io::Error::last_os_error()),
                _ => Err(io::Error::other("unexpected Windows process wait result")),
            }
        }

        pub(crate) fn terminate_and_reap(&mut self) -> Result<(), AppServerRunError> {
            if self.terminated {
                return Ok(());
            }
            ensure_job_empty(&self.job, &self.process, CHILD_CLEANUP_TIMEOUT)?;
            self.terminated = true;
            Ok(())
        }

        #[cfg(test)]
        pub(crate) fn active_processes(&self) -> Result<u32, AppServerRunError> {
            job_active_processes(&self.job).map_err(|()| cleanup_error())
        }
    }

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            if !self.terminated {
                let _ = terminate_job_and_reap(&self.job, &self.process, CHILD_CLEANUP_TIMEOUT);
                self.terminated = true;
            }
        }
    }

    pub(crate) fn spawn(
        command: &Command,
        stdio: OwnedChildStdio,
    ) -> Result<OwnedChild, AppServerRunError> {
        spawn_with_pre_resume(command, stdio, || Ok(()))
    }

    pub(crate) fn spawn_with_pre_resume(
        command: &Command,
        stdio: OwnedChildStdio,
        pre_resume: impl FnOnce() -> Result<(), AppServerRunError>,
    ) -> Result<OwnedChild, AppServerRunError> {
        let redirects = RedirectHandles::create(stdio)?;
        let job = create_kill_on_close_job()?;
        let attributes = ProcThreadAttributes::for_handles(redirects.child_handles())?;
        let executable_path = PathBuf::from(command.get_program());
        if !executable_path.is_absolute() {
            return Err(spawn_error());
        }
        let mut command_line = command_line(
            command.get_program(),
            command.get_args().map(OsStr::to_os_string),
        )?;
        let environment = command_environment(command)?;
        let executable = wide_null(command.get_program())?;
        let current_directory = command
            .get_current_dir()
            .map(Path::to_path_buf)
            .map_or_else(std::env::current_dir, Ok)
            .map_err(|_| spawn_error())?;
        let current_directory = wide_null(non_verbatim_path(&current_directory)?)?;

        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb =
            u32::try_from(size_of::<STARTUPINFOEXW>()).map_err(|_| spawn_error())?;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = redirects.child_stdin.raw();
        startup.StartupInfo.hStdOutput = redirects.child_stdout.raw();
        startup.StartupInfo.hStdError = redirects.child_stderr.raw();
        startup.lpAttributeList = attributes.raw();

        // SAFETY: every pointer references live storage through this call. The
        // mutable command line and double-NUL environment meet CreateProcessW
        // contracts, and only the three standard handles are inherited.
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
                current_directory.as_ptr(),
                &raw const startup.StartupInfo,
                &raw mut process_info,
            )
        };
        if created == 0 {
            return Err(spawn_error());
        }

        let process = owned_handle(process_info.hProcess).ok_or_else(containment_error)?;
        let Some(thread_handle) = owned_handle(process_info.hThread) else {
            terminate_unassigned_process(&process, CHILD_CLEANUP_TIMEOUT)?;
            return Err(containment_error());
        };
        drop(attributes);

        // SAFETY: the primary process is still suspended, so assignment
        // precedes its first instruction and any descendant creation.
        if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
            terminate_unassigned_process(&process, CHILD_CLEANUP_TIMEOUT)?;
            return Err(containment_error());
        }
        if let Err(error) = pre_resume() {
            terminate_job_and_reap(&job, &process, CHILD_CLEANUP_TIMEOUT)?;
            return Err(error);
        }

        // SAFETY: this is the retained suspended primary thread returned by
        // CreateProcessW and it has not yet been resumed.
        if unsafe { ResumeThread(thread_handle.raw()) } == u32::MAX {
            terminate_job_and_reap(&job, &process, CHILD_CLEANUP_TIMEOUT)?;
            return Err(containment_error());
        }
        drop(thread_handle);

        let RedirectHandles {
            child_stdin,
            child_stdout,
            child_stderr,
            parent_stdin,
            parent_stdout,
            parent_stderr,
        } = redirects;
        drop(child_stdin);
        drop(child_stdout);
        drop(child_stderr);
        Ok(OwnedChild {
            job,
            process,
            stdin: parent_stdin.map(File::from),
            stdout: parent_stdout.map(File::from),
            stderr: parent_stderr.map(File::from),
            terminated: false,
        })
    }

    struct RedirectHandles {
        child_stdin: OwnedHandle,
        child_stdout: OwnedHandle,
        child_stderr: OwnedHandle,
        parent_stdin: Option<OwnedHandle>,
        parent_stdout: Option<OwnedHandle>,
        parent_stderr: Option<OwnedHandle>,
    }

    impl RedirectHandles {
        fn create(stdio: OwnedChildStdio) -> Result<Self, AppServerRunError> {
            let (child_stdin, parent_stdin) = if stdio == OwnedChildStdio::Duplex {
                let (child_reader, parent_writer) = create_anonymous_pipe(true)?;
                (child_reader, Some(parent_writer))
            } else {
                (open_null(GENERIC_READ)?, None)
            };
            let (child_stdout, parent_stdout) =
                if matches!(stdio, OwnedChildStdio::Duplex | OwnedChildStdio::Stdout) {
                    let (parent_reader, child_writer) = create_anonymous_pipe(false)?;
                    (child_writer, Some(parent_reader))
                } else {
                    (open_null(GENERIC_WRITE)?, None)
                };
            let (child_stderr, parent_stderr) = if stdio == OwnedChildStdio::Duplex {
                let (parent_reader, child_writer) = create_anonymous_pipe(false)?;
                (child_writer, Some(parent_reader))
            } else {
                (open_null(GENERIC_WRITE)?, None)
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

        fn child_handles(&self) -> [HANDLE; 3] {
            [
                self.child_stdin.raw(),
                self.child_stdout.raw(),
                self.child_stderr.raw(),
            ]
        }
    }

    fn create_anonymous_pipe(
        parent_is_writer: bool,
    ) -> Result<(OwnedHandle, OwnedHandle), AppServerRunError> {
        let attributes = inheritable_security_attributes()?;
        let mut read_handle: HANDLE = null_mut();
        let mut write_handle: HANDLE = null_mut();
        // SAFETY: output pointers and security attributes are valid.
        if unsafe {
            CreatePipe(
                &raw mut read_handle,
                &raw mut write_handle,
                &raw const attributes,
                0,
            )
        } == 0
        {
            return Err(spawn_error());
        }
        let reader = owned_handle(read_handle).ok_or_else(spawn_error)?;
        let writer = owned_handle(write_handle).ok_or_else(spawn_error)?;
        let parent = if parent_is_writer {
            writer.raw()
        } else {
            reader.raw()
        };
        // SAFETY: this is the live parent-owned end and must not be inherited.
        if unsafe { SetHandleInformation(parent, HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(spawn_error());
        }
        Ok((reader, writer))
    }

    fn open_null(desired_access: u32) -> Result<OwnedHandle, AppServerRunError> {
        let path = wide_null(OsStr::new("NUL"))?;
        let attributes = inheritable_security_attributes()?;
        // SAFETY: path and attributes remain live; the returned handle is
        // transferred exactly once into OwnedHandle.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                desired_access,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                &raw const attributes,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        owned_handle(handle).ok_or_else(spawn_error)
    }

    fn inheritable_security_attributes() -> Result<SECURITY_ATTRIBUTES, AppServerRunError> {
        Ok(SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).map_err(|_| spawn_error())?,
            lpSecurityDescriptor: null_mut(),
            bInheritHandle: 1,
        })
    }

    fn create_kill_on_close_job() -> Result<OwnedHandle, AppServerRunError> {
        // SAFETY: unnamed Job creation uses no caller pointers.
        let job = unsafe { CreateJobObjectW(null(), null()) };
        let job = owned_handle(job).ok_or_else(containment_error)?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(|_| containment_error())?;
        // SAFETY: job and immutable limit storage are valid.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size,
            )
        } == 0
        {
            return Err(containment_error());
        }
        Ok(job)
    }

    struct ProcThreadAttributes {
        storage: Vec<usize>,
        handles: StableHandleList,
    }

    /// `UpdateProcThreadAttribute` retains this pointer until process creation;
    /// heap ownership keeps it stable when the attribute owner itself moves.
    struct StableHandleList(Box<[HANDLE; 3]>);

    impl StableHandleList {
        fn new(handles: [HANDLE; 3]) -> Self {
            Self(Box::new(handles))
        }

        fn as_ptr(&self) -> *const HANDLE {
            self.0.as_ptr()
        }
    }

    impl ProcThreadAttributes {
        fn for_handles(handles: [HANDLE; 3]) -> Result<Self, AppServerRunError> {
            let mut bytes = 0_usize;
            // SAFETY: null is the documented sizing probe for one attribute.
            unsafe {
                InitializeProcThreadAttributeList(null_mut(), 1, 0, &raw mut bytes);
            }
            if bytes == 0 {
                return Err(spawn_error());
            }
            let words = bytes
                .checked_add(size_of::<usize>() - 1)
                .map(|value| value / size_of::<usize>())
                .ok_or_else(spawn_error)?;
            let mut storage = vec![0_usize; words];
            let raw = storage.as_mut_ptr().cast();
            // SAFETY: storage is aligned and at least the probed byte size.
            if unsafe { InitializeProcThreadAttributeList(raw, 1, 0, &raw mut bytes) } == 0 {
                return Err(spawn_error());
            }
            let result = Self {
                storage,
                handles: StableHandleList::new(handles),
            };
            let attribute =
                usize::try_from(PROC_THREAD_ATTRIBUTE_HANDLE_LIST).map_err(|_| spawn_error())?;
            // SAFETY: the initialized list and heap-owned handle payload stay
            // live at stable addresses through CreateProcessW; Drop deletes
            // the attribute list before either backing allocation is released.
            if unsafe {
                UpdateProcThreadAttribute(
                    result.raw(),
                    0,
                    attribute,
                    result.handles.as_ptr().cast(),
                    size_of::<[HANDLE; 3]>(),
                    null_mut(),
                    null(),
                )
            } == 0
            {
                return Err(spawn_error());
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

    fn ensure_job_empty(
        job: &OwnedHandle,
        process: &OwnedHandle,
        timeout: Duration,
    ) -> Result<(), AppServerRunError> {
        match job_active_processes(job) {
            Ok(0) => {
                // SAFETY: zero active Job members means the primary has
                // exited; this bounded observation also proves its handle is
                // signaled before cleanup returns.
                if unsafe { WaitForSingleObject(process.raw(), millis(timeout)) } == WAIT_OBJECT_0 {
                    Ok(())
                } else {
                    Err(cleanup_error())
                }
            }
            Ok(_) | Err(()) => terminate_job_and_reap(job, process, timeout),
        }
    }

    fn terminate_job_and_reap(
        job: &OwnedHandle,
        process: &OwnedHandle,
        timeout: Duration,
    ) -> Result<(), AppServerRunError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(cleanup_error)?;
        // SAFETY: the retained Job is live. No PID/name/tree scan authority is
        // used; this exact owned handle terminates all members.
        if unsafe { TerminateJobObject(job.raw(), PROCESS_TEARDOWN_EXIT_CODE) } == 0 {
            return Err(cleanup_error());
        }
        // SAFETY: the primary process handle remains owned through this wait.
        if unsafe { WaitForSingleObject(process.raw(), millis_until(deadline)) } != WAIT_OBJECT_0 {
            return Err(cleanup_error());
        }
        loop {
            match job_active_processes(job) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(()) => return Err(cleanup_error()),
            }
            if Instant::now() >= deadline {
                return Err(cleanup_error());
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn terminate_unassigned_process(
        process: &OwnedHandle,
        timeout: Duration,
    ) -> Result<(), AppServerRunError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(cleanup_error)?;
        // SAFETY: the exact process is live, owned, and still suspended.
        if unsafe { TerminateProcess(process.raw(), PROCESS_TEARDOWN_EXIT_CODE) } == 0 {
            return Err(cleanup_error());
        }
        // SAFETY: the same owned process handle remains valid for this reap.
        if unsafe { WaitForSingleObject(process.raw(), millis_until(deadline)) } == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(cleanup_error())
        }
    }

    fn job_active_processes(job: &OwnedHandle) -> Result<u32, ()> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        let size =
            u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>()).map_err(|_| ())?;
        // SAFETY: accounting is writable storage of the exact queried type.
        if unsafe {
            QueryInformationJobObject(
                job.raw(),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size,
                null_mut(),
            )
        } == 0
        {
            Err(())
        } else {
            Ok(accounting.ActiveProcesses)
        }
    }

    fn command_line(
        executable: &OsStr,
        arguments: impl IntoIterator<Item = OsString>,
    ) -> Result<Vec<u16>, AppServerRunError> {
        let mut result = Vec::new();
        append_quoted_argument(&mut result, executable)?;
        for argument in arguments {
            result.push(u16::from(b' '));
            append_quoted_argument(&mut result, &argument)?;
        }
        result.push(0);
        Ok(result)
    }

    fn append_quoted_argument(
        output: &mut Vec<u16>,
        argument: &OsStr,
    ) -> Result<(), AppServerRunError> {
        let encoded: Vec<u16> = argument.encode_wide().collect();
        if encoded.contains(&0) {
            return Err(spawn_error());
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

    fn command_environment(command: &Command) -> Result<Vec<u16>, AppServerRunError> {
        // Windows may expose `=C:`-style current-directory records. They are
        // shell bookkeeping, not application configuration, and are omitted
        // from this explicit child environment block.
        let mut environment = std::env::vars_os()
            .filter(|(name, _)| !name.to_string_lossy().starts_with('='))
            .collect::<Vec<_>>();
        for (name, value) in command.get_envs() {
            environment.retain(|(existing, _)| {
                !existing
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&name.to_string_lossy())
            });
            if let Some(value) = value {
                environment.push((name.to_os_string(), value.to_os_string()));
            }
        }
        environment.sort_by(|(left, _), (right, _)| {
            left.to_string_lossy()
                .to_ascii_uppercase()
                .cmp(&right.to_string_lossy().to_ascii_uppercase())
        });
        for pair in environment.windows(2) {
            if pair[0]
                .0
                .to_string_lossy()
                .eq_ignore_ascii_case(&pair[1].0.to_string_lossy())
            {
                return Err(spawn_error());
            }
        }

        let mut block = Vec::new();
        for (name, value) in environment {
            let name: Vec<u16> = name.encode_wide().collect();
            let value: Vec<u16> = value.encode_wide().collect();
            if name.is_empty()
                || name.contains(&0)
                || value.contains(&0)
                || name.contains(&u16::from(b'='))
            {
                return Err(spawn_error());
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

    fn wide_null(value: impl AsRef<OsStr>) -> Result<Vec<u16>, AppServerRunError> {
        let mut wide: Vec<u16> = value.as_ref().encode_wide().collect();
        if wide.contains(&0) {
            return Err(spawn_error());
        }
        wide.push(0);
        Ok(wide)
    }

    fn non_verbatim_path(path: &Path) -> Result<PathBuf, AppServerRunError> {
        let text = path.as_os_str().to_string_lossy();
        if let Some(without_prefix) = text.strip_prefix(r"\\?\") {
            if without_prefix.starts_with("UNC\\") {
                return Err(spawn_error());
            }
            return Ok(PathBuf::from(without_prefix));
        }
        Ok(path.to_path_buf())
    }

    fn millis(duration: Duration) -> u32 {
        u32::try_from(duration.as_millis()).unwrap_or(u32::MAX - 1)
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

    fn owned_handle(handle: HANDLE) -> Option<OwnedHandle> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            return None;
        }
        // SAFETY: successful Win32 creation transfers one unique handle.
        Some(unsafe { OwnedHandle::from_raw_handle(handle.cast()) })
    }

    trait OwnedHandleExt {
        fn raw(&self) -> HANDLE;
    }

    impl OwnedHandleExt for OwnedHandle {
        fn raw(&self) -> HANDLE {
            self.as_raw_handle().cast()
        }
    }

    const fn spawn_error() -> AppServerRunError {
        AppServerRunError::new(AppServerRunErrorKind::SpawnFailed)
    }

    const fn containment_error() -> AppServerRunError {
        AppServerRunError::new(AppServerRunErrorKind::JobObjectFailed)
    }

    const fn cleanup_error() -> AppServerRunError {
        AppServerRunError::new(AppServerRunErrorKind::ChildCleanupFailed)
    }

    #[cfg(test)]
    mod tests {
        use super::{HANDLE, StableHandleList};

        #[test]
        fn inherited_handle_payload_address_survives_owner_moves() {
            let handles = StableHandleList::new([std::ptr::null_mut::<_>() as HANDLE; 3]);
            let address = handles.as_ptr();
            let moved = Some(handles);

            assert_eq!(
                moved.as_ref().expect("moved handle owner").as_ptr(),
                address,
                "UpdateProcThreadAttribute payload moved before CreateProcessW"
            );
        }
    }
}

fn drive_child(
    child: &mut OwnedChild,
    config: &AppServerRunConfig,
    deadline: Instant,
) -> Result<AppServerRunEvidence, AppServerRunError> {
    let mut stdin = child
        .take_stdin()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::PipeUnavailable))?;
    let stdout = child
        .take_stdout()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::PipeUnavailable))?;
    let stderr = child
        .take_stderr()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::PipeUnavailable))?;

    let receiver = start_readers(stdout, stderr);

    let protocol = AppServerProtocol::new("lattice_devos", "0.1.0");
    let mut session = AppServerSession::new();

    ensure_before_deadline(deadline)?;
    session
        .mark_request_sent(SessionRequest::Initialize)
        .map_err(|error| map_session_error(&error))?;
    send_json(&mut stdin, &protocol.initialize_request(0))?;
    receive_until(&receiver, deadline, &mut session, |session| {
        session.initialize_evidence().is_some()
    })?;
    let initialize = require_matching_codex_home(&session, config)?;
    drain_available(&receiver, deadline, &mut session)?;

    ensure_before_deadline(deadline)?;
    send_json(&mut stdin, &protocol.initialized_notification())?;
    #[cfg(windows)]
    if config.pinned_resources().is_some() {
        send_json(
            &mut stdin,
            &json!({
                "method": "windowsSandbox/readiness",
                "id": WINDOWS_SANDBOX_READINESS_REQUEST_ID
            }),
        )?;
        receive_windows_sandbox_readiness(&receiver, deadline, &mut session)?;
    }
    session
        .mark_request_sent(SessionRequest::ThreadStart)
        .map_err(|error| map_session_error(&error))?;
    send_json(
        &mut stdin,
        &protocol.thread_start_request(1, config.working_directory()),
    )?;
    receive_until(&receiver, deadline, &mut session, |session| {
        session.thread_id().is_some()
    })?;
    drain_available(&receiver, deadline, &mut session)?;

    let thread_id = session
        .thread_id()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::ProtocolFailed))?
        .to_owned();
    ensure_before_deadline(deadline)?;
    session
        .mark_request_sent(SessionRequest::TurnStart)
        .map_err(|error| map_session_error(&error))?;
    send_json(
        &mut stdin,
        &protocol.turn_start_request(2, &thread_id, config.working_directory(), config.prompt()),
    )?;
    receive_until(&receiver, deadline, &mut session, |session| {
        session.turn_id().is_some()
    })?;
    if let Err(error) = receive_until(&receiver, deadline, &mut session, |session| {
        session.outcome().is_some()
    }) {
        if error.kind() == AppServerRunErrorKind::Timeout {
            interrupt_timed_out_turn(&mut stdin, &session, &thread_id);
            return Err(error);
        }
        return Err(error);
    }
    drain_available(&receiver, deadline, &mut session)?;
    ensure_before_deadline(deadline)?;

    build_run_evidence(&session, initialize, thread_id)
}

#[cfg(windows)]
fn receive_windows_sandbox_readiness(
    receiver: &Receiver<ReaderEvent>,
    caller_deadline: Instant,
    session: &mut AppServerSession,
) -> Result<(), AppServerRunError> {
    let bootstrap_deadline = Instant::now()
        .checked_add(FS_SANDBOX_READINESS_TIMEOUT)
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::InvalidTimeout))?;
    let deadline = bootstrap_deadline.min(caller_deadline);
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(readiness_timeout(caller_deadline, bootstrap_deadline));
        };
        let event = match receiver.recv_timeout(remaining) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => {
                return Err(readiness_timeout(caller_deadline, bootstrap_deadline));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AppServerRunError::new(AppServerRunErrorKind::AmbiguousEof));
            }
        };
        match event {
            ReaderEvent::Line(line) => {
                let message: Value = serde_json::from_str(&line)
                    .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::ProtocolFailed))?;
                if message.get("id").and_then(Value::as_i64)
                    == Some(WINDOWS_SANDBOX_READINESS_REQUEST_ID)
                {
                    if message.pointer("/result/status").and_then(Value::as_str) == Some("ready")
                        && message.get("error").is_none()
                    {
                        return Ok(());
                    }
                    return Err(AppServerRunError::new(
                        AppServerRunErrorKind::FsSandboxBootstrapFailed,
                    ));
                }
                session
                    .ingest(message)
                    .map_err(|error| map_session_error(&error))?;
            }
            ReaderEvent::Eof => {
                return Err(AppServerRunError::new(AppServerRunErrorKind::AmbiguousEof));
            }
            ReaderEvent::Failed => {
                return Err(AppServerRunError::new(AppServerRunErrorKind::StdoutFailed));
            }
            ReaderEvent::LineTooLarge => {
                return Err(AppServerRunError::new(
                    AppServerRunErrorKind::StdoutLineTooLarge,
                ));
            }
        }
    }
}

#[cfg(windows)]
fn readiness_timeout(caller_deadline: Instant, bootstrap_deadline: Instant) -> AppServerRunError {
    let kind = if caller_deadline <= bootstrap_deadline {
        AppServerRunErrorKind::Timeout
    } else {
        AppServerRunErrorKind::FsSandboxHelperTimeout
    };
    AppServerRunError::new(kind)
}

fn start_readers<Stdout, Stderr>(stdout: Stdout, stderr: Stderr) -> Receiver<ReaderEvent>
where
    Stdout: Read + Send + 'static,
    Stderr: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            let mut bounded_line = reader
                .by_ref()
                .take(u64::try_from(MAX_STDOUT_LINE_BYTES + 1).unwrap_or(u64::MAX));
            match bounded_line.read_line(&mut line) {
                Ok(0) => {
                    let _ = sender.send(ReaderEvent::Eof);
                    break;
                }
                Ok(read) if read > MAX_STDOUT_LINE_BYTES => {
                    let _ = sender.send(ReaderEvent::LineTooLarge);
                    break;
                }
                Ok(_) => {
                    while line.ends_with(['\r', '\n']) {
                        line.pop();
                    }
                    if sender.send(ReaderEvent::Line(line)).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = sender.send(ReaderEvent::Failed);
                    break;
                }
            }
        }
    });
    std::thread::spawn(move || {
        // Keep draining the pipe for the child's full lifetime. Reading only a
        // prefix can close the pipe early and either block or break a healthy
        // app-server after enough diagnostic output.
        let mut stderr = stderr;
        let _ = std::io::copy(&mut stderr, &mut std::io::sink());
    });
    receiver
}

fn build_run_evidence(
    session: &AppServerSession,
    initialize: InitializeEvidence,
    thread_id: String,
) -> Result<AppServerRunEvidence, AppServerRunError> {
    let turn_id = session
        .turn_id()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::ProtocolFailed))?
        .to_owned();
    let outcome = session
        .outcome()
        .cloned()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::ProtocolFailed))?;
    Ok(AppServerRunEvidence {
        initialize,
        thread_id,
        turn_id,
        outcome,
    })
}

fn require_matching_codex_home(
    session: &AppServerSession,
    config: &AppServerRunConfig,
) -> Result<InitializeEvidence, AppServerRunError> {
    let initialize = session
        .initialize_evidence()
        .cloned()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::ProtocolFailed))?;
    if !same_existing_directory(&initialize.codex_home, config.codex_home()) {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::CodexHomeMismatch,
        ));
    }
    Ok(initialize)
}

fn receive_until(
    receiver: &Receiver<ReaderEvent>,
    deadline: Instant,
    session: &mut AppServerSession,
    complete: impl Fn(&AppServerSession) -> bool,
) -> Result<(), AppServerRunError> {
    while !complete(session) {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::Timeout))?;
        let event = match receiver.recv_timeout(remaining) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => {
                return Err(AppServerRunError::new(AppServerRunErrorKind::Timeout));
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AppServerRunError::new(AppServerRunErrorKind::AmbiguousEof));
            }
        };
        ingest_reader_event(event, session)?;
    }
    Ok(())
}

fn drain_available(
    receiver: &Receiver<ReaderEvent>,
    deadline: Instant,
    session: &mut AppServerSession,
) -> Result<(), AppServerRunError> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::Timeout))?;
        match receiver.recv_timeout(PROTOCOL_QUIET_WINDOW.min(remaining)) {
            Ok(event) => ingest_reader_event(event, session)?,
            Err(RecvTimeoutError::Timeout) => return Ok(()),
            Err(RecvTimeoutError::Disconnected) if session.outcome().is_some() => return Ok(()),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AppServerRunError::new(AppServerRunErrorKind::AmbiguousEof));
            }
        }
    }
}

fn ingest_reader_event(
    event: ReaderEvent,
    session: &mut AppServerSession,
) -> Result<(), AppServerRunError> {
    match event {
        ReaderEvent::Line(line) => {
            session
                .ingest_json_line(&line)
                .map_err(|error| map_session_error(&error))?;
            Ok(())
        }
        ReaderEvent::Eof => session
            .finish_eof()
            .map(|_| ())
            .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::AmbiguousEof)),
        ReaderEvent::Failed => Err(AppServerRunError::new(AppServerRunErrorKind::StdoutFailed)),
        ReaderEvent::LineTooLarge => Err(AppServerRunError::new(
            AppServerRunErrorKind::StdoutLineTooLarge,
        )),
    }
}

fn interrupt_timed_out_turn(stdin: &mut impl Write, session: &AppServerSession, thread_id: &str) {
    let Some(turn_id) = session.turn_id().map(ToOwned::to_owned) else {
        return;
    };
    let _ = send_turn_interrupt(stdin, thread_id, &turn_id);
}

fn send_turn_interrupt(
    stdin: &mut impl Write,
    thread_id: &str,
    turn_id: &str,
) -> Result<(), AppServerRunError> {
    let request = json!({
        "method": "turn/interrupt",
        "id": INTERRUPT_REQUEST_ID,
        "params": {"threadId": thread_id, "turnId": turn_id}
    });
    // Deadline expiry revokes the child's effect window. The interrupt is
    // advisory only: never wait for Codex to acknowledge or emit a terminal,
    // because it and its descendants would remain writable during that grace.
    // The caller immediately closes the owned process tree and waits for the
    // direct child to be reaped before returning.
    send_json(stdin, &request)
}

fn send_json(writer: &mut impl Write, message: &Value) -> Result<(), AppServerRunError> {
    serde_json::to_writer(&mut *writer, message)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::WriteFailed))?;
    writer
        .write_all(b"\n")
        .and_then(|()| writer.flush())
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::WriteFailed))
}

pub(crate) fn validate_live_paths(config: &AppServerRunConfig) -> Result<(), AppServerRunError> {
    let launcher = std::fs::symlink_metadata(config.launcher())
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidLauncher))?;
    if !launcher.file_type().is_file() {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidLauncher,
        ));
    }
    let codex_home = std::fs::symlink_metadata(config.codex_home())
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidCodexHome))?;
    if !codex_home.file_type().is_dir() {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidCodexHome,
        ));
    }
    let working_directory = std::fs::symlink_metadata(config.working_directory())
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidWorkingDirectory))?;
    if !working_directory.file_type().is_dir() {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidWorkingDirectory,
        ));
    }
    validate_owned_codex_home(config)?;
    validate_pinned_resources(config)?;
    Ok(())
}

pub(crate) fn configure_pinned_child_environment(
    command: &mut Command,
    resources: Option<&PinnedCodexResources>,
) -> Result<(), AppServerRunError> {
    let Some(resources) = resources else {
        return Ok(());
    };
    let resources_directory = resources.resources_directory();
    let ambient = env::var_os("PATH");
    let child_path = pinned_child_path(resources_directory, ambient.as_deref())?;
    command
        .env(
            "CODEX_MANAGED_PACKAGE_ROOT",
            resources.managed_package_root(),
        )
        .env("PATH", child_path);
    Ok(())
}

fn pinned_child_path(
    resources_directory: &Path,
    ambient: Option<&OsStr>,
) -> Result<OsString, AppServerRunError> {
    let paths = std::iter::once(resources_directory.to_path_buf()).chain(
        ambient
            .into_iter()
            .flat_map(env::split_paths)
            .filter(|path| {
                !same_existing_directory(path, resources_directory)
                    && !path.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .eq_ignore_ascii_case("codex-resources")
                    })
                    && !path.join("codex-windows-sandbox-setup.exe").is_file()
                    && !path.join("codex-command-runner.exe").is_file()
            }),
    );
    env::join_paths(paths)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::PinnedResourcePathInvalid))
}

fn validate_pinned_resources(config: &AppServerRunConfig) -> Result<(), AppServerRunError> {
    validate_pinned_resources_for_launcher(config.launcher(), config.pinned_resources())
}

pub(crate) fn validate_pinned_resources_for_launcher(
    launcher: &Path,
    resources: Option<&PinnedCodexResources>,
) -> Result<(), AppServerRunError> {
    let Some(resources) = resources else {
        return Ok(());
    };
    let managed_root = resources.managed_package_root();
    let root_metadata = std::fs::symlink_metadata(managed_root)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::PinnedResourcesMissing))?;
    if !root_metadata.file_type().is_dir() || metadata_is_reparse(&root_metadata) {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidPinnedResources,
        ));
    }
    let resources_directory = resources.resources_directory();
    let bundle_root = resources_directory
        .parent()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::InvalidPinnedResources))?;
    let launcher_parent = launcher
        .parent()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::InvalidPinnedResources))?;
    let launcher_parent_metadata = std::fs::symlink_metadata(launcher_parent)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::PinnedResourcesMissing))?;
    if !launcher_parent_metadata.file_type().is_dir()
        || metadata_is_reparse(&launcher_parent_metadata)
    {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidPinnedResources,
        ));
    }
    let launcher_bundle_root = launcher_parent
        .parent()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::InvalidPinnedResources))?;
    let platform_scope = bundle_root
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::InvalidPinnedResources))?;
    let managed_scope = managed_root
        .parent()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::InvalidPinnedResources))?;
    if managed_root.file_name() != Some(OsStr::new("codex"))
        || resources_directory.file_name() != Some(OsStr::new("codex-resources"))
        || !same_existing_directory(launcher_bundle_root, bundle_root)
        || !same_existing_directory(platform_scope, managed_scope)
    {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidPinnedResources,
        ));
    }
    let resources_metadata = std::fs::symlink_metadata(resources_directory)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::PinnedResourcesMissing))?;
    if !resources_metadata.file_type().is_dir() || metadata_is_reparse(&resources_metadata) {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidPinnedResources,
        ));
    }
    let codex_path_directory = bundle_root.join("codex-path");
    let codex_path_metadata = std::fs::symlink_metadata(&codex_path_directory)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::PinnedResourcesMissing))?;
    if !codex_path_metadata.file_type().is_dir() || metadata_is_reparse(&codex_path_metadata) {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidPinnedResources,
        ));
    }
    validate_pinned_resource_files([
        (
            resources_directory.join("codex-windows-sandbox-setup.exe"),
            resources.digests.sandbox_setup.as_str(),
        ),
        (
            resources_directory.join("codex-command-runner.exe"),
            resources.digests.command_runner.as_str(),
        ),
        (
            launcher_parent.join("codex-code-mode-host.exe"),
            resources.digests.code_mode_host.as_str(),
        ),
        (
            codex_path_directory.join("rg.exe"),
            resources.digests.rg.as_str(),
        ),
        (
            bundle_root.join("codex-package.json"),
            resources.digests.package_manifest.as_str(),
        ),
        (
            managed_root.join("package.json"),
            resources.digests.managed_package_manifest.as_str(),
        ),
    ])
}

fn validate_pinned_resource_files(
    resources: [(PathBuf, &str); 6],
) -> Result<(), AppServerRunError> {
    for (path, expected_sha256) in resources {
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::PinnedResourcesMissing))?;
        if !metadata.file_type().is_file() || metadata_is_reparse(&metadata) {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::InvalidPinnedResources,
            ));
        }
        if file_sha256(&path, AppServerRunErrorKind::PinnedResourcesMissing)? != expected_sha256 {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::PinnedResourcesDigestMismatch,
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_owned_codex_home(
    config: &AppServerRunConfig,
) -> Result<(), AppServerRunError> {
    let codex_home = std::fs::canonicalize(config.codex_home())
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidCodexHome))?;
    let working_directory = std::fs::canonicalize(config.working_directory())
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidWorkingDirectory))?;
    if codex_home.starts_with(&working_directory) || working_directory.starts_with(&codex_home) {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::CodexHomeOverlap,
        ));
    }

    let marker = config.codex_home().join(CODEX_HOME_OWNERSHIP_MARKER_NAME);
    validate_isolated_home_file(&marker, AppServerRunErrorKind::CodexHomeOwnershipMissing)?;
    if std::fs::read(&marker).ok().as_deref() != Some(CODEX_HOME_OWNERSHIP_MARKER_BYTES) {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::CodexHomeOwnershipMissing,
        ));
    }

    let auth_state = config.codex_home().join("auth.json");
    validate_isolated_home_file(&auth_state, AppServerRunErrorKind::InvalidCodexHome)?;
    let config_path = config.codex_home().join("config.toml");
    validate_isolated_home_file(&config_path, AppServerRunErrorKind::InvalidCodexHome)?;
    if !restore_owned_codex_home_config(
        &config_path,
        config.codex_home(),
        config.working_directory(),
    ) {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::InvalidCodexHome,
        ));
    }

    let mut ambient_homes = Vec::new();
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        ambient_homes.push(PathBuf::from(path));
    }
    for variable in ["USERPROFILE", "HOME"] {
        if let Some(path) = std::env::var_os(variable) {
            ambient_homes.push(PathBuf::from(path).join(".codex"));
        }
    }
    if ambient_homes
        .iter()
        .any(|ambient| same_existing_directory(ambient, &codex_home))
    {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::AmbientCodexHomeDenied,
        ));
    }

    Ok(())
}

fn restore_owned_codex_home_config(
    config_path: &Path,
    codex_home: &Path,
    working_directory: &Path,
) -> bool {
    let Ok(bytes) = std::fs::read(config_path) else {
        return false;
    };
    if bytes.as_slice() == CODEX_HOME_CONFIG_BYTES {
        return true;
    }

    let prefix = format!(
        "{}\n[projects.'",
        String::from_utf8_lossy(CODEX_HOME_CONFIG_BYTES)
    );
    let suffix = "']\ntrust_level = \"trusted\"\n";
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    let Some(trusted_worktree) = text
        .strip_prefix(&prefix)
        .and_then(|text| text.strip_suffix(suffix))
    else {
        return false;
    };
    let trusted_worktree = trusted_worktree.to_ascii_lowercase();
    let current_worktree = working_directory.to_string_lossy().to_ascii_lowercase();
    let prior_delivery_worktree = codex_home
        .parent()
        .map(|root| root.join("runtime-delivery"))
        .map(|root| root.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|root| is_lattice_delivery_worktree(&trusted_worktree, &root));
    if trusted_worktree != current_worktree && !prior_delivery_worktree {
        return false;
    }
    std::fs::write(config_path, CODEX_HOME_CONFIG_BYTES).is_ok()
}

fn is_lattice_delivery_worktree(path: &str, runtime_delivery_root: &str) -> bool {
    let Some(relative) = path.strip_prefix(&format!("{runtime_delivery_root}\\")) else {
        return false;
    };
    let Some(task) = relative.strip_suffix("\\repo") else {
        return false;
    };
    task.len() == 69
        && task.starts_with("task-")
        && task[5..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_isolated_home_file(
    path: &Path,
    error_kind: AppServerRunErrorKind,
) -> Result<(), AppServerRunError> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| AppServerRunError::new(error_kind))?;
    if !metadata.file_type().is_file() || metadata_is_reparse(&metadata) {
        return Err(AppServerRunError::new(error_kind));
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &std::fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn same_existing_directory(left: &Path, right: &Path) -> bool {
    let Ok(left) = std::fs::canonicalize(left) else {
        return false;
    };
    let Ok(right) = std::fs::canonicalize(right) else {
        return false;
    };
    left == right
}

fn ensure_before_deadline(deadline: Instant) -> Result<(), AppServerRunError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .map(|_| ())
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::Timeout))
}

fn launcher_sha256(path: &Path) -> Result<String, AppServerRunError> {
    file_sha256(path, AppServerRunErrorKind::LauncherReadFailed)
}

fn file_sha256(
    path: &Path,
    read_error: AppServerRunErrorKind,
) -> Result<String, AppServerRunError> {
    let mut file = File::open(path).map_err(|_| AppServerRunError::new(read_error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| AppServerRunError::new(read_error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let bytes = hasher.finalize();
    let mut digest = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut digest, "{byte:02x}").map_err(|_| AppServerRunError::new(read_error))?;
    }
    Ok(digest)
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn map_session_error(error: &SessionError) -> AppServerRunError {
    let kind = match error {
        SessionError::Terminal(ProtocolError::IncompleteToolExecution) => {
            AppServerRunErrorKind::IncompleteToolExecution
        }
        _ => AppServerRunErrorKind::ProtocolFailed,
    };
    AppServerRunError::new(kind)
}

#[cfg(all(test, windows))]
mod resource_environment_tests {
    use std::ffi::OsStr;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{
        PinnedCodexResourceDigests, PinnedCodexResources, configure_pinned_child_environment,
        pinned_child_path,
    };

    #[test]
    fn pinned_package_root_and_resources_replace_hostile_codex_resource_paths() {
        let managed_root = PathBuf::from(r"C:\pinned\node_modules\@openai\codex");
        let resources_directory = PathBuf::from(
            r"C:\pinned\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\codex-resources",
        );
        let resources = PinnedCodexResources::new(
            managed_root.clone(),
            resources_directory.clone(),
            PinnedCodexResourceDigests::new(
                "a".repeat(64),
                "b".repeat(64),
                "c".repeat(64),
                "d".repeat(64),
                "e".repeat(64),
                "f".repeat(64),
            )
            .expect("valid digest bundle"),
        )
        .expect("absolute pinned resource binding");
        let ambient = std::env::join_paths([
            Path::new(r"C:\global\codex-0.144.6\codex-resources"),
            Path::new(r"C:\WindowsApps\CodexDesktop\codex-resources"),
            Path::new(r"C:\Windows\System32"),
        ])
        .expect("hostile ambient path fixture");
        let child_path = pinned_child_path(resources.resources_directory(), Some(&ambient))
            .expect("pinned child path");
        let child_paths = std::env::split_paths(&child_path).collect::<Vec<_>>();

        assert_eq!(child_paths.first(), Some(&resources_directory));
        assert_eq!(
            child_paths,
            vec![resources_directory, PathBuf::from(r"C:\Windows\System32")]
        );

        let mut command = Command::new("unused");
        configure_pinned_child_environment(&mut command, Some(&resources))
            .expect("bind pinned child environment");
        let managed = command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new("CODEX_MANAGED_PACKAGE_ROOT"))
            .expect("managed package root is explicit");
        assert_eq!(managed.1, Some(managed_root.as_os_str()));
    }
}

#[cfg(test)]
mod deadline_tests {
    use std::io::{self, Write};

    use serde_json::json;

    use super::send_turn_interrupt;

    #[derive(Default)]
    struct FlushSpy {
        bytes: Vec<u8>,
        flush_count: usize,
    }

    impl Write for FlushSpy {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flush_count += 1;
            Ok(())
        }
    }

    #[test]
    fn timed_out_turn_interrupt_is_one_exact_flushed_jsonl_request() {
        let mut output = FlushSpy::default();

        send_turn_interrupt(&mut output, "thread-deadline", "turn-deadline")
            .expect("write interrupt request");

        assert_eq!(output.flush_count, 1);
        let payload = output
            .bytes
            .strip_suffix(b"\n")
            .expect("interrupt request ends with one JSONL delimiter");
        assert!(!payload.contains(&b'\n'));
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(payload).expect("parse interrupt request"),
            json!({
                "id": 4,
                "method": "turn/interrupt",
                "params": {
                    "threadId": "thread-deadline",
                    "turnId": "turn-deadline"
                }
            })
        );
    }
}

#[cfg(all(test, windows))]
mod windows_containment_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{
        AppServerRunError, AppServerRunErrorKind, OwnedChildStdio, spawn_owned_child,
        spawn_windows_owned_command_with_pre_resume, stop_owned_child,
    };

    static NEXT_MARKER: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn pre_resume_failure_never_resumes_the_suspended_child() {
        let sequence = NEXT_MARKER.fetch_add(1, Ordering::Relaxed);
        let marker = std::env::temp_dir().join(format!(
            "lattice-codex-suspended-pre-resume-{}-{sequence}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&marker);
        let system_root = std::env::var_os("SystemRoot").expect("Windows system root");
        let powershell = PathBuf::from(system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let script = format!(
            "[IO.File]::WriteAllText('{}', 'unexpected')",
            marker.display().to_string().replace('\'', "''")
        );
        let mut command = Command::new(powershell);
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(script)
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let error =
            spawn_windows_owned_command_with_pre_resume(&command, OwnedChildStdio::Null, || {
                assert!(
                    !marker.exists(),
                    "the suspended child executed before Job assignment completed"
                );
                Err(AppServerRunError::new(
                    AppServerRunErrorKind::JobObjectFailed,
                ))
            })
            .expect_err("injected pre-resume failure must fail closed");

        assert_eq!(error.kind(), AppServerRunErrorKind::JobObjectFailed);
        assert!(!marker.exists(), "a pre-resume failure resumed the child");
    }

    #[test]
    fn parent_exit_with_live_descendant_is_terminated_to_zero_before_return() {
        let sequence = NEXT_MARKER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lattice-codex-job-accounting-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create exact Job accounting root");
        let trigger = root.join("descendant.trigger");
        let effect = root.join("descendant.effect");
        let pid = root.join("descendant.pid");
        let quote = |path: &std::path::Path| path.display().to_string().replace('\'', "''");
        let descendant = format!(
            "`$stop = [DateTime]::UtcNow.AddSeconds(20); while (!(Test-Path -LiteralPath '{}')) {{ if ([DateTime]::UtcNow -ge `$stop) {{ exit 91 }}; Start-Sleep -Milliseconds 10 }}; [IO.File]::WriteAllText('{}', 'survived')",
            quote(&trigger),
            quote(&effect)
        );
        let script = format!(
            "$grandchild = '{}'; $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($grandchild)); $descendant = Start-Process -FilePath \"$PSHOME\\powershell.exe\" -WindowStyle Hidden -ArgumentList @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded) -PassThru; [IO.File]::WriteAllText('{}', [string]$descendant.Id); exit 0",
            descendant.replace('\'', "''"),
            quote(&pid)
        );
        let system_root = std::env::var_os("SystemRoot").expect("Windows system root");
        let powershell = PathBuf::from(system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let mut command = Command::new(powershell);
        command
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(script)
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = spawn_owned_child(&mut command, OwnedChildStdio::Null)
            .expect("spawn assigned suspended root");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().expect("observe owned root").is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "root did not exit on time");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(pid.exists(), "root exited before recording its descendant");
        assert!(
            child.active_processes().expect("query retained Job") > 0,
            "the exited root must leave its blocked descendant active"
        );

        stop_owned_child(&mut child).expect("terminate retained Job and prove zero members");
        assert_eq!(
            child.active_processes().expect("query reaped Job"),
            0,
            "cleanup returned before Job accounting reached zero"
        );
        fs::write(&trigger, b"release\n").expect("release any escaped descendant");
        thread::sleep(Duration::from_millis(500));
        assert!(
            !effect.exists(),
            "a descendant wrote after zero-member cleanup returned"
        );
        fs::remove_dir_all(&root).expect("remove exact Job accounting root");
    }
}
