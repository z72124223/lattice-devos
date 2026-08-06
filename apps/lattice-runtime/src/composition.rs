//! Sole concrete composition root for the bounded TASK-032 delivery lane.

use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_codebase_memory::digest_query_text;
use lattice_codex_adapter::{
    CodexDeliveryAdapter, CodexDeliveryAdapterConfig, CodexIdentityExpectation,
    PinnedCodexResourceDigests, PinnedCodexResources,
};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CompletedDeliveryEvidence, Component, ContentDigest,
    DeliveryProfile, DeliveryReceipt, DeliveryRunRequest, DeliveryRuntime, DeliveryStage,
    DeliveryTerminalStatus, GATEWAY_TASK_SPEC_SCHEMA_VERSION, GatewayActorId, GatewayActorKind,
    GatewayAdapterId, GatewayChannelId, GatewayClientKind, GatewayDenialCode, GatewayInstanceId,
    GatewayPeerContext, GatewayReply, GatewayReplyBody, GatewayRequest, GatewayRequestBody,
    GatewaySessionId, GatewayStatusObservation, GatewayStatusTarget, GatewayTaskProjection,
    GatewayTaskState, GitObjectId, GraphMemoryReceipt, GraphMemoryRunRequest, HermesEvidence,
    HermesReflectionCandidate, HermesReflectionContent, HermesReflectionFinding,
    HermesReflectionReceipt, HermesResearchRequest, Invocation, MemoryQuery, ProjectId,
    ProjectSnapshotId, RequestId, RuntimeKind, SubjectBinding, TaskId, TaskSpecSubmission,
};
use lattice_gateway_ipc::{build_reply, task_spec_document_digest};
use lattice_graphify_adapter::{
    ExactGitSnapshotMaterializer, GitSnapshotConfig, GraphOutputLimits, GraphifyRuntimeConfig,
    PinnedGraphifyAdapter, SnapshotBridge, SnapshotLimits,
};
use lattice_hermes_adapter::{
    CanonicalReflection, HermesAdapterError, HermesAdapterErrorKind, HermesReflectionJob,
    ReflectionClassification, ReflectionEvidence, ReflectionEvidenceKind,
};
#[cfg(windows)]
use lattice_hermes_adapter::{
    CodexReflectionBrokerConfig, HermesOfflineRuntimeManifest, HermesProductionRunnerConfig,
    HermesWslContainmentConfig, ProductionHermesPort as HermesAdapterProductionPort,
    ProductionHermesRunner,
};
use lattice_openclaw_adapter::{
    AuthenticationKey, GatewayTransportErrorKind, OpenClawGatewayConfig, OpenClawGatewayServer,
    OpenClawLaunchAttestationKey, OpenClawLaunchAttestationTag, OpenClawOfficialLaunchEvidence,
    OpenClawOfficialLaunchRecord, OpenClawProcessStartNonce,
};
use lattice_orchestrator::{
    DeliveryOrchestratorError, delivery_status, graph_memory_status, run_delivery, run_graph_memory,
};
use lattice_ports::{
    DeliveryFailureCertainty, GatewayService, GatewayServiceError, GatewayServiceResult,
    GraphMemoryFailureCertainty, GraphMemoryPortError, GraphMemoryStage, HermesPort,
    HermesReflectionMemoryPort, PortError, PortErrorKind, PortResult,
};
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
use crate::mcp::{self, DeliveryToolArguments, DeliveryToolService, ToolExecutionError};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_TIMEOUT_SECONDS: u64 = 3_600;
const FINALIZATION_RESERVE: Duration = Duration::from_secs(30);
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
const FULL_CHAIN_HERMES_TASK_ID: &str = "TASK-037";
#[cfg(windows)]
const FULL_CHAIN_HERMES_MODEL: &str = "hermes-agent";
#[cfg(windows)]
const FULL_CHAIN_CODEX_BROKER_MODEL: &str = "gpt-5.6-sol";
#[cfg(windows)]
const FULL_CHAIN_HERMES_SESSION_PREFIX: &str = "task037-hermes-session-";
#[cfg(windows)]
const MAX_HERMES_RUNTIME_MANIFEST_BYTES: u64 = 64 * 1024;
#[cfg(windows)]
const HERMES_OPERATION_TIMEOUT: Duration = Duration::from_mins(1);
#[cfg(windows)]
const HERMES_POLL_INTERVAL: Duration = Duration::from_millis(250);
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_GUEST_ROOT: &str = concat!(
    "/var/tmp/lattice-runtime-targets/",
    "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-offline-final-2UEmH84h"
);
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_MANIFEST_SHA256: &str =
    "51e7c972ffebb00ed0e09e688b7d8490853764295a3c28237dd198950f7524ab";
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_TREE_SHA256: &str =
    "3159e079402fad16348d5ee5be97f84b2bb8f2bf1fa4efaf65dfc73506918e4d";
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_FILE_COUNT: u64 = 14_076;
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_BYTE_COUNT: u64 = 722_642_720;
const OPENCLAW_ADAPTER_VERSION: &str = "1.0.0";
const FIXED_GATEWAY_TASK_REVISION: &str = "1";
const SCRIPTED_SERVER_BYTES: &[u8] = include_bytes!("fixtures/task032-scripted-codex.ps1");
const OFFICIAL_CODEX_VERSION: &str = "codex-cli 0.146.0";
const OFFICIAL_CODEX_LAUNCHER_SHA256: &str =
    "bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb";
const OFFICIAL_SANDBOX_SETUP_SHA256: &str =
    "c12d225b34e7f82cdab6bbc714797abed661f40e158104694953889750121cef";
const OFFICIAL_COMMAND_RUNNER_SHA256: &str =
    "0102fa1820ecd03bb03a991fd2303a1a484118f7da8a71864f88ec94bca61d6d";
const OFFICIAL_CODE_MODE_HOST_SHA256: &str =
    "6ef1de0e04d859f8f4f6d4d64f0f3ceeec28658423d91de160f5e804280d1c36";
const OFFICIAL_RG_SHA256: &str = "14231169855ec5205cf5a1b6f1db358ff4aed4247c86b69ce8aae647c77f6680";
const OFFICIAL_PACKAGE_MANIFEST_SHA256: &str =
    "aaa0646d6b615da94187b51efd50c69621a00867761161ae55cc16cfd545bec7";
const OFFICIAL_MANAGED_PACKAGE_MANIFEST_SHA256: &str =
    "24dd8c63a4d2b7bc2ded86c887974f842093ce4f2ed8473267a91e036c38da20";
const MAX_OFFICIAL_LAUNCHER_BYTES: u64 = 512 * 1024 * 1024;
const MAX_OFFICIAL_RESOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_OFFICIAL_MANIFEST_BYTES: u64 = 64 * 1024;

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
    GraphConfiguration,
    GraphExecution,
    GraphReceiptRead,
    HermesProductionRunnerRequired,
    HermesExecution,
    HermesReceiptRead,
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
            Self::OfficialLiveBlocked => "LATTICE_OFFICIAL_CODEX_IDENTITY_REJECTED",
            Self::ScriptedFixtureRejected => "LATTICE_SCRIPTED_FIXTURE_REJECTED",
            Self::GraphConfiguration => "LATTICE_GRAPH_MEMORY_CONFIGURATION_REJECTED",
            Self::GraphExecution => "LATTICE_GRAPH_MEMORY_RUN_REJECTED",
            Self::GraphReceiptRead => "LATTICE_GRAPH_MEMORY_RECEIPT_REJECTED",
            Self::HermesProductionRunnerRequired => "LATTICE_HERMES_PRODUCTION_RUNNER_REQUIRED",
            Self::HermesExecution => "LATTICE_HERMES_REFLECTION_REJECTED",
            Self::HermesReceiptRead => "LATTICE_HERMES_MEMORY_RECEIPT_REJECTED",
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
    pinned_codex_resources: Option<PinnedCodexResources>,
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
        let pinned_codex_resources = if runtime == DeliveryRuntime::OfficialCodexAppServer {
            Some(validate_official_codex_identity(
                &launcher,
                &version,
                &launcher_sha256,
                &delivery_root,
            )?)
        } else {
            None
        };
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
            pinned_codex_resources.clone(),
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
            pinned_codex_resources,
        })
    }
}

fn validate_official_codex_identity(
    launcher: &Path,
    version: &str,
    launcher_sha256: &str,
    delivery_root: &Path,
) -> Result<PinnedCodexResources, LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::OfficialLiveBlocked);
    if version != OFFICIAL_CODEX_VERSION
        || launcher_sha256 != OFFICIAL_CODEX_LAUNCHER_SHA256
        || delivery_root.file_name() != Some(OsStr::new("delivery"))
    {
        return Err(rejected());
    }
    let fixture_root = delivery_root.parent().ok_or_else(rejected)?;
    let fixture_id = fixture_root
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(rejected)?;
    if !is_lower_hex(fixture_id, 32) {
        return Err(rejected());
    }
    let lattice_delivery_root = fixture_root.parent().ok_or_else(rejected)?;
    if lattice_delivery_root.file_name() != Some(OsStr::new("lattice-delivery")) {
        return Err(rejected());
    }
    let target_root = lattice_delivery_root.parent().ok_or_else(rejected)?;
    if target_root.file_name() != Some(OsStr::new("target")) {
        return Err(rejected());
    }
    let install_root = target_root.join("codex-official").join("0.146.0");
    let managed_package_root = install_root
        .join("node_modules")
        .join("@openai")
        .join("codex");
    let managed_package_manifest = managed_package_root.join("package.json");
    let expected_launcher = install_root
        .join("node_modules")
        .join("@openai")
        .join("codex-win32-x64")
        .join("vendor")
        .join("x86_64-pc-windows-msvc")
        .join("bin")
        .join("codex.exe");
    let bundle_root = expected_launcher
        .parent()
        .and_then(Path::parent)
        .ok_or_else(rejected)?;
    let sandbox_setup = bundle_root
        .join("codex-resources")
        .join("codex-windows-sandbox-setup.exe");
    let command_runner = bundle_root
        .join("codex-resources")
        .join("codex-command-runner.exe");
    let code_mode_host = bundle_root.join("bin").join("codex-code-mode-host.exe");
    let rg = bundle_root.join("codex-path").join("rg.exe");
    let package_manifest = bundle_root.join("codex-package.json");
    if !same_declared_path(launcher, &expected_launcher) {
        return Err(rejected());
    }
    for path in [
        expected_launcher.as_path(),
        sandbox_setup.as_path(),
        command_runner.as_path(),
        code_mode_host.as_path(),
        rg.as_path(),
        package_manifest.as_path(),
        managed_package_manifest.as_path(),
    ] {
        reject_reparse_path(path, target_root)?;
    }
    let canonical_expected = fs::canonicalize(&expected_launcher).map_err(|_| rejected())?;
    let canonical_launcher = fs::canonicalize(launcher).map_err(|_| rejected())?;
    if canonical_launcher != canonical_expected
        || official_file_sha256(&canonical_launcher, MAX_OFFICIAL_LAUNCHER_BYTES)?
            != OFFICIAL_CODEX_LAUNCHER_SHA256
        || official_file_sha256(&sandbox_setup, MAX_OFFICIAL_RESOURCE_BYTES)?
            != OFFICIAL_SANDBOX_SETUP_SHA256
        || official_file_sha256(&command_runner, MAX_OFFICIAL_RESOURCE_BYTES)?
            != OFFICIAL_COMMAND_RUNNER_SHA256
        || official_file_sha256(&code_mode_host, MAX_OFFICIAL_RESOURCE_BYTES)?
            != OFFICIAL_CODE_MODE_HOST_SHA256
        || official_file_sha256(&rg, MAX_OFFICIAL_RESOURCE_BYTES)? != OFFICIAL_RG_SHA256
        || official_file_sha256(&package_manifest, MAX_OFFICIAL_MANIFEST_BYTES)?
            != OFFICIAL_PACKAGE_MANIFEST_SHA256
        || official_file_sha256(&managed_package_manifest, MAX_OFFICIAL_MANIFEST_BYTES)?
            != OFFICIAL_MANAGED_PACKAGE_MANIFEST_SHA256
    {
        return Err(rejected());
    }
    PinnedCodexResources::new(
        managed_package_root,
        bundle_root.join("codex-resources"),
        PinnedCodexResourceDigests::new(
            OFFICIAL_SANDBOX_SETUP_SHA256,
            OFFICIAL_COMMAND_RUNNER_SHA256,
            OFFICIAL_CODE_MODE_HOST_SHA256,
            OFFICIAL_RG_SHA256,
            OFFICIAL_PACKAGE_MANIFEST_SHA256,
            OFFICIAL_MANAGED_PACKAGE_MANIFEST_SHA256,
        )
        .map_err(|_| rejected())?,
    )
    .map_err(|_| rejected())
}

