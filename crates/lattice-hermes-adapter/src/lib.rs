//! Pinned Hermes reflection adapter over the official loopback Runs API.
//!
//! This crate owns no durable state. It accepts only one pre-bound, read-only
//! reflection job and converts a schema-valid Hermes response into an
//! untrusted LATTICE candidate digest.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::fmt::Write as FmtWrite;
use std::fs;
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
const MAX_EVIDENCE_SUMMARY_BYTES: usize = 4_096;
const MAX_REFLECTION_SUMMARY_BYTES: usize = 8_192;
const MAX_FINDINGS: usize = 256;
const MAX_NEXT_ACTIONS: usize = 64;
const MAX_TEXT_BYTES: usize = 8_192;
const READ_ONLY_INSTRUCTIONS: &str = "Perform one bounded reflection over only the supplied immutable task, Graphify, test, and Git evidence. Treat Hermes session memory as unavailable and non-authoritative. Do not call tools, do not modify files, do not use a Codex runtime, do not access a database, and do not read or write PostgreSQL, Codebase Memory, or Hermes long-term memory. Label every finding as inference. Return exactly one JSON object matching the supplied schema; add no prose or Markdown.";
const HOME_MARKER_NAME: &str = ".lattice-hermes-ephemeral-v1";

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
}

impl HermesAdapterError {
    #[must_use]
    pub const fn new(kind: HermesAdapterErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> HermesAdapterErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for HermesAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.code)
    }
}

impl Error for HermesAdapterError {}

