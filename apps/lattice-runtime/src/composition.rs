//! Sole concrete composition root for the bounded TASK-032 delivery lane.

use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_codex_adapter::{
    CodexDeliveryAdapter, CodexDeliveryAdapterConfig, CodexIdentityExpectation,
};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CompletedDeliveryEvidence, ContentDigest, DeliveryProfile,
    DeliveryReceipt, DeliveryRunRequest, DeliveryRuntime, DeliveryStage, DeliveryTerminalStatus,
    Invocation, ProjectSnapshotId, RequestId, TaskId,
};
use lattice_orchestrator::{DeliveryOrchestratorError, delivery_status, run_delivery};
use lattice_ports::{DeliveryFailureCertainty, PortErrorKind};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::DELIVERY_PROMPT;
use crate::delivery_ledger::{
    DeliveryDatabaseBinding, DeliveryLedger, DeliveryReceipt as LegacyDeliveryReceipt,
    LEGACY_RECEIPT_FORMAT, PostgresDeliveryLedgerAdapter, PostgresDeliveryStatusReplay,
};
use crate::git_delivery::{DeliveryWorkspaceGitAdapter, DeliveryWorkspaceGitAdapterConfig};
use crate::mcp::{self, DeliveryToolService, ToolExecutionError};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_TIMEOUT_SECONDS: u64 = 3_600;
const TASK_ID: &str = "TASK-032";
const PROJECT_SNAPSHOT_ID: &str = "task032-delivery:snapshot:1";
const SCRIPTED_FIXTURE_MARKER_NAME: &str = ".lattice-delivery-fixture-v1.json";
const SCRIPTED_FIXTURE_KIND: &str = "LATTICE_DELIVERY_SCRIPTED_ACCEPTANCE_V1";
const MAX_SCRIPTED_MARKER_BYTES: u64 = 4 * 1024;
const MAX_SCRIPTED_LAUNCHER_BYTES: u64 = 64 * 1024;
const MAX_SCRIPTED_SERVER_BYTES: u64 = 64 * 1024;
const SCRIPTED_SERVER_BYTES: &[u8] = include_bytes!("fixtures/task032-scripted-codex.ps1");

/// Static, secret-free composition failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LatticedErrorKind {
    Configuration,
    DatabaseSecret,
    DatabaseConnect,
    LedgerConfiguration,
    WorkspaceConfiguration,
    CodexConfiguration,
    Contract,
    Intent,
    OutcomePersistence,
    ReceiptRead,
    ReceiptMismatch,
    DeliveryFailed,
    ReconciliationRequired,
    OfficialLiveBlocked,
    ScriptedFixtureRejected,
    Transport,
}

impl LatticedErrorKind {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Configuration => "LATTICED_CONFIGURATION_REJECTED",
            Self::DatabaseSecret => "LATTICED_DATABASE_SECRET_MISSING",
            Self::DatabaseConnect => "LATTICED_DATABASE_CONNECT_REJECTED",
            Self::LedgerConfiguration => "LATTICED_LEDGER_CONFIGURATION_REJECTED",
            Self::WorkspaceConfiguration => "LATTICED_WORKSPACE_CONFIGURATION_REJECTED",
            Self::CodexConfiguration => "LATTICED_CODEX_CONFIGURATION_REJECTED",
            Self::Contract => "LATTICE_DELIVERY_CONTRACT_REJECTED",
            Self::Intent => "LATTICE_DELIVERY_INTENT_REJECTED",
            Self::OutcomePersistence => "LATTICE_DELIVERY_OUTCOME_PERSIST_REJECTED",
            Self::ReceiptRead => "LATTICE_DELIVERY_RECEIPT_REJECTED",
            Self::ReceiptMismatch => "LATTICE_DELIVERY_RECEIPT_MISMATCH",
            Self::DeliveryFailed => "LATTICE_DELIVERY_FAILED",
            Self::ReconciliationRequired => "LATTICE_DELIVERY_RECONCILIATION_REQUIRED",
            Self::OfficialLiveBlocked => "LATTICE_OFFICIAL_CODEX_FAILED_DIAGNOSTIC",
            Self::ScriptedFixtureRejected => "LATTICE_SCRIPTED_FIXTURE_REJECTED",
            Self::Transport => "LATTICED_STDIO_REJECTED",
        }
    }
}

/// Bounded composition failure safe for CLI/MCP diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatticedError {
    kind: LatticedErrorKind,
}

impl LatticedError {
    const fn new(kind: LatticedErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> LatticedErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }
}

impl fmt::Display for LatticedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for LatticedError {}

/// Fixed process-owned inputs for one executable delivery profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LatticedDeliveryConfig {
    launcher: PathBuf,
    version: String,
    launcher_sha256: String,
    schema_directory: PathBuf,
    codex_home: PathBuf,
    delivery_root: PathBuf,
    git_executable: PathBuf,
    timeout: Duration,
    runtime: DeliveryRuntime,
}