#[cfg(windows)]
fn same_declared_path(actual: &Path, expected: &Path) -> bool {
    actual
        .as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy())
}

#[cfg(not(windows))]
fn same_declared_path(actual: &Path, expected: &Path) -> bool {
    actual == expected
}

fn reject_reparse_path(path: &Path, boundary: &Path) -> Result<(), LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::OfficialLiveBlocked);
    let mut current = path;
    loop {
        let metadata = fs::symlink_metadata(current).map_err(|_| rejected())?;
        if metadata_is_reparse(&metadata) {
            return Err(rejected());
        }
        if current == boundary {
            return Ok(());
        }
        current = current.parent().ok_or_else(rejected)?;
    }
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn official_file_sha256(path: &Path, max_bytes: u64) -> Result<String, LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::OfficialLiveBlocked);
    let metadata = fs::symlink_metadata(path).map_err(|_| rejected())?;
    if !metadata.file_type().is_file() || metadata.len() > max_bytes {
        return Err(rejected());
    }
    let mut file = fs::File::open(path).map_err(|_| rejected())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file.read(&mut buffer).map_err(|_| rejected())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| rejected())?;
    }
    Ok(output)
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
        let (config, database, password) = delivery_environment()?;
        Self::for_delivery(config, database, password)
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
        let scripted_graph_paths = if config.runtime == DeliveryRuntime::ScriptedAcceptance {
            Some(validate_scripted_fixture(config)?)
        } else {
            None
        };
        let finalization_deadline = deadline(self.timeout)?;
        let effect_deadline = effect_deadline(finalization_deadline)?;
        let ledger = DeliveryLedger::connect(&self.database, &self.password, finalization_deadline)
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
            DeliveryWorkspaceGitAdapter::with_deadline(workspace_config, effect_deadline);
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
            config.pinned_codex_resources.clone(),
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::CodexConfiguration))?;
        let mut codex = CodexDeliveryAdapter::with_deadline(codex_config, effect_deadline);
        match run_delivery(request, &mut ledger, &mut workspace_git, &mut codex) {
            Ok(receipt) => {
                let graph_paths = match scripted_graph_paths {
                    Some(paths) => paths,
                    None => official_graph_paths(config)?,
                };
                let graph_receipt = run_delivery_graph_memory(
                    &self.database,
                    &self.password,
                    config,
                    &graph_paths,
                    finalization_deadline,
                    &receipt,
                )?;
                composed_receipt_json(&receipt, "lattice-delivery", &graph_receipt)
            }
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
    fn run(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        validate_mcp_task_binding(arguments)?;
        self.run_json()
            .map_err(|error| ToolExecutionError::new(error.code()))
    }

    fn status(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        validate_mcp_task_binding(arguments)?;
        self.status_json()
            .map_err(|error| ToolExecutionError::new(error.code()))
    }
}

