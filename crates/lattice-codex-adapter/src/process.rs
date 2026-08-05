//! Owned-process transport for one bounded Codex app-server turn.

use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use win32job::{ExtendedLimitInfo, Job};

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
const INTERRUPT_GRACE: Duration = Duration::from_secs(5);
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
sandbox = \"unelevated\"\n";

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
    let Ok(mut child) = command.spawn() else {
        return Err(AppServerRunError::new(AppServerRunErrorKind::SpawnFailed));
    };
    let Ok(process_tree) = OwnedProcessTree::attach(&child) else {
        let _ = terminate_uncontained_process_tree_bounded(&mut child);
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::JobObjectFailed,
        ));
    };
    let result = ensure_before_deadline(deadline)
        .and_then(|()| verify_app_server_identity(config, before_spawn))
        .and_then(|()| ensure_before_deadline(deadline))
        .and_then(|()| drive_child(&mut child, config, deadline));
    let cleanup = stop_owned_child(&mut child, process_tree);
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
    let Ok(mut child) = command.spawn() else {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::FsSandboxBootstrapFailed,
        ));
    };
    let Ok(process_tree) = OwnedProcessTree::attach(&child) else {
        let _ = terminate_uncontained_process_tree_bounded(&mut child);
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::JobObjectFailed,
        ));
    };
    let post_attach = ensure_before_sandbox_deadline(caller_deadline, bootstrap_deadline)
        .and_then(|()| verify_preflight_identity(config, expected_launcher_digest))
        .and_then(|()| ensure_before_sandbox_deadline(caller_deadline, bootstrap_deadline));
    let result = post_attach
        .and_then(|()| wait_for_sandbox_preflight(&mut child, caller_deadline, bootstrap_deadline));
    let cleanup = stop_owned_child(&mut child, process_tree);
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
    child: &mut Child,
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

#[cfg(windows)]
pub(crate) struct OwnedProcessTree {
    _job: Job,
}

#[cfg(windows)]
impl OwnedProcessTree {
    pub(crate) fn attach(child: &Child) -> Result<Self, ()> {
        let mut limits = ExtendedLimitInfo::new();
        limits.limit_kill_on_job_close();
        let job = Job::create_with_limit_info(&limits).map_err(|_| ())?;
        job.assign_process(child.as_raw_handle() as isize)
            .map_err(|_| ())?;
        Ok(Self { _job: job })
    }
}

#[cfg(not(windows))]
pub(crate) struct OwnedProcessTree;

#[cfg(not(windows))]
impl OwnedProcessTree {
    pub(crate) fn attach(_: &Child) -> Result<Self, ()> {
        Ok(Self)
    }
}

fn drive_child(
    child: &mut Child,
    config: &AppServerRunConfig,
    deadline: Instant,
) -> Result<AppServerRunEvidence, AppServerRunError> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::PipeUnavailable))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::PipeUnavailable))?;
    let stderr = child
        .stderr
        .take()
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
            interrupt_timed_out_turn(&mut stdin, &receiver, &mut session, &thread_id)?;
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

fn start_readers(
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
) -> Receiver<ReaderEvent> {
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

fn interrupt_timed_out_turn(
    stdin: &mut impl Write,
    receiver: &Receiver<ReaderEvent>,
    session: &mut AppServerSession,
    thread_id: &str,
) -> Result<(), AppServerRunError> {
    let Some(turn_id) = session.turn_id().map(ToOwned::to_owned) else {
        return Ok(());
    };
    let request = json!({
        "method": "turn/interrupt",
        "id": INTERRUPT_REQUEST_ID,
        "params": {"threadId": thread_id, "turnId": turn_id}
    });
    if send_json(stdin, &request).is_err() {
        return Ok(());
    }

    let grace_deadline = Instant::now()
        .checked_add(INTERRUPT_GRACE)
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::Timeout))?;
    while session.outcome().is_none() {
        let Some(remaining) = grace_deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        let event = match receiver.recv_timeout(remaining) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        };
        match event {
            ReaderEvent::Line(line) => {
                match is_interrupt_ack(&line) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(_) => break,
                }
                if session.ingest_json_line(&line).is_err() {
                    break;
                }
            }
            ReaderEvent::Eof | ReaderEvent::Failed | ReaderEvent::LineTooLarge => break,
        }
    }
    Ok(())
}

