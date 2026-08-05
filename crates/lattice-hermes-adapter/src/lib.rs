//! Pinned Hermes reflection adapter over the official loopback Runs API.
//!
//! This crate owns no durable state. It accepts only one pre-bound, read-only
//! reflection job and converts a schema-valid Hermes response into an
//! untrusted LATTICE candidate digest.

mod broker;
mod containment;
mod runtime;
#[cfg(windows)]
mod windows_job;

pub use broker::CodexProxyInvocation;
#[cfg(windows)]
pub use broker::{CodexBrokerReceipt, CodexReflectionBrokerConfig};
pub use containment::{
    HermesContainmentFrame, HermesContainmentFrameLimits, HermesSandboxProfile,
    build_hermes_bwrap_arguments, parse_containment_frame,
};
#[cfg(windows)]
pub use containment::{HermesSocketpairReceipt, HermesWslContainmentConfig};
pub use runtime::{
    HERMES_CPYTHON_ARCHIVE_BYTES, HERMES_CPYTHON_ARCHIVE_SHA256, HERMES_CPYTHON_BUILD_RELEASE,
    HERMES_CPYTHON_PROVENANCE, HERMES_CPYTHON_SHA256SUMS_SHA256, HERMES_CPYTHON_VERSION,
    HERMES_PYPROJECT_SHA256, HERMES_RUNTIME_ARCHIVE_SHA256, HERMES_UV_LOCK_SHA256,
    HermesOfflineRuntimeManifest,
};

/// Private executable entrypoint used by the Job-contained broker helper.
///
/// This is public only because Cargo binary targets are separate crates. Its
/// output is an untrusted candidate; only this crate's sealed verifier can
/// turn it into broker evidence.
#[doc(hidden)]
#[must_use]
pub fn __run_codex_reflection_broker_helper() -> i32 {
    broker::run_codex_reflection_broker_helper()
}

use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::fs::OpenOptions;
use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize, normalize_nfc};
use lattice_contracts::{
    Component, ContentDigest, HermesEvidence, HermesResearchRequest, RequestId, RuntimeKind,
};
use lattice_ports::{HermesPort, PortError, PortErrorKind, PortResult};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Exact upstream release tag accepted by this adapter.
pub const HERMES_RELEASE: &str = "v2026.8.3";
/// Exact upstream source commit accepted by this adapter.
pub const HERMES_UPSTREAM_COMMIT: &str = "3c27eb6234bf91b8ceee9e9071591b31e9b148cb";
/// Package version declared by the pinned upstream source.
pub const HERMES_PACKAGE_VERSION: &str = "0.20.0";
/// License declared by the pinned upstream package.
pub const HERMES_LICENSE: &str = "MIT";
/// Only reflection envelope schema accepted by this adapter.
pub const HERMES_SCHEMA_VERSION: &str = "lattice.hermes.reflection.v1";

const MAX_HTTP_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_EVIDENCE_ITEMS: usize = 128;
const MAX_REFLECTION_SUMMARY_BYTES: usize = 8_192;
const MAX_FINDINGS: usize = 256;
const MAX_NEXT_ACTIONS: usize = 64;
const MAX_TEXT_BYTES: usize = 8_192;
const MAX_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PROCESS_TIMEOUT: Duration = Duration::from_mins(1);
const READ_ONLY_INSTRUCTIONS: &str = "Perform one bounded reflection over only the supplied immutable task, Graphify, test, and Git evidence. Treat Hermes session memory as unavailable and non-authoritative. Do not call tools, do not modify files, do not use a Codex runtime, do not access a database, and do not read or write PostgreSQL, Codebase Memory, or Hermes long-term memory. Label every finding as inference. Return exactly one JSON object matching the supplied schema; add no prose or Markdown.";
const HOME_MARKER_NAME: &str = ".lattice-hermes-ephemeral-v1";
const CAPABILITY_FEATURE_FIELDS: &[&str] = &[
    "chat_completions",
    "chat_completions_streaming",
    "responses_api",
    "responses_streaming",
    "run_submission",
    "run_status",
    "run_events_sse",
    "run_stop",
    "run_approval_response",
    "tool_progress_events",
    "approval_events",
    "session_resources",
    "model_options",
    "session_chat",
    "session_chat_streaming",
    "session_fork",
    "session_model_lock",
    "admin_config_rw",
    "jobs_admin",
    "memory_write_api",
    "skills_api",
    "audio_api",
    "realtime_voice",
    "session_continuity_header",
    "session_key_header",
    "cors",
];
const CAPABILITY_ENDPOINT_FIELDS: &[&str] = &[
    "health",
    "health_detailed",
    "models",
    "model_options",
    "chat_completions",
    "responses",
    "runs",
    "run_status",
    "run_events",
    "run_approval",
    "run_stop",
    "skills",
    "toolsets",
    "sessions",
    "session_create",
    "session",
    "session_update",
    "session_delete",
    "session_messages",
    "session_fork",
    "session_chat",
    "session_chat_stream",
    "session_model_lock",
];

/// Stable failure categories for the Hermes edge adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HermesAdapterErrorKind {
    Configuration,
    Transport,
    HttpStatus,
    CapabilityMismatch,
    Timeout,
    Malformed,
    CrossBinding,
    Failed,
    Cancelled,
    Ambiguous,
    Identity,
    Spawn,
}

/// Fail-closed adapter error containing only a stable non-secret code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesAdapterError {
    kind: HermesAdapterErrorKind,
    code: &'static str,
    recovery_receipt: Option<Box<HermesRunRecoveryReceipt>>,
}

impl HermesAdapterError {
    #[must_use]
    pub const fn new(kind: HermesAdapterErrorKind, code: &'static str) -> Self {
        Self {
            kind,
            code,
            recovery_receipt: None,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> HermesAdapterErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }

    #[must_use]
    pub fn recovery_receipt(&self) -> Option<&HermesRunRecoveryReceipt> {
        self.recovery_receipt.as_deref()
    }

    fn with_recovery_receipt(mut self, receipt: HermesRunRecoveryReceipt) -> Self {
        self.recovery_receipt = Some(Box::new(receipt));
        self
    }
}

impl fmt::Display for HermesAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.code)
    }
}

impl Error for HermesAdapterError {}

pub type HermesAdapterResult<T> = Result<T, HermesAdapterError>;

/// Opaque, secret-free binding needed to reconcile one ambiguous submission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesRunRecoveryReceipt {
    run_id: Option<String>,
    request_id: String,
    session_id: String,
    input_digest: ContentDigest,
    model: String,
}

impl HermesRunRecoveryReceipt {
    #[must_use]
    pub fn run_id(&self) -> Option<&str> {
        self.run_id.as_deref()
    }

    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub const fn input_digest(&self) -> &ContentDigest {
        &self.input_digest
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
}

/// Enforced memory policy for production Hermes child processes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum HermesMemoryPolicy {
    /// A never-reused, caller-selected `HERMES_HOME` with no inherited home,
    /// provider, database, Git credential, or normal `CODEX_HOME` state.
    EphemeralIsolatedHome,
}

/// Exact child-process boundary for the pinned Hermes gateway.
#[derive(Clone)]
pub struct HermesProcessConfig {
    executable: PathBuf,
    executable_sha256: String,
    isolation_root: PathBuf,
    product_root: PathBuf,
    working_directory: PathBuf,
    hermes_home: PathBuf,
    codex_home: PathBuf,
    temp_directory: PathBuf,
    endpoint: SocketAddr,
    api_key: String,
    model: String,
    startup_timeout: Duration,
}

