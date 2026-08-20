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
use lattice_codebase_memory::digest_query_text;
use lattice_codex_adapter::{
    CodexDeliveryAdapter, CodexDeliveryAdapterConfig, CodexIdentityExpectation,
};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CompletedDeliveryEvidence, ContentDigest, DeliveryProfile,
    DeliveryReceipt, DeliveryRunRequest, DeliveryRuntime, DeliveryStage, DeliveryTerminalStatus,
    GitObjectId, GraphMemoryReceipt, GraphMemoryRunRequest, Invocation, MemoryQuery, ProjectId,
    ProjectSnapshotId, RequestId, TaskId,
};
use lattice_graphify_adapter::{
    ExactGitSnapshotMaterializer, GitSnapshotConfig, GraphOutputLimits, GraphifyRuntimeConfig,
    PinnedGraphifyAdapter, SnapshotBridge, SnapshotLimits,
};
use lattice_orchestrator::{
    DeliveryOrchestratorError, delivery_status, graph_memory_status, run_delivery, run_graph_memory,
};
use lattice_ports::{DeliveryFailureCertainty, PortErrorKind};
use lattice_postgres_codebase_memory::{ExtensionTarget, PostgresCodebaseMemory};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::DELIVERY_PROMPT;
use crate::delivery_ledger::{
    DeliveryDatabaseBinding, DeliveryLedger, DeliveryReceipt as LegacyDeliveryReceipt,
    LEGACY_RECEIPT_FORMAT, PostgresDeliveryLedgerAdapter, PostgresDeliveryStatusReplay,
    connect_fixed_runtime_client,
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
const MAX_GIT_EXECUTABLE_BYTES: u64 = 64 * 1024 * 1024;
const GRAPH_TASK_ID: &str = "TASK-033";
const GRAPH_PROJECT_ID: &str = "task032-delivery";
const GRAPH_PROJECT_SNAPSHOT_ID: &str = "task032-delivery:graph-snapshot:1";
const GRAPH_QUERY: &str = "lattice_delivery_fixture";
const GRAPH_RETRIEVAL_LIMIT: u16 = 10;
const GRAPH_MEMORY_ROOT_NAME: &str = "graph-memory";
const GRAPHIFY_RUNTIME_RELATIVE_PATH: &str = "target/supply-chain/graphify-v0.9.33/wsl-runtime";
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
    TerminalCauseRejected,
    ReconciliationRequired,
    OfficialLiveBlocked,
    ScriptedFixtureRejected,
    GraphConfiguration,
    GraphExecution,
    GraphReceiptRead,
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
            Self::TerminalCauseRejected => "LATTICE_DELIVERY_TERMINAL_CAUSE_REJECTED",
            Self::ReconciliationRequired => "LATTICE_DELIVERY_RECONCILIATION_REQUIRED",
            Self::OfficialLiveBlocked => "LATTICE_OFFICIAL_CODEX_FAILED_DIAGNOSTIC",
            Self::ScriptedFixtureRejected => "LATTICE_SCRIPTED_FIXTURE_REJECTED",
            Self::GraphConfiguration => "LATTICE_GRAPH_MEMORY_CONFIGURATION_REJECTED",
            Self::GraphExecution => "LATTICE_GRAPH_MEMORY_RUN_REJECTED",
            Self::GraphReceiptRead => "LATTICE_GRAPH_MEMORY_RECEIPT_REJECTED",
            Self::Transport => "LATTICED_STDIO_REJECTED",
        }
    }
}

/// Bounded composition failure safe for CLI/MCP diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LatticedError {
    kind: LatticedErrorKind,
    terminal_cause: Option<TerminalCause>,
}

impl LatticedError {
    pub(crate) const fn new(kind: LatticedErrorKind) -> Self {
        Self {
            kind,
            terminal_cause: None,
        }
    }

    const fn terminal(kind: LatticedErrorKind, terminal_cause: TerminalCause) -> Self {
        Self {
            kind,
            terminal_cause: Some(terminal_cause),
        }
    }

    #[must_use]
    pub const fn kind(self) -> LatticedErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }

    /// Returns a stage/cause pair only for a verified closed terminal failure.
    #[must_use]
    pub const fn terminal_cause(self) -> Option<TerminalCause> {
        self.terminal_cause
    }
}