fn is_interrupt_ack(line: &str) -> Result<bool, AppServerRunError> {
    let message: Value = serde_json::from_str(line)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::ProtocolFailed))?;
    if message.get("id").and_then(Value::as_i64) != Some(INTERRUPT_REQUEST_ID) {
        return Ok(false);
    }
    let object = message
        .as_object()
        .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::ProtocolFailed))?;
    if object.contains_key("method")
        || object.contains_key("error")
        || !object.get("result").is_some_and(Value::is_object)
    {
        return Err(AppServerRunError::new(
            AppServerRunErrorKind::ProtocolFailed,
        ));
    }
    Ok(true)
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
    let config_metadata = std::fs::metadata(&config_path)
        .map_err(|_| AppServerRunError::new(AppServerRunErrorKind::InvalidCodexHome))?;
    if config_metadata.len() != u64::try_from(CODEX_HOME_CONFIG_BYTES.len()).unwrap_or(u64::MAX)
        || std::fs::read(&config_path).ok().as_deref() != Some(CODEX_HOME_CONFIG_BYTES)
    {
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

pub(crate) fn stop_owned_child(
    child: &mut Child,
    process_tree: OwnedProcessTree,
) -> Result<(), AppServerRunError> {
    // Closing a Windows Job Object configured with KILL_ON_JOB_CLOSE is the
    // primary, tree-wide termination mechanism. Do this before waiting on any
    // external cleanup command so teardown cannot deadlock behind the child.
    drop(process_tree);
    terminate_child_bounded(child)
}

pub(crate) fn terminate_child_bounded(child: &mut Child) -> Result<(), AppServerRunError> {
    match child.try_wait() {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(_) => {
            return Err(AppServerRunError::new(
                AppServerRunErrorKind::ChildCleanupFailed,
            ));
        }
    }

    // The Job Object close above normally wins this race on Windows. The
    // direct kill is a bounded fallback and is also the non-Windows path.
    let _ = child.kill();
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
pub(crate) fn terminate_uncontained_process_tree_bounded(
    child: &mut Child,
) -> Result<(), AppServerRunError> {
    let pid = child.id().to_string();
    let mut tree_stopped = false;
    if let Some(taskkill) = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .map(|root| root.join("System32").join("taskkill.exe"))
        .filter(|path| path.is_file())
    {
        let mut command = Command::new(taskkill);
        crate::scrub_protected_environment(&mut command);
        command
            .args(["/PID", pid.as_str(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Ok(mut taskkill_child) = command.spawn() {
            let deadline = Instant::now()
                .checked_add(CHILD_CLEANUP_TIMEOUT)
                .ok_or_else(|| AppServerRunError::new(AppServerRunErrorKind::ChildCleanupFailed))?;
            loop {
                match taskkill_child.try_wait() {
                    Ok(Some(status)) => {
                        tree_stopped = status.success();
                        break;
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    let _ = terminate_child_bounded(&mut taskkill_child);
                    break;
                };
                std::thread::sleep(Duration::from_millis(10).min(remaining));
            }
        }
    }
    let direct_cleanup = terminate_child_bounded(child);
    if tree_stopped {
        direct_cleanup
    } else {
        Err(AppServerRunError::new(
            AppServerRunErrorKind::ChildCleanupFailed,
        ))
    }
}

#[cfg(not(windows))]
pub(crate) fn terminate_uncontained_process_tree_bounded(
    child: &mut Child,
) -> Result<(), AppServerRunError> {
    terminate_child_bounded(child)
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