impl HermesProcessConfig {
    /// Constructs a process-local, loopback-only Hermes gateway configuration.
    ///
    /// # Errors
    ///
    /// Rejects a missing/non-absolute executable, non-loopback listener,
    /// existing or daily home paths, shared Hermes/Codex homes, unsafe auth or
    /// model values, and a zero startup deadline.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        executable: impl Into<PathBuf>,
        executable_sha256: impl Into<String>,
        isolation_root: impl Into<PathBuf>,
        product_root: impl Into<PathBuf>,
        endpoint: SocketAddr,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
    ) -> HermesAdapterResult<Self> {
        let executable = executable.into();
        let executable_sha256 = executable_sha256.into();
        let isolation_root = isolation_root.into();
        let product_root = product_root.into();
        let api_key = api_key.into();
        let model = model.into();
        if !executable.is_absolute() || !executable.is_file() {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_EXECUTABLE_REJECTED",
            ));
        }
        let executable = fs::canonicalize(&executable).map_err(|_| {
            error(
                HermesAdapterErrorKind::Identity,
                "HERMES_EXECUTABLE_CANONICALIZATION_FAILED",
            )
        })?;
        validate_sha256(&executable_sha256, "HERMES_EXECUTABLE_SHA256_REJECTED")?;
        if sha256_file(&executable)? != executable_sha256 {
            return Err(error(
                HermesAdapterErrorKind::Identity,
                "HERMES_EXECUTABLE_HASH_MISMATCH",
            ));
        }
        let (isolation_root, product_root) =
            validate_isolation_boundary(&isolation_root, &product_root)?;
        let working_directory = isolation_root.join("cwd");
        let hermes_home = isolation_root.join("hermes-home");
        let codex_home = isolation_root.join("codex-home");
        let temp_directory = isolation_root.join("tmp");
        if !endpoint.ip().is_loopback() {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PROCESS_ENDPOINT_NOT_LOOPBACK",
            ));
        }
        if is_daily_home(&hermes_home, "HERMES_HOME")
            || is_daily_home(&codex_home, "CODEX_HOME")
            || default_hermes_homes()
                .iter()
                .any(|daily| same_path(&hermes_home, daily))
        {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PROCESS_HOME_ISOLATION_REJECTED",
            ));
        }
        if api_key.trim().is_empty()
            || api_key.len() > 4_096
            || api_key.chars().any(char::is_control)
        {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PROCESS_API_KEY_REJECTED",
            ));
        }
        validate_identifier(&model, 256, "HERMES_PROCESS_MODEL_REJECTED")?;
        if startup_timeout.is_zero() || startup_timeout > MAX_PROCESS_TIMEOUT {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PROCESS_STARTUP_TIMEOUT_REJECTED",
            ));
        }
        Ok(Self {
            executable,
            executable_sha256,
            isolation_root,
            product_root,
            working_directory,
            hermes_home,
            codex_home,
            temp_directory,
            endpoint,
            api_key,
            model,
            startup_timeout,
        })
    }

    #[must_use]
    pub const fn memory_policy(&self) -> HermesMemoryPolicy {
        HermesMemoryPolicy::EphemeralIsolatedHome
    }

    #[must_use]
    pub fn isolation_root(&self) -> &Path {
        &self.isolation_root
    }

    #[must_use]
    pub fn product_root(&self) -> &Path {
        &self.product_root
    }

    #[must_use]
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    #[must_use]
    pub fn hermes_home(&self) -> &Path {
        &self.hermes_home
    }

    #[must_use]
    pub fn codex_home(&self) -> &Path {
        &self.codex_home
    }

    #[must_use]
    pub fn temp_directory(&self) -> &Path {
        &self.temp_directory
    }

    /// Returns the trusted SHA-256 bound to the canonical executable path.
    #[must_use]
    pub fn executable_sha256(&self) -> &str {
        &self.executable_sha256
    }

    /// Builds an authenticated adapter config bound to the same child endpoint.
    ///
    /// # Errors
    ///
    /// Rejects invalid run/poll deadline values.
    pub fn adapter_config(
        &self,
        timeout: Duration,
        poll_interval: Duration,
    ) -> HermesAdapterResult<HermesAdapterConfig> {
        HermesAdapterConfig::new(self.endpoint, self.api_key.clone(), timeout, poll_interval)
    }

    /// Builds the exact isolated `hermes gateway` command.
    ///
    /// The returned command inherits no user home, provider credential,
    /// database credential, Git credential, or ordinary `CODEX_HOME` value.
    ///
    /// # Errors
    ///
    /// Returns a configuration error if a required environment value cannot be
    /// represented safely.
    pub fn gateway_command(&self) -> HermesAdapterResult<Command> {
        let mut command = self.base_command_with_arg("gateway");
        command
            .env("API_SERVER_ENABLED", "true")
            .env("API_SERVER_HOST", self.endpoint.ip().to_string())
            .env("API_SERVER_PORT", self.endpoint.port().to_string())
            .env("API_SERVER_KEY", &self.api_key)
            .env("API_SERVER_MODEL_NAME", &self.model);
        Ok(command)
    }

    /// Builds the secret-free executable identity probe.
    ///
    /// The bearer key and all API-server settings are intentionally absent.
    /// [`Self::verify_pinned_version`] executes this command under the bounded
    /// process timeout.
    ///
    /// # Errors
    ///
    /// This fixed command shape currently has no fallible inputs.
    pub fn version_probe_command(&self) -> HermesAdapterResult<Command> {
        Ok(self.base_command_with_arg("--version"))
    }

    /// Verifies the executable reports the exact package version declared by
    /// upstream commit [`HERMES_UPSTREAM_COMMIT`].
    ///
    /// # Errors
    ///
    /// Fails closed when the probe cannot run, exits unsuccessfully, or does
    /// not contain the exact standalone package-version token.
    pub fn verify_pinned_version(&self) -> HermesAdapterResult<()> {
        self.verify_executable_identity()?;
        let mut command = self.version_probe_command()?;
        let output = bounded_output(&mut command, self.startup_timeout)?;
        if !output.status.success() {
            return Err(error(
                HermesAdapterErrorKind::Identity,
                "HERMES_VERSION_PROBE_NONZERO",
            ));
        }
        validate_pinned_version_output(&output.stdout, &output.stderr)
    }

    /// Verifies identity, creates fresh marked homes, and starts Hermes.
    ///
    /// # Errors
    ///
    /// Fails closed on identity, home ownership, or process-spawn ambiguity.
    pub fn spawn(&self) -> HermesAdapterResult<HermesProcess> {
        prepare_isolated_run(self)?;
        self.verify_pinned_version()?;
        validate_prepared_isolation(self)?;
        let mut command = self.gateway_command()?;
        command.stdout(Stdio::null()).stderr(Stdio::null());
        let child = command
            .spawn()
            .map_err(|_| error(HermesAdapterErrorKind::Spawn, "HERMES_GATEWAY_SPAWN_FAILED"))?;
        Ok(HermesProcess {
            child,
            endpoint: self.endpoint,
            startup_timeout: self.startup_timeout,
        })
    }

    fn verify_executable_identity(&self) -> HermesAdapterResult<()> {
        let canonical = fs::canonicalize(&self.executable).map_err(|_| {
            error(
                HermesAdapterErrorKind::Identity,
                "HERMES_EXECUTABLE_CANONICALIZATION_FAILED",
            )
        })?;
        if canonical != self.executable || sha256_file(&canonical)? != self.executable_sha256 {
            return Err(error(
                HermesAdapterErrorKind::Identity,
                "HERMES_EXECUTABLE_IDENTITY_CHANGED",
            ));
        }
        Ok(())
    }

    fn base_command_with_arg(&self, argument: &str) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .arg(argument)
            .current_dir(&self.working_directory)
            .env_clear();
        for name in ["SystemRoot", "WINDIR", "ComSpec", "LANG", "LC_ALL"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("HERMES_HOME", &self.hermes_home)
            .env("CODEX_HOME", &self.codex_home)
            .env("HOME", &self.hermes_home)
            .env("USERPROFILE", &self.hermes_home)
            .env("TEMP", &self.temp_directory)
            .env("TMP", &self.temp_directory)
            .env("TMPDIR", &self.temp_directory)
            .env("NO_COLOR", "1");
        command
    }
}

/// Owned pinned Hermes child process. Dropping it reaps only that child.
pub struct HermesProcess {
    child: Child,
    endpoint: SocketAddr,
    startup_timeout: Duration,
}

impl HermesProcess {
    #[must_use]
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    #[must_use]
    pub const fn startup_timeout(&self) -> Duration {
        self.startup_timeout
    }

    #[must_use]
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Terminates and reaps the owned process.
    ///
    /// # Errors
    ///
    /// Reports an ambiguous teardown if kill or wait cannot confirm exit.
    pub fn terminate(mut self) -> HermesAdapterResult<()> {
        terminate_child(&mut self.child)
    }
}

impl Drop for HermesProcess {
    fn drop(&mut self) {
        let _ = terminate_child(&mut self.child);
    }
}

/// Fixed loopback connection and deadline settings.
#[derive(Clone)]
pub struct HermesAdapterConfig {
    endpoint: SocketAddr,
    api_key: String,
    timeout: Duration,
    poll_interval: Duration,
    containment_receipt: Option<HermesContainmentReceipt>,
}

/// Sealed evidence emitted only by a real OS sandbox verifier.
///
/// There is intentionally no public constructor. Empty directories,
/// `current_dir`, environment clearing, prompt instructions, and process
/// lifecycle ownership are not sufficient to mint this receipt. A future
/// same-process runner must additionally bind the contained Hermes PID,
/// endpoint, and nonce before this receipt can become constructible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesContainmentReceipt {
    endpoint: SocketAddr,
    api_key_sha256: String,
    receipt_digest: ContentDigest,
}

impl HermesContainmentReceipt {
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    fn verify_binding(&self, endpoint: SocketAddr, api_key: &str) -> HermesAdapterResult<()> {
        if self.endpoint != endpoint || self.api_key_sha256 != sha256_text(api_key) {
            return Err(cross_binding(
                "HERMES_CONTAINMENT_ENDPOINT_BINDING_REJECTED",
            ));
        }
        Ok(())
    }
}

/// Joint identity proof for the runtime, socketpair sandbox, and broker.
///
/// This is infrastructure evidence only. It deliberately cannot be installed
/// into [`HermesAdapterConfig`] and cannot authorize [`RuntimeKind::Live`]
/// because it does not bind a running Hermes PID, endpoint, and nonce.
#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesContainmentPrerequisites {
    receipt_digest: ContentDigest,
}

#[cfg(windows)]
impl HermesContainmentPrerequisites {
    /// Verifies the three independent prerequisite receipts without claiming
    /// that any Hermes endpoint was launched inside that containment.
    ///
    /// # Errors
    ///
    /// Rejects runtime, socketpair, broker, identity, reap, descriptor, or
    /// digest drift.
    pub fn verify(
        runtime: &HermesOfflineRuntimeManifest,
        socketpair: &HermesSocketpairReceipt,
        broker: &CodexBrokerReceipt,
    ) -> HermesAdapterResult<Self> {
        socketpair.validate_for_containment()?;
        broker.validate_for_containment()?;
        if runtime.payload_file_count() == 0 || runtime.payload_byte_count() == 0 {
            return Err(error(
                HermesAdapterErrorKind::Identity,
                "HERMES_CONTAINMENT_RUNTIME_REJECTED",
            ));
        }
        let mut digest = Sha256::new();
        digest.update(b"lattice.hermes.containment-prerequisites.v1\0");
        for field in [
            HERMES_RELEASE.to_owned(),
            HERMES_UPSTREAM_COMMIT.to_owned(),
            HERMES_CPYTHON_VERSION.to_owned(),
            runtime.payload_file_count().to_string(),
            runtime.payload_byte_count().to_string(),
            runtime.payload_manifest_sha256().to_owned(),
            socketpair.receipt_digest().as_str().to_owned(),
            broker.receipt_digest().as_str().to_owned(),
        ] {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field.as_bytes());
        }
        let receipt_digest = ContentDigest::from_sha256(encode_sha256(&digest.finalize()))
            .map_err(|_| malformed("HERMES_CONTAINMENT_PREREQUISITES_REJECTED"))?;
        Ok(Self { receipt_digest })
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

impl HermesAdapterConfig {
    /// Creates a loopback-only authenticated client configuration.
    ///
    /// # Errors
    ///
    /// Rejects non-loopback endpoints, empty/control-bearing keys, and invalid
    /// deadline values.
    pub fn new(
        endpoint: SocketAddr,
        api_key: impl Into<String>,
        timeout: Duration,
        poll_interval: Duration,
    ) -> HermesAdapterResult<Self> {
        let api_key = api_key.into();
        if !endpoint.ip().is_loopback() {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_ENDPOINT_NOT_LOOPBACK",
            ));
        }
        if api_key.trim().is_empty()
            || api_key.len() > 4_096
            || api_key.chars().any(char::is_control)
        {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_API_KEY_REJECTED",
            ));
        }
        if timeout.is_zero() || poll_interval.is_zero() || poll_interval > timeout {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_DEADLINE_REJECTED",
            ));
        }
        Ok(Self {
            endpoint,
            api_key,
            timeout,
            poll_interval,
            containment_receipt: None,
        })
    }

    #[must_use]
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    #[must_use]
    pub const fn poll_interval(&self) -> Duration {
        self.poll_interval
    }
}

/// Whitelisted source category for one immutable reflection input.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReflectionEvidenceKind {
    Task,
    Graphify,
    Test,
    Git,
}

impl ReflectionEvidenceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Graphify => "graphify",
            Self::Test => "test",
            Self::Git => "git",
        }
    }
}