impl LatticedDeliveryConfig {
    /// Validates the process-owned configuration before any effect is attempted.
    ///
    /// # Errors
    ///
    /// Returns a static configuration failure for a malformed path, digest,
    /// timeout, prompt binding, or runtime identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        launcher: PathBuf,
        version: impl Into<String>,
        launcher_sha256: impl Into<String>,
        schema_directory: PathBuf,
        codex_home: PathBuf,
        delivery_root: PathBuf,
        git_executable: PathBuf,
        timeout: Duration,
        runtime: DeliveryRuntime,
    ) -> Result<Self, LatticedError> {
        if timeout.is_zero() || timeout > Duration::from_secs(MAX_TIMEOUT_SECONDS) {
            return Err(LatticedError::new(LatticedErrorKind::Configuration));
        }
        let version = version.into();
        let launcher_sha256 = launcher_sha256.into();
        let identity = CodexIdentityExpectation::new(
            launcher.clone(),
            version.clone(),
            launcher_sha256.clone(),
        );
        CodexDeliveryAdapterConfig::new(
            identity,
            schema_directory.clone(),
            codex_home.clone(),
            DELIVERY_PROMPT,
            timeout,
            runtime,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::CodexConfiguration))?;
        DeliveryWorkspaceGitAdapterConfig::new(
            delivery_root.clone(),
            git_executable.clone(),
            timeout,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::WorkspaceConfiguration))?;
        Ok(Self {
            launcher,
            version,
            launcher_sha256,
            schema_directory,
            codex_home,
            delivery_root,
            git_executable,
            timeout,
            runtime,
        })
    }
}

/// One shared service used by the canonical `latticed` MCP process and the
/// `lattice-runtime` compatibility command.
pub struct LatticedDeliveryService {
    request: Option<DeliveryRunRequest>,
    database: DeliveryDatabaseBinding,
    password: String,
    timeout: Duration,
    delivery: Option<LatticedDeliveryConfig>,
}

impl LatticedDeliveryService {
    /// Creates the executable service for a fixed process-owned delivery lane.
    ///
    /// # Errors
    ///
    /// Rejects an empty database secret or malformed deterministic binding.
    pub fn for_delivery(
        config: LatticedDeliveryConfig,
        database: DeliveryDatabaseBinding,
        password: String,
    ) -> Result<Self, LatticedError> {
        let request = request_for_delivery(database.run_id(), &config)?;
        validate_secret_and_timeout(&password, config.timeout)?;
        Ok(Self {
            request: Some(request),
            database,
            password,
            timeout: config.timeout,
            delivery: Some(config),
        })
    }

    /// Creates the restart-safe read-only service without run-only paths.
    ///
    /// # Errors
    ///
    /// Rejects an empty database secret, invalid timeout, or malformed fixed
    /// request binding.
    pub fn status_only(
        database: DeliveryDatabaseBinding,
        password: String,
        timeout: Duration,
    ) -> Result<Self, LatticedError> {
        validate_secret_and_timeout(&password, timeout)?;
        invocation_for_run(database.run_id())?;
        Ok(Self {
            request: None,
            database,
            password,
            timeout,
            delivery: None,
        })
    }

