//! Sole concrete composition root for the bounded TASK-032 delivery lane.

use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_SHARE_READ, GetFileInformationByHandle,
};

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_codebase_memory::digest_query_text;
use lattice_codex_adapter::{
    CodexDeliveryAdapter, CodexDeliveryAdapterConfig, CodexIdentityExpectation,
    PinnedCodexResourceDigests, PinnedCodexResources,
};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, CodexDeliveryEvidence, CodexDeliveryRequest,
    CompletedDeliveryEvidence, Component, ContentDigest, DeliveryOutcomeEvidence,
    DeliveryOutcomeRequest, DeliveryProfile, DeliveryReceipt, DeliveryRunRequest, DeliveryRuntime,
    DeliveryStage, DeliveryStatusRequest, DeliveryTerminalStatus, DurableIntentEvidence,
    FixedTestEvidence, GatewayActorId, GatewayActorKind, GatewayAdapterId, GatewayChannelId,
    GatewayClientKind, GatewayDenialCode, GatewayInstanceId, GatewayPeerContext, GatewayReply,
    GatewayReplyBody, GatewayRequest, GatewayRequestBody, GatewaySessionId,
    GatewayStatusObservation, GatewayStatusTarget, GatewayTaskProjection, GatewayTaskState,
    GitCommitEvidence, GitObjectId, GraphMemoryReceipt, GraphMemoryRunRequest, HermesEvidence,
    HermesReflectionCandidate, HermesReflectionContent, HermesReflectionFinding,
    HermesReflectionReceipt, HermesResearchRequest, HolderProcessId, Invocation, MemoryQuery,
    PreparedWorkspaceEvidence, ProjectId, ProjectSnapshotId, RequestId, RuntimeAdmissionMode,
    RuntimeKind, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId, SubjectBinding,
    TaskId, TaskIngressPeerEvidence, TaskSpecSubmission, WorkspaceChangeEvidence,
    WriterLeaseAuthorityHead,
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
    ProductionHermesRunner, preparation::verify_official_preparation_for_launch,
};
use lattice_openclaw_adapter::{
    AuthenticationKey, GatewayTransportErrorKind, OpenClawGatewayConfig, OpenClawGatewayServer,
    OpenClawLaunchAttestationKey, OpenClawLaunchAttestationTag, OpenClawOfficialLaunchEvidence,
    OpenClawOfficialLaunchRecord, OpenClawProcessStartNonce,
};
use lattice_orchestrator::{
    ControlledTaskOrchestratorError, ControlledTaskRequest, DeliveryOrchestratorError,
    delivery_status, graph_memory_status, run_controlled_task, run_delivery, run_delivery_governed,
    run_graph_memory,
};
#[cfg(test)]
use lattice_ports::TaskLifecycleAutonomyEvidence;
use lattice_ports::{
    ControlledTaskExecutionError, ControlledTaskExecutionErrorKind, ControlledTaskExecutionPort,
    DeliveryCodexPort, DeliveryFailureCertainty, DeliveryLedgerPort, DeliveryPortError,
    DeliveryPortResult, GatewayService, GatewayServiceError, GatewayServiceResult,
    GraphMemoryFailureCertainty, GraphMemoryPortError, GraphMemoryStage, HermesPort,
    HermesReflectionMemoryPort, PortError, PortErrorKind, PortResult, TaskLifecycleError,
    TaskLifecycleErrorKind, TaskLifecycleEvidence, TaskLifecyclePort, TaskLifecycleResult,
    TestRunnerPort, WorkspaceGitPort, WriterAuthorityGuardPort,
};
use lattice_postgres_codebase_memory::{
    ExtensionTarget, PostgresCodebaseMemory, apply_extension as apply_postgres_memory_extension,
    verify_embedded_extension_manifest, verify_extension as verify_memory_extension,
};
use lattice_postgres_writer_lease::{
    ExtensionTarget as WriterLeaseExtensionTarget, PostgresWriterLease,
    apply_extension as apply_postgres_writer_extension,
    verify_extension as verify_writer_extension,
};
use lattice_task_domain::{
    AcceptanceCriterion, ApprovalRequirement, ApprovalRequirements, Capability, CapabilityRequest,
    DeploymentPolicy, EvidenceType, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    ScopeOperation, TASK_SPEC_SCHEMA_VERSION, TaskBudget, TaskScope, TaskSpec, TaskSpecInput,
    TaskState,
};
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::DELIVERY_PROMPT;
use crate::delivery_ledger::{
    DeliveryDatabaseBinding, DeliveryLedger, DeliveryReceipt as LegacyDeliveryReceipt,
    DeliveryStatus, LEGACY_RECEIPT_FORMAT, PostgresDeliveryLedgerAdapter,
    PostgresDeliveryStatusReplay, connect_fixed_runtime_client,
};
use crate::git_delivery::{
    BASELINE_COMMIT_SHA, DeliveryWorkspaceGitAdapter, DeliveryWorkspaceGitAdapterConfig,
};
use crate::mcp::{
    self, DeliveryToolArguments, DeliveryToolService, ObservedEffectKind, TaskStatusArguments,
    TaskSubmitArguments, ToolExecutionError, record_observed_effect,
};
use crate::task_control::{
    PostgresTaskLifecycle, TaskPersistenceFoundation, task_admission_command_id,
};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const MAX_TIMEOUT_SECONDS: u64 = 3_600;
const FINALIZATION_RESERVE: Duration = Duration::from_secs(30);
const CONTROLLED_TASK_MAX_RUNTIME: Duration = Duration::from_mins(5);
const TASK_ID: &str = "TASK-032";
const PROJECT_SNAPSHOT_ID: &str = "task032-delivery:snapshot:1";
const CONTROLLED_TASK_ID: &str = "TASK-038-CANARY";
const CONTROLLED_PROJECT_ID: &str = "task038-controlled-canary";
const CONTROLLED_PROJECT_SNAPSHOT_ID: &str = "task038-controlled-canary:snapshot:1";
const TASK050_ACCEPTANCE_PROFILE_ENV: &str = "LATTICE_TASK050_ACCEPTANCE_PROFILE";
const TASK050_ACCEPTANCE_TASK_SPEC_SHA256_ENV: &str = "LATTICE_TASK050_ACCEPTANCE_TASK_SPEC_SHA256";
const TASK050_ASK_USER_PROJECT_ID: &str = "task050-fresh-process";
const TASK050_ASK_USER_PROJECT_SNAPSHOT_ID: &str = "task050-snapshot";
const TASK050_ASK_USER_TASK_ID: &str = "TASK-050-FRESH";
const TASK050_PROCEED_PROJECT_ID: &str = "task050-proceed-current";
const TASK050_PROCEED_PROJECT_SNAPSHOT_ID: &str = "task050-proceed-snapshot";
const TASK050_PROCEED_TASK_ID: &str = "TASK-050-PROCEED";
const CONTROLLED_WRITER_FENCING_HIGH_WATER: u64 = 1;
const CONTROLLED_WRITER_ACQUIRED_HIGH_WATER: u64 = 1;
const CONTROLLED_WRITER_TRANSITION_HIGH_WATER: u64 = 2;
const CONTROLLED_WRITER_COMMAND_HIGH_WATER: u64 = 2;
const SCRIPTED_FIXTURE_MARKER_NAME: &str = ".lattice-delivery-fixture-v1.json";
const SCRIPTED_FIXTURE_KIND: &str = "LATTICE_DELIVERY_SCRIPTED_ACCEPTANCE_V1";
const MAX_SCRIPTED_MARKER_BYTES: u64 = 4 * 1024;
const MAX_SCRIPTED_LAUNCHER_BYTES: u64 = 64 * 1024;
const MAX_SCRIPTED_SERVER_BYTES: u64 = 64 * 1024;
const MAX_GIT_EXECUTABLE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_LATTICED_EXECUTABLE_BYTES: u64 = 64 * 1024 * 1024;
const GRAPH_TASK_ID: &str = "TASK-033";
const GRAPH_PROJECT_ID: &str = "task032-delivery";
const GRAPH_PROJECT_SNAPSHOT_ID: &str = "task032-delivery:graph-snapshot:1";
const GRAPH_QUERY: &str = "lattice_delivery_fixture";
const GRAPH_RETRIEVAL_LIMIT: u16 = 10;
const GRAPH_MEMORY_ROOT_NAME: &str = "graph-memory";
const GRAPHIFY_RUNTIME_RELATIVE_PATH: &str = "target/supply-chain/graphify-v0.9.33/wsl-runtime";
const FULL_CHAIN_HERMES_TASK_ID: &str = "TASK-037";
const STORE_DAEMON_INSTANCE_ID_ENV: &str = "LATTICE_STORE_DAEMON_INSTANCE_ID";
const STORE_DAEMON_EPOCH_ENV: &str = "LATTICE_STORE_DAEMON_EPOCH";
const STORE_AUTHORITY_REVISION_ENV: &str = "LATTICE_STORE_AUTHORITY_REVISION";
const STORE_OBSERVATION_DIGEST_ENV: &str = "LATTICE_STORE_OBSERVATION_DIGEST";
const STORE_AUTHORITY_HEAD_DIGEST_ENV: &str = "LATTICE_STORE_AUTHORITY_HEAD_DIGEST";
const TASK_INGRESS_KIND_ENV: &str = "LATTICE_TASK_INGRESS_KIND";
const TASK_INGRESS_PROFILE_DIGEST_ENV: &str = "LATTICE_TASK_INGRESS_PROFILE_SHA256";
const TASK_INGRESS_SECURE_TUNNEL: &str = "CHATGPT_SECURE_MCP_TUNNEL";
const TASK_INGRESS_LOCAL_ACCEPTANCE: &str = "LOCAL_CANONICAL_MCP_ACCEPTANCE";
#[cfg(windows)]
const FULL_CHAIN_HERMES_MODEL: &str = "hermes-agent";
#[cfg(windows)]
const FULL_CHAIN_CODEX_BROKER_MODEL: &str = "gpt-5.6-terra";
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
    "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-errorfix-v1"
);
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_MANIFEST_SHA256: &str =
    "e3a3272b6cead30cd2df1af755df031766475595fdacfb080d0886671b6d1fbb";
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_TREE_SHA256: &str =
    "cb0e331bcb2b4fe2fd0977401d246819aadb800b645ca31ec233ad4e25b96929";
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_FILE_COUNT: u64 = 14_077;
#[cfg(windows)]
const OFFICIAL_HERMES_RUNTIME_BYTE_COUNT: u64 = 722_643_145;
const OPENCLAW_ADAPTER_VERSION: &str = "1.0.0";
const FIXED_GATEWAY_TASK_REVISION: &str = "1";
const SCRIPTED_SERVER_BYTES: &[u8] = include_bytes!("fixtures/task032-scripted-codex.ps1");
const MAX_OFFICIAL_LAUNCHER_BYTES: u64 = 512 * 1024 * 1024;
const MAX_OFFICIAL_RESOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_OFFICIAL_MANIFEST_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OfficialBundleFileRole {
    Launcher,
    SandboxSetup,
    CommandRunner,
    CodeModeHost,
    Rg,
    PackageManifest,
    ManagedPackageManifest,
}

const OFFICIAL_BUNDLE_FILE_ROLES: [OfficialBundleFileRole; 7] = [
    OfficialBundleFileRole::Launcher,
    OfficialBundleFileRole::SandboxSetup,
    OfficialBundleFileRole::CommandRunner,
    OfficialBundleFileRole::CodeModeHost,
    OfficialBundleFileRole::Rg,
    OfficialBundleFileRole::PackageManifest,
    OfficialBundleFileRole::ManagedPackageManifest,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OfficialBundleFilePolicy {
    role: OfficialBundleFileRole,
    relative_path: &'static str,
    sha256: &'static str,
    max_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OfficialBundlePolicy {
    version: &'static str,
    package_version: &'static str,
    target: &'static str,
    files: [OfficialBundleFilePolicy; 7],
}

const OFFICIAL_BUNDLE_POLICY: OfficialBundlePolicy = OfficialBundlePolicy {
    version: "codex-cli 0.146.0",
    package_version: "0.146.0",
    target: "x86_64-pc-windows-msvc",
    files: [
        OfficialBundleFilePolicy {
            role: OfficialBundleFileRole::Launcher,
            relative_path: "codex-official/0.146.0/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe",
            sha256: "bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb",
            max_bytes: MAX_OFFICIAL_LAUNCHER_BYTES,
        },
        OfficialBundleFilePolicy {
            role: OfficialBundleFileRole::SandboxSetup,
            relative_path: "codex-official/0.146.0/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex-resources/codex-windows-sandbox-setup.exe",
            sha256: "c12d225b34e7f82cdab6bbc714797abed661f40e158104694953889750121cef",
            max_bytes: MAX_OFFICIAL_RESOURCE_BYTES,
        },
        OfficialBundleFilePolicy {
            role: OfficialBundleFileRole::CommandRunner,
            relative_path: "codex-official/0.146.0/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex-resources/codex-command-runner.exe",
            sha256: "0102fa1820ecd03bb03a991fd2303a1a484118f7da8a71864f88ec94bca61d6d",
            max_bytes: MAX_OFFICIAL_RESOURCE_BYTES,
        },
        OfficialBundleFilePolicy {
            role: OfficialBundleFileRole::CodeModeHost,
            relative_path: "codex-official/0.146.0/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex-code-mode-host.exe",
            sha256: "6ef1de0e04d859f8f4f6d4d64f0f3ceeec28658423d91de160f5e804280d1c36",
            max_bytes: MAX_OFFICIAL_RESOURCE_BYTES,
        },
        OfficialBundleFilePolicy {
            role: OfficialBundleFileRole::Rg,
            relative_path: "codex-official/0.146.0/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex-path/rg.exe",
            sha256: "14231169855ec5205cf5a1b6f1db358ff4aed4247c86b69ce8aae647c77f6680",
            max_bytes: MAX_OFFICIAL_RESOURCE_BYTES,
        },
        OfficialBundleFilePolicy {
            role: OfficialBundleFileRole::PackageManifest,
            relative_path: "codex-official/0.146.0/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/codex-package.json",
            sha256: "aaa0646d6b615da94187b51efd50c69621a00867761161ae55cc16cfd545bec7",
            max_bytes: MAX_OFFICIAL_MANIFEST_BYTES,
        },
        OfficialBundleFilePolicy {
            role: OfficialBundleFileRole::ManagedPackageManifest,
            relative_path: "codex-official/0.146.0/node_modules/@openai/codex/package.json",
            sha256: "24dd8c63a4d2b7bc2ded86c887974f842093ce4f2ed8473267a91e036c38da20",
            max_bytes: MAX_OFFICIAL_MANIFEST_BYTES,
        },
    ],
};

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
    HermesPreparationMissing,
    HermesPreparationRequired,
    HermesProductionLivenessRejected,
    HermesProductionRunnerRequired,
    HermesTeardownRejected,
    HermesExecution,
    HermesReceiptRead,
    TaskControl,
    TaskReconciliationRequired,
    WriterLease,
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
            Self::HermesPreparationMissing => "LATTICE_HERMES_PREPARATION_REQUIRED",
            Self::HermesPreparationRequired => "LATTICE_HERMES_PREPARATION_REJECTED",
            Self::HermesProductionLivenessRejected => "LATTICE_HERMES_PRODUCTION_LIVENESS_REJECTED",
            Self::HermesProductionRunnerRequired => "LATTICE_HERMES_PRODUCTION_RUNNER_REQUIRED",
            Self::HermesTeardownRejected => "LATTICE_HERMES_TEARDOWN_REJECTED",
            Self::HermesExecution => "LATTICE_HERMES_REFLECTION_REJECTED",
            Self::HermesReceiptRead => "LATTICE_HERMES_MEMORY_RECEIPT_REJECTED",
            Self::TaskControl => "LATTICE_TASK_CONTROL_REJECTED",
            Self::TaskReconciliationRequired => "LATTICE_TASK_RECONCILIATION_REQUIRED",
            Self::WriterLease => "LATTICE_WRITER_LEASE_REJECTED",
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

const STARTUP_DIAGNOSTIC_SCHEMA: &str = "lattice.latticed.startup-diagnostic.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupDiagnosticStage {
    ConfigurationValidationStarted,
    ConfigurationValidated,
    ServiceAssemblyStarted,
    ServiceAssembled,
    StdioLoopEntered,
    WaitingForMcpInput,
    McpInitializeReceived,
    McpInitializedNotificationReceived,
    McpToolsListReceived,
    McpEndOfStream,
    StartupFailed,
}

impl StartupDiagnosticStage {
    const fn code(self) -> &'static str {
        match self {
            Self::ConfigurationValidationStarted => "CONFIGURATION_VALIDATION_STARTED",
            Self::ConfigurationValidated => "CONFIGURATION_VALIDATED",
            Self::ServiceAssemblyStarted => "SERVICE_ASSEMBLY_STARTED",
            Self::ServiceAssembled => "SERVICE_ASSEMBLED",
            Self::StdioLoopEntered => "STDIO_LOOP_ENTERED",
            Self::WaitingForMcpInput => "WAITING_FOR_MCP_INPUT",
            Self::McpInitializeReceived => "MCP_INITIALIZE_RECEIVED",
            Self::McpInitializedNotificationReceived => "MCP_INITIALIZED_NOTIFICATION_RECEIVED",
            Self::McpToolsListReceived => "MCP_TOOLS_LIST_RECEIVED",
            Self::McpEndOfStream => "MCP_END_OF_STREAM",
            Self::StartupFailed => "STARTUP_FAILED",
        }
    }
}

/// Fixed-vocabulary, non-authoritative startup state safe to mirror to stderr.
///
/// Every field is selected from compile-time constants. This prevents process
/// configuration, request contents, credentials, paths, and raw errors from
/// entering the product diagnostic stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StartupDiagnostic {
    stage: StartupDiagnosticStage,
    last_completed_stage: &'static str,
    waiting_reason: &'static str,
    configuration_health: &'static str,
    dependency_health: &'static str,
    failure_classification: &'static str,
}

impl StartupDiagnostic {
    const fn new(
        stage: StartupDiagnosticStage,
        last_completed_stage: &'static str,
        waiting_reason: &'static str,
        configuration_health: &'static str,
        dependency_health: &'static str,
        failure_classification: &'static str,
    ) -> Self {
        Self {
            stage,
            last_completed_stage,
            waiting_reason,
            configuration_health,
            dependency_health,
            failure_classification,
        }
    }

    const fn configuration_validation_started() -> Self {
        Self::new(
            StartupDiagnosticStage::ConfigurationValidationStarted,
            "NONE",
            "CONFIGURATION_VALIDATION",
            "CHECKING",
            "NOT_CHECKED",
            "NONE",
        )
    }

    const fn configuration_validated() -> Self {
        Self::new(
            StartupDiagnosticStage::ConfigurationValidated,
            "CONFIGURATION_VALIDATED",
            "SERVICE_ASSEMBLY",
            "VALID",
            "CONFIGURED_NO_CONNECTIVITY_PROBE",
            "NONE",
        )
    }

    const fn service_assembly_started() -> Self {
        Self::new(
            StartupDiagnosticStage::ServiceAssemblyStarted,
            "CONFIGURATION_VALIDATED",
            "SERVICE_ASSEMBLY",
            "VALID",
            "ASSEMBLY_IN_PROGRESS",
            "NONE",
        )
    }

    const fn service_assembled() -> Self {
        Self::new(
            StartupDiagnosticStage::ServiceAssembled,
            "SERVICE_ASSEMBLED",
            "STDIO_ENTRY",
            "VALID",
            "ASSEMBLED_NO_CONNECTIVITY_PROBE",
            "NONE",
        )
    }

    const fn stdio_loop_entered() -> Self {
        Self::new(
            StartupDiagnosticStage::StdioLoopEntered,
            "STDIO_LOOP_ENTERED",
            "MCP_INPUT",
            "VALID",
            "MCP_SESSION_PENDING",
            "NONE",
        )
    }

    const fn from_mcp_event(event: mcp::StdioLifecycleEvent) -> Self {
        match event {
            mcp::StdioLifecycleEvent::WaitingForInput => Self::new(
                StartupDiagnosticStage::WaitingForMcpInput,
                "STDIO_LOOP_ENTERED",
                "MCP_INPUT",
                "VALID",
                "MCP_SESSION_PENDING",
                "NONE",
            ),
            mcp::StdioLifecycleEvent::InitializeReceived => Self::new(
                StartupDiagnosticStage::McpInitializeReceived,
                "MCP_INITIALIZE_RECEIVED",
                "INITIALIZED_NOTIFICATION",
                "VALID",
                "MCP_SESSION_PENDING",
                "NONE",
            ),
            mcp::StdioLifecycleEvent::InitializedNotificationReceived => Self::new(
                StartupDiagnosticStage::McpInitializedNotificationReceived,
                "MCP_INITIALIZED_NOTIFICATION_RECEIVED",
                "MCP_INPUT",
                "VALID",
                "MCP_SESSION_ACTIVE",
                "NONE",
            ),
            mcp::StdioLifecycleEvent::ToolsListReceived => Self::new(
                StartupDiagnosticStage::McpToolsListReceived,
                "MCP_TOOLS_LIST_RECEIVED",
                "MCP_INPUT",
                "VALID",
                "MCP_SESSION_ACTIVE",
                "NONE",
            ),
            mcp::StdioLifecycleEvent::EndOfStream => Self::new(
                StartupDiagnosticStage::McpEndOfStream,
                "MCP_END_OF_STREAM",
                "NONE",
                "VALID",
                "STDIN_EOF",
                "NONE",
            ),
        }
    }

    const fn failure(
        last_completed_stage: &'static str,
        configuration_health: &'static str,
        dependency_health: &'static str,
        failure: LatticedErrorKind,
    ) -> Self {
        Self::new(
            StartupDiagnosticStage::StartupFailed,
            last_completed_stage,
            "NONE",
            configuration_health,
            dependency_health,
            failure.code(),
        )
    }

    fn render(self) -> String {
        format!(
            concat!(
                "{{\"schema\":\"{}\",",
                "\"stage\":\"{}\",\"last_completed_stage\":\"{}\",",
                "\"waiting_reason\":\"{}\",\"configuration_health\":\"{}\",",
                "\"dependency_health\":\"{}\",\"failure_classification\":\"{}\"}}"
            ),
            STARTUP_DIAGNOSTIC_SCHEMA,
            self.stage.code(),
            self.last_completed_stage,
            self.waiting_reason,
            self.configuration_health,
            self.dependency_health,
            self.failure_classification,
        )
    }
}

fn write_startup_diagnostic<W: Write>(writer: &mut W, diagnostic: StartupDiagnostic) {
    // Diagnostics are non-authoritative. A closed or failed stderr consumer
    // must not terminate the MCP server or change its stdout protocol.
    let _ = writeln!(writer, "{}", diagnostic.render());
}

fn emit_startup_diagnostic(diagnostic: StartupDiagnostic) {
    write_startup_diagnostic(&mut io::stderr().lock(), diagnostic);
}

impl Error for LatticedError {}

fn observed_port_effect(kind: ObservedEffectKind, stage: DeliveryStage) -> DeliveryPortResult<()> {
    record_observed_effect(kind).map_err(|_| {
        DeliveryPortError::new(
            stage,
            PortErrorKind::Malformed,
            DeliveryFailureCertainty::Known,
            "LATTICE_MCP_OBSERVED_EFFECT_REJECTED",
        )
    })
}

fn observed_database_attempt(stage: DeliveryStage) -> DeliveryPortResult<()> {
    observed_port_effect(ObservedEffectKind::Database, stage)?;
    observed_port_effect(ObservedEffectKind::Network, stage)
}

struct ObservedLedger<L> {
    inner: L,
}

impl<L: DeliveryLedgerPort> DeliveryLedgerPort for ObservedLedger<L> {
    fn record_intent(
        &mut self,
        request: &DeliveryRunRequest,
    ) -> DeliveryPortResult<DurableIntentEvidence> {
        observed_database_attempt(DeliveryStage::Intent)?;
        self.inner.record_intent(request)
    }

    fn record_outcome(
        &mut self,
        request: &DeliveryOutcomeRequest,
    ) -> DeliveryPortResult<DeliveryOutcomeEvidence> {
        observed_database_attempt(DeliveryStage::Outcome)?;
        self.inner.record_outcome(request)
    }

    fn load_receipt(
        &mut self,
        request: &DeliveryStatusRequest,
    ) -> DeliveryPortResult<DeliveryReceipt> {
        observed_database_attempt(DeliveryStage::Receipt)?;
        self.inner.load_receipt(request)
    }
}

struct ObservedWorkspace<W> {
    inner: W,
}

impl<W: WorkspaceGitPort> WorkspaceGitPort for ObservedWorkspace<W> {
    fn prepare(
        &mut self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
    ) -> DeliveryPortResult<PreparedWorkspaceEvidence> {
        observed_port_effect(
            ObservedEffectKind::Filesystem,
            DeliveryStage::WorkspacePrepare,
        )?;
        observed_port_effect(ObservedEffectKind::Process, DeliveryStage::WorkspacePrepare)?;
        self.inner.prepare(request, intent)
    }

    fn inspect_changes(
        &mut self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        workspace: &PreparedWorkspaceEvidence,
        codex: &CodexDeliveryEvidence,
    ) -> DeliveryPortResult<WorkspaceChangeEvidence> {
        observed_port_effect(
            ObservedEffectKind::Filesystem,
            DeliveryStage::ScopeVerification,
        )?;
        observed_port_effect(
            ObservedEffectKind::Process,
            DeliveryStage::ScopeVerification,
        )?;
        self.inner
            .inspect_changes(request, intent, workspace, codex)
    }

    fn commit(
        &mut self,
        request: &DeliveryRunRequest,
        workspace: &PreparedWorkspaceEvidence,
        changes: &WorkspaceChangeEvidence,
        test: &FixedTestEvidence,
    ) -> DeliveryPortResult<GitCommitEvidence> {
        observed_port_effect(ObservedEffectKind::Filesystem, DeliveryStage::GitCommit)?;
        observed_port_effect(ObservedEffectKind::Process, DeliveryStage::GitCommit)?;
        self.inner.commit(request, workspace, changes, test)
    }
}

impl<W: TestRunnerPort> TestRunnerPort for ObservedWorkspace<W> {
    fn run_fixed(
        &mut self,
        request: &DeliveryRunRequest,
        workspace: &PreparedWorkspaceEvidence,
        changes: &WorkspaceChangeEvidence,
    ) -> DeliveryPortResult<FixedTestEvidence> {
        observed_port_effect(ObservedEffectKind::Filesystem, DeliveryStage::FixedTest)?;
        observed_port_effect(ObservedEffectKind::Process, DeliveryStage::FixedTest)?;
        self.inner.run_fixed(request, workspace, changes)
    }
}

struct ObservedCodex<C> {
    inner: C,
}

impl<C: DeliveryCodexPort> DeliveryCodexPort for ObservedCodex<C> {
    fn run_delivery(
        &mut self,
        request: CodexDeliveryRequest,
    ) -> DeliveryPortResult<CodexDeliveryEvidence> {
        observed_port_effect(ObservedEffectKind::Codex, DeliveryStage::Codex)?;
        observed_port_effect(ObservedEffectKind::Process, DeliveryStage::Codex)?;
        self.inner.run_delivery(request)
    }