/// One bounded, digest-addressed evidence item exposed to Hermes.
#[derive(Clone, Eq, PartialEq)]
pub struct ReflectionEvidence {
    kind: ReflectionEvidenceKind,
    digest: ContentDigest,
    sensitive_value_digests: Vec<ContentDigest>,
}

impl fmt::Debug for ReflectionEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReflectionEvidence")
            .field("kind", &self.kind)
            .field("digest", &self.digest)
            .field(
                "sensitive_value_digest_count",
                &self.sensitive_value_digests.len(),
            )
            .finish()
    }
}

impl ReflectionEvidence {
    /// Constructs typed, digest-only evidence with no raw text surface.
    ///
    /// # Errors
    ///
    /// This constructor currently cannot fail, but retains the result contract
    /// for compatibility with digest-validation extensions.
    pub fn new(kind: ReflectionEvidenceKind, digest: ContentDigest) -> HermesAdapterResult<Self> {
        Self::new_digest_only(kind, digest, Vec::new())
    }

    /// Constructs typed evidence plus digest-only sensitive values.
    ///
    /// # Errors
    ///
    /// Rejects duplicate, zero, or excessive sensitive-value digests. No raw
    /// evidence text crosses this public boundary.
    pub fn new_digest_only(
        kind: ReflectionEvidenceKind,
        digest: ContentDigest,
        mut sensitive_value_digests: Vec<ContentDigest>,
    ) -> HermesAdapterResult<Self> {
        if sensitive_value_digests.len() > MAX_EVIDENCE_ITEMS
            || sensitive_value_digests
                .iter()
                .any(|digest| digest.as_str().bytes().all(|byte| byte == b'0'))
        {
            return Err(malformed("HERMES_SENSITIVE_DIGEST_REJECTED"));
        }
        sensitive_value_digests.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        if sensitive_value_digests
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(malformed("HERMES_SENSITIVE_DIGEST_REJECTED"));
        }
        Ok(Self {
            kind,
            digest,
            sensitive_value_digests,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> ReflectionEvidenceKind {
        self.kind
    }

    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }

    #[must_use]
    pub fn sensitive_value_digests(&self) -> &[ContentDigest] {
        &self.sensitive_value_digests
    }
}

/// One immutable Hermes request plus fixed session, input, and model routing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesReflectionJob {
    request: HermesResearchRequest,
    session_id: String,
    model: String,
    evidence: Vec<ReflectionEvidence>,
    input_digest: ContentDigest,
    prompt: String,
}

impl HermesReflectionJob {
    /// Creates a single-request job and freezes its canonical input digest.
    ///
    /// # Errors
    ///
    /// Rejects unsafe identifiers, empty/duplicate evidence, and canonical
    /// encoding failures.
    pub fn new(
        request: HermesResearchRequest,
        session_id: impl Into<String>,
        model: impl Into<String>,
        evidence: Vec<ReflectionEvidence>,
    ) -> HermesAdapterResult<Self> {
        let session_id = session_id.into();
        let model = model.into();
        validate_identifier(&session_id, 256, "HERMES_SESSION_ID_REJECTED")?;
        validate_identifier(&model, 256, "HERMES_MODEL_REJECTED")?;
        if evidence.is_empty() || evidence.len() > MAX_EVIDENCE_ITEMS {
            return Err(malformed("HERMES_EVIDENCE_COUNT_REJECTED"));
        }
        let mut seen = HashSet::new();
        if evidence
            .iter()
            .any(|item| !seen.insert(item.digest().as_str()))
        {
            return Err(malformed("HERMES_DUPLICATE_EVIDENCE_REJECTED"));
        }
        let canonical_input = input_value(&request, &session_id, &model, &evidence);
        let domain = HashDomain::new("lattice.hermes.reflection-input", "1")
            .map_err(|_| malformed("HERMES_INPUT_DOMAIN_REJECTED"))?;
        let input_digest = ContentDigest::from_sha256(
            canonical_sha256(&domain, &canonical_input)
                .map_err(|_| malformed("HERMES_INPUT_CANONICALIZATION_REJECTED"))?
                .to_hex(),
        )
        .map_err(|_| malformed("HERMES_INPUT_DIGEST_REJECTED"))?;
        let canonical_input_bytes = canonicalize(&canonical_input)
            .map_err(|_| malformed("HERMES_INPUT_CANONICALIZATION_REJECTED"))?;
        let canonical_input_text = String::from_utf8(canonical_input_bytes.into_vec())
            .map_err(|_| malformed("HERMES_INPUT_UTF8_REJECTED"))?;
        let prompt = format!(
            "{READ_ONLY_INSTRUCTIONS}\n\nThe immutable input is canonical JSON:\n{canonical_input_text}\n\nReturn this exact object shape with no additional keys:\n{{\"schema_version\":\"{HERMES_SCHEMA_VERSION}\",\"binding\":{{\"request_id\":\"...\",\"task_id\":\"...\",\"attempt_id\":\"...\",\"project_snapshot_id\":\"...\",\"subject_digest\":\"...\",\"session_id\":\"...\",\"input_digest\":\"{}\",\"model\":\"...\"}},\"summary\":\"...\",\"findings\":[{{\"classification\":\"inference\",\"statement\":\"...\",\"evidence_digests\":[\"...\"]}}],\"next_actions\":[\"...\"]}}",
            input_digest.as_str(),
        );
        Ok(Self {
            request,
            session_id,
            model,
            evidence,
            input_digest,
            prompt,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &HermesResearchRequest {
        &self.request
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub const fn input_digest(&self) -> &ContentDigest {
        &self.input_digest
    }

    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }
}

/// Exact immutable request/session/input/model binding echoed by Hermes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectionBinding {
    request_id: String,
    task_id: String,
    attempt_id: String,
    project_snapshot_id: String,
    subject_digest: String,
    session_id: String,
    input_digest: String,
    model: String,
}

impl ReflectionBinding {
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    #[must_use]
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    #[must_use]
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }

    #[must_use]
    pub fn project_snapshot_id(&self) -> &str {
        &self.project_snapshot_id
    }

    #[must_use]
    pub fn subject_digest(&self) -> &str {
        &self.subject_digest
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn input_digest(&self) -> &str {
        &self.input_digest
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }
}

/// Allowed epistemic label for an untrusted Hermes finding.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReflectionClassification {
    Inference,
}

impl ReflectionClassification {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inference => "inference",
        }
    }
}

/// One typed, evidence-bound finding in a reflection candidate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectionFinding {
    classification: ReflectionClassification,
    statement: String,
    evidence_digests: Vec<String>,
}

impl ReflectionFinding {
    #[must_use]
    pub const fn classification(&self) -> ReflectionClassification {
        self.classification
    }

    #[must_use]
    pub fn statement(&self) -> &str {
        &self.statement
    }

    #[must_use]
    pub fn evidence_digests(&self) -> &[String] {
        &self.evidence_digests
    }
}

/// Strict, canonical Hermes reflection accepted as untrusted evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalReflection {
    binding: ReflectionBinding,
    summary: String,
    findings: Vec<ReflectionFinding>,
    next_actions: Vec<String>,
    canonical_bytes: Vec<u8>,
    output_digest: ContentDigest,
}

impl CanonicalReflection {
    #[must_use]
    pub const fn binding(&self) -> &ReflectionBinding {
        &self.binding
    }

    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    #[must_use]
    pub fn findings(&self) -> &[ReflectionFinding] {
        &self.findings
    }

    #[must_use]
    pub fn next_actions(&self) -> &[String] {
        &self.next_actions
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    #[must_use]
    pub const fn output_digest(&self) -> &ContentDigest {
        &self.output_digest
    }
}

/// One production reflection together with the normalized Hermes port
/// evidence derived from exactly the same canonical output digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesReflectionEvidence {
    reflection: CanonicalReflection,
    evidence: HermesEvidence,
}

impl HermesReflectionEvidence {
    /// Returns the strict canonical reflection payload.
    #[must_use]
    pub const fn reflection(&self) -> &CanonicalReflection {
        &self.reflection
    }

    /// Returns the normalized untrusted-candidate evidence reference.
    #[must_use]
    pub const fn evidence(&self) -> &HermesEvidence {
        &self.evidence
    }

    /// Splits the typed payload and normalized evidence without recomputing
    /// or resubmitting a Hermes run.
    #[must_use]
    pub fn into_parts(self) -> (CanonicalReflection, HermesEvidence) {
        (self.reflection, self.evidence)
    }

    fn into_evidence(self) -> HermesEvidence {
        self.evidence
    }
}

/// Synchronous production adapter for one pre-bound Hermes reflection job.
pub struct HermesReflectionAdapter {
    config: HermesAdapterConfig,
    job: HermesReflectionJob,
    active_run: Option<HermesRunRecoveryReceipt>,
}

impl HermesReflectionAdapter {
    /// Connects configuration and a fixed job without performing I/O.
    ///
    /// # Errors
    ///
    /// This constructor currently has no runtime failure after both inputs
    /// pass their own constructors; the result shape matches the adapter's
    /// other fail-closed entry points.
    pub fn connect(
        config: HermesAdapterConfig,
        job: HermesReflectionJob,
    ) -> HermesAdapterResult<Self> {
        Ok(Self {
            config,
            job,
            active_run: None,
        })
    }