    /// Loads the full fixed service configuration from process environment.
    /// MCP callers cannot influence any of these values.
    ///
    /// # Errors
    ///
    /// Fails closed for every missing or malformed process input.
    pub fn from_environment() -> Result<Self, LatticedError> {
        let timeout = match env::var("LATTICE_DELIVERY_TIMEOUT_SECONDS") {
            Ok(value) => parse_timeout(&value)?,
            Err(env::VarError::NotPresent) => Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
            Err(env::VarError::NotUnicode(_)) => {
                return Err(LatticedError::new(LatticedErrorKind::Configuration));
            }
        };
        let runtime = match required_environment("LATTICE_DELIVERY_CODEX_MODE")?.as_str() {
            "SCRIPTED_ACCEPTANCE" => DeliveryRuntime::ScriptedAcceptance,
            "OFFICIAL_CODEX_APP_SERVER" => DeliveryRuntime::OfficialCodexAppServer,
            _ => return Err(LatticedError::new(LatticedErrorKind::Configuration)),
        };
        let port = required_environment("LATTICE_TASK019_PORT")?
            .parse::<u16>()
            .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
        let database = DeliveryDatabaseBinding::new(
            required_environment("LATTICE_TASK019_HOST")?,
            port,
            required_environment("LATTICE_TASK019_RUN_ID")?,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
        let config = LatticedDeliveryConfig::new(
            PathBuf::from(required_environment("LATTICE_DELIVERY_LAUNCHER")?),
            required_environment("LATTICE_DELIVERY_LAUNCHER_VERSION")?,
            required_environment("LATTICE_DELIVERY_LAUNCHER_SHA256")?,
            PathBuf::from(required_environment("LATTICE_DELIVERY_SCHEMA_DIR")?),
            PathBuf::from(required_environment("LATTICE_DELIVERY_CODEX_HOME")?),
            PathBuf::from(required_environment("LATTICE_DELIVERY_ROOT")?),
            PathBuf::from(required_environment("LATTICE_DELIVERY_GIT_EXE")?),
            timeout,
            runtime,
        )?;
        Self::for_delivery(
            config,
            database,
            required_environment("LATTICE_TASK019_PASSWORD")?,
        )
    }

    /// Returns the process-configured run request. Status-only services resolve
    /// their exact request from `PostgreSQL` and therefore return `None` here.
    #[must_use]
    pub const fn request_binding(&self) -> Option<&DeliveryRunRequest> {
        self.request.as_ref()
    }

    /// Executes the one fixed delivery through the pure orchestrator.
    ///
    /// # Errors
    ///
    /// Returns only a bounded composition classification when no independently
    /// verified terminal receipt is available.
    pub fn run_json(&mut self) -> Result<Value, LatticedError> {
        let config = self
            .delivery
            .as_ref()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))?;
        let request = self
            .request
            .as_ref()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))?;
        if config.runtime == DeliveryRuntime::OfficialCodexAppServer {
            // Temporary fail-closed incident gate. The official Windows
            // sandbox helper must not be retried until its upstream regression
            // is resolved or the user explicitly authorizes a safety posture.
            return Err(LatticedError::new(LatticedErrorKind::OfficialLiveBlocked));
        }
        validate_scripted_fixture(config)?;
        let deadline = deadline(self.timeout)?;
        let ledger = DeliveryLedger::connect(&self.database, &self.password, deadline)
            .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
        let repository = config.delivery_root.join("repo");
        let mut ledger = PostgresDeliveryLedgerAdapter::for_delivery(
            ledger,
            request.clone(),
            path_text(&config.launcher)?,
            config.version.clone(),
            config.launcher_sha256.clone(),
            path_text(&config.schema_directory)?,
            path_text(&config.codex_home)?,
            path_text(&repository)?,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?;
        let workspace_config = DeliveryWorkspaceGitAdapterConfig::new(
            config.delivery_root.clone(),
            config.git_executable.clone(),
            config.timeout,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::WorkspaceConfiguration))?;
        let mut workspace_git =
            DeliveryWorkspaceGitAdapter::with_deadline(workspace_config, deadline);
        let identity = CodexIdentityExpectation::new(
            config.launcher.clone(),
            config.version.clone(),
            config.launcher_sha256.clone(),
        );
        let codex_config = CodexDeliveryAdapterConfig::new(
            identity,
            config.schema_directory.clone(),
            config.codex_home.clone(),
            DELIVERY_PROMPT,
            config.timeout,
            config.runtime,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::CodexConfiguration))?;
        let mut codex = CodexDeliveryAdapter::with_deadline(codex_config, deadline);
        match run_delivery(request, &mut ledger, &mut workspace_git, &mut codex) {
            Ok(receipt) => receipt_json(&receipt, "lattice-delivery"),
            Err(DeliveryOrchestratorError::Terminal { receipt, .. }) => Err(LatticedError::new(
                terminal_run_error_kind(receipt.status()),
            )),
            Err(error) => Err(map_orchestrator_error(&error)),
        }
    }

    /// Reads the exact terminal receipt from a fresh `PostgreSQL` connection.
    ///
    /// # Errors
    ///
    /// Fails closed when the connection, persisted evidence, or binding cannot
    /// be independently verified.
    pub fn status_json(&mut self) -> Result<Value, LatticedError> {
        let ledger =
            DeliveryLedger::connect(&self.database, &self.password, deadline(self.timeout)?)
                .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
        let expected_invocation = invocation_for_run(self.database.run_id())?;
        match PostgresDeliveryLedgerAdapter::for_status(
            ledger,
            &expected_invocation,
            DeliveryProfile::Task032CodexPostgres,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::ReconciliationRequired))?
        {
            PostgresDeliveryStatusReplay::Legacy(receipt) => Ok(legacy_receipt_json(&receipt)),
            PostgresDeliveryStatusReplay::Typed(mut ledger) => {
                let status_request = ledger.request().status_request();
                let receipt = delivery_status(&status_request, ledger.as_mut())
                    .map_err(|error| map_orchestrator_error(&error))?;
                receipt_json(&receipt, "delivery-ledger")
            }
        }
    }
}

impl DeliveryToolService for LatticedDeliveryService {
    fn run(&mut self) -> Result<Value, ToolExecutionError> {
        self.run_json()
            .map_err(|error| ToolExecutionError::new(error.code()))
    }