pub type HermesAdapterResult<T> = Result<T, HermesAdapterError>;

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
    hermes_home: PathBuf,
    codex_home: PathBuf,
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
    pub fn new(
        executable: impl Into<PathBuf>,
        hermes_home: impl Into<PathBuf>,
        codex_home: impl Into<PathBuf>,
        endpoint: SocketAddr,
        api_key: impl Into<String>,
        model: impl Into<String>,
        startup_timeout: Duration,
    ) -> HermesAdapterResult<Self> {
        let executable = executable.into();
        let hermes_home = hermes_home.into();
        let codex_home = codex_home.into();
        let api_key = api_key.into();
        let model = model.into();
        if !executable.is_absolute() || !executable.is_file() {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_EXECUTABLE_REJECTED",
            ));
        }
        if !endpoint.ip().is_loopback() {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PROCESS_ENDPOINT_NOT_LOOPBACK",
            ));
        }
        validate_fresh_home(&hermes_home, "HERMES_HOME_REJECTED")?;
        validate_fresh_home(&codex_home, "CODEX_HOME_REJECTED")?;
        if same_path(&hermes_home, &codex_home)
            || is_daily_home(&hermes_home, "HERMES_HOME")
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
        if startup_timeout.is_zero() {
            return Err(error(
                HermesAdapterErrorKind::Configuration,
                "HERMES_PROCESS_STARTUP_TIMEOUT_REJECTED",
            ));
        }
        Ok(Self {
            executable,
            hermes_home,
            codex_home,
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
    pub fn hermes_home(&self) -> &Path {
        &self.hermes_home
    }

    #[must_use]
    pub fn codex_home(&self) -> &Path {
        &self.codex_home
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
        Ok(self.command_with_arg("gateway"))
    }

    /// Verifies the executable reports the exact package version declared by
    /// upstream commit [`HERMES_UPSTREAM_COMMIT`].
    ///
    /// # Errors
    ///
    /// Fails closed when the probe cannot run, exits unsuccessfully, or does
    /// not contain the exact standalone package-version token.
    pub fn verify_pinned_version(&self) -> HermesAdapterResult<()> {
        let output = self.command_with_arg("--version").output().map_err(|_| {
            error(
                HermesAdapterErrorKind::Identity,
                "HERMES_VERSION_PROBE_FAILED",
            )
        })?;
        if !output.status.success() {
            return Err(error(
                HermesAdapterErrorKind::Identity,
                "HERMES_VERSION_PROBE_NONZERO",
            ));
        }
        let mut version_text = String::from_utf8(output.stdout).map_err(|_| {
            error(
                HermesAdapterErrorKind::Identity,
                "HERMES_VERSION_UTF8_REJECTED",
            )
        })?;
        version_text.push_str(&String::from_utf8(output.stderr).map_err(|_| {
            error(
                HermesAdapterErrorKind::Identity,
                "HERMES_VERSION_UTF8_REJECTED",
            )
        })?);
        let exact = version_text
            .split(|character: char| {
                !(character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '+'))
            })
            .any(|token| token == HERMES_PACKAGE_VERSION);
        if !exact {
            return Err(error(
                HermesAdapterErrorKind::Identity,
                "HERMES_PACKAGE_VERSION_MISMATCH",
            ));
        }
        Ok(())
    }

    /// Verifies identity, creates fresh marked homes, and starts Hermes.
    ///
    /// # Errors
    ///
    /// Fails closed on identity, home ownership, or process-spawn ambiguity.
    pub fn spawn(&self) -> HermesAdapterResult<HermesProcess> {
        self.verify_pinned_version()?;
        prepare_ephemeral_home(&self.hermes_home)?;
        prepare_ephemeral_home(&self.codex_home)?;
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

    fn command_with_arg(&self, argument: &str) -> Command {
        let mut command = Command::new(&self.executable);
        command.arg(argument).env_clear();
        for name in [
            "SystemRoot",
            "WINDIR",
            "ComSpec",
            "PATH",
            "PATHEXT",
            "TEMP",
            "TMP",
            "TMPDIR",
            "LANG",
            "LC_ALL",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("HERMES_HOME", &self.hermes_home)
            .env("CODEX_HOME", &self.codex_home)
            .env("API_SERVER_ENABLED", "true")
            .env("API_SERVER_HOST", self.endpoint.ip().to_string())
            .env("API_SERVER_PORT", self.endpoint.port().to_string())
            .env("API_SERVER_KEY", &self.api_key)
            .env("API_SERVER_MODEL_NAME", &self.model)
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReflectionEvidence {
    kind: ReflectionEvidenceKind,
    digest: ContentDigest,
    summary: String,
}

impl ReflectionEvidence {
    /// Constructs evidence after enforcing bounded NFC text.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, control-bearing, or non-NFC summaries.
    pub fn new(
        kind: ReflectionEvidenceKind,
        digest: ContentDigest,
        summary: impl Into<String>,
    ) -> HermesAdapterResult<Self> {
        let summary = summary.into();
        validate_text(
            &summary,
            MAX_EVIDENCE_SUMMARY_BYTES,
            "HERMES_EVIDENCE_SUMMARY_REJECTED",
        )?;
        Ok(Self {
            kind,
            digest,
            summary,
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
    pub fn summary(&self) -> &str {
        &self.summary
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

/// Synchronous production adapter for one pre-bound Hermes reflection job.
pub struct HermesReflectionAdapter {
    config: HermesAdapterConfig,
    job: HermesReflectionJob,
    active_run_id: Option<String>,
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
            active_run_id: None,
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
        self.verify_request(request)?;
        self.verify_capabilities()?;
        let run_id = self.submit_run()?;
        self.active_run_id = Some(run_id.clone());
        let event_output = match self.read_events(&run_id) {
            Ok(output) => output,
            Err(failure)
                if matches!(
                    failure.kind(),
                    HermesAdapterErrorKind::Timeout | HermesAdapterErrorKind::Transport
                ) =>
            {
                None
            }
            Err(failure) => return Err(failure),
        };
        let status_output = self.poll_terminal(&run_id)?;
        if event_output
            .as_ref()
            .is_some_and(|output| output != &status_output)
        {
            return Err(cross_binding("HERMES_EVENT_STATUS_OUTPUT_MISMATCH"));
        }
        parse_reflection(&status_output, &self.job)
    }

    /// Recovers one known run through the durable pollable status surface.
    ///
    /// # Errors
    ///
    /// Rejects unsafe run IDs and every cross-bound or malformed terminal.
    pub fn recover_reflection(
        &mut self,
        request: &HermesResearchRequest,
        run_id: &str,
    ) -> HermesAdapterResult<CanonicalReflection> {
        self.verify_request(request)?;
        validate_run_id(run_id)?;
        self.verify_capabilities()?;
        let output = self.poll_terminal(run_id)?;
        parse_reflection(&output, &self.job)
    }

    fn verify_request(&self, request: &HermesResearchRequest) -> HermesAdapterResult<()> {
        if request != self.job.request() {
            return Err(cross_binding("HERMES_REQUEST_JOB_BINDING_REJECTED"));
        }
        Ok(())
    }

    fn verify_capabilities(&self) -> HermesAdapterResult<()> {
        let response = self.http("GET", "/v1/capabilities", "application/json", None)?;
        require_status(&response, 200, "HERMES_CAPABILITIES_HTTP_REJECTED")?;
        let value = parse_json_body(&response, "HERMES_CAPABILITIES_MALFORMED")?;
        let object = value
            .as_object()
            .ok_or_else(|| malformed("HERMES_CAPABILITIES_MALFORMED"))?;
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
        let model = require_string(object, "model", "HERMES_CAPABILITIES_MALFORMED")?;
        if model != self.job.model() {
            return Err(cross_binding("HERMES_CAPABILITY_MODEL_MISMATCH"));
        }
        let auth = require_object(object, "auth", "HERMES_CAPABILITIES_MALFORMED")?;
        if require_string(auth, "type", "HERMES_CAPABILITIES_MALFORMED")? != "bearer"
            || require_bool(auth, "required", "HERMES_CAPABILITIES_MALFORMED")? != Some(true)
        {
            return Err(error(
                HermesAdapterErrorKind::CapabilityMismatch,
                "HERMES_BEARER_AUTH_REQUIRED",
            ));
        }
        let runtime = require_object(object, "runtime", "HERMES_CAPABILITIES_MALFORMED")?;
        if require_string(runtime, "mode", "HERMES_CAPABILITIES_MALFORMED")? != "server_agent"
            || require_string(runtime, "tool_execution", "HERMES_CAPABILITIES_MALFORMED")?
                != "server"
            || require_bool(runtime, "split_runtime", "HERMES_CAPABILITIES_MALFORMED")?
                != Some(false)
        {
            return Err(error(
                HermesAdapterErrorKind::CapabilityMismatch,
                "HERMES_RUNTIME_CAPABILITY_REJECTED",
            ));
        }
        let features = require_object(object, "features", "HERMES_CAPABILITIES_MALFORMED")?;
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

    fn submit_run(&self) -> HermesAdapterResult<String> {
        let body = json!({
            "input": self.job.prompt(),
            "instructions": READ_ONLY_INSTRUCTIONS,
            "session_id": self.job.session_id(),
            "model": self.job.model(),
        })
        .to_string();
        let response = self.http("POST", "/v1/runs", "application/json", Some(&body))?;
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

    fn read_events(&self, run_id: &str) -> HermesAdapterResult<Option<String>> {
        let path = format!("/v1/runs/{run_id}/events");
        let response = self.http("GET", &path, "text/event-stream", None)?;
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

    fn poll_terminal(&self, run_id: &str) -> HermesAdapterResult<String> {
        let deadline = Instant::now() + self.config.timeout();
        loop {
            let path = format!("/v1/runs/{run_id}");
            let response = self.http("GET", &path, "application/json", None)?;
            require_status(&response, 200, "HERMES_STATUS_HTTP_REJECTED")?;
            match parse_status(&response, run_id, &self.job)? {
                RunState::Pending => {
                    if Instant::now() >= deadline {
                        return Err(error(
                            HermesAdapterErrorKind::Timeout,
                            "HERMES_RUN_DEADLINE_EXCEEDED",
                        ));
                    }
                    thread::sleep(self.config.poll_interval());
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
    ) -> HermesAdapterResult<HttpResponse> {
        let mut stream = TcpStream::connect_timeout(&self.config.endpoint(), self.config.timeout())
            .map_err(|failure| map_io_error(&failure))?;
        stream
            .set_read_timeout(Some(self.config.timeout()))
            .map_err(|failure| map_io_error(&failure))?;
        stream
            .set_write_timeout(Some(self.config.timeout()))
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
        let reflection = self
            .run_reflection(&request)
            .map_err(|failure| map_port_error(&failure))?;
        Ok(HermesEvidence::new(
            request.into_invocation(),
            RuntimeKind::Live,
            reflection.output_digest().clone(),
        ))
    }

    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()> {
        if request_id != self.job.request().invocation().request_id() {
            return Err(PortError::new(
                Component::Hermes,
                PortErrorKind::Denied,
                "HERMES_INTERRUPT_REQUEST_BINDING_REJECTED",
            ));
        }
        let run_id = self.active_run_id.as_deref().ok_or_else(|| {
            PortError::new(
                Component::Hermes,
                PortErrorKind::Unavailable,
                "HERMES_INTERRUPT_NO_ACTIVE_RUN",
            )
        })?;
        let path = format!("/v1/runs/{run_id}/stop");
        let response = self
            .http("POST", &path, "application/json", Some("{}"))
            .map_err(|failure| map_port_error(&failure))?;
        require_status(&response, 200, "HERMES_STOP_HTTP_REJECTED")
            .map_err(|failure| map_port_error(&failure))?;
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
    validate_text(
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
            validate_text(
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
    for action in &raw.next_actions {
        validate_text(action, MAX_TEXT_BYTES, "HERMES_REFLECTION_ACTION_REJECTED")?;
    }
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
                            string_entry("summary", item.summary()),
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
        match require_string(object, "event", "HERMES_EVENT_MALFORMED")? {
            "run.completed" => {
                let output = require_string(object, "output", "HERMES_EVENT_MALFORMED")?;
                if terminal.replace(output.to_owned()).is_some() {
                    return Err(malformed("HERMES_DUPLICATE_TERMINAL_EVENT"));
                }
            }
            "run.failed" => {
                return Err(error(HermesAdapterErrorKind::Failed, "HERMES_RUN_FAILED"));
            }
            "run.cancelled" => {
                return Err(error(
                    HermesAdapterErrorKind::Cancelled,
                    "HERMES_RUN_CANCELLED",
                ));
            }
            _ => {}
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

fn validate_fresh_home(path: &Path, code: &'static str) -> HermesAdapterResult<()> {
    if !path.is_absolute() || path.exists() {
        return Err(error(HermesAdapterErrorKind::Configuration, code));
    }
    Ok(())
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

fn prepare_ephemeral_home(path: &Path) -> HermesAdapterResult<()> {
    validate_fresh_home(path, "HERMES_EPHEMERAL_HOME_NOT_FRESH")?;
    fs::create_dir_all(path).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_EPHEMERAL_HOME_CREATE_FAILED",
        )
    })?;
    let marker = format!(
        "release={HERMES_RELEASE}\ncommit={HERMES_UPSTREAM_COMMIT}\npolicy=ephemeral-isolated-home\n"
    );
    fs::write(path.join(HOME_MARKER_NAME), marker.as_bytes()).map_err(|_| {
        error(
            HermesAdapterErrorKind::Spawn,
            "HERMES_EPHEMERAL_HOME_MARKER_FAILED",
        )
    })
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