fn delivery_environment()
-> Result<(LatticedDeliveryConfig, DeliveryDatabaseBinding, String), LatticedError> {
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
    Ok((
        config,
        database,
        required_environment("LATTICE_TASK019_PASSWORD")?,
    ))
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

/// One live Hermes result carrying both normalized evidence and persistable content.
pub struct ProductionHermesOutput {
    evidence: HermesEvidence,
    candidate: HermesReflectionCandidate,
}

/// Opaque proof issued only by the composition-owned verified Hermes runner wrapper.
///
/// There is deliberately no public constructor. A containment canary, endpoint
/// probe, or adapter-reported `RuntimeKind::Live` cannot mint this value.
struct HermesProductionSeal {
    receipt_digest: ContentDigest,
}

mod production_hermes_sealed {
    pub trait Sealed {
        fn has_production_seal(&self) -> bool;
    }
}

impl ProductionHermesOutput {
    /// Validates a canonical reflection against the exact request and graph receipt.
    ///
    /// # Errors
    ///
    /// Rejects fake runtime evidence or any request, graph, or reflection substitution.
    fn new(
        _seal: &HermesProductionSeal,
        request: &HermesResearchRequest,
        graph_request: &GraphMemoryRunRequest,
        graph_receipt: &GraphMemoryReceipt,
        evidence: HermesEvidence,
        candidate: HermesReflectionCandidate,
    ) -> PortResult<Self> {
        if evidence.runtime() != RuntimeKind::Live
            || evidence.invocation() != request.invocation()
            || evidence.output_digest() != candidate.reflection_digest()
            || !candidate.matches_request(graph_request)
            || candidate.graph_receipt_digest() != graph_receipt.receipt_digest()
        {
            return Err(PortError::new(
                Component::Hermes,
                PortErrorKind::Denied,
                "HERMES_PRODUCTION_REFLECTION_BINDING_REJECTED",
            ));
        }
        Ok(Self {
            evidence,
            candidate,
        })
    }

    fn into_candidate(self) -> HermesReflectionCandidate {
        let Self {
            evidence,
            candidate,
        } = self;
        debug_assert_eq!(evidence.output_digest(), candidate.reflection_digest());
        candidate
    }
}

/// Injectable Hermes boundary required by the production full-chain coordinator.
///
/// Implementations must expose a live preflight classification before any Codex
/// effect and return both [`HermesEvidence`] and bounded canonical reflection content.
pub trait FullChainHermesPort: HermesPort + Send + production_hermes_sealed::Sealed {
    /// Reports the verified runtime classification for this configured port.
    fn runtime_kind(&self) -> RuntimeKind;

    /// Produces one exact-graph-bound canonical reflection.
    ///
    /// # Errors
    ///
    /// Returns a bounded port failure when live evidence or canonical content is absent.
    fn research_canonical(
        &mut self,
        request: &HermesResearchRequest,
        graph_request: &GraphMemoryRunRequest,
        graph_receipt: &GraphMemoryReceipt,
    ) -> PortResult<ProductionHermesOutput>;
}

#[cfg(windows)]
struct FullChainHermes {
    ready: Option<ProductionHermesRunner>,
    bound: Option<HermesAdapterProductionPort>,
    model: String,
    session_id: String,
    seal: HermesProductionSeal,
}

#[cfg(windows)]
impl FullChainHermes {
    fn from_ready(mut runner: ProductionHermesRunner, run_id: &str) -> Result<Self, LatticedError> {
        runner
            .verify_live()
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let receipt_digest = runner.containment_receipt().receipt_digest().clone();
        Ok(Self {
            ready: Some(runner),
            bound: None,
            model: FULL_CHAIN_HERMES_MODEL.to_owned(),
            session_id: format!("{FULL_CHAIN_HERMES_SESSION_PREFIX}{run_id}"),
            seal: HermesProductionSeal { receipt_digest },
        })
    }
}

#[cfg(windows)]
impl production_hermes_sealed::Sealed for FullChainHermes {
    fn has_production_seal(&self) -> bool {
        self.ready.is_some()
    }
}

#[cfg(windows)]
impl HermesPort for FullChainHermes {
    fn research(&mut self, _request: HermesResearchRequest) -> PortResult<HermesEvidence> {
        Err(PortError::new(
            Component::Hermes,
            PortErrorKind::Denied,
            "HERMES_PRODUCTION_GRAPH_CONTEXT_REQUIRED",
        ))
    }

    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()> {
        self.bound.as_mut().map_or_else(
            || {
                Err(PortError::new(
                    Component::Hermes,
                    PortErrorKind::Denied,
                    "HERMES_PRODUCTION_RUN_NOT_BOUND",
                ))
            },
            |port| HermesPort::interrupt(port, request_id),
        )
    }
}

#[cfg(windows)]
impl FullChainHermesPort for FullChainHermes {
    fn runtime_kind(&self) -> RuntimeKind {
        RuntimeKind::Live
    }

    fn research_canonical(
        &mut self,
        request: &HermesResearchRequest,
        graph_request: &GraphMemoryRunRequest,
        graph_receipt: &GraphMemoryReceipt,
    ) -> PortResult<ProductionHermesOutput> {
        if self.bound.is_some() {
            return Err(PortError::new(
                Component::Hermes,
                PortErrorKind::Ambiguous,
                "HERMES_PRODUCTION_ALREADY_BOUND",
            ));
        }
        let job = HermesReflectionJob::new(
            request.clone(),
            self.session_id.clone(),
            self.model.clone(),
            hermes_job_evidence(graph_request, graph_receipt)?,
        )
        .map_err(|failure| map_hermes_adapter_error(&failure))?;
        let input_digest = job.input_digest().clone();
        let runner = self.ready.take().ok_or_else(|| {
            PortError::new(
                Component::Hermes,
                PortErrorKind::Denied,
                "HERMES_PRODUCTION_RUNNER_REQUIRED",
            )
        })?;
        let port = runner
            .bind(job)
            .map_err(|failure| map_hermes_adapter_error(&failure))?;
        if port.containment_receipt().receipt_digest() != &self.seal.receipt_digest {
            return Err(hermes_port_error(
                PortErrorKind::Denied,
                "HERMES_PRODUCTION_IDENTITY_BINDING_REJECTED",
            ));
        }
        self.bound = Some(port);
        let output = self
            .bound
            .as_mut()
            .expect("bound port installed immediately above")
            .run_reflection_evidence(request)?;
        let (reflection, evidence) = output.into_parts();
        let candidate = reflection_candidate(
            request,
            graph_request,
            graph_receipt,
            &self.session_id,
            &self.model,
            &input_digest,
            &self.seal.receipt_digest,
            &reflection,
            &evidence,
        )?;
        ProductionHermesOutput::new(
            &self.seal,
            request,
            graph_request,
            graph_receipt,
            evidence,
            candidate,
        )
    }
}

#[cfg(windows)]
struct HermesEnvironmentConfig {
    containment: HermesWslContainmentConfig,
    runtime_manifest: HermesOfflineRuntimeManifest,
    broker: CodexReflectionBrokerConfig,
    api_key: String,
    timeout: Duration,
}

#[cfg(windows)]
impl HermesEnvironmentConfig {
    fn from_environment() -> Result<Self, LatticedError> {
        let runtime_manifest_path =
            PathBuf::from(hermes_environment("LATTICE_HERMES_RUNTIME_MANIFEST")?);
        let runtime_manifest_bytes =
            read_regular_file(&runtime_manifest_path, MAX_HERMES_RUNTIME_MANIFEST_BYTES).map_err(
                |_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired),
            )?;
        let runtime_manifest = HermesOfflineRuntimeManifest::from_canonical_json(
            &runtime_manifest_bytes,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let runtime_guest_root = hermes_environment("LATTICE_HERMES_RUNTIME_GUEST_ROOT")?;
        validate_official_hermes_runtime_identity(
            &runtime_guest_root,
            &runtime_manifest_bytes,
            &runtime_manifest,
        )?;
        let api_key = hermes_environment("LATTICE_HERMES_API_KEY")?;
        validate_hermes_api_key(&api_key)?;
        let product_root = PathBuf::from(hermes_environment("LATTICE_HERMES_PRODUCT_ROOT")?);
        let containment = HermesWslContainmentConfig::new(
            PathBuf::from(hermes_environment("LATTICE_HERMES_WSL_EXE")?),
            runtime_guest_root,
            PathBuf::from(hermes_environment("LATTICE_HERMES_ISOLATION_ROOT")?),
            product_root.clone(),
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let broker = CodexReflectionBrokerConfig::new(
            PathBuf::from(hermes_environment("LATTICE_HERMES_BROKER_HELPER")?),
            hermes_environment("LATTICE_HERMES_BROKER_HELPER_SHA256")?,
            PathBuf::from(hermes_environment("LATTICE_HERMES_CODEX_LAUNCHER")?),
            PathBuf::from(hermes_environment("LATTICE_HERMES_CODEX_HOME")?),
            PathBuf::from(hermes_environment("LATTICE_HERMES_BROKER_ISOLATION_ROOT")?),
            product_root,
            FULL_CHAIN_CODEX_BROKER_MODEL,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let timeout_seconds = hermes_environment("LATTICE_HERMES_DEADLINE_SECONDS")?
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=300).contains(seconds))
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        Ok(Self {
            containment,
            runtime_manifest,
            broker,
            api_key,
            timeout: Duration::from_secs(timeout_seconds),
        })
    }

    fn launch(self, run_id: &str) -> Result<FullChainHermes, LatticedError> {
        let absolute_deadline = Instant::now()
            .checked_add(self.timeout)
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let broker_receipt = self
            .broker
            .run_zero_model_preflight(absolute_deadline)
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let runner = HermesProductionRunnerConfig::new(
            self.containment,
            &self.runtime_manifest,
            self.broker,
            &broker_receipt,
            self.api_key,
            FULL_CHAIN_HERMES_MODEL,
            HERMES_OPERATION_TIMEOUT.min(self.timeout),
            self.timeout,
            HERMES_POLL_INTERVAL,
        )
        .and_then(|config| config.launch(absolute_deadline))
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        FullChainHermes::from_ready(runner, run_id)
    }
}

#[cfg(windows)]
fn validate_official_hermes_runtime_identity(
    runtime_guest_root: &str,
    runtime_manifest_bytes: &[u8],
    runtime_manifest: &HermesOfflineRuntimeManifest,
) -> Result<(), LatticedError> {
    let digest = Sha256::digest(runtime_manifest_bytes);
    let mut manifest_sha256 = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut manifest_sha256, "{byte:02x}")
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
    }
    if runtime_guest_root != OFFICIAL_HERMES_RUNTIME_GUEST_ROOT
        || manifest_sha256 != OFFICIAL_HERMES_RUNTIME_MANIFEST_SHA256
        || runtime_manifest.payload_file_count() != OFFICIAL_HERMES_RUNTIME_FILE_COUNT
        || runtime_manifest.payload_byte_count() != OFFICIAL_HERMES_RUNTIME_BYTE_COUNT
        || runtime_manifest.payload_manifest_sha256() != OFFICIAL_HERMES_RUNTIME_TREE_SHA256
    {
        return Err(LatticedError::new(
            LatticedErrorKind::HermesProductionRunnerRequired,
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_hermes_api_key(api_key: &str) -> Result<(), LatticedError> {
    if api_key.trim().is_empty()
        || api_key.len() < 16
        || api_key.len() > 4_096
        || !api_key.is_ascii()
        || api_key.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(LatticedError::new(
            LatticedErrorKind::HermesProductionRunnerRequired,
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn hermes_environment(name: &'static str) -> Result<String, LatticedError> {
    required_environment(name)
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))
}

#[cfg(windows)]
struct OpenClawEnvironmentLaunch {
    authentication_key: AuthenticationKey,
    launch_record: OpenClawOfficialLaunchRecord,
    launch_record_id: String,
    process_id: u32,
    process_nonce_hex: String,
}

#[cfg(windows)]
fn openclaw_launch_from_environment() -> Result<OpenClawEnvironmentLaunch, LatticedError> {
    let authentication_key = AuthenticationKey::new(parse_lowercase_hex_environment::<32>(
        "LATTICE_OPENCLAW_AUTH_KEY_HEX",
    )?)
    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let attestation_key = OpenClawLaunchAttestationKey::new(parse_lowercase_hex_environment::<32>(
        "LATTICE_OPENCLAW_LAUNCH_ATTESTATION_KEY_HEX",
    )?)
    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let attestation_tag = OpenClawLaunchAttestationTag::new(parse_lowercase_hex_environment::<32>(
        "LATTICE_OPENCLAW_LAUNCH_ATTESTATION_TAG_HEX",
    )?)
    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let process_nonce_hex = required_environment("LATTICE_OPENCLAW_PROCESS_START_NONCE")?;
    let process_nonce =
        OpenClawProcessStartNonce::new(parse_lowercase_hex::<16>(&process_nonce_hex)?)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let launch_record_id = required_environment("LATTICE_OPENCLAW_LAUNCH_RECORD_ID")?;
    let process_id = required_environment("LATTICE_OPENCLAW_PROCESS_ID")?
        .parse::<u32>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Transport))?;
    let launch_evidence = OpenClawOfficialLaunchEvidence::new(
        launch_record_id.clone(),
        process_id,
        process_nonce,
        content_digest_environment("LATTICE_OPENCLAW_PACKAGE_TARBALL_SHA256")?,
        content_digest_environment("LATTICE_OPENCLAW_ENTRYPOINT_SHA256")?,
        content_digest_environment("LATTICE_OPENCLAW_PROFILE_SHA256")?,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let launch_record = OpenClawOfficialLaunchRecord::verify_lattice_attestation(
        launch_evidence,
        &attestation_key,
        attestation_tag,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    Ok(OpenClawEnvironmentLaunch {
        authentication_key,
        launch_record,
        launch_record_id,
        process_id,
        process_nonce_hex,
    })
}

#[cfg(windows)]
fn openclaw_from_environment()
-> Result<(OpenClawGatewayConfig, OpenClawOfficialLaunchRecord), LatticedError> {
    let OpenClawEnvironmentLaunch {
        authentication_key,
        launch_record,
        launch_record_id,
        process_id,
        process_nonce_hex,
    } = openclaw_launch_from_environment()?;
    let session_epoch = required_environment("LATTICE_OPENCLAW_SESSION_EPOCH")?
        .parse::<u64>()
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Transport))?;
    let session_receipt = digest(
        "lattice.openclaw.full-chain-session",
        &CanonicalValue::Object(vec![
            (
                "launch_record_id".to_owned(),
                CanonicalValue::String(launch_record_id),
            ),
            (
                "process_id".to_owned(),
                CanonicalValue::String(process_id.to_string()),
            ),
            (
                "process_start_nonce".to_owned(),
                CanonicalValue::String(process_nonce_hex),
            ),
            (
                "session_epoch".to_owned(),
                CanonicalValue::String(session_epoch.to_string()),
            ),
        ]),
    )?;
    let schema_digest = digest(
        "lattice.openclaw.full-chain-schema",
        &CanonicalValue::String("LATGW001".to_owned()),
    )?;
    let peer = GatewayPeerContext::new_fake(
        GatewayClientKind::OpenClaw,
        GatewayInstanceId::new(required_environment(
            "LATTICE_OPENCLAW_GATEWAY_INSTANCE_ID",
        )?)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?,
        GatewayAdapterId::new("openclaw-lattice-plugin")
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?,
        OPENCLAW_ADAPTER_VERSION,
        launch_record.entrypoint_digest().clone(),
        schema_digest,
        GatewayActorId::new(required_environment("LATTICE_OPENCLAW_ACTOR_ID")?)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?,
        GatewayActorKind::ResponsibleUser,
        GatewayChannelId::new(required_environment("LATTICE_OPENCLAW_CHANNEL_ID")?)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?,
        GatewaySessionId::new(required_environment("LATTICE_OPENCLAW_SESSION_ID")?)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?,
        session_epoch,
        session_receipt.clone(),
        session_receipt,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let port = required_environment("LATTICE_OPENCLAW_GATEWAY_PORT")?
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Transport))?;
    let timeout_millis = required_environment("LATTICE_OPENCLAW_DEADLINE_MS")?
        .parse::<u64>()
        .ok()
        .filter(|millis| (1..=30_000).contains(millis))
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Transport))?;
    let config = OpenClawGatewayConfig::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        Duration::from_millis(timeout_millis),
        ProjectId::new(GRAPH_PROJECT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?,
        peer,
        authentication_key,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    Ok((config, launch_record))
}