    fn status(&mut self) -> Result<Value, ToolExecutionError> {
        self.status_json()
            .map_err(|error| ToolExecutionError::new(error.code()))
    }
}

/// Starts the canonical newline-delimited MCP stdio server.
///
/// # Errors
///
/// Returns a bounded startup/configuration or transport failure.
pub fn serve_stdio_from_environment() -> Result<(), LatticedError> {
    let service = LatticedDeliveryService::from_environment()?;
    let input = io::stdin();
    let output = io::stdout();
    mcp::serve(service, input.lock(), output.lock())
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))
}

fn stable_run_binding(run_id: &str) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "profile".to_owned(),
            CanonicalValue::String(DeliveryProfile::Task032CodexPostgres.as_str().to_owned()),
        ),
        (
            "run_id".to_owned(),
            CanonicalValue::String(run_id.to_owned()),
        ),
    ])
}

fn invocation_for_run(run_id: &str) -> Result<Invocation, LatticedError> {
    let binding = stable_run_binding(run_id);
    let subject_digest = digest("lattice.task032.delivery-subject", &binding)?;
    Invocation::new(
        CONTRACT_VERSION,
        RequestId::new(format!("task032-request-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        TaskId::new(TASK_ID).map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        AttemptId::new(format!("task032-attempt-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        ProjectSnapshotId::new(PROJECT_SNAPSHOT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        subject_digest,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
}

fn request_for_delivery(
    run_id: &str,
    config: &LatticedDeliveryConfig,
) -> Result<DeliveryRunRequest, LatticedError> {
    let prompt_digest = digest(
        "lattice.task032.delivery-prompt",
        &CanonicalValue::String(DELIVERY_PROMPT.to_owned()),
    )?;
    let repository = config.delivery_root.join("repo");
    let binding = CanonicalValue::Object(vec![
        (
            "changed_path".to_owned(),
            CanonicalValue::String("answer.txt".to_owned()),
        ),
        (
            "codex_home".to_owned(),
            CanonicalValue::String(path_text(&config.codex_home)?),
        ),
        (
            "delivery_root".to_owned(),
            CanonicalValue::String(path_text(&config.delivery_root)?),
        ),
        (
            "git_executable".to_owned(),
            CanonicalValue::String(path_text(&config.git_executable)?),
        ),
        (
            "launcher_path".to_owned(),
            CanonicalValue::String(path_text(&config.launcher)?),
        ),
        (
            "launcher_sha256".to_owned(),
            CanonicalValue::String(config.launcher_sha256.clone()),
        ),
        (
            "launcher_version".to_owned(),
            CanonicalValue::String(config.version.clone()),
        ),
        (
            "profile".to_owned(),
            CanonicalValue::String(DeliveryProfile::Task032CodexPostgres.as_str().to_owned()),
        ),
        (
            "prompt_digest".to_owned(),
            CanonicalValue::String(prompt_digest.as_str().to_owned()),
        ),
        (
            "repository_path".to_owned(),
            CanonicalValue::String(path_text(&repository)?),
        ),
        (
            "runtime".to_owned(),
            CanonicalValue::String(runtime_name(config.runtime).to_owned()),
        ),
        (
            "schema_directory".to_owned(),
            CanonicalValue::String(path_text(&config.schema_directory)?),
        ),
        (
            "test_command_id".to_owned(),
            CanonicalValue::String("git-diff-no-index-exact-answer-v1".to_owned()),
        ),
        (
            "timeout_nanos".to_owned(),
            CanonicalValue::String(config.timeout.as_nanos().to_string()),
        ),
    ]);
    let configuration_digest = digest(
        "lattice.task032.delivery-execution-configuration-v2",
        &binding,
    )?;
    let invocation = invocation_for_run(run_id)?;
    DeliveryRunRequest::new(
        invocation,
        DeliveryProfile::Task032CodexPostgres,
        configuration_digest,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
}

fn validate_scripted_fixture(config: &LatticedDeliveryConfig) -> Result<(), LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected);
    if config.runtime != DeliveryRuntime::ScriptedAcceptance
        || config.delivery_root.file_name() != Some(OsStr::new("delivery"))
        || config.schema_directory.file_name() != Some(OsStr::new("schema"))
    {
        return Err(rejected());
    }

    let fixture_root = canonical_directory(config.delivery_root.parent().ok_or_else(rejected)?)?;
    let schema_parent =
        canonical_directory(config.schema_directory.parent().ok_or_else(rejected)?)?;
    if schema_parent != fixture_root {
        return Err(rejected());
    }
    let fixture_id = fixture_root
        .file_name()
        .and_then(OsStr::to_str)
        .filter(|value| is_lower_hex(value, 32))
        .ok_or_else(rejected)?;
    let fixture_parent = fixture_root.parent().ok_or_else(rejected)?;
    if fixture_parent.file_name() != Some(OsStr::new("lattice-delivery")) {
        return Err(rejected());
    }
    let target_root = fixture_parent.parent().ok_or_else(rejected)?;
    if target_root.file_name() != Some(OsStr::new("target")) {
        return Err(rejected());
    }
    let repository_root = canonical_directory(target_root.parent().ok_or_else(rejected)?)?;

    let launcher = canonical_regular_file(&config.launcher, MAX_SCRIPTED_LAUNCHER_BYTES)?;
    let codex_home = canonical_directory(&config.codex_home)?;
    if launcher != fixture_root.join("scripted-codex.cmd")
        || codex_home != canonical_directory(&fixture_root.join("codex-home"))?
    {
        return Err(rejected());
    }
    let server = canonical_regular_file(
        &fixture_root.join("scripted-codex.ps1"),
        MAX_SCRIPTED_SERVER_BYTES,
    )?;
    let marker_path = fixture_root.join(SCRIPTED_FIXTURE_MARKER_NAME);
    let marker_bytes = read_regular_file(&marker_path, MAX_SCRIPTED_MARKER_BYTES)?;
    let marker: Value = serde_json::from_slice(&marker_bytes).map_err(|_| rejected())?;
    let object = marker.as_object().ok_or_else(rejected)?;
    let expected_keys = [
        "codex_mode",
        "fixture_id",
        "kind",
        "launcher_path",
        "launcher_sha256",
        "repository_root",
        "root",
        "server_path",
        "server_sha256",
    ];
    if object.len() != expected_keys.len()
        || expected_keys.iter().any(|key| !object.contains_key(*key))
        || marker_string(object, "kind")? != SCRIPTED_FIXTURE_KIND
        || marker_string(object, "codex_mode")? != "SCRIPTED_ACCEPTANCE"
        || marker_string(object, "fixture_id")? != fixture_id
        || canonical_directory(Path::new(marker_string(object, "root")?))? != fixture_root
        || canonical_directory(Path::new(marker_string(object, "repository_root")?))?
            != repository_root
        || canonical_regular_file(
            Path::new(marker_string(object, "launcher_path")?),
            MAX_SCRIPTED_LAUNCHER_BYTES,
        )? != launcher
        || canonical_regular_file(
            Path::new(marker_string(object, "server_path")?),
            MAX_SCRIPTED_SERVER_BYTES,
        )? != server
    {
        return Err(rejected());
    }

    let launcher_sha256 = marker_string(object, "launcher_sha256")?;
    let server_sha256 = marker_string(object, "server_sha256")?;
    if !is_lower_hex(launcher_sha256, 64)
        || !is_lower_hex(server_sha256, 64)
        || launcher_sha256 != config.launcher_sha256
        || file_sha256(&launcher, MAX_SCRIPTED_LAUNCHER_BYTES)? != launcher_sha256
        || file_sha256(&server, MAX_SCRIPTED_SERVER_BYTES)? != server_sha256
        || read_regular_file(&server, MAX_SCRIPTED_SERVER_BYTES)? != SCRIPTED_SERVER_BYTES
        || read_regular_file(&launcher, MAX_SCRIPTED_LAUNCHER_BYTES)?
            != scripted_launcher_bytes(server_sha256)
    {
        return Err(rejected());
    }
    Ok(())
}

fn marker_string<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str, LatticedError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))
}