    /// Executes capabilities -> run submission -> SSE -> bound status.
    ///
    /// # Errors
    ///
    /// Fails closed on unavailable capabilities, timeout, malformed output,
    /// failed/cancelled runs, or any request/run/session/input/model binding
    /// mismatch.
    pub fn run_reflection(
        &mut self,
        request: &HermesResearchRequest,
    ) -> HermesAdapterResult<CanonicalReflection> {
        self.require_containment_receipt()?;
        self.verify_request(request)?;
        if let Some(receipt) = &self.active_run {
            return Err(error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_RUN_RECONCILIATION_REQUIRED",
            )
            .with_recovery_receipt(receipt.clone()));
        }
        let deadline = self.operation_deadline()?;
        self.verify_capabilities(deadline)?;
        let run_id = match self.submit_run(deadline) {
            Ok(run_id) => run_id,
            Err(failure)
                if matches!(
                    failure.kind(),
                    HermesAdapterErrorKind::Timeout | HermesAdapterErrorKind::Transport
                ) =>
            {
                let receipt = recovery_receipt(&self.job, None);
                self.active_run = Some(receipt.clone());
                return Err(failure.with_recovery_receipt(receipt));
            }
            Err(failure) => return Err(failure),
        };
        let receipt = recovery_receipt(&self.job, Some(run_id.clone()));
        self.active_run = Some(receipt.clone());
        let event_output = match self.read_events(&run_id, deadline) {
            Ok(output) => output,
            Err(failure)
                if matches!(
                    failure.kind(),
                    HermesAdapterErrorKind::Timeout | HermesAdapterErrorKind::Transport
                ) =>
            {
                None
            }
            Err(failure) => return Err(failure.with_recovery_receipt(receipt.clone())),
        };
        let status_output = match self.poll_terminal(&run_id, deadline) {
            Ok(output) => output,
            Err(failure) => {
                if matches!(
                    failure.kind(),
                    HermesAdapterErrorKind::Failed | HermesAdapterErrorKind::Cancelled
                ) {
                    self.active_run = None;
                    return Err(failure);
                }
                return Err(failure.with_recovery_receipt(receipt));
            }
        };
        if event_output
            .as_ref()
            .is_some_and(|output| output != &status_output)
        {
            return Err(
                cross_binding("HERMES_EVENT_STATUS_OUTPUT_MISMATCH").with_recovery_receipt(receipt)
            );
        }
        let reflection = parse_reflection(&status_output, &self.job)
            .map_err(|failure| failure.with_recovery_receipt(receipt))?;
        self.active_run = None;
        Ok(reflection)
    }

    /// Executes one contained production reflection and returns both the
    /// canonical payload and its normalized Hermes evidence reference.
    ///
    /// # Errors
    ///
    /// Denies uncontained endpoints and otherwise preserves every strict
    /// capability, transport, recovery, schema, and cross-binding failure
    /// from [`Self::run_reflection`].
    pub fn run_reflection_evidence(
        &mut self,
        request: &HermesResearchRequest,
    ) -> PortResult<HermesReflectionEvidence> {
        self.require_containment_receipt()
            .map_err(|failure| map_port_error(&failure))?;
        let invocation = request.invocation().clone();
        let reflection = self
            .run_reflection(request)
            .map_err(|failure| map_port_error(&failure))?;
        let evidence = HermesEvidence::new(
            invocation,
            RuntimeKind::Live,
            reflection.output_digest().clone(),
        );
        Ok(HermesReflectionEvidence {
            reflection,
            evidence,
        })
    }

    /// Reconciles one known run against the same Hermes server process.
    ///
    /// # Errors
    ///
    /// Rejects unsafe run IDs and every cross-bound or malformed terminal.
    pub fn reconcile_reflection(
        &mut self,
        request: &HermesResearchRequest,
        receipt: &HermesRunRecoveryReceipt,
    ) -> HermesAdapterResult<CanonicalReflection> {
        self.require_containment_receipt()?;
        self.verify_request(request)?;
        if self.active_run.as_ref() != Some(receipt) {
            return Err(cross_binding("HERMES_RECOVERY_RECEIPT_BINDING_REJECTED"));
        }
        let run_id = receipt.run_id().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_SUBMISSION_OUTCOME_UNKNOWN",
            )
            .with_recovery_receipt(receipt.clone())
        })?;
        validate_run_id(run_id)?;
        let deadline = self.operation_deadline()?;
        self.verify_capabilities(deadline)?;
        let output = self
            .poll_terminal(run_id, deadline)
            .map_err(|failure| failure.with_recovery_receipt(receipt.clone()))?;
        let reflection = parse_reflection(&output, &self.job)
            .map_err(|failure| failure.with_recovery_receipt(receipt.clone()))?;
        self.active_run = None;
        Ok(reflection)
    }

    #[must_use]
    pub const fn active_recovery_receipt(&self) -> Option<&HermesRunRecoveryReceipt> {
        self.active_run.as_ref()
    }

    fn require_containment_receipt(&self) -> HermesAdapterResult<&HermesContainmentReceipt> {
        let receipt = self.config.containment_receipt.as_ref().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_LIVE_RUNTIME_RECEIPT_REQUIRED",
            )
        })?;
        receipt.verify_binding(self.config.endpoint, &self.config.api_key)?;
        Ok(receipt)
    }

    fn verify_request(&self, request: &HermesResearchRequest) -> HermesAdapterResult<()> {
        if request != self.job.request() {
            return Err(cross_binding("HERMES_REQUEST_JOB_BINDING_REJECTED"));
        }
        Ok(())
    }

    fn operation_deadline(&self) -> HermesAdapterResult<Instant> {
        Instant::now()
            .checked_add(self.config.timeout())
            .ok_or_else(|| {
                error(
                    HermesAdapterErrorKind::Timeout,
                    "HERMES_OPERATION_DEADLINE_REJECTED",
                )
            })
    }

    fn verify_capabilities(&self, deadline: Instant) -> HermesAdapterResult<()> {
        let response = self.http(
            "GET",
            "/v1/capabilities",
            "application/json",
            None,
            deadline,
        )?;
        require_status(&response, 200, "HERMES_CAPABILITIES_HTTP_REJECTED")?;
        let value = parse_json_body(&response, "HERMES_CAPABILITIES_MALFORMED")?;
        let object = value
            .as_object()
            .ok_or_else(|| malformed("HERMES_CAPABILITIES_MALFORMED"))?;
        validate_capability_identity(object, self.job.model())?;
        validate_capability_runtime(object, self.config.containment_receipt.is_some())?;
        validate_capability_features(object)?;
        validate_capability_endpoints(object)
    }

    fn submit_run(&self, deadline: Instant) -> HermesAdapterResult<String> {
        let body = json!({
            "input": self.job.prompt(),
            "instructions": READ_ONLY_INSTRUCTIONS,
            "session_id": self.job.session_id(),
            "model": self.job.model(),
        })
        .to_string();
        let response = self.http(
            "POST",
            "/v1/runs",
            "application/json",
            Some(&body),
            deadline,
        )?;
        require_status(&response, 202, "HERMES_RUN_SUBMIT_HTTP_REJECTED")?;
        let value = parse_json_body(&response, "HERMES_RUN_SUBMIT_MALFORMED")?;
        let object = value
            .as_object()
            .ok_or_else(|| malformed("HERMES_RUN_SUBMIT_MALFORMED"))?;
        if object.len() != 2
            || require_string(object, "status", "HERMES_RUN_SUBMIT_MALFORMED")? != "started"
        {
            return Err(malformed("HERMES_RUN_SUBMIT_MALFORMED"));
        }
        let run_id = require_string(object, "run_id", "HERMES_RUN_SUBMIT_MALFORMED")?;
        validate_run_id(run_id)?;
        Ok(run_id.to_owned())
    }

    fn read_events(&self, run_id: &str, deadline: Instant) -> HermesAdapterResult<Option<String>> {
        let path = format!("/v1/runs/{run_id}/events");
        let response = self.http("GET", &path, "text/event-stream", None, deadline)?;
        if matches!(response.status, 404 | 409 | 410 | 503) {
            return Err(error(
                HermesAdapterErrorKind::Transport,
                "HERMES_EVENTS_RECOVERABLE_UNAVAILABLE",
            ));
        }
        require_status(&response, 200, "HERMES_EVENTS_HTTP_REJECTED")?;
        let content_type = response
            .header("content-type")
            .ok_or_else(|| malformed("HERMES_EVENTS_CONTENT_TYPE_MISSING"))?;
        if !content_type
            .to_ascii_lowercase()
            .starts_with("text/event-stream")
        {
            return Err(malformed("HERMES_EVENTS_CONTENT_TYPE_REJECTED"));
        }
        let body = std::str::from_utf8(&response.body)
            .map_err(|_| malformed("HERMES_EVENTS_UTF8_REJECTED"))?;
        parse_sse_terminal(body, run_id)
    }

    fn poll_terminal(&self, run_id: &str, deadline: Instant) -> HermesAdapterResult<String> {
        loop {
            let path = format!("/v1/runs/{run_id}");
            let response = self.http("GET", &path, "application/json", None, deadline)?;
            if response.status == 404 {
                return Err(error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_RUN_NOT_RECOVERABLE",
                ));
            }
            require_status(&response, 200, "HERMES_STATUS_HTTP_REJECTED")?;
            match parse_status(&response, run_id, &self.job)? {
                RunState::Pending => {
                    if Instant::now() >= deadline {
                        return Err(error(
                            HermesAdapterErrorKind::Timeout,
                            "HERMES_RUN_DEADLINE_EXCEEDED",
                        ));
                    }
                    let remaining = remaining_until(deadline)?;
                    thread::sleep(self.config.poll_interval().min(remaining));
                }
                RunState::Completed(output) => return Ok(output),
            }
        }
    }

    fn http(
        &self,
        method: &str,
        path: &str,
        accept: &str,
        body: Option<&str>,
        deadline: Instant,
    ) -> HermesAdapterResult<HttpResponse> {
        let mut stream =
            TcpStream::connect_timeout(&self.config.endpoint(), remaining_until(deadline)?)
                .map_err(|failure| map_io_error(&failure))?;
        stream
            .set_read_timeout(Some(remaining_until(deadline)?))
            .map_err(|failure| map_io_error(&failure))?;
        stream
            .set_write_timeout(Some(remaining_until(deadline)?))
            .map_err(|failure| map_io_error(&failure))?;
        let body_bytes = body.unwrap_or_default().as_bytes();
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nAccept: {accept}\r\nConnection: close\r\n",
            self.config.endpoint(),
            self.config.api_key,
        );
        if body.is_some() {
            request.push_str("Content-Type: application/json\r\n");
        }
        FmtWrite::write_fmt(
            &mut request,
            format_args!("Content-Length: {}\r\n\r\n", body_bytes.len()),
        )
        .expect("writing to a String cannot fail");
        stream
            .write_all(request.as_bytes())
            .map_err(|failure| map_io_error(&failure))?;
        if !body_bytes.is_empty() {
            stream
                .write_all(body_bytes)
                .map_err(|failure| map_io_error(&failure))?;
        }
        stream.flush().map_err(|failure| map_io_error(&failure))?;
        let _ = stream.shutdown(Shutdown::Write);

        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8_192];
        loop {
            stream
                .set_read_timeout(Some(remaining_until(deadline)?))
                .map_err(|failure| map_io_error(&failure))?;
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if bytes.len().saturating_add(count) > MAX_HTTP_RESPONSE_BYTES {
                        return Err(malformed("HERMES_HTTP_RESPONSE_LIMIT_EXCEEDED"));
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                Err(failure) => return Err(map_io_error(&failure)),
            }
        }
        parse_http_response(&bytes)
    }
}