#[cfg(windows)]
fn parse_lowercase_hex_environment<const N: usize>(
    name: &'static str,
) -> Result<[u8; N], LatticedError> {
    parse_lowercase_hex(&required_environment(name)?)
}

#[cfg(windows)]
fn parse_lowercase_hex<const N: usize>(value: &str) -> Result<[u8; N], LatticedError> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(LatticedError::new(LatticedErrorKind::Transport));
    }
    let mut bytes = [0_u8; N];
    for (index, output) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *output = u8::from_str_radix(&value[start..start + 2], 16)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    }
    Ok(bytes)
}

#[cfg(windows)]
fn content_digest_environment(name: &'static str) -> Result<ContentDigest, LatticedError> {
    ContentDigest::from_sha256(required_environment(name)?)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FullChainEntry {
    CodexAppMcp,
    OpenClawTyped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FullChainRunMode {
    Fresh,
    ResumeExisting,
}

fn parse_full_chain_run_mode(value: Option<&str>) -> Result<FullChainRunMode, LatticedError> {
    match value {
        None | Some("FRESH") => Ok(FullChainRunMode::Fresh),
        Some("RESUME_EXISTING") => Ok(FullChainRunMode::ResumeExisting),
        Some(_) => Err(LatticedError::new(LatticedErrorKind::Configuration)),
    }
}

fn full_chain_run_mode_from_environment() -> Result<FullChainRunMode, LatticedError> {
    match env::var("LATTICE_FULL_CHAIN_RUN_MODE") {
        Ok(value) => parse_full_chain_run_mode(Some(&value)),
        Err(env::VarError::NotPresent) => parse_full_chain_run_mode(None),
        Err(env::VarError::NotUnicode(_)) => {
            Err(LatticedError::new(LatticedErrorKind::Configuration))
        }
    }
}

impl FullChainEntry {
    const fn name(self) -> &'static str {
        match self {
            Self::CodexAppMcp => "codex-app-mcp",
            Self::OpenClawTyped => "openclaw-typed",
        }
    }

    const fn classification(self) -> &'static str {
        match self {
            Self::CodexAppMcp => "official-codex-app-live",
            Self::OpenClawTyped => "official-package-preflight-only",
        }
    }

    const fn runtime_kind(self) -> &'static str {
        match self {
            Self::CodexAppMcp => "Live",
            Self::OpenClawTyped => "Fake",
        }
    }
}

struct FullChainCore<H> {
    delivery: LatticedDeliveryService,
    hermes: H,
    submission: TaskSpecSubmission,
    run_mode: FullChainRunMode,
}

impl<H: FullChainHermesPort> FullChainCore<H> {
    fn run_json(&mut self, entry: FullChainEntry) -> Result<Value, LatticedError> {
        let base = match self.run_mode {
            FullChainRunMode::Fresh => self.delivery.run_json()?,
            FullChainRunMode::ResumeExisting => self.delivery.status_json()?,
        };
        let reflection = self.load_or_run_reflection(&base)?;
        append_full_chain_json(base, &reflection, entry)
    }

    fn status_json(&mut self, entry: FullChainEntry) -> Result<Value, LatticedError> {
        let base = self.delivery.status_json()?;
        let request = graph_request_from_json(self.delivery.database.run_id(), &base)?;
        let reflection = load_reflection_from_postgres(
            &self.delivery.database,
            &self.delivery.password,
            self.delivery.timeout,
            &request,
        )
        .map_err(|error| map_reflection_read_error(&error))?;
        append_full_chain_json(base, &reflection, entry)
    }

    fn load_or_run_reflection(
        &mut self,
        base: &Value,
    ) -> Result<HermesReflectionReceipt, LatticedError> {
        let request = graph_request_from_json(self.delivery.database.run_id(), base)?;
        match load_reflection_from_postgres(
            &self.delivery.database,
            &self.delivery.password,
            self.delivery.timeout,
            &request,
        ) {
            Ok(receipt) => return Ok(receipt),
            Err(error)
                if error.kind() == PortErrorKind::Unavailable
                    && error.code() == "MEMORY_RECEIPT_UNAVAILABLE" => {}
            Err(error) => return Err(map_reflection_read_error(&error)),
        }

        let graph_receipt = load_delivery_graph_receipt(
            &self.delivery.database,
            &self.delivery.password,
            deadline(self.delivery.timeout)?,
            &request,
        )?;
        let hermes_request =
            hermes_request_for_graph(self.delivery.database.run_id(), &request, &graph_receipt)?;
        let output = self
            .hermes
            .research_canonical(&hermes_request, &request, &graph_receipt)
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesExecution))?;
        let candidate = output.into_candidate();
        let persisted = persist_reflection_to_postgres(
            &self.delivery.database,
            &self.delivery.password,
            self.delivery.timeout,
            &candidate,
        )?;
        let replayed = load_reflection_from_postgres(
            &self.delivery.database,
            &self.delivery.password,
            self.delivery.timeout,
            &request,
        )
        .map_err(|error| map_reflection_read_error(&error))?;
        if replayed != persisted {
            return Err(LatticedError::new(LatticedErrorKind::HermesReceiptRead));
        }
        Ok(replayed)
    }
}

/// Shared service used by both typed MCP tools and typed `OpenClaw` ingress.
pub struct FullChainService<H> {
    inner: Arc<Mutex<FullChainCore<H>>>,
}

impl<H> Clone for FullChainService<H> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<H: FullChainHermesPort> FullChainService<H> {
    fn handle_submit(
        core: &mut FullChainCore<H>,
        request: &GatewayRequest,
        submission: &TaskSpecSubmission,
    ) -> GatewayServiceResult<GatewayReply> {
        if submission != &core.submission {
            return gateway_reply(
                request,
                GatewayReplyBody::Denied(GatewayDenialCode::CommandSubstitution),
            );
        }
        let result = core
            .run_json(FullChainEntry::OpenClawTyped)
            .map_err(map_gateway_service_error)?;
        let receipt_digest =
            full_chain_receipt_digest(&result).map_err(map_gateway_service_error)?;
        gateway_reply(
            request,
            GatewayReplyBody::SubmitAccepted {
                binding: core.submission.binding().clone(),
                command_receipt_digest: receipt_digest,
            },
        )
    }

    fn handle_status(
        core: &mut FullChainCore<H>,
        request: &GatewayRequest,
        target: GatewayStatusTarget,
    ) -> GatewayServiceResult<GatewayReply> {
        let fixed_binding = core.submission.binding().clone();
        match &target {
            GatewayStatusTarget::Project(project)
                if project.project_id() != fixed_binding.project_id()
                    || project.cursor().is_some() =>
            {
                return gateway_reply(
                    request,
                    GatewayReplyBody::Denied(GatewayDenialCode::ScopeDenied),
                );
            }
            GatewayStatusTarget::Task(task) if task.binding() != &fixed_binding => {
                return gateway_reply(
                    request,
                    GatewayReplyBody::Denied(GatewayDenialCode::CommandSubstitution),
                );
            }
            GatewayStatusTarget::Command { .. } => {
                return gateway_reply(
                    request,
                    GatewayReplyBody::Denied(GatewayDenialCode::DownstreamDenied),
                );
            }
            GatewayStatusTarget::Project(_) | GatewayStatusTarget::Task(_) => {}
        }
        let result = core
            .status_json(FullChainEntry::OpenClawTyped)
            .map_err(map_gateway_service_error)?;
        let receipt_digest =
            full_chain_receipt_digest(&result).map_err(map_gateway_service_error)?;
        if matches!(
            &target,
            GatewayStatusTarget::Task(task)
                if task.expected_ledger_head_digest() != &receipt_digest
        ) {
            return gateway_reply(
                request,
                GatewayReplyBody::Denied(GatewayDenialCode::CommandSubstitution),
            );
        }
        let projection = GatewayTaskProjection::new(
            fixed_binding,
            GatewayTaskState::Completed,
            receipt_digest.clone(),
            receipt_digest,
        )
        .map_err(|_| {
            GatewayServiceError::new(
                PortErrorKind::Malformed,
                "FULL_CHAIN_STATUS_PROJECTION_REJECTED",
            )
        })?;
        let observation = match target {
            GatewayStatusTarget::Project(project) => GatewayStatusObservation::Project {
                project_id: project.project_id().clone(),
                tasks: vec![projection],
                next_cursor: None,
            },
            GatewayStatusTarget::Task(_) => GatewayStatusObservation::Task(projection),
            GatewayStatusTarget::Command { .. } => unreachable!("handled above"),
        };
        gateway_reply(request, GatewayReplyBody::StatusObserved(observation))
    }
}

impl<H: FullChainHermesPort> DeliveryToolService for FullChainService<H> {
    fn run(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        let mut core = self
            .inner
            .lock()
            .map_err(|_| ToolExecutionError::new(LatticedErrorKind::Transport.code()))?;
        if arguments.binding() != core.submission.binding() {
            return Err(ToolExecutionError::new(
                "LATTICE_FULL_CHAIN_BINDING_REJECTED",
            ));
        }
        core.run_json(FullChainEntry::CodexAppMcp)
            .map_err(|error| ToolExecutionError::new(error.code()))
    }

    fn status(&mut self, arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        let mut core = self
            .inner
            .lock()
            .map_err(|_| ToolExecutionError::new(LatticedErrorKind::Transport.code()))?;
        if arguments.binding() != core.submission.binding() {
            return Err(ToolExecutionError::new(
                "LATTICE_FULL_CHAIN_BINDING_REJECTED",
            ));
        }
        core.status_json(FullChainEntry::CodexAppMcp)
            .map_err(|error| ToolExecutionError::new(error.code()))
    }
}