    fn interrupt_delivery(&mut self, request_id: &RequestId) -> DeliveryPortResult<()> {
        observed_port_effect(ObservedEffectKind::Codex, DeliveryStage::Codex)?;
        observed_port_effect(ObservedEffectKind::Process, DeliveryStage::Codex)?;
        self.inner.interrupt_delivery(request_id)
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
    official_bundle: Option<LaunchReadyOfficialBundle>,
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
        let official_bundle = if runtime == DeliveryRuntime::OfficialCodexAppServer {
            Some(validate_official_codex_identity(
                &launcher,
                &version,
                &launcher_sha256,
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
            official_bundle
                .as_ref()
                .map(|bundle| bundle.resources().clone()),
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
            official_bundle,
        })
    }

    fn status_process(timeout: Duration) -> Self {
        Self {
            launcher: PathBuf::new(),
            version: String::new(),
            launcher_sha256: String::new(),
            schema_directory: PathBuf::new(),
            codex_home: PathBuf::new(),
            delivery_root: PathBuf::new(),
            git_executable: PathBuf::new(),
            timeout,
            runtime: DeliveryRuntime::OfficialCodexAppServer,
            official_bundle: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OfficialBundleEvidenceProvenance {
    RealWindowsFilesystem,
    #[cfg(test)]
    SyntheticTest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OfficialFileIdentity {
    volume_serial_number: u64,
    file_index: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OfficialBundleFileFacts {
    role: OfficialBundleFileRole,
    declared_path: PathBuf,
    expected_path: PathBuf,
    canonical_path: PathBuf,
    canonical_expected_path: PathBuf,
    is_regular_file: bool,
    reparse_component_count: u32,
    byte_count: u64,
    sha256: Option<String>,
    captured_identity: Option<OfficialFileIdentity>,
    observed_identity: Option<OfficialFileIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OfficialBundleFacts {
    provenance: OfficialBundleEvidenceProvenance,
    version: String,
    declared_launcher_sha256: String,
    official_target_root: PathBuf,
    launcher_target_root: Option<PathBuf>,
    files: Vec<OfficialBundleFileFacts>,
}

mod official_bundle_provider_sealed {
    pub trait Sealed {}
}

trait OfficialBundleEvidenceProvider: official_bundle_provider_sealed::Sealed {
    fn facts(&self) -> &OfficialBundleFacts;
}

#[derive(Debug)]
struct PinnedOfficialFile {
    path: PathBuf,
    boundary: PathBuf,
    identity: OfficialFileIdentity,
    handle: fs::File,
}

#[derive(Clone, Debug)]
struct RealPinnedOfficialBundle {
    facts: OfficialBundleFacts,
    pinned_files: Arc<[PinnedOfficialFile]>,
}

impl PartialEq for RealPinnedOfficialBundle {
    fn eq(&self, other: &Self) -> bool {
        self.facts == other.facts
    }
}

impl Eq for RealPinnedOfficialBundle {}

impl official_bundle_provider_sealed::Sealed for RealPinnedOfficialBundle {}

impl OfficialBundleEvidenceProvider for RealPinnedOfficialBundle {
    fn facts(&self) -> &OfficialBundleFacts {
        &self.facts
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct SyntheticOfficialBundleEvidenceProvider {
    facts: OfficialBundleFacts,
}

#[cfg(test)]
impl official_bundle_provider_sealed::Sealed for SyntheticOfficialBundleEvidenceProvider {}

#[cfg(test)]
impl OfficialBundleEvidenceProvider for SyntheticOfficialBundleEvidenceProvider {
    fn facts(&self) -> &OfficialBundleFacts {
        &self.facts
    }
}

#[cfg(test)]
impl SyntheticOfficialBundleEvidenceProvider {
    fn complete(policy: &OfficialBundlePolicy) -> Self {
        let target_root = PathBuf::from(r"C:\lattice\target");
        Self::complete_at(policy, &target_root)
    }

    fn complete_at(policy: &OfficialBundlePolicy, target_root: &Path) -> Self {
        let files = OFFICIAL_BUNDLE_FILE_ROLES
            .iter()
            .copied()
            .enumerate()
            .map(|(index, role)| {
                let path = policy.expected_path(&target_root, role);
                let identity = OfficialFileIdentity {
                    volume_serial_number: 7,
                    file_index: (index + 1) as u64,
                };
                OfficialBundleFileFacts {
                    role,
                    declared_path: path.clone(),
                    expected_path: path.clone(),
                    canonical_path: path.clone(),
                    canonical_expected_path: path,
                    is_regular_file: true,
                    reparse_component_count: 0,
                    byte_count: 1,
                    sha256: Some(policy.file_policy(role).sha256.to_owned()),
                    captured_identity: Some(identity),
                    observed_identity: Some(identity),
                }
            })
            .collect();
        Self {
            facts: OfficialBundleFacts {
                provenance: OfficialBundleEvidenceProvenance::SyntheticTest,
                version: policy.version.to_owned(),
                declared_launcher_sha256: policy
                    .file_policy(OfficialBundleFileRole::Launcher)
                    .sha256
                    .to_owned(),
                official_target_root: target_root.to_path_buf(),
                launcher_target_root: Some(target_root.to_path_buf()),
                files,
            },
        }
    }

    fn remove(&mut self, role: OfficialBundleFileRole) {
        self.facts.files.retain(|file| file.role != role);
    }

    fn file_mut(&mut self, role: OfficialBundleFileRole) -> &mut OfficialBundleFileFacts {
        self.facts
            .files
            .iter_mut()
            .find(|file| file.role == role)
            .expect("complete synthetic bundle facts")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OfficialIdentityRejection {
    Layout,
    Version,
    DeclaredLauncherDigest,
    TargetSplit,
    MissingFile(OfficialBundleFileRole),
    UnexpectedFileFacts,
    PathMismatch(OfficialBundleFileRole),
    ReparsePath(OfficialBundleFileRole),
    NotRegularFile(OfficialBundleFileRole),
    OversizedFile(OfficialBundleFileRole),
    UnreadableFile(OfficialBundleFileRole),
    DigestMismatch(OfficialBundleFileRole),
    FileIdentityUnavailable(OfficialBundleFileRole),
    FileIdentityChanged(OfficialBundleFileRole),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ValidatedOfficialBundleFacts;

#[derive(Clone, Debug, Eq, PartialEq)]
struct LaunchReadyOfficialBundle {
    resources: PinnedCodexResources,
    real_bundle: RealPinnedOfficialBundle,
    _validated: ValidatedOfficialBundleFacts,
}

impl OfficialBundlePolicy {
    const fn production() -> &'static Self {
        &OFFICIAL_BUNDLE_POLICY
    }

    fn file_policy(&self, role: OfficialBundleFileRole) -> &OfficialBundleFilePolicy {
        self.files
            .iter()
            .find(|policy| policy.role == role)
            .expect("complete fixed official bundle policy")
    }

    fn expected_path(&self, target_root: &Path, role: OfficialBundleFileRole) -> PathBuf {
        self.file_policy(role)
            .relative_path
            .split('/')
            .fold(target_root.to_path_buf(), |path, component| {
                path.join(component)
            })
    }

    fn launcher_target_root(launcher: &Path) -> Result<PathBuf, OfficialIdentityRejection> {
        launcher
            .ancestors()
            .nth(9)
            .map(Path::to_path_buf)
            .ok_or(OfficialIdentityRejection::Layout)
    }

    fn evaluate(
        &self,
        provider: &impl OfficialBundleEvidenceProvider,
    ) -> Result<ValidatedOfficialBundleFacts, OfficialIdentityRejection> {
        let facts = provider.facts();
        if facts.version != self.version {
            return Err(OfficialIdentityRejection::Version);
        }
        if facts.declared_launcher_sha256
            != self.file_policy(OfficialBundleFileRole::Launcher).sha256
        {
            return Err(OfficialIdentityRejection::DeclaredLauncherDigest);
        }
        let launcher_target_root = facts
            .launcher_target_root
            .as_deref()
            .ok_or(OfficialIdentityRejection::TargetSplit)?;
        if !same_declared_path(&facts.official_target_root, launcher_target_root) {
            return Err(OfficialIdentityRejection::TargetSplit);
        }
        if facts.files.len() != OFFICIAL_BUNDLE_FILE_ROLES.len() {
            for role in OFFICIAL_BUNDLE_FILE_ROLES {
                if facts.files.iter().all(|file| file.role != role) {
                    return Err(OfficialIdentityRejection::MissingFile(role));
                }
            }
            return Err(OfficialIdentityRejection::UnexpectedFileFacts);
        }
        for file_policy in &self.files {
            let file = facts
                .files
                .iter()
                .find(|file| file.role == file_policy.role)
                .ok_or(OfficialIdentityRejection::MissingFile(file_policy.role))?;
            let expected_path = self.expected_path(&facts.official_target_root, file_policy.role);
            if !same_declared_path(&file.declared_path, &expected_path)
                || !same_declared_path(&file.expected_path, &expected_path)
                || file.canonical_path != file.canonical_expected_path
            {
                return Err(OfficialIdentityRejection::PathMismatch(file_policy.role));
            }
            if file.reparse_component_count != 0 {
                return Err(OfficialIdentityRejection::ReparsePath(file_policy.role));
            }
            if !file.is_regular_file {
                return Err(OfficialIdentityRejection::NotRegularFile(file_policy.role));
            }
            if file.byte_count > file_policy.max_bytes {
                return Err(OfficialIdentityRejection::OversizedFile(file_policy.role));
            }
            let sha256 = file
                .sha256
                .as_deref()
                .ok_or(OfficialIdentityRejection::UnreadableFile(file_policy.role))?;
            if sha256 != file_policy.sha256 {
                return Err(OfficialIdentityRejection::DigestMismatch(file_policy.role));
            }
            let captured = file.captured_identity.ok_or(
                OfficialIdentityRejection::FileIdentityUnavailable(file_policy.role),
            )?;
            let observed = file.observed_identity.ok_or(
                OfficialIdentityRejection::FileIdentityUnavailable(file_policy.role),
            )?;
            if captured != observed {
                return Err(OfficialIdentityRejection::FileIdentityChanged(
                    file_policy.role,
                ));
            }
        }
        Ok(ValidatedOfficialBundleFacts)
    }
}

impl RealPinnedOfficialBundle {
    fn capture(
        policy: &OfficialBundlePolicy,
        launcher: &Path,
        version: &str,
        launcher_sha256: &str,
    ) -> Result<Self, OfficialIdentityRejection> {
        let target_root = OfficialBundlePolicy::launcher_target_root(launcher)?;
        let launcher_target_root = Some(target_root.clone());
        let mut files = Vec::with_capacity(OFFICIAL_BUNDLE_FILE_ROLES.len());
        let mut pinned_files = Vec::with_capacity(OFFICIAL_BUNDLE_FILE_ROLES.len());
        for role in OFFICIAL_BUNDLE_FILE_ROLES {
            let expected_path = policy.expected_path(&target_root, role);
            let declared_path = if role == OfficialBundleFileRole::Launcher {
                launcher.to_path_buf()
            } else {
                expected_path.clone()
            };
            if let Some((facts, pinned)) = capture_official_file(
                role,
                declared_path,
                expected_path,
                &target_root,
                policy.file_policy(role).max_bytes,
            ) {
                files.push(facts);
                pinned_files.push(pinned);
            }
        }
        Ok(Self {
            facts: OfficialBundleFacts {
                provenance: OfficialBundleEvidenceProvenance::RealWindowsFilesystem,
                version: version.to_owned(),
                declared_launcher_sha256: launcher_sha256.to_owned(),
                official_target_root: target_root,
                launcher_target_root,
                files,
            },
            pinned_files: Arc::from(pinned_files),
        })
    }

    fn ensure_current(&self) -> Result<(), OfficialIdentityRejection> {
        for pinned in &*self.pinned_files {
            let role = file_role_for_path(&self.facts, &pinned.path)
                .ok_or(OfficialIdentityRejection::UnexpectedFileFacts)?;
            let handle_identity = official_file_identity(&pinned.handle)
                .ok_or(OfficialIdentityRejection::FileIdentityUnavailable(role))?;
            let path_identity = fs::File::open(&pinned.path)
                .ok()
                .and_then(|current| official_file_identity(&current))
                .ok_or(OfficialIdentityRejection::FileIdentityUnavailable(role))?;
            let reparse_count = reparse_component_count(&pinned.path, &pinned.boundary)
                .ok_or(OfficialIdentityRejection::ReparsePath(role))?;
            if handle_identity != pinned.identity || path_identity != pinned.identity {
                return Err(OfficialIdentityRejection::FileIdentityChanged(role));
            }
            if reparse_count != 0 {
                return Err(OfficialIdentityRejection::ReparsePath(role));
            }
        }
        Ok(())
    }
}

impl LaunchReadyOfficialBundle {
    fn from_real(
        policy: &OfficialBundlePolicy,
        real_bundle: RealPinnedOfficialBundle,
        validated: ValidatedOfficialBundleFacts,
    ) -> Result<Self, OfficialIdentityRejection> {
        let target_root = &real_bundle.facts.official_target_root;
        let install_root = target_root
            .join("codex-official")
            .join(policy.package_version);
        let managed_package_root = install_root
            .join("node_modules")
            .join("@openai")
            .join("codex");
        let bundle_root = install_root
            .join("node_modules")
            .join("@openai")
            .join("codex-win32-x64")
            .join("vendor")
            .join(policy.target);
        let resources = PinnedCodexResources::new(
            managed_package_root,
            bundle_root.join("codex-resources"),
            PinnedCodexResourceDigests::new(
                policy
                    .file_policy(OfficialBundleFileRole::SandboxSetup)
                    .sha256,
                policy
                    .file_policy(OfficialBundleFileRole::CommandRunner)
                    .sha256,
                policy
                    .file_policy(OfficialBundleFileRole::CodeModeHost)
                    .sha256,
                policy.file_policy(OfficialBundleFileRole::Rg).sha256,
                policy
                    .file_policy(OfficialBundleFileRole::PackageManifest)
                    .sha256,
                policy
                    .file_policy(OfficialBundleFileRole::ManagedPackageManifest)
                    .sha256,
            )
            .map_err(|_| OfficialIdentityRejection::Layout)?,
        )
        .map_err(|_| OfficialIdentityRejection::Layout)?;
        Ok(Self {
            resources,
            real_bundle,
            _validated: validated,
        })
    }

    fn resources(&self) -> &PinnedCodexResources {
        &self.resources
    }

    fn ensure_current(&self) -> Result<(), OfficialIdentityRejection> {
        self.real_bundle.ensure_current()
    }
}

fn validate_official_codex_identity(
    launcher: &Path,
    version: &str,
    launcher_sha256: &str,
) -> Result<LaunchReadyOfficialBundle, LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::OfficialLiveBlocked);
    let policy = OfficialBundlePolicy::production();
    let real_bundle = RealPinnedOfficialBundle::capture(policy, launcher, version, launcher_sha256)
        .map_err(|_| rejected())?;
    let validated = policy.evaluate(&real_bundle).map_err(|_| rejected())?;
    LaunchReadyOfficialBundle::from_real(policy, real_bundle, validated).map_err(|_| rejected())
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

fn reparse_component_count(path: &Path, boundary: &Path) -> Option<u32> {
    let mut current = path;
    let mut count = 0_u32;
    loop {
        let metadata = fs::symlink_metadata(current).ok()?;
        if metadata_is_reparse(&metadata) {
            count = count.checked_add(1)?;
        }
        if current == boundary {
            return Some(count);
        }
        current = current.parent()?;
    }
}

fn reject_reparse_path(path: &Path, boundary: &Path) -> Result<(), LatticedError> {
    match reparse_component_count(path, boundary) {
        Some(0) => Ok(()),
        Some(_) | None => Err(LatticedError::new(LatticedErrorKind::OfficialLiveBlocked)),
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

fn capture_official_file(
    role: OfficialBundleFileRole,
    declared_path: PathBuf,
    expected_path: PathBuf,
    boundary: &Path,
    max_bytes: u64,
) -> Option<(OfficialBundleFileFacts, PinnedOfficialFile)> {
    let reparse_component_count = reparse_component_count(&declared_path, boundary)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    options.share_mode(FILE_SHARE_READ);
    let mut handle = options.open(&declared_path).ok()?;
    let handle_metadata = handle.metadata().ok()?;
    let captured_identity = official_file_identity(&handle);
    let observed_identity = fs::File::open(&declared_path)
        .ok()
        .and_then(|observed| official_file_identity(&observed));
    let sha256 = (handle_metadata.len() <= max_bytes)
        .then(|| official_file_sha256_from_handle(&mut handle))
        .flatten();
    let canonical_path = fs::canonicalize(&declared_path).ok()?;
    let canonical_expected_path =
        fs::canonicalize(&expected_path).unwrap_or_else(|_| expected_path.clone());
    let facts = OfficialBundleFileFacts {
        role,
        declared_path: declared_path.clone(),
        expected_path,
        canonical_path,
        canonical_expected_path,
        is_regular_file: handle_metadata.is_file(),
        reparse_component_count,
        byte_count: handle_metadata.len(),
        sha256,
        captured_identity,
        observed_identity,
    };
    let identity = captured_identity?;
    Some((
        facts,
        PinnedOfficialFile {
            path: declared_path,
            boundary: boundary.to_path_buf(),
            identity,
            handle,
        },
    ))
}

fn official_file_sha256_from_handle(file: &mut fs::File) -> Option<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").ok()?;
    }
    Some(output)
}

fn official_file_sha256(path: &Path, max_bytes: u64) -> Result<String, LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::OfficialLiveBlocked);
    let metadata = fs::symlink_metadata(path).map_err(|_| rejected())?;
    if !metadata.file_type().is_file() || metadata.len() > max_bytes {
        return Err(rejected());
    }
    let mut file = fs::File::open(path).map_err(|_| rejected())?;
    official_file_sha256_from_handle(&mut file).ok_or_else(rejected)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn official_file_identity(file: &fs::File) -> Option<OfficialFileIdentity> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &raw mut information) } == 0
    {
        return None;
    }
    Some(OfficialFileIdentity {
        volume_serial_number: u64::from(information.dwVolumeSerialNumber),
        file_index: (u64::from(information.nFileIndexHigh) << 32)
            | u64::from(information.nFileIndexLow),
    })
}

#[cfg(not(windows))]
fn official_file_identity(_file: &fs::File) -> Option<OfficialFileIdentity> {
    None
}

fn file_role_for_path(facts: &OfficialBundleFacts, path: &Path) -> Option<OfficialBundleFileRole> {
    facts
        .files
        .iter()
        .find(|file| same_declared_path(&file.declared_path, path))
        .map(|file| file.role)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeliveryContinuation {
    WriterOnly,
    GraphMemory,
}

fn validate_scripted_execution_lane(runtime: DeliveryRuntime) -> Result<(), LatticedError> {
    if runtime == DeliveryRuntime::ScriptedAcceptance {
        Ok(())
    } else {
        Err(LatticedError::new(LatticedErrorKind::OfficialLiveBlocked))
    }
}

const fn requires_scripted_fixture_validation(runtime: DeliveryRuntime) -> bool {
    matches!(runtime, DeliveryRuntime::ScriptedAcceptance)
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

    /// Executes the repository-owned scripted acceptance fixture.
    ///
    /// # Errors
    ///
    /// Rejects every official runtime before a database connection or writer
    /// effect. Production and compatibility delivery runs must enter through
    /// the controlled Task coordinator instead.
    pub fn run_scripted_acceptance_json(&mut self) -> Result<Value, LatticedError> {
        let config = self
            .delivery
            .as_ref()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))?
            .clone();
        validate_scripted_execution_lane(config.runtime)?;
        let request = self
            .request
            .clone()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))?;
        self.run_request_json(
            &config,
            &request,
            None,
            None,
            None,
            None,
            DeliveryContinuation::GraphMemory,
        )
    }

    /// Executes the controlled canary under its complete Task Spec binding.
    ///
    /// # Errors
    ///
    /// Fails closed for a substituted Task Spec, missing delivery
    /// configuration, invalid ledger identity, or any governed delivery error.
    pub fn run_task_json(
        &mut self,
        binding: &SubjectBinding,
        store_authority: &StoreAuthorityHead,
        writer_authority: &WriterLeaseAuthorityHead,
        writer_guard: &mut dyn WriterAuthorityGuardPort,
        delivery_root: &Path,
    ) -> Result<Value, LatticedError> {
        let expected = fixed_gateway_submission()?;
        if expected.binding() != binding {
            return Err(LatticedError::new(LatticedErrorKind::Contract));
        }
        let config = controlled_task_delivery_config(
            self.delivery
                .as_ref()
                .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))?,
            delivery_root,
            FullChainRunMode::Fresh,
        )
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))?;
        let request = request_for_task_delivery(self.database.run_id(), &config, binding)?;
        let identity = task_ledger_identity(binding)?;
        self.run_request_json(
            &config,
            &request,
            Some(identity),
            Some(store_authority),
            Some(writer_authority),
            Some(writer_guard),
            DeliveryContinuation::WriterOnly,
        )
    }

    #[allow(clippy::too_many_lines)]
    fn run_request_json(
        &mut self,
        config: &LatticedDeliveryConfig,
        request: &DeliveryRunRequest,
        identity: Option<lattice_contracts::TaskLedgerStreamIdentity>,
        store_authority: Option<&StoreAuthorityHead>,
        writer_authority: Option<&WriterLeaseAuthorityHead>,
        writer_guard: Option<&mut dyn WriterAuthorityGuardPort>,
        continuation: DeliveryContinuation,
    ) -> Result<Value, LatticedError> {
        if let Some(bundle) = &config.official_bundle {
            bundle
                .ensure_current()
                .map_err(|_| LatticedError::new(LatticedErrorKind::OfficialLiveBlocked))?;
        }
        let scripted_graph_paths = if requires_scripted_fixture_validation(config.runtime) {
            Some(validate_scripted_fixture(config)?)
        } else {
            None
        };
        let finalization_deadline = deadline(self.timeout)?;
        let effect_deadline = effect_deadline(finalization_deadline)?;
        record_observed_effect(ObservedEffectKind::Database)
            .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
        let ledger = match (identity, writer_authority) {
            (Some(identity), Some(authority)) => DeliveryLedger::connect_for_identity_and_writer(
                &self.database,
                &self.password,
                finalization_deadline,
                identity,
                store_authority
                    .cloned()
                    .ok_or_else(|| LatticedError::new(LatticedErrorKind::Contract))?,
                authority.clone(),
            ),
            (Some(identity), None) => DeliveryLedger::connect_for_identity(
                &self.database,
                &self.password,
                finalization_deadline,
                identity,
            ),
            (None, None) => {
                DeliveryLedger::connect(&self.database, &self.password, finalization_deadline)
            }
            (None, Some(_)) => return Err(LatticedError::new(LatticedErrorKind::Contract)),
        }
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
        let repository = config.delivery_root.join("repo");
        let mut ledger = ObservedLedger {
            inner: PostgresDeliveryLedgerAdapter::for_delivery(
                ledger,
                request.clone(),
                path_text(&config.launcher)?,
                config.version.clone(),
                config.launcher_sha256.clone(),
                path_text(&config.schema_directory)?,
                path_text(&config.codex_home)?,
                path_text(&repository)?,
            )
            .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
        };
        let workspace_config = DeliveryWorkspaceGitAdapterConfig::new(
            config.delivery_root.clone(),
            config.git_executable.clone(),
            config.timeout,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::WorkspaceConfiguration))?;
        let mut workspace_git = ObservedWorkspace {
            inner: DeliveryWorkspaceGitAdapter::with_deadline(workspace_config, effect_deadline),
        };
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
            config
                .official_bundle
                .as_ref()
                .map(|bundle| bundle.resources().clone()),
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::CodexConfiguration))?;
        let mut codex = ObservedCodex {
            inner: CodexDeliveryAdapter::with_deadline(codex_config, effect_deadline),
        };
        let delivery_result = match (writer_authority, writer_guard) {
            (Some(authority), Some(guard)) => run_delivery_governed(
                request,
                authority,
                guard,
                &mut ledger,
                &mut workspace_git,
                &mut codex,
            ),
            (None, None) => run_delivery(request, &mut ledger, &mut workspace_git, &mut codex),
            (Some(_), None) | (None, Some(_)) => {
                return Err(LatticedError::new(LatticedErrorKind::Contract));
            }
        };
        match delivery_result {
            Ok(receipt) => match continuation {
                DeliveryContinuation::WriterOnly => receipt_json(&receipt, "lattice-task-writer"),
                DeliveryContinuation::GraphMemory => {
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
            },
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
        let expected_invocation = invocation_for_run(self.database.run_id())?;
        self.status_request_json(
            &expected_invocation,
            None,
            DeliveryContinuation::GraphMemory,
        )
    }

    /// Reads only the durable delivery receipt, without requiring Graphify or
    /// Hermes analysis evidence.
    fn core_status_json(&mut self) -> Result<Value, LatticedError> {
        let mut ledger =
            DeliveryLedger::connect(&self.database, &self.password, deadline(self.timeout)?)
                .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
        if ledger
            .status()
            .map_err(|_| LatticedError::new(LatticedErrorKind::ReconciliationRequired))?
            == DeliveryStatus::NotStarted
        {
            return Ok(core_not_started_status_json());
        }
        let expected_invocation = invocation_for_run(self.database.run_id())?;
        self.status_request_json(&expected_invocation, None, DeliveryContinuation::WriterOnly)
    }

    /// Replays one controlled task result from `PostgreSQL` under its Task Spec.
    ///
    /// # Errors
    ///
    /// Fails closed for a substituted Task Spec or any invalid, missing, or
    /// cross-bound durable delivery evidence.
    pub fn status_task_json(&mut self, binding: &SubjectBinding) -> Result<Value, LatticedError> {
        let expected = fixed_gateway_submission()?;
        if expected.binding() != binding {
            return Err(LatticedError::new(LatticedErrorKind::Contract));
        }
        let invocation = invocation_for_task(self.database.run_id(), binding)?;
        self.status_request_json(
            &invocation,
            Some(task_ledger_identity(binding)?),
            DeliveryContinuation::WriterOnly,
        )
    }

    fn run_task_downstream_json(
        &mut self,
        binding: &SubjectBinding,
    ) -> Result<Value, LatticedError> {
        let base = self.status_task_json(binding)?;
        let config = self
            .delivery
            .as_ref()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::Configuration))?;
        let graph_paths = official_graph_paths(config)?;
        let request = graph_request_from_json(self.database.run_id(), &base)?;
        let graph_receipt = run_graph_memory_request(
            &self.database,
            &self.password,
            config,
            &graph_paths,
            deadline(self.timeout)?,
            &request,
        );
        match graph_receipt {
            Ok(receipt) => append_graph_receipt_to_json(base, &receipt),
            Err(error) => append_optional_component_degraded_json(base, "graphify", error),
        }
    }

    fn status_task_downstream_json(
        &mut self,
        binding: &SubjectBinding,
    ) -> Result<Value, LatticedError> {
        let base = self.status_task_json(binding)?;
        let request = graph_request_from_json(self.database.run_id(), &base)?;
        let graph_receipt = load_delivery_graph_receipt(
            &self.database,
            &self.password,
            deadline(self.timeout)?,
            &request,
        );
        match graph_receipt {
            Ok(receipt) => append_graph_receipt_to_json(base, &receipt),
            Err(error) => append_optional_component_degraded_json(base, "graphify", error),
        }
    }

    fn status_request_json(
        &mut self,
        expected_invocation: &Invocation,
        identity: Option<lattice_contracts::TaskLedgerStreamIdentity>,
        continuation: DeliveryContinuation,
    ) -> Result<Value, LatticedError> {
        record_observed_effect(ObservedEffectKind::Database)
            .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
        let ledger = match identity {
            Some(identity) => DeliveryLedger::connect_for_identity(
                &self.database,
                &self.password,
                deadline(self.timeout)?,
                identity,
            ),
            None => {
                DeliveryLedger::connect(&self.database, &self.password, deadline(self.timeout)?)
            }
        }
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
        match PostgresDeliveryLedgerAdapter::for_status(
            ledger,
            expected_invocation,
            DeliveryProfile::Task032CodexPostgres,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::ReconciliationRequired))?
        {
            PostgresDeliveryStatusReplay::Legacy(receipt) => Ok(legacy_receipt_json(&receipt)),
            PostgresDeliveryStatusReplay::Typed(mut ledger) => {
                let status_request = ledger.request().status_request();
                let receipt = delivery_status(&status_request, ledger.as_mut())
                    .map_err(|error| map_orchestrator_error(&error))?;
                match continuation {
                    DeliveryContinuation::WriterOnly => {
                        receipt_json(&receipt, "task-delivery-ledger")
                    }
                    DeliveryContinuation::GraphMemory => {
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
    }
}

fn core_not_started_status_json() -> Value {
    json!({
        "component": "delivery-receipt",
        "status": "NOT_STARTED",
        "scope": "receipt-only"
    })
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

fn delivery_environment_for_mode(
    run_mode: FullChainRunMode,
) -> Result<(LatticedDeliveryConfig, DeliveryDatabaseBinding, String), LatticedError> {
    if run_mode == FullChainRunMode::Fresh {
        return delivery_environment();
    }
    let timeout = match env::var("LATTICE_DELIVERY_TIMEOUT_SECONDS") {
        Ok(value) => parse_timeout(&value)?,
        Err(env::VarError::NotPresent) => Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(LatticedError::new(LatticedErrorKind::Configuration));
        }
    };
    if required_environment("LATTICE_DELIVERY_CODEX_MODE")? != "OFFICIAL_CODEX_APP_SERVER" {
        return Err(LatticedError::new(LatticedErrorKind::Configuration));
    }
    let port = required_environment("LATTICE_TASK019_PORT")?
        .parse::<u16>()
        .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
    let database = DeliveryDatabaseBinding::new(
        required_environment("LATTICE_TASK019_HOST")?,
        port,
        required_environment("LATTICE_TASK019_RUN_ID")?,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
    Ok((
        LatticedDeliveryConfig::status_process(timeout),
        database,
        required_environment("LATTICE_TASK019_PASSWORD")?,
    ))
}

/// Installs and verifies the same-database Graphify and Writer Lease extensions
/// required by the local Runtime. The durable Store remains the only authority.
///
/// The command temporarily closes Runtime admission while it holds the extension
/// migration locks, then restores the configured Runtime authority. A failed setup
/// never reports readiness and restores the exact prior admission row.
pub fn bootstrap_postgres_extensions_from_environment() -> Result<(), LatticedError> {
    let (_config, database, password) =
        delivery_environment_for_mode(FullChainRunMode::ResumeExisting)?;
    let submission = fixed_gateway_submission()?;
    let authority = configured_store_authority()?;
    let configured_admission = RuntimeAdmissionSnapshot::from_authority(&authority)?;
    let mut foundation_probe = PostgresTaskLifecycle::connect(
        &database,
        &password,
        deadline(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))?,
        task_ledger_identity(submission.binding())?,
        authority.clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?;
    let foundation = foundation_probe
        .persistence_foundation(submission.binding())
        .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?;
    drop(foundation_probe);

    let memory_target = ExtensionTarget::new(database.database_name(), database.run_id())
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let memory_manifest = verify_embedded_extension_manifest()
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let writer_target = WriterLeaseExtensionTarget::new(
        database.database_name(),
        foundation.database_identity_digest().clone(),
        foundation.global_manifest_digest().clone(),
        memory_manifest.manifest_sha256().clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
    let mut migrator = connect_migrator(&database, &password)?;
    let admission = RuntimeAdmissionSnapshot::load(&mut migrator)?;
    admission.stop(&mut migrator)?;

    let setup = (|| {
        apply_postgres_memory_extension(&mut migrator, &memory_target)
            .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
        verify_memory_extension(
            &mut migrator,
            &memory_target,
            lattice_postgres_codebase_memory::ExtensionDatabaseRole::Migrator,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
        apply_postgres_writer_extension(&mut migrator, &writer_target)
            .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
        verify_writer_extension(&mut migrator, &writer_target)
            .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
        Ok(())
    })();
    match setup {
        Ok(()) => configured_admission.restore(&mut migrator)?,
        Err(error) => {
            admission.restore(&mut migrator)?;
            return Err(error);
        }
    }
    Ok(())
}

fn connect_migrator(
    database: &DeliveryDatabaseBinding,
    password: &str,
) -> Result<Client, LatticedError> {
    let host = required_environment("LATTICE_TASK019_HOST")?;
    let port = required_environment("LATTICE_TASK019_PORT")?
        .parse::<u16>()
        .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
    let database_name = database.database_name();
    let mut config = Config::new();
    config
        .host(&host)
        .port(port)
        .user("lattice_migrator_login")
        .password(password)
        .dbname(&database_name)
        .application_name("lattice-devos-task019")
        .ssl_mode(SslMode::Disable);
    let mut client = config
        .connect(NoTls)
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    client
        .batch_execute("SET ROLE lattice_migrator")
        .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?;
    Ok(client)
}

struct RuntimeAdmissionSnapshot {
    mode: String,
    daemon_instance_id: Option<String>,
    daemon_epoch: Option<i64>,
    authority_revision: i64,
    observation_digest: Option<Vec<u8>>,
    authority_head_digest: Option<Vec<u8>>,
}

impl RuntimeAdmissionSnapshot {
    fn from_authority(authority: &StoreAuthorityHead) -> Result<Self, LatticedError> {
        if authority.admission() != RuntimeAdmissionMode::Active {
            return Err(LatticedError::new(LatticedErrorKind::LedgerConfiguration));
        }
        let digest = |value: &ContentDigest| {
            (0..value.as_str().len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&value.as_str()[index..index + 2], 16))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))
        };
        Ok(Self {
            mode: "ACTIVE".to_owned(),
            daemon_instance_id: Some(authority.daemon_instance_id().as_str().to_owned()),
            daemon_epoch: Some(
                i64::try_from(authority.daemon_epoch().get())
                    .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
            ),
            authority_revision: i64::try_from(authority.revision().get())
                .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
            observation_digest: Some(digest(authority.observation_digest())?),
            authority_head_digest: Some(digest(authority.head_digest())?),
        })
    }

    fn load(client: &mut Client) -> Result<Self, LatticedError> {
        let row = client
            .query_one(
                "SELECT admission_mode::text, daemon_instance_id, daemon_epoch, authority_revision, \
                 observation_digest, authority_head_digest \
                 FROM ONLY control.runtime_admission WHERE singleton FOR UPDATE",
                &[],
            )
            .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?;
        Ok(Self {
            mode: row
                .try_get(0)
                .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
            daemon_instance_id: row
                .try_get(1)
                .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
            daemon_epoch: row
                .try_get(2)
                .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
            authority_revision: row
                .try_get(3)
                .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
            observation_digest: row
                .try_get(4)
                .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
            authority_head_digest: row
                .try_get(5)
                .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?,
        })
    }

    fn stop(&self, client: &mut Client) -> Result<(), LatticedError> {
        client
            .execute(
                "UPDATE ONLY control.runtime_admission SET admission_mode = 'STOPPED', \
                 daemon_instance_id = NULL, daemon_epoch = NULL, authority_revision = 0, \
                 observation_digest = NULL, authority_head_digest = NULL, \
                 updated_at = pg_catalog.clock_timestamp() WHERE singleton",
                &[],
            )
            .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?;
        Ok(())
    }

    fn restore(&self, client: &mut Client) -> Result<(), LatticedError> {
        client
            .execute(
                "UPDATE ONLY control.runtime_admission SET admission_mode = $1, \
                 daemon_instance_id = $2, daemon_epoch = $3, authority_revision = $4, \
                 observation_digest = $5, authority_head_digest = $6, \
                 updated_at = pg_catalog.clock_timestamp() WHERE singleton",
                &[
                    &self.mode,
                    &self.daemon_instance_id,
                    &self.daemon_epoch,
                    &self.authority_revision,
                    &self.observation_digest,
                    &self.authority_head_digest,
                ],
            )
            .map_err(|_| LatticedError::new(LatticedErrorKind::LedgerConfiguration))?;
        Ok(())
    }
}

/// Starts the canonical newline-delimited MCP stdio server. The default
/// `TASK_ONLY` mode remains Hermes-free; exact `PRODUCTION` mode may lazily
/// activate Hermes only when Delivery Run is invoked.
///
/// # Errors
///
/// Returns a bounded startup/configuration or transport failure.
pub fn serve_stdio_from_environment() -> Result<(), LatticedError> {
    serve_stdio_from_environment_with_diagnostics(&mut emit_startup_diagnostic)
}

fn serve_stdio_from_environment_with_diagnostics<F>(diagnostic: &mut F) -> Result<(), LatticedError>
where
    F: FnMut(StartupDiagnostic),
{
    diagnostic(StartupDiagnostic::configuration_validation_started());
    let run_mode = match full_chain_run_mode_from_environment() {
        Ok(run_mode) => run_mode,
        Err(error) => {
            diagnostic(StartupDiagnostic::failure(
                "NONE",
                "REJECTED",
                "NOT_CHECKED",
                error.kind(),
            ));
            return Err(error);
        }
    };
    let integration_mode = match runtime_integration_mode_from_environment() {
        Ok(mode) => mode,
        Err(error) => {
            diagnostic(StartupDiagnostic::failure(
                "NONE",
                "REJECTED",
                "NOT_CHECKED",
                error.kind(),
            ));
            return Err(error);
        }
    };
    let hermes_mode = match canonical_hermes_mode_from_environment() {
        Ok(mode) => mode,
        Err(error) => {
            diagnostic(StartupDiagnostic::failure(
                "NONE",
                "REJECTED",
                "NOT_CHECKED",
                error.kind(),
            ));
            return Err(error);
        }
    };
    #[cfg(not(windows))]
    if integration_mode.uses_hermes() && hermes_mode == CanonicalHermesMode::Production {
        let error = LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired);
        diagnostic(StartupDiagnostic::failure(
            "NONE",
            "REJECTED",
            "NOT_CHECKED",
            error.kind(),
        ));
        return Err(error);
    }
    let (config, database, password) = match delivery_environment_for_mode(run_mode) {
        Ok(environment) => environment,
        Err(error) => {
            diagnostic(StartupDiagnostic::failure(
                "NONE",
                "REJECTED",
                "NOT_CHECKED",
                error.kind(),
            ));
            return Err(error);
        }
    };
    let submission = match gateway_submission_from_environment(run_mode) {
        Ok(submission) => submission,
        Err(error) => {
            diagnostic(StartupDiagnostic::failure(
                "NONE",
                "REJECTED",
                "NOT_CHECKED",
                error.kind(),
            ));
            return Err(error);
        }
    };
    diagnostic(StartupDiagnostic::configuration_validated());
    diagnostic(StartupDiagnostic::service_assembly_started());
    let hermes = match (integration_mode, hermes_mode) {
        (_, CanonicalHermesMode::TaskOnly)
        | (RuntimeIntegrationMode::CoreOnly | RuntimeIntegrationMode::Graphify, _) => {
            CanonicalHermes::TaskOnly(DeferredTaskHermes)
        }
        #[cfg(windows)]
        (RuntimeIntegrationMode::GraphifyHermes, CanonicalHermesMode::Production) => {
            CanonicalHermes::Production {
                active: None,
                activation_attempted: false,
            }
        }
        #[cfg(not(windows))]
        (RuntimeIntegrationMode::GraphifyHermes, CanonicalHermesMode::Production) => {
            unreachable!("non-Windows production mode returns before composition")
        }
    };
    let (service, binding) = assemble_full_chain_service_with_mode(
        config,
        &database,
        &password,
        hermes,
        submission,
        run_mode,
        integration_mode,
        integration_mode.uses_hermes() && hermes_mode == CanonicalHermesMode::Production,
    )
    .inspect_err(|error| {
        diagnostic(StartupDiagnostic::failure(
            "CONFIGURATION_VALIDATED",
            "VALID",
            "ASSEMBLY_REJECTED",
            error.kind(),
        ));
    })?;
    diagnostic(StartupDiagnostic::service_assembled());
    diagnostic(StartupDiagnostic::stdio_loop_entered());
    let input = io::stdin();
    let output = io::stdout();
    let shutdown = service.clone();
    let serve_result = mcp::serve_with_lifecycle_observer(
        service,
        binding,
        input.lock(),
        output.lock(),
        |event| {
            diagnostic(StartupDiagnostic::from_mcp_event(event));
        },
    )
    .map_err(|_| {
        let error = LatticedError::new(LatticedErrorKind::Transport);
        diagnostic(StartupDiagnostic::failure(
            "STDIO_LOOP_ENTERED",
            "VALID",
            "MCP_TRANSPORT_REJECTED",
            error.kind(),
        ));
        error
    });
    shutdown.finish_hermes_session(serve_result)
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
    use super::LatticedError;

    pub trait Sealed {
        fn has_production_seal(&self) -> bool;
        fn is_production_configured(&self) -> bool;
        fn ensure_ready(&mut self, run_id: &str) -> Result<(), LatticedError>;
        fn terminate(&mut self) -> Result<(), LatticedError>;
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

/// Deliberately inert Hermes edge used by the canonical task-only `latticed`
/// process. No method can mint live evidence; TASK-037 reconnects the verified
/// production runner only after the governed writer slice is complete.
struct DeferredTaskHermes;

impl production_hermes_sealed::Sealed for DeferredTaskHermes {
    fn has_production_seal(&self) -> bool {
        false
    }

    fn is_production_configured(&self) -> bool {
        false
    }

    fn ensure_ready(&mut self, _run_id: &str) -> Result<(), LatticedError> {
        Err(LatticedError::new(
            LatticedErrorKind::HermesProductionRunnerRequired,
        ))
    }

    fn terminate(&mut self) -> Result<(), LatticedError> {
        Ok(())
    }
}

impl HermesPort for DeferredTaskHermes {
    fn research(&mut self, _request: HermesResearchRequest) -> PortResult<HermesEvidence> {
        Err(PortError::new(
            Component::Hermes,
            PortErrorKind::Denied,
            "HERMES_DEFERRED_UNTIL_TASK037",
        ))
    }

    fn interrupt(&mut self, _request_id: &RequestId) -> PortResult<()> {
        Err(PortError::new(
            Component::Hermes,
            PortErrorKind::Denied,
            "HERMES_DEFERRED_UNTIL_TASK037",
        ))
    }
}

impl FullChainHermesPort for DeferredTaskHermes {
    fn runtime_kind(&self) -> RuntimeKind {
        RuntimeKind::Fake
    }

    fn research_canonical(
        &mut self,
        _request: &HermesResearchRequest,
        _graph_request: &GraphMemoryRunRequest,
        _graph_receipt: &GraphMemoryReceipt,
    ) -> PortResult<ProductionHermesOutput> {
        Err(PortError::new(
            Component::Hermes,
            PortErrorKind::Denied,
            "HERMES_DEFERRED_UNTIL_TASK037",
        ))
    }
}

enum CanonicalHermes {
    TaskOnly(DeferredTaskHermes),
    #[cfg(windows)]
    Production {
        active: Option<Box<FullChainHermes>>,
        activation_attempted: bool,
    },
}

fn activate_canonical_hermes_once<'a, T, F>(
    active: &'a mut Option<T>,
    activation_attempted: &mut bool,
    launch: F,
) -> Result<&'a mut T, LatticedError>
where
    F: FnOnce() -> Result<T, LatticedError>,
{
    if active.is_none() {
        if *activation_attempted {
            return Err(LatticedError::new(
                LatticedErrorKind::HermesProductionRunnerRequired,
            ));
        }
        *activation_attempted = true;
        *active = Some(launch()?);
    }
    active
        .as_mut()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))
}