impl HermesPort for HermesReflectionAdapter {
    fn research(&mut self, request: HermesResearchRequest) -> PortResult<HermesEvidence> {
        self.run_reflection_evidence(&request)
            .map(HermesReflectionEvidence::into_evidence)
    }

    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()> {
        self.require_containment_receipt()
            .map_err(|failure| map_port_error(&failure))?;
        if request_id != self.job.request().invocation().request_id() {
            return Err(PortError::new(
                Component::Hermes,
                PortErrorKind::Denied,
                "HERMES_INTERRUPT_REQUEST_BINDING_REJECTED",
            ));
        }
        let receipt = self.active_run.clone().ok_or_else(|| {
            PortError::new(
                Component::Hermes,
                PortErrorKind::Unavailable,
                "HERMES_INTERRUPT_NO_ACTIVE_RUN",
            )
        })?;
        let run_id = receipt.run_id().ok_or_else(|| {
            PortError::new(
                Component::Hermes,
                PortErrorKind::Ambiguous,
                "HERMES_SUBMISSION_OUTCOME_UNKNOWN",
            )
        })?;
        let path = format!("/v1/runs/{run_id}/stop");
        let deadline = self
            .operation_deadline()
            .map_err(|failure| map_port_error(&failure))?;
        let response = self
            .http("POST", &path, "application/json", Some("{}"), deadline)
            .map_err(|failure| map_port_error(&failure))?;
        require_status(&response, 200, "HERMES_STOP_HTTP_REJECTED")
            .map_err(|failure| map_port_error(&failure))?;
        let value = parse_json_body(&response, "HERMES_STOP_MALFORMED")
            .map_err(|failure| map_port_error(&failure))?;
        let object = value
            .as_object()
            .ok_or_else(|| map_port_error(&malformed("HERMES_STOP_MALFORMED")))?;
        if object.len() != 2
            || require_string(object, "run_id", "HERMES_STOP_MALFORMED")
                .map_err(|failure| map_port_error(&failure))?
                != run_id
            || require_string(object, "status", "HERMES_STOP_MALFORMED")
                .map_err(|failure| map_port_error(&failure))?
                != "stopping"
        {
            return Err(map_port_error(&malformed("HERMES_STOP_MALFORMED")));
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReflection {
    schema_version: String,
    binding: RawBinding,
    summary: String,
    findings: Vec<RawFinding>,
    next_actions: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBinding {
    request_id: String,
    task_id: String,
    attempt_id: String,
    project_snapshot_id: String,
    subject_digest: String,
    session_id: String,
    input_digest: String,
    model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFinding {
    classification: String,
    statement: String,
    evidence_digests: Vec<String>,
}

fn parse_reflection(
    output: &str,
    job: &HermesReflectionJob,
) -> HermesAdapterResult<CanonicalReflection> {
    let raw: RawReflection =
        serde_json::from_str(output).map_err(|_| malformed("HERMES_REFLECTION_SCHEMA_REJECTED"))?;
    if raw.schema_version != HERMES_SCHEMA_VERSION {
        return Err(malformed("HERMES_REFLECTION_VERSION_REJECTED"));
    }
    let invocation = job.request().invocation();
    let expected = ReflectionBinding {
        request_id: invocation.request_id().as_str().to_owned(),
        task_id: invocation.task_id().as_str().to_owned(),
        attempt_id: invocation.attempt_id().as_str().to_owned(),
        project_snapshot_id: invocation.project_snapshot_id().as_str().to_owned(),
        subject_digest: invocation.subject_digest().as_str().to_owned(),
        session_id: job.session_id().to_owned(),
        input_digest: job.input_digest().as_str().to_owned(),
        model: job.model().to_owned(),
    };
    let actual = ReflectionBinding {
        request_id: raw.binding.request_id,
        task_id: raw.binding.task_id,
        attempt_id: raw.binding.attempt_id,
        project_snapshot_id: raw.binding.project_snapshot_id,
        subject_digest: raw.binding.subject_digest,
        session_id: raw.binding.session_id,
        input_digest: raw.binding.input_digest,
        model: raw.binding.model,
    };
    if actual != expected {
        return Err(cross_binding("HERMES_REFLECTION_BINDING_REJECTED"));
    }
    validate_redacted_text(
        &raw.summary,
        MAX_REFLECTION_SUMMARY_BYTES,
        "HERMES_REFLECTION_SUMMARY_REJECTED",
    )?;
    if raw.findings.is_empty() || raw.findings.len() > MAX_FINDINGS {
        return Err(malformed("HERMES_REFLECTION_FINDING_COUNT_REJECTED"));
    }
    if raw.next_actions.len() > MAX_NEXT_ACTIONS {
        return Err(malformed("HERMES_REFLECTION_ACTION_COUNT_REJECTED"));
    }
    let allowed_digests = job
        .evidence
        .iter()
        .map(|evidence| evidence.digest().as_str())
        .collect::<HashSet<_>>();
    let findings = raw
        .findings
        .into_iter()
        .map(|finding| {
            let classification = match finding.classification.as_str() {
                "inference" => ReflectionClassification::Inference,
                _ => return Err(malformed("HERMES_REFLECTION_CLASSIFICATION_REJECTED")),
            };
            validate_redacted_text(
                &finding.statement,
                MAX_TEXT_BYTES,
                "HERMES_REFLECTION_STATEMENT_REJECTED",
            )?;
            if finding.evidence_digests.is_empty()
                || finding.evidence_digests.len() > MAX_EVIDENCE_ITEMS
            {
                return Err(malformed("HERMES_REFLECTION_EVIDENCE_COUNT_REJECTED"));
            }
            let mut seen = HashSet::new();
            for digest in &finding.evidence_digests {
                if !allowed_digests.contains(digest.as_str()) || !seen.insert(digest) {
                    return Err(cross_binding("HERMES_REFLECTION_EVIDENCE_BINDING_REJECTED"));
                }
            }
            Ok(ReflectionFinding {
                classification,
                statement: finding.statement,
                evidence_digests: finding.evidence_digests,
            })
        })
        .collect::<HermesAdapterResult<Vec<_>>>()?;
    validate_reflection_actions(&raw.next_actions)?;
    let canonical_value = reflection_value(&actual, &raw.summary, &findings, &raw.next_actions);
    let canonical_bytes = canonicalize(&canonical_value)
        .map_err(|_| malformed("HERMES_REFLECTION_CANONICALIZATION_REJECTED"))?
        .into_vec();
    let domain = HashDomain::new("lattice.hermes.reflection", "1")
        .map_err(|_| malformed("HERMES_REFLECTION_DOMAIN_REJECTED"))?;
    let output_digest = ContentDigest::from_sha256(
        canonical_sha256(&domain, &canonical_value)
            .map_err(|_| malformed("HERMES_REFLECTION_CANONICALIZATION_REJECTED"))?
            .to_hex(),
    )
    .map_err(|_| malformed("HERMES_REFLECTION_DIGEST_REJECTED"))?;
    Ok(CanonicalReflection {
        binding: actual,
        summary: raw.summary,
        findings,
        next_actions: raw.next_actions,
        canonical_bytes,
        output_digest,
    })
}

fn validate_reflection_actions(actions: &[String]) -> HermesAdapterResult<()> {
    for action in actions {
        validate_redacted_text(action, MAX_TEXT_BYTES, "HERMES_REFLECTION_ACTION_REJECTED")?;
    }
    Ok(())
}

fn input_value(
    request: &HermesResearchRequest,
    session_id: &str,
    model: &str,
    evidence: &[ReflectionEvidence],
) -> CanonicalValue {
    let invocation = request.invocation();
    CanonicalValue::Object(vec![
        (
            "binding".to_owned(),
            CanonicalValue::Object(vec![
                string_entry("attempt_id", invocation.attempt_id().as_str()),
                string_entry("model", model),
                string_entry(
                    "project_snapshot_id",
                    invocation.project_snapshot_id().as_str(),
                ),
                string_entry("request_id", invocation.request_id().as_str()),
                string_entry("session_id", session_id),
                string_entry("subject_digest", invocation.subject_digest().as_str()),
                string_entry("task_id", invocation.task_id().as_str()),
            ]),
        ),
        (
            "evidence".to_owned(),
            CanonicalValue::Array(
                evidence
                    .iter()
                    .map(|item| {
                        CanonicalValue::Object(vec![
                            string_entry("digest", item.digest().as_str()),
                            string_entry("kind", item.kind().as_str()),
                            (
                                "sensitive_value_digests".to_owned(),
                                CanonicalValue::Array(
                                    item.sensitive_value_digests()
                                        .iter()
                                        .map(|digest| {
                                            CanonicalValue::String(digest.as_str().to_owned())
                                        })
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        string_entry("schema_version", "lattice.hermes.reflection-input.v1"),
    ])
}

fn reflection_value(
    binding: &ReflectionBinding,
    summary: &str,
    findings: &[ReflectionFinding],
    next_actions: &[String],
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "binding".to_owned(),
            CanonicalValue::Object(vec![
                string_entry("attempt_id", binding.attempt_id()),
                string_entry("input_digest", binding.input_digest()),
                string_entry("model", binding.model()),
                string_entry("project_snapshot_id", binding.project_snapshot_id()),
                string_entry("request_id", binding.request_id()),
                string_entry("session_id", binding.session_id()),
                string_entry("subject_digest", binding.subject_digest()),
                string_entry("task_id", binding.task_id()),
            ]),
        ),
        (
            "findings".to_owned(),
            CanonicalValue::Array(
                findings
                    .iter()
                    .map(|finding| {
                        CanonicalValue::Object(vec![
                            string_entry("classification", finding.classification().as_str()),
                            (
                                "evidence_digests".to_owned(),
                                CanonicalValue::Array(
                                    finding
                                        .evidence_digests()
                                        .iter()
                                        .map(|digest| CanonicalValue::String(digest.clone()))
                                        .collect(),
                                ),
                            ),
                            string_entry("statement", finding.statement()),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "next_actions".to_owned(),
            CanonicalValue::Array(
                next_actions
                    .iter()
                    .map(|action| CanonicalValue::String(action.clone()))
                    .collect(),
            ),
        ),
        string_entry("schema_version", HERMES_SCHEMA_VERSION),
        string_entry("summary", summary),
    ])
}

fn string_entry(key: &str, value: &str) -> (String, CanonicalValue) {
    (key.to_owned(), CanonicalValue::String(value.to_owned()))
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn parse_http_response(bytes: &[u8]) -> HermesAdapterResult<HttpResponse> {
    let header_end =
        find_bytes(bytes, b"\r\n\r\n").ok_or_else(|| malformed("HERMES_HTTP_HEADERS_MALFORMED"))?;
    let header_bytes = &bytes[..header_end];
    let raw_body = &bytes[header_end + 4..];
    let headers_text = std::str::from_utf8(header_bytes)
        .map_err(|_| malformed("HERMES_HTTP_HEADERS_MALFORMED"))?;
    let mut lines = headers_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| malformed("HERMES_HTTP_STATUS_MALFORMED"))?;
    let mut status_parts = status_line.split_whitespace();
    if status_parts.next() != Some("HTTP/1.1") {
        return Err(malformed("HERMES_HTTP_STATUS_MALFORMED"));
    }
    let status = status_parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| malformed("HERMES_HTTP_STATUS_MALFORMED"))?;
    let headers = lines
        .map(|line| {
            let (name, value) = line
                .split_once(':')
                .ok_or_else(|| malformed("HERMES_HTTP_HEADER_MALFORMED"))?;
            if name.trim().is_empty() {
                return Err(malformed("HERMES_HTTP_HEADER_MALFORMED"));
            }
            Ok((name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        })
        .collect::<HermesAdapterResult<Vec<_>>>()?;
    let chunked = headers
        .iter()
        .any(|(name, value)| name == "transfer-encoding" && value.eq_ignore_ascii_case("chunked"));
    let body = if chunked {
        decode_chunked(raw_body)?
    } else {
        if headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .and_then(|(_, value)| value.parse::<usize>().ok())
            .is_some_and(|length| length != raw_body.len())
        {
            return Err(malformed("HERMES_HTTP_BODY_LENGTH_MISMATCH"));
        }
        raw_body.to_vec()
    };
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn decode_chunked(mut bytes: &[u8]) -> HermesAdapterResult<Vec<u8>> {
    let mut output = Vec::new();
    loop {
        let line_end =
            find_bytes(bytes, b"\r\n").ok_or_else(|| malformed("HERMES_HTTP_CHUNK_MALFORMED"))?;
        let size_text = std::str::from_utf8(&bytes[..line_end])
            .map_err(|_| malformed("HERMES_HTTP_CHUNK_MALFORMED"))?;
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or_default(), 16)
            .map_err(|_| malformed("HERMES_HTTP_CHUNK_MALFORMED"))?;
        bytes = &bytes[line_end + 2..];
        if size == 0 {
            break;
        }
        if bytes.len() < size + 2 || &bytes[size..size + 2] != b"\r\n" {
            return Err(malformed("HERMES_HTTP_CHUNK_MALFORMED"));
        }
        if output.len().saturating_add(size) > MAX_HTTP_RESPONSE_BYTES {
            return Err(malformed("HERMES_HTTP_RESPONSE_LIMIT_EXCEEDED"));
        }
        output.extend_from_slice(&bytes[..size]);
        bytes = &bytes[size + 2..];
    }
    Ok(output)
}

fn parse_json_body(response: &HttpResponse, code: &'static str) -> HermesAdapterResult<Value> {
    let content_type = response
        .header("content-type")
        .ok_or_else(|| malformed(code))?;
    if !content_type
        .to_ascii_lowercase()
        .starts_with("application/json")
    {
        return Err(malformed(code));
    }
    serde_json::from_slice(&response.body).map_err(|_| malformed(code))
}

fn parse_sse_terminal(body: &str, expected_run_id: &str) -> HermesAdapterResult<Option<String>> {
    let normalized = body.replace("\r\n", "\n");
    let mut terminal = None;
    for block in normalized.split("\n\n") {
        let data = block
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>();
        if data.is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&data.join("\n"))
            .map_err(|_| malformed("HERMES_EVENT_MALFORMED"))?;
        let object = value
            .as_object()
            .ok_or_else(|| malformed("HERMES_EVENT_MALFORMED"))?;
        let run_id = require_string(object, "run_id", "HERMES_EVENT_MALFORMED")?;
        if run_id != expected_run_id {
            return Err(cross_binding("HERMES_EVENT_RUN_BINDING_REJECTED"));
        }
        if !object
            .get("timestamp")
            .is_some_and(serde_json::Value::is_number)
        {
            return Err(malformed("HERMES_EVENT_MALFORMED"));
        }
        match require_string(object, "event", "HERMES_EVENT_MALFORMED")? {
            "run.completed" => {
                require_only_keys(
                    object,
                    &["event", "run_id", "timestamp", "output", "usage"],
                    "HERMES_EVENT_UNKNOWN_FIELD",
                )?;
                if let Some(usage) = object.get("usage") {
                    let usage = usage
                        .as_object()
                        .ok_or_else(|| malformed("HERMES_EVENT_MALFORMED"))?;
                    require_only_keys(
                        usage,
                        &["input_tokens", "output_tokens", "total_tokens"],
                        "HERMES_EVENT_UNKNOWN_FIELD",
                    )?;
                    if usage.values().any(|value| !value.is_u64()) {
                        return Err(malformed("HERMES_EVENT_MALFORMED"));
                    }
                }
                let output = require_string(object, "output", "HERMES_EVENT_MALFORMED")?;
                if terminal.replace(output.to_owned()).is_some() {
                    return Err(malformed("HERMES_DUPLICATE_TERMINAL_EVENT"));
                }
            }
            "run.failed" => {
                require_only_keys(
                    object,
                    &["event", "run_id", "timestamp", "error"],
                    "HERMES_EVENT_UNKNOWN_FIELD",
                )?;
                require_string(object, "error", "HERMES_EVENT_MALFORMED")?;
                return Err(error(HermesAdapterErrorKind::Failed, "HERMES_RUN_FAILED"));
            }
            "run.cancelled" => {
                require_only_keys(
                    object,
                    &["event", "run_id", "timestamp"],
                    "HERMES_EVENT_UNKNOWN_FIELD",
                )?;
                return Err(error(
                    HermesAdapterErrorKind::Cancelled,
                    "HERMES_RUN_CANCELLED",
                ));
            }
            "message.delta" => {
                require_only_keys(
                    object,
                    &["event", "run_id", "timestamp", "delta"],
                    "HERMES_EVENT_UNKNOWN_FIELD",
                )?;
                require_string(object, "delta", "HERMES_EVENT_MALFORMED")?;
            }
            "reasoning.available" => {
                require_only_keys(
                    object,
                    &["event", "run_id", "timestamp", "text"],
                    "HERMES_EVENT_UNKNOWN_FIELD",
                )?;
                require_string(object, "text", "HERMES_EVENT_MALFORMED")?;
            }
            "tool.started" | "tool.completed" | "approval.request" | "subagent.start"
            | "subagent.complete" => {
                return Err(error(
                    HermesAdapterErrorKind::CapabilityMismatch,
                    "HERMES_UNEXPECTED_EXECUTION_EVENT",
                ));
            }
            _ => return Err(malformed("HERMES_EVENT_DISCRIMINATOR_REJECTED")),
        }
    }
    Ok(terminal)
}

enum RunState {
    Pending,
    Completed(String),
}

fn parse_status(
    response: &HttpResponse,
    expected_run_id: &str,
    job: &HermesReflectionJob,
) -> HermesAdapterResult<RunState> {
    let value = parse_json_body(response, "HERMES_STATUS_MALFORMED")?;
    let object = value
        .as_object()
        .ok_or_else(|| malformed("HERMES_STATUS_MALFORMED"))?;
    require_only_keys(
        object,
        &[
            "object",
            "run_id",
            "status",
            "created_at",
            "updated_at",
            "session_id",
            "model",
            "last_event",
            "output",
            "usage",
            "error",
        ],
        "HERMES_STATUS_UNKNOWN_FIELD",
    )?;
    if let Some(usage) = object.get("usage") {
        let usage = usage
            .as_object()
            .ok_or_else(|| malformed("HERMES_STATUS_MALFORMED"))?;
        require_only_keys(
            usage,
            &["input_tokens", "output_tokens", "total_tokens"],
            "HERMES_STATUS_UNKNOWN_FIELD",
        )?;
        if usage.values().any(|value| !value.is_u64()) {
            return Err(malformed("HERMES_STATUS_MALFORMED"));
        }
    }
    if require_string(object, "object", "HERMES_STATUS_MALFORMED")? != "hermes.run" {
        return Err(malformed("HERMES_STATUS_OBJECT_REJECTED"));
    }
    if require_string(object, "run_id", "HERMES_STATUS_MALFORMED")? != expected_run_id {
        return Err(cross_binding("HERMES_STATUS_RUN_BINDING_REJECTED"));
    }
    if require_string(object, "session_id", "HERMES_STATUS_MALFORMED")? != job.session_id() {
        return Err(cross_binding("HERMES_STATUS_SESSION_BINDING_REJECTED"));
    }
    if require_string(object, "model", "HERMES_STATUS_MALFORMED")? != job.model() {
        return Err(cross_binding("HERMES_STATUS_MODEL_BINDING_REJECTED"));
    }
    match require_string(object, "status", "HERMES_STATUS_MALFORMED")? {
        "queued" | "running" | "stopping" => Ok(RunState::Pending),
        "completed" => Ok(RunState::Completed(
            require_string(object, "output", "HERMES_STATUS_MALFORMED")?.to_owned(),
        )),
        "failed" => Err(error(HermesAdapterErrorKind::Failed, "HERMES_RUN_FAILED")),
        "cancelled" => Err(error(
            HermesAdapterErrorKind::Cancelled,
            "HERMES_RUN_CANCELLED",
        )),
        _ => Err(malformed("HERMES_STATUS_VALUE_REJECTED")),
    }
}

fn validate_capability_identity(
    object: &Map<String, Value>,
    expected_model: &str,
) -> HermesAdapterResult<()> {
    require_only_keys(
        object,
        &[
            "object",
            "platform",
            "model",
            "auth",
            "runtime",
            "features",
            "endpoints",
        ],
        "HERMES_CAPABILITIES_UNKNOWN_FIELD",
    )?;
    require_string(object, "object", "HERMES_CAPABILITIES_MALFORMED").and_then(|value| {
        require_equal(
            value,
            "hermes.api_server.capabilities",
            "HERMES_CAPABILITY_OBJECT_REJECTED",
        )
    })?;
    require_string(object, "platform", "HERMES_CAPABILITIES_MALFORMED").and_then(|value| {
        require_equal(value, "hermes-agent", "HERMES_CAPABILITY_PLATFORM_REJECTED")
    })?;
    if require_string(object, "model", "HERMES_CAPABILITIES_MALFORMED")? != expected_model {
        return Err(cross_binding("HERMES_CAPABILITY_MODEL_MISMATCH"));
    }
    let auth = require_object(object, "auth", "HERMES_CAPABILITIES_MALFORMED")?;
    require_only_keys(
        auth,
        &["type", "required"],
        "HERMES_CAPABILITIES_UNKNOWN_FIELD",
    )?;
    if require_string(auth, "type", "HERMES_CAPABILITIES_MALFORMED")? != "bearer"
        || require_bool(auth, "required", "HERMES_CAPABILITIES_MALFORMED")? != Some(true)
    {
        return Err(error(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_BEARER_AUTH_REQUIRED",
        ));
    }
    Ok(())
}

fn validate_capability_runtime(
    object: &Map<String, Value>,
    has_containment_receipt: bool,
) -> HermesAdapterResult<&str> {
    let runtime = require_object(object, "runtime", "HERMES_CAPABILITIES_MALFORMED")?;
    require_only_keys(
        runtime,
        &["mode", "tool_execution", "split_runtime", "description"],
        "HERMES_CAPABILITIES_UNKNOWN_FIELD",
    )?;
    if require_string(runtime, "mode", "HERMES_CAPABILITIES_MALFORMED")? != "server_agent"
        || require_bool(runtime, "split_runtime", "HERMES_CAPABILITIES_MALFORMED")? != Some(false)
    {
        return Err(error(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_RUNTIME_CAPABILITY_REJECTED",
        ));
    }
    let tool_execution =
        require_string(runtime, "tool_execution", "HERMES_CAPABILITIES_MALFORMED")?;
    if tool_execution != "server" {
        return Err(error(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_RUNTIME_CAPABILITY_REJECTED",
        ));
    }
    if !has_containment_receipt {
        return Err(error(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_SERVER_TOOL_CONTAINMENT_REQUIRED",
        ));
    }
    Ok(tool_execution)
}

fn validate_capability_features(object: &Map<String, Value>) -> HermesAdapterResult<()> {
    let features = require_object(object, "features", "HERMES_CAPABILITIES_MALFORMED")?;
    require_only_keys(
        features,
        CAPABILITY_FEATURE_FIELDS,
        "HERMES_CAPABILITIES_UNKNOWN_FIELD",
    )?;
    for name in ["run_submission", "run_status", "run_events_sse", "run_stop"] {
        if require_bool(features, name, "HERMES_CAPABILITIES_MALFORMED")? != Some(true) {
            return Err(error(
                HermesAdapterErrorKind::CapabilityMismatch,
                "HERMES_REQUIRED_CAPABILITY_MISSING",
            ));
        }
    }
    for name in ["admin_config_rw", "memory_write_api"] {
        if require_bool(features, name, "HERMES_CAPABILITIES_MALFORMED")? != Some(false) {
            return Err(error(
                HermesAdapterErrorKind::CapabilityMismatch,
                "HERMES_MUTATING_CAPABILITY_REJECTED",
            ));
        }
    }
    Ok(())
}

fn validate_capability_endpoints(object: &Map<String, Value>) -> HermesAdapterResult<()> {
    let Some(endpoints) = object.get("endpoints") else {
        return Ok(());
    };
    let endpoints = endpoints
        .as_object()
        .ok_or_else(|| malformed("HERMES_CAPABILITIES_MALFORMED"))?;
    require_only_keys(
        endpoints,
        CAPABILITY_ENDPOINT_FIELDS,
        "HERMES_CAPABILITIES_UNKNOWN_FIELD",
    )?;
    for endpoint in endpoints.values() {
        let endpoint = endpoint
            .as_object()
            .ok_or_else(|| malformed("HERMES_CAPABILITIES_MALFORMED"))?;
        require_only_keys(
            endpoint,
            &["method", "path"],
            "HERMES_CAPABILITIES_UNKNOWN_FIELD",
        )?;
        require_string(endpoint, "method", "HERMES_CAPABILITIES_MALFORMED")?;
        require_string(endpoint, "path", "HERMES_CAPABILITIES_MALFORMED")?;
    }
    Ok(())
}

fn require_object<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    code: &'static str,
) -> HermesAdapterResult<&'a Map<String, Value>> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| malformed(code))
}

fn require_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    code: &'static str,
) -> HermesAdapterResult<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| malformed(code))
}

fn require_bool(
    object: &Map<String, Value>,
    key: &str,
    code: &'static str,
) -> HermesAdapterResult<Option<bool>> {
    object
        .get(key)
        .map(|value| value.as_bool().ok_or_else(|| malformed(code)))
        .transpose()
}

fn require_only_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    code: &'static str,
) -> HermesAdapterResult<()> {
    if object
        .keys()
        .any(|key| !allowed.iter().any(|allowed_key| key == allowed_key))
    {
        return Err(malformed(code));
    }
    Ok(())
}

fn require_equal(actual: &str, expected: &str, code: &'static str) -> HermesAdapterResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(error(HermesAdapterErrorKind::CapabilityMismatch, code))
    }
}

fn require_status(
    response: &HttpResponse,
    expected: u16,
    code: &'static str,
) -> HermesAdapterResult<()> {
    if response.status == expected {
        Ok(())
    } else {
        Err(error(HermesAdapterErrorKind::HttpStatus, code))
    }
}

fn recovery_receipt(job: &HermesReflectionJob, run_id: Option<String>) -> HermesRunRecoveryReceipt {
    HermesRunRecoveryReceipt {
        run_id,
        request_id: job.request().invocation().request_id().as_str().to_owned(),
        session_id: job.session_id().to_owned(),
        input_digest: job.input_digest().clone(),
        model: job.model().to_owned(),
    }
}

fn remaining_until(deadline: Instant) -> HermesAdapterResult<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(error(
            HermesAdapterErrorKind::Timeout,
            "HERMES_RUN_DEADLINE_EXCEEDED",
        ))
    } else {
        Ok(remaining)
    }
}

fn bounded_output(
    command: &mut Command,
    timeout: Duration,
) -> HermesAdapterResult<std::process::Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|_| {
        error(
            HermesAdapterErrorKind::Identity,
            "HERMES_VERSION_PROBE_FAILED",
        )
    })?;
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        error(
            HermesAdapterErrorKind::Timeout,
            "HERMES_VERSION_PROBE_TIMEOUT",
        )
    })?;
    loop {
        if child
            .try_wait()
            .map_err(|_| {
                error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_VERSION_PROBE_STATUS_UNKNOWN",
                )
            })?
            .is_some()
        {
            return child.wait_with_output().map_err(|_| {
                error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_VERSION_PROBE_OUTPUT_UNKNOWN",
                )
            });
        }
        let now = Instant::now();
        if now >= deadline {
            child.kill().map_err(|_| {
                error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_VERSION_PROBE_KILL_UNKNOWN",
                )
            })?;
            child.wait().map_err(|_| {
                error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_VERSION_PROBE_REAP_UNKNOWN",
                )
            })?;
            return Err(error(
                HermesAdapterErrorKind::Timeout,
                "HERMES_VERSION_PROBE_TIMEOUT",
            ));
        }
        thread::sleep(Duration::from_millis(5).min(deadline.saturating_duration_since(now)));
    }
}

