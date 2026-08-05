//! Typed TASK-032 delivery port backed by the supervised app-server transport.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lattice_contracts::{
    CodexDeliveryEvidence, CodexDeliveryRequest, ContentDigest, DeliveryRuntime, DeliveryStage,
    RequestId,
};
use lattice_ports::{
    DeliveryCodexPort, DeliveryFailureCertainty, DeliveryPortError, DeliveryPortResult,
    PortErrorKind,
};
use sha2::{Digest, Sha256};

use crate::{
    AppServerRunConfig, AppServerRunError, AppServerRunErrorKind, CodexIdentityError,
    CodexIdentityErrorKind, CodexIdentityExpectation, TurnStatus, run_codex_app_server_until,
};

/// LATTICE-owned, fixed configuration for one bounded Codex delivery lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexDeliveryAdapterConfig {
    identity: CodexIdentityExpectation,
    schema_output_dir: PathBuf,
    codex_home: PathBuf,
    prompt: String,
    timeout: Duration,
    runtime: DeliveryRuntime,
}

impl CodexDeliveryAdapterConfig {
    /// Validates fixed composition-time configuration.
    ///
    /// The workspace is intentionally absent: it is accepted only from prior
    /// typed [`lattice_contracts::PreparedWorkspaceEvidence`].
    ///
    /// # Errors
    ///
    /// Returns a known Codex-stage error for malformed fixed configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        identity: CodexIdentityExpectation,
        schema_output_dir: PathBuf,
        codex_home: PathBuf,
        prompt: impl Into<String>,
        timeout: Duration,
        runtime: DeliveryRuntime,
    ) -> DeliveryPortResult<Self> {
        let prompt = prompt.into();
        if !identity.launcher_path().is_absolute() {
            return Err(known_config("CODEX_CONFIG_LAUNCHER_NOT_ABSOLUTE"));
        }
        if identity.version().trim().is_empty() {
            return Err(known_config("CODEX_CONFIG_VERSION_EMPTY"));
        }
        if !is_lowercase_sha256(identity.launcher_sha256()) {
            return Err(known_config("CODEX_CONFIG_LAUNCHER_DIGEST_INVALID"));
        }
        if !schema_output_dir.is_absolute() {
            return Err(known_config("CODEX_CONFIG_SCHEMA_PATH_NOT_ABSOLUTE"));
        }
        if !codex_home.is_absolute() {
            return Err(known_config("CODEX_CONFIG_HOME_NOT_ABSOLUTE"));
        }
        if prompt.trim().is_empty() {
            return Err(known_config("CODEX_CONFIG_PROMPT_EMPTY"));
        }
        if timeout.is_zero() {
            return Err(known_config("CODEX_CONFIG_TIMEOUT_ZERO"));
        }
        Ok(Self {
            identity,
            schema_output_dir,
            codex_home,
            prompt,
            timeout,
            runtime,
        })
    }

    /// Returns the explicitly configured evidence origin.
    #[must_use]
    pub const fn runtime(&self) -> DeliveryRuntime {
        self.runtime
    }
}

/// Concrete typed Codex delivery adapter.
///
/// It performs identity preflight before starting the app-server and never
/// invokes workspace, Git, test, or ledger adapters.
#[derive(Debug)]
pub struct CodexDeliveryAdapter {
    config: CodexDeliveryAdapterConfig,
    deadline: Option<Instant>,
}

impl CodexDeliveryAdapter {
    /// Binds one adapter to immutable composition-time configuration.
    #[must_use]
    pub const fn new(config: CodexDeliveryAdapterConfig) -> Self {
        Self {
            config,
            deadline: None,
        }
    }

    /// Binds this adapter to the composition-owned absolute delivery deadline.
    ///
    /// Unlike [`Self::new`], this constructor never restarts the timeout when
    /// the Codex stage begins after earlier delivery effects.
    #[must_use]
    pub const fn with_deadline(config: CodexDeliveryAdapterConfig, deadline: Instant) -> Self {
        Self {
            config,
            deadline: Some(deadline),
        }
    }

    /// Returns the immutable configuration used by this adapter.
    #[must_use]
    pub const fn config(&self) -> &CodexDeliveryAdapterConfig {
        &self.config
    }
}