fn canonical_directory(path: &Path) -> Result<PathBuf, LatticedError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?;
    if !metadata.file_type().is_dir() {
        return Err(LatticedError::new(
            LatticedErrorKind::ScriptedFixtureRejected,
        ));
    }
    fs::canonicalize(path)
        .map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))
}

fn canonical_regular_file(path: &Path, max_bytes: u64) -> Result<PathBuf, LatticedError> {
    read_regular_file(path, max_bytes)?;
    fs::canonicalize(path)
        .map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))
}

fn read_regular_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>, LatticedError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?;
    if !metadata.file_type().is_file() || metadata.len() > max_bytes {
        return Err(LatticedError::new(
            LatticedErrorKind::ScriptedFixtureRejected,
        ));
    }
    fs::read(path).map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))
}

fn file_sha256(path: &Path, max_bytes: u64) -> Result<String, LatticedError> {
    let bytes = read_regular_file(path, max_bytes)?;
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}")
            .map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?;
    }
    Ok(output)
}

fn scripted_launcher_bytes(server_sha256: &str) -> Vec<u8> {
    format!(
        concat!(
            "@echo off\r\n",
            "if \"%~1\"==\"--version\" if \"%~2\"==\"\" goto version\r\n",
            "if \"%~1\"==\"app-server\" if \"%~2\"==\"generate-json-schema\" if \"%~3\"==\"--out\" if \"%~4\" NEQ \"\" if \"%~5\"==\"\" goto schema\r\n",
            "if \"%~1\"==\"app-server\" if \"%~2\"==\"--listen\" if \"%~3\"==\"stdio://\" if \"%~4\"==\"\" goto server\r\n",
            "exit /b 11\r\n",
            ":version\r\n",
            "echo codex-cli 0.144.6\r\n",
            "exit /b 0\r\n",
            ":schema\r\n",
            "\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0scripted-codex.ps1\" -ExpectedSelfSha256 \"{server_sha256}\" -Mode Schema -SchemaRoot \"%~4\"\r\n",
            "exit /b %ERRORLEVEL%\r\n",
            ":server\r\n",
            "\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0scripted-codex.ps1\" -ExpectedSelfSha256 \"{server_sha256}\" -Mode Server\r\n",
            "exit /b %ERRORLEVEL%\r\n"
        ),
        server_sha256 = server_sha256
    )
    .into_bytes()
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest(schema_id: &str, value: &CanonicalValue) -> Result<ContentDigest, LatticedError> {
    let domain = HashDomain::new(schema_id, "1.0")
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    let digest = canonical_sha256(&domain, value)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
}

fn receipt_json(
    receipt: &DeliveryReceipt,
    component: &'static str,
) -> Result<Value, LatticedError> {
    let outcome = receipt.outcome();
    let request = outcome.request();
    let mut value = Map::new();
    value.insert(
        "status".to_owned(),
        Value::String(terminal_status_name(receipt.status()).to_owned()),
    );
    value.insert("component".to_owned(), Value::String(component.to_owned()));
    value.insert(
        "profile".to_owned(),
        Value::String(request.binding().profile().as_str().to_owned()),
    );
    value.insert(
        "request_id".to_owned(),
        Value::String(
            request
                .binding()
                .invocation()
                .request_id()
                .as_str()
                .to_owned(),
        ),
    );
    value.insert(
        "configuration_digest".to_owned(),
        Value::String(request.binding().configuration_digest().as_str().to_owned()),
    );
    value.insert(
        "intent_digest".to_owned(),
        Value::String(request.intent_digest().as_str().to_owned()),
    );
    value.insert(
        "outcome_digest".to_owned(),
        Value::String(outcome.outcome_digest().as_str().to_owned()),
    );
    value.insert(
        "receipt_digest".to_owned(),
        Value::String(receipt.receipt_digest().as_str().to_owned()),
    );

    if let Some(completed) = request.completed_evidence() {
        value.extend(completed_receipt_fields(completed));
    } else {
        let stage = request
            .failure_stage()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
        let code = request
            .failure_code()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
        value.insert(
            "failure_stage".to_owned(),
            Value::String(stage_name(stage).to_owned()),
        );
        value.insert("failure_code".to_owned(), Value::String(code.to_owned()));
    }
    Ok(Value::Object(value))
}

struct LegacyReceiptJsonFields<'a> {
    intent_digest: &'a str,
    outcome_digest: &'a str,
    launcher_path: &'a str,
    version: &'a str,
    launcher_sha256: &'a str,
    schema_bundle_sha256: &'a str,
    schema_file_count: usize,
    thread_id: &'a str,
    turn_id: &'a str,
    repository_path: &'a str,
    commit_sha: &'a str,
    parent_sha: &'a str,
}