impl production_hermes_sealed::Sealed for CanonicalHermes {
    fn has_production_seal(&self) -> bool {
        match self {
            Self::TaskOnly(hermes) => hermes.has_production_seal(),
            #[cfg(windows)]
            Self::Production { active, .. } => active.as_ref().is_some_and(|hermes| {
                production_hermes_sealed::Sealed::has_production_seal(hermes.as_ref())
            }),
        }
    }

    fn is_production_configured(&self) -> bool {
        #[cfg(windows)]
        {
            matches!(self, Self::Production { .. })
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    fn ensure_ready(&mut self, run_id: &str) -> Result<(), LatticedError> {
        match self {
            Self::TaskOnly(hermes) => hermes.ensure_ready(run_id),
            #[cfg(windows)]
            Self::Production {
                active,
                activation_attempted,
            } => {
                let hermes = activate_canonical_hermes_once(active, activation_attempted, || {
                    require_hermes_preparation_environment()?;
                    HermesEnvironmentConfig::from_environment()?
                        .launch(run_id)
                        .map(Box::new)
                })?;
                hermes.ensure_ready(run_id)
            }
        }
    }

    fn terminate(&mut self) -> Result<(), LatticedError> {
        match self {
            Self::TaskOnly(hermes) => hermes.terminate(),
            #[cfg(windows)]
            Self::Production { active, .. } => active.take().map_or(Ok(()), |mut hermes| {
                production_hermes_sealed::Sealed::terminate(hermes.as_mut())
            }),
        }
    }
}

impl HermesPort for CanonicalHermes {
    fn research(&mut self, request: HermesResearchRequest) -> PortResult<HermesEvidence> {
        match self {
            Self::TaskOnly(hermes) => hermes.research(request),
            #[cfg(windows)]
            Self::Production { active, .. } => active.as_mut().map_or_else(
                || {
                    Err(PortError::new(
                        Component::Hermes,
                        PortErrorKind::Denied,
                        "HERMES_PRODUCTION_RUNNER_REQUIRED",
                    ))
                },
                |hermes| hermes.research(request),
            ),
        }
    }

    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()> {
        match self {
            Self::TaskOnly(hermes) => hermes.interrupt(request_id),
            #[cfg(windows)]
            Self::Production { active, .. } => active.as_mut().map_or_else(
                || {
                    Err(PortError::new(
                        Component::Hermes,
                        PortErrorKind::Denied,
                        "HERMES_PRODUCTION_RUNNER_REQUIRED",
                    ))
                },
                |hermes| hermes.interrupt(request_id),
            ),
        }
    }
}

impl FullChainHermesPort for CanonicalHermes {
    fn runtime_kind(&self) -> RuntimeKind {
        match self {
            Self::TaskOnly(hermes) => hermes.runtime_kind(),
            #[cfg(windows)]
            Self::Production { active, .. } => active
                .as_ref()
                .map_or(RuntimeKind::Fake, |hermes| hermes.runtime_kind()),
        }
    }

    fn research_canonical(
        &mut self,
        request: &HermesResearchRequest,
        graph_request: &GraphMemoryRunRequest,
        graph_receipt: &GraphMemoryReceipt,
    ) -> PortResult<ProductionHermesOutput> {
        match self {
            Self::TaskOnly(hermes) => {
                hermes.research_canonical(request, graph_request, graph_receipt)
            }
            #[cfg(windows)]
            Self::Production { active, .. } => active.as_mut().map_or_else(
                || {
                    Err(PortError::new(
                        Component::Hermes,
                        PortErrorKind::Denied,
                        "HERMES_PRODUCTION_RUNNER_REQUIRED",
                    ))
                },
                |hermes| hermes.research_canonical(request, graph_request, graph_receipt),
            ),
        }
    }
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
        if runner.verify_live().is_err() {
            return match runner.terminate() {
                Ok(()) => Err(LatticedError::new(
                    LatticedErrorKind::HermesProductionLivenessRejected,
                )),
                Err(_) => Err(LatticedError::new(
                    LatticedErrorKind::HermesTeardownRejected,
                )),
            };
        }
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

#[cfg(any(windows, test))]
fn full_chain_hermes_state_has_seal<R, B>(ready: Option<&R>, bound: Option<&B>) -> bool {
    ready.is_some() || bound.is_some()
}

#[cfg(any(windows, test))]
fn hermes_failure_allows_reconciliation(failure: &PortError) -> bool {
    matches!(
        (failure.kind(), failure.code()),
        (
            PortErrorKind::Timeout,
            "HERMES_LOOPBACK_TIMEOUT" | "HERMES_RUN_DEADLINE_EXCEEDED"
        ) | (
            PortErrorKind::Unavailable,
            "HERMES_LOOPBACK_TRANSPORT_FAILED"
        )
    )
}

#[cfg(any(windows, test))]
fn run_or_reconcile_active_hermes<P, R, O>(
    port: &mut P,
    run: impl FnOnce(&mut P) -> PortResult<O>,
    known_run_receipt: impl FnOnce(&P) -> Option<R>,
    reconcile: impl FnOnce(&mut P, &R) -> PortResult<O>,
) -> PortResult<O> {
    match run(port) {
        Ok(output) => Ok(output),
        Err(initial_failure) if hermes_failure_allows_reconciliation(&initial_failure) => {
            match known_run_receipt(port) {
                Some(receipt) => reconcile(port, &receipt).map_err(|failure| {
                    if hermes_failure_allows_reconciliation(&failure) {
                        PortError::new(
                            Component::Hermes,
                            PortErrorKind::Ambiguous,
                            "HERMES_RUN_RECONCILIATION_REQUIRED",
                        )
                    } else {
                        failure
                    }
                }),
                None => Err(initial_failure),
            }
        }
        Err(initial_failure) => Err(initial_failure),
    }
}

#[cfg(windows)]
impl production_hermes_sealed::Sealed for FullChainHermes {
    fn has_production_seal(&self) -> bool {
        full_chain_hermes_state_has_seal(self.ready.as_ref(), self.bound.as_ref())
    }

    fn is_production_configured(&self) -> bool {
        true
    }

    fn ensure_ready(&mut self, _run_id: &str) -> Result<(), LatticedError> {
        if let Some(runner) = self.ready.as_mut() {
            if runner.verify_live().is_ok() {
                return Ok(());
            }
            match production_hermes_sealed::Sealed::terminate(self) {
                Ok(()) => Err(LatticedError::new(
                    LatticedErrorKind::HermesProductionLivenessRejected,
                )),
                Err(teardown) => Err(teardown),
            }
        } else if self.bound.is_some() {
            Ok(())
        } else {
            Err(LatticedError::new(
                LatticedErrorKind::HermesProductionRunnerRequired,
            ))
        }
    }

    fn terminate(&mut self) -> Result<(), LatticedError> {
        if let Some(port) = self.bound.take() {
            return port
                .terminate()
                .map_err(|_| LatticedError::new(LatticedErrorKind::HermesTeardownRejected));
        }
        self.ready.take().map_or(Ok(()), |runner| {
            runner
                .terminate()
                .map_err(|_| LatticedError::new(LatticedErrorKind::HermesTeardownRejected))
        })
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
        let port = self
            .bound
            .as_mut()
            .expect("bound port installed immediately above");
        let output = run_or_reconcile_active_hermes(
            port,
            |port| port.run_reflection_evidence(request),
            |port| {
                port.active_recovery_receipt()
                    .filter(|receipt| receipt.run_id().is_some())
                    .cloned()
            },
            |port, receipt| port.reconcile_reflection_evidence(request, receipt),
        )?;
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
        let product_root = PathBuf::from(
            std::env::var_os("LATTICE_HERMES_PRODUCT_ROOT")
                .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesPreparationRequired))?,
        );
        let preparation_root = PathBuf::from(
            std::env::var_os("LATTICE_HERMES_PREPARATION_ROOT")
                .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesPreparationRequired))?,
        );
        let preparation_receipt = std::env::var("LATTICE_HERMES_PREPARATION_RECEIPT_SHA256")
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesPreparationRequired))?;
        verify_official_preparation_for_launch(
            &preparation_root,
            &product_root,
            &preparation_receipt,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesPreparationRequired))?;

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
        let containment = HermesWslContainmentConfig::new(
            PathBuf::from(hermes_environment("LATTICE_HERMES_WSL_EXE")?),
            runtime_guest_root,
            PathBuf::from(hermes_environment("LATTICE_HERMES_ISOLATION_ROOT")?),
            product_root.clone(),
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let broker = CodexReflectionBrokerConfig::new(
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

/// Fixed, redacted result of the canonical Hermes configuration preflight.
///
/// It is operational output only. It never contains an environment value, a
/// file path, a credential, or a raw validation error.
#[derive(Debug, Eq, PartialEq)]
pub enum HermesProductionPreflight {
    /// One or more required settings are absent. The names are safe to report.
    MissingConfiguration(Vec<&'static str>),
    /// Present configuration did not satisfy the pinned local validation.
    ConfigurationRejected,
    /// Required configuration passed static parsing, but launch-time asset,
    /// identity, containment, and broker verification has not run.
    ConfigurationPresentUnverified,
}

/// Fixed, redacted result of the secret-free Hermes runtime/isolation check.
#[derive(Debug, Eq, PartialEq)]
pub enum HermesRuntimePreflight {
    MissingConfiguration(Vec<&'static str>),
    ConfigurationRejected,
    ConfigurationPresentUnverified,
}

impl HermesRuntimePreflight {
    /// Renders one stable, stderr-safe record.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::MissingConfiguration(names) => format!(
                "LATTICE_HERMES_RUNTIME_PREFLIGHT_MISSING_CONFIGURATION:{}",
                names.join(",")
            ),
            Self::ConfigurationRejected => {
                "LATTICE_HERMES_RUNTIME_PREFLIGHT_CONFIGURATION_REJECTED".to_owned()
            }
            Self::ConfigurationPresentUnverified => {
                "LATTICE_HERMES_RUNTIME_PREFLIGHT_CONFIGURATION_PRESENT_UNVERIFIED".to_owned()
            }
        }
    }
}

/// Fixed, redacted result of the standalone Graphify runtime identity check.
/// This check never starts Graphify, PostgreSQL, Hermes, or a delivery run.
#[derive(Debug, Eq, PartialEq)]
pub enum GraphifyRuntimePreflight {
    MissingConfiguration(Vec<&'static str>),
    ConfigurationRejected,
    IdentityVerified,
}

impl GraphifyRuntimePreflight {
    /// Renders one stable, stderr-safe record.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::MissingConfiguration(names) => format!(
                "LATTICE_GRAPHIFY_RUNTIME_PREFLIGHT_MISSING_CONFIGURATION:{}",
                names.join(",")
            ),
            Self::ConfigurationRejected => {
                "LATTICE_GRAPHIFY_RUNTIME_PREFLIGHT_CONFIGURATION_REJECTED".to_owned()
            }
            Self::IdentityVerified => {
                "LATTICE_GRAPHIFY_RUNTIME_PREFLIGHT_IDENTITY_VERIFIED".to_owned()
            }
        }
    }

    /// Returns true only when the pinned runtime identity was verified.
    #[must_use]
    pub const fn is_identity_verified(&self) -> bool {
        matches!(self, Self::IdentityVerified)
    }
}

/// Verifies an independently configured, pinned Graphify runtime without starting it.
/// The runtime root is deliberately supplied outside any historical delivery fixture.
#[must_use]
pub fn graphify_runtime_preflight_from_environment() -> GraphifyRuntimePreflight {
    const REQUIRED: [&str; 2] = ["LATTICE_GRAPHIFY_RUNTIME_ROOT", "LATTICE_GRAPHIFY_WSL_EXE"];
    let missing = REQUIRED
        .into_iter()
        .filter(|name| std::env::var_os(name).is_none())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return GraphifyRuntimePreflight::MissingConfiguration(missing);
    }

    let result = (|| {
        let runtime_root = PathBuf::from(
            std::env::var_os("LATTICE_GRAPHIFY_RUNTIME_ROOT")
                .ok_or_else(|| LatticedError::new(LatticedErrorKind::GraphConfiguration))?,
        );
        let wsl_executable = PathBuf::from(
            std::env::var_os("LATTICE_GRAPHIFY_WSL_EXE")
                .ok_or_else(|| LatticedError::new(LatticedErrorKind::GraphConfiguration))?,
        );
        let staging_root = runtime_root.join(".lattice-preflight-staging");
        GraphifyRuntimeConfig::new(
            wsl_executable,
            runtime_root,
            staging_root,
            Duration::from_secs(30),
            GraphOutputLimits::default(),
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
        Ok::<(), LatticedError>(())
    })();

    if result.is_ok() {
        GraphifyRuntimePreflight::IdentityVerified
    } else {
        GraphifyRuntimePreflight::ConfigurationRejected
    }
}

/// Validates only the secret-free runtime and isolation configuration without
/// launching WSL, Hermes, a broker, a provider, or any child process.
#[must_use]
pub fn hermes_runtime_preflight_from_environment() -> HermesRuntimePreflight {
    #[cfg(not(windows))]
    {
        HermesRuntimePreflight::ConfigurationRejected
    }
    #[cfg(windows)]
    {
        const REQUIRED: [&str; 7] = [
            "LATTICE_HERMES_PREPARATION_ROOT",
            "LATTICE_HERMES_PREPARATION_RECEIPT_SHA256",
            "LATTICE_HERMES_RUNTIME_MANIFEST",
            "LATTICE_HERMES_RUNTIME_GUEST_ROOT",
            "LATTICE_HERMES_PRODUCT_ROOT",
            "LATTICE_HERMES_WSL_EXE",
            "LATTICE_HERMES_ISOLATION_ROOT",
        ];
        let missing = REQUIRED
            .into_iter()
            .filter(|name| std::env::var_os(name).is_none())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return HermesRuntimePreflight::MissingConfiguration(missing);
        }

        let result = (|| {
            let product_root = PathBuf::from(hermes_environment("LATTICE_HERMES_PRODUCT_ROOT")?);
            let preparation_root =
                PathBuf::from(hermes_environment("LATTICE_HERMES_PREPARATION_ROOT")?);
            let preparation_receipt =
                hermes_environment("LATTICE_HERMES_PREPARATION_RECEIPT_SHA256")?;
            verify_official_preparation_for_launch(
                &preparation_root,
                &product_root,
                &preparation_receipt,
            )
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesPreparationRequired))?;
            let runtime_manifest_path =
                PathBuf::from(hermes_environment("LATTICE_HERMES_RUNTIME_MANIFEST")?);
            let runtime_manifest_bytes =
                read_regular_file(&runtime_manifest_path, MAX_HERMES_RUNTIME_MANIFEST_BYTES)
                    .map_err(|_| {
                        LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired)
                    })?;
            let runtime_manifest =
                HermesOfflineRuntimeManifest::from_canonical_json(&runtime_manifest_bytes)
                    .map_err(|_| {
                        LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired)
                    })?;
            let runtime_guest_root = hermes_environment("LATTICE_HERMES_RUNTIME_GUEST_ROOT")?;
            validate_official_hermes_runtime_identity(
                &runtime_guest_root,
                &runtime_manifest_bytes,
                &runtime_manifest,
            )?;
            HermesWslContainmentConfig::new(
                PathBuf::from(hermes_environment("LATTICE_HERMES_WSL_EXE")?),
                runtime_guest_root,
                PathBuf::from(hermes_environment("LATTICE_HERMES_ISOLATION_ROOT")?),
                product_root,
            )
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
            Ok::<(), LatticedError>(())
        })();
        if result.is_ok() {
            HermesRuntimePreflight::ConfigurationPresentUnverified
        } else {
            HermesRuntimePreflight::ConfigurationRejected
        }
    }
}

impl HermesProductionPreflight {
    /// Renders one stable, stderr-safe record.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::MissingConfiguration(names) => format!(
                "LATTICE_HERMES_PREFLIGHT_MISSING_CONFIGURATION:{}",
                names.join(",")
            ),
            Self::ConfigurationRejected => {
                "LATTICE_HERMES_PREFLIGHT_CONFIGURATION_REJECTED".to_owned()
            }
            Self::ConfigurationPresentUnverified => {
                "LATTICE_HERMES_PREFLIGHT_CONFIGURATION_PRESENT_UNVERIFIED".to_owned()
            }
        }
    }
}

fn classify_hermes_production_preflight(
    missing: Vec<&'static str>,
    configuration_passed_static_validation: bool,
) -> HermesProductionPreflight {
    if !missing.is_empty() {
        HermesProductionPreflight::MissingConfiguration(missing)
    } else if configuration_passed_static_validation {
        HermesProductionPreflight::ConfigurationPresentUnverified
    } else {
        HermesProductionPreflight::ConfigurationRejected
    }
}

/// Checks the local Hermes production prerequisite without launching a process
/// or accessing a network, database, MCP, or provider.
#[must_use]
pub fn hermes_production_preflight_from_environment() -> HermesProductionPreflight {
    #[cfg(not(windows))]
    {
        classify_hermes_production_preflight(Vec::new(), false)
    }
    #[cfg(windows)]
    {
        const REQUIRED: [&str; 12] = [
            "LATTICE_HERMES_PREPARATION_ROOT",
            "LATTICE_HERMES_PREPARATION_RECEIPT_SHA256",
            "LATTICE_HERMES_RUNTIME_MANIFEST",
            "LATTICE_HERMES_RUNTIME_GUEST_ROOT",
            "LATTICE_HERMES_API_KEY",
            "LATTICE_HERMES_PRODUCT_ROOT",
            "LATTICE_HERMES_WSL_EXE",
            "LATTICE_HERMES_ISOLATION_ROOT",
            "LATTICE_HERMES_CODEX_LAUNCHER",
            "LATTICE_HERMES_CODEX_HOME",
            "LATTICE_HERMES_BROKER_ISOLATION_ROOT",
            "LATTICE_HERMES_DEADLINE_SECONDS",
        ];
        let missing = REQUIRED
            .into_iter()
            .filter(|name| std::env::var_os(name).is_none())
            .collect::<Vec<_>>();
        let configuration_passed_static_validation =
            missing.is_empty() && HermesEnvironmentConfig::from_environment().is_ok();
        classify_hermes_production_preflight(missing, configuration_passed_static_validation)
    }
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

/// Process-owned choice between ordinary Runtime work and an explicit
/// cross-module integration run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeIntegrationMode {
    CoreOnly,
    Graphify,
    GraphifyHermes,
}

impl RuntimeIntegrationMode {
    const fn uses_graphify(self) -> bool {
        !matches!(self, Self::CoreOnly)
    }