fn validate_pinned_version_output(stdout: &[u8], stderr: &[u8]) -> HermesAdapterResult<()> {
    if !stderr.is_empty() {
        return Err(error(
            HermesAdapterErrorKind::Identity,
            "HERMES_VERSION_STDERR_REJECTED",
        ));
    }
    let stdout = std::str::from_utf8(stdout).map_err(|_| {
        error(
            HermesAdapterErrorKind::Identity,
            "HERMES_VERSION_UTF8_REJECTED",
        )
    })?;
    let expected = format!(
        "Hermes Agent v{HERMES_PACKAGE_VERSION} ({})",
        HERMES_RELEASE.strip_prefix('v').unwrap_or(HERMES_RELEASE)
    );
    if stdout.lines().next() != Some(expected.as_str()) {
        return Err(error(
            HermesAdapterErrorKind::Identity,
            "HERMES_PACKAGE_VERSION_MISMATCH",
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str, code: &'static str) -> HermesAdapterResult<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(error(HermesAdapterErrorKind::Identity, code));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> HermesAdapterResult<String> {
    let metadata = fs::metadata(path).map_err(|_| {
        error(
            HermesAdapterErrorKind::Identity,
            "HERMES_EXECUTABLE_METADATA_FAILED",
        )
    })?;
    if !metadata.is_file() || metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(error(
            HermesAdapterErrorKind::Identity,
            "HERMES_EXECUTABLE_FILE_REJECTED",
        ));
    }
    let mut file = fs::File::open(path).map_err(|_| {
        error(
            HermesAdapterErrorKind::Identity,
            "HERMES_EXECUTABLE_READ_FAILED",
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let count = file.read(&mut buffer).map_err(|_| {
            error(
                HermesAdapterErrorKind::Identity,
                "HERMES_EXECUTABLE_READ_FAILED",
            )
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").map_err(|_| {
            error(
                HermesAdapterErrorKind::Identity,
                "HERMES_EXECUTABLE_HASH_FAILED",
            )
        })?;
    }
    Ok(encoded)
}

fn sha256_text(value: &str) -> String {
    encode_sha256(&Sha256::digest(value.as_bytes()))
}

fn encode_sha256(digest: &[u8]) -> String {
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn validate_identifier(value: &str, max: usize, code: &'static str) -> HermesAdapterResult<()> {
    if value.is_empty()
        || value.len() > max
        || value != value.trim()
        || value != normalize_nfc(value)
        || value.chars().any(char::is_control)
    {
        return Err(malformed(code));
    }
    Ok(())
}

fn validate_text(value: &str, max: usize, code: &'static str) -> HermesAdapterResult<()> {
    if value.trim().is_empty()
        || value.len() > max
        || value != normalize_nfc(value)
        || value.contains('\0')
    {
        return Err(malformed(code));
    }
    Ok(())
}

fn validate_redacted_text(value: &str, max: usize, code: &'static str) -> HermesAdapterResult<()> {
    validate_text(value, max, code)?;
    let lower = value.to_ascii_lowercase();
    let fixed_patterns = [
        "authorization: bearer ",
        "-----begin private key-----",
        "-----begin rsa private key-----",
        "-----begin openssh private key-----",
        "postgres://",
        "postgresql://",
        "openai_api_key=",
        "openrouter_api_key=",
        "anthropic_api_key=",
        "api_key=",
        "api-key=",
        "password=",
        "passwd=",
        "secret=",
        "token=",
    ];
    if fixed_patterns.iter().any(|pattern| lower.contains(pattern))
        || contains_long_secret_prefix(&lower, "sk-")
        || contains_long_secret_prefix(&lower, "ghp_")
    {
        return Err(malformed(code));
    }
    Ok(())
}

fn contains_long_secret_prefix(value: &str, prefix: &str) -> bool {
    let mut remainder = value;
    while let Some(index) = remainder.find(prefix) {
        let candidate = &remainder[index + prefix.len()..];
        let length = candidate
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            .count();
        if length >= 16 {
            return true;
        }
        remainder = &candidate[length..];
    }
    false
}

fn validate_run_id(run_id: &str) -> HermesAdapterResult<()> {
    if !run_id.starts_with("run_")
        || run_id.len() > 128
        || !run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(malformed("HERMES_RUN_ID_REJECTED"));
    }
    Ok(())
}

fn map_io_error(failure: &std::io::Error) -> HermesAdapterError {
    if matches!(
        failure.kind(),
        IoErrorKind::TimedOut | IoErrorKind::WouldBlock
    ) {
        error(HermesAdapterErrorKind::Timeout, "HERMES_LOOPBACK_TIMEOUT")
    } else {
        error(
            HermesAdapterErrorKind::Transport,
            "HERMES_LOOPBACK_TRANSPORT_FAILED",
        )
    }
}

fn map_port_error(failure: &HermesAdapterError) -> PortError {
    let kind = match failure.kind() {
        HermesAdapterErrorKind::Configuration | HermesAdapterErrorKind::CrossBinding => {
            PortErrorKind::Denied
        }
        HermesAdapterErrorKind::Transport
        | HermesAdapterErrorKind::HttpStatus
        | HermesAdapterErrorKind::Failed
        | HermesAdapterErrorKind::Spawn => PortErrorKind::Unavailable,
        HermesAdapterErrorKind::CapabilityMismatch => PortErrorKind::CapabilityMismatch,
        HermesAdapterErrorKind::Timeout => PortErrorKind::Timeout,
        HermesAdapterErrorKind::Malformed => PortErrorKind::Malformed,
        HermesAdapterErrorKind::Cancelled => PortErrorKind::Cancelled,
        HermesAdapterErrorKind::Ambiguous => PortErrorKind::Ambiguous,
        HermesAdapterErrorKind::Identity => PortErrorKind::VersionMismatch,
    };
    PortError::new(Component::Hermes, kind, failure.code())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

const fn error(kind: HermesAdapterErrorKind, code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(kind, code)
}

const fn malformed(code: &'static str) -> HermesAdapterError {
    error(HermesAdapterErrorKind::Malformed, code)
}

const fn cross_binding(code: &'static str) -> HermesAdapterError {
    error(HermesAdapterErrorKind::CrossBinding, code)
}

fn is_daily_home(candidate: &Path, variable: &str) -> bool {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .is_some_and(|daily| same_path(candidate, &daily))
}

fn default_hermes_homes() -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        homes.push(PathBuf::from(local).join("hermes"));
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        homes.push(PathBuf::from(home).join(".hermes"));
    }
    homes
}

fn same_path(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    } else {
        left == right
    }
}

fn validate_isolation_boundary(
    isolation_root: &Path,
    product_root: &Path,
) -> HermesAdapterResult<(PathBuf, PathBuf)> {
    if !isolation_root.is_absolute()
        || isolation_root.exists()
        || isolation_root.file_name().is_none()
        || !product_root.is_absolute()
        || !product_root.is_dir()
    {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_ISOLATION_ROOT_REJECTED",
        ));
    }
    #[cfg(windows)]
    if windows_path_is_unsupported(isolation_root) || windows_path_is_unsupported(product_root) {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_ISOLATION_UNC_REJECTED",
        ));
    }
    let parent = isolation_root.parent().ok_or_else(|| {
        error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_ISOLATION_PARENT_REJECTED",
        )
    })?;
    if !parent.is_dir() {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_ISOLATION_PARENT_REJECTED",
        ));
    }
    reject_link_or_reparse_ancestors(parent)?;
    reject_link_or_reparse_ancestors(product_root)?;
    let canonical_parent = fs::canonicalize(parent).map_err(|_| {
        error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_ISOLATION_PARENT_REJECTED",
        )
    })?;
    let canonical_product = fs::canonicalize(product_root).map_err(|_| {
        error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PRODUCT_ROOT_REJECTED",
        )
    })?;
    reject_link_or_reparse_ancestors(&canonical_parent)?;
    reject_link_or_reparse_ancestors(&canonical_product)?;
    let canonical_candidate =
        canonical_parent.join(isolation_root.file_name().ok_or_else(|| {
            error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_ISOLATION_ROOT_REJECTED",
            )
        })?);
    if canonical_candidate.exists()
        || path_is_within(&canonical_candidate, &canonical_product)
        || path_is_within(&canonical_product, &canonical_candidate)
    {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PRODUCT_ROOT_OVERLAP_REJECTED",
        ));
    }
    Ok((canonical_candidate, canonical_product))
}