impl<'a> From<&'a LegacyDeliveryReceipt> for LegacyReceiptJsonFields<'a> {
    fn from(receipt: &'a LegacyDeliveryReceipt) -> Self {
        Self {
            intent_digest: receipt.intent_digest(),
            outcome_digest: receipt.outcome_digest(),
            launcher_path: receipt.launcher_path(),
            version: receipt.version(),
            launcher_sha256: receipt.launcher_sha256(),
            schema_bundle_sha256: receipt.schema_bundle_sha256(),
            schema_file_count: receipt.schema_file_count(),
            thread_id: receipt.thread_id(),
            turn_id: receipt.turn_id(),
            repository_path: receipt.repository_path(),
            commit_sha: receipt.commit_sha(),
            parent_sha: receipt.parent_sha(),
        }
    }
}

fn legacy_receipt_json(receipt: &LegacyDeliveryReceipt) -> Value {
    let fields = receipt.into();
    legacy_receipt_json_from_fields(&fields)
}

fn legacy_receipt_json_from_fields(fields: &LegacyReceiptJsonFields<'_>) -> Value {
    json!({
        "status": "COMPLETED",
        "component": "delivery-ledger",
        "receipt_format": LEGACY_RECEIPT_FORMAT,
        "changed_path": "answer.txt",
        "commit_sha": fields.commit_sha,
        "intent_digest": fields.intent_digest,
        "launcher_path": fields.launcher_path,
        "launcher_sha256": fields.launcher_sha256,
        "outcome_digest": fields.outcome_digest,
        "parent_sha": fields.parent_sha,
        "repository_path": fields.repository_path,
        "schema_bundle_sha256": fields.schema_bundle_sha256,
        "schema_file_count": fields.schema_file_count,
        "test": "FIXED_TEST_PASSED",
        "test_command_id": "git-diff-no-index-exact-answer-v1",
        "thread_id": fields.thread_id,
        "turn_id": fields.turn_id,
        "version": fields.version,
    })
}