impl DeliveryCodexPort for CodexDeliveryAdapter {
    fn run_delivery(
        &mut self,
        request: CodexDeliveryRequest,
    ) -> DeliveryPortResult<CodexDeliveryEvidence> {
        let deadline = match self.deadline {
            Some(deadline) => deadline,
            None => Instant::now()
                .checked_add(self.config.timeout)
                .ok_or_else(|| known_config("CODEX_CONFIG_TIMEOUT_OVERFLOW"))?,
        };
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| known(PortErrorKind::Timeout, "CODEX_DELIVERY_DEADLINE_EXPIRED"))?;
        if remaining > self.config.timeout {
            return Err(known_config("CODEX_DELIVERY_DEADLINE_INVALID"));
        }
        let identity = self
            .config
            .identity
            .preflight_with_deadline(
                self.config.identity.launcher_path(),
                &self.config.schema_output_dir,
                deadline,
            )
            .map_err(map_identity_error)?;

        let launcher_sha256 = ContentDigest::from_sha256(identity.launcher_sha256().to_owned())
            .map_err(|_| known_config("CODEX_IDENTITY_LAUNCHER_DIGEST_INVALID"))?;
        let schema_bundle_sha256 =
            ContentDigest::from_sha256(identity.schema_bundle_sha256().to_owned())
                .map_err(|_| known_config("CODEX_IDENTITY_SCHEMA_DIGEST_INVALID"))?;
        let schema_file_count = u32::try_from(identity.schema_file_count())
            .map_err(|_| known_config("CODEX_IDENTITY_SCHEMA_COUNT_OVERFLOW"))?;
        let launcher_locator = path_text(identity.launcher_path())
            .ok_or_else(|| known_config("CODEX_IDENTITY_LAUNCHER_PATH_INVALID"))?
            .to_owned();
        let workspace = PathBuf::from(request.workspace().workspace_locator());
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| known(PortErrorKind::Timeout, "CODEX_DELIVERY_DEADLINE_EXPIRED"))?;
        let process_config = AppServerRunConfig::new(
            identity.launcher_path().to_path_buf(),
            identity.launcher_sha256(),
            self.config.codex_home.clone(),
            workspace,
            self.config.prompt.clone(),
            remaining,
        )
        .map_err(map_process_error)?;

        let run =
            run_codex_app_server_until(&process_config, deadline).map_err(map_process_error)?;
        match run.outcome().status {
            TurnStatus::Completed => {}
            TurnStatus::Failed => {
                return Err(ambiguous(
                    PortErrorKind::Unavailable,
                    "CODEX_APP_SERVER_TURN_FAILED",
                ));
            }
            TurnStatus::Interrupted => {
                return Err(ambiguous(
                    PortErrorKind::Cancelled,
                    "CODEX_APP_SERVER_TURN_INTERRUPTED",
                ));
            }
        }

        let output_digest = delivery_output_digest(
            &request,
            self.config.runtime,
            &identity,
            run.thread_id(),
            run.turn_id(),
        );
        CodexDeliveryEvidence::new(
            &request,
            self.config.runtime,
            launcher_locator,
            identity.version(),
            launcher_sha256,
            schema_bundle_sha256,
            schema_file_count,
            run.thread_id(),
            run.turn_id(),
            output_digest,
        )
        .map_err(|_| ambiguous(PortErrorKind::Malformed, "CODEX_DELIVERY_EVIDENCE_INVALID"))
    }

    fn interrupt_delivery(&mut self, _request_id: &RequestId) -> DeliveryPortResult<()> {
        // `run_delivery` owns a synchronous process lifetime, so no active run
        // can be mutably interrupted through this instance at the same time.
        Err(known(PortErrorKind::Denied, "CODEX_DELIVERY_NOT_ACTIVE"))
    }
}

fn delivery_output_digest(
    request: &CodexDeliveryRequest,
    runtime: DeliveryRuntime,
    identity: &crate::CodexIdentityEvidence,
    thread_id: &str,
    turn_id: &str,
) -> ContentDigest {
    let mut hasher = Sha256::new();
    hasher.update(b"lattice.codex-delivery.output-evidence.v1\0");
    for field in [
        runtime_name(runtime),
        request.request().invocation().request_id().as_str(),
        request.request().configuration_digest().as_str(),
        request.intent().intent_digest().as_str(),
        request.workspace().evidence_digest().as_str(),
        identity.version(),
        identity.launcher_sha256(),
        identity.schema_bundle_sha256(),
        thread_id,
        turn_id,
    ] {
        hash_field(&mut hasher, field.as_bytes());
    }
    hash_field(
        &mut hasher,
        &u64::try_from(identity.schema_file_count())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    ContentDigest::from_sha256(hex_digest(hasher.finalize().as_ref()))
        .expect("SHA-256 output is always a valid content digest")
}

fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(bytes);
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

const fn runtime_name(runtime: DeliveryRuntime) -> &'static str {
    match runtime {
        DeliveryRuntime::ScriptedAcceptance => "SCRIPTED_ACCEPTANCE",
        DeliveryRuntime::OfficialCodexAppServer => "OFFICIAL_CODEX_APP_SERVER",
    }
}