#[cfg(windows)]
fn windows_path_is_unsupported(path: &Path) -> bool {
    use std::path::{Component, Prefix};

    match path.components().next() {
        Some(Component::Prefix(prefix)) => {
            !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        }
        _ => true,
    }
}

fn path_is_within(candidate: &Path, ancestor: &Path) -> bool {
    let normalize = |path: &Path| {
        path.components()
            .map(|component| {
                let value = component.as_os_str().to_string_lossy().into_owned();
                if cfg!(windows) {
                    value.to_ascii_lowercase()
                } else {
                    value
                }
            })
            .collect::<Vec<_>>()
    };
    let candidate = normalize(candidate);
    let ancestor = normalize(ancestor);
    candidate.len() >= ancestor.len() && candidate[..ancestor.len()] == ancestor
}

fn reject_link_or_reparse_ancestors(path: &Path) -> HermesAdapterResult<()> {
    for ancestor in path.ancestors() {
        if !ancestor.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| {
            error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_ISOLATION_PATH_METADATA_FAILED",
            )
        })?;
        if metadata.file_type().is_symlink() || metadata_is_reparse_point(&metadata) {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_ISOLATION_REPARSE_REJECTED",
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
const fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn prepare_isolated_run(config: &HermesProcessConfig) -> HermesAdapterResult<()> {
    let (isolation_root, product_root) =
        validate_isolation_boundary(&config.isolation_root, &config.product_root)?;
    if isolation_root != config.isolation_root || product_root != config.product_root {
        return Err(error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_ISOLATION_IDENTITY_CHANGED",
        ));
    }
    fs::create_dir(&config.isolation_root).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_ISOLATION_ROOT_CREATE_FAILED",
        )
    })?;
    let marker = format!(
        "release={HERMES_RELEASE}\ncommit={HERMES_UPSTREAM_COMMIT}\npolicy=lattice-owned-ephemeral-root\n"
    );
    let mut marker_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(config.isolation_root.join(HOME_MARKER_NAME))
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_ISOLATION_MARKER_FAILED",
            )
        })?;
    marker_file.write_all(marker.as_bytes()).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_ISOLATION_MARKER_FAILED",
        )
    })?;
    for directory in [
        &config.working_directory,
        &config.hermes_home,
        &config.codex_home,
        &config.temp_directory,
    ] {
        fs::create_dir(directory).map_err(|_| {
            error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_ISOLATION_CHILD_CREATE_FAILED",
            )
        })?;
    }
    Ok(())
}