fn completed_receipt_fields(completed: &CompletedDeliveryEvidence) -> Map<String, Value> {
    let codex = completed.codex();
    let workspace = completed.workspace();
    let git = completed.git();
    Map::from_iter([
        (
            "launcher_path".to_owned(),
            Value::String(codex.launcher_locator().to_owned()),
        ),
        (
            "version".to_owned(),
            Value::String(codex.version().to_owned()),
        ),
        (
            "launcher_sha256".to_owned(),
            Value::String(codex.launcher_sha256().as_str().to_owned()),
        ),
        (
            "schema_bundle_sha256".to_owned(),
            Value::String(codex.schema_bundle_sha256().as_str().to_owned()),
        ),
        (
            "schema_file_count".to_owned(),
            Value::from(codex.schema_file_count()),
        ),
        (
            "repository_path".to_owned(),
            Value::String(workspace.workspace_locator().to_owned()),
        ),
        ("changed_paths".to_owned(), json!(["answer.txt"])),
        (
            "test".to_owned(),
            Value::String("FIXED_TEST_PASSED".to_owned()),
        ),
        (
            "test_command_id".to_owned(),
            Value::String("git-diff-no-index-exact-answer-v1".to_owned()),
        ),
        (
            "baseline_commit".to_owned(),
            Value::String(workspace.baseline_commit().to_owned()),
        ),
        (
            "parent_sha".to_owned(),
            Value::String(git.parent_commit().to_owned()),
        ),
        (
            "commit_sha".to_owned(),
            Value::String(git.commit().to_owned()),
        ),
        (
            "thread_id".to_owned(),
            Value::String(codex.thread_id().to_owned()),
        ),
        (
            "turn_id".to_owned(),
            Value::String(codex.turn_id().to_owned()),
        ),
        (
            "codex_runtime".to_owned(),
            Value::String(runtime_name(codex.runtime()).to_owned()),
        ),
    ])
}

const fn terminal_status_name(status: DeliveryTerminalStatus) -> &'static str {
    match status {
        DeliveryTerminalStatus::Completed => "COMPLETED",
        DeliveryTerminalStatus::Failed => "FAILED",
        DeliveryTerminalStatus::ReconciliationRequired => "RECONCILIATION_REQUIRED",
    }
}

const fn runtime_name(runtime: DeliveryRuntime) -> &'static str {
    match runtime {
        DeliveryRuntime::ScriptedAcceptance => "SCRIPTED_ACCEPTANCE",
        DeliveryRuntime::OfficialCodexAppServer => "OFFICIAL_CODEX_APP_SERVER",
    }
}

const fn stage_name(stage: DeliveryStage) -> &'static str {
    match stage {
        DeliveryStage::Intent => "INTENT",
        DeliveryStage::WorkspacePrepare => "WORKSPACE_PREPARE",
        DeliveryStage::Codex => "CODEX",
        DeliveryStage::ScopeVerification => "SCOPE_VERIFICATION",
        DeliveryStage::FixedTest => "FIXED_TEST",
        DeliveryStage::GitCommit => "GIT_COMMIT",
        DeliveryStage::Outcome => "OUTCOME",
        DeliveryStage::Receipt => "RECEIPT",
    }
}

fn map_orchestrator_error(error: &DeliveryOrchestratorError) -> LatticedError {
    let kind = match error {
        DeliveryOrchestratorError::Intent(_) => LatticedErrorKind::Intent,
        DeliveryOrchestratorError::Contract(_) => LatticedErrorKind::Contract,
        DeliveryOrchestratorError::OutcomePersistence(_) => {
            LatticedErrorKind::ReconciliationRequired
        }
        DeliveryOrchestratorError::ReceiptRead(error)
            if error.certainty() == DeliveryFailureCertainty::Ambiguous
                || error.kind() == PortErrorKind::Ambiguous =>
        {
            LatticedErrorKind::ReconciliationRequired
        }
        DeliveryOrchestratorError::ReceiptRead(_) => LatticedErrorKind::ReceiptRead,
        DeliveryOrchestratorError::ReceiptMismatch => LatticedErrorKind::ReceiptMismatch,
        DeliveryOrchestratorError::Terminal { cause, receipt } => match receipt.status() {
            DeliveryTerminalStatus::Failed => LatticedErrorKind::DeliveryFailed,
            DeliveryTerminalStatus::ReconciliationRequired
                if cause.certainty() == DeliveryFailureCertainty::Ambiguous
                    || cause.kind() == PortErrorKind::Ambiguous =>
            {
                LatticedErrorKind::ReconciliationRequired
            }
            DeliveryTerminalStatus::ReconciliationRequired => {
                LatticedErrorKind::ReconciliationRequired
            }
            DeliveryTerminalStatus::Completed => LatticedErrorKind::ReceiptMismatch,
        },
    };
    LatticedError::new(kind)
}