fn path_text(path: &Path) -> Option<&str> {
    path.to_str().filter(|value| !value.trim().is_empty())
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn map_identity_error(error: CodexIdentityError) -> DeliveryPortError {
    let kind = match error.kind() {
        CodexIdentityErrorKind::VersionMismatch => PortErrorKind::VersionMismatch,
        CodexIdentityErrorKind::LauncherPathMismatch
        | CodexIdentityErrorKind::LauncherDigestMismatch
        | CodexIdentityErrorKind::SchemaBundleInvalid
        | CodexIdentityErrorKind::SchemaBundleEmpty => PortErrorKind::CapabilityMismatch,
        CodexIdentityErrorKind::VersionOutputInvalid => PortErrorKind::Malformed,
        CodexIdentityErrorKind::LauncherNotFile
        | CodexIdentityErrorKind::LauncherReadFailed
        | CodexIdentityErrorKind::LauncherChanged
        | CodexIdentityErrorKind::VersionCommandFailed
        | CodexIdentityErrorKind::SchemaOutputExists
        | CodexIdentityErrorKind::SchemaGenerationFailed
        | CodexIdentityErrorKind::SchemaReadFailed
        | CodexIdentityErrorKind::Timeout
        | CodexIdentityErrorKind::ProcessContainmentFailed => PortErrorKind::Unavailable,
    };
    known(kind, error.to_string())
}

fn map_process_error(error: AppServerRunError) -> DeliveryPortError {
    let (kind, certainty) = match error.kind() {
        AppServerRunErrorKind::InvalidLauncher
        | AppServerRunErrorKind::InvalidLauncherSha256
        | AppServerRunErrorKind::InvalidCodexHome
        | AppServerRunErrorKind::CodexHomeOwnershipMissing
        | AppServerRunErrorKind::CodexHomeOverlap
        | AppServerRunErrorKind::AmbientCodexHomeDenied
        | AppServerRunErrorKind::InvalidWorkingDirectory
        | AppServerRunErrorKind::InvalidPrompt
        | AppServerRunErrorKind::InvalidTimeout
        | AppServerRunErrorKind::LauncherReadFailed
        | AppServerRunErrorKind::LauncherDigestMismatch
        | AppServerRunErrorKind::SpawnFailed => (
            process_error_kind(error.kind()),
            DeliveryFailureCertainty::Known,
        ),
        AppServerRunErrorKind::LauncherChanged
        | AppServerRunErrorKind::PipeUnavailable
        | AppServerRunErrorKind::WriteFailed
        | AppServerRunErrorKind::StdoutFailed
        | AppServerRunErrorKind::StdoutLineTooLarge
        | AppServerRunErrorKind::ProtocolFailed
        | AppServerRunErrorKind::CodexHomeMismatch
        | AppServerRunErrorKind::Timeout
        | AppServerRunErrorKind::AmbiguousEof
        | AppServerRunErrorKind::ChildCleanupFailed
        | AppServerRunErrorKind::JobObjectFailed => (
            process_error_kind(error.kind()),
            DeliveryFailureCertainty::Ambiguous,
        ),
    };
    DeliveryPortError::new(
        DeliveryStage::Codex,
        kind,
        certainty,
        process_error_code(error.kind()),
    )
}

const fn process_error_kind(kind: AppServerRunErrorKind) -> PortErrorKind {
    match kind {
        AppServerRunErrorKind::InvalidLauncher
        | AppServerRunErrorKind::InvalidLauncherSha256
        | AppServerRunErrorKind::InvalidCodexHome
        | AppServerRunErrorKind::InvalidWorkingDirectory
        | AppServerRunErrorKind::InvalidPrompt
        | AppServerRunErrorKind::InvalidTimeout
        | AppServerRunErrorKind::ProtocolFailed
        | AppServerRunErrorKind::StdoutLineTooLarge
        | AppServerRunErrorKind::CodexHomeMismatch => PortErrorKind::Malformed,
        AppServerRunErrorKind::CodexHomeOwnershipMissing
        | AppServerRunErrorKind::CodexHomeOverlap
        | AppServerRunErrorKind::AmbientCodexHomeDenied => PortErrorKind::Denied,
        AppServerRunErrorKind::Timeout => PortErrorKind::Timeout,
        AppServerRunErrorKind::AmbiguousEof
        | AppServerRunErrorKind::ChildCleanupFailed
        | AppServerRunErrorKind::JobObjectFailed
        | AppServerRunErrorKind::LauncherChanged => PortErrorKind::Ambiguous,
        AppServerRunErrorKind::LauncherReadFailed
        | AppServerRunErrorKind::LauncherDigestMismatch
        | AppServerRunErrorKind::SpawnFailed
        | AppServerRunErrorKind::PipeUnavailable
        | AppServerRunErrorKind::WriteFailed
        | AppServerRunErrorKind::StdoutFailed => PortErrorKind::Unavailable,
    }
}

const fn process_error_code(kind: AppServerRunErrorKind) -> &'static str {
    match kind {
        AppServerRunErrorKind::InvalidLauncher => "CODEX_APP_SERVER_INVALID_LAUNCHER",
        AppServerRunErrorKind::InvalidLauncherSha256 => "CODEX_APP_SERVER_INVALID_LAUNCHER_SHA256",
        AppServerRunErrorKind::InvalidCodexHome => "CODEX_APP_SERVER_INVALID_CODEX_HOME",
        AppServerRunErrorKind::CodexHomeOwnershipMissing => {
            "CODEX_APP_SERVER_CODEX_HOME_OWNERSHIP_MISSING"
        }
        AppServerRunErrorKind::CodexHomeOverlap => "CODEX_APP_SERVER_CODEX_HOME_OVERLAP",
        AppServerRunErrorKind::AmbientCodexHomeDenied => {
            "CODEX_APP_SERVER_AMBIENT_CODEX_HOME_DENIED"
        }
        AppServerRunErrorKind::InvalidWorkingDirectory => {
            "CODEX_APP_SERVER_INVALID_WORKING_DIRECTORY"
        }
        AppServerRunErrorKind::InvalidPrompt => "CODEX_APP_SERVER_INVALID_PROMPT",
        AppServerRunErrorKind::InvalidTimeout => "CODEX_APP_SERVER_INVALID_TIMEOUT",
        AppServerRunErrorKind::LauncherReadFailed => "CODEX_APP_SERVER_LAUNCHER_READ_FAILED",
        AppServerRunErrorKind::LauncherDigestMismatch => {
            "CODEX_APP_SERVER_LAUNCHER_DIGEST_MISMATCH"
        }
        AppServerRunErrorKind::LauncherChanged => "CODEX_APP_SERVER_LAUNCHER_CHANGED",
        AppServerRunErrorKind::SpawnFailed => "CODEX_APP_SERVER_SPAWN_FAILED",
        AppServerRunErrorKind::PipeUnavailable => "CODEX_APP_SERVER_PIPE_UNAVAILABLE",
        AppServerRunErrorKind::WriteFailed => "CODEX_APP_SERVER_WRITE_FAILED",
        AppServerRunErrorKind::StdoutFailed => "CODEX_APP_SERVER_STDOUT_FAILED",
        AppServerRunErrorKind::StdoutLineTooLarge => "CODEX_APP_SERVER_STDOUT_LINE_TOO_LARGE",
        AppServerRunErrorKind::ProtocolFailed => "CODEX_APP_SERVER_PROTOCOL_FAILED",
        AppServerRunErrorKind::CodexHomeMismatch => "CODEX_APP_SERVER_CODEX_HOME_MISMATCH",
        AppServerRunErrorKind::Timeout => "CODEX_APP_SERVER_TIMEOUT",
        AppServerRunErrorKind::AmbiguousEof => "CODEX_APP_SERVER_AMBIGUOUS_EOF",
        AppServerRunErrorKind::ChildCleanupFailed => "CODEX_APP_SERVER_CHILD_CLEANUP_FAILED",
        AppServerRunErrorKind::JobObjectFailed => "CODEX_APP_SERVER_JOB_OBJECT_FAILED",
    }
}

fn known_config(code: &'static str) -> DeliveryPortError {
    known(PortErrorKind::Malformed, code)
}

fn known(kind: PortErrorKind, code: impl Into<String>) -> DeliveryPortError {
    DeliveryPortError::new(
        DeliveryStage::Codex,
        kind,
        DeliveryFailureCertainty::Known,
        code,
    )
}

fn ambiguous(kind: PortErrorKind, code: impl Into<String>) -> DeliveryPortError {
    DeliveryPortError::new(
        DeliveryStage::Codex,
        kind,
        DeliveryFailureCertainty::Ambiguous,
        code,
    )
}