    const fn uses_hermes(self) -> bool {
        matches!(self, Self::GraphifyHermes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Task050AcceptanceProfile {
    AskUser,
    Proceed,
}

impl Task050AcceptanceProfile {
    const fn identity(self) -> GatewaySubmissionIdentity<'static> {
        match self {
            Self::AskUser => GatewaySubmissionIdentity {
                project: TASK050_ASK_USER_PROJECT_ID,
                snapshot: TASK050_ASK_USER_PROJECT_SNAPSHOT_ID,
                task: TASK050_ASK_USER_TASK_ID,
            },
            Self::Proceed => GatewaySubmissionIdentity {
                project: TASK050_PROCEED_PROJECT_ID,
                snapshot: TASK050_PROCEED_PROJECT_SNAPSHOT_ID,
                task: TASK050_PROCEED_TASK_ID,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct Task050AcceptanceSelectorInput<'a> {
    profile: Option<&'a str>,
    task_spec_sha256: Option<&'a str>,
    task050_live: Option<&'a str>,
    task019_live: Option<&'a str>,
    phase: Option<&'a str>,
    host: Option<&'a str>,
    run_id: Option<&'a str>,
    ingress_kind: Option<&'a str>,
}

fn select_task050_acceptance_profile(
    run_mode: FullChainRunMode,
    input: Task050AcceptanceSelectorInput<'_>,
) -> Result<Option<Task050AcceptanceProfile>, LatticedErrorKind> {
    let profile = match (input.profile, input.task_spec_sha256) {
        (None, None) => return Ok(None),
        (Some("ASK_USER"), Some(_)) => Task050AcceptanceProfile::AskUser,
        (Some("PROCEED"), Some(_)) => Task050AcceptanceProfile::Proceed,
        _ => return Err(LatticedErrorKind::Configuration),
    };
    if run_mode != FullChainRunMode::ResumeExisting
        || input.task050_live != Some("1")
        || input.task019_live != Some("1")
        || !matches!(input.phase, Some("initial" | "restart"))
        || input.host != Some("127.0.0.1")
        || !input.run_id.is_some_and(|run_id| {
            run_id.len() == 32
                && run_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        || input.ingress_kind != Some(TASK_INGRESS_LOCAL_ACCEPTANCE)
    {
        return Err(LatticedErrorKind::Configuration);
    }
    let submission = match task050_acceptance_gateway_submission(profile) {
        Ok(submission) => submission,
        Err(error) => return Err(error.kind()),
    };
    if input.task_spec_sha256 != Some(submission.binding().task_spec_digest().as_str()) {
        return Err(LatticedErrorKind::Configuration);
    }
    Ok(Some(profile))
}

fn optional_unicode_environment(name: &str) -> Result<Option<String>, LatticedError> {
    match env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => {
            Err(LatticedError::new(LatticedErrorKind::Configuration))
        }
    }
}

fn gateway_submission_from_environment(
    run_mode: FullChainRunMode,
) -> Result<TaskSpecSubmission, LatticedError> {
    let profile = optional_unicode_environment(TASK050_ACCEPTANCE_PROFILE_ENV)?;
    let task_spec_sha256 = optional_unicode_environment(TASK050_ACCEPTANCE_TASK_SPEC_SHA256_ENV)?;
    if profile.is_none() && task_spec_sha256.is_none() {
        return fixed_gateway_submission();
    }
    let task050_live = optional_unicode_environment("LATTICE_TASK050_LIVE")?;
    let task019_live = optional_unicode_environment("LATTICE_TASK019_LIVE")?;
    let phase = optional_unicode_environment("LATTICE_TASK019_PHASE")?;
    let host = optional_unicode_environment("LATTICE_TASK019_HOST")?;
    let run_id = optional_unicode_environment("LATTICE_TASK019_RUN_ID")?;
    let ingress_kind = optional_unicode_environment(TASK_INGRESS_KIND_ENV)?;
    let selected = select_task050_acceptance_profile(
        run_mode,
        Task050AcceptanceSelectorInput {
            profile: profile.as_deref(),
            task_spec_sha256: task_spec_sha256.as_deref(),
            task050_live: task050_live.as_deref(),
            task019_live: task019_live.as_deref(),
            phase: phase.as_deref(),
            host: host.as_deref(),
            run_id: run_id.as_deref(),
            ingress_kind: ingress_kind.as_deref(),
        },
    )
    .map_err(LatticedError::new)?;
    selected.map_or_else(
        fixed_gateway_submission,
        task050_acceptance_gateway_submission,
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CanonicalHermesMode {
    TaskOnly,
    Production,
}

const CONTROLLED_TASK_SCHEMA_OUTPUT_CHILD: &str = "codex-schema-output";

fn controlled_task_delivery_config(
    configured: &LatticedDeliveryConfig,
    delivery_root: &Path,
    run_mode: FullChainRunMode,
) -> Option<LatticedDeliveryConfig> {
    if run_mode == FullChainRunMode::ResumeExisting {
        return None;
    }
    let mut config = configured.clone();
    config.delivery_root = delivery_root.to_path_buf();
    config.schema_directory = delivery_root.join(CONTROLLED_TASK_SCHEMA_OUTPUT_CHILD);
    Some(config)
}

fn controlled_submit_delivery_root(
    configured_root: &Path,
    task_identity: &ContentDigest,
    run_mode: FullChainRunMode,
) -> Result<PathBuf, LatticedError> {
    if run_mode == FullChainRunMode::ResumeExisting {
        return Ok(configured_root.to_path_buf());
    }
    let delivery_root = configured_root.join(format!("task-{}", task_identity.as_str()));
    match fs::symlink_metadata(&delivery_root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(delivery_root),
        Ok(_) | Err(_) => Err(LatticedError::new(
            LatticedErrorKind::WorkspaceConfiguration,
        )),
    }
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

fn parse_runtime_integration_mode(
    value: Option<&str>,
) -> Result<RuntimeIntegrationMode, LatticedError> {
    match value {
        None | Some("CORE_ONLY") => Ok(RuntimeIntegrationMode::CoreOnly),
        Some("GRAPHIFY") => Ok(RuntimeIntegrationMode::Graphify),
        // Keep the legacy spelling readable, but express the new composition
        // in terms of the independently degradable components.
        Some("GRAPHIFY_HERMES") | Some("FULL_CHAIN") => Ok(RuntimeIntegrationMode::GraphifyHermes),
        Some(_) => Err(LatticedError::new(LatticedErrorKind::Configuration)),
    }
}

fn runtime_integration_mode_from_environment() -> Result<RuntimeIntegrationMode, LatticedError> {
    match env::var("LATTICE_RUNTIME_INTEGRATION") {
        Ok(value) => parse_runtime_integration_mode(Some(&value)),
        Err(env::VarError::NotPresent) => parse_runtime_integration_mode(None),
        Err(env::VarError::NotUnicode(_)) => {
            Err(LatticedError::new(LatticedErrorKind::Configuration))
        }
    }
}

fn canonical_hermes_mode_from_environment() -> Result<CanonicalHermesMode, LatticedError> {
    match env::var("LATTICE_HERMES_MODE") {
        Ok(value) => canonical_hermes_mode_from_value(Some(&value)),
        Err(env::VarError::NotPresent) => canonical_hermes_mode_from_value(None),
        Err(env::VarError::NotUnicode(_)) => {
            Err(LatticedError::new(LatticedErrorKind::Configuration))
        }
    }
}

fn canonical_hermes_mode_from_value(
    value: Option<&str>,
) -> Result<CanonicalHermesMode, LatticedError> {
    match value {
        None | Some("TASK_ONLY") => Ok(CanonicalHermesMode::TaskOnly),
        Some("PRODUCTION") => Ok(CanonicalHermesMode::Production),
        Some(_) => Err(LatticedError::new(LatticedErrorKind::Configuration)),
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
    integration_mode: RuntimeIntegrationMode,
    process_start_identity: ContentDigest,
    task_ingress_peer: TaskIngressPeerEvidence,
    store_authority: StoreAuthorityHead,
}

fn load_canonical_reflection<L>(
    request: &GraphMemoryRunRequest,
    load_reflection: L,
) -> Result<HermesReflectionReceipt, LatticedError>
where
    L: FnOnce(&GraphMemoryRunRequest) -> Result<HermesReflectionReceipt, GraphMemoryPortError>,
{
    load_reflection(request).map_err(|error| map_reflection_read_error(&error))
}

fn map_hermes_research_error(error: &PortError) -> LatticedError {
    if error.kind() == PortErrorKind::Ambiguous
        && error.code() == "HERMES_RUN_RECONCILIATION_REQUIRED"
    {
        LatticedError::new(LatticedErrorKind::ReconciliationRequired)
    } else {
        LatticedError::new(LatticedErrorKind::HermesExecution)
    }
}

fn load_or_run_canonical_reflection<H, L, G, P>(
    hermes: &mut H,
    run_id: &str,
    request: &GraphMemoryRunRequest,
    mut load_reflection: L,
    load_graph_receipt: G,
    persist_reflection: P,
) -> Result<HermesReflectionReceipt, LatticedError>
where
    H: FullChainHermesPort,
    L: FnMut(&GraphMemoryRunRequest) -> Result<HermesReflectionReceipt, GraphMemoryPortError>,
    G: FnOnce(&GraphMemoryRunRequest) -> Result<GraphMemoryReceipt, LatticedError>,
    P: FnOnce(&HermesReflectionCandidate) -> Result<HermesReflectionReceipt, LatticedError>,
{
    match load_reflection(request) {
        Ok(receipt) => return Ok(receipt),
        Err(error)
            if error.kind() == PortErrorKind::Unavailable
                && error.code() == "MEMORY_RECEIPT_UNAVAILABLE" => {}
        Err(error) => return Err(map_reflection_read_error(&error)),
    }

    let graph_receipt = load_graph_receipt(request)?;
    let hermes_request = hermes_request_for_graph(run_id, request, &graph_receipt)?;
    let output = hermes
        .research_canonical(&hermes_request, request, &graph_receipt)
        .map_err(|error| map_hermes_research_error(&error))?;
    let candidate = output.into_candidate();
    let persisted = persist_reflection(&candidate)?;
    let replayed = load_reflection(request).map_err(|error| map_reflection_read_error(&error))?;
    if replayed != persisted {
        return Err(LatticedError::new(LatticedErrorKind::HermesReceiptRead));
    }
    Ok(replayed)
}

impl<H: FullChainHermesPort> FullChainCore<H> {
    fn runtime_status_json(&mut self) -> Result<Value, LatticedError> {
        let mut base = self.delivery.core_status_json()?;
        let object = base
            .as_object_mut()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
        object.insert(
            "runtime_integration".to_owned(),
            Value::String(
                match self.integration_mode {
                    RuntimeIntegrationMode::CoreOnly => "CORE_ONLY",
                    RuntimeIntegrationMode::Graphify => "GRAPHIFY",
                    RuntimeIntegrationMode::GraphifyHermes => "GRAPHIFY_HERMES",
                }
                .to_owned(),
            ),
        );
        let graphify_status = if self.integration_mode.uses_graphify() {
            match graphify_runtime_preflight_from_environment() {
                GraphifyRuntimePreflight::IdentityVerified => "READY",
                GraphifyRuntimePreflight::MissingConfiguration(_)
                | GraphifyRuntimePreflight::ConfigurationRejected => "DEGRADED",
            }
        } else {
            "DEFERRED"
        };
        let hermes_status = if self.integration_mode.uses_hermes() {
            match hermes_runtime_preflight_from_environment() {
                HermesRuntimePreflight::ConfigurationPresentUnverified => "PREPARED",
                HermesRuntimePreflight::MissingConfiguration(_)
                | HermesRuntimePreflight::ConfigurationRejected => "DEGRADED",
            }
        } else {
            "DEFERRED"
        };
        object.insert(
            "graphify_runtime_status".to_owned(),
            Value::String(graphify_status.to_owned()),
        );
        object.insert(
            "hermes_runtime_status".to_owned(),
            Value::String(hermes_status.to_owned()),
        );
        Ok(base)
    }

    fn status_json(&mut self, entry: FullChainEntry) -> Result<Value, LatticedError> {
        if !self.integration_mode.uses_graphify() {
            return self.delivery.core_status_json();
        }
        let base = self.delivery.status_json()?;
        if !self.integration_mode.uses_hermes() {
            return Ok(base);
        }
        let request = graph_request_from_json(self.delivery.database.run_id(), &base)?;
        let reflection = load_canonical_reflection(&request, |request| {
            load_reflection_from_postgres(
                &self.delivery.database,
                &self.delivery.password,
                self.delivery.timeout,
                request,
            )
        })?;
        append_full_chain_json(base, &reflection, entry)
    }

    fn run_task_json(
        &mut self,
        binding: &SubjectBinding,
        writer_authority: &WriterLeaseAuthorityHead,
        writer_guard: &mut dyn WriterAuthorityGuardPort,
        delivery_root: &Path,
    ) -> Result<Value, LatticedError> {
        match self.run_mode {
            FullChainRunMode::Fresh => self.delivery.run_task_json(
                binding,
                &self.store_authority,
                writer_authority,
                writer_guard,
                delivery_root,
            ),
            FullChainRunMode::ResumeExisting => self.delivery.status_task_json(binding),
        }
    }

    fn status_task_json(&mut self, binding: &SubjectBinding) -> Result<Value, LatticedError> {
        self.delivery.status_task_json(binding)
    }

    fn run_task_downstream_json(
        &mut self,
        entry: FullChainEntry,
        binding: &SubjectBinding,
    ) -> Result<Value, LatticedError> {
        if !self.integration_mode.uses_graphify() {
            return self.delivery.status_task_json(binding);
        }
        let base = self.delivery.run_task_downstream_json(binding)?;
        if !self.integration_mode.uses_hermes() {
            return Ok(base);
        }
        match self.load_or_run_reflection(&base) {
            Ok(reflection) => append_full_chain_json(base, &reflection, entry),
            Err(error) => append_optional_component_degraded_json(base, "hermes", error),
        }
    }

    fn status_task_downstream_json(
        &mut self,
        entry: FullChainEntry,
        binding: &SubjectBinding,
    ) -> Result<Value, LatticedError> {
        if !self.integration_mode.uses_graphify() {
            return self.delivery.status_task_json(binding);
        }
        let base = self.delivery.status_task_downstream_json(binding)?;
        if !self.integration_mode.uses_hermes() {
            return Ok(base);
        }
        let request = graph_request_from_json(self.delivery.database.run_id(), &base)?;
        let reflection = load_canonical_reflection(&request, |request| {
            load_reflection_from_postgres(
                &self.delivery.database,
                &self.delivery.password,
                self.delivery.timeout,
                request,
            )
        });
        match reflection {
            Ok(receipt) => append_full_chain_json(base, &receipt, entry),
            Err(error) => append_optional_component_degraded_json(base, "hermes", error),
        }
    }

    fn load_or_run_reflection(
        &mut self,
        base: &Value,
    ) -> Result<HermesReflectionReceipt, LatticedError> {
        let request = graph_request_from_json(self.delivery.database.run_id(), base)?;
        let database = &self.delivery.database;
        let password = &self.delivery.password;
        let timeout = self.delivery.timeout;
        let run_id = database.run_id().to_owned();
        load_or_run_canonical_reflection(
            &mut self.hermes,
            &run_id,
            &request,
            |request| load_reflection_from_postgres(database, password, timeout, request),
            |request| load_delivery_graph_receipt(database, password, deadline(timeout)?, request),
            |candidate| persist_reflection_to_postgres(database, password, timeout, candidate),
        )
    }
}

struct FullChainTaskExecution<'a, H> {
    core: &'a mut FullChainCore<H>,
    task_identity: ContentDigest,
}

impl<H: FullChainHermesPort> ControlledTaskExecutionPort for FullChainTaskExecution<'_, H> {
    fn execute(
        &mut self,
        binding: &SubjectBinding,
        writer_authority: &WriterLeaseAuthorityHead,
        writer_guard: &mut dyn WriterAuthorityGuardPort,
    ) -> Result<ContentDigest, ControlledTaskExecutionError> {
        let delivery_root = controlled_submit_delivery_root(
            &self
                .core
                .delivery
                .delivery
                .as_ref()
                .ok_or_else(|| {
                    ControlledTaskExecutionError::new(
                        ControlledTaskExecutionErrorKind::Known,
                        LatticedErrorKind::Configuration.code(),
                    )
                })?
                .delivery_root,
            &self.task_identity,
            self.core.run_mode,
        )
        .map_err(|error| {
            ControlledTaskExecutionError::new(
                controlled_execution_error_kind(error.kind()),
                error.code(),
            )
        })?;
        let value = self
            .core
            .run_task_json(binding, writer_authority, writer_guard, &delivery_root)
            .map_err(|error| {
                let kind = controlled_execution_error_kind(error.kind());
                ControlledTaskExecutionError::new(kind, error.code())
            })?;
        delivery_receipt_digest(&value).map_err(|error| {
            ControlledTaskExecutionError::new(
                ControlledTaskExecutionErrorKind::Ambiguous,
                error.code(),
            )
        })
    }
}

const fn controlled_execution_error_kind(
    kind: LatticedErrorKind,
) -> ControlledTaskExecutionErrorKind {
    match kind {
        LatticedErrorKind::Configuration
        | LatticedErrorKind::DatabaseSecret
        | LatticedErrorKind::LedgerConfiguration
        | LatticedErrorKind::WorkspaceConfiguration
        | LatticedErrorKind::CodexConfiguration
        | LatticedErrorKind::Contract
        | LatticedErrorKind::OfficialLiveBlocked
        | LatticedErrorKind::ScriptedFixtureRejected
        | LatticedErrorKind::DeliveryFailed => ControlledTaskExecutionErrorKind::Known,
        _ => ControlledTaskExecutionErrorKind::Ambiguous,
    }
}

fn task_lifecycle<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    binding: &SubjectBinding,
) -> TaskLifecycleResult<PostgresTaskLifecycle> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| {
            TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "LATTICE_MCP_OBSERVED_EFFECT_REJECTED",
            )
        })?;
    PostgresTaskLifecycle::connect_with_ingress_peer(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout).map_err(|_| {
            TaskLifecycleError::new(
                TaskLifecycleErrorKind::Unavailable,
                "LATTICE_TASK_LEDGER_DEADLINE_REJECTED",
            )
        })?,
        task_ledger_identity(binding).map_err(|_| {
            TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "LATTICE_TASK_LEDGER_IDENTITY_REJECTED",
            )
        })?,
        core.store_authority.clone(),
        core.task_ingress_peer.clone(),
    )
}

fn configured_store_authority() -> Result<StoreAuthorityHead, LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::LedgerConfiguration);
    let daemon_instance_id = StoreDaemonInstanceId::new(
        required_environment(STORE_DAEMON_INSTANCE_ID_ENV).map_err(|_| rejected())?,
    )
    .map_err(|_| rejected())?;
    let daemon_epoch = required_environment(STORE_DAEMON_EPOCH_ENV)
        .map_err(|_| rejected())?
        .parse::<u64>()
        .ok()
        .and_then(|value| lattice_contracts::DaemonEpoch::new(value).ok())
        .ok_or_else(rejected)?;
    let authority_revision = required_environment(STORE_AUTHORITY_REVISION_ENV)
        .map_err(|_| rejected())?
        .parse::<u64>()
        .ok()
        .and_then(|value| StoreAuthorityRevision::new(value).ok())
        .ok_or_else(rejected)?;
    let observation_digest = ContentDigest::from_sha256(
        required_environment(STORE_OBSERVATION_DIGEST_ENV).map_err(|_| rejected())?,
    )
    .map_err(|_| rejected())?;
    let head_digest = ContentDigest::from_sha256(
        required_environment(STORE_AUTHORITY_HEAD_DIGEST_ENV).map_err(|_| rejected())?,
    )
    .map_err(|_| rejected())?;
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        daemon_instance_id,
        daemon_epoch,
        RuntimeAdmissionMode::Active,
        authority_revision,
        observation_digest,
        head_digest,
    )
    .map_err(|_| rejected())
}

fn configured_task_ingress_peer(
    process_start_identity: &ContentDigest,
) -> Result<TaskIngressPeerEvidence, LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::Configuration);
    let ingress_kind = required_environment(TASK_INGRESS_KIND_ENV)?;
    let profile_digest =
        ContentDigest::from_sha256(required_environment(TASK_INGRESS_PROFILE_DIGEST_ENV)?)
            .map_err(|_| rejected())?;
    let executable = env::current_exe().map_err(|_| rejected())?;
    let adapter_binary_digest = ContentDigest::from_sha256(official_file_sha256(
        &executable,
        MAX_LATTICED_EXECUTABLE_BYTES,
    )?)
    .map_err(|_| rejected())?;
    let schema_digest = mcp::task_ingress_schema_digest().ok_or_else(rejected)?;
    let channel_id = GatewayChannelId::new("main").map_err(|_| rejected())?;

    match ingress_kind.as_str() {
        TASK_INGRESS_SECURE_TUNNEL => TaskIngressPeerEvidence::new_chatgpt_secure_mcp_tunnel_live(
            GatewayInstanceId::new("latticed-chatgpt-secure-mcp").map_err(|_| rejected())?,
            env!("CARGO_PKG_VERSION"),
            adapter_binary_digest,
            schema_digest,
            channel_id,
            profile_digest,
            process_start_identity.clone(),
        )
        .map_err(|_| rejected()),
        TASK_INGRESS_LOCAL_ACCEPTANCE => {
            TaskIngressPeerEvidence::new_local_canonical_mcp_acceptance_live(
                GatewayInstanceId::new("latticed-local-canonical-acceptance")
                    .map_err(|_| rejected())?,
                env!("CARGO_PKG_VERSION"),
                adapter_binary_digest,
                schema_digest,
                channel_id,
                profile_digest,
                process_start_identity.clone(),
            )
            .map_err(|_| rejected())
        }
        _ => Err(rejected()),
    }
}

fn task_writer_lease<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    foundation: &TaskPersistenceFoundation,
) -> Result<PostgresWriterLease, LatticedError> {
    let memory_manifest = verify_embedded_extension_manifest()
        .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
    let target = WriterLeaseExtensionTarget::new(
        core.delivery.database.database_name(),
        foundation.database_identity_digest().clone(),
        foundation.global_manifest_digest().clone(),
        memory_manifest.manifest_sha256().clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout)?,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    PostgresWriterLease::new(client, target, &core.store_authority, 600)
        .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))
}

fn controlled_task_request<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    binding: SubjectBinding,
    client_request_id: &str,
) -> Result<ControlledTaskRequest, LatticedError> {
    let invocation = invocation_for_task(core.delivery.database.run_id(), &binding)?;
    let task_suffix = binding.task_spec_digest().as_str()[..24].to_owned();
    ControlledTaskRequest::new(
        binding,
        client_request_id,
        invocation.attempt_id().clone(),
        format!("lattice-mcp-lease-{task_suffix}"),
        "codex-writer",
        format!("lattice-mcp-worktree-{task_suffix}"),
        HolderProcessId::new(u64::from(process::id()))
            .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?,
        core.process_start_identity.clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))
}

fn controlled_task_reference(
    binding: &SubjectBinding,
    admission_command_id: &str,
    run_id: &str,
    ingress_profile_digest: &ContentDigest,
) -> Result<ContentDigest, LatticedError> {
    let value = CanonicalValue::Object(vec![
        (
            "admission_command_id".to_owned(),
            CanonicalValue::String(admission_command_id.to_owned()),
        ),
        (
            "ingress_profile_digest".to_owned(),
            CanonicalValue::String(ingress_profile_digest.as_str().to_owned()),
        ),
        (
            "run_id".to_owned(),
            CanonicalValue::String(run_id.to_owned()),
        ),
        (
            "task_spec_digest".to_owned(),
            CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
        ),
    ]);
    digest("lattice.task.public-reference", &value)
}

fn verified_controlled_task_reference<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    binding: &SubjectBinding,
) -> Result<ContentDigest, LatticedError> {
    let mut lifecycle = task_lifecycle(core, binding)
        .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?;
    let admission_command_id = lifecycle
        .verified_admission_command_id(binding)
        .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?;
    controlled_task_reference(
        binding,
        &admission_command_id,
        core.delivery.database.run_id(),
        core.task_ingress_peer.profile_digest(),
    )
}

fn task_public_status(evidence: &TaskLifecycleEvidence, task_ref: &ContentDigest) -> Value {
    let task_state = if evidence.admitted() {
        evidence.state().as_str()
    } else {
        "NOT_SUBMITTED"
    };
    let status = if evidence.admitted() {
        match evidence.state() {
            TaskState::Completed => "COMPLETED",
            TaskState::Rejected | TaskState::Blocked | TaskState::Failed | TaskState::Cancelled => {
                "FAILED"
            }
            // Submit is synchronous and serialized by the sole coordinator. A
            // separately observable non-terminal state therefore means the
            // owning call ended without a durable terminal projection.
            TaskState::Draft
            | TaskState::AwaitingExecutionApproval
            | TaskState::Preparing
            | TaskState::Executing
            | TaskState::Verifying
            | TaskState::Reviewing
            | TaskState::AwaitingMergeApproval
            | TaskState::Merging
            | TaskState::Stopping => "RECONCILIATION_REQUIRED",
        }
    } else {
        "NOT_SUBMITTED"
    };
    json!({
        "ledger_head_digest": evidence.ledger_head_digest().as_str(),
        "result_digest": evidence.result_digest().map(ContentDigest::as_str),
        "schema_version": "lattice.task.status.v1",
        "status": status,
        "task_ref": task_ref.as_str(),
        "task_state": task_state,
    })
}

const fn expected_completed_writer_history(
    has_current_authority: bool,
    fencing_high_water: u64,
    transition_high_water: u64,
    command_high_water: u64,
) -> bool {
    !has_current_authority
        && fencing_high_water == CONTROLLED_WRITER_FENCING_HIGH_WATER
        && transition_high_water == CONTROLLED_WRITER_TRANSITION_HIGH_WATER
        && command_high_water == CONTROLLED_WRITER_COMMAND_HIGH_WATER
}

const fn expected_merging_writer_history(
    has_current_authority: bool,
    fencing_high_water: u64,
    transition_high_water: u64,
    command_high_water: u64,
) -> bool {
    fencing_high_water == CONTROLLED_WRITER_FENCING_HIGH_WATER
        && if has_current_authority {
            transition_high_water == CONTROLLED_WRITER_ACQUIRED_HIGH_WATER
                && command_high_water == CONTROLLED_WRITER_ACQUIRED_HIGH_WATER
        } else {
            transition_high_water == CONTROLLED_WRITER_TRANSITION_HIGH_WATER
                && command_high_water == CONTROLLED_WRITER_COMMAND_HIGH_WATER
        }
}

fn verify_merging_writer_history(
    writer_lease: &mut PostgresWriterLease,
    evidence: &TaskLifecycleEvidence,
) -> Result<(), LatticedError> {
    if evidence.state() != TaskState::Merging || evidence.result_digest().is_none() {
        return Ok(());
    }

    let project_id = evidence.binding().project_id();
    let persisted = writer_lease
        .inspect_project(project_id)
        .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::TaskReconciliationRequired))?;
    if persisted.project_id() != project_id
        || !expected_merging_writer_history(
            persisted.current_authority().is_some(),
            persisted.fencing_high_water(),
            persisted.transition_high_water(),
            persisted.command_high_water(),
        )
    {
        return Err(LatticedError::new(
            LatticedErrorKind::TaskReconciliationRequired,
        ));
    }
    Ok(())
}

fn verify_completed_writer_history(
    writer_lease: &mut PostgresWriterLease,
    evidence: &TaskLifecycleEvidence,
) -> Result<(), LatticedError> {
    if evidence.state() != TaskState::Completed {
        return Ok(());
    }

    let project_id = evidence.binding().project_id();
    let persisted = writer_lease
        .inspect_project(project_id)
        .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::TaskReconciliationRequired))?;
    if persisted.project_id() != project_id
        || !expected_completed_writer_history(
            persisted.current_authority().is_some(),
            persisted.fencing_high_water(),
            persisted.transition_high_water(),
            persisted.command_high_water(),
        )
    {
        return Err(LatticedError::new(
            LatticedErrorKind::TaskReconciliationRequired,
        ));
    }
    Ok(())
}

fn verified_task_status<H: FullChainHermesPort>(
    core: &mut FullChainCore<H>,
    evidence: &TaskLifecycleEvidence,
    task_ref: &ContentDigest,
) -> Result<Value, LatticedError> {
    if evidence.state() == TaskState::Completed {
        let mut lifecycle = task_lifecycle(core, evidence.binding())
            .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?;
        let foundation = lifecycle
            .persistence_foundation(evidence.binding())
            .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?;
        let mut writer_lease = task_writer_lease(core, &foundation)?;
        verify_completed_writer_history(&mut writer_lease, evidence)?;
        let expected = evidence
            .result_digest()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
        let receipt = core.status_task_json(evidence.binding())?;
        if &delivery_receipt_digest(&receipt)? != expected {
            return Err(LatticedError::new(LatticedErrorKind::ReceiptMismatch));
        }
    }
    Ok(task_public_status(evidence, task_ref))
}

const fn controlled_task_error_code(error: &ControlledTaskOrchestratorError) -> &'static str {
    match error {
        ControlledTaskOrchestratorError::RequestRejected => "LATTICE_TASK_REQUEST_REJECTED",
        ControlledTaskOrchestratorError::Lifecycle(error) => error.code(),
        ControlledTaskOrchestratorError::Lease(error) => error.code(),
        ControlledTaskOrchestratorError::Execution(error) => error.code(),
        ControlledTaskOrchestratorError::StateMismatch => "LATTICE_TASK_STATE_MISMATCH",
        ControlledTaskOrchestratorError::LeaseMismatch => "LATTICE_WRITER_LEASE_MISMATCH",
        ControlledTaskOrchestratorError::ReconciliationRequired => {
            "LATTICE_TASK_RECONCILIATION_REQUIRED"
        }
    }
}

/// Shared service used by both typed MCP tools and typed `OpenClaw` ingress.
pub struct FullChainService<H> {
    inner: Arc<Mutex<FullChainCore<H>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExistingCompletionPolicy {
    Ignore,
    AcceptOrExecute,
    Require,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ControlledWriterDecision {
    Execute,
    ReplayExisting,
}

fn controlled_writer_decision(
    policy: ExistingCompletionPolicy,
    binding: &SubjectBinding,
    existing: Option<&TaskLifecycleEvidence>,
) -> Result<ControlledWriterDecision, ToolExecutionError> {
    if policy == ExistingCompletionPolicy::Ignore {
        return Ok(ControlledWriterDecision::Execute);
    }
    let existing =
        existing.ok_or_else(|| ToolExecutionError::new("LATTICE_TASK_RECONCILIATION_REQUIRED"))?;
    if existing.binding() != binding {
        return Err(ToolExecutionError::new(
            "LATTICE_TASK_RECONCILIATION_REQUIRED",
        ));
    }
    if !existing.admitted() {
        return if policy == ExistingCompletionPolicy::AcceptOrExecute {
            Ok(ControlledWriterDecision::Execute)
        } else {
            Err(ToolExecutionError::new(
                "LATTICE_TASK_RECONCILIATION_REQUIRED",
            ))
        };
    }
    if existing.state() == TaskState::Completed && existing.result_digest().is_some() {
        Ok(ControlledWriterDecision::ReplayExisting)
    } else {
        Err(ToolExecutionError::new(
            "LATTICE_TASK_RECONCILIATION_REQUIRED",
        ))
    }
}

impl<H> Clone for FullChainService<H> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<H: FullChainHermesPort> FullChainService<H> {
    fn finish_hermes_session(
        &self,
        serve_result: Result<(), LatticedError>,
    ) -> Result<(), LatticedError> {
        let mut core = self
            .inner
            .lock()
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesTeardownRejected))?;
        finish_hermes_owner(serve_result, &mut core.hermes)
    }
}

fn finish_hermes_owner<H: FullChainHermesPort>(
    serve_result: Result<(), LatticedError>,
    hermes: &mut H,
) -> Result<(), LatticedError> {
    finish_hermes_session(
        serve_result,
        production_hermes_sealed::Sealed::terminate(hermes),
    )
}