impl<H: FullChainHermesPort> GatewayService for FullChainService<H> {
    fn handle(
        &mut self,
        peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply> {
        if peer.client_kind() != GatewayClientKind::OpenClaw || peer.runtime() != RuntimeKind::Fake
        {
            return gateway_reply(
                &request,
                GatewayReplyBody::Denied(GatewayDenialCode::RoleDenied),
            );
        }
        let mut core = self.inner.lock().map_err(|_| {
            GatewayServiceError::new(
                PortErrorKind::Unavailable,
                "FULL_CHAIN_SERVICE_LOCK_REJECTED",
            )
        })?;
        if request.project_id() != core.submission.binding().project_id() {
            return gateway_reply(
                &request,
                GatewayReplyBody::Denied(GatewayDenialCode::ScopeDenied),
            );
        }

        match request.body().clone() {
            GatewayRequestBody::Submit(submission) => {
                Self::handle_submit(&mut core, &request, &submission)
            }
            GatewayRequestBody::Status(target) => Self::handle_status(&mut core, &request, target),
            GatewayRequestBody::Plan(_)
            | GatewayRequestBody::Approve(_)
            | GatewayRequestBody::Reject(_)
            | GatewayRequestBody::Stop(_) => gateway_reply(
                &request,
                GatewayReplyBody::Denied(GatewayDenialCode::RoleDenied),
            ),
        }
    }
}

/// One composition result containing both MCP and official-package `OpenClaw` surfaces.
pub struct FullChainRuntime<H>
where
    H: FullChainHermesPort + 'static,
{
    mcp_service: FullChainService<H>,
    openclaw_server: OpenClawGatewayServer<FullChainService<H>>,
}

impl<H> FullChainRuntime<H>
where
    H: FullChainHermesPort + 'static,
{
    /// Splits the one assembled runtime for concurrent MCP and loopback serving.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        FullChainService<H>,
        OpenClawGatewayServer<FullChainService<H>>,
    ) {
        (self.mcp_service, self.openclaw_server)
    }
}

/// Starts the single full-chain executable surface from process-owned inputs.
///
/// Hermes is resolved first. Until a same-runner PID/endpoint/nonce seal exists,
/// this entry fails before `PostgreSQL`, listener, MCP, Codex, or Graphify effects.
///
/// # Errors
///
/// Returns a stable startup, configuration, database, or transport failure.
pub fn serve_full_chain_from_environment() -> Result<(), LatticedError> {
    #[cfg(not(windows))]
    {
        Err(LatticedError::new(
            LatticedErrorKind::HermesProductionRunnerRequired,
        ))
    }
    #[cfg(windows)]
    {
        let hermes_environment = HermesEnvironmentConfig::from_environment()?;
        let run_mode = full_chain_run_mode_from_environment()?;
        let (config, database, password) = delivery_environment()?;
        let (openclaw_config, launch_record) = openclaw_from_environment()?;
        let hermes = hermes_environment.launch(database.run_id())?;
        let runtime = assemble_full_chain_runtime_with_mode(
            config,
            &database,
            &password,
            hermes,
            openclaw_config,
            launch_record,
            run_mode,
        )?;
        serve_full_chain_runtime(runtime)
    }
}

/// Serves MCP stdio and continuously pumps the authenticated `OpenClaw` listener.
///
/// Both surfaces hold clones of the same [`FullChainService`], so they serialize
/// through one coordinator and share `PostgreSQL` receipts. Process lifetime is the
/// shutdown policy for this bounded entrypoint.
///
/// # Errors
///
/// Returns a bounded MCP startup or stdio transport failure. A fatal `OpenClaw`
/// listener failure terminates the executable with exit code 2 rather than
/// leaving a falsely healthy MCP-only process.
pub fn serve_full_chain_runtime<H>(runtime: FullChainRuntime<H>) -> Result<(), LatticedError>
where
    H: FullChainHermesPort + 'static,
{
    let (mcp_service, openclaw_server) = runtime.into_parts();
    let endpoint = openclaw_server
        .local_addr()
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    thread::Builder::new()
        .name("lattice-openclaw-full-chain".to_owned())
        .spawn(move || {
            run_openclaw_pump(openclaw_server, |failure| {
                eprintln!("{}", failure.code);
                if fatal_openclaw_pump_error(failure.kind) {
                    process::exit(2);
                }
                OpenClawPumpControl::Continue
            });
        })
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    eprintln!(
        "{}",
        json!({
            "classification": "official-package-transport",
            "endpoint": endpoint.to_string(),
            "entrypoint": "openclaw-typed",
            "event": "ready",
            "runtime_kind": "Fake"
        })
    );
    let input = io::stdin();
    let output = io::stdout();
    mcp::serve(mcp_service, input.lock(), output.lock())
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenClawPumpFailure {
    kind: GatewayTransportErrorKind,
    code: &'static str,
}

trait FullChainOpenClawPump: Send + 'static {
    fn pump_once(&mut self) -> Result<(), OpenClawPumpFailure>;
}