impl fmt::Display for LatticedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for LatticedError {}

/// Closed, secret-free terminal identity suitable for CLI and MCP output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCause {
    stage: &'static str,
    code: &'static str,
}

impl TerminalCause {
    #[must_use]
    pub const fn stage(self) -> &'static str {
        self.stage
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }
}

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
        let fixture = validate_scripted_fixture(config)?;
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
            Ok(receipt) => {
                let graph_receipt = run_delivery_graph_memory(
                    &self.database,
                    &self.password,
                    config,
                    &fixture,
                    deadline,
                    &receipt,
                )?;
                composed_receipt_json(&receipt, "lattice-delivery", &graph_receipt)
            }
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
                let graph_request =
                    graph_request_for_delivery_receipt(self.database.run_id(), &receipt)?;
                let graph_receipt = load_delivery_graph_receipt(
                    &self.database,
                    &self.password,
                    deadline(self.timeout)?,
                    &graph_request,
                )?;
                composed_receipt_json(&receipt, "delivery-ledger", &graph_receipt)
            }
        }
    }
}

impl DeliveryToolService for LatticedDeliveryService {
    fn run(&mut self) -> Result<Value, ToolExecutionError> {
        self.run_json().map_err(tool_execution_error)
    }

    fn status(&mut self) -> Result<Value, ToolExecutionError> {
        self.status_json().map_err(tool_execution_error)
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ScriptedFixturePaths {
    root: PathBuf,
    repository_root: PathBuf,
}

fn validate_scripted_fixture(
    config: &LatticedDeliveryConfig,
) -> Result<ScriptedFixturePaths, LatticedError> {
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
    Ok(ScriptedFixturePaths {
        root: fixture_root,
        repository_root,
    })
}

fn graph_request_for_delivery_receipt(
    run_id: &str,
    receipt: &DeliveryReceipt,
) -> Result<GraphMemoryRunRequest, LatticedError> {
    if receipt.status() != DeliveryTerminalStatus::Completed {
        return Err(LatticedError::new(LatticedErrorKind::ReceiptMismatch));
    }
    let delivery_request = receipt.outcome().request();
    let completed = delivery_request
        .completed_evidence()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new(format!("task033-graph-request-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        TaskId::new(GRAPH_TASK_ID).map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        AttemptId::new(format!("task033-graph-attempt-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        ProjectSnapshotId::new(GRAPH_PROJECT_SNAPSHOT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        receipt.receipt_digest().clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    let query_digest = digest_query_text(GRAPH_QUERY)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new(GRAPH_PROJECT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        GitObjectId::new(completed.git().commit())
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        query_digest,
        delivery_request.binding().configuration_digest().clone(),
        GRAPH_RETRIEVAL_LIMIT,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
}

fn run_delivery_graph_memory(
    database: &DeliveryDatabaseBinding,
    password: &str,
    config: &LatticedDeliveryConfig,
    fixture: &ScriptedFixturePaths,
    deadline: Instant,
    delivery_receipt: &DeliveryReceipt,
) -> Result<GraphMemoryReceipt, LatticedError> {
    let request = graph_request_for_delivery_receipt(database.run_id(), delivery_receipt)?;
    let query = MemoryQuery::new(&request, GRAPH_QUERY, GRAPH_RETRIEVAL_LIMIT)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::GraphExecution))?;
    let graph_root = fixture.root.join(GRAPH_MEMORY_ROOT_NAME);
    let bridge = SnapshotBridge::new();
    let snapshot_config = GitSnapshotConfig::new(
        config.git_executable.clone(),
        graph_executable_sha256(&config.git_executable)?,
        config.delivery_root.join("repo"),
        graph_root.join("snapshots"),
        SnapshotLimits::default(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let mut snapshot = ExactGitSnapshotMaterializer::with_bridge(snapshot_config, bridge.clone());

    let system_root = env::var_os("SystemRoot")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let graphify_config = GraphifyRuntimeConfig::new(
        PathBuf::from(system_root).join("System32/wsl.exe"),
        fixture.repository_root.join(GRAPHIFY_RUNTIME_RELATIVE_PATH),
        graph_root.join("staging"),
        remaining,
        GraphOutputLimits::default(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let mut graphify = PinnedGraphifyAdapter::new(graphify_config, bridge);
    let client = connect_fixed_runtime_client(database, password, deadline)
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    let target = ExtensionTarget::new(database.database_name(), database.run_id())
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let mut memory = PostgresCodebaseMemory::new(client, target)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;

    run_graph_memory(&request, &query, &mut snapshot, &mut graphify, &mut memory)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphExecution))
}

fn load_delivery_graph_receipt(
    database: &DeliveryDatabaseBinding,
    password: &str,
    deadline: Instant,
    request: &GraphMemoryRunRequest,
) -> Result<GraphMemoryReceipt, LatticedError> {
    let client = connect_fixed_runtime_client(database, password, deadline)
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    let target = ExtensionTarget::new(database.database_name(), database.run_id())
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let mut memory = PostgresCodebaseMemory::new(client, target)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    graph_memory_status(request, &mut memory)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphReceiptRead))
}

fn graph_executable_sha256(path: &Path) -> Result<String, LatticedError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_GIT_EXECUTABLE_BYTES {
        return Err(LatticedError::new(LatticedErrorKind::GraphConfiguration));
    }
    let bytes =
        fs::read(path).map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}")
            .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    }
    Ok(output)
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

struct GraphReceiptJsonFields<'a> {
    project_id: &'a str,
    commit_sha: &'a str,
    query_digest: &'a str,
    analysis_digest: &'a str,
    record_count: u64,
    persistence_digest: &'a str,
    retrieval_digest: &'a str,
    result_count: u64,
    receipt_digest: &'a str,
    database_identity_digest: &'a str,
    extension_manifest_digest: &'a str,
}

fn composed_receipt_json(
    delivery_receipt: &DeliveryReceipt,
    component: &'static str,
    graph_receipt: &GraphMemoryReceipt,
) -> Result<Value, LatticedError> {
    let request = graph_receipt.persistence().request();
    let persistence = graph_receipt.persistence();
    let retrieval = graph_receipt.retrieval();
    let identity = persistence.identity();
    let fields = GraphReceiptJsonFields {
        project_id: request.project_id().as_str(),
        commit_sha: request.commit_id().as_str(),
        query_digest: request.query_digest().as_str(),
        analysis_digest: persistence.analysis_digest().as_str(),
        record_count: u64::from(persistence.record_count()),
        persistence_digest: persistence.persistence_digest().as_str(),
        retrieval_digest: retrieval.retrieval_digest().as_str(),
        result_count: u64::try_from(retrieval.results().len())
            .map_err(|_| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?,
        receipt_digest: graph_receipt.receipt_digest().as_str(),
        database_identity_digest: identity.database_identity_digest().as_str(),
        extension_manifest_digest: identity.extension_manifest_digest().as_str(),
    };
    append_graph_receipt_fields(receipt_json(delivery_receipt, component)?, &fields)
}

fn append_graph_receipt_fields(
    mut value: Value,
    fields: &GraphReceiptJsonFields<'_>,
) -> Result<Value, LatticedError> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    let additions = Map::from_iter([
        (
            "graph_status".to_owned(),
            Value::String("COMPLETED".to_owned()),
        ),
        (
            "graph_project_id".to_owned(),
            Value::String(fields.project_id.to_owned()),
        ),
        (
            "graph_commit_sha".to_owned(),
            Value::String(fields.commit_sha.to_owned()),
        ),
        (
            "graph_query_digest".to_owned(),
            Value::String(fields.query_digest.to_owned()),
        ),
        (
            "graph_analysis_digest".to_owned(),
            Value::String(fields.analysis_digest.to_owned()),
        ),
        (
            "graph_record_count".to_owned(),
            Value::from(fields.record_count),
        ),
        (
            "graph_persistence_digest".to_owned(),
            Value::String(fields.persistence_digest.to_owned()),
        ),
        (
            "graph_retrieval_digest".to_owned(),
            Value::String(fields.retrieval_digest.to_owned()),
        ),
        (
            "graph_result_count".to_owned(),
            Value::from(fields.result_count),
        ),
        (
            "graph_receipt_digest".to_owned(),
            Value::String(fields.receipt_digest.to_owned()),
        ),
        (
            "graph_database_identity_digest".to_owned(),
            Value::String(fields.database_identity_digest.to_owned()),
        ),
        (
            "graph_extension_manifest_digest".to_owned(),
            Value::String(fields.extension_manifest_digest.to_owned()),
        ),
    ]);
    if additions.keys().any(|key| object.contains_key(key)) {
        return Err(LatticedError::new(LatticedErrorKind::ReceiptMismatch));
    }
    object.extend(additions);
    Ok(value)
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
            DeliveryTerminalStatus::Failed => {
                return terminal_error(cause.stage(), cause.code(), receipt);
            }
            DeliveryTerminalStatus::ReconciliationRequired
                if cause.certainty() == DeliveryFailureCertainty::Ambiguous
                    || cause.kind() == PortErrorKind::Ambiguous =>
            {
                LatticedErrorKind::ReconciliationRequired
            }
            status => terminal_run_error_kind(status),
        },
    };
    LatticedError::new(kind)
}