const fn terminal_run_error_kind(status: DeliveryTerminalStatus) -> LatticedErrorKind {
    match status {
        DeliveryTerminalStatus::Failed => LatticedErrorKind::DeliveryFailed,
        DeliveryTerminalStatus::ReconciliationRequired => LatticedErrorKind::ReconciliationRequired,
        DeliveryTerminalStatus::Completed => LatticedErrorKind::ReceiptMismatch,
    }
}

fn required_environment(name: &'static str) -> Result<String, LatticedError> {
    env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            if name == "LATTICE_TASK019_PASSWORD" {
                LatticedError::new(LatticedErrorKind::DatabaseSecret)
            } else {
                LatticedError::new(LatticedErrorKind::Configuration)
            }
        })
}

fn validate_secret_and_timeout(password: &str, timeout: Duration) -> Result<(), LatticedError> {
    if password.is_empty() {
        return Err(LatticedError::new(LatticedErrorKind::DatabaseSecret));
    }
    if timeout.is_zero() || timeout > Duration::from_secs(MAX_TIMEOUT_SECONDS) {
        return Err(LatticedError::new(LatticedErrorKind::Configuration));
    }
    Ok(())
}

fn parse_timeout(value: &str) -> Result<Duration, LatticedError> {
    value
        .parse::<u64>()
        .ok()
        .filter(|seconds| (1..=MAX_TIMEOUT_SECONDS).contains(seconds))
        .map(Duration::from_secs)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))
}

fn deadline(timeout: Duration) -> Result<Instant, LatticedError> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))
}

fn path_text(path: &Path) -> Result<String, LatticedError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_ports::{DeliveryFailureCertainty, DeliveryPortError};

    #[test]
    fn terminal_delivery_receipts_are_never_run_success() {
        assert_eq!(
            terminal_run_error_kind(DeliveryTerminalStatus::Failed),
            LatticedErrorKind::DeliveryFailed
        );
        assert_eq!(
            terminal_run_error_kind(DeliveryTerminalStatus::ReconciliationRequired),
            LatticedErrorKind::ReconciliationRequired
        );
        assert_eq!(
            terminal_run_error_kind(DeliveryTerminalStatus::Completed),
            LatticedErrorKind::ReceiptMismatch
        );
    }

    #[test]
    fn ambiguous_outcome_persistence_requires_reconciliation() {
        let error = DeliveryOrchestratorError::OutcomePersistence(DeliveryPortError::new(
            DeliveryStage::Outcome,
            PortErrorKind::Ambiguous,
            DeliveryFailureCertainty::Ambiguous,
            "OUTCOME_UNKNOWN",
        ));

        assert_eq!(
            map_orchestrator_error(&error).kind(),
            LatticedErrorKind::ReconciliationRequired
        );
    }

    #[test]
    fn known_outcome_persistence_failure_after_intent_requires_reconciliation() {
        let error = DeliveryOrchestratorError::OutcomePersistence(DeliveryPortError::new(
            DeliveryStage::Outcome,
            PortErrorKind::Timeout,
            DeliveryFailureCertainty::Known,
            "OUTCOME_DEADLINE_EXPIRED",
        ));

        assert_eq!(
            map_orchestrator_error(&error).kind(),
            LatticedErrorKind::ReconciliationRequired
        );
    }

    #[test]
    fn legacy_status_json_is_versioned_without_invented_typed_evidence() {
        let value = legacy_receipt_json_from_fields(&LegacyReceiptJsonFields {
            intent_digest: &"a".repeat(64),
            outcome_digest: &"b".repeat(64),
            launcher_path: r"C:\tools\codex.exe",
            version: "codex-cli 0.144.6",
            launcher_sha256: &"c".repeat(64),
            schema_bundle_sha256: &"d".repeat(64),
            schema_file_count: 1,
            thread_id: "thread-1",
            turn_id: "turn-1",
            repository_path: r"C:\delivery\repo",
            commit_sha: &"e".repeat(40),
            parent_sha: &"f".repeat(40),
        });
        let object = value.as_object().expect("legacy receipt object");

        assert_eq!(
            object.get("receipt_format").and_then(Value::as_str),
            Some("legacy-delivery-result-v1")
        );
        assert_eq!(
            object.get("status").and_then(Value::as_str),
            Some("COMPLETED")
        );
        for unavailable in [
            "configuration_digest",
            "profile",
            "request_id",
            "receipt_digest",
            "codex_runtime",
        ] {
            assert!(
                !object.contains_key(unavailable),
                "legacy replay must not synthesize {unavailable}"
            );
        }
    }
}