impl<S> FullChainOpenClawPump for OpenClawGatewayServer<S>
where
    S: GatewayService + Send + 'static,
{
    fn pump_once(&mut self) -> Result<(), OpenClawPumpFailure> {
        self.serve_once().map_err(|error| OpenClawPumpFailure {
            kind: error.kind(),
            code: error.code(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpenClawPumpControl {
    Continue,
    Stop,
}

fn run_openclaw_pump<P, F>(mut pump: P, mut on_failure: F)
where
    P: FullChainOpenClawPump,
    F: FnMut(OpenClawPumpFailure) -> OpenClawPumpControl,
{
    loop {
        if let Err(failure) = pump.pump_once()
            && on_failure(failure) == OpenClawPumpControl::Stop
        {
            return;
        }
    }
}

const fn fatal_openclaw_pump_error(kind: GatewayTransportErrorKind) -> bool {
    matches!(
        kind,
        GatewayTransportErrorKind::Configuration
            | GatewayTransportErrorKind::Unavailable
            | GatewayTransportErrorKind::NonLocal
            | GatewayTransportErrorKind::Capacity
    )
}

/// Assembles the only production full-chain core and both bounded entry surfaces.
///
/// The `OpenClaw` peer remains visibly `RuntimeKind::Fake`; the official launch
/// record proves package/process provenance only. Both surfaces share the same
/// in-process coordinator and `PostgreSQL` task, graph, reflection, and command receipts.
///
/// # Errors
///
/// Rejects non-official Codex, fake Hermes, malformed fixed Task Spec, database,
/// or official `OpenClaw` launch configuration before serving.
pub fn assemble_full_chain_runtime<H>(
    config: LatticedDeliveryConfig,
    database: &DeliveryDatabaseBinding,
    password: &str,
    hermes: H,
    openclaw_config: OpenClawGatewayConfig,
    launch_record: OpenClawOfficialLaunchRecord,
) -> Result<FullChainRuntime<H>, LatticedError>
where
    H: FullChainHermesPort + 'static,
{
    assemble_full_chain_runtime_with_mode(
        config,
        database,
        password,
        hermes,
        openclaw_config,
        launch_record,
        FullChainRunMode::Fresh,
    )
}

fn full_chain_delivery_service(
    config: LatticedDeliveryConfig,
    database: &DeliveryDatabaseBinding,
    password: &str,
    run_mode: FullChainRunMode,
) -> Result<LatticedDeliveryService, LatticedError> {
    let timeout = config.timeout;
    match run_mode {
        FullChainRunMode::Fresh => {
            LatticedDeliveryService::for_delivery(config, database.clone(), password.to_owned())
        }
        FullChainRunMode::ResumeExisting => {
            LatticedDeliveryService::status_only(database.clone(), password.to_owned(), timeout)
        }
    }
}

fn assemble_full_chain_runtime_with_mode<H>(
    config: LatticedDeliveryConfig,
    database: &DeliveryDatabaseBinding,
    password: &str,
    hermes: H,
    openclaw_config: OpenClawGatewayConfig,
    launch_record: OpenClawOfficialLaunchRecord,
    run_mode: FullChainRunMode,
) -> Result<FullChainRuntime<H>, LatticedError>
where
    H: FullChainHermesPort + 'static,
{
    if config.runtime != DeliveryRuntime::OfficialCodexAppServer {
        return Err(LatticedError::new(LatticedErrorKind::OfficialLiveBlocked));
    }
    if hermes.runtime_kind() != RuntimeKind::Live
        || !production_hermes_sealed::Sealed::has_production_seal(&hermes)
    {
        return Err(LatticedError::new(
            LatticedErrorKind::HermesProductionRunnerRequired,
        ));
    }
    let submission = fixed_gateway_submission()?;
    let openclaw_config = openclaw_config
        .with_frozen_submission(submission.clone())
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let timeout = config.timeout;
    let delivery = full_chain_delivery_service(config, database, password, run_mode)?;
    let core = FullChainCore {
        delivery,
        hermes,
        submission,
        run_mode,
    };
    let mcp_service = FullChainService {
        inner: Arc::new(Mutex::new(core)),
    };
    let client = connect_fixed_runtime_client(database, password, deadline(timeout)?)
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    let target = ExtensionTarget::new(database.database_name(), database.run_id())
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let idempotency = PostgresCodebaseMemory::new(client, target)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let openclaw_server = OpenClawGatewayServer::bind_official_launch_with_durable_idempotency(
        openclaw_config,
        mcp_service.clone(),
        launch_record,
        idempotency,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    Ok(FullChainRuntime {
        mcp_service,
        openclaw_server,
    })
}

/// Returns the one server-owned immutable Task Spec admitted by typed `OpenClaw` submit.
///
/// # Errors
///
/// Returns a contract failure if canonical hashing or fixed binding construction fails.
pub fn fixed_gateway_submission() -> Result<TaskSpecSubmission, LatticedError> {
    let document = CanonicalValue::Object(vec![
        (
            "project_id".to_owned(),
            CanonicalValue::String(GRAPH_PROJECT_ID.to_owned()),
        ),
        (
            "project_snapshot_id".to_owned(),
            CanonicalValue::String(PROJECT_SNAPSHOT_ID.to_owned()),
        ),
        (
            "revision".to_owned(),
            CanonicalValue::String(FIXED_GATEWAY_TASK_REVISION.to_owned()),
        ),
        (
            "schema_version".to_owned(),
            CanonicalValue::String(GATEWAY_TASK_SPEC_SCHEMA_VERSION.to_owned()),
        ),
        (
            "task_id".to_owned(),
            CanonicalValue::String(TASK_ID.to_owned()),
        ),
    ]);
    let bytes = canonicalize(&document)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?
        .into_vec();
    let digest = task_spec_document_digest(&bytes)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    let binding = SubjectBinding::new(
        ProjectId::new(GRAPH_PROJECT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        ProjectSnapshotId::new(PROJECT_SNAPSHOT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        TaskId::new(TASK_ID).map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        FIXED_GATEWAY_TASK_REVISION,
        digest.clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    TaskSpecSubmission::new(binding, bytes, digest)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
}

fn validate_mcp_task_binding(arguments: &DeliveryToolArguments) -> Result<(), ToolExecutionError> {
    let submission = fixed_gateway_submission()
        .map_err(|_| ToolExecutionError::new(LatticedErrorKind::Contract.code()))?;
    if arguments.binding() != submission.binding() {
        return Err(ToolExecutionError::new(
            "LATTICE_FULL_CHAIN_BINDING_REJECTED",
        ));
    }
    Ok(())
}

fn graph_request_from_json(
    run_id: &str,
    value: &Value,
) -> Result<GraphMemoryRunRequest, LatticedError> {
    let object = value
        .as_object()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    let delivery_receipt_digest = json_digest(object, "receipt_digest")?;
    let configuration_digest = json_digest(object, "configuration_digest")?;
    let commit = object
        .get("commit_sha")
        .and_then(Value::as_str)
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
        delivery_receipt_digest,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new(GRAPH_PROJECT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        GitObjectId::new(commit).map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        digest_query_text(GRAPH_QUERY)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        configuration_digest,
        GRAPH_RETRIEVAL_LIMIT,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
}

fn hermes_request_for_graph(
    run_id: &str,
    graph_request: &GraphMemoryRunRequest,
    graph_receipt: &GraphMemoryReceipt,
) -> Result<HermesResearchRequest, LatticedError> {
    if !graph_receipt.matches_request(graph_request) {
        return Err(LatticedError::new(LatticedErrorKind::ReceiptMismatch));
    }
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new(format!("task037-hermes-request-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        TaskId::new(FULL_CHAIN_HERMES_TASK_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        AttemptId::new(format!("task037-hermes-attempt-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        graph_request.invocation().project_snapshot_id().clone(),
        graph_receipt.receipt_digest().clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    Ok(HermesResearchRequest::new(invocation))
}

#[cfg(any(windows, test))]
fn hermes_job_evidence(
    graph_request: &GraphMemoryRunRequest,
    graph_receipt: &GraphMemoryReceipt,
) -> PortResult<Vec<ReflectionEvidence>> {
    if !graph_receipt.matches_request(graph_request) {
        return Err(hermes_port_error(
            PortErrorKind::Denied,
            "HERMES_PRODUCTION_GRAPH_BINDING_REJECTED",
        ));
    }
    let task_spec = fixed_gateway_submission().map_err(|_| {
        hermes_port_error(
            PortErrorKind::Malformed,
            "HERMES_PRODUCTION_TASK_CONTEXT_REJECTED",
        )
    })?;
    let task = ReflectionEvidence::new_digest_only(
        ReflectionEvidenceKind::Task,
        task_spec.claimed_spec_digest().clone(),
        vec![graph_request.invocation().subject_digest().clone()],
    )
    .map_err(|failure| map_hermes_adapter_error(&failure))?;

    let mut graph_details = vec![
        graph_receipt.persistence().analysis_digest().clone(),
        graph_receipt.persistence().persistence_digest().clone(),
        graph_receipt.retrieval().retrieval_digest().clone(),
    ];
    graph_details.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    graph_details.dedup();
    let graphify = ReflectionEvidence::new_digest_only(
        ReflectionEvidenceKind::Graphify,
        graph_receipt.receipt_digest().clone(),
        graph_details,
    )
    .map_err(|failure| map_hermes_adapter_error(&failure))?;

    let git_context = CanonicalValue::Object(vec![
        (
            "commit_sha".to_owned(),
            CanonicalValue::String(graph_request.commit_id().as_str().to_owned()),
        ),
        (
            "project_id".to_owned(),
            CanonicalValue::String(graph_request.project_id().as_str().to_owned()),
        ),
        (
            "project_snapshot_id".to_owned(),
            CanonicalValue::String(
                graph_request
                    .invocation()
                    .project_snapshot_id()
                    .as_str()
                    .to_owned(),
            ),
        ),
    ]);
    let git = ReflectionEvidence::new(
        ReflectionEvidenceKind::Git,
        hermes_canonical_digest("lattice.hermes.git-context", &git_context)?,
    )
    .map_err(|failure| map_hermes_adapter_error(&failure))?;
    Ok(vec![task, graphify, git])
}

#[cfg(any(windows, test))]
#[allow(clippy::too_many_arguments)]
fn reflection_candidate(
    request: &HermesResearchRequest,
    graph_request: &GraphMemoryRunRequest,
    graph_receipt: &GraphMemoryReceipt,
    session_id: &str,
    model: &str,
    input_digest: &ContentDigest,
    identity_digest: &ContentDigest,
    reflection: &CanonicalReflection,
    evidence: &HermesEvidence,
) -> PortResult<HermesReflectionCandidate> {
    let invocation = request.invocation();
    let binding = reflection.binding();
    if !graph_receipt.matches_request(graph_request)
        || invocation.subject_digest() != graph_receipt.receipt_digest()
        || binding.request_id() != invocation.request_id().as_str()
        || binding.task_id() != invocation.task_id().as_str()
        || binding.attempt_id() != invocation.attempt_id().as_str()
        || binding.project_snapshot_id() != invocation.project_snapshot_id().as_str()
        || binding.subject_digest() != invocation.subject_digest().as_str()
        || binding.session_id() != session_id
        || binding.input_digest() != input_digest.as_str()
        || binding.model() != model
        || evidence.invocation() != invocation
        || evidence.runtime() != RuntimeKind::Live
        || evidence.output_digest() != reflection.output_digest()
    {
        return Err(hermes_port_error(
            PortErrorKind::Denied,
            "HERMES_PRODUCTION_REFLECTION_BINDING_REJECTED",
        ));
    }

    let findings = reflection
        .findings()
        .iter()
        .map(|finding| {
            if finding.classification() != ReflectionClassification::Inference {
                return Err(hermes_port_error(
                    PortErrorKind::Denied,
                    "HERMES_PRODUCTION_CLASSIFICATION_REJECTED",
                ));
            }
            HermesReflectionFinding::new(
                finding.statement().to_owned(),
                hermes_finding_evidence_digest(finding.evidence_digests())?,
            )
            .map_err(|_| {
                hermes_port_error(
                    PortErrorKind::Malformed,
                    "HERMES_PRODUCTION_REFLECTION_CONTENT_REJECTED",
                )
            })
        })
        .collect::<PortResult<Vec<_>>>()?;
    let content = HermesReflectionContent::new(
        reflection.summary().to_owned(),
        findings,
        reflection.next_actions().to_vec(),
    )
    .map_err(|_| {
        hermes_port_error(
            PortErrorKind::Malformed,
            "HERMES_PRODUCTION_REFLECTION_CONTENT_REJECTED",
        )
    })?;
    HermesReflectionCandidate::new(
        graph_request,
        graph_receipt,
        content,
        identity_digest.clone(),
        input_digest.clone(),
        reflection.output_digest().clone(),
    )
    .map_err(|_| {
        hermes_port_error(
            PortErrorKind::Denied,
            "HERMES_PRODUCTION_CANDIDATE_BINDING_REJECTED",
        )
    })
}

#[cfg(any(windows, test))]
fn hermes_finding_evidence_digest(digests: &[String]) -> PortResult<ContentDigest> {
    if digests.is_empty() {
        return Err(hermes_port_error(
            PortErrorKind::Malformed,
            "HERMES_PRODUCTION_FINDING_EVIDENCE_REJECTED",
        ));
    }
    let mut parsed = digests
        .iter()
        .map(|digest| {
            ContentDigest::from_sha256(digest.clone()).map_err(|_| {
                hermes_port_error(
                    PortErrorKind::Malformed,
                    "HERMES_PRODUCTION_FINDING_EVIDENCE_REJECTED",
                )
            })
        })
        .collect::<PortResult<Vec<_>>>()?;
    parsed.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    parsed.dedup();
    let value = CanonicalValue::Array(
        parsed
            .iter()
            .map(|digest| CanonicalValue::String(digest.as_str().to_owned()))
            .collect(),
    );
    hermes_canonical_digest("lattice.hermes.reflection-finding-evidence-set", &value)
}

#[cfg(any(windows, test))]
fn hermes_canonical_digest(schema_id: &str, value: &CanonicalValue) -> PortResult<ContentDigest> {
    let domain = HashDomain::new(schema_id, "1").map_err(|_| {
        hermes_port_error(
            PortErrorKind::Malformed,
            "HERMES_PRODUCTION_CANONICAL_CONTEXT_REJECTED",
        )
    })?;
    let digest = canonical_sha256(&domain, value).map_err(|_| {
        hermes_port_error(
            PortErrorKind::Malformed,
            "HERMES_PRODUCTION_CANONICAL_CONTEXT_REJECTED",
        )
    })?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| {
        hermes_port_error(
            PortErrorKind::Malformed,
            "HERMES_PRODUCTION_CANONICAL_CONTEXT_REJECTED",
        )
    })
}

#[cfg(any(windows, test))]
fn map_hermes_adapter_error(error: &HermesAdapterError) -> PortError {
    let kind = match error.kind() {
        HermesAdapterErrorKind::Configuration
        | HermesAdapterErrorKind::CrossBinding
        | HermesAdapterErrorKind::Identity => PortErrorKind::Denied,
        HermesAdapterErrorKind::CapabilityMismatch => PortErrorKind::CapabilityMismatch,
        HermesAdapterErrorKind::Malformed => PortErrorKind::Malformed,
        HermesAdapterErrorKind::Timeout => PortErrorKind::Timeout,
        HermesAdapterErrorKind::Cancelled => PortErrorKind::Cancelled,
        HermesAdapterErrorKind::Ambiguous => PortErrorKind::Ambiguous,
        HermesAdapterErrorKind::Transport
        | HermesAdapterErrorKind::HttpStatus
        | HermesAdapterErrorKind::Failed
        | HermesAdapterErrorKind::Spawn => PortErrorKind::Unavailable,
    };
    hermes_port_error(kind, error.code())
}

#[cfg(any(windows, test))]
fn hermes_port_error(kind: PortErrorKind, code: &'static str) -> PortError {
    PortError::new(Component::Hermes, kind, code)
}

fn reflection_memory(
    database: &DeliveryDatabaseBinding,
    password: &str,
    timeout: Duration,
    stage: GraphMemoryStage,
) -> Result<PostgresCodebaseMemory, GraphMemoryPortError> {
    let operation_deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        GraphMemoryPortError::new(
            stage,
            PortErrorKind::Unavailable,
            GraphMemoryFailureCertainty::Known,
            "MEMORY_REFLECTION_DEADLINE_REJECTED",
        )
    })?;
    let client =
        connect_fixed_runtime_client(database, password, operation_deadline).map_err(|_| {
            GraphMemoryPortError::new(
                stage,
                PortErrorKind::Unavailable,
                GraphMemoryFailureCertainty::Known,
                "MEMORY_DATABASE_CONNECT_REJECTED",
            )
        })?;
    let target =
        ExtensionTarget::new(database.database_name(), database.run_id()).map_err(|_| {
            GraphMemoryPortError::new(
                stage,
                PortErrorKind::Malformed,
                GraphMemoryFailureCertainty::Known,
                "MEMORY_REFLECTION_TARGET_REJECTED",
            )
        })?;
    PostgresCodebaseMemory::new(client, target).map_err(|_| {
        GraphMemoryPortError::new(
            stage,
            PortErrorKind::Unavailable,
            GraphMemoryFailureCertainty::Known,
            "MEMORY_REFLECTION_ADAPTER_REJECTED",
        )
    })
}

fn load_reflection_from_postgres(
    database: &DeliveryDatabaseBinding,
    password: &str,
    timeout: Duration,
    request: &GraphMemoryRunRequest,
) -> Result<HermesReflectionReceipt, GraphMemoryPortError> {
    reflection_memory(
        database,
        password,
        timeout,
        GraphMemoryStage::ReflectionReceipt,
    )?
    .load_reflection(request)
}

fn persist_reflection_to_postgres(
    database: &DeliveryDatabaseBinding,
    password: &str,
    timeout: Duration,
    candidate: &HermesReflectionCandidate,
) -> Result<HermesReflectionReceipt, LatticedError> {
    reflection_memory(
        database,
        password,
        timeout,
        GraphMemoryStage::ReflectionPersistence,
    )
    .map_err(|error| {
        if error.certainty() == GraphMemoryFailureCertainty::Ambiguous {
            LatticedError::new(LatticedErrorKind::ReconciliationRequired)
        } else {
            LatticedError::new(LatticedErrorKind::HermesExecution)
        }
    })?
    .persist_reflection(candidate)
    .map_err(|error| {
        if error.certainty() == GraphMemoryFailureCertainty::Ambiguous {
            LatticedError::new(LatticedErrorKind::ReconciliationRequired)
        } else {
            LatticedError::new(LatticedErrorKind::HermesExecution)
        }
    })
}

fn map_reflection_read_error(error: &GraphMemoryPortError) -> LatticedError {
    if error.certainty() == GraphMemoryFailureCertainty::Ambiguous {
        LatticedError::new(LatticedErrorKind::ReconciliationRequired)
    } else {
        LatticedError::new(LatticedErrorKind::HermesReceiptRead)
    }
}

fn append_full_chain_json(
    mut base: Value,
    reflection: &HermesReflectionReceipt,
    entry: FullChainEntry,
) -> Result<Value, LatticedError> {
    let object = base
        .as_object_mut()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    let findings = reflection
        .content()
        .findings()
        .iter()
        .map(|finding| {
            json!({
                "evidence_digest": finding.evidence_digest().as_str(),
                "statement": finding.statement(),
            })
        })
        .collect::<Vec<_>>();
    object.insert(
        "entrypoint".to_owned(),
        Value::String(entry.name().to_owned()),
    );
    object.insert(
        "entrypoint_classification".to_owned(),
        Value::String(entry.classification().to_owned()),
    );
    object.insert(
        "entrypoint_runtime_kind".to_owned(),
        Value::String(entry.runtime_kind().to_owned()),
    );
    object.insert(
        "full_chain_receipt_digest".to_owned(),
        Value::String(reflection.receipt_digest().as_str().to_owned()),
    );
    object.insert("hermes_findings".to_owned(), Value::Array(findings));
    object.insert(
        "hermes_graph_receipt_digest".to_owned(),
        Value::String(reflection.graph_receipt_digest().as_str().to_owned()),
    );
    object.insert(
        "hermes_identity_digest".to_owned(),
        Value::String(reflection.hermes_identity_digest().as_str().to_owned()),
    );
    object.insert(
        "hermes_input_digest".to_owned(),
        Value::String(reflection.input_digest().as_str().to_owned()),
    );
    object.insert(
        "hermes_next_actions".to_owned(),
        json!(reflection.content().next_actions()),
    );
    object.insert(
        "hermes_reflection_digest".to_owned(),
        Value::String(reflection.reflection_digest().as_str().to_owned()),
    );
    object.insert(
        "hermes_provenance_status".to_owned(),
        Value::String("PERSISTED_STRUCTURED_INFERENCE".to_owned()),
    );
    object.insert(
        "hermes_schema_version".to_owned(),
        Value::String(reflection.schema_version().to_owned()),
    );
    object.insert(
        "hermes_status".to_owned(),
        Value::String(reflection.status().as_str().to_owned()),
    );
    object.insert(
        "hermes_summary".to_owned(),
        Value::String(reflection.content().summary().to_owned()),
    );
    Ok(base)
}

fn json_digest(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<ContentDigest, LatticedError> {
    let value = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    ContentDigest::from_sha256(value)
        .map_err(|_| LatticedError::new(LatticedErrorKind::ReceiptMismatch))
}

fn full_chain_receipt_digest(value: &Value) -> Result<ContentDigest, LatticedError> {
    let object = value
        .as_object()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    json_digest(object, "full_chain_receipt_digest")
}

fn gateway_reply(
    request: &GatewayRequest,
    body: GatewayReplyBody,
) -> GatewayServiceResult<GatewayReply> {
    build_reply(request, body).map_err(|_| {
        GatewayServiceError::new(
            PortErrorKind::Malformed,
            "FULL_CHAIN_GATEWAY_REPLY_REJECTED",
        )
    })
}

const fn gateway_error_kind(kind: LatticedErrorKind) -> PortErrorKind {
    match kind {
        LatticedErrorKind::ReconciliationRequired => PortErrorKind::Ambiguous,
        LatticedErrorKind::DatabaseConnect
        | LatticedErrorKind::GraphReceiptRead
        | LatticedErrorKind::HermesReceiptRead => PortErrorKind::Unavailable,
        LatticedErrorKind::Configuration
        | LatticedErrorKind::Contract
        | LatticedErrorKind::ReceiptMismatch => PortErrorKind::Malformed,
        LatticedErrorKind::Intent
        | LatticedErrorKind::OutcomePersistence
        | LatticedErrorKind::DeliveryFailed
        | LatticedErrorKind::OfficialLiveBlocked
        | LatticedErrorKind::ScriptedFixtureRejected
        | LatticedErrorKind::GraphExecution
        | LatticedErrorKind::HermesProductionRunnerRequired
        | LatticedErrorKind::HermesExecution
        | LatticedErrorKind::DatabaseSecret
        | LatticedErrorKind::LedgerConfiguration
        | LatticedErrorKind::WorkspaceConfiguration
        | LatticedErrorKind::CodexConfiguration
        | LatticedErrorKind::ReceiptRead
        | LatticedErrorKind::GraphConfiguration
        | LatticedErrorKind::Transport => PortErrorKind::Denied,
    }
}

fn map_gateway_service_error(error: LatticedError) -> GatewayServiceError {
    GatewayServiceError::new(gateway_error_kind(error.kind()), error.code())
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
struct DeliveryGraphPaths {
    root: PathBuf,
    repository_root: PathBuf,
}

fn validate_scripted_fixture(
    config: &LatticedDeliveryConfig,
) -> Result<DeliveryGraphPaths, LatticedError> {
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
    Ok(DeliveryGraphPaths {
        root: fixture_root,
        repository_root,
    })
}

fn official_graph_paths(
    config: &LatticedDeliveryConfig,
) -> Result<DeliveryGraphPaths, LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::OfficialLiveBlocked);
    if config.runtime != DeliveryRuntime::OfficialCodexAppServer {
        return Err(rejected());
    }
    let fixture_root = config.delivery_root.parent().ok_or_else(rejected)?;
    let lattice_delivery_root = fixture_root.parent().ok_or_else(rejected)?;
    let target_root = lattice_delivery_root.parent().ok_or_else(rejected)?;
    reject_reparse_path(&config.delivery_root, target_root)?;

    let delivery_root = fs::canonicalize(&config.delivery_root).map_err(|_| rejected())?;
    let canonical_fixture_root = delivery_root.parent().ok_or_else(rejected)?.to_path_buf();
    let repository_root =
        fs::canonicalize(target_root.parent().ok_or_else(rejected)?).map_err(|_| rejected())?;
    Ok(DeliveryGraphPaths {
        root: canonical_fixture_root,
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
    fixture: &DeliveryGraphPaths,
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

fn effect_deadline(finalization_deadline: Instant) -> Result<Instant, LatticedError> {
    let effect = finalization_deadline
        .checked_sub(FINALIZATION_RESERVE)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))?;
    if effect <= Instant::now() {
        return Err(LatticedError::new(LatticedErrorKind::Configuration));
    }
    Ok(effect)
}

fn path_text(path: &Path) -> Result<String, LatticedError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use lattice_ports::{DeliveryFailureCertainty, DeliveryPortError};

    fn test_content_digest(fill: char) -> ContentDigest {
        ContentDigest::from_sha256(fill.to_string().repeat(64)).expect("test digest")
    }

    struct TestOpenClawPump {
        calls: Arc<AtomicUsize>,
        outcomes: VecDeque<Result<(), OpenClawPumpFailure>>,
    }

    impl FullChainOpenClawPump for TestOpenClawPump {
        fn pump_once(&mut self) -> Result<(), OpenClawPumpFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcomes.pop_front().expect("bounded pump outcome")
        }
    }

    #[test]
    fn effect_deadline_reserves_time_for_cleanup_and_terminal_ledger_finalization() {
        let finalization = deadline(Duration::from_mins(2)).expect("finalization deadline");
        let effect = effect_deadline(finalization).expect("effect deadline");

        assert_eq!(finalization.duration_since(effect), FINALIZATION_RESERVE);
        assert!(effect > Instant::now());
    }

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

    #[test]
    fn full_chain_entry_classifications_do_not_promote_openclaw_identity() {
        assert_eq!(FullChainEntry::CodexAppMcp.runtime_kind(), "Live");
        assert_eq!(
            FullChainEntry::OpenClawTyped.classification(),
            "official-package-preflight-only"
        );
        assert_eq!(FullChainEntry::OpenClawTyped.runtime_kind(), "Fake");
    }

    #[test]
    fn full_chain_run_mode_is_process_owned_and_fail_closed() {
        assert_eq!(
            parse_full_chain_run_mode(None).expect("default fresh mode"),
            FullChainRunMode::Fresh
        );
        assert_eq!(
            parse_full_chain_run_mode(Some("FRESH")).expect("explicit fresh mode"),
            FullChainRunMode::Fresh
        );
        assert_eq!(
            parse_full_chain_run_mode(Some("RESUME_EXISTING")).expect("bounded resume mode"),
            FullChainRunMode::ResumeExisting
        );
        assert!(parse_full_chain_run_mode(Some("resume_existing")).is_err());
        assert!(parse_full_chain_run_mode(Some("")).is_err());
    }

    #[test]
    fn resume_existing_mode_removes_delivery_run_capability() {
        let config = LatticedDeliveryConfig::new(
            PathBuf::from(r"C:\tools\codex.exe"),
            "codex-cli 0.147.0",
            "a".repeat(64),
            PathBuf::from(r"C:\delivery\schema"),
            PathBuf::from(r"C:\delivery\codex-home"),
            PathBuf::from(r"C:\delivery\root"),
            PathBuf::from(r"C:\tools\git.exe"),
            Duration::from_secs(30),
            DeliveryRuntime::ScriptedAcceptance,
        )
        .expect("fixed delivery config");
        let database =
            DeliveryDatabaseBinding::new("127.0.0.1", 54_171, "63ee62186707475c91b62c614d2c2528")
                .expect("fixed database binding");

        let fresh = full_chain_delivery_service(
            config.clone(),
            &database,
            "bounded-password",
            FullChainRunMode::Fresh,
        )
        .expect("fresh service");
        let resumed = full_chain_delivery_service(
            config,
            &database,
            "bounded-password",
            FullChainRunMode::ResumeExisting,
        )
        .expect("resume service");

        assert!(fresh.request_binding().is_some());
        assert!(resumed.request_binding().is_none());
    }

    #[test]
    fn full_chain_startup_requires_a_true_production_hermes_runner() {
        let error = serve_full_chain_from_environment()
            .expect_err("incomplete official Hermes chain fails before external effects");
        assert_eq!(
            error.kind(),
            LatticedErrorKind::HermesProductionRunnerRequired
        );
        assert_eq!(error.code(), "LATTICE_HERMES_PRODUCTION_RUNNER_REQUIRED");
    }

    #[cfg(windows)]
    #[test]
    fn production_hermes_environment_accepts_only_the_frozen_runtime_identity() {
        let manifest_bytes = br#"{"cpython_archive_bytes":111375313,"cpython_archive_sha256":"a140c0868258075d160fa0da51ddffd423efbc9dd350695abd33e7ce3ce94352","cpython_build_release":"20260804","cpython_provenance":"astral-sh/python-build-standalone","cpython_sha256sums_sha256":"eccfdcc61c9fe48b7fe61db8812925ce30f23943d16c60861001004a4ae8f55c","cpython_version":"3.12.13","hermes_archive_sha256":"a9a84a25999a23a859a9d17ef3134ea1c3371d8bf1984313eab839e939528152","hermes_commit":"3c27eb6234bf91b8ceee9e9071591b31e9b148cb","hermes_release":"v2026.8.3","payload_byte_count":722642720,"payload_file_count":14076,"payload_manifest_sha256":"3159e079402fad16348d5ee5be97f84b2bb8f2bf1fa4efaf65dfc73506918e4d","platform":"x86_64-unknown-linux-gnu","pyproject_sha256":"64d1085ee1c23caf0ae0d9e65c73e280f466362ed43fdda1531f18f3af1d9869","schema":"lattice.hermes.offline-runtime.v1","uv_lock_sha256":"aab3c83f71b683507a590b6315b23bdc0abd6b63b76b2349eae15bf00dfbaf2b"}"#;
        let manifest = HermesOfflineRuntimeManifest::from_canonical_json(manifest_bytes)
            .expect("exact frozen runtime manifest");
        validate_official_hermes_runtime_identity(
            OFFICIAL_HERMES_RUNTIME_GUEST_ROOT,
            manifest_bytes,
            &manifest,
        )
        .expect("exact runtime identity");

        assert!(
            validate_official_hermes_runtime_identity(
                "/var/tmp/lattice-runtime-targets/drift",
                manifest_bytes,
                &manifest,
            )
            .is_err()
        );
        let mut drifted_bytes = manifest_bytes.to_vec();
        drifted_bytes.push(b'\n');
        assert!(
            validate_official_hermes_runtime_identity(
                OFFICIAL_HERMES_RUNTIME_GUEST_ROOT,
                &drifted_bytes,
                &manifest,
            )
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn production_hermes_secret_is_rejected_before_any_canary() {
        validate_hermes_api_key("process-local-key").expect("bounded process-local secret");
        for rejected in ["", "                ", "short", "control-byte-123\n"] {
            assert!(validate_hermes_api_key(rejected).is_err());
        }
        assert!(validate_hermes_api_key(&"x".repeat(4_097)).is_err());
    }

    #[test]
    fn hermes_finding_commitment_sorts_deduplicates_and_commits_every_digest() {
        let first = test_content_digest('a');
        let second = test_content_digest('b');
        let canonical = hermes_finding_evidence_digest(&[
            second.as_str().to_owned(),
            first.as_str().to_owned(),
            second.as_str().to_owned(),
        ])
        .expect("bounded evidence set");
        let reordered = hermes_finding_evidence_digest(&[
            first.as_str().to_owned(),
            second.as_str().to_owned(),
        ])
        .expect("reordered evidence set");
        let incomplete = hermes_finding_evidence_digest(&[first.as_str().to_owned()])
            .expect("one evidence item");

        assert_eq!(canonical, reordered);
        assert_ne!(canonical, incomplete);
        let empty = hermes_finding_evidence_digest(&[])
            .expect_err("an empty evidence set is never committed");
        assert_eq!(empty.code(), "HERMES_PRODUCTION_FINDING_EVIDENCE_REJECTED");
    }

    #[test]
    fn full_chain_openclaw_pump_only_terminates_for_process_level_failures() {
        for fatal in [
            GatewayTransportErrorKind::Configuration,
            GatewayTransportErrorKind::Unavailable,
            GatewayTransportErrorKind::NonLocal,
            GatewayTransportErrorKind::Capacity,
        ] {
            assert!(fatal_openclaw_pump_error(fatal), "{fatal:?}");
        }
        for request_scoped in [
            GatewayTransportErrorKind::Timeout,
            GatewayTransportErrorKind::Ambiguous,
            GatewayTransportErrorKind::Malformed,
            GatewayTransportErrorKind::Authentication,
            GatewayTransportErrorKind::Replay,
            GatewayTransportErrorKind::Codec,
            GatewayTransportErrorKind::ForbiddenPayload,
            GatewayTransportErrorKind::CrossProject,
            GatewayTransportErrorKind::Service,
            GatewayTransportErrorKind::Reply,
        ] {
            assert!(
                !fatal_openclaw_pump_error(request_scoped),
                "{request_scoped:?}"
            );
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let pump = TestOpenClawPump {
            calls: Arc::clone(&calls),
            outcomes: VecDeque::from([
                Err(OpenClawPumpFailure {
                    kind: GatewayTransportErrorKind::Authentication,
                    code: "OPENCLAW_GATEWAY_AUTH_REJECTED",
                }),
                Ok(()),
                Err(OpenClawPumpFailure {
                    kind: GatewayTransportErrorKind::Unavailable,
                    code: "OPENCLAW_GATEWAY_UNAVAILABLE",
                }),
            ]),
        };
        let mut observed = Vec::new();
        run_openclaw_pump(pump, |failure| {
            observed.push(failure);
            if fatal_openclaw_pump_error(failure.kind) {
                OpenClawPumpControl::Stop
            } else {
                OpenClawPumpControl::Continue
            }
        });

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            observed,
            vec![
                OpenClawPumpFailure {
                    kind: GatewayTransportErrorKind::Authentication,
                    code: "OPENCLAW_GATEWAY_AUTH_REJECTED",
                },
                OpenClawPumpFailure {
                    kind: GatewayTransportErrorKind::Unavailable,
                    code: "OPENCLAW_GATEWAY_UNAVAILABLE",
                },
            ]
        );
    }

    #[test]
    fn full_chain_final_receipt_exposes_content_without_claiming_replayed_hermes_is_live() {
        let request = GraphMemoryRunRequest::new(
            Invocation::new(
                CONTRACT_VERSION,
                RequestId::new("full-chain-receipt-request").expect("request"),
                TaskId::new(GRAPH_TASK_ID).expect("task"),
                AttemptId::new("full-chain-receipt-attempt").expect("attempt"),
                ProjectSnapshotId::new(GRAPH_PROJECT_SNAPSHOT_ID).expect("snapshot"),
                test_content_digest('9'),
            )
            .expect("invocation"),
            ProjectId::new(GRAPH_PROJECT_ID).expect("project"),
            GitObjectId::new("a".repeat(40)).expect("commit"),
            test_content_digest('b'),
            test_content_digest('c'),
            GRAPH_RETRIEVAL_LIMIT,
        )
        .expect("graph request");
        let content = HermesReflectionContent::new(
            "Bounded persisted summary.",
            vec![
                HermesReflectionFinding::new(
                    "One persisted inference finding.",
                    test_content_digest('d'),
                )
                .expect("finding"),
            ],
            vec!["Review before action.".to_owned()],
        )
        .expect("reflection content");
        let receipt = HermesReflectionReceipt::replay(
            request,
            test_content_digest('e'),
            content,
            test_content_digest('f'),
            test_content_digest('1'),
            test_content_digest('2'),
            test_content_digest('3'),
        )
        .expect("reflection receipt");

        let value = append_full_chain_json(
            json!({"status": "COMPLETED"}),
            &receipt,
            FullChainEntry::OpenClawTyped,
        )
        .expect("final receipt");

        assert_eq!(value["full_chain_receipt_digest"], "3".repeat(64));
        assert_eq!(value["hermes_summary"], "Bounded persisted summary.");
        assert_eq!(
            value["hermes_findings"][0]["statement"],
            "One persisted inference finding."
        );
        assert_eq!(value["hermes_next_actions"][0], "Review before action.");
        assert_eq!(
            value["hermes_provenance_status"],
            "PERSISTED_STRUCTURED_INFERENCE"
        );
        assert_eq!(value["entrypoint_runtime_kind"], "Fake");
        assert!(value.get("hermes_runtime_kind").is_none());
        assert_eq!(
            full_chain_receipt_digest(&value).expect("typed final digest"),
            test_content_digest('3')
        );
    }
}