fn validate_prepared_isolation(config: &HermesProcessConfig) -> HermesAdapterResult<()> {
    reject_link_or_reparse_ancestors(&config.isolation_root)?;
    reject_link_or_reparse_ancestors(&config.product_root)?;
    let canonical_root = fs::canonicalize(&config.isolation_root).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_ISOLATION_ROOT_RECHECK_FAILED",
        )
    })?;
    if canonical_root != config.isolation_root
        || path_is_within(&canonical_root, &config.product_root)
        || path_is_within(&config.product_root, &canonical_root)
    {
        return Err(error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_ISOLATION_ROOT_RECHECK_FAILED",
        ));
    }
    for directory in [
        &config.working_directory,
        &config.hermes_home,
        &config.codex_home,
        &config.temp_directory,
    ] {
        reject_link_or_reparse_ancestors(directory)?;
        if directory.parent() != Some(config.isolation_root.as_path())
            || !directory.is_dir()
            || fs::read_dir(directory)
                .map_err(|_| {
                    error(
                        HermesAdapterErrorKind::Spawn,
                        "HERMES_ISOLATION_CHILD_RECHECK_FAILED",
                    )
                })?
                .next()
                .is_some()
        {
            return Err(error(
                HermesAdapterErrorKind::Spawn,
                "HERMES_ISOLATION_CHILD_RECHECK_FAILED",
            ));
        }
    }
    if !config.isolation_root.join(HOME_MARKER_NAME).is_file()
        || config.hermes_home.join(HOME_MARKER_NAME).exists()
        || config.codex_home.join(HOME_MARKER_NAME).exists()
    {
        return Err(error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_ISOLATION_MARKER_RECHECK_FAILED",
        ));
    }
    Ok(())
}

fn terminate_child(child: &mut Child) -> HermesAdapterResult<()> {
    if child
        .try_wait()
        .map_err(|_| {
            error(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CHILD_STATUS_UNKNOWN",
            )
        })?
        .is_some()
    {
        return Ok(());
    }
    child.kill().map_err(|_| {
        error(
            HermesAdapterErrorKind::Ambiguous,
            "HERMES_CHILD_KILL_UNKNOWN",
        )
    })?;
    child.wait().map(|_| ()).map_err(|_| {
        error(
            HermesAdapterErrorKind::Ambiguous,
            "HERMES_CHILD_REAP_UNKNOWN",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_version_output_requires_the_exact_official_first_line() {
        let exact = b"Hermes Agent v0.20.0 (2026.8.3)\nPython: 3.13.5\n";
        validate_pinned_version_output(exact, b"").expect("exact version line");

        let token_only = b"unrelated helper 0.20.0\n";
        let mismatch = validate_pinned_version_output(token_only, b"")
            .expect_err("a version token is not executable identity");
        assert_eq!(mismatch.code(), "HERMES_PACKAGE_VERSION_MISMATCH");

        let stderr = validate_pinned_version_output(exact, b"warning")
            .expect_err("probe diagnostics fail closed");
        assert_eq!(stderr.code(), "HERMES_VERSION_STDERR_REJECTED");
    }

    #[test]
    fn version_probe_runner_terminates_at_its_absolute_bound() {
        #[cfg(windows)]
        let mut command = {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 5",
            ]);
            command
        };
        #[cfg(not(windows))]
        let mut command = {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 5"]);
            command
        };

        let started = Instant::now();
        let failure = bounded_output(&mut command, Duration::from_millis(25))
            .expect_err("long-running identity probe must time out");

        assert_eq!(failure.kind(), HermesAdapterErrorKind::Timeout);
        assert_eq!(failure.code(), "HERMES_VERSION_PROBE_TIMEOUT");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    mod reflection_api {
        include!("../tests/reflection_api.rs");
    }
}