fn finish_hermes_session(
    serve_result: Result<(), LatticedError>,
    teardown_result: Result<(), LatticedError>,
) -> Result<(), LatticedError> {
    match teardown_result {
        Ok(()) => serve_result,
        Err(teardown) => Err(teardown),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CanonicalHermesTool {
    DeliveryRun,
    DeliveryStatus,
    TaskSubmit,
    TaskStatus,
}

fn apply_canonical_hermes_tool_policy<H: FullChainHermesPort>(
    hermes: &mut H,
    run_id: &str,
    tool: CanonicalHermesTool,
) -> Result<(), ToolExecutionError> {
    if tool == CanonicalHermesTool::DeliveryRun
        && production_hermes_sealed::Sealed::is_production_configured(hermes)
    {
        production_hermes_sealed::Sealed::ensure_ready(hermes, run_id)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
    }
    Ok(())
}

impl<H: FullChainHermesPort> FullChainService<H> {
    fn run_delivery_tool_core(
        core: &mut FullChainCore<H>,
        entry: FullChainEntry,
    ) -> Result<Value, ToolExecutionError> {
        if core.run_mode != FullChainRunMode::Fresh {
            return Err(ToolExecutionError::new("LATTICE_DELIVERY_RUN_STATUS_ONLY"));
        }
        let run_id = core.delivery.database.run_id().to_owned();
        apply_canonical_hermes_tool_policy(
            &mut core.hermes,
            &run_id,
            CanonicalHermesTool::DeliveryRun,
        )?;
        let binding = core.submission.binding().clone();
        let submission = core.submission.clone();
        let evidence = Self::run_controlled_writer(
            core,
            &submission,
            "delivery-run-controlled-compatibility",
            ExistingCompletionPolicy::AcceptOrExecute,
        )?;
        let task_ref = verified_controlled_task_reference(core, evidence.binding())
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        verified_task_status(core, &evidence, &task_ref)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        core.run_task_downstream_json(entry, &binding)
            .map_err(|error| ToolExecutionError::new(error.code()))
    }

    fn run_controlled_writer(
        core: &mut FullChainCore<H>,
        submission: &TaskSpecSubmission,
        client_request_id: &str,
        existing_completion_policy: ExistingCompletionPolicy,
    ) -> Result<TaskLifecycleEvidence, ToolExecutionError> {
        let binding = submission.binding().clone();
        let mut lifecycle = task_lifecycle(core, &binding)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let foundation = lifecycle
            .persistence_foundation(&binding)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let mut writer_lease = task_writer_lease(core, &foundation)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let existing = if existing_completion_policy == ExistingCompletionPolicy::Ignore {
            None
        } else {
            Some(
                lifecycle
                    .load(&binding)
                    .map_err(|error| ToolExecutionError::new(error.code()))?,
            )
        };
        match controlled_writer_decision(existing_completion_policy, &binding, existing.as_ref())? {
            ControlledWriterDecision::ReplayExisting => {
                let existing = existing.ok_or_else(|| {
                    ToolExecutionError::new("LATTICE_TASK_RECONCILIATION_REQUIRED")
                })?;
                verify_completed_writer_history(&mut writer_lease, &existing)
                    .map_err(|error| ToolExecutionError::new(error.code()))?;
                return Ok(existing);
            }
            ControlledWriterDecision::Execute => {}
        }
        let preexisting = lifecycle
            .load(&binding)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        verify_merging_writer_history(&mut writer_lease, &preexisting)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let request = controlled_task_request(core, binding.clone(), client_request_id)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let task_identity = controlled_task_reference(
            &binding,
            &task_admission_command_id(client_request_id),
            core.delivery.database.run_id(),
            core.task_ingress_peer.profile_digest(),
        )
        .map_err(|error| ToolExecutionError::new(error.code()))?;
        let outcome = {
            let mut execution = FullChainTaskExecution {
                core,
                task_identity,
            };
            run_controlled_task(&request, &mut lifecycle, &mut writer_lease, &mut execution)
        };
        match outcome {
            Ok(evidence) => Ok(evidence),
            Err(error @ ControlledTaskOrchestratorError::Execution(_)) => {
                let evidence = lifecycle
                    .load(&binding)
                    .map_err(|load| ToolExecutionError::new(load.code()))?;
                if !matches!(
                    evidence.state(),
                    TaskState::Failed | TaskState::Stopping | TaskState::Completed
                ) {
                    return Err(ToolExecutionError::new(controlled_task_error_code(&error)));
                }
                Ok(evidence)
            }
            Err(error @ ControlledTaskOrchestratorError::ReconciliationRequired) => {
                let evidence = lifecycle
                    .load(&binding)
                    .map_err(|load| ToolExecutionError::new(load.code()))?;
                if !evidence.admitted() {
                    return Err(ToolExecutionError::new(controlled_task_error_code(&error)));
                }
                Ok(evidence)
            }
            Err(error) => Err(ToolExecutionError::new(controlled_task_error_code(&error))),
        }
    }

    fn handle_submit(
        core: &mut FullChainCore<H>,
        request: &GatewayRequest,
        submission: &TaskSpecSubmission,
    ) -> GatewayServiceResult<GatewayReply> {
        // The existing OpenClaw peer is intentionally Fake/preflight-only and
        // cannot be promoted into the new live MCP Task ingress authority.
        // Its read-only transport/status surface remains available. A future
        // live OpenClaw submit requires a distinct closed ingress contract and
        // per-call authority binding.
        gateway_reply(
            request,
            GatewayReplyBody::Denied(openclaw_submit_denial(submission, &core.submission)),
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
        let mut lifecycle = task_lifecycle(core, &fixed_binding)
            .map_err(|error| map_task_lifecycle_gateway_error(&error))?;
        let task_evidence = lifecycle
            .load(&fixed_binding)
            .map_err(|error| GatewayServiceError::new(PortErrorKind::Denied, error.code()))?;
        let result = if task_evidence.admitted() {
            core.status_task_downstream_json(FullChainEntry::OpenClawTyped, &fixed_binding)
        } else {
            core.status_json(FullChainEntry::OpenClawTyped)
        }
        .map_err(map_gateway_service_error)?;
        let receipt_digest = runtime_receipt_digest(&result).map_err(map_gateway_service_error)?;
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

fn openclaw_submit_denial(
    submission: &TaskSpecSubmission,
    expected: &TaskSpecSubmission,
) -> GatewayDenialCode {
    if submission == expected {
        GatewayDenialCode::DownstreamDenied
    } else {
        GatewayDenialCode::CommandSubstitution
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
        Self::run_delivery_tool_core(&mut core, FullChainEntry::CodexAppMcp)
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
        let run_id = core.delivery.database.run_id().to_owned();
        apply_canonical_hermes_tool_policy(
            &mut core.hermes,
            &run_id,
            CanonicalHermesTool::DeliveryStatus,
        )?;
        let binding = core.submission.binding().clone();
        let mut lifecycle = task_lifecycle(&core, &binding)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let evidence = lifecycle
            .load(&binding)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        if evidence.admitted() {
            core.status_task_downstream_json(FullChainEntry::CodexAppMcp, &binding)
                .map_err(|error| ToolExecutionError::new(error.code()))
        } else {
            core.status_json(FullChainEntry::CodexAppMcp)
                .map_err(|error| ToolExecutionError::new(error.code()))
        }
    }

    fn runtime_status(
        &mut self,
        arguments: &DeliveryToolArguments,
    ) -> Result<Value, ToolExecutionError> {
        let mut core = self
            .inner
            .lock()
            .map_err(|_| ToolExecutionError::new(LatticedErrorKind::Transport.code()))?;
        if arguments.binding() != core.submission.binding() {
            return Err(ToolExecutionError::new(
                "LATTICE_FULL_CHAIN_BINDING_REJECTED",
            ));
        }
        core.runtime_status_json()
            .map_err(|error| ToolExecutionError::new(error.code()))
    }

    fn task_submit(
        &mut self,
        arguments: &TaskSubmitArguments,
    ) -> Result<Value, ToolExecutionError> {
        if arguments.intent() != mcp::CONTROLLED_CODEX_CANARY_INTENT {
            return Err(ToolExecutionError::new("LATTICE_TASK_REQUEST_REJECTED"));
        }
        let mut core = self
            .inner
            .lock()
            .map_err(|_| ToolExecutionError::new(LatticedErrorKind::Transport.code()))?;
        let run_id = core.delivery.database.run_id().to_owned();
        apply_canonical_hermes_tool_policy(
            &mut core.hermes,
            &run_id,
            CanonicalHermesTool::TaskSubmit,
        )?;
        let existing_completion_policy = match core.run_mode {
            FullChainRunMode::Fresh => ExistingCompletionPolicy::Ignore,
            FullChainRunMode::ResumeExisting => ExistingCompletionPolicy::Require,
        };
        let submission = mcp_gateway_submission(arguments.client_request_id())
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let evidence = Self::run_controlled_writer(
            &mut core,
            &submission,
            arguments.client_request_id(),
            existing_completion_policy,
        )?;
        let task_ref = verified_controlled_task_reference(&core, evidence.binding())
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        verified_task_status(&mut core, &evidence, &task_ref)
            .map_err(|error| ToolExecutionError::new(error.code()))
    }

    fn task_status(
        &mut self,
        arguments: &TaskStatusArguments,
    ) -> Result<Value, ToolExecutionError> {
        let mut core = self
            .inner
            .lock()
            .map_err(|_| ToolExecutionError::new(LatticedErrorKind::Transport.code()))?;
        let run_id = core.delivery.database.run_id().to_owned();
        apply_canonical_hermes_tool_policy(
            &mut core.hermes,
            &run_id,
            CanonicalHermesTool::TaskStatus,
        )?;
        let submission = mcp_gateway_submission(arguments.client_request_id())
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let binding = submission.binding().clone();
        let mut lifecycle = task_lifecycle(&core, &binding)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let evidence = lifecycle
            .load(&binding)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let admission_command_id = lifecycle
            .verified_admission_command_id(&binding)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let task_ref = controlled_task_reference(
            &binding,
            &admission_command_id,
            core.delivery.database.run_id(),
            core.task_ingress_peer.profile_digest(),
        )
        .map_err(|error| ToolExecutionError::new(error.code()))?;
        if arguments.task_ref() != task_ref.as_str() {
            return Err(ToolExecutionError::new("LATTICE_TASK_REFERENCE_REJECTED"));
        }
        verified_task_status(&mut core, &evidence, &task_ref)
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
    mcp_binding: SubjectBinding,
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
        SubjectBinding,
        OpenClawGatewayServer<FullChainService<H>>,
    ) {
        (self.mcp_service, self.mcp_binding, self.openclaw_server)
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
        let (config, database, password) = delivery_environment_for_mode(run_mode)?;
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

/// Starts and owns the production Hermes runner until standard input closes.
///
/// This is the standalone process entry for the same production runner used by
/// the full-chain composition. Closing stdin is the bounded shutdown signal;
/// dropping the owner reaps the contained process tree.
///
/// # Errors
///
/// Returns the existing production configuration or runner failure before
/// reporting a successful process exit.
pub fn launch_hermes_from_environment() -> Result<(), LatticedError> {
    require_hermes_preparation_environment()?;
    #[cfg(not(windows))]
    {
        Err(LatticedError::new(
            LatticedErrorKind::HermesProductionRunnerRequired,
        ))
    }
    #[cfg(windows)]
    {
        let hermes_environment = HermesEnvironmentConfig::from_environment()?;
        launch_hermes_until_eof(
            io::stdin(),
            HERMES_POLL_INTERVAL,
            |run_id| hermes_environment.launch(run_id),
            emit_hermes_launch_ready,
        )
    }
}

fn require_hermes_preparation_environment() -> Result<(), LatticedError> {
    if [
        "LATTICE_HERMES_PRODUCT_ROOT",
        "LATTICE_HERMES_PREPARATION_ROOT",
        "LATTICE_HERMES_PREPARATION_RECEIPT_SHA256",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_none())
    {
        return Err(LatticedError::new(
            LatticedErrorKind::HermesPreparationMissing,
        ));
    }
    Ok(())
}

fn emit_hermes_launch_ready() -> Result<(), LatticedError> {
    let stderr = io::stderr();
    let mut output = stderr.lock();
    writeln!(output, "LATTICE_HERMES_READY")
        .and_then(|()| output.flush())
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))
}

trait HermesStandaloneOwner {
    fn verify_live(&mut self) -> Result<(), LatticedError>;
    fn terminate(self) -> Result<(), LatticedError>;
}

#[cfg(windows)]
impl HermesStandaloneOwner for FullChainHermes {
    fn verify_live(&mut self) -> Result<(), LatticedError> {
        self.ready
            .as_mut()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?
            .verify_live()
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionLivenessRejected))
    }

    fn terminate(mut self) -> Result<(), LatticedError> {
        self.ready
            .take()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?
            .terminate()
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesTeardownRejected))
    }
}

fn launch_hermes_until_eof<R, H, F, G>(
    mut input: R,
    poll_interval: Duration,
    launch: F,
    ready: G,
) -> Result<(), LatticedError>
where
    R: Read + Send + 'static,
    H: HermesStandaloneOwner,
    F: FnOnce(&str) -> Result<H, LatticedError>,
    G: FnOnce() -> Result<(), LatticedError>,
{
    let mut owner = launch("standalone-hermes")?;
    if let Err(failure) = owner.verify_live() {
        return match owner.terminate() {
            Ok(()) => Err(failure),
            Err(teardown) => Err(teardown),
        };
    }
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let (reader_gate_sender, reader_gate_receiver) = std::sync::mpsc::sync_channel(1);
    let (reader_ready_sender, reader_ready_receiver) = std::sync::mpsc::sync_channel(1);
    let reader = thread::Builder::new()
        .name("lattice-hermes-stdin".to_owned())
        .spawn(move || {
            let _ = reader_ready_sender.send(());
            if reader_gate_receiver.recv().is_err() {
                return;
            }
            let result = io::copy(&mut input, &mut io::sink())
                .map(|_| ())
                .map_err(|_| LatticedError::new(LatticedErrorKind::Transport));
            let _ = sender.send(result);
        });
    if reader.is_err() {
        let failure = LatticedError::new(LatticedErrorKind::Transport);
        return match owner.terminate() {
            Ok(()) => Err(failure),
            Err(teardown) => Err(teardown),
        };
    }
    if reader_ready_receiver.recv().is_err() {
        let failure = LatticedError::new(LatticedErrorKind::Transport);
        return match owner.terminate() {
            Ok(()) => Err(failure),
            Err(teardown) => Err(teardown),
        };
    }
    if let Err(failure) = owner.verify_live() {
        return match owner.terminate() {
            Ok(()) => Err(failure),
            Err(teardown) => Err(teardown),
        };
    }
    if let Err(failure) = ready() {
        return match owner.terminate() {
            Ok(()) => Err(failure),
            Err(teardown) => Err(teardown),
        };
    }
    if reader_gate_sender.send(()).is_err() {
        let failure = LatticedError::new(LatticedErrorKind::Transport);
        return match owner.terminate() {
            Ok(()) => Err(failure),
            Err(teardown) => Err(teardown),
        };
    }

    loop {
        match receiver.recv_timeout(poll_interval) {
            Ok(input_result) => {
                let live_result = owner.verify_live();
                return match owner.terminate() {
                    Ok(()) => live_result.and(input_result),
                    Err(teardown) => Err(teardown),
                };
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Err(failure) = owner.verify_live() {
                    return match owner.terminate() {
                        Ok(()) => Err(failure),
                        Err(teardown) => Err(teardown),
                    };
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                let input_failure = LatticedError::new(LatticedErrorKind::Transport);
                return match owner.terminate() {
                    Ok(()) => Err(input_failure),
                    Err(teardown) => Err(teardown),
                };
            }
        }
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
    let (mcp_service, mcp_binding, openclaw_server) = runtime.into_parts();
    openclaw_server
        .local_addr()
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    thread::Builder::new()
        .name("lattice-openclaw-full-chain".to_owned())
        .spawn(move || {
            run_openclaw_pump(openclaw_server, |failure| {
                if fatal_openclaw_pump_error(failure.kind) {
                    eprintln!("{}", LatticedErrorKind::Transport.code());
                    process::exit(2);
                }
                OpenClawPumpControl::Continue
            });
        })
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let input = io::stdin();
    let output = io::stdout();
    mcp::serve_legacy_delivery_observer(mcp_service, mcp_binding, input.lock(), output.lock())
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OpenClawPumpFailure {
    kind: GatewayTransportErrorKind,
}

trait FullChainOpenClawPump: Send + 'static {
    fn pump_once(&mut self) -> Result<(), OpenClawPumpFailure>;
}

impl<S> FullChainOpenClawPump for OpenClawGatewayServer<S>
where
    S: GatewayService + Send + 'static,
{
    fn pump_once(&mut self) -> Result<(), OpenClawPumpFailure> {
        self.serve_once()
            .map_err(|error| OpenClawPumpFailure { kind: error.kind() })
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
    let submission = fixed_gateway_submission()?;
    let openclaw_config = openclaw_config
        .with_frozen_submission(submission.clone())
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let timeout = config.timeout;
    let (mcp_service, mcp_binding) = assemble_full_chain_service_with_mode(
        config,
        database,
        password,
        hermes,
        submission,
        run_mode,
        RuntimeIntegrationMode::GraphifyHermes,
        true,
    )?;
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
        mcp_binding,
        openclaw_server,
    })
}

#[allow(clippy::too_many_arguments)]
fn assemble_full_chain_service_with_mode<H>(
    config: LatticedDeliveryConfig,
    database: &DeliveryDatabaseBinding,
    password: &str,
    hermes: H,
    submission: TaskSpecSubmission,
    run_mode: FullChainRunMode,
    integration_mode: RuntimeIntegrationMode,
    require_production_hermes: bool,
) -> Result<(FullChainService<H>, SubjectBinding), LatticedError>
where
    H: FullChainHermesPort + 'static,
{
    validate_controlled_task_timeout(config.timeout, run_mode)?;
    if config.runtime != DeliveryRuntime::OfficialCodexAppServer {
        return Err(LatticedError::new(LatticedErrorKind::OfficialLiveBlocked));
    }
    let is_production_configured =
        production_hermes_sealed::Sealed::is_production_configured(&hermes);
    if require_production_hermes != is_production_configured {
        return Err(LatticedError::new(
            LatticedErrorKind::HermesProductionRunnerRequired,
        ));
    }
    let store_authority = configured_store_authority()?;
    let mcp_binding = submission.binding().clone();
    let delivery = full_chain_delivery_service(config, database, password, run_mode)?;
    let process_start_identity = daemon_process_start_identity()?;
    let task_ingress_peer = configured_task_ingress_peer(&process_start_identity)?;
    let core = FullChainCore {
        delivery,
        hermes,
        submission,
        run_mode,
        integration_mode,
        process_start_identity,
        task_ingress_peer,
        store_authority,
    };
    Ok((
        FullChainService {
            inner: Arc::new(Mutex::new(core)),
        },
        mcp_binding,
    ))
}

fn validate_controlled_task_timeout(
    timeout: Duration,
    run_mode: FullChainRunMode,
) -> Result<(), LatticedError> {
    if run_mode == FullChainRunMode::Fresh
        && (timeout <= FINALIZATION_RESERVE || timeout > CONTROLLED_TASK_MAX_RUNTIME)
    {
        return Err(LatticedError::new(LatticedErrorKind::Configuration));
    }
    Ok(())
}

/// Returns the one server-owned immutable Task Spec admitted by bounded MCP Task submit.
///
/// # Errors
///
/// Returns a contract failure if canonical hashing or fixed binding construction fails.
pub fn fixed_gateway_submission() -> Result<TaskSpecSubmission, LatticedError> {
    gateway_submission(GatewaySubmissionIdentity {
        project: CONTROLLED_PROJECT_ID,
        snapshot: CONTROLLED_PROJECT_SNAPSHOT_ID,
        task: CONTROLLED_TASK_ID,
    })
}

/// Creates one durable MCP task binding from a client idempotency key.
///
/// A terminal result for one request is never reused as the evidence for a
/// different request; retries with the same key remain idempotent.
fn mcp_gateway_submission(client_request_id: &str) -> Result<TaskSpecSubmission, LatticedError> {
    let request_digest = digest(
        "lattice.mcp.task-submission.v1",
        &CanonicalValue::String(client_request_id.to_owned()),
    )?;
    let task = format!(
        "TASK-MCP-{}",
        request_digest.as_str()[..24].to_ascii_uppercase()
    );
    gateway_submission(GatewaySubmissionIdentity {
        project: CONTROLLED_PROJECT_ID,
        snapshot: CONTROLLED_PROJECT_SNAPSHOT_ID,
        task: &task,
    })
}

#[derive(Clone, Copy)]
struct GatewaySubmissionIdentity<'a> {
    project: &'a str,
    snapshot: &'a str,
    task: &'a str,
}

fn task050_acceptance_gateway_submission(
    profile: Task050AcceptanceProfile,
) -> Result<TaskSpecSubmission, LatticedError> {
    gateway_submission(profile.identity())
}

fn gateway_submission(
    identity: GatewaySubmissionIdentity<'_>,
) -> Result<TaskSpecSubmission, LatticedError> {
    let task_spec = TaskSpec::new(TaskSpecInput {
        schema_version: TASK_SPEC_SCHEMA_VERSION.to_owned(),
        task_id: TaskId::new(identity.task)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        revision: FIXED_GATEWAY_TASK_REVISION.to_owned(),
        created_at: "2026-08-09T00:00:00Z".to_owned(),
        created_by: "chatgpt-mcp-controlled-profile".to_owned(),
        project_id: identity.project.to_owned(),
        project_snapshot_id: ProjectSnapshotId::new(identity.snapshot)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        base_ref: "main".to_owned(),
        base_commit_id: BASELINE_COMMIT_SHA.to_owned(),
        goal: "Create answer.txt with the exact approved LATTICE delivery sentinel.".to_owned(),
        non_goals: vec![
            "Do not modify any other product path.".to_owned(),
            "Do not deploy, publish, or merge a protected branch.".to_owned(),
        ],
        risk_class: RiskClass::R0,
        depends_on: Vec::new(),
        scope: TaskScope {
            allowed_paths: vec!["answer.txt".to_owned()],
            forbidden_paths: vec![".git/**".to_owned()],
            allowed_operations: vec![ScopeOperation::Create],
        },
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-038-CANARY".to_owned(),
            description: "The controlled writer creates only the approved sentinel file."
                .to_owned(),
            evidence_type: EvidenceType::Test,
            expected_result: "answer.txt is exactly LATTICE_DELIVERY_OK followed by LF.".to_owned(),
        }],
        verification_commands: vec!["git-diff-no-index-exact-answer-v1".to_owned()],
        required_checks: vec![RequiredCheck::Scope, RequiredCheck::Test],
        requested_capabilities: [
            Capability::ReadRepository,
            Capability::WriteProductCode,
            Capability::RunTests,
            Capability::GitWorktree,
            Capability::UseCodex,
        ]
        .into_iter()
        .map(|capability| CapabilityRequest {
            capability,
            contract_version: "1".to_owned(),
        })
        .collect(),
        budget: TaskBudget {
            accounting_currency: "TWD".to_owned(),
            max_agents: "1".to_owned(),
            max_duration_seconds: "300".to_owned(),
            max_attempts: "1".to_owned(),
            max_model_calls: "1".to_owned(),
            max_external_cost: "0".to_owned(),
        },
        runtime_profile: RuntimeProfile::Codex,
        network_policy: NetworkPolicy::LoopbackOnly,
        deployment_policy: DeploymentPolicy::Deny,
        approval_requirements: ApprovalRequirements {
            execution: ApprovalRequirement::NotRequired,
            merge: ApprovalRequirement::NotRequired,
            protected_release: ApprovalRequirement::ProtectedGuardian,
        },
    })
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    let bytes = task_spec
        .canonical_document()
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    let digest = task_spec_document_digest(&bytes)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    if digest.as_str() != task_spec.spec_hash().to_hex() {
        return Err(LatticedError::new(LatticedErrorKind::Contract));
    }
    let binding = SubjectBinding::new(
        ProjectId::new(identity.project)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        ProjectSnapshotId::new(identity.snapshot)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        TaskId::new(identity.task).map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        FIXED_GATEWAY_TASK_REVISION,
        digest.clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    TaskSpecSubmission::new(binding, bytes, digest)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
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

/// Optional analysis must never erase a verified PostgreSQL receipt.  The
/// caller receives a bounded, machine-readable degradation signal instead.
fn append_optional_component_degraded_json(
    mut base: Value,
    component: &'static str,
    error: LatticedError,
) -> Result<Value, LatticedError> {
    let object = base
        .as_object_mut()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    object.insert(
        format!("{component}_status"),
        Value::String("DEGRADED".to_owned()),
    );
    object.insert(
        format!("{component}_error_code"),
        Value::String(error.code().to_owned()),
    );
    Ok(base)
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

fn runtime_receipt_digest(value: &Value) -> Result<ContentDigest, LatticedError> {
    let object = value
        .as_object()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    match object.get("full_chain_receipt_digest") {
        Some(_) => full_chain_receipt_digest(value),
        None => delivery_receipt_digest(value),
    }
}

fn delivery_receipt_digest(value: &Value) -> Result<ContentDigest, LatticedError> {
    let object = value
        .as_object()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
    json_digest(object, "receipt_digest")
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
        LatticedErrorKind::ReconciliationRequired
        | LatticedErrorKind::TaskReconciliationRequired
        | LatticedErrorKind::HermesTeardownRejected => PortErrorKind::Ambiguous,
        LatticedErrorKind::DatabaseConnect
        | LatticedErrorKind::GraphReceiptRead
        | LatticedErrorKind::HermesReceiptRead
        | LatticedErrorKind::HermesProductionLivenessRejected => PortErrorKind::Unavailable,
        LatticedErrorKind::Configuration
        | LatticedErrorKind::Contract
        | LatticedErrorKind::ReceiptMismatch => PortErrorKind::Malformed,
        LatticedErrorKind::Intent
        | LatticedErrorKind::OutcomePersistence
        | LatticedErrorKind::DeliveryFailed
        | LatticedErrorKind::OfficialLiveBlocked
        | LatticedErrorKind::ScriptedFixtureRejected
        | LatticedErrorKind::GraphExecution
        | LatticedErrorKind::HermesPreparationMissing
        | LatticedErrorKind::HermesPreparationRequired
        | LatticedErrorKind::HermesProductionRunnerRequired
        | LatticedErrorKind::HermesExecution
        | LatticedErrorKind::DatabaseSecret
        | LatticedErrorKind::LedgerConfiguration
        | LatticedErrorKind::WorkspaceConfiguration
        | LatticedErrorKind::CodexConfiguration
        | LatticedErrorKind::ReceiptRead
        | LatticedErrorKind::GraphConfiguration
        | LatticedErrorKind::TaskControl
        | LatticedErrorKind::WriterLease
        | LatticedErrorKind::Transport => PortErrorKind::Denied,
    }
}

fn map_gateway_service_error(error: LatticedError) -> GatewayServiceError {
    GatewayServiceError::new(gateway_error_kind(error.kind()), error.code())
}

fn map_task_lifecycle_gateway_error(error: &TaskLifecycleError) -> GatewayServiceError {
    let kind = match error.kind() {
        TaskLifecycleErrorKind::Rejected => PortErrorKind::Denied,
        TaskLifecycleErrorKind::Unavailable => PortErrorKind::Unavailable,
        TaskLifecycleErrorKind::Ambiguous => PortErrorKind::Ambiguous,
        TaskLifecycleErrorKind::Corrupt => PortErrorKind::Malformed,
    };
    GatewayServiceError::new(kind, error.code())
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

fn daemon_process_start_identity() -> Result<ContentDigest, LatticedError> {
    let executable =
        env::current_exe().map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
    let observed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
    digest(
        "lattice.latticed.process-start",
        &CanonicalValue::Object(vec![
            (
                "executable".to_owned(),
                CanonicalValue::String(path_text(&executable)?),
            ),
            (
                "observed_unix_nanos".to_owned(),
                CanonicalValue::String(observed.as_nanos().to_string()),
            ),
            (
                "process_id".to_owned(),
                CanonicalValue::String(process::id().to_string()),
            ),
        ]),
    )
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

fn invocation_for_task(
    run_id: &str,
    binding: &SubjectBinding,
) -> Result<Invocation, LatticedError> {
    Invocation::new(
        CONTRACT_VERSION,
        RequestId::new(format!("task038-request-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        binding.task_id().clone(),
        AttemptId::new(format!("task038-attempt-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        binding.project_snapshot_id().clone(),
        binding.task_spec_digest().clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
}

fn task_ledger_identity(
    binding: &SubjectBinding,
) -> Result<lattice_contracts::TaskLedgerStreamIdentity, LatticedError> {
    lattice_contracts::TaskLedgerStreamIdentity::new(
        binding.project_id().clone(),
        binding.project_snapshot_id().clone(),
        binding.task_id().clone(),
        binding.task_revision(),
        binding.task_spec_digest().clone(),
        "TWD",
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))
}

fn request_for_delivery(
    run_id: &str,
    config: &LatticedDeliveryConfig,
) -> Result<DeliveryRunRequest, LatticedError> {
    request_for_delivery_invocation(config, invocation_for_run(run_id)?)
}

fn request_for_task_delivery(
    run_id: &str,
    config: &LatticedDeliveryConfig,
    binding: &SubjectBinding,
) -> Result<DeliveryRunRequest, LatticedError> {
    request_for_delivery_invocation(config, invocation_for_task(run_id, binding)?)
}

fn request_for_delivery_invocation(
    config: &LatticedDeliveryConfig,
    invocation: Invocation,
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
    run_graph_memory_request(database, password, config, fixture, deadline, &request)
}

fn run_graph_memory_request(
    database: &DeliveryDatabaseBinding,
    password: &str,
    config: &LatticedDeliveryConfig,
    fixture: &DeliveryGraphPaths,
    deadline: Instant,
    request: &GraphMemoryRunRequest,
) -> Result<GraphMemoryReceipt, LatticedError> {
    let query = MemoryQuery::new(request, GRAPH_QUERY, GRAPH_RETRIEVAL_LIMIT)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::GraphExecution))?;
    let graph_root = fixture.root.join(GRAPH_MEMORY_ROOT_NAME);
    let bridge = SnapshotBridge::new();
    let git_executable_sha256 = graph_executable_sha256(&config.git_executable)?;
    let snapshot_config = GitSnapshotConfig::new(
        config.git_executable.clone(),
        git_executable_sha256,
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
        graphify_wsl_executable_from_environment(
            PathBuf::from(system_root).join("System32/wsl.exe"),
        ),
        graphify_runtime_root_from_environment(&fixture.repository_root),
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

    run_graph_memory(request, &query, &mut snapshot, &mut graphify, &mut memory)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphExecution))
}

fn graphify_runtime_root_from_environment(repository_root: &Path) -> PathBuf {
    graphify_runtime_root_from_value(
        std::env::var_os("LATTICE_GRAPHIFY_RUNTIME_ROOT")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        repository_root,
    )
}

fn graphify_runtime_root_from_value(
    configured: Option<PathBuf>,
    repository_root: &Path,
) -> PathBuf {
    configured.unwrap_or_else(|| repository_root.join(GRAPHIFY_RUNTIME_RELATIVE_PATH))
}

fn graphify_wsl_executable_from_environment(default: PathBuf) -> PathBuf {
    graphify_wsl_executable_from_value(
        std::env::var_os("LATTICE_GRAPHIFY_WSL_EXE")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        default,
    )
}

fn graphify_wsl_executable_from_value(configured: Option<PathBuf>, default: PathBuf) -> PathBuf {
    configured.unwrap_or(default)
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
    append_graph_receipt_to_json(receipt_json(delivery_receipt, component)?, graph_receipt)
}

fn append_graph_receipt_to_json(
    value: Value,
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
    append_graph_receipt_fields(value, &fields)
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
    use lattice_codebase_memory::{normalize_analysis, plan_retrieval};
    use lattice_contracts::{
        CodeSnapshotEvidence, CodebaseMemoryPersistenceIdentity, DaemonEpoch, GraphConfidence,
        GraphMemoryPersistenceEvidence, GraphSourceProvenance, GraphifyIdentity,
        GraphifyRawEvidence, GraphifyRawNode, MemoryRetrievalDisposition, MemoryRetrievalEvidence,
        MemoryRetrievalPlan, NormalizedGraphAnalysis, TrackedSource,
    };
    use lattice_ports::{
        AutonomyDisposition, AutonomyReceiptProjection, CodebaseMemoryPort,
        DeliveryFailureCertainty, DeliveryPortError,
    };
    use lattice_postgres_codebase_memory::{
        ExtensionApplyOutcome as MemoryExtensionApplyOutcome,
        ExtensionDatabaseRole as MemoryExtensionDatabaseRole,
        ExtensionTarget as MemoryExtensionTarget, apply_extension as apply_memory_extension,
        verify_extension as verify_memory_extension,
    };
    use lattice_postgres_writer_lease::{
        ExtensionApplyOutcome as WriterExtensionApplyOutcome,
        apply_extension as apply_writer_extension, verify_extension as verify_writer_extension,
    };
    use lattice_writer_lease::{
        CommandOutcome as WriterCommandOutcome, WriterLeaseAcquireRequest, WriterLeaseRepository,
        WriterLeaseRepositoryCommand,
    };
    use postgres::config::SslMode;
    use postgres::{Client, Config, NoTls};

    const TASK050_PROFILE_MARKER_PREFIX: &str = "TASK050_LATTICED_PROFILE_INPUT=";

    fn task050_test_digest(value: char) -> ContentDigest {
        ContentDigest::from_sha256(value.to_string().repeat(64)).expect("TASK050 digest")
    }

    fn task050_store_authority() -> StoreAuthorityHead {
        StoreAuthorityHead::new(
            RuntimeKind::Live,
            StoreDaemonInstanceId::new("task050-fresh-process").expect("TASK050 daemon"),
            DaemonEpoch::new(50).expect("TASK050 epoch"),
            RuntimeAdmissionMode::Active,
            StoreAuthorityRevision::new(50).expect("TASK050 revision"),
            task050_test_digest('a'),
            task050_test_digest('b'),
        )
        .expect("TASK050 Store authority")
    }

    #[test]
    fn postgres_bootstrap_restores_the_configured_active_authority() {
        let authority = task050_store_authority();
        let snapshot = RuntimeAdmissionSnapshot::from_authority(&authority)
            .expect("configured active authority is representable");
        assert_eq!(snapshot.mode, "ACTIVE");
        assert_eq!(
            snapshot.daemon_instance_id.as_deref(),
            Some("task050-fresh-process")
        );
        assert_eq!(snapshot.daemon_epoch, Some(50));
        assert_eq!(snapshot.authority_revision, 50);
        assert_eq!(
            snapshot.observation_digest.as_deref(),
            Some(&[0xaa; 32][..])
        );
        assert_eq!(
            snapshot.authority_head_digest.as_deref(),
            Some(&[0xbb; 32][..])
        );
    }

    fn task050_connect_as(database: &str, role: &str) -> Result<Client, Box<dyn Error>> {
        let host = required_environment("LATTICE_TASK019_HOST")?;
        let port = required_environment("LATTICE_TASK019_PORT")?.parse::<u16>()?;
        let password = required_environment("LATTICE_TASK019_PASSWORD")?;
        let mut config = Config::new();
        config
            .host(&host)
            .port(port)
            .user(&format!("{role}_login"))
            .password(password)
            .dbname(database)
            .application_name("lattice-devos-task050-canonical-profile")
            .ssl_mode(SslMode::Disable);
        let mut client = config.connect(NoTls)?;
        client.batch_execute(&format!("SET ROLE {role}"))?;
        Ok(client)
    }

    fn task050_ingress_peer(
        profile: Task050AcceptanceProfile,
    ) -> Result<TaskIngressPeerEvidence, Box<dyn Error>> {
        let executable = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("debug")
            .join("latticed.exe");
        let binary_digest = ContentDigest::from_sha256(official_file_sha256(
            &executable,
            MAX_LATTICED_EXECUTABLE_BYTES,
        )?)?;
        let profile_digest = match profile {
            Task050AcceptanceProfile::AskUser => task050_test_digest('c'),
            Task050AcceptanceProfile::Proceed => task050_test_digest('d'),
        };
        Ok(
            TaskIngressPeerEvidence::new_local_canonical_mcp_acceptance_live(
                GatewayInstanceId::new("latticed-local-canonical-acceptance")?,
                env!("CARGO_PKG_VERSION"),
                binary_digest,
                mcp::task_ingress_schema_digest().ok_or("TASK050 MCP schema digest missing")?,
                GatewayChannelId::new("main")?,
                profile_digest,
                task050_test_digest('e'),
            )?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn task050_autonomy_marker(
        phase: &str,
        profile: Task050AcceptanceProfile,
        run_id: &str,
        ingress_peer: &TaskIngressPeerEvidence,
        authority: &StoreAuthorityHead,
        binding: &SubjectBinding,
        evidence: &TaskLifecycleEvidence,
        task_ref: &ContentDigest,
    ) -> Result<Value, Box<dyn Error>> {
        let profile_name = match profile {
            Task050AcceptanceProfile::AskUser => "ASK_USER",
            Task050AcceptanceProfile::Proceed => "PROCEED",
        };
        let autonomy = evidence
            .autonomy_receipt()
            .ok_or("TASK050 autonomy projection missing")?;
        Ok(json!({
            "authority_head_digest": authority.head_digest().as_str(),
            "authority_revision": authority.revision().get(),
            "autonomy_projection_sha256": autonomy.receipt_digest().as_str(),
            "daemon_epoch": authority.daemon_epoch().get(),
            "daemon_instance_id": authority.daemon_instance_id().as_str(),
            "database_run_id": run_id,
            "expected_status": task_public_status(evidence, task_ref),
            "ingress_profile_sha256": ingress_peer.profile_digest().as_str(),
            "observation_digest": authority.observation_digest().as_str(),
            "phase": phase,
            "profile": profile_name,
            "schema": "lattice.task050.latticed-profile-input.v2",
            "task_ref": task_ref.as_str(),
            "task_spec_digest": binding.task_spec_digest().as_str(),
        }))
    }

    #[allow(clippy::too_many_lines)]
    fn run_task050_canonical_latticed_profiles() -> Result<(), Box<dyn Error>> {
        if required_environment("LATTICE_TASK019_LIVE")? != "1"
            || required_environment("LATTICE_TASK019_HOST")? != "127.0.0.1"
        {
            return Err("TASK050 live boundary rejected".into());
        }
        let phase = required_environment("LATTICE_TASK019_PHASE")?;
        if !matches!(phase.as_str(), "initial" | "restart") {
            return Err("TASK050 phase rejected".into());
        }
        let run_id = required_environment("LATTICE_TASK019_RUN_ID")?;
        let port = required_environment("LATTICE_TASK019_PORT")?.parse::<u16>()?;
        let password = required_environment("LATTICE_TASK019_PASSWORD")?;
        let database = DeliveryDatabaseBinding::new("127.0.0.1", port, run_id.clone())?;
        let authority = task050_store_authority();
        let ask_submission =
            task050_acceptance_gateway_submission(Task050AcceptanceProfile::AskUser)?;
        let ask_peer = task050_ingress_peer(Task050AcceptanceProfile::AskUser)?;
        let mut foundation_probe = PostgresTaskLifecycle::connect_with_ingress_peer(
            &database,
            &password,
            Instant::now() + Duration::from_mins(2),
            task_ledger_identity(ask_submission.binding())?,
            authority.clone(),
            ask_peer,
        )?;
        let foundation = foundation_probe.persistence_foundation(ask_submission.binding())?;
        drop(foundation_probe);

        let memory_target = MemoryExtensionTarget::new(database.database_name(), &run_id)?;
        let memory_manifest = verify_embedded_extension_manifest()?;
        let writer_target = WriterLeaseExtensionTarget::new(
            database.database_name(),
            foundation.database_identity_digest().clone(),
            foundation.global_manifest_digest().clone(),
            memory_manifest.manifest_sha256().clone(),
        )?;
        let mut migrator = task050_connect_as(&database.database_name(), "lattice_migrator")?;
        if phase == "initial" {
            assert!(matches!(
                apply_memory_extension(&mut migrator, &memory_target)?,
                MemoryExtensionApplyOutcome::Installed
                    | MemoryExtensionApplyOutcome::AlreadyCurrent
            ));
            assert!(matches!(
                apply_writer_extension(&mut migrator, &writer_target)?,
                WriterExtensionApplyOutcome::Installed
                    | WriterExtensionApplyOutcome::Activated
                    | WriterExtensionApplyOutcome::AlreadyCurrent
            ));
        } else {
            verify_memory_extension(
                &mut migrator,
                &memory_target,
                MemoryExtensionDatabaseRole::Migrator,
            )?;
            verify_writer_extension(&mut migrator, &writer_target)?;
        }
        drop(migrator);

        for profile in [
            Task050AcceptanceProfile::AskUser,
            Task050AcceptanceProfile::Proceed,
        ] {
            let submission = task050_acceptance_gateway_submission(profile)?;
            let binding = submission.binding().clone();
            let ingress_peer = task050_ingress_peer(profile)?;
            let client_request_id = match profile {
                Task050AcceptanceProfile::AskUser => "task050-canonical-ask-user",
                Task050AcceptanceProfile::Proceed => "task050-canonical-proceed",
            };
            let mut lifecycle = PostgresTaskLifecycle::connect_with_ingress_peer(
                &database,
                &password,
                Instant::now() + Duration::from_mins(2),
                task_ledger_identity(&binding)?,
                authority.clone(),
                ingress_peer.clone(),
            )?;
            lifecycle.admit(&binding, client_request_id)?;
            let mut writer = None;
            let writer_authority = if profile == Task050AcceptanceProfile::Proceed {
                let runtime = connect_fixed_runtime_client(
                    &database,
                    &password,
                    Instant::now() + Duration::from_mins(2),
                )?;
                let mut repository =
                    PostgresWriterLease::new(runtime, writer_target.clone(), &authority, 600)?;
                let current = if phase == "initial" {
                    let receipt = repository.execute(WriterLeaseRepositoryCommand::Acquire(
                        WriterLeaseAcquireRequest {
                            command_id: "task050-canonical-proceed-acquire".to_owned(),
                            expected_head: None,
                            project_id: binding.project_id().clone(),
                            project_snapshot_id: binding.project_snapshot_id().clone(),
                            task_id: binding.task_id().clone(),
                            task_revision: binding.task_revision().to_owned(),
                            task_spec_digest: binding.task_spec_digest().clone(),
                            attempt_id: AttemptId::new("task050-canonical-proceed-attempt")?,
                            lease_id: "task050-canonical-proceed-lease".to_owned(),
                            lease_holder_id: "codex-writer".to_owned(),
                            worktree_id: "task050-canonical-acceptance".to_owned(),
                            holder_process_id: HolderProcessId::new(u64::from(process::id()))?,
                            holder_process_start_identity: task050_test_digest('f'),
                        },
                    ))?;
                    if receipt.outcome != WriterCommandOutcome::Applied {
                        return Err("TASK050 writer acquire denied".into());
                    }
                    receipt.after.ok_or("TASK050 writer authority missing")?
                } else {
                    repository
                        .current_authority(binding.project_id())?
                        .ok_or("TASK050 current writer authority missing")?
                        .independent_head()
                        .clone()
                };
                repository.assert_current(&current)?;
                writer = Some(repository);
                Some(current)
            } else {
                None
            };
            let evidence =
                lifecycle.record_autonomy_receipt(&binding, writer_authority.as_ref())?;
            if evidence.state() != TaskState::Draft
                || evidence.result_digest().is_some()
                || evidence
                    .autonomy_receipt()
                    .map(AutonomyReceiptProjection::disposition)
                    != Some(match profile {
                        Task050AcceptanceProfile::AskUser => AutonomyDisposition::AskUser,
                        Task050AcceptanceProfile::Proceed => AutonomyDisposition::Proceed,
                    })
            {
                return Err("TASK050 autonomy projection rejected".into());
            }
            if let (Some(repository), Some(current)) = (writer.as_mut(), writer_authority.as_ref())
            {
                repository.assert_current(current)?;
            }
            let task_ref = controlled_task_reference(
                &binding,
                &task_admission_command_id(client_request_id),
                &run_id,
                ingress_peer.profile_digest(),
            )?;
            let marker = task050_autonomy_marker(
                &phase,
                profile,
                &run_id,
                &ingress_peer,
                &authority,
                &binding,
                &evidence,
                &task_ref,
            )?;
            println!("\n{TASK050_PROFILE_MARKER_PREFIX}{marker}");
        }
        Ok(())
    }

    #[test]
    fn task050_acceptance_profile_selector_is_private_exact_and_fail_closed() {
        let ask = task050_acceptance_gateway_submission(Task050AcceptanceProfile::AskUser)
            .expect("ASK_USER submission");
        let proceed = task050_acceptance_gateway_submission(Task050AcceptanceProfile::Proceed)
            .expect("PROCEED submission");

        assert_eq!(
            ask.binding().task_spec_digest().as_str(),
            "0915bc62fe4613bebda5a82e65863a325b7102124a61aa0efc9310a33a18be59"
        );
        assert_eq!(
            proceed.binding().task_spec_digest().as_str(),
            "0cdfb9ee77f8f3b819ddbd74bf2d58537da11ec065bb1526889bb08adf77e86d"
        );
        assert_ne!(ask.binding(), proceed.binding());

        let exact = Task050AcceptanceSelectorInput {
            profile: Some("ASK_USER"),
            task_spec_sha256: Some(ask.binding().task_spec_digest().as_str()),
            task050_live: Some("1"),
            task019_live: Some("1"),
            phase: Some("initial"),
            host: Some("127.0.0.1"),
            run_id: Some("05000000000000000000000000000001"),
            ingress_kind: Some(TASK_INGRESS_LOCAL_ACCEPTANCE),
        };
        assert_eq!(
            select_task050_acceptance_profile(FullChainRunMode::ResumeExisting, exact),
            Ok(Some(Task050AcceptanceProfile::AskUser))
        );
        assert_eq!(
            select_task050_acceptance_profile(
                FullChainRunMode::ResumeExisting,
                Task050AcceptanceSelectorInput {
                    profile: Some("PROCEED"),
                    task_spec_sha256: Some(proceed.binding().task_spec_digest().as_str()),
                    phase: Some("restart"),
                    ..exact
                }
            ),
            Ok(Some(Task050AcceptanceProfile::Proceed))
        );
        assert_eq!(
            select_task050_acceptance_profile(FullChainRunMode::Fresh, exact),
            Err(LatticedErrorKind::Configuration)
        );
        for rejected in [
            Task050AcceptanceSelectorInput {
                task050_live: None,
                ..exact
            },
            Task050AcceptanceSelectorInput {
                task019_live: None,
                ..exact
            },
            Task050AcceptanceSelectorInput {
                phase: Some("memory_setup"),
                ..exact
            },
            Task050AcceptanceSelectorInput {
                host: Some("localhost"),
                ..exact
            },
            Task050AcceptanceSelectorInput {
                run_id: None,
                ..exact
            },
            Task050AcceptanceSelectorInput {
                run_id: Some("0500000000000000000000000000000A"),
                ..exact
            },
            Task050AcceptanceSelectorInput {
                ingress_kind: Some(TASK_INGRESS_SECURE_TUNNEL),
                ..exact
            },
        ] {
            assert_eq!(
                select_task050_acceptance_profile(FullChainRunMode::ResumeExisting, rejected),
                Err(LatticedErrorKind::Configuration)
            );
        }
        assert_eq!(
            select_task050_acceptance_profile(
                FullChainRunMode::ResumeExisting,
                Task050AcceptanceSelectorInput {
                    task_spec_sha256: Some(proceed.binding().task_spec_digest().as_str()),
                    ..exact
                }
            ),
            Err(LatticedErrorKind::Configuration)
        );
        assert_eq!(
            select_task050_acceptance_profile(
                FullChainRunMode::ResumeExisting,
                Task050AcceptanceSelectorInput {
                    profile: None,
                    task_spec_sha256: None,
                    task050_live: None,
                    task019_live: None,
                    phase: None,
                    host: None,
                    run_id: None,
                    ingress_kind: None,
                }
            ),
            Ok(None)
        );
    }

    #[test]
    #[ignore = "requires the coordinated marker-owned TASK-019 PostgreSQL fixture"]
    fn task050_canonical_latticed_profiles_when_provisioned() {
        if env::var("LATTICE_TASK050_LIVE").ok().as_deref() != Some("1") {
            return;
        }
        run_task050_canonical_latticed_profiles().expect("TASK050 canonical profiles");
    }

    #[test]
    fn synthetic_official_bundle_facts_exercise_only_the_pure_fixed_policy() {
        let policy = OfficialBundlePolicy::production();
        let complete = SyntheticOfficialBundleEvidenceProvider::complete(policy);

        assert!(policy.evaluate(&complete).is_ok());
        assert_eq!(
            complete.facts().provenance,
            OfficialBundleEvidenceProvenance::SyntheticTest
        );

        let mut missing_sandbox = complete.clone();
        missing_sandbox.remove(OfficialBundleFileRole::SandboxSetup);
        assert_eq!(
            policy.evaluate(&missing_sandbox),
            Err(OfficialIdentityRejection::MissingFile(
                OfficialBundleFileRole::SandboxSetup
            ))
        );

        let mut missing_runner = complete.clone();
        missing_runner.remove(OfficialBundleFileRole::CommandRunner);
        assert_eq!(
            policy.evaluate(&missing_runner),
            Err(OfficialIdentityRejection::MissingFile(
                OfficialBundleFileRole::CommandRunner
            ))
        );
    }

    #[test]
    fn synthetic_official_bundle_facts_reject_manifest_drift_and_target_split() {
        let policy = OfficialBundlePolicy::production();

        let mut manifest_drift = SyntheticOfficialBundleEvidenceProvider::complete(policy);
        manifest_drift
            .file_mut(OfficialBundleFileRole::PackageManifest)
            .sha256 = Some("0".repeat(64));
        assert_eq!(
            policy.evaluate(&manifest_drift),
            Err(OfficialIdentityRejection::DigestMismatch(
                OfficialBundleFileRole::PackageManifest
            ))
        );

        let mut target_split = SyntheticOfficialBundleEvidenceProvider::complete(policy);
        target_split.facts.launcher_target_root = Some(PathBuf::from(r"D:\foreign\target"));
        assert_eq!(
            policy.evaluate(&target_split),
            Err(OfficialIdentityRejection::TargetSplit)
        );
    }

    #[test]
    fn synthetic_official_bundle_facts_reject_file_id_replacement_capture_mix() {
        let policy = OfficialBundlePolicy::production();
        let mut replacement_mix = SyntheticOfficialBundleEvidenceProvider::complete(policy);
        replacement_mix
            .file_mut(OfficialBundleFileRole::CommandRunner)
            .observed_identity = Some(OfficialFileIdentity {
            volume_serial_number: 7,
            file_index: 999,
        });

        assert_eq!(
            policy.evaluate(&replacement_mix),
            Err(OfficialIdentityRejection::FileIdentityChanged(
                OfficialBundleFileRole::CommandRunner
            ))
        );
    }

    #[test]
    fn official_bundle_target_root_is_launcher_owned_for_short_external_delivery_base() {
        let policy = OfficialBundlePolicy::production();
        let official_target_root = PathBuf::from(r"C:\lattice-official\target");
        let launcher =
            policy.expected_path(&official_target_root, OfficialBundleFileRole::Launcher);
        let user_owned_delivery_base = PathBuf::from(r"C:\d");
        let derived_target_root = OfficialBundlePolicy::launcher_target_root(&launcher)
            .expect("official target root from validated launcher ancestry");

        assert_eq!(derived_target_root, official_target_root);
        assert_ne!(derived_target_root, user_owned_delivery_base);

        let valid =
            SyntheticOfficialBundleEvidenceProvider::complete_at(policy, &derived_target_root);
        assert!(policy.evaluate(&valid).is_ok());

        let mut identity_drift = valid;
        identity_drift
            .file_mut(OfficialBundleFileRole::CommandRunner)
            .observed_identity = Some(OfficialFileIdentity {
            volume_serial_number: 7,
            file_index: 999,
        });
        assert_eq!(
            policy.evaluate(&identity_drift),
            Err(OfficialIdentityRejection::FileIdentityChanged(
                OfficialBundleFileRole::CommandRunner
            ))
        );
    }

    #[cfg(windows)]
    #[test]
    fn real_pinned_file_handle_blocks_replacement_for_guard_lifetime() {
        static NEXT_PINNED_FIXTURE: AtomicUsize = AtomicUsize::new(0);
        let unique = NEXT_PINNED_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = env::temp_dir().join(format!(
            "lattice-official-pinned-file-{}-{unique}",
            process::id()
        ));
        let path = root.join("codex-command-runner.exe");
        fs::create_dir_all(&root).expect("create pinned-file fixture root");
        fs::write(&path, b"captured official file facts").expect("write pinned-file fixture");

        let (facts, pinned) = capture_official_file(
            OfficialBundleFileRole::CommandRunner,
            path.clone(),
            path.clone(),
            &root,
            MAX_OFFICIAL_RESOURCE_BYTES,
        )
        .expect("capture pinned-file facts and handle");
        let replacement_while_pinned = fs::write(&path, b"replacement");
        let content_while_pinned = fs::read(&path).expect("read pinned-file fixture");
        drop(pinned);
        let replacement_after_drop = fs::write(&path, b"replacement");
        fs::remove_dir_all(&root).expect("remove pinned-file fixture root");

        assert_eq!(facts.captured_identity, facts.observed_identity);
        assert!(replacement_while_pinned.is_err());
        assert_eq!(content_while_pinned, b"captured official file facts");
        assert!(replacement_after_drop.is_ok());
    }

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
    fn production_composition_stderr_uses_only_fixed_public_diagnostics() {
        let source = include_str!("composition.rs");
        let stderr_macro = ["eprint", "ln!"].concat();
        let stderr_inline_macro = ["eprint", "!"].concat();
        let fixed_emission = [
            "eprint",
            "ln!(\"{}\", LatticedErrorKind::Transport.code());",
        ]
        .concat();
        let diagnostic_emission = [
            "write_startup_diagnostic(&mut io::",
            "stderr().lock(), diagnostic);",
        ]
        .concat();
        let direct_stderr = ["io::", "stderr"].concat();
        let inherited_stdio = ["Stdio", "::inherit"].concat();

        assert_eq!(source.matches(&stderr_macro).count(), 1);
        assert!(source.contains(&fixed_emission));
        assert!(source.contains(&diagnostic_emission));
        assert_eq!(
            LatticedErrorKind::Transport.code(),
            "LATTICED_STDIO_REJECTED"
        );
        assert!(!source.contains(&stderr_inline_macro));
        assert_eq!(source.matches(&direct_stderr).count(), 2);
        assert!(!source.contains(&inherited_stdio));
    }

    #[test]
    fn startup_diagnostic_write_failure_is_non_authoritative() {
        struct ClosedDiagnosticSink;

        impl Write for ClosedDiagnosticSink {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "closed diagnostic sink",
                ))
            }

            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "closed diagnostic sink",
                ))
            }
        }

        write_startup_diagnostic(
            &mut ClosedDiagnosticSink,
            StartupDiagnostic::configuration_validation_started(),
        );
    }

    #[test]
    fn startup_diagnostic_is_fixed_vocabulary_and_omits_secret_like_input() {
        let progress = StartupDiagnostic::configuration_validation_started().render();
        let progress_value: Value =
            serde_json::from_str(&progress).expect("progress diagnostic JSON");
        let rendered = StartupDiagnostic::failure(
            "CONFIGURATION_VALIDATED",
            "VALID",
            "ASSEMBLY_REJECTED",
            LatticedErrorKind::Configuration,
        )
        .render();
        let value: Value = serde_json::from_str(&rendered).expect("diagnostic JSON");

        assert_eq!(progress_value["waiting_reason"], "CONFIGURATION_VALIDATION");
        assert_eq!(value["schema"], STARTUP_DIAGNOSTIC_SCHEMA);
        assert_eq!(value["stage"], "STARTUP_FAILED");
        assert_eq!(value["last_completed_stage"], "CONFIGURATION_VALIDATED");
        assert_eq!(value["configuration_health"], "VALID");
        assert_eq!(value["dependency_health"], "ASSEMBLY_REJECTED");
        assert_eq!(
            value["failure_classification"],
            "LATTICED_CONFIGURATION_REJECTED"
        );
        assert!(!rendered.contains("LATTICE_TASK019_PASSWORD"));
        assert!(!rendered.contains("ignored-secret"));
        assert!(!rendered.contains("127.0.0.1"));
    }

    #[test]
    fn effect_deadline_reserves_time_for_cleanup_and_terminal_ledger_finalization() {
        let finalization = deadline(Duration::from_mins(2)).expect("finalization deadline");
        let effect = effect_deadline(finalization).expect("effect deadline");

        assert_eq!(finalization.duration_since(effect), FINALIZATION_RESERVE);
        assert!(effect > Instant::now());
    }

    #[test]
    fn controlled_task_timeout_is_bounded_inside_the_lease_ttl() {
        assert!(
            validate_controlled_task_timeout(CONTROLLED_TASK_MAX_RUNTIME, FullChainRunMode::Fresh)
                .is_ok()
        );
        assert!(
            validate_controlled_task_timeout(
                CONTROLLED_TASK_MAX_RUNTIME + Duration::from_secs(1),
                FullChainRunMode::Fresh
            )
            .is_err()
        );
        assert!(
            validate_controlled_task_timeout(FINALIZATION_RESERVE, FullChainRunMode::Fresh)
                .is_err()
        );
        assert!(
            validate_controlled_task_timeout(
                Duration::from_secs(MAX_TIMEOUT_SECONDS),
                FullChainRunMode::ResumeExisting
            )
            .is_ok()
        );
    }

    #[test]
    fn completed_task_requires_one_released_and_replay_verified_writer_history() {
        assert!(expected_completed_writer_history(false, 1, 2, 2));
        assert!(!expected_completed_writer_history(true, 1, 2, 2));
        assert!(!expected_completed_writer_history(false, 0, 0, 0));
        assert!(!expected_completed_writer_history(false, 2, 4, 4));
        assert_eq!(
            LatticedErrorKind::TaskReconciliationRequired.code(),
            "LATTICE_TASK_RECONCILIATION_REQUIRED"
        );
    }

    #[test]
    fn resume_existing_task_submit_replays_only_completed_evidence() {
        let binding = fixed_gateway_submission()
            .expect("fixed controlled canary")
            .binding()
            .clone();
        let completed = TaskLifecycleEvidence::new(
            binding.clone(),
            TaskLifecycleAutonomyEvidence::HistoricalOptional(None),
            TaskState::Completed,
            test_content_digest('7'),
            Some(test_content_digest('8')),
        );

        assert_eq!(
            controlled_writer_decision(
                ExistingCompletionPolicy::Require,
                &binding,
                Some(&completed),
            ),
            Ok(ControlledWriterDecision::ReplayExisting)
        );
    }

    #[test]
    fn resume_existing_task_submit_never_selects_execution_for_invalid_evidence() {
        let binding = fixed_gateway_submission()
            .expect("fixed controlled canary")
            .binding()
            .clone();
        let mismatched_binding = SubjectBinding::new(
            binding.project_id().clone(),
            binding.project_snapshot_id().clone(),
            binding.task_id().clone(),
            FIXED_GATEWAY_TASK_REVISION,
            test_content_digest('9'),
        )
        .expect("mismatched test binding");
        let cases = [
            TaskLifecycleEvidence::new(
                binding.clone(),
                TaskLifecycleAutonomyEvidence::Unadmitted,
                TaskState::Draft,
                test_content_digest('7'),
                None,
            ),
            TaskLifecycleEvidence::new(
                binding.clone(),
                TaskLifecycleAutonomyEvidence::HistoricalOptional(None),
                TaskState::Executing,
                test_content_digest('7'),
                None,
            ),
            TaskLifecycleEvidence::new(
                binding.clone(),
                TaskLifecycleAutonomyEvidence::HistoricalOptional(None),
                TaskState::Failed,
                test_content_digest('7'),
                Some(test_content_digest('8')),
            ),
            TaskLifecycleEvidence::new(
                binding.clone(),
                TaskLifecycleAutonomyEvidence::HistoricalOptional(None),
                TaskState::Stopping,
                test_content_digest('7'),
                Some(test_content_digest('8')),
            ),
            TaskLifecycleEvidence::new(
                mismatched_binding,
                TaskLifecycleAutonomyEvidence::HistoricalOptional(None),
                TaskState::Completed,
                test_content_digest('7'),
                Some(test_content_digest('8')),
            ),
        ];

        for existing in cases {
            let error = controlled_writer_decision(
                ExistingCompletionPolicy::Require,
                &binding,
                Some(&existing),
            )
            .expect_err("resume must not select a new controlled execution");
            assert_eq!(error.code(), "LATTICE_TASK_RECONCILIATION_REQUIRED");
        }
        let missing = controlled_writer_decision(ExistingCompletionPolicy::Require, &binding, None)
            .expect_err("missing evidence must not select execution");
        assert_eq!(missing.code(), "LATTICE_TASK_RECONCILIATION_REQUIRED");
    }

    #[test]
    fn fresh_task_submit_still_selects_controlled_execution() {
        let binding = fixed_gateway_submission()
            .expect("fixed controlled canary")
            .binding()
            .clone();

        assert_eq!(
            controlled_writer_decision(ExistingCompletionPolicy::Ignore, &binding, None),
            Ok(ControlledWriterDecision::Execute)
        );
    }

    #[test]
    fn graphify_runtime_root_can_be_configured_outside_a_delivery_fixture() {
        let repository_root = Path::new(r"C:\legacy-delivery-fixture");
        let configured = PathBuf::from(r"C:\ProgramData\LATTICE\graphify-runtime");
        let default_wsl = PathBuf::from(r"C:\Windows\System32\wsl.exe");
        let configured_wsl = PathBuf::from(r"C:\LATTICE\pinned\wsl.exe");

        assert_eq!(
            graphify_runtime_root_from_value(Some(configured.clone()), repository_root),
            configured
        );
        assert_eq!(
            graphify_runtime_root_from_value(None, repository_root),
            repository_root.join(GRAPHIFY_RUNTIME_RELATIVE_PATH)
        );
        assert_eq!(
            graphify_wsl_executable_from_value(Some(configured_wsl.clone()), default_wsl.clone()),
            configured_wsl
        );
        assert_eq!(
            graphify_wsl_executable_from_value(None, default_wsl.clone()),
            default_wsl
        );
    }

    #[test]
    fn controlled_submit_delivery_root_selection_is_task_scoped_and_resume_safe() {
        static NEXT_DELIVERY_BASE: AtomicUsize = AtomicUsize::new(0);
        let unique = NEXT_DELIVERY_BASE.fetch_add(1, Ordering::Relaxed);
        let base = env::temp_dir().join(format!(
            "lattice-controlled-delivery-base-{}-{unique}",
            process::id()
        ));
        fs::create_dir_all(&base).expect("create configured delivery base");
        let first_identity = test_content_digest('1');
        let second_identity = test_content_digest('2');

        let first =
            controlled_submit_delivery_root(&base, &first_identity, FullChainRunMode::Fresh)
                .expect("first fresh task root");
        let second =
            controlled_submit_delivery_root(&base, &second_identity, FullChainRunMode::Fresh)
                .expect("second fresh task root");
        let existing = base.join("existing-task-root");
        fs::create_dir(&existing).expect("create existing resume root");
        let resumed = controlled_submit_delivery_root(
            &existing,
            &first_identity,
            FullChainRunMode::ResumeExisting,
        )
        .expect("resume existing task root");

        assert_eq!(first.parent(), Some(base.as_path()));
        assert_eq!(second.parent(), Some(base.as_path()));
        assert!(!first.exists());
        assert!(!second.exists());
        assert_ne!(first, second);
        assert_eq!(resumed, existing);
        fs::remove_dir_all(&base).expect("remove controlled delivery fixture");
    }

    #[test]
    fn controlled_task_schema_output_is_absent_deterministic_and_task_scoped() {
        static NEXT_SCHEMA_BASE: AtomicUsize = AtomicUsize::new(0);
        let unique = NEXT_SCHEMA_BASE.fetch_add(1, Ordering::Relaxed);
        let base = env::temp_dir().join(format!(
            "lattice-controlled-schema-base-{}-{unique}",
            process::id()
        ));
        let configured_schema_bundle = base.join("configured-schema-bundle");
        let task_base = base.join("tasks");
        fs::create_dir_all(&configured_schema_bundle).expect("create configured schema bundle");
        fs::create_dir_all(&task_base).expect("create task base");
        for index in 0..275 {
            fs::write(
                configured_schema_bundle.join(format!("schema-{index:03}.json")),
                b"configured-read-only-schema",
            )
            .expect("write configured schema fixture");
        }

        let mut configured = LatticedDeliveryConfig::status_process(Duration::from_secs(30));
        configured.schema_directory = configured_schema_bundle.clone();
        configured.delivery_root = task_base.clone();
        let first_identity = test_content_digest('1');
        let second_identity = test_content_digest('2');
        let first_root =
            controlled_submit_delivery_root(&task_base, &first_identity, FullChainRunMode::Fresh)
                .expect("first task root");
        let retry_root =
            controlled_submit_delivery_root(&task_base, &first_identity, FullChainRunMode::Fresh)
                .expect("same task retry root");
        let second_root =
            controlled_submit_delivery_root(&task_base, &second_identity, FullChainRunMode::Fresh)
                .expect("second task root");
        let first =
            controlled_task_delivery_config(&configured, &first_root, FullChainRunMode::Fresh)
                .expect("first fresh task config");
        let retry =
            controlled_task_delivery_config(&configured, &retry_root, FullChainRunMode::Fresh)
                .expect("same task retry config");
        let second =
            controlled_task_delivery_config(&configured, &second_root, FullChainRunMode::Fresh)
                .expect("second fresh task config");
        let resumed = controlled_task_delivery_config(
            &configured,
            &base.join("existing-task-root"),
            FullChainRunMode::ResumeExisting,
        );
        let first_expected = first_root.join(CONTROLLED_TASK_SCHEMA_OUTPUT_CHILD);
        let second_expected = second_root.join(CONTROLLED_TASK_SCHEMA_OUTPUT_CHILD);
        let configured_file_count = fs::read_dir(&configured_schema_bundle)
            .expect("read configured schema bundle")
            .count();
        let configured_bundle_unchanged = configured.schema_directory == configured_schema_bundle;
        let first_output_was_absent = !first.schema_directory.exists();
        let retry_output_was_absent = !retry.schema_directory.exists();
        let second_output_was_absent = !second.schema_directory.exists();
        fs::remove_dir_all(&base).expect("remove controlled schema fixture");

        assert!(configured_bundle_unchanged);
        assert_eq!(configured_file_count, 275);
        assert_eq!(first.schema_directory, first_expected);
        assert_eq!(retry.schema_directory, first_expected);
        assert_eq!(second.schema_directory, second_expected);
        assert_ne!(first.schema_directory, second.schema_directory);
        assert!(first_output_was_absent);
        assert!(retry_output_was_absent);
        assert!(second_output_was_absent);
        assert!(resumed.is_none());
    }

    #[test]
    fn fresh_task_reference_binds_client_request_and_does_not_reuse_fixed_spec_digest() {
        let binding = fixed_gateway_submission()
            .expect("fixed controlled canary")
            .binding()
            .clone();
        let profile_digest = test_content_digest('a');
        let first = controlled_task_reference(
            &binding,
            "mcp-submit:fresh-request-1",
            "fresh-run-1",
            &profile_digest,
        )
        .expect("first fresh task reference");
        let retry = controlled_task_reference(
            &binding,
            "mcp-submit:fresh-request-1",
            "fresh-run-1",
            &profile_digest,
        )
        .expect("deterministic fresh task reference retry");
        let second = controlled_task_reference(
            &binding,
            "mcp-submit:fresh-request-2",
            "fresh-run-1",
            &profile_digest,
        )
        .expect("second fresh task reference");

        assert_eq!(retry, first);
        assert_ne!(&first, binding.task_spec_digest());
        assert_ne!(&second, binding.task_spec_digest());
        assert_ne!(second, first);
        let public_status = task_public_status(
            &TaskLifecycleEvidence::new(
                binding,
                TaskLifecycleAutonomyEvidence::HistoricalOptional(None),
                TaskState::Executing,
                test_content_digest('7'),
                None,
            ),
            &first,
        );
        assert_eq!(
            public_status.get("task_ref").and_then(Value::as_str),
            Some(first.as_str())
        );
    }

    #[test]
    fn mcp_submissions_are_distinct_per_request_and_idempotent_per_key() {
        let first =
            mcp_gateway_submission("runtime-core-graphify-1").expect("first MCP submission");
        let retry =
            mcp_gateway_submission("runtime-core-graphify-1").expect("idempotent MCP submission");
        let second =
            mcp_gateway_submission("runtime-core-graphify-2").expect("second MCP submission");

        assert_eq!(first.binding(), retry.binding());
        assert_ne!(first.binding(), second.binding());
        assert_ne!(
            first.binding(),
            fixed_gateway_submission()
                .expect("legacy submission")
                .binding()
        );
        assert!(first.binding().task_id().as_str().starts_with("TASK-MCP-"));
    }

    #[test]
    fn merging_recovery_requires_exact_acquired_or_released_writer_history() {
        assert!(expected_merging_writer_history(true, 1, 1, 1));
        assert!(expected_merging_writer_history(false, 1, 2, 2));
        assert!(!expected_merging_writer_history(false, 0, 0, 0));
        assert!(!expected_merging_writer_history(true, 1, 2, 2));
        assert!(!expected_merging_writer_history(false, 1, 1, 1));
    }

    #[test]
    fn official_runtime_cannot_enter_the_unfenced_scripted_acceptance_lane() {
        assert!(validate_scripted_execution_lane(DeliveryRuntime::ScriptedAcceptance).is_ok());
        let error = validate_scripted_execution_lane(DeliveryRuntime::OfficialCodexAppServer)
            .expect_err("official Codex must enter through the controlled Task coordinator");
        assert_eq!(error.kind(), LatticedErrorKind::OfficialLiveBlocked);
        assert!(requires_scripted_fixture_validation(
            DeliveryRuntime::ScriptedAcceptance
        ));
        assert!(!requires_scripted_fixture_validation(
            DeliveryRuntime::OfficialCodexAppServer
        ));
    }

    #[test]
    fn durable_delivery_failure_is_a_known_controlled_task_outcome() {
        assert_eq!(
            controlled_execution_error_kind(LatticedErrorKind::DeliveryFailed),
            ControlledTaskExecutionErrorKind::Known
        );
        assert_eq!(
            controlled_execution_error_kind(LatticedErrorKind::ReconciliationRequired),
            ControlledTaskExecutionErrorKind::Ambiguous
        );
        assert_eq!(
            controlled_execution_error_kind(LatticedErrorKind::DatabaseConnect),
            ControlledTaskExecutionErrorKind::Ambiguous
        );
    }

    #[test]
    fn controlled_canary_has_an_independent_task_subject() {
        let submission = fixed_gateway_submission().expect("fixed controlled canary");
        let binding = submission.binding();

        assert_eq!(binding.project_id().as_str(), CONTROLLED_PROJECT_ID);
        assert_eq!(binding.task_id().as_str(), CONTROLLED_TASK_ID);
        assert_eq!(
            binding.project_snapshot_id().as_str(),
            CONTROLLED_PROJECT_SNAPSHOT_ID
        );
        assert_ne!(binding.task_id().as_str(), TASK_ID);
        assert_ne!(binding.project_snapshot_id().as_str(), PROJECT_SNAPSHOT_ID);
    }

    #[test]
    fn openclaw_submit_is_always_fail_closed() {
        let expected = fixed_gateway_submission().expect("fixed controlled canary");
        assert_eq!(
            openclaw_submit_denial(&expected, &expected),
            GatewayDenialCode::DownstreamDenied
        );

        let substituted = TaskSpecSubmission::new(
            expected.binding().clone(),
            b"{}".to_vec(),
            expected.claimed_spec_digest().clone(),
        )
        .expect("bounded substituted document");
        assert_eq!(
            openclaw_submit_denial(&substituted, &expected),
            GatewayDenialCode::CommandSubstitution
        );
    }

    #[test]
    fn separately_observed_non_terminal_task_requires_reconciliation() {
        let binding = fixed_gateway_submission()
            .expect("fixed controlled canary")
            .binding()
            .clone();
        let evidence = TaskLifecycleEvidence::new(
            binding,
            TaskLifecycleAutonomyEvidence::HistoricalOptional(None),
            TaskState::Executing,
            test_content_digest('7'),
            None,
        );
        let status = task_public_status(&evidence, &test_content_digest('9'));

        assert_eq!(
            status.get("status").and_then(Value::as_str),
            Some("RECONCILIATION_REQUIRED")
        );
        assert_eq!(
            status.get("task_state").and_then(Value::as_str),
            Some("EXECUTING")
        );
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
    fn runtime_integration_selects_independently_degradable_components() {
        assert_eq!(
            parse_runtime_integration_mode(None).expect("default integration mode"),
            RuntimeIntegrationMode::CoreOnly
        );
        assert_eq!(
            parse_runtime_integration_mode(Some("CORE_ONLY")).expect("core-only mode"),
            RuntimeIntegrationMode::CoreOnly
        );
        assert_eq!(
            parse_runtime_integration_mode(Some("GRAPHIFY")).expect("graphify mode"),
            RuntimeIntegrationMode::Graphify
        );
        assert_eq!(
            parse_runtime_integration_mode(Some("GRAPHIFY_HERMES"))
                .expect("graphify and hermes mode"),
            RuntimeIntegrationMode::GraphifyHermes
        );
        assert_eq!(
            parse_runtime_integration_mode(Some("FULL_CHAIN")).expect("legacy alias"),
            RuntimeIntegrationMode::GraphifyHermes
        );
        assert!(parse_runtime_integration_mode(Some("full_chain")).is_err());
        assert!(parse_runtime_integration_mode(Some("")).is_err());
    }

    #[test]
    fn optional_component_failure_preserves_the_verified_core_value() {
        let value = append_optional_component_degraded_json(
            json!({"receipt_digest": "a".repeat(64), "status": "COMPLETED"}),
            "graphify",
            LatticedError::new(LatticedErrorKind::GraphExecution),
        )
        .expect("degraded projection");
        assert_eq!(value["status"], "COMPLETED");
        assert_eq!(value["receipt_digest"], "a".repeat(64));
        assert_eq!(value["graphify_status"], "DEGRADED");
        assert_eq!(
            value["graphify_error_code"],
            "LATTICE_GRAPH_MEMORY_RUN_REJECTED"
        );
    }

    #[test]
    fn runtime_receipt_digest_uses_durable_receipt_when_analysis_is_absent() {
        let core_only = json!({"receipt_digest": "a".repeat(64)});
        assert_eq!(
            runtime_receipt_digest(&core_only)
                .expect("core receipt digest")
                .as_str(),
            "a".repeat(64)
        );

        let full_chain = json!({
            "receipt_digest": "a".repeat(64),
            "full_chain_receipt_digest": "b".repeat(64),
        });
        assert_eq!(
            runtime_receipt_digest(&full_chain)
                .expect("full-chain receipt digest")
                .as_str(),
            "b".repeat(64)
        );
    }

    #[test]
    fn canonical_hermes_mode_is_explicit_and_fail_closed() {
        assert_eq!(
            canonical_hermes_mode_from_value(None).expect("default task-only mode"),
            CanonicalHermesMode::TaskOnly
        );
        assert_eq!(
            canonical_hermes_mode_from_value(Some("TASK_ONLY")).expect("explicit task-only mode"),
            CanonicalHermesMode::TaskOnly
        );
        assert_eq!(
            canonical_hermes_mode_from_value(Some("PRODUCTION")).expect("production mode"),
            CanonicalHermesMode::Production
        );
        assert!(canonical_hermes_mode_from_value(Some("production")).is_err());
        assert!(canonical_hermes_mode_from_value(Some("")).is_err());
    }

    #[test]
    fn canonical_hermes_activation_launches_once_and_does_not_retry_failure() {
        let mut active = None;
        let mut attempted = false;
        let mut launch_calls = 0;

        assert_eq!(
            *activate_canonical_hermes_once(&mut active, &mut attempted, || {
                launch_calls += 1;
                Ok(7_u8)
            })
            .expect("first activation"),
            7
        );
        assert_eq!(
            *activate_canonical_hermes_once(&mut active, &mut attempted, || {
                launch_calls += 1;
                Ok(9_u8)
            })
            .expect("reuse active owner"),
            7
        );
        assert_eq!(launch_calls, 1);

        let mut failed_active: Option<u8> = None;
        let mut failed_attempted = false;
        let first =
            activate_canonical_hermes_once(&mut failed_active, &mut failed_attempted, || {
                Err(LatticedError::new(LatticedErrorKind::Configuration))
            })
            .expect_err("first activation failure");
        assert_eq!(first.kind(), LatticedErrorKind::Configuration);
        let second =
            activate_canonical_hermes_once(&mut failed_active, &mut failed_attempted, || {
                panic!("failed activation must not be retried")
            })
            .expect_err("failed activation stays closed");
        assert_eq!(
            second.kind(),
            LatticedErrorKind::HermesProductionRunnerRequired
        );
    }

    struct RecordingHermesLifecycle {
        ready_calls: usize,
        terminate_calls: usize,
        teardown_fails: bool,
    }

    impl production_hermes_sealed::Sealed for RecordingHermesLifecycle {
        fn has_production_seal(&self) -> bool {
            true
        }

        fn is_production_configured(&self) -> bool {
            true
        }

        fn ensure_ready(&mut self, _run_id: &str) -> Result<(), LatticedError> {
            self.ready_calls += 1;
            Ok(())
        }

        fn terminate(&mut self) -> Result<(), LatticedError> {
            self.terminate_calls += 1;
            if self.teardown_fails {
                Err(LatticedError::new(
                    LatticedErrorKind::HermesTeardownRejected,
                ))
            } else {
                Ok(())
            }
        }
    }

    impl HermesPort for RecordingHermesLifecycle {
        fn research(&mut self, _request: HermesResearchRequest) -> PortResult<HermesEvidence> {
            Err(PortError::new(
                Component::Hermes,
                PortErrorKind::Denied,
                "TEST_HERMES_RESEARCH_NOT_AVAILABLE",
            ))
        }

        fn interrupt(&mut self, _request_id: &RequestId) -> PortResult<()> {
            Ok(())
        }
    }

    impl FullChainHermesPort for RecordingHermesLifecycle {
        fn runtime_kind(&self) -> RuntimeKind {
            RuntimeKind::Live
        }

        fn research_canonical(
            &mut self,
            _request: &HermesResearchRequest,
            _graph_request: &GraphMemoryRunRequest,
            _graph_receipt: &GraphMemoryReceipt,
        ) -> PortResult<ProductionHermesOutput> {
            Err(PortError::new(
                Component::Hermes,
                PortErrorKind::Denied,
                "TEST_HERMES_RESEARCH_NOT_AVAILABLE",
            ))
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CanonicalReflectionEvent {
        Ready,
        ReflectionLoadMiss,
        GraphReceiptLoad,
        Research,
        Persist,
        ReflectionReload,
        ReflectionLoadHit,
    }

    struct RecordingReflectionHermes {
        events: Arc<Mutex<Vec<CanonicalReflectionEvent>>>,
        ready_calls: usize,
        research_calls: usize,
        research_failure: Option<PortError>,
        sealed: bool,
        seal: HermesProductionSeal,
    }

    impl production_hermes_sealed::Sealed for RecordingReflectionHermes {
        fn has_production_seal(&self) -> bool {
            self.sealed
        }

        fn is_production_configured(&self) -> bool {
            true
        }

        fn ensure_ready(&mut self, _run_id: &str) -> Result<(), LatticedError> {
            self.ready_calls += 1;
            self.events
                .lock()
                .expect("events lock")
                .push(CanonicalReflectionEvent::Ready);
            Ok(())
        }

        fn terminate(&mut self) -> Result<(), LatticedError> {
            Ok(())
        }
    }

    impl HermesPort for RecordingReflectionHermes {
        fn research(&mut self, _request: HermesResearchRequest) -> PortResult<HermesEvidence> {
            Err(PortError::new(
                Component::Hermes,
                PortErrorKind::Denied,
                "TEST_CANONICAL_HERMES_RESEARCH_REQUIRES_GRAPH",
            ))
        }

        fn interrupt(&mut self, _request_id: &RequestId) -> PortResult<()> {
            Ok(())
        }
    }

    impl FullChainHermesPort for RecordingReflectionHermes {
        fn runtime_kind(&self) -> RuntimeKind {
            RuntimeKind::Live
        }

        fn research_canonical(
            &mut self,
            request: &HermesResearchRequest,
            graph_request: &GraphMemoryRunRequest,
            graph_receipt: &GraphMemoryReceipt,
        ) -> PortResult<ProductionHermesOutput> {
            self.research_calls += 1;
            self.events
                .lock()
                .expect("events lock")
                .push(CanonicalReflectionEvent::Research);
            if let Some(failure) = self.research_failure.clone() {
                return Err(failure);
            }
            let content = HermesReflectionContent::new(
                "The exact graph receipt supports one deterministic finding.",
                vec![
                    HermesReflectionFinding::new(
                        "Persist only the exact graph-bound Hermes candidate.",
                        test_content_digest('6'),
                    )
                    .expect("finding"),
                ],
                vec!["Replay the persisted candidate for Status.".to_owned()],
            )
            .expect("reflection content");
            let candidate = HermesReflectionCandidate::new(
                graph_request,
                graph_receipt,
                content,
                self.seal.receipt_digest.clone(),
                test_content_digest('7'),
                test_content_digest('8'),
            )
            .expect("reflection candidate");
            let evidence = HermesEvidence::new(
                request.invocation().clone(),
                RuntimeKind::Live,
                candidate.reflection_digest().clone(),
            );
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

    fn canonical_reflection_request() -> GraphMemoryRunRequest {
        GraphMemoryRunRequest::new(
            Invocation::new(
                CONTRACT_VERSION,
                RequestId::new("task066-graph-request").expect("request"),
                TaskId::new(GRAPH_TASK_ID).expect("task"),
                AttemptId::new("task066-graph-attempt").expect("attempt"),
                ProjectSnapshotId::new(GRAPH_PROJECT_SNAPSHOT_ID).expect("snapshot"),
                test_content_digest('a'),
            )
            .expect("invocation"),
            ProjectId::new(GRAPH_PROJECT_ID).expect("project"),
            GitObjectId::new("1".repeat(40)).expect("commit"),
            test_content_digest('b'),
            test_content_digest('c'),
            GRAPH_RETRIEVAL_LIMIT,
        )
        .expect("graph request")
    }

    fn canonical_graph_receipt(request: &GraphMemoryRunRequest) -> GraphMemoryReceipt {
        let persistence = GraphMemoryPersistenceEvidence::replay(
            request.clone(),
            CodebaseMemoryPersistenceIdentity::v2(
                test_content_digest('1'),
                test_content_digest('2'),
                test_content_digest('3'),
                test_content_digest('4'),
            )
            .expect("persistence identity"),
            test_content_digest('d'),
            test_content_digest('e'),
            1,
            test_content_digest('f'),
        )
        .expect("persistence evidence");
        let retrieval = MemoryRetrievalEvidence::replay(
            &persistence,
            request.retrieval_limit(),
            MemoryRetrievalDisposition::NoAnswer,
            Vec::new(),
            test_content_digest('5'),
            test_content_digest('6'),
        )
        .expect("retrieval evidence");
        GraphMemoryReceipt::new(persistence, retrieval, test_content_digest('7'))
            .expect("graph receipt")
    }

    fn task068_graph_memory_fixture() -> (NormalizedGraphAnalysis, MemoryRetrievalPlan) {
        let query_text = "TASK068CanonicalHermesReflection";
        let request = GraphMemoryRunRequest::new(
            Invocation::new(
                CONTRACT_VERSION,
                RequestId::new("task068-live-request").expect("request id"),
                TaskId::new("TASK-068").expect("task id"),
                AttemptId::new("task068-live-attempt").expect("attempt id"),
                ProjectSnapshotId::new("task068-live-snapshot").expect("snapshot id"),
                test_content_digest('a'),
            )
            .expect("invocation"),
            ProjectId::new("task068-hermes-replay").expect("project"),
            GitObjectId::new("3".repeat(40)).expect("commit"),
            digest_query_text(query_text).expect("query digest"),
            test_content_digest('c'),
            5,
        )
        .expect("graph request");
        let source =
            TrackedSource::new("src/task068.rs", test_content_digest('d')).expect("tracked source");
        let snapshot = CodeSnapshotEvidence::new(
            &request,
            GitObjectId::new("4".repeat(40)).expect("tree"),
            vec![source.clone()],
            test_content_digest('e'),
            test_content_digest('f'),
        )
        .expect("snapshot");
        let provenance = GraphSourceProvenance::new(&source, Some(1), Some(2)).expect("provenance");
        let raw = GraphifyRawEvidence::new(
            &request,
            &snapshot,
            GraphifyIdentity::task033(
                test_content_digest('1'),
                test_content_digest('2'),
                test_content_digest('3'),
            )
            .expect("graphify identity"),
            vec![
                GraphifyRawNode::new(
                    "node-task068-canonical-hermes-reflection",
                    query_text,
                    "trait",
                    provenance,
                    GraphConfidence::Extracted,
                )
                .expect("graph node"),
            ],
            Vec::new(),
            test_content_digest('4'),
            test_content_digest('5'),
            test_content_digest('6'),
        )
        .expect("raw graph evidence");
        let analysis = normalize_analysis(&request, &snapshot, &raw).expect("normalized analysis");
        let query = MemoryQuery::new(&request, query_text, 5).expect("memory query");
        let plan = plan_retrieval(&analysis, &query).expect("retrieval plan");
        (analysis, plan)
    }

    fn task068_reflection_candidate(
        graph_receipt: &GraphMemoryReceipt,
    ) -> HermesReflectionCandidate {
        HermesReflectionCandidate::new(
            graph_receipt.persistence().request(),
            graph_receipt,
            HermesReflectionContent::new(
                "The exact graph receipt supports one deterministic finding.",
                vec![
                    HermesReflectionFinding::new(
                        "Persist only the exact graph-bound Hermes candidate.",
                        test_content_digest('6'),
                    )
                    .expect("finding"),
                ],
                vec!["Replay the persisted candidate for Status.".to_owned()],
            )
            .expect("reflection content"),
            test_content_digest('9'),
            test_content_digest('7'),
            test_content_digest('8'),
        )
        .expect("reflection candidate")
    }

    fn assert_task068_reflection(
        receipt: &HermesReflectionReceipt,
        candidate: &HermesReflectionCandidate,
    ) {
        assert_eq!(receipt.request(), candidate.request());
        assert_eq!(
            receipt.graph_receipt_digest(),
            candidate.graph_receipt_digest()
        );
        assert_eq!(receipt.content(), candidate.content());
        assert_eq!(
            receipt.hermes_identity_digest(),
            candidate.hermes_identity_digest()
        );
        assert_eq!(receipt.input_digest(), candidate.input_digest());
        assert_eq!(receipt.reflection_digest(), candidate.reflection_digest());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires the marker-owned TASK-019 PostgreSQL restart harness"]
    fn canonical_hermes_reflection_survives_postgres_restart_when_provisioned() {
        assert_eq!(
            required_environment("LATTICE_TASK019_LIVE").expect("live gate"),
            "1"
        );
        let phase = required_environment("LATTICE_TASK019_PHASE").expect("phase");
        assert!(matches!(phase.as_str(), "initial" | "restart"));
        let port = required_environment("LATTICE_TASK019_PORT")
            .expect("port")
            .parse::<u16>()
            .expect("valid port");
        let database = DeliveryDatabaseBinding::new(
            required_environment("LATTICE_TASK019_HOST").expect("host"),
            port,
            required_environment("LATTICE_TASK019_RUN_ID").expect("run id"),
        )
        .expect("marker-owned database binding");
        let password = required_environment("LATTICE_TASK019_PASSWORD").expect("password");
        let timeout = Duration::from_secs(30);
        let (analysis, plan) = task068_graph_memory_fixture();
        let request = analysis.request().clone();
        let (ready_calls, research_calls, persist_calls);

        let receipt = if phase == "initial" {
            let mut memory =
                reflection_memory(&database, &password, timeout, GraphMemoryStage::Persistence)
                    .expect("production memory owner");
            let persistence = memory
                .persist_analysis(&analysis)
                .expect("persist deterministic graph analysis");
            let graph_receipt = memory
                .retrieve(&persistence, plan)
                .expect("persist deterministic graph receipt");
            drop(memory);
            let candidate = task068_reflection_candidate(&graph_receipt);
            let events = Arc::new(Mutex::new(Vec::new()));
            let mut hermes = RecordingReflectionHermes {
                events,
                ready_calls: 0,
                research_calls: 0,
                research_failure: None,
                sealed: true,
                seal: HermesProductionSeal {
                    receipt_digest: test_content_digest('9'),
                },
            };
            apply_canonical_hermes_tool_policy(
                &mut hermes,
                database.run_id(),
                CanonicalHermesTool::DeliveryRun,
            )
            .expect("Delivery Run readies the canonical Hermes owner");
            let mut reflection_load_calls = 0;
            let mut graph_load_calls = 0;
            let mut persistence_calls = 0;
            let replayed = load_or_run_canonical_reflection(
                &mut hermes,
                database.run_id(),
                &request,
                |request| {
                    reflection_load_calls += 1;
                    load_reflection_from_postgres(&database, &password, timeout, request)
                },
                |request| {
                    graph_load_calls += 1;
                    load_delivery_graph_receipt(
                        &database,
                        &password,
                        deadline(timeout).expect("deadline"),
                        request,
                    )
                },
                |candidate| {
                    persistence_calls += 1;
                    persist_reflection_to_postgres(&database, &password, timeout, candidate)
                },
            )
            .expect("production reflection round");
            assert_task068_reflection(&replayed, &candidate);
            assert_eq!(reflection_load_calls, 2);
            assert_eq!(graph_load_calls, 1);
            assert_eq!(persistence_calls, 1);
            ready_calls = hermes.ready_calls;
            research_calls = hermes.research_calls;
            persist_calls = persistence_calls;
            assert_eq!(ready_calls, 1);
            assert_eq!(research_calls, 1);
            replayed
        } else {
            let graph_receipt = load_delivery_graph_receipt(
                &database,
                &password,
                deadline(timeout).expect("deadline"),
                &request,
            )
            .expect("restart graph receipt");
            let candidate = task068_reflection_candidate(&graph_receipt);
            let events = Arc::new(Mutex::new(Vec::new()));
            let mut hermes = RecordingReflectionHermes {
                events: Arc::clone(&events),
                ready_calls: 0,
                research_calls: 0,
                research_failure: None,
                sealed: false,
                seal: HermesProductionSeal {
                    receipt_digest: test_content_digest('8'),
                },
            };
            apply_canonical_hermes_tool_policy(
                &mut hermes,
                database.run_id(),
                CanonicalHermesTool::DeliveryStatus,
            )
            .expect("fresh Status stays Hermes-free");
            let load_events = Arc::clone(&events);
            let replayed = load_canonical_reflection(&request, |request| {
                load_events
                    .lock()
                    .expect("events lock")
                    .push(CanonicalReflectionEvent::ReflectionLoadHit);
                load_reflection_from_postgres(&database, &password, timeout, request)
            })
            .expect("fresh Status exact replay");
            assert_eq!(hermes.ready_calls, 0);
            assert_eq!(hermes.research_calls, 0);
            assert!(!production_hermes_sealed::Sealed::has_production_seal(
                &hermes
            ));
            assert_eq!(
                *events.lock().expect("events lock"),
                vec![CanonicalReflectionEvent::ReflectionLoadHit]
            );
            assert_task068_reflection(&replayed, &candidate);
            let expected = ContentDigest::from_sha256(
                required_environment("LATTICE_TASK068_EXPECTED_RECEIPT_SHA256")
                    .expect("initial receipt digest"),
            )
            .expect("valid initial receipt digest");
            assert_eq!(replayed.receipt_digest(), &expected);
            ready_calls = hermes.ready_calls;
            research_calls = hermes.research_calls;
            persist_calls = 0;
            replayed
        };

        println!(
            "TASK068_HERMES_POSTGRES_REPLAY_OK phase={} receipt_sha256={} ready_calls={} research_calls={} persist_calls={}",
            phase,
            receipt.receipt_digest().as_str(),
            ready_calls,
            research_calls,
            persist_calls
        );
    }

    #[test]
    fn canonical_delivery_run_reflects_once_persists_and_replays_exact_receipt() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let stored = Arc::new(Mutex::new(None::<HermesReflectionReceipt>));
        let request = canonical_reflection_request();
        let graph_receipt = canonical_graph_receipt(&request);
        let mut hermes = RecordingReflectionHermes {
            events: Arc::clone(&events),
            ready_calls: 0,
            research_calls: 0,
            research_failure: None,
            sealed: true,
            seal: HermesProductionSeal {
                receipt_digest: test_content_digest('9'),
            },
        };

        apply_canonical_hermes_tool_policy(
            &mut hermes,
            "task066-run",
            CanonicalHermesTool::DeliveryRun,
        )
        .expect("delivery run readies Hermes");
        assert!(production_hermes_sealed::Sealed::has_production_seal(
            &hermes
        ));

        let load_events = Arc::clone(&events);
        let load_stored = Arc::clone(&stored);
        let graph_events = Arc::clone(&events);
        let persist_events = Arc::clone(&events);
        let persist_stored = Arc::clone(&stored);
        let expected_graph_receipt = graph_receipt.clone();
        let replayed = load_or_run_canonical_reflection(
            &mut hermes,
            "task066-run",
            &request,
            move |_request| {
                let stored = load_stored.lock().expect("stored lock");
                if let Some(receipt) = stored.as_ref() {
                    load_events
                        .lock()
                        .expect("events lock")
                        .push(CanonicalReflectionEvent::ReflectionReload);
                    Ok(receipt.clone())
                } else {
                    load_events
                        .lock()
                        .expect("events lock")
                        .push(CanonicalReflectionEvent::ReflectionLoadMiss);
                    Err(GraphMemoryPortError::new(
                        GraphMemoryStage::ReflectionReceipt,
                        PortErrorKind::Unavailable,
                        GraphMemoryFailureCertainty::Known,
                        "MEMORY_RECEIPT_UNAVAILABLE",
                    ))
                }
            },
            move |_request| {
                graph_events
                    .lock()
                    .expect("events lock")
                    .push(CanonicalReflectionEvent::GraphReceiptLoad);
                Ok(expected_graph_receipt)
            },
            move |candidate| {
                persist_events
                    .lock()
                    .expect("events lock")
                    .push(CanonicalReflectionEvent::Persist);
                let receipt = HermesReflectionReceipt::from_candidate(
                    candidate.clone(),
                    test_content_digest('a'),
                )
                .expect("persisted reflection receipt");
                *persist_stored.lock().expect("stored lock") = Some(receipt.clone());
                Ok(receipt)
            },
        )
        .expect("canonical reflection round");

        let persisted = stored
            .lock()
            .expect("stored lock")
            .clone()
            .expect("persisted receipt");
        assert_eq!(replayed, persisted);
        assert_eq!(replayed.receipt_digest(), persisted.receipt_digest());
        assert_eq!(hermes.ready_calls, 1);
        assert_eq!(hermes.research_calls, 1);
        assert!(production_hermes_sealed::Sealed::has_production_seal(
            &hermes
        ));
        assert_eq!(
            *events.lock().expect("events lock"),
            vec![
                CanonicalReflectionEvent::Ready,
                CanonicalReflectionEvent::ReflectionLoadMiss,
                CanonicalReflectionEvent::GraphReceiptLoad,
                CanonicalReflectionEvent::Research,
                CanonicalReflectionEvent::Persist,
                CanonicalReflectionEvent::ReflectionReload,
            ]
        );

        events.lock().expect("events lock").clear();
        let mut status_hermes = RecordingReflectionHermes {
            events: Arc::clone(&events),
            ready_calls: 0,
            research_calls: 0,
            research_failure: None,
            sealed: false,
            seal: HermesProductionSeal {
                receipt_digest: test_content_digest('9'),
            },
        };
        apply_canonical_hermes_tool_policy(
            &mut status_hermes,
            "task066-run",
            CanonicalHermesTool::DeliveryStatus,
        )
        .expect("fresh status keeps Hermes inactive");
        let status_events = Arc::clone(&events);
        let expected = persisted.clone();
        let status_replay = load_canonical_reflection(&request, move |_request| {
            status_events
                .lock()
                .expect("events lock")
                .push(CanonicalReflectionEvent::ReflectionLoadHit);
            Ok(expected)
        })
        .expect("fresh status replays the persisted Run candidate");

        assert_eq!(status_replay, persisted);
        assert_eq!(status_hermes.ready_calls, 0);
        assert_eq!(status_hermes.research_calls, 0);
        assert!(!production_hermes_sealed::Sealed::has_production_seal(
            &status_hermes
        ));
        assert_eq!(
            *events.lock().expect("events lock"),
            vec![CanonicalReflectionEvent::ReflectionLoadHit]
        );
    }

    #[test]
    fn canonical_reflection_reload_mismatch_fails_closed() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let request = canonical_reflection_request();
        let graph_receipt = canonical_graph_receipt(&request);
        let mismatched = HermesReflectionReceipt::new(
            &request,
            &graph_receipt,
            HermesReflectionContent::new("Substituted replay.", Vec::new(), Vec::new())
                .expect("reflection content"),
            test_content_digest('8'),
            test_content_digest('9'),
            test_content_digest('a'),
            test_content_digest('b'),
        )
        .expect("mismatched receipt");
        let mut hermes = RecordingReflectionHermes {
            events,
            ready_calls: 0,
            research_calls: 0,
            research_failure: None,
            sealed: true,
            seal: HermesProductionSeal {
                receipt_digest: test_content_digest('c'),
            },
        };
        let expected_graph_receipt = graph_receipt.clone();
        let mut load_calls = 0;

        let error = load_or_run_canonical_reflection(
            &mut hermes,
            "task066-mismatch",
            &request,
            |_request| {
                load_calls += 1;
                if load_calls == 1 {
                    Err(GraphMemoryPortError::new(
                        GraphMemoryStage::ReflectionReceipt,
                        PortErrorKind::Unavailable,
                        GraphMemoryFailureCertainty::Known,
                        "MEMORY_RECEIPT_UNAVAILABLE",
                    ))
                } else {
                    Ok(mismatched.clone())
                }
            },
            move |_request| Ok(expected_graph_receipt),
            move |candidate| {
                HermesReflectionReceipt::from_candidate(candidate.clone(), test_content_digest('d'))
                    .map_err(|_| LatticedError::new(LatticedErrorKind::HermesExecution))
            },
        )
        .expect_err("substituted reload must fail closed");

        assert_eq!(error.kind(), LatticedErrorKind::HermesReceiptRead);
        assert_eq!(hermes.research_calls, 1);
    }

    #[test]
    fn canonical_hermes_reconciliation_required_is_not_collapsed_to_execution_failure() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let request = canonical_reflection_request();
        let graph_receipt = canonical_graph_receipt(&request);
        let expected_graph_receipt = graph_receipt.clone();
        let mut hermes = RecordingReflectionHermes {
            events: Arc::clone(&events),
            ready_calls: 0,
            research_calls: 0,
            research_failure: Some(PortError::new(
                Component::Hermes,
                PortErrorKind::Ambiguous,
                "HERMES_RUN_RECONCILIATION_REQUIRED",
            )),
            sealed: true,
            seal: HermesProductionSeal {
                receipt_digest: test_content_digest('d'),
            },
        };
        let mut load_calls = 0;

        let error = load_or_run_canonical_reflection(
            &mut hermes,
            "task071-reconcile",
            &request,
            |_request| {
                load_calls += 1;
                Err(GraphMemoryPortError::new(
                    GraphMemoryStage::ReflectionReceipt,
                    PortErrorKind::Unavailable,
                    GraphMemoryFailureCertainty::Known,
                    "MEMORY_RECEIPT_UNAVAILABLE",
                ))
            },
            move |_request| Ok(expected_graph_receipt),
            |_candidate| panic!("reconciliation-required output must not persist"),
        )
        .expect_err("active Hermes run remains reconciliation-required");

        assert_eq!(error.kind(), LatticedErrorKind::ReconciliationRequired);
        assert_eq!(error.code(), "LATTICE_DELIVERY_RECONCILIATION_REQUIRED");
        assert_eq!(load_calls, 1);
        assert_eq!(hermes.research_calls, 1);
        assert_eq!(
            *events.lock().expect("events lock"),
            vec![CanonicalReflectionEvent::Research]
        );

        for failure in [
            PortError::new(
                Component::Hermes,
                PortErrorKind::Ambiguous,
                "HERMES_RUN_NOT_RECOVERABLE",
            ),
            PortError::new(
                Component::Hermes,
                PortErrorKind::Malformed,
                "HERMES_STATUS_MALFORMED",
            ),
        ] {
            assert_eq!(
                map_hermes_research_error(&failure).kind(),
                LatticedErrorKind::HermesExecution
            );
        }
    }

    #[test]
    fn production_owner_seal_survives_ready_to_bound_transition() {
        let mut ready = Some(());
        let mut bound = None;
        assert!(full_chain_hermes_state_has_seal(
            ready.as_ref(),
            bound.as_ref()
        ));

        bound = ready.take();
        assert!(full_chain_hermes_state_has_seal(
            ready.as_ref(),
            bound.as_ref()
        ));

        bound = None;
        assert!(!full_chain_hermes_state_has_seal(
            ready.as_ref(),
            bound.as_ref()
        ));
    }

    #[test]
    fn canonical_hermes_recovery_is_receipt_gated_and_never_resubmits() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let output = run_or_reconcile_active_hermes(
            &mut (),
            |()| Ok::<_, PortError>("initial"),
            |()| panic!("successful run must not inspect recovery state"),
            |(), _: &u8| panic!("successful run must not reconcile"),
        )
        .expect("initial success is unchanged");
        assert_eq!(output, "initial");

        for (kind, code) in [
            (
                PortErrorKind::VersionMismatch,
                "HERMES_PACKAGE_VERSION_MISMATCH",
            ),
            (
                PortErrorKind::CapabilityMismatch,
                "HERMES_UNEXPECTED_EXECUTION_EVENT",
            ),
            (
                PortErrorKind::Malformed,
                "HERMES_EVENT_DISCRIMINATOR_REJECTED",
            ),
            (PortErrorKind::Cancelled, "HERMES_RUN_CANCELLED"),
            (PortErrorKind::Denied, "HERMES_EVENT_STATUS_OUTPUT_MISMATCH"),
            (PortErrorKind::Ambiguous, "HERMES_RUN_NOT_RECOVERABLE"),
            (
                PortErrorKind::Timeout,
                "HERMES_PRODUCTION_DEADLINE_EXCEEDED",
            ),
            (PortErrorKind::Unavailable, "HERMES_RUN_FAILED"),
            (PortErrorKind::Unavailable, "HERMES_STATUS_HTTP_REJECTED"),
        ] {
            let initial_failure = run_or_reconcile_active_hermes(
                &mut (),
                |()| Err::<&str, _>(PortError::new(Component::Hermes, kind, code)),
                |()| panic!("non-recoverable failure must not inspect recovery state"),
                |(), _: &u8| panic!("non-recoverable failure must not reconcile"),
            )
            .expect_err("non-recoverable failure remains fail-closed");
            assert_eq!(initial_failure.kind(), kind);
            assert_eq!(initial_failure.code(), code);
        }

        let events = Rc::new(RefCell::new(Vec::new()));
        let run_events = Rc::clone(&events);
        let receipt_events = Rc::clone(&events);
        let initial_failure = run_or_reconcile_active_hermes(
            &mut (),
            move |()| {
                run_events.borrow_mut().push("run");
                Err::<&str, _>(PortError::new(
                    Component::Hermes,
                    PortErrorKind::Unavailable,
                    "HERMES_LOOPBACK_TRANSPORT_FAILED",
                ))
            },
            move |()| {
                receipt_events.borrow_mut().push("active_receipt");
                None::<u8>
            },
            |(), _| panic!("missing receipt must not reconcile"),
        )
        .expect_err("missing receipt preserves the initial failure");
        assert_eq!(initial_failure.kind(), PortErrorKind::Unavailable);
        assert_eq!(initial_failure.code(), "HERMES_LOOPBACK_TRANSPORT_FAILED");
        assert_eq!(&*events.borrow(), &["run", "active_receipt"]);

        for (kind, code) in [
            (PortErrorKind::Timeout, "HERMES_LOOPBACK_TIMEOUT"),
            (PortErrorKind::Timeout, "HERMES_RUN_DEADLINE_EXCEEDED"),
            (
                PortErrorKind::Unavailable,
                "HERMES_LOOPBACK_TRANSPORT_FAILED",
            ),
        ] {
            let events = Rc::new(RefCell::new(Vec::new()));
            let run_events = Rc::clone(&events);
            let receipt_events = Rc::clone(&events);
            let reconcile_events = Rc::clone(&events);
            let recovered = run_or_reconcile_active_hermes(
                &mut (),
                move |()| {
                    run_events.borrow_mut().push("run");
                    Err::<&str, _>(PortError::new(Component::Hermes, kind, code))
                },
                move |()| {
                    receipt_events.borrow_mut().push("active_receipt");
                    Some(7_u8)
                },
                move |(), receipt| {
                    reconcile_events.borrow_mut().push("reconcile");
                    assert_eq!(*receipt, 7);
                    Ok("normalized-evidence")
                },
            )
            .expect("active known-run receipt permits one same-port reconciliation");
            assert_eq!(recovered, "normalized-evidence");
            assert_eq!(&*events.borrow(), &["run", "active_receipt", "reconcile"]);
        }

        for (kind, code) in [
            (PortErrorKind::Timeout, "HERMES_LOOPBACK_TIMEOUT"),
            (PortErrorKind::Timeout, "HERMES_RUN_DEADLINE_EXCEEDED"),
            (
                PortErrorKind::Unavailable,
                "HERMES_LOOPBACK_TRANSPORT_FAILED",
            ),
        ] {
            let repeated_uncertainty = run_or_reconcile_active_hermes(
                &mut (),
                |()| {
                    Err::<&str, _>(PortError::new(
                        Component::Hermes,
                        PortErrorKind::Timeout,
                        "HERMES_LOOPBACK_TIMEOUT",
                    ))
                },
                |()| Some(7_u8),
                |(), receipt| {
                    assert_eq!(*receipt, 7);
                    Err(PortError::new(Component::Hermes, kind, code))
                },
            )
            .expect_err("a second transient observation remains reconciliation-required");
            assert_eq!(repeated_uncertainty.kind(), PortErrorKind::Ambiguous);
            assert_eq!(
                repeated_uncertainty.code(),
                "HERMES_RUN_RECONCILIATION_REQUIRED"
            );
        }

        let definitive_failure = run_or_reconcile_active_hermes(
            &mut (),
            |()| {
                Err::<&str, _>(PortError::new(
                    Component::Hermes,
                    PortErrorKind::Timeout,
                    "HERMES_LOOPBACK_TIMEOUT",
                ))
            },
            |()| Some(7_u8),
            |(), _| {
                Err(PortError::new(
                    Component::Hermes,
                    PortErrorKind::Malformed,
                    "HERMES_STATUS_MALFORMED",
                ))
            },
        )
        .expect_err("definitive reconciliation failure remains exact");
        assert_eq!(definitive_failure.kind(), PortErrorKind::Malformed);
        assert_eq!(definitive_failure.code(), "HERMES_STATUS_MALFORMED");
    }

    #[test]
    fn canonical_tool_policy_activates_only_delivery_run_and_terminates_once() {
        let mut hermes = RecordingHermesLifecycle {
            ready_calls: 0,
            terminate_calls: 0,
            teardown_fails: false,
        };
        for tool in [
            CanonicalHermesTool::DeliveryStatus,
            CanonicalHermesTool::TaskSubmit,
            CanonicalHermesTool::TaskStatus,
        ] {
            apply_canonical_hermes_tool_policy(&mut hermes, "task064-run", tool)
                .expect("non-delivery tools keep Hermes inactive");
        }
        assert_eq!(hermes.ready_calls, 0);

        apply_canonical_hermes_tool_policy(
            &mut hermes,
            "task064-run",
            CanonicalHermesTool::DeliveryRun,
        )
        .expect("delivery run activates Hermes");
        assert_eq!(hermes.ready_calls, 1);

        finish_hermes_owner(Ok(()), &mut hermes).expect("explicit teardown succeeds");
        assert_eq!(hermes.terminate_calls, 1);

        let mut task_only = DeferredTaskHermes;
        apply_canonical_hermes_tool_policy(
            &mut task_only,
            "task064-task-only",
            CanonicalHermesTool::DeliveryRun,
        )
        .expect("task-only delivery retains the pre-TASK-064 path");
    }

    #[test]
    fn canonical_session_calls_teardown_once_and_propagates_failure() {
        let mut hermes = RecordingHermesLifecycle {
            ready_calls: 0,
            terminate_calls: 0,
            teardown_fails: true,
        };

        let error = finish_hermes_owner(
            Err(LatticedError::new(LatticedErrorKind::Transport)),
            &mut hermes,
        )
        .expect_err("teardown ambiguity overrides transport failure");

        assert_eq!(error.kind(), LatticedErrorKind::HermesTeardownRejected);
        assert_eq!(hermes.terminate_calls, 1);
    }

    #[cfg(windows)]
    #[test]
    fn canonical_production_hermes_is_configured_but_inactive_until_delivery_run() {
        let mut hermes = CanonicalHermes::Production {
            active: None,
            activation_attempted: false,
        };

        assert!(production_hermes_sealed::Sealed::is_production_configured(
            &hermes
        ));
        assert!(!production_hermes_sealed::Sealed::has_production_seal(
            &hermes
        ));
        assert_eq!(hermes.runtime_kind(), RuntimeKind::Fake);
        production_hermes_sealed::Sealed::terminate(&mut hermes)
            .expect("inactive production mode has no process to reap");
    }

    #[test]
    fn hermes_teardown_failure_overrides_stdio_success_or_failure() {
        assert_eq!(
            finish_hermes_session(
                Ok(()),
                Err(LatticedError::new(
                    LatticedErrorKind::HermesTeardownRejected,
                )),
            )
            .expect_err("teardown ambiguity cannot become success")
            .kind(),
            LatticedErrorKind::HermesTeardownRejected
        );
        assert_eq!(
            finish_hermes_session(
                Err(LatticedError::new(LatticedErrorKind::Transport)),
                Err(LatticedError::new(
                    LatticedErrorKind::HermesTeardownRejected,
                )),
            )
            .expect_err("teardown ambiguity has precedence")
            .kind(),
            LatticedErrorKind::HermesTeardownRejected
        );
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
    fn full_chain_startup_requires_prepared_assets_before_a_production_hermes_runner() {
        let error = serve_full_chain_from_environment()
            .expect_err("incomplete official Hermes chain fails before external effects");
        assert_eq!(error.kind(), LatticedErrorKind::HermesPreparationRequired);
        assert_eq!(error.code(), "LATTICE_HERMES_PREPARATION_REJECTED");
    }

    #[test]
    fn parsed_hermes_configuration_is_not_launch_authority() {
        assert_eq!(
            classify_hermes_production_preflight(Vec::new(), true),
            HermesProductionPreflight::ConfigurationPresentUnverified
        );
        assert_eq!(
            classify_hermes_production_preflight(Vec::new(), true).render(),
            "LATTICE_HERMES_PREFLIGHT_CONFIGURATION_PRESENT_UNVERIFIED"
        );
    }

    #[cfg(windows)]
    #[test]
    fn production_hermes_environment_accepts_only_the_frozen_runtime_identity() {
        let manifest_bytes = br#"{"cpython_archive_bytes":111375313,"cpython_archive_sha256":"a140c0868258075d160fa0da51ddffd423efbc9dd350695abd33e7ce3ce94352","cpython_build_release":"20260804","cpython_provenance":"astral-sh/python-build-standalone","cpython_sha256sums_sha256":"eccfdcc61c9fe48b7fe61db8812925ce30f23943d16c60861001004a4ae8f55c","cpython_version":"3.12.13","hermes_archive_sha256":"a9a84a25999a23a859a9d17ef3134ea1c3371d8bf1984313eab839e939528152","hermes_commit":"3c27eb6234bf91b8ceee9e9071591b31e9b148cb","hermes_release":"v2026.8.3","payload_byte_count":722643145,"payload_file_count":14077,"payload_manifest_sha256":"cb0e331bcb2b4fe2fd0977401d246819aadb800b645ca31ec233ad4e25b96929","platform":"x86_64-unknown-linux-gnu","pyproject_sha256":"64d1085ee1c23caf0ae0d9e65c73e280f466362ed43fdda1531f18f3af1d9869","schema":"lattice.hermes.offline-runtime.v1","uv_lock_sha256":"aab3c83f71b683507a590b6315b23bdc0abd6b63b76b2349eae15bf00dfbaf2b"}"#;
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
                }),
                Ok(()),
                Err(OpenClawPumpFailure {
                    kind: GatewayTransportErrorKind::Unavailable,
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
                },
                OpenClawPumpFailure {
                    kind: GatewayTransportErrorKind::Unavailable,
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

    #[test]
    fn hermes_launch_lifecycle_launches_once_and_terminates_owner_on_stdin_eof() {
        use std::cell::Cell;
        use std::io::Cursor;
        use std::rc::Rc;

        struct FakeOwner {
            verifies: Rc<Cell<usize>>,
            terminations: Rc<Cell<usize>>,
        }

        impl HermesStandaloneOwner for FakeOwner {
            fn verify_live(&mut self) -> Result<(), LatticedError> {
                self.verifies.set(self.verifies.get() + 1);
                Ok(())
            }

            fn terminate(self) -> Result<(), LatticedError> {
                self.terminations.set(self.terminations.get() + 1);
                Ok(())
            }
        }

        let launches = Rc::new(Cell::new(0));
        let verifies = Rc::new(Cell::new(0));
        let terminations = Rc::new(Cell::new(0));
        let observed_launches = Rc::clone(&launches);
        let observed_verifies = Rc::clone(&verifies);
        let observed_terminations = Rc::clone(&terminations);

        launch_hermes_until_eof(
            Cursor::new(Vec::<u8>::new()),
            Duration::from_millis(1),
            move |run_id| {
                assert_eq!(run_id, "standalone-hermes");
                observed_launches.set(observed_launches.get() + 1);
                Ok(FakeOwner {
                    verifies: observed_verifies,
                    terminations: observed_terminations,
                })
            },
            || Ok(()),
        )
        .expect("EOF explicitly terminates the standalone Hermes owner");

        assert_eq!(launches.get(), 1);
        assert_eq!(verifies.get(), 3);
        assert_eq!(terminations.get(), 1);
    }

    #[test]
    fn hermes_launch_lifecycle_reports_child_exit_and_still_terminates_owner() {
        use std::cell::Cell;
        use std::rc::Rc;
        use std::sync::mpsc;

        struct BlockingReader(mpsc::Receiver<()>);

        impl Read for BlockingReader {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                let _ = self.0.recv();
                Ok(0)
            }
        }

        struct ExitingOwner {
            verifies: Rc<Cell<usize>>,
            terminations: Rc<Cell<usize>>,
        }

        impl HermesStandaloneOwner for ExitingOwner {
            fn verify_live(&mut self) -> Result<(), LatticedError> {
                let next = self.verifies.get() + 1;
                self.verifies.set(next);
                if next >= 3 {
                    Err(LatticedError::new(
                        LatticedErrorKind::HermesProductionLivenessRejected,
                    ))
                } else {
                    Ok(())
                }
            }

            fn terminate(self) -> Result<(), LatticedError> {
                self.terminations.set(self.terminations.get() + 1);
                Ok(())
            }
        }

        let verifies = Rc::new(Cell::new(0));
        let terminations = Rc::new(Cell::new(0));
        let ready = Rc::new(Cell::new(false));
        let observed_verifies = Rc::clone(&verifies);
        let observed_terminations = Rc::clone(&terminations);
        let observed_ready = Rc::clone(&ready);
        let (release, blocked) = mpsc::channel();

        let failure = launch_hermes_until_eof(
            BlockingReader(blocked),
            Duration::from_millis(1),
            move |_| {
                Ok(ExitingOwner {
                    verifies: observed_verifies,
                    terminations: observed_terminations,
                })
            },
            move || {
                observed_ready.set(true);
                Ok(())
            },
        )
        .expect_err("child exit cannot remain falsely healthy");
        drop(release);

        assert_eq!(
            failure.kind(),
            LatticedErrorKind::HermesProductionLivenessRejected
        );
        assert!(ready.get());
        assert!(verifies.get() >= 3);
        assert_eq!(terminations.get(), 1);
    }

    #[test]
    fn hermes_launch_lifecycle_reports_ambiguous_teardown_at_stdin_eof() {
        use std::io::Cursor;

        struct AmbiguousTeardownOwner;

        impl HermesStandaloneOwner for AmbiguousTeardownOwner {
            fn verify_live(&mut self) -> Result<(), LatticedError> {
                Ok(())
            }

            fn terminate(self) -> Result<(), LatticedError> {
                Err(LatticedError::new(
                    LatticedErrorKind::HermesTeardownRejected,
                ))
            }
        }

        let failure = launch_hermes_until_eof(
            Cursor::new(Vec::<u8>::new()),
            Duration::from_millis(1),
            |_| Ok(AmbiguousTeardownOwner),
            || Ok(()),
        )
        .expect_err("teardown ambiguity cannot become success");

        assert_eq!(failure.kind(), LatticedErrorKind::HermesTeardownRejected);
    }

    #[test]
    fn hermes_launch_lifecycle_reports_stdin_failure_after_live_check_and_teardown() {
        use std::cell::Cell;
        use std::rc::Rc;

        struct FailingReader;

        impl Read for FailingReader {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("controlled stdin failure"))
            }
        }

        struct LiveOwner {
            verifies: Rc<Cell<usize>>,
            terminations: Rc<Cell<usize>>,
        }

        impl HermesStandaloneOwner for LiveOwner {
            fn verify_live(&mut self) -> Result<(), LatticedError> {
                self.verifies.set(self.verifies.get() + 1);
                Ok(())
            }

            fn terminate(self) -> Result<(), LatticedError> {
                self.terminations.set(self.terminations.get() + 1);
                Ok(())
            }
        }

        let verifies = Rc::new(Cell::new(0));
        let terminations = Rc::new(Cell::new(0));
        let observed_verifies = Rc::clone(&verifies);
        let observed_terminations = Rc::clone(&terminations);

        let failure = launch_hermes_until_eof(
            FailingReader,
            Duration::from_millis(1),
            move |_| {
                Ok(LiveOwner {
                    verifies: observed_verifies,
                    terminations: observed_terminations,
                })
            },
            || Ok(()),
        )
        .expect_err("stdin failure cannot become success");

        assert_eq!(failure.kind(), LatticedErrorKind::Transport);
        assert_eq!(verifies.get(), 3);
        assert_eq!(terminations.get(), 1);
    }

    #[test]
    fn hermes_launch_lifecycle_emits_ready_after_live_verification_before_input() {
        use std::sync::{Arc, Mutex};

        struct EventReader(Arc<Mutex<Vec<&'static str>>>);

        impl Read for EventReader {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                self.0.lock().expect("event lock").push("input");
                Ok(0)
            }
        }

        struct EventOwner(Arc<Mutex<Vec<&'static str>>>);

        impl HermesStandaloneOwner for EventOwner {
            fn verify_live(&mut self) -> Result<(), LatticedError> {
                self.0.lock().expect("event lock").push("verify");
                Ok(())
            }

            fn terminate(self) -> Result<(), LatticedError> {
                self.0.lock().expect("event lock").push("terminate");
                Ok(())
            }
        }

        let events = Arc::new(Mutex::new(Vec::new()));
        let reader_events = Arc::clone(&events);
        let launch_events = Arc::clone(&events);
        let ready_events = Arc::clone(&events);

        launch_hermes_until_eof(
            EventReader(reader_events),
            Duration::from_millis(1),
            move |_| {
                launch_events.lock().expect("event lock").push("launch");
                Ok(EventOwner(Arc::clone(&launch_events)))
            },
            move || {
                ready_events.lock().expect("event lock").push("ready");
                Ok(())
            },
        )
        .expect("ready signal participates in the bounded lifecycle");

        assert_eq!(
            events.lock().expect("event lock").as_slice(),
            [
                "launch",
                "verify",
                "verify",
                "ready",
                "input",
                "verify",
                "terminate",
            ]
        );
    }

    #[test]
    fn core_status_exposes_a_not_started_receipt_without_reconciliation() {
        let status = core_not_started_status_json();

        assert_eq!(status["component"], "delivery-receipt");
        assert_eq!(status["status"], "NOT_STARTED");
        assert_eq!(status["scope"], "receipt-only");
    }
}