fn terminal_error(stage: DeliveryStage, code: &str, receipt: &DeliveryReceipt) -> LatticedError {
    let request = receipt.outcome().request();
    if request.failure_stage() != Some(stage) || request.failure_code() != Some(code) {
        return LatticedError::new(LatticedErrorKind::ReceiptMismatch);
    }
    let Some(cause) = terminal_cause(stage, code) else {
        return LatticedError::new(LatticedErrorKind::TerminalCauseRejected);
    };
    LatticedError::terminal(LatticedErrorKind::DeliveryFailed, cause)
}

fn tool_execution_error(error: LatticedError) -> ToolExecutionError {
    match error.terminal_cause() {
        Some(cause) => ToolExecutionError::terminal(error.code(), cause.stage(), cause.code()),
        None => ToolExecutionError::new(error.code()),
    }
}

fn terminal_cause(stage: DeliveryStage, code: &str) -> Option<TerminalCause> {
    let code = CLOSED_TERMINAL_CAUSE_CODES
        .iter()
        .copied()
        .find(|candidate| *candidate == code)?;
    Some(TerminalCause {
        stage: stage_name(stage),
        code,
    })
}

// This explicit allowlist is the complete safe vocabulary emitted by the
// current delivery adapters. Unknown or malformed future leaves fail closed.
const CLOSED_TERMINAL_CAUSE_CODES: &[&str] = &[
    "ANSWER_ALTERNATE_DATA_STREAM_DRIFT",
    "ANSWER_BYTES_DRIFT_BEFORE_STAGE",
    "ANSWER_BYTES_MISMATCH",
    "ANSWER_HARDLINK_COUNT_DRIFT",
    "ROOT_CREATE_FAILED",
    "ROOT_INSPECTION_FAILED",
    "ROOT_MUST_BE_ABSENT",
    "ROOT_MUST_BE_ABSOLUTE_AND_NORMALIZED",
    "ROOT_PARENT_MISSING",
    "DIRECTORY_CANONICALIZE_FAILED",
    "DIRECTORY_INSPECTION_FAILED",
    "DIRECTORY_NOT_CANONICAL",
    "DIRECTORY_PATH_ESCAPE",
    "CONTROL_DIRECTORY_CREATE_FAILED",
    "CONTROL_FILE_CREATE_FAILED",
    "CONTROL_FILE_WRITE_FAILED",
    "BASELINE_SOURCE_BOUNDARY_DRIFT",
    "BASELINE_SOURCE_BYTES_DRIFT",
    "BASELINE_SOURCE_DIRECTORY_CREATE_FAILED",
    "BASELINE_SOURCE_INSPECTION_FAILED",
    "BASELINE_SOURCE_MISSING",
    "BASELINE_TREE_INSPECTION_FAILED",
    "GIT_INIT_FAILED",
    "GIT_ADD_FAILED",
    "GIT_CONFIG_DRIFT",
    "GIT_CONFIG_INSPECTION_FAILED",
    "GIT_CONFIG_UNSAFE",
    "GIT_DELIVERY_ALREADY_PREPARED",
    "GIT_DELIVERY_BINDING_MISMATCH",
    "GIT_DELIVERY_COMMIT_EVIDENCE_DRIFT",
    "GIT_DELIVERY_CONFIG_PATH_NOT_ABSOLUTE",
    "GIT_DELIVERY_CONFIG_TIMEOUT_ZERO",
    "GIT_DELIVERY_DEADLINE_EXPIRED",
    "GIT_DELIVERY_DEADLINE_INVALID",
    "GIT_DELIVERY_DIGEST_DOMAIN_INVALID",
    "GIT_DELIVERY_DIGEST_FAILED",
    "GIT_DELIVERY_DIGEST_INVALID",
    "GIT_DELIVERY_STAGE_OUT_OF_ORDER",
    "GIT_DELIVERY_TYPED_COMMIT_EVIDENCE_INVALID",
    "GIT_DELIVERY_WORKSPACE_LOCATOR_INVALID",
    "GIT_EXE_CANONICALIZE_FAILED",
    "GIT_EXE_INSPECTION_FAILED",
    "GIT_EXE_MUST_BE_ABSOLUTE",
    "GIT_EXE_NOT_REGULAR",
    "GIT_OBJECT_ID_MALFORMED",
    "GIT_POINTER_DRIFT",
    "GIT_POINTER_MISSING",
    "GIT_POINTER_READ_FAILED",
    "GIT_POINTER_UNSAFE",
    "INITIAL_COMMIT_FAILED",
    "INITIAL_REFS_FAILED",
    "INITIAL_REPOSITORY_NOT_CLEAN",
    "INITIAL_SOURCE_STAGE_FAILED",
    "INITIAL_STATUS_FAILED",
    "ATTRIBUTES_DRIFT",
    "EXPECTED_ANSWER_DRIFT",
    "GLOBAL_CONFIG_DRIFT",
    "HEAD_DRIFT",
    "HEAD_INSPECTION_FAILED",
    "HOOKS_DIRECTORY_NOT_EMPTY",
    "HOOKS_READ_FAILED",
    "INDEX_DRIFT",
    "INDEX_INSPECTION_FAILED",
    "NON_REGULAR_REPOSITORY_ENTRY",
    "FOREIGN_PATH",
    "REPOSITORY_SCAN_FAILED",
    "SCOPE_STATUS_FAILED",
    "STAGED_ANSWER_BYTES_DRIFT",
    "STAGED_BLOB_READ_FAILED",
    "STAGED_SCOPE_DRIFT",
    "STAGED_SCOPE_FAILED",
    "STAGED_STATUS_DRIFT",
    "STAGED_STATUS_FAILED",
    "STREAM_INSPECTION_FAILED",
    "STREAM_PROBE_UNAVAILABLE",
    "HARDLINK_INSPECTION_FAILED",
    "HARDLINK_PROBE_UNAVAILABLE",
    "REF_DRIFT",
    "REF_INSPECTION_FAILED",
    "REF_OUTPUT_MALFORMED",
    "UNEXPECTED_GIT_REFS",
    "UNEXPECTED_GIT_STATUS",
    "UNSAFE_LOCAL_GIT_CONFIG",
    "ANSWER_MISSING",
    "FIXED_TEST_START_FAILED",
    "FIXED_TEST_FAILED",
    "DEADLINE_INVALID",
    "COMMIT_OUTCOME_UNKNOWN",
    "COMMIT_WAIT_UNKNOWN",
    "COMMIT_EXIT_UNKNOWN",
    "COMMIT_DID_NOT_ADVANCE_HEAD",
    "POST_COMMIT_EVIDENCE_REJECTED",
    "POST_COMMIT_BLOB_FAILED",
    "POST_COMMIT_DEADLINE_UNKNOWN",
    "POST_COMMIT_DIFF_FAILED",
    "POST_COMMIT_HEAD_UNKNOWN",
    "POST_COMMIT_METADATA_UNKNOWN",
    "POST_COMMIT_PARENT_FAILED",
    "POST_COMMIT_REFS_FAILED",
    "POST_COMMIT_STATUS_FAILED",
    "POST_COMMIT_VERIFICATION_FAILED",
    "DELIVERY_LEDGER_BINDING_MISMATCH",
    "DELIVERY_LEDGER_CHECKPOINT_CORRUPT",
    "DELIVERY_LEDGER_CONNECT_FAILED",
    "DELIVERY_LEDGER_DEADLINE_EXPIRED",
    "DELIVERY_LEDGER_EVIDENCE_INVALID",
    "DELIVERY_LEDGER_INTENT_EVIDENCE_INVALID",
    "DELIVERY_LEDGER_INVALID_BINDING",
    "DELIVERY_LEDGER_MUTATION_DEADLINE_AMBIGUOUS",
    "DELIVERY_LEDGER_COMMIT_OUTCOME_UNKNOWN",
    "DELIVERY_LEDGER_OUTCOME_APPEND_CORRUPT",
    "DELIVERY_LEDGER_OUTCOME_EVIDENCE_INVALID",
    "DELIVERY_LEDGER_PERSISTED_INTENT_CORRUPT",
    "DELIVERY_LEDGER_PHYSICAL_STATE_MISMATCH",
    "DELIVERY_LEDGER_RETAINED_ROW_CORRUPT",
    "DELIVERY_LEDGER_RECONCILIATION_REQUIRED",
    "DELIVERY_LEDGER_REJECTED",
    "DELIVERY_LEDGER_SCHEMA_REJECTED",
    "DELIVERY_LEDGER_STATUS_ONLY",
    "CONTRACT_EVIDENCE_REJECTED",
    "OUTCOME_PERSISTENCE_AFTER_DURABLE_INTENT_UNKNOWN",
    "CODEX_LAUNCHER_PATH_MISMATCH",
    "CODEX_LAUNCHER_NOT_FILE",
    "CODEX_LAUNCHER_READ_FAILED",
    "CODEX_LAUNCHER_DIGEST_MISMATCH",
    "CODEX_LAUNCHER_CHANGED",
    "CODEX_VERSION_COMMAND_FAILED",
    "CODEX_VERSION_OUTPUT_INVALID",
    "CODEX_VERSION_MISMATCH",
    "CODEX_SCHEMA_OUTPUT_EXISTS",
    "CODEX_SCHEMA_GENERATION_FAILED",
    "CODEX_SCHEMA_BUNDLE_INVALID",
    "CODEX_SCHEMA_BUNDLE_EMPTY",
    "CODEX_SCHEMA_READ_FAILED",
    "CODEX_IDENTITY_TIMEOUT",
    "CODEX_IDENTITY_PROCESS_CONTAINMENT_FAILED",
    "CODEX_CONFIG_HOME_NOT_ABSOLUTE",
    "CODEX_CONFIG_LAUNCHER_DIGEST_INVALID",
    "CODEX_CONFIG_LAUNCHER_NOT_ABSOLUTE",
    "CODEX_CONFIG_PROMPT_EMPTY",
    "CODEX_CONFIG_SCHEMA_PATH_NOT_ABSOLUTE",
    "CODEX_CONFIG_TIMEOUT_OVERFLOW",
    "CODEX_CONFIG_TIMEOUT_ZERO",
    "CODEX_CONFIG_VERSION_EMPTY",
    "CODEX_IDENTITY_LAUNCHER_DIGEST_INVALID",
    "CODEX_IDENTITY_LAUNCHER_PATH_INVALID",
    "CODEX_IDENTITY_SCHEMA_COUNT_OVERFLOW",
    "CODEX_IDENTITY_SCHEMA_DIGEST_INVALID",
    "CODEX_METADATA_INSPECTION_FAILED",
    "CODEX_METADATA_NOT_EMPTY",
    "CODEX_METADATA_REMOVE_FAILED",
    "CODEX_METADATA_UNSAFE",
    "CODEX_DELIVERY_DEADLINE_EXPIRED",
    "CODEX_DELIVERY_DEADLINE_INVALID",
    "CODEX_DELIVERY_EVIDENCE_INVALID",
    "CODEX_DELIVERY_NOT_ACTIVE",
    "CODEX_APP_SERVER_INVALID_LAUNCHER",
    "CODEX_APP_SERVER_INVALID_LAUNCHER_SHA256",
    "CODEX_APP_SERVER_INVALID_CODEX_HOME",
    "CODEX_APP_SERVER_CODEX_HOME_OWNERSHIP_MISSING",
    "CODEX_APP_SERVER_CODEX_HOME_OVERLAP",
    "CODEX_APP_SERVER_AMBIENT_CODEX_HOME_DENIED",
    "CODEX_APP_SERVER_INVALID_WORKING_DIRECTORY",
    "CODEX_APP_SERVER_INVALID_PROMPT",
    "CODEX_APP_SERVER_INVALID_TIMEOUT",
    "CODEX_APP_SERVER_LAUNCHER_READ_FAILED",
    "CODEX_APP_SERVER_LAUNCHER_DIGEST_MISMATCH",
    "CODEX_APP_SERVER_LAUNCHER_CHANGED",
    "CODEX_APP_SERVER_SPAWN_FAILED",
    "CODEX_APP_SERVER_PIPE_UNAVAILABLE",
    "CODEX_APP_SERVER_WRITE_FAILED",
    "CODEX_APP_SERVER_STDOUT_FAILED",
    "CODEX_APP_SERVER_STDOUT_LINE_TOO_LARGE",
    "CODEX_APP_SERVER_PROTOCOL_FAILED",
    "CODEX_APP_SERVER_CODEX_HOME_MISMATCH",
    "CODEX_APP_SERVER_TIMEOUT",
    "CODEX_APP_SERVER_AMBIGUOUS_EOF",
    "CODEX_APP_SERVER_CHILD_CLEANUP_FAILED",
    "CODEX_APP_SERVER_JOB_OBJECT_FAILED",
    "CODEX_APP_SERVER_TURN_FAILED",
    "CODEX_APP_SERVER_TURN_INTERRUPTED",
];

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
    fn known_terminal_causes_are_closed_and_distinguishable_without_details() {
        let matrix = [
            (DeliveryStage::WorkspacePrepare, "ROOT_CREATE_FAILED"),
            (DeliveryStage::Codex, "CODEX_LAUNCHER_NOT_FILE"),
            (DeliveryStage::ScopeVerification, "FOREIGN_PATH"),
            (DeliveryStage::FixedTest, "FIXED_TEST_FAILED"),
            (DeliveryStage::GitCommit, "COMMIT_EXIT_UNKNOWN"),
            (
                DeliveryStage::Outcome,
                "DELIVERY_LEDGER_COMMIT_OUTCOME_UNKNOWN",
            ),
            (
                DeliveryStage::Receipt,
                "DELIVERY_LEDGER_RETAINED_ROW_CORRUPT",
            ),
        ];

        for (stage, code) in matrix {
            let cause = terminal_cause(stage, code).expect("closed terminal cause");
            assert_eq!(cause.stage(), stage_name(stage));
            assert_eq!(cause.code(), code);
            assert!(!format!("{cause:?}").contains("secret"));
            assert!(!format!("{cause:?}").contains("C:\\"));
        }

        for &code in IDENTITY_TERMINAL_CAUSE_CODES {
            let cause = terminal_cause(DeliveryStage::Codex, code)
                .expect("every Codex identity/process leaf is closed");
            assert_eq!(cause.stage(), "CODEX");
            assert_eq!(cause.code(), code);
        }

        assert!(terminal_cause(DeliveryStage::Codex, "CODEX_TOKEN=secret").is_none());
        assert!(terminal_cause(DeliveryStage::Codex, "UNKNOWN_LEAF").is_none());
    }

    const IDENTITY_TERMINAL_CAUSE_CODES: &[&str] = &[
        "CODEX_LAUNCHER_PATH_MISMATCH",
        "CODEX_LAUNCHER_NOT_FILE",
        "CODEX_LAUNCHER_READ_FAILED",
        "CODEX_LAUNCHER_DIGEST_MISMATCH",
        "CODEX_LAUNCHER_CHANGED",
        "CODEX_VERSION_COMMAND_FAILED",
        "CODEX_VERSION_OUTPUT_INVALID",
        "CODEX_VERSION_MISMATCH",
        "CODEX_SCHEMA_OUTPUT_EXISTS",
        "CODEX_SCHEMA_GENERATION_FAILED",
        "CODEX_SCHEMA_BUNDLE_INVALID",
        "CODEX_SCHEMA_BUNDLE_EMPTY",
        "CODEX_SCHEMA_READ_FAILED",
        "CODEX_IDENTITY_TIMEOUT",
        "CODEX_IDENTITY_PROCESS_CONTAINMENT_FAILED",
        "CODEX_APP_SERVER_INVALID_LAUNCHER",
        "CODEX_APP_SERVER_INVALID_LAUNCHER_SHA256",
        "CODEX_APP_SERVER_INVALID_CODEX_HOME",
        "CODEX_APP_SERVER_CODEX_HOME_OWNERSHIP_MISSING",
        "CODEX_APP_SERVER_CODEX_HOME_OVERLAP",
        "CODEX_APP_SERVER_AMBIENT_CODEX_HOME_DENIED",
        "CODEX_APP_SERVER_INVALID_WORKING_DIRECTORY",
        "CODEX_APP_SERVER_INVALID_PROMPT",
        "CODEX_APP_SERVER_INVALID_TIMEOUT",
        "CODEX_APP_SERVER_LAUNCHER_READ_FAILED",
        "CODEX_APP_SERVER_LAUNCHER_DIGEST_MISMATCH",
        "CODEX_APP_SERVER_LAUNCHER_CHANGED",
        "CODEX_APP_SERVER_SPAWN_FAILED",
        "CODEX_APP_SERVER_PIPE_UNAVAILABLE",
        "CODEX_APP_SERVER_WRITE_FAILED",
        "CODEX_APP_SERVER_STDOUT_FAILED",
        "CODEX_APP_SERVER_STDOUT_LINE_TOO_LARGE",
        "CODEX_APP_SERVER_PROTOCOL_FAILED",
        "CODEX_APP_SERVER_CODEX_HOME_MISMATCH",
        "CODEX_APP_SERVER_TIMEOUT",
        "CODEX_APP_SERVER_AMBIGUOUS_EOF",
        "CODEX_APP_SERVER_CHILD_CLEANUP_FAILED",
        "CODEX_APP_SERVER_JOB_OBJECT_FAILED",
        "CODEX_APP_SERVER_TURN_FAILED",
        "CODEX_APP_SERVER_TURN_INTERRUPTED",
    ];

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

    #[test]
    fn graph_receipt_fields_are_flat_and_fixed() {
        let value = append_graph_receipt_fields(
            json!({"status": "COMPLETED"}),
            &GraphReceiptJsonFields {
                project_id: "task032-delivery",
                commit_sha: &"a".repeat(40),
                query_digest: &"b".repeat(64),
                analysis_digest: &"c".repeat(64),
                record_count: 7,
                persistence_digest: &"d".repeat(64),
                retrieval_digest: &"e".repeat(64),
                result_count: 2,
                receipt_digest: &"f".repeat(64),
                database_identity_digest: &"1".repeat(64),
                extension_manifest_digest: &"2".repeat(64),
            },
        )
        .expect("append fixed graph receipt fields");
        let object = value.as_object().expect("receipt object");

        assert_eq!(object.len(), 13);
        assert_eq!(
            object.get("graph_status").and_then(Value::as_str),
            Some("COMPLETED")
        );
        assert_eq!(
            object.get("graph_project_id").and_then(Value::as_str),
            Some("task032-delivery")
        );
        assert_eq!(
            object.get("graph_record_count").and_then(Value::as_u64),
            Some(7)
        );
        assert_eq!(
            object.get("graph_result_count").and_then(Value::as_u64),
            Some(2)
        );
        for name in [
            "graph_commit_sha",
            "graph_query_digest",
            "graph_analysis_digest",
            "graph_persistence_digest",
            "graph_retrieval_digest",
            "graph_receipt_digest",
            "graph_database_identity_digest",
            "graph_extension_manifest_digest",
        ] {
            assert_eq!(
                object.get(name).and_then(Value::as_str).map(str::len),
                Some(if name == "graph_commit_sha" { 40 } else { 64 })
            );
        }
    }
}
