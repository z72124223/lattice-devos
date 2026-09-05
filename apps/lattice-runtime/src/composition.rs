//! Sole concrete composition root for the bounded TASK-032 delivery lane.

use std::collections::BTreeSet;
use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError, mpsc};
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
    PreparedWorkspaceEvidence, ProjectAuthorityReceipt, ProjectClass, ProjectId, ProjectLifecycle,
    ProjectSnapshotId, RequestId, RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead,
    StoreAuthorityRevision, StoreDaemonInstanceId, SubjectBinding, TaskId, TaskIngressPeerEvidence,
    TaskIntakeBinding, TaskSpecSubmission, WorkspaceChangeEvidence, WriterLeaseAuthorityHead,
};
#[cfg(test)]
use lattice_foreman_state::ForemanSnapshot;
use lattice_foreman_state::{
    DependencyBinding, DependencyContinuation, DependencyContinuationState,
    ForemanCheckpointIntent, ForemanServerObservation, ForemanState, SoleForemanBinding,
    reconstruct,
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
    CodexReflectionBrokerConfig, DirectCodexReflection, HermesOfflineRuntimeManifest,
    HermesProductionRunnerConfig, HermesWslContainmentConfig,
    ProductionHermesPort as HermesAdapterProductionPort, ProductionHermesRunner,
    preparation::verify_official_preparation_for_launch,
};
use lattice_openclaw_adapter::{
    AuthenticationKey, GatewayTransportErrorKind, OpenClawGatewayConfig, OpenClawGatewayServer,
    OpenClawLaunchAttestationKey, OpenClawLaunchAttestationTag, OpenClawOfficialLaunchEvidence,
    OpenClawOfficialLaunchRecord, OpenClawProcessStartNonce,
};
use lattice_orchestrator::{
    ControlledTaskOrchestratorError, ControlledTaskRequest, DeliveryOrchestratorError,
    ForemanCheckpointOrchestratorError, GeneralTaskIntakeError, GeneralTaskIntakeRequest,
    GraphMemoryOrchestratorError, checkpoint_foreman, create_general_task, delivery_status,
    graph_memory_status, run_controlled_task, run_delivery, run_delivery_governed,
    run_graph_memory,
};
#[cfg(test)]
use lattice_ports::TaskLifecycleAutonomyEvidence;
use lattice_ports::{
    ControlledTaskExecutionError, ControlledTaskExecutionErrorKind, ControlledTaskExecutionPort,
    DeliveryCodexPort, DeliveryFailureCertainty, DeliveryLedgerPort, DeliveryPortError,
    DeliveryPortResult, ForemanCoordinationPort, ForemanRuntimeStatus, GatewayService,
    GatewayServiceError, GatewayServiceResult, GraphMemoryFailureCertainty, GraphMemoryPortError,
    GraphMemoryStage, HermesPort, HermesReflectionMemoryPort, PortError, PortErrorKind, PortResult,
    TaskIntakeLifecycleEvidence, TaskIntakeLifecyclePort, TaskLifecycleError,
    TaskLifecycleErrorKind, TaskLifecycleEvidence, TaskLifecyclePort, TaskLifecycleResult,
    TestRunnerPort, WorkspaceGitPort, WriterAuthorityGuardPort,
};
use lattice_postgres_codebase_memory::{
    ExtensionBootstrapGlobalProfile as MemoryBootstrapGlobalProfile,
    ExtensionBootstrapProfile as MemoryBootstrapProfile, ExtensionTarget, PostgresCodebaseMemory,
    apply_extension as apply_postgres_memory_extension, inspect_bootstrap_profile,
    verify_embedded_extension_manifest, verify_extension as verify_memory_extension,
};
use lattice_postgres_foreman::{
    ExecutionEnvironmentDescriptor, ExtensionApplyOutcome as ForemanExtensionApplyOutcome,
    ExtensionDatabaseRole as ForemanExtensionDatabaseRole,
    ExtensionTarget as ForemanExtensionTarget, PostgresForeman, RestartTaskKind,
    apply_extension as apply_postgres_foreman_extension,
    verify_extension as verify_postgres_foreman_extension,
};
use lattice_postgres_store::{
    ControlProductCommand, DatabaseRole as StoreDatabaseRole, MigrationBootstrapProfile,
    MigrationTarget as StoreMigrationTarget, PostgresControlProduct, PostgresForemanCoordination,
    PostgresProjectRegistry, PostgresTaskLedger, PostgresTaskLedgerErrorKind,
    apply_migrations as apply_store_migrations, inspect_migration_profile,
    verify_postgres_schema as verify_store_schema,
};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome as WriterExtensionApplyOutcome,
    ExtensionTarget as WriterLeaseExtensionTarget, PostgresWriterLease, V3BootstrapProfile,
    V3ExtensionTarget, V4ExtensionTarget, V5ExtensionTarget,
    apply_extension as apply_postgres_writer_extension, apply_v3_extension, apply_v4_extension,
    apply_v5_extension, inspect_v3_bootstrap_profile, rebind_existing_v3_extension,
    rebind_v5_for_store_v8, verify_extension as verify_writer_extension,
};
use lattice_task_domain::{
    AcceptanceCriterion, ApprovalRequirement, ApprovalRequirements, Capability, CapabilityRequest,
    DeploymentPolicy, EvidenceType, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    ScopeOperation, TASK_SPEC_SCHEMA_VERSION, TaskBudget, TaskScope, TaskSpec, TaskSpecInput,
    TaskState,
};
use lattice_task_ledger::{
    TaskIngressRequestKind, TaskSubmissionEnvelope, foreman_coordination_identity,
};
use lattice_writer_lease::{
    WriterLeaseAcquireRequest, WriterLeaseRepository, WriterLeaseRepositoryError,
    WriterLeaseRepositoryErrorKind,
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
use crate::managed_file_identity::ManagedEffectBundleGuard;
use crate::managed_foreman_service::{
    FormalForemanIdentity, MANAGED_GRACEFUL_SHUTDOWN_COMPLETE, MANAGED_GRACEFUL_SHUTDOWN_IDLE,
    MANAGED_STATUS_MAX_DURATION, MANAGED_STATUS_TIMEOUT, ManagedForemanServiceConfig,
    ManagedRestartProjectBlockerOutcome, ManagedRestartWriterBlockerOutcome,
    managed_task_public_status, record_managed_restart_project_blocker,
    record_managed_restart_writer_blocker, run_managed_task,
};
use crate::managed_worker_adapter::ManagedWorkerCancellation;
use crate::mcp::{
    self, ControlSnapshotArguments, ControlUpdateArguments, DeliveryToolArguments,
    DeliveryToolService, ForemanCheckpointArguments, ObservedEffectKind,
    TASK_PUBLIC_OBJECTIVE_SUMMARY, TaskStatusArguments, TaskSubmitArguments, ToolExecutionError,
    record_observed_effect, task_public_objective_digest,
};
use crate::project_bridge::{ProjectSelector, ResolvedProjectAuthority, resolve_project_authority};
use crate::task_control::{
    PostgresTaskLifecycle, TaskAdmissionProfile, TaskPersistenceFoundation,
    task_admission_command_id,
};

const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
const POSTGRES_BOOTSTRAP_GLOBAL_ADVISORY_LOCK: i64 = 0x4c41_5454_4943_4501;
const POSTGRES_BOOTSTRAP_GATE_TIMEOUT: Duration = Duration::from_secs(30);
const POSTGRES_BOOTSTRAP_GATE_POLL_INTERVAL: Duration = Duration::from_millis(20);
const MAX_TIMEOUT_SECONDS: u64 = 3_600;
const FINALIZATION_RESERVE: Duration = Duration::from_secs(30);
const CONTROLLED_TASK_MAX_RUNTIME: Duration = Duration::from_mins(5);
const GENERAL_TASK_INGRESS_ID: &str = "lattice_task_submit.v1";
const MANAGED_FOREMAN_MODE_ENV: &str = "LATTICE_MANAGED_FOREMAN_MODE";
const MANAGED_FOREMAN_NODE_ENV: &str = "LATTICE_MANAGED_NODE_EXE";
const MANAGED_FOREMAN_BRIDGE_ENV: &str = "LATTICE_MANAGED_WORKER_BRIDGE";
const MANAGED_FOREMAN_WORKTREE_ROOT_ENV: &str = "LATTICE_MANAGED_WORKTREE_ROOT";
const MANAGED_FOREMAN_NPM_ENV: &str = "LATTICE_MANAGED_NPM_EXE";
const MANAGED_FOREMAN_CARGO_ENV: &str = "LATTICE_MANAGED_CARGO_EXE";
const MANAGED_FOREMAN_EXECUTION_ENVIRONMENT_ENV: &str =
    "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_JSON";
const DELIVERY_CODEX_HOME_ENV: &str = "LATTICE_DELIVERY_CODEX_HOME";
const MANAGED_SUPERVISOR_WORKERS: usize = 4;
const OPENCLAW_PUMP_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(15);
const OPENCLAW_PUMP_WAKE_TIMEOUT: Duration = Duration::from_millis(250);
const FULL_CHAIN_STDIN_POLL_INTERVAL: Duration = Duration::from_millis(25);
const FULL_CHAIN_STDIN_CHUNK_BYTES: usize = 8 * 1024;
const MANAGED_SCHEDULER_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(15);
const MANAGED_SCHEDULER_QUEUE_CAPACITY: usize = 64;
const MANAGED_RESTART_TASK_LIMIT: usize = 1_024;
const MANAGED_CAPACITY_RETRY_DELAY: Duration = Duration::from_secs(1);
const MANAGED_DURABLE_RESCAN_INTERVAL: Duration = Duration::from_secs(30);
const MANAGED_RECOVERY_RETRY_MAX_EXPONENT: u8 = 4;
const MANAGED_FOREMAN_NOT_ACTIVE: &str = "LATTICE_MANAGED_FOREMAN_NOT_ACTIVE";
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
const MANAGED_SCRIPTED_ACTIVE_RESTART_ENV: &str = "LATTICE_MANAGED_SCRIPTED_ACTIVE_RESTART";
const MANAGED_SCRIPTED_OWNER_MARKER_ENV: &str = "LATTICE_MANAGED_SCRIPTED_OWNER_MARKER";
const MANAGED_SCRIPTED_ACTIVE_MARKER_NAME: &str = ".lattice-managed-active-restart-v1";
const MANAGED_SCRIPTED_ACTIVE_MARKER_BYTES: &[u8] = b"lattice.phase4.scripted-active-restart.v1\n";
const MANAGED_SCRIPTED_OWNER_KIND: &str = "LATTICE_PHASE4_MANAGED_FOREMAN_ACCEPTANCE_V1";
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
    RuntimePostgresProvision,
    RuntimePostgresBoundary,
    RuntimePostgresMigration,
    RuntimePostgresExternalAdoption,
    RuntimePostgresForeman,
    RuntimePostgresMigrationPermission,
    RuntimePostgresMigrationUnsafeSetting,
    RuntimePostgresVerification,
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
    GraphSnapshotExecution,
    GraphifyExecution,
    GraphNormalization,
    GraphPersistence,
    GraphRetrieval,
    GraphReceipt,
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
    ForemanReplayCorrupt,
    ForemanReplayUnsupported,
    ForemanReplayUnavailable,
    WriterLease,
    ManagedTeardownRejected,
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
            Self::RuntimePostgresProvision => "LATTICED_RUNTIME_POSTGRES_PROVISION_REJECTED",
            Self::RuntimePostgresBoundary => "LATTICED_RUNTIME_POSTGRES_BOUNDARY_REJECTED",
            Self::RuntimePostgresMigration => "LATTICED_RUNTIME_POSTGRES_MIGRATION_REJECTED",
            Self::RuntimePostgresExternalAdoption => {
                "LATTICED_RUNTIME_POSTGRES_EXTERNAL_ADOPTION_REJECTED"
            }
            Self::RuntimePostgresForeman => "LATTICED_RUNTIME_POSTGRES_FOREMAN_REJECTED",
            Self::RuntimePostgresMigrationPermission => {
                "LATTICED_RUNTIME_POSTGRES_MIGRATION_PERMISSION_REJECTED"
            }
            Self::RuntimePostgresMigrationUnsafeSetting => {
                "LATTICED_RUNTIME_POSTGRES_MIGRATION_SETTING_REJECTED"
            }
            Self::RuntimePostgresVerification => "LATTICED_RUNTIME_POSTGRES_VERIFICATION_REJECTED",
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
            Self::GraphSnapshotExecution => "LATTICE_GRAPH_MEMORY_SNAPSHOT_REJECTED",
            Self::GraphifyExecution => "LATTICE_GRAPH_MEMORY_GRAPHIFY_REJECTED",
            Self::GraphNormalization => "LATTICE_GRAPH_MEMORY_NORMALIZATION_REJECTED",
            Self::GraphPersistence => "LATTICE_GRAPH_MEMORY_PERSISTENCE_REJECTED",
            Self::GraphRetrieval => "LATTICE_GRAPH_MEMORY_RETRIEVAL_REJECTED",
            Self::GraphReceipt => "LATTICE_GRAPH_MEMORY_RECEIPT_STAGE_REJECTED",
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
            Self::ForemanReplayCorrupt => "FOREMAN_REPLAY_CORRUPT",
            Self::ForemanReplayUnsupported => "FOREMAN_REPLAY_UNSUPPORTED",
            Self::ForemanReplayUnavailable => "FOREMAN_REPLAY_UNAVAILABLE",
            Self::WriterLease => "LATTICE_WRITER_LEASE_REJECTED",
            Self::ManagedTeardownRejected => "LATTICE_MANAGED_SCHEDULER_TEARDOWN_REJECTED",
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
    managed_effect_guard: ManagedEffectBundleGuard,
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
        let managed_effect_guard =
            ManagedEffectBundleGuard::capture(real_bundle.facts.files.iter().map(|file| {
                (
                    file.declared_path.clone(),
                    policy.file_policy(file.role).max_bytes,
                )
            }))
            .map_err(|()| {
                OfficialIdentityRejection::FileIdentityChanged(OfficialBundleFileRole::Launcher)
            })?;
        Ok(Self {
            resources,
            real_bundle,
            managed_effect_guard,
            _validated: validated,
        })
    }

    fn resources(&self) -> &PinnedCodexResources {
        &self.resources
    }

    fn ensure_current(&self) -> Result<(), OfficialIdentityRejection> {
        self.real_bundle.ensure_current()?;
        self.managed_effect_guard.verify().map_err(|()| {
            OfficialIdentityRejection::FileIdentityChanged(OfficialBundleFileRole::Launcher)
        })
    }

    fn managed_effect_guard(&self) -> ManagedEffectBundleGuard {
        self.managed_effect_guard.clone()
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
        submission: &TaskSpecSubmission,
        binding: &SubjectBinding,
        store_authority: &StoreAuthorityHead,
        writer_authority: &WriterLeaseAuthorityHead,
        writer_guard: &mut dyn WriterAuthorityGuardPort,
        delivery_root: &Path,
    ) -> Result<Value, LatticedError> {
        if submission.binding() != binding {
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

    /// Replays the durable delivery receipt without running a delivery effect.
    ///
    /// A replay failure remains a reconciliation blocker. This method never
    /// appends a receipt or converts uncertain evidence into a terminal result.
    pub fn reconcile_json(&mut self) -> Result<Value, LatticedError> {
        let receipt = self.core_status_json()?;
        Ok(json!({
            "component": "delivery-reconciliation",
            "status": "RECONCILIATION_NOT_REQUIRED",
            "delivery_receipt": receipt,
        }))
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
        self.status_task_json_at(binding, deadline(self.timeout)?)
    }

    fn status_task_json_at(
        &mut self,
        binding: &SubjectBinding,
        operation_deadline: Instant,
    ) -> Result<Value, LatticedError> {
        let invocation = invocation_for_task(self.database.run_id(), binding)?;
        self.status_request_json_at(
            &invocation,
            Some(task_ledger_identity(binding)?),
            DeliveryContinuation::WriterOnly,
            operation_deadline,
        )
    }

    fn task_delivery_status(
        &mut self,
        binding: &SubjectBinding,
    ) -> Result<DeliveryStatus, LatticedError> {
        record_observed_effect(ObservedEffectKind::Database)
            .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
        let mut ledger = DeliveryLedger::connect_for_identity(
            &self.database,
            &self.password,
            deadline(self.timeout)?,
            task_ledger_identity(binding)?,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
        ledger
            .status()
            .map_err(|_| LatticedError::new(LatticedErrorKind::ReconciliationRequired))
    }

    fn historical_terminal_status_json(
        &mut self,
        binding: &SubjectBinding,
    ) -> Result<Value, LatticedError> {
        match historical_delivery_status_action(self.task_delivery_status(binding)?) {
            HistoricalDeliveryStatusAction::NotStarted => Ok(core_not_started_status_json()),
            HistoricalDeliveryStatusAction::Failed => {
                self.historical_typed_terminal_status_json(binding, "FAILED")
            }
            HistoricalDeliveryStatusAction::ReconciliationRequired => {
                self.historical_typed_terminal_status_json(binding, "RECONCILIATION_REQUIRED")
            }
            HistoricalDeliveryStatusAction::ReceiptMismatch => {
                Err(LatticedError::new(LatticedErrorKind::ReceiptMismatch))
            }
        }
    }

    fn historical_typed_terminal_status_json(
        &mut self,
        binding: &SubjectBinding,
        expected_status: &'static str,
    ) -> Result<Value, LatticedError> {
        let receipt = self.status_task_json(binding)?;
        if receipt.get("status").and_then(Value::as_str) != Some(expected_status) {
            return Err(LatticedError::new(LatticedErrorKind::ReceiptMismatch));
        }
        let (stage, code) = delivery_failure_projection(&receipt)
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
        Ok(historical_terminal_status_json(
            expected_status,
            &stage,
            &code,
        ))
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
        self.status_request_json_at(
            expected_invocation,
            identity,
            continuation,
            deadline(self.timeout)?,
        )
    }

    fn status_request_json_at(
        &mut self,
        expected_invocation: &Invocation,
        identity: Option<lattice_contracts::TaskLedgerStreamIdentity>,
        continuation: DeliveryContinuation,
        operation_deadline: Instant,
    ) -> Result<Value, LatticedError> {
        record_observed_effect(ObservedEffectKind::Database)
            .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
        let ledger = match identity {
            Some(identity) => DeliveryLedger::connect_for_identity(
                &self.database,
                &self.password,
                operation_deadline,
                identity,
            ),
            None => DeliveryLedger::connect(&self.database, &self.password, operation_deadline),
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
                            operation_deadline,
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

fn historical_terminal_status_json(status: &str, stage: &str, code: &str) -> Value {
    json!({
        "component": "delivery-receipt",
        "failure_code": code,
        "failure_stage": stage,
        "scope": "receipt-only",
        "status": status,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoricalDeliveryStatusAction {
    NotStarted,
    Failed,
    ReconciliationRequired,
    ReceiptMismatch,
}

const fn historical_delivery_status_action(
    delivery_status: DeliveryStatus,
) -> HistoricalDeliveryStatusAction {
    match delivery_status {
        DeliveryStatus::NotStarted => HistoricalDeliveryStatusAction::NotStarted,
        DeliveryStatus::Failed => HistoricalDeliveryStatusAction::Failed,
        DeliveryStatus::ReconciliationRequired => {
            HistoricalDeliveryStatusAction::ReconciliationRequired
        }
        DeliveryStatus::Completed => HistoricalDeliveryStatusAction::ReceiptMismatch,
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

fn delivery_environment_for_mode(
    run_mode: FullChainRunMode,
) -> Result<(LatticedDeliveryConfig, DeliveryDatabaseBinding, String), LatticedError> {
    if run_mode == FullChainRunMode::Fresh {
        let environment = delivery_environment()?;
        if environment.0.runtime == DeliveryRuntime::ScriptedAcceptance {
            validate_managed_scripted_active_restart_admission(
                run_mode,
                &environment.0,
                &environment.1,
            )?;
        }
        return Ok(environment);
    }
    let timeout = match env::var("LATTICE_DELIVERY_TIMEOUT_SECONDS") {
        Ok(value) => parse_timeout(&value)?,
        Err(env::VarError::NotPresent) => Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(LatticedError::new(LatticedErrorKind::Configuration));
        }
    };
    let runtime = required_environment("LATTICE_DELIVERY_CODEX_MODE")?;
    if runtime == "SCRIPTED_ACCEPTANCE" {
        let environment = delivery_environment()?;
        validate_managed_scripted_active_restart_admission(
            run_mode,
            &environment.0,
            &environment.1,
        )?;
        return Ok(environment);
    }
    if runtime != "OFFICIAL_CODEX_APP_SERVER" {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PostgresBootstrapAction {
    V5Apply,
    V6Rebind,
    V4Apply,
    V7Apply,
    WriterV5Apply,
    WriterV8Rebind,
    V8Apply,
    V8VerifyOnly,
}

const fn postgres_bootstrap_action(
    store: MigrationBootstrapProfile,
    memory: MemoryBootstrapProfile,
    writer: V3BootstrapProfile,
) -> Option<PostgresBootstrapAction> {
    match (store, memory, writer) {
        (
            MigrationBootstrapProfile::V5,
            MemoryBootstrapProfile::Empty | MemoryBootstrapProfile::V2 | MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V5FallbackRequired,
        )
        | (
            MigrationBootstrapProfile::V5,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V5Bridge,
        ) => Some(PostgresBootstrapAction::V5Apply),
        (
            MigrationBootstrapProfile::V6,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V6BridgePending,
        ) => Some(PostgresBootstrapAction::V6Rebind),
        (
            MigrationBootstrapProfile::V6,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V6Current,
        ) => Some(PostgresBootstrapAction::V4Apply),
        (
            MigrationBootstrapProfile::V6,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V6V4BridgeLegacyF252Rebind,
        ) => Some(PostgresBootstrapAction::V4Apply),
        (
            MigrationBootstrapProfile::V6,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V6V4Bridge,
        ) => Some(PostgresBootstrapAction::V7Apply),
        (
            MigrationBootstrapProfile::V7,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V7V4Current,
        ) => Some(PostgresBootstrapAction::WriterV5Apply),
        (
            MigrationBootstrapProfile::V7,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V7V5RebindPending,
        ) => Some(PostgresBootstrapAction::WriterV8Rebind),
        (
            MigrationBootstrapProfile::V7,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V7V5Current,
        ) => Some(PostgresBootstrapAction::V8Apply),
        (
            MigrationBootstrapProfile::V8LegacyPrefix,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V8V5RebindPending,
        )
        | (
            MigrationBootstrapProfile::V8,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V8V5RebindPending,
        ) => Some(PostgresBootstrapAction::WriterV8Rebind),
        (
            MigrationBootstrapProfile::V8LegacyPrefix,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V8V5Current,
        ) => Some(PostgresBootstrapAction::V8Apply),
        (
            MigrationBootstrapProfile::V8,
            MemoryBootstrapProfile::V3,
            V3BootstrapProfile::V8V5Current,
        ) => Some(PostgresBootstrapAction::V8VerifyOnly),
        _ => None,
    }
}

fn acquire_postgres_bootstrap_gate(client: &mut Client) -> Result<(), LatticedError> {
    let started_at = Instant::now();
    loop {
        let acquired = client
            .query_one(
                "SELECT pg_catalog.pg_try_advisory_lock($1::bigint)",
                &[&POSTGRES_BOOTSTRAP_GLOBAL_ADVISORY_LOCK],
            )
            .and_then(|row| row.try_get::<_, bool>(0))
            .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresMigration))?;
        if acquired {
            return Ok(());
        }
        let elapsed = started_at.elapsed();
        if elapsed >= POSTGRES_BOOTSTRAP_GATE_TIMEOUT {
            return Err(LatticedError::new(
                LatticedErrorKind::RuntimePostgresMigration,
            ));
        }
        std::thread::sleep(std::cmp::min(
            POSTGRES_BOOTSTRAP_GATE_POLL_INTERVAL,
            POSTGRES_BOOTSTRAP_GATE_TIMEOUT.saturating_sub(elapsed),
        ));
    }
}

fn release_postgres_bootstrap_gate(client: &mut Client) -> Result<(), LatticedError> {
    let released = client
        .query_one(
            "SELECT pg_catalog.pg_advisory_unlock($1::bigint)",
            &[&POSTGRES_BOOTSTRAP_GLOBAL_ADVISORY_LOCK],
        )
        .and_then(|row| row.try_get::<_, bool>(0))
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresMigration))?;
    if !released {
        return Err(LatticedError::new(
            LatticedErrorKind::RuntimePostgresMigration,
        ));
    }
    Ok(())
}

fn verify_postgres_runtime_gates(
    database: &DeliveryDatabaseBinding,
    password: &str,
    store_target: &StoreMigrationTarget,
    authority: &StoreAuthorityHead,
    foreman_target: &ForemanExtensionTarget,
) -> Result<(), LatticedError> {
    let runtime = connect_fixed_runtime_client(
        database,
        password,
        deadline(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))?,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    let ledger = PostgresTaskLedger::new(runtime, store_target)
        .map_err(|_| LatticedError::new(LatticedErrorKind::ForemanReplayCorrupt))?;
    let mut coordination = PostgresForemanCoordination::new(ledger, authority.clone());
    coordination
        .load_runtime_status()
        .map_err(|error| foreman_replay_latticed(ToolExecutionError::new(error.code())))?;
    let mut foreman_runtime = connect_fixed_runtime_client(
        database,
        password,
        deadline(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))?,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    verify_postgres_foreman_extension(
        &mut foreman_runtime,
        foreman_target,
        ForemanExtensionDatabaseRole::Runtime,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresVerification))?;
    Ok(())
}

/// Performs the only product migration path: Store v5 foundation, Memory v3,
/// Writer v2/v3 bridge, Store v6/rebind, Store-v7 submission profile, then
/// the exact Store-v8 external-adoption profile and fresh-runtime replay proof.
///
/// The command temporarily closes Runtime admission while it holds the extension
/// migration locks, then restores the configured Runtime authority after every
/// migrator-owned gate passes. Pre-mutation rejection preserves admission;
/// once mutation closes admission, any later owner failure leaves exact
/// STOPPED/no-leader for retry. Fresh Runtime-role replay must also pass before
/// readiness is reported.
///
/// # Errors
///
/// Returns a closed configuration, migration, verification, or replay failure.
pub fn bootstrap_postgres_extensions_from_environment() -> Result<(), LatticedError> {
    let (_config, database, password) =
        delivery_environment_for_mode(FullChainRunMode::ResumeExisting)?;
    let authority = configured_store_authority()?;
    let configured_admission = RuntimeAdmissionSnapshot::from_authority(&authority)?;
    let store_target = StoreMigrationTarget::new(database.database_name(), database.run_id())
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresMigration))?;
    let mut migrator = connect_migrator(&database, &password)?;
    acquire_postgres_bootstrap_gate(&mut migrator)?;
    let database_identity =
        ContentDigest::from_sha256(store_target.expected_database_identity_sha256().as_str())
            .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresVerification))?;
    let writer_v3 = V3ExtensionTarget::new(database.database_name(), database_identity.clone())
        .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
    let writer_v4 = V4ExtensionTarget::new(database.database_name(), database_identity.clone())
        .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
    let writer_v5 = V5ExtensionTarget::new(database.database_name(), database_identity.clone())
        .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;

    // Classify only exact history before mutation. Product bootstrap does not
    // normalize historical prefixes. Fresh setup first proves the Writer
    // namespace absent before creating its stopped v5 foundation.
    let mut profile = inspect_migration_profile(&mut migrator, &store_target)
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresVerification))?;
    if profile == MigrationBootstrapProfile::LegacyPrefix {
        return Err(LatticedError::new(
            LatticedErrorKind::RuntimePostgresVerification,
        ));
    }
    if profile == MigrationBootstrapProfile::Fresh {
        if inspect_v3_bootstrap_profile(&mut migrator, &writer_v3)
            .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?
            != V3BootstrapProfile::V5FallbackRequired
        {
            return Err(LatticedError::new(LatticedErrorKind::WriterLease));
        }
        apply_store_migrations(&mut migrator, &store_target)
            .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresMigration))?;
        profile = MigrationBootstrapProfile::V5;
    }
    let memory_target = ExtensionTarget::new(database.database_name(), database.run_id())
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let memory_global = match profile {
        MigrationBootstrapProfile::V5 => MemoryBootstrapGlobalProfile::V5,
        MigrationBootstrapProfile::V6 => MemoryBootstrapGlobalProfile::V6,
        MigrationBootstrapProfile::V7 => MemoryBootstrapGlobalProfile::V7,
        MigrationBootstrapProfile::V8LegacyPrefix => MemoryBootstrapGlobalProfile::V8LegacyPrefix,
        MigrationBootstrapProfile::V8 => MemoryBootstrapGlobalProfile::V8,
        MigrationBootstrapProfile::Fresh | MigrationBootstrapProfile::LegacyPrefix => {
            return Err(LatticedError::new(
                LatticedErrorKind::RuntimePostgresVerification,
            ));
        }
    };
    let Ok(memory_profile) =
        inspect_bootstrap_profile(&mut migrator, &memory_target, memory_global)
    else {
        return Err(LatticedError::new(LatticedErrorKind::GraphConfiguration));
    };
    let Ok(writer_profile) = inspect_v3_bootstrap_profile(&mut migrator, &writer_v3) else {
        return Err(LatticedError::new(LatticedErrorKind::WriterLease));
    };
    let Some(action) = postgres_bootstrap_action(profile, memory_profile, writer_profile) else {
        return Err(LatticedError::new(LatticedErrorKind::WriterLease));
    };
    let foreman_target =
        ForemanExtensionTarget::new(database.database_name(), database.run_id())
            .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresVerification))?;
    let terminal_current = if action == PostgresBootstrapAction::V8VerifyOnly {
        verify_postgres_foreman_extension(
            &mut migrator,
            &foreman_target,
            ForemanExtensionDatabaseRole::Migrator,
        )
        .is_ok()
    } else {
        false
    };

    let admission = RuntimeAdmissionSnapshot::load(&mut migrator)?;
    if admission != configured_admission && !admission.is_stopped_no_leader() {
        return Err(LatticedError::new(
            LatticedErrorKind::RuntimePostgresVerification,
        ));
    }
    if terminal_current && admission == configured_admission {
        release_postgres_bootstrap_gate(&mut migrator)?;
        drop(migrator);
        return verify_postgres_runtime_gates(
            &database,
            &password,
            &store_target,
            &authority,
            &foreman_target,
        );
    }
    if !admission.is_stopped_no_leader() {
        admission.stop(&mut migrator)?;
    }
    if !RuntimeAdmissionSnapshot::load(&mut migrator)?.is_stopped_no_leader() {
        return Err(LatticedError::new(
            LatticedErrorKind::RuntimePostgresVerification,
        ));
    }

    if action != PostgresBootstrapAction::V8VerifyOnly {
        let setup = (|| {
            let mut next_action = action;
            let mut next_writer_profile = writer_profile;
            for _ in 0..6 {
                match next_action {
                    PostgresBootstrapAction::V5Apply => {
                        match next_writer_profile {
                            V3BootstrapProfile::V5Bridge => {
                                if apply_v3_extension(&mut migrator, &writer_v3).map_err(|_| {
                                    LatticedError::new(LatticedErrorKind::WriterLease)
                                })? != WriterExtensionApplyOutcome::Bridged
                                {
                                    return Err(LatticedError::new(LatticedErrorKind::WriterLease));
                                }
                            }
                            V3BootstrapProfile::V5FallbackRequired => {
                                let store = verify_store_schema(
                                    &mut migrator,
                                    &store_target,
                                    StoreDatabaseRole::Migrator,
                                )
                                .map_err(|_| {
                                    LatticedError::new(
                                        LatticedErrorKind::RuntimePostgresVerification,
                                    )
                                })?;
                                if store.schema_version() != 5 {
                                    return Err(LatticedError::new(
                                        LatticedErrorKind::RuntimePostgresVerification,
                                    ));
                                }
                                let memory_manifest = verify_embedded_extension_manifest()
                                    .map_err(|_| {
                                        LatticedError::new(LatticedErrorKind::GraphConfiguration)
                                    })?;
                                apply_postgres_memory_extension(&mut migrator, &memory_target)
                                    .map_err(|_| {
                                        LatticedError::new(LatticedErrorKind::GraphConfiguration)
                                    })?;
                                let global_manifest =
                                    ContentDigest::from_sha256(store.manifest_sha256().as_str())
                                        .map_err(|_| {
                                            LatticedError::new(
                                                LatticedErrorKind::RuntimePostgresVerification,
                                            )
                                        })?;
                                let writer_target = WriterLeaseExtensionTarget::new(
                                    database.database_name(),
                                    database_identity.clone(),
                                    global_manifest,
                                    memory_manifest.manifest_sha256().clone(),
                                )
                                .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
                                apply_postgres_writer_extension(&mut migrator, &writer_target)
                                    .map_err(|_| {
                                        LatticedError::new(LatticedErrorKind::WriterLease)
                                    })?;
                                verify_writer_extension(&mut migrator, &writer_target).map_err(
                                    |_| LatticedError::new(LatticedErrorKind::WriterLease),
                                )?;
                                verify_memory_extension(
                                    &mut migrator,
                                    &memory_target,
                                    lattice_postgres_codebase_memory::ExtensionDatabaseRole::Migrator,
                                )
                                .map_err(|_| {
                                    LatticedError::new(LatticedErrorKind::GraphConfiguration)
                                })?;
                                if apply_v3_extension(&mut migrator, &writer_v3).map_err(|_| {
                                    LatticedError::new(LatticedErrorKind::WriterLease)
                                })? != WriterExtensionApplyOutcome::Bridged
                                {
                                    return Err(LatticedError::new(LatticedErrorKind::WriterLease));
                                }
                            }
                            V3BootstrapProfile::V6BridgePending
                            | V3BootstrapProfile::V6Current
                            | V3BootstrapProfile::V6V4Bridge
                            | V3BootstrapProfile::V6V4BridgeLegacyF252Rebind
                            | V3BootstrapProfile::V7V4Current
                            | V3BootstrapProfile::V7V5RebindPending
                            | V3BootstrapProfile::V7V5Current
                            | V3BootstrapProfile::V8V5RebindPending
                            | V3BootstrapProfile::V8V5Current => {
                                return Err(LatticedError::new(LatticedErrorKind::WriterLease));
                            }
                        }
                        apply_store_migrations(&mut migrator, &store_target).map_err(|_| {
                            LatticedError::new(LatticedErrorKind::RuntimePostgresMigration)
                        })?;
                    }
                    PostgresBootstrapAction::V6Rebind => {
                        match rebind_existing_v3_extension(&mut migrator, &writer_v3)
                            .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?
                        {
                            WriterExtensionApplyOutcome::Rebound => {}
                            _ => return Err(LatticedError::new(LatticedErrorKind::WriterLease)),
                        }
                    }
                    PostgresBootstrapAction::V4Apply => {
                        if apply_v4_extension(&mut migrator, &writer_v4)
                            .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?
                            != WriterExtensionApplyOutcome::Bridged
                        {
                            return Err(LatticedError::new(LatticedErrorKind::WriterLease));
                        }
                    }
                    PostgresBootstrapAction::V7Apply => {
                        apply_store_migrations(&mut migrator, &store_target).map_err(|_| {
                            LatticedError::new(LatticedErrorKind::RuntimePostgresMigration)
                        })?;
                    }
                    PostgresBootstrapAction::WriterV5Apply => {
                        if apply_v5_extension(&mut migrator, &writer_v5)
                            .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?
                            != WriterExtensionApplyOutcome::Activated
                        {
                            return Err(LatticedError::new(LatticedErrorKind::WriterLease));
                        }
                    }
                    PostgresBootstrapAction::WriterV8Rebind => {
                        match rebind_v5_for_store_v8(&mut migrator, &writer_v5)
                            .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?
                        {
                            WriterExtensionApplyOutcome::Rebound
                            | WriterExtensionApplyOutcome::AlreadyCurrent => {}
                            _ => return Err(LatticedError::new(LatticedErrorKind::WriterLease)),
                        }
                    }
                    PostgresBootstrapAction::V8Apply => {
                        apply_store_migrations(&mut migrator, &store_target).map_err(|_| {
                            LatticedError::new(LatticedErrorKind::RuntimePostgresExternalAdoption)
                        })?;
                    }
                    PostgresBootstrapAction::V8VerifyOnly => {}
                }

                profile =
                    inspect_migration_profile(&mut migrator, &store_target).map_err(|_| {
                        LatticedError::new(LatticedErrorKind::RuntimePostgresVerification)
                    })?;
                let memory_global = match profile {
                    MigrationBootstrapProfile::V5 => MemoryBootstrapGlobalProfile::V5,
                    MigrationBootstrapProfile::V6 => MemoryBootstrapGlobalProfile::V6,
                    MigrationBootstrapProfile::V7 => MemoryBootstrapGlobalProfile::V7,
                    MigrationBootstrapProfile::V8LegacyPrefix => {
                        MemoryBootstrapGlobalProfile::V8LegacyPrefix
                    }
                    MigrationBootstrapProfile::V8 => MemoryBootstrapGlobalProfile::V8,
                    MigrationBootstrapProfile::Fresh | MigrationBootstrapProfile::LegacyPrefix => {
                        return Err(LatticedError::new(
                            LatticedErrorKind::RuntimePostgresVerification,
                        ));
                    }
                };
                let memory_profile =
                    inspect_bootstrap_profile(&mut migrator, &memory_target, memory_global)
                        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
                next_writer_profile = inspect_v3_bootstrap_profile(&mut migrator, &writer_v3)
                    .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
                next_action =
                    postgres_bootstrap_action(profile, memory_profile, next_writer_profile)
                        .ok_or_else(|| LatticedError::new(LatticedErrorKind::WriterLease))?;
                if next_action == PostgresBootstrapAction::V8VerifyOnly {
                    let final_store = verify_store_schema(
                        &mut migrator,
                        &store_target,
                        StoreDatabaseRole::Migrator,
                    )
                    .map_err(|_| {
                        LatticedError::new(LatticedErrorKind::RuntimePostgresVerification)
                    })?;
                    if final_store.schema_version() != 8 {
                        return Err(LatticedError::new(
                            LatticedErrorKind::RuntimePostgresVerification,
                        ));
                    }
                    return Ok(());
                }
            }
            Err(LatticedError::new(
                LatticedErrorKind::RuntimePostgresVerification,
            ))
        })();
        if let Err(error) = setup {
            return Err(error);
        }
    }

    #[cfg(test)]
    if tests::foreman_catalog_measurement_requested() {
        return tests::measure_foreman_catalog_profile(
            &mut migrator,
            &database,
            &configured_admission,
        );
    }

    // The managed foreman is a same-database extension of the exact Store-v8
    // successor. Explicit bootstrap is its only installation/rebind path;
    // ordinary Runtime startup remains verify-only.
    match apply_postgres_foreman_extension(&mut migrator, &foreman_target) {
        Ok(
            ForemanExtensionApplyOutcome::Installed(_)
            | ForemanExtensionApplyOutcome::Upgraded(_)
            | ForemanExtensionApplyOutcome::Rebound(_)
            | ForemanExtensionApplyOutcome::AlreadyCurrent(_),
        ) => {}
        Err(_) => {
            return Err(LatticedError::new(
                LatticedErrorKind::RuntimePostgresForeman,
            ));
        }
    }
    lattice_postgres_store::apply_control_product_extension(&mut migrator, &store_target)
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresMigration))?;
    let final_store =
        verify_store_schema(&mut migrator, &store_target, StoreDatabaseRole::Migrator)
            .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresVerification))?;
    if final_store.schema_version() != 8
        || !RuntimeAdmissionSnapshot::load(&mut migrator)?.is_stopped_no_leader()
    {
        return Err(LatticedError::new(
            LatticedErrorKind::RuntimePostgresVerification,
        ));
    }

    if !RuntimeAdmissionSnapshot::load(&mut migrator)?.is_stopped_no_leader() {
        return Err(LatticedError::new(
            LatticedErrorKind::RuntimePostgresVerification,
        ));
    }
    if configured_admission.restore(&mut migrator).is_err() {
        return Err(LatticedError::new(LatticedErrorKind::LedgerConfiguration));
    }
    let observed_admission = RuntimeAdmissionSnapshot::load(&mut migrator)?;
    if observed_admission != configured_admission {
        return Err(LatticedError::new(
            LatticedErrorKind::RuntimePostgresVerification,
        ));
    }
    if release_postgres_bootstrap_gate(&mut migrator).is_err() {
        return Err(LatticedError::new(
            LatticedErrorKind::RuntimePostgresMigration,
        ));
    }
    drop(migrator);
    verify_postgres_runtime_gates(
        &database,
        &password,
        &store_target,
        &authority,
        &foreman_target,
    )
}

/// Initializes the LATTICE-owned local `PostgreSQL` database before normal Runtime
/// startup. This is deliberately separate from the historical disposable
/// acceptance harness: callers must already have started an isolated loopback
/// cluster with the fixed `lattice_bootstrap` superuser.
///
/// # Errors
///
/// Returns a stable configuration, connection, or ledger error. The database is
/// left untouched when its existing role or schema boundary cannot be verified.
pub fn initialize_runtime_postgres_from_environment() -> Result<(), LatticedError> {
    let (_config, database, password) =
        delivery_environment_for_mode(FullChainRunMode::ResumeExisting)?;
    let database_name = database.database_name();
    let target = StoreMigrationTarget::new(database_name.clone(), database.run_id())
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresProvision))?;

    let host = required_environment("LATTICE_TASK019_HOST")?;
    let port = required_environment("LATTICE_TASK019_PORT")?
        .parse::<u16>()
        .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
    let mut bootstrap = Config::new();
    bootstrap
        .host(&host)
        .port(port)
        .user("runtime_bootstrap")
        .password(&password)
        .dbname("postgres")
        .application_name("lattice-runtime-bootstrap")
        .ssl_mode(SslMode::Disable);
    let mut bootstrap = bootstrap
        .connect(NoTls)
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    let quoted_password = bootstrap
        .query_one("SELECT quote_literal($1::text)", &[&password])
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresProvision))?
        .get::<_, String>(0);
    bootstrap
        .batch_execute(&format!(
            "DO $$ BEGIN \
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lattice_migrator') THEN \
                   CREATE ROLE lattice_migrator NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; \
                 END IF; \
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lattice_runtime') THEN \
                   CREATE ROLE lattice_runtime NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; \
                 END IF; \
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lattice_guardian') THEN \
                   CREATE ROLE lattice_guardian NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; \
                 END IF; \
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lattice_readonly') THEN \
                   CREATE ROLE lattice_readonly NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS; \
                 END IF; \
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lattice_migrator_login') THEN \
                   CREATE ROLE lattice_migrator_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD {quoted_password}; \
                 END IF; \
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lattice_runtime_login') THEN \
                   CREATE ROLE lattice_runtime_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD {quoted_password}; \
                 END IF; \
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lattice_guardian_login') THEN \
                   CREATE ROLE lattice_guardian_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD {quoted_password}; \
                 END IF; \
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lattice_readonly_login') THEN \
                   CREATE ROLE lattice_readonly_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD {quoted_password}; \
                 END IF; \
               END $$; \
             GRANT lattice_migrator TO lattice_migrator_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_runtime TO lattice_runtime_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_guardian TO lattice_guardian_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_readonly TO lattice_readonly_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE;"
        ))
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresProvision))?;
    let database_exists = bootstrap
        .query_opt(
            "SELECT 1 FROM pg_database WHERE datname = $1",
            &[&database_name],
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresProvision))?
        .is_some();
    if !database_exists {
        bootstrap
            .batch_execute(&format!(
                "CREATE DATABASE {database_name} OWNER lattice_migrator"
            ))
            .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresProvision))?;
    }
    bootstrap
        .batch_execute(&format!(
            "REVOKE ALL ON DATABASE {database_name} FROM PUBLIC; \
             GRANT CONNECT ON DATABASE {database_name} TO lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, lattice_guardian_login, lattice_readonly_login; \
             SET ROLE lattice_migrator; COMMENT ON DATABASE {database_name} IS '{}'; RESET ROLE; \
             REVOKE ALL ON DATABASE postgres FROM PUBLIC; \
             REVOKE ALL ON DATABASE template0 FROM PUBLIC; \
             REVOKE ALL ON DATABASE template1 FROM PUBLIC;",
            target.database_comment()
        ))
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresBoundary))?;
    drop(bootstrap);
    let mut target_bootstrap = Config::new();
    target_bootstrap
        .host(&host)
        .port(port)
        .user("runtime_bootstrap")
        .password(&password)
        .dbname(&database_name)
        .application_name("lattice-runtime-bootstrap")
        .ssl_mode(SslMode::Disable);
    let mut target_bootstrap = target_bootstrap
        .connect(NoTls)
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    target_bootstrap
        .batch_execute(
            "REVOKE ALL PRIVILEGES ON FUNCTION \
                 pg_catalog.lo_creat(integer), pg_catalog.lo_create(oid), \
                 pg_catalog.lo_from_bytea(oid, bytea), pg_catalog.lo_import(text), \
                 pg_catalog.lo_import(text, oid), \
                 pg_catalog.pg_logical_emit_message(boolean, text, text, boolean), \
                 pg_catalog.pg_logical_emit_message(boolean, text, bytea, boolean), \
                 pg_catalog.pg_advisory_lock(bigint), \
                 pg_catalog.pg_advisory_lock(integer, integer), \
                 pg_catalog.pg_advisory_lock_shared(bigint), \
                 pg_catalog.pg_advisory_lock_shared(integer, integer), \
                 pg_catalog.pg_try_advisory_lock(bigint), \
                 pg_catalog.pg_try_advisory_lock(integer, integer), \
                 pg_catalog.pg_try_advisory_lock_shared(bigint), \
                 pg_catalog.pg_try_advisory_lock_shared(integer, integer), \
                 pg_catalog.pg_advisory_xact_lock(bigint), \
                 pg_catalog.pg_advisory_xact_lock(integer, integer), \
                 pg_catalog.pg_advisory_xact_lock_shared(bigint), \
                 pg_catalog.pg_advisory_xact_lock_shared(integer, integer), \
                 pg_catalog.pg_try_advisory_xact_lock(bigint), \
                 pg_catalog.pg_try_advisory_xact_lock(integer, integer), \
                 pg_catalog.pg_try_advisory_xact_lock_shared(bigint), \
                 pg_catalog.pg_try_advisory_xact_lock_shared(integer, integer), \
                 pg_catalog.pg_cancel_backend(integer), \
                 pg_catalog.pg_terminate_backend(integer, bigint), \
                 pg_catalog.pg_export_snapshot(), pg_catalog.pg_current_xact_id(), \
                 pg_catalog.txid_current() \
             FROM PUBLIC, lattice_migrator, lattice_runtime, lattice_guardian, \
                 lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             GRANT EXECUTE ON FUNCTION pg_catalog.pg_try_advisory_lock(bigint), \
                 pg_catalog.pg_advisory_xact_lock(bigint), \
                 pg_catalog.pg_current_xact_id() TO lattice_migrator",
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::RuntimePostgresBoundary))?;
    drop(target_bootstrap);

    // Provisioning ends here. The explicit --postgres-bootstrap command is
    // the only path allowed to install or migrate Store/Memory/Writer schemas.
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

/// Imports externally issued delivery evidence through the explicit maintenance
/// CLI. The MCP service never receives raw receipts or a migrator connection.
///
/// # Errors
/// Returns a bounded failure without exposing paths, receipt bytes or credentials.
pub fn import_external_result_from_environment(path: &Path) -> Result<Value, &'static str> {
    let request = crate::external_result_import::parse(path)?;
    let (_config, database, password) =
        delivery_environment_for_mode(FullChainRunMode::ResumeExisting)
            .map_err(|error| error.code())?;
    let git = PathBuf::from(
        required_environment("LATTICE_DELIVERY_GIT_EXE").map_err(|error| error.code())?,
    );
    let mut migrator = connect_migrator(&database, &password).map_err(|error| error.code())?;
    acquire_postgres_bootstrap_gate(&mut migrator).map_err(|error| error.code())?;
    let result = (|| {
        let target = StoreMigrationTarget::new(database.database_name(), database.run_id())
            .map_err(|_| "LATTICE_EXTERNAL_RESULT_IMPORT_DATABASE_UNAVAILABLE")?;
        let until =
            deadline(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS)).map_err(|error| error.code())?;
        let runtime = connect_fixed_runtime_client(&database, &password, until)
            .map_err(|_| "LATTICE_EXTERNAL_RESULT_IMPORT_DATABASE_UNAVAILABLE")?;
        let mut ledger = PostgresTaskLedger::new(runtime, &target).map_err(|error| error.code())?;
        let retained = ledger
            .load_submission_by_task_ref(request.adoption.task_ref())
            .map_err(|error| error.code())?
            .ok_or("LATTICE_TASK_REFERENCE_NOT_FOUND")?;
        let stream = retained.ledger().stream();
        let exact_replay = stream.commands().iter().any(|record| {
            let command = record.request();
            command.command_id().as_str() == request.adoption.command_id()
                && command.subject_digest() == request.adoption.result_digest()
                && command.expected_head().head_digest()
                    == request.adoption.expected_ledger_head_digest()
        });
        if stream.head().head_digest() != request.adoption.expected_ledger_head_digest()
            && !exact_replay
        {
            return Err("LATTICE_EXTERNAL_RESULT_EVIDENCE_MISMATCH");
        }
        let runtime = connect_fixed_runtime_client(&database, &password, until)
            .map_err(|_| "LATTICE_EXTERNAL_RESULT_IMPORT_DATABASE_UNAVAILABLE")?;
        let mut registry =
            PostgresProjectRegistry::new(runtime, &target).map_err(|error| error.code())?;
        let registered = registry.load().map_err(|error| error.code())?;
        let project = registered
            .state()
            .project(retained.submission().identity().project_id())
            .ok_or("PROJECT_IS_NOT_REGISTERED")?;
        if project.authority().lifecycle() != ProjectLifecycle::Active {
            return Err("PROJECT_IS_NOT_ACTIVE");
        }
        // Historical intake keeps its original snapshot. Only the registered
        // repository locator is read here; observing delivery does not refresh it.
        let repository = Path::new(project.observation().canonical_root());
        let verified = request.verify(retained.submission(), repository, &git)?;
        crate::external_result_import::retain(&mut migrator, &verified)
    })();
    let released = release_postgres_bootstrap_gate(&mut migrator).map_err(|error| error.code());
    match (result, released) {
        (Ok(receipt), Ok(())) => Ok(receipt),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

/// Trusted local maintenance executes the fixed verifier and adopts through Runtime.
pub fn import_local_result_from_environment(path: &Path) -> Result<Value, &'static str> {
    let request = crate::local_result_import::parse(path)?;
    let (_config, database, password) =
        delivery_environment_for_mode(FullChainRunMode::ResumeExisting).map_err(|e| e.code())?;
    let git =
        PathBuf::from(required_environment("LATTICE_DELIVERY_GIT_EXE").map_err(|e| e.code())?);
    let node =
        PathBuf::from(required_environment("LATTICE_LOCAL_RESULT_NODE_EXE").map_err(|e| e.code())?);
    let mut migrator = connect_migrator(&database, &password).map_err(|e| e.code())?;
    acquire_postgres_bootstrap_gate(&mut migrator).map_err(|e| e.code())?;
    let result = (|| {
        let target = StoreMigrationTarget::new(database.database_name(), database.run_id())
            .map_err(|_| "LATTICE_LOCAL_RESULT_DATABASE_UNAVAILABLE")?;
        let until = deadline(Duration::from_secs(180)).map_err(|e| e.code())?;
        let runtime = connect_fixed_runtime_client(&database, &password, until)
            .map_err(|_| "LATTICE_LOCAL_RESULT_DATABASE_UNAVAILABLE")?;
        let mut ledger = PostgresTaskLedger::new(runtime, &target).map_err(|e| e.code())?;
        let retained = ledger
            .load_submission_by_task_ref(request.adoption.task_ref())
            .map_err(|e| e.code())?
            .ok_or("LATTICE_TASK_REFERENCE_NOT_FOUND")?;
        let stream = retained.ledger().stream();
        let replay = stream.commands().iter().any(|r| {
            r.request().command_id().as_str() == request.adoption.command_id()
                && r.request().subject_digest() == request.adoption.result_digest()
                && r.request().expected_head().head_digest()
                    == request.adoption.expected_ledger_head_digest()
        });
        if stream.head().head_digest() != request.adoption.expected_ledger_head_digest() && !replay
        {
            return Err("LATTICE_LOCAL_RESULT_EVIDENCE_MISMATCH");
        }
        let runtime = connect_fixed_runtime_client(&database, &password, until)
            .map_err(|_| "LATTICE_LOCAL_RESULT_DATABASE_UNAVAILABLE")?;
        let mut registry = PostgresProjectRegistry::new(runtime, &target).map_err(|e| e.code())?;
        let registered = registry.load().map_err(|e| e.code())?;
        let project = registered
            .state()
            .project(retained.submission().identity().project_id())
            .ok_or("PROJECT_IS_NOT_REGISTERED")?;
        if project.authority().lifecycle() != ProjectLifecycle::Active {
            return Err("PROJECT_IS_NOT_ACTIVE");
        }
        let import = request.verify_and_retain(
            retained.submission(),
            Path::new(project.observation().canonical_root()),
            &git,
            &node,
            &mut migrator,
        )?;
        let authority = configured_store_authority().map_err(|e| e.code())?;
        let peer =
            configured_task_ingress_peer(&daemon_process_start_identity().map_err(|e| e.code())?)
                .map_err(|e| e.code())?;
        let mut lifecycle = PostgresTaskLifecycle::connect_with_ingress_peer_and_admission_profile(
            &database,
            &password,
            until,
            retained.submission().identity().clone(),
            authority,
            peer,
            TaskAdmissionProfile::GeneralTaskIntake(Box::new(retained.submission().clone())),
        )
        .map_err(|e| e.code())?;
        let now = time::OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .map_err(|_| "LATTICE_LOCAL_RESULT_IMPORT_REJECTED")?
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|_| "LATTICE_LOCAL_RESULT_IMPORT_REJECTED")?;
        let evidence = lifecycle
            .adopt_local_result(&request.adoption, &now)
            .map_err(|e| e.code())?;
        let mut status =
            general_task_public_status(&evidence, retained.submission()).map_err(|e| e.code())?;
        status
            .as_object_mut()
            .ok_or("LATTICE_LOCAL_RESULT_IMPORT_REJECTED")?
            .insert("verification".to_owned(), import);
        Ok(status)
    })();
    let released = release_postgres_bootstrap_gate(&mut migrator).map_err(|e| e.code());
    match (result, released) {
        (Ok(receipt), Ok(())) => Ok(receipt),
        (Err(e), _) | (_, Err(e)) => Err(e),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
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
                 FROM ONLY control.runtime_admission WHERE singleton",
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
        let affected = client
            .execute(
                "UPDATE ONLY control.runtime_admission SET admission_mode = 'STOPPED', \
                 daemon_instance_id = NULL, daemon_epoch = NULL, authority_revision = 0, \
                 observation_digest = NULL, authority_head_digest = NULL, \
                 updated_at = pg_catalog.clock_timestamp() \
                 WHERE singleton AND admission_mode = $1 \
                   AND daemon_instance_id IS NOT DISTINCT FROM $2 \
                   AND daemon_epoch IS NOT DISTINCT FROM $3 \
                   AND authority_revision = $4 \
                   AND observation_digest IS NOT DISTINCT FROM $5 \
                   AND authority_head_digest IS NOT DISTINCT FROM $6",
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
        if affected != 1 {
            return Err(LatticedError::new(LatticedErrorKind::LedgerConfiguration));
        }
        Ok(())
    }

    fn restore(&self, client: &mut Client) -> Result<(), LatticedError> {
        let affected = client
            .execute(
                "UPDATE ONLY control.runtime_admission SET admission_mode = $1, \
                 daemon_instance_id = $2, daemon_epoch = $3, authority_revision = $4, \
                 observation_digest = $5, authority_head_digest = $6, \
                 updated_at = pg_catalog.clock_timestamp() \
                 WHERE singleton AND admission_mode = 'STOPPED' \
                   AND daemon_instance_id IS NULL AND daemon_epoch IS NULL \
                   AND authority_revision = 0 AND observation_digest IS NULL \
                   AND authority_head_digest IS NULL",
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
        if affected != 1 {
            return Err(LatticedError::new(LatticedErrorKind::LedgerConfiguration));
        }
        Ok(())
    }

    fn is_stopped_no_leader(&self) -> bool {
        self.mode == "STOPPED"
            && self.daemon_instance_id.is_none()
            && self.daemon_epoch.is_none()
            && self.authority_revision == 0
            && self.observation_digest.is_none()
            && self.authority_head_digest.is_none()
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
    wsl_executable: PathBuf,
    runtime_guest_root: String,
    isolation_parent: PathBuf,
    product_root: PathBuf,
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
        // This is only a fresh loopback credential between LATTICE and the
        // contained Hermes process. It is not an OpenAI or user-provided API
        // key: model access is owned by the verified Codex app-server broker.
        let api_key = new_hermes_session_token()?;
        let wsl_executable = PathBuf::from(hermes_environment("LATTICE_HERMES_WSL_EXE")?);
        let isolation_parent =
            PathBuf::from(hermes_environment("LATTICE_HERMES_ISOLATION_PARENT")?);
        let preflight_token = new_hermes_session_token()?;
        HermesWslContainmentConfig::new(
            wsl_executable.clone(),
            runtime_guest_root.clone(),
            isolation_parent.join(format!("preflight-{}", &preflight_token[..32])),
            product_root.clone(),
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let broker = CodexReflectionBrokerConfig::new(
            PathBuf::from(hermes_environment("LATTICE_HERMES_CODEX_LAUNCHER")?),
            PathBuf::from(hermes_environment("LATTICE_HERMES_CODEX_HOME")?),
            PathBuf::from(hermes_environment("LATTICE_HERMES_BROKER_ISOLATION_ROOT")?),
            product_root.clone(),
            FULL_CHAIN_CODEX_BROKER_MODEL,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let timeout_seconds = hermes_environment("LATTICE_HERMES_DEADLINE_SECONDS")?
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=300).contains(seconds))
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        Ok(Self {
            wsl_executable,
            runtime_guest_root,
            isolation_parent,
            product_root,
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
        let attempt_token = new_hermes_session_token()?;
        let containment = HermesWslContainmentConfig::new(
            self.wsl_executable,
            self.runtime_guest_root,
            self.isolation_parent
                .join(format!("run-{}", &attempt_token[..32])),
            self.product_root,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let runner = HermesProductionRunnerConfig::new(
            containment,
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
fn new_hermes_session_token() -> Result<String, LatticedError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}")
            .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
    }
    Ok(token)
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

/// Fixed, redacted result of verifying Hermes' local Codex broker boundary.
///
/// This check never starts Hermes or asks a model to produce a response. It
/// validates only the dedicated Codex sign-in, pinned launcher, and an owned
/// temporary broker configuration.
#[derive(Debug, Eq, PartialEq)]
pub enum HermesCodexBrokerPreflight {
    MissingConfiguration(Vec<&'static str>),
    ConfigurationRejected,
    Ready,
}

impl HermesCodexBrokerPreflight {
    /// Renders one stable, stderr-safe record.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::MissingConfiguration(names) => format!(
                "LATTICE_HERMES_CODEX_BROKER_PREFLIGHT_MISSING_CONFIGURATION:{}",
                names.join(",")
            ),
            Self::ConfigurationRejected => {
                "LATTICE_HERMES_CODEX_BROKER_PREFLIGHT_CONFIGURATION_REJECTED".to_owned()
            }
            Self::Ready => "LATTICE_HERMES_CODEX_BROKER_PREFLIGHT_READY".to_owned(),
        }
    }
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

fn hermes_activation_status(preflight: HermesProductionPreflight) -> &'static str {
    match preflight {
        HermesProductionPreflight::MissingConfiguration(_) => "CONFIGURATION_REQUIRED",
        HermesProductionPreflight::ConfigurationRejected => "CONFIGURATION_REJECTED",
        HermesProductionPreflight::ConfigurationPresentUnverified => "PREPARED",
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

/// Validates the direct Codex reflection configuration without launching a
/// model turn, PostgreSQL, or the retired Hermes Gateway.
#[must_use]
pub fn hermes_runtime_preflight_from_environment() -> HermesRuntimePreflight {
    #[cfg(not(windows))]
    {
        HermesRuntimePreflight::ConfigurationRejected
    }
    #[cfg(windows)]
    {
        const REQUIRED: [&str; 5] = [
            "LATTICE_HERMES_PRODUCT_ROOT",
            "LATTICE_HERMES_CODEX_LAUNCHER",
            "LATTICE_HERMES_CODEX_HOME",
            "LATTICE_HERMES_BROKER_ISOLATION_ROOT",
            "LATTICE_HERMES_DEADLINE_SECONDS",
        ];
        let missing = REQUIRED
            .into_iter()
            .filter(|name| std::env::var_os(name).is_none())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return HermesRuntimePreflight::MissingConfiguration(missing);
        }

        let result = DirectCodexHermesEnvironmentConfig::from_environment();
        if result.is_err() {
            HermesRuntimePreflight::ConfigurationRejected
        } else {
            HermesRuntimePreflight::ConfigurationPresentUnverified
        }
    }
}

/// Verifies the Hermes-to-Codex local broker boundary without starting Hermes
/// or issuing a model request. Any temporary broker root is owned and cleaned
/// by the adapter when this check returns.
#[must_use]
pub fn hermes_codex_broker_preflight_from_environment() -> HermesCodexBrokerPreflight {
    #[cfg(not(windows))]
    {
        HermesCodexBrokerPreflight::ConfigurationRejected
    }
    #[cfg(windows)]
    {
        const REQUIRED: [&str; 4] = [
            "LATTICE_HERMES_CODEX_LAUNCHER",
            "LATTICE_HERMES_CODEX_HOME",
            "LATTICE_HERMES_BROKER_ISOLATION_ROOT",
            "LATTICE_HERMES_DEADLINE_SECONDS",
        ];
        let missing = REQUIRED
            .into_iter()
            .filter(|name| std::env::var_os(name).is_none())
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return HermesCodexBrokerPreflight::MissingConfiguration(missing);
        }

        let result = (|| {
            let configuration = DirectCodexHermesEnvironmentConfig::from_environment()?;
            let deadline = Instant::now()
                .checked_add(configuration.timeout)
                .ok_or_else(|| {
                    LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired)
                })?;
            let _receipt = configuration
                .broker
                .run_zero_model_preflight(deadline)
                .map_err(|_| {
                    LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired)
                })?;
            Ok::<(), LatticedError>(())
        })();

        if result.is_ok() {
            HermesCodexBrokerPreflight::Ready
        } else {
            HermesCodexBrokerPreflight::ConfigurationRejected
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
        const REQUIRED: [&str; 11] = [
            "LATTICE_HERMES_PREPARATION_ROOT",
            "LATTICE_HERMES_PREPARATION_RECEIPT_SHA256",
            "LATTICE_HERMES_RUNTIME_MANIFEST",
            "LATTICE_HERMES_RUNTIME_GUEST_ROOT",
            "LATTICE_HERMES_PRODUCT_ROOT",
            "LATTICE_HERMES_WSL_EXE",
            "LATTICE_HERMES_ISOLATION_PARENT",
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

#[derive(Clone, Copy)]
struct ManagedScriptedRestartSelectorInput<'a> {
    run_mode: FullChainRunMode,
    runtime: DeliveryRuntime,
    enabled: Option<&'a str>,
    foreman_mode: Option<&'a str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManagedScriptedAcceptanceBinding {
    owner_root: PathBuf,
    control_origin: String,
    project_root: PathBuf,
    managed_worktree_root: PathBuf,
}

#[derive(Clone, Copy)]
struct ManagedScriptedAcceptanceBindingInput<'a> {
    owner_root: &'a Path,
    control_port: u16,
    control_origin: &'a str,
    marker_project_root: &'a Path,
    configured_project_root: &'a Path,
    configured_worktree_root: &'a Path,
}

fn select_managed_scripted_acceptance_binding(
    input: ManagedScriptedAcceptanceBindingInput<'_>,
) -> Result<ManagedScriptedAcceptanceBinding, LatticedErrorKind> {
    let expected_origin = format!("http://127.0.0.1:{}", input.control_port);
    let expected_worktree_root = input.owner_root.join("managed-worktrees");
    if input.control_port == 0
        || input.control_origin != expected_origin
        || !same_declared_path(input.marker_project_root, input.configured_project_root)
        || !same_declared_path(
            input.configured_project_root,
            &input.owner_root.join("repository"),
        )
        || !same_declared_path(input.configured_worktree_root, &expected_worktree_root)
    {
        return Err(LatticedErrorKind::ScriptedFixtureRejected);
    }
    Ok(ManagedScriptedAcceptanceBinding {
        owner_root: input.owner_root.to_path_buf(),
        control_origin: expected_origin,
        project_root: input.configured_project_root.to_path_buf(),
        managed_worktree_root: input.configured_worktree_root.to_path_buf(),
    })
}

fn select_managed_scripted_active_restart(
    input: ManagedScriptedRestartSelectorInput<'_>,
) -> Result<bool, LatticedErrorKind> {
    if input.runtime != DeliveryRuntime::ScriptedAcceptance {
        return Ok(false);
    }
    match (input.enabled, input.foreman_mode) {
        (None, _) => Ok(false),
        (Some("1"), Some("ACTIVE")) if input.run_mode == FullChainRunMode::ResumeExisting => {
            Ok(true)
        }
        _ => Err(LatticedErrorKind::Configuration),
    }
}

fn validate_managed_scripted_active_restart_admission(
    run_mode: FullChainRunMode,
    config: &LatticedDeliveryConfig,
    database: &DeliveryDatabaseBinding,
) -> Result<ManagedScriptedAcceptanceBinding, LatticedError> {
    let enabled = optional_unicode_environment(MANAGED_SCRIPTED_ACTIVE_RESTART_ENV)?;
    let foreman_mode = optional_unicode_environment(MANAGED_FOREMAN_MODE_ENV)?;
    if !select_managed_scripted_active_restart(ManagedScriptedRestartSelectorInput {
        run_mode,
        runtime: config.runtime,
        enabled: enabled.as_deref(),
        foreman_mode: foreman_mode.as_deref(),
    })
    .map_err(LatticedError::new)?
    {
        return Err(LatticedError::new(LatticedErrorKind::OfficialLiveBlocked));
    }

    let fixture_root = canonical_directory(
        config
            .delivery_root
            .parent()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?,
    )?;
    let active_marker = canonical_regular_file(
        &fixture_root.join(MANAGED_SCRIPTED_ACTIVE_MARKER_NAME),
        MAX_SCRIPTED_MARKER_BYTES,
    )?;
    if active_marker != fixture_root.join(MANAGED_SCRIPTED_ACTIVE_MARKER_NAME)
        || read_regular_file(&active_marker, MAX_SCRIPTED_MARKER_BYTES)?
            != MANAGED_SCRIPTED_ACTIVE_MARKER_BYTES
    {
        return Err(LatticedError::new(
            LatticedErrorKind::ScriptedFixtureRejected,
        ));
    }

    let owner_marker_configured =
        PathBuf::from(required_environment(MANAGED_SCRIPTED_OWNER_MARKER_ENV)?);
    if !owner_marker_configured.is_absolute() {
        return Err(LatticedError::new(
            LatticedErrorKind::ScriptedFixtureRejected,
        ));
    }
    let owner_marker = canonical_regular_file(&owner_marker_configured, MAX_SCRIPTED_MARKER_BYTES)?;
    let owner_root = canonical_directory(
        owner_marker
            .parent()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?,
    )?;
    let temporary_root = canonical_directory(&env::temp_dir())?;
    let expected_root_name = format!("lattice-phase4-managed-foreman-{}", database.run_id());
    if owner_marker != owner_root.join(".phase4-owner.json")
        || owner_root.parent() != Some(temporary_root.as_path())
        || owner_root.file_name() != Some(OsStr::new(&expected_root_name))
    {
        return Err(LatticedError::new(
            LatticedErrorKind::ScriptedFixtureRejected,
        ));
    }

    let marker_bytes = read_regular_file(&owner_marker, MAX_SCRIPTED_MARKER_BYTES)?;
    let marker: Value = serde_json::from_slice(&marker_bytes)
        .map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?;
    let object = marker
        .as_object()
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?;
    let expected_keys = [
        "codex_sha256",
        "control_home",
        "control_port",
        "data_root",
        "latticed_sha256",
        "owner",
        "postgres_executable",
        "postgres_port",
        "postgres_sha256",
        "project_root",
        "root",
        "run_id",
    ];
    let postgres_port = required_environment("LATTICE_TASK019_PORT")?
        .parse::<u16>()
        .map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?;
    let control_port = object
        .get("control_port")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| *value != 0 && *value != postgres_port)
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?;
    let postgres_executable = canonical_regular_file(
        Path::new(marker_string(object, "postgres_executable")?),
        MAX_LATTICED_EXECUTABLE_BYTES,
    )?;
    let current_executable = canonical_regular_file(
        &env::current_exe()
            .map_err(|_| LatticedError::new(LatticedErrorKind::ScriptedFixtureRejected))?,
        MAX_LATTICED_EXECUTABLE_BYTES,
    )?;
    let data_root = canonical_directory(Path::new(marker_string(object, "data_root")?))?;
    let control_home = canonical_directory(Path::new(marker_string(object, "control_home")?))?;
    let project_root = canonical_directory(Path::new(marker_string(object, "project_root")?))?;
    if object.len() != expected_keys.len()
        || expected_keys.iter().any(|key| !object.contains_key(*key))
        || marker_string(object, "owner")? != MANAGED_SCRIPTED_OWNER_KIND
        || marker_string(object, "run_id")? != database.run_id()
        || canonical_directory(Path::new(marker_string(object, "root")?))? != owner_root
        || data_root != owner_root.join("postgres-data")
        || control_home != owner_root.join("control-home")
        || project_root != owner_root.join("repository")
        || object.get("postgres_port").and_then(Value::as_u64) != Some(u64::from(postgres_port))
        || control_port == postgres_port
        || marker_string(object, "codex_sha256")? != config.launcher_sha256
        || marker_string(object, "postgres_sha256")?
            != file_sha256(&postgres_executable, MAX_LATTICED_EXECUTABLE_BYTES)?
        || marker_string(object, "latticed_sha256")?
            != file_sha256(&current_executable, MAX_LATTICED_EXECUTABLE_BYTES)?
    {
        return Err(LatticedError::new(
            LatticedErrorKind::ScriptedFixtureRejected,
        ));
    }
    let configured_project_root = canonical_directory(Path::new(&required_environment(
        "LATTICE_GRAPHIFY_SOURCE_ROOT",
    )?))?;
    let configured_worktree_root = canonical_directory(Path::new(&required_environment(
        MANAGED_FOREMAN_WORKTREE_ROOT_ENV,
    )?))?;
    let control_origin = required_environment("LATTICE_CONTROL_ORIGIN")?;
    select_managed_scripted_acceptance_binding(ManagedScriptedAcceptanceBindingInput {
        owner_root: &owner_root,
        control_port,
        control_origin: &control_origin,
        marker_project_root: &project_root,
        configured_project_root: &configured_project_root,
        configured_worktree_root: &configured_worktree_root,
    })
    .map_err(LatticedError::new)
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

fn optional_absolute_path_environment(name: &str) -> Result<Option<PathBuf>, LatticedError> {
    optional_unicode_environment(name)?.map_or(Ok(None), |value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            Ok(Some(path))
        } else {
            Err(LatticedError::new(LatticedErrorKind::Configuration))
        }
    })
}

fn managed_foreman_service_from_environment(
    config: &LatticedDeliveryConfig,
    database: &DeliveryDatabaseBinding,
    password: &str,
    store_authority: &StoreAuthorityHead,
    task_ingress_peer: &TaskIngressPeerEvidence,
    process_start_identity: &ContentDigest,
) -> Result<Option<ManagedForemanServiceConfig>, LatticedError> {
    match optional_unicode_environment(MANAGED_FOREMAN_MODE_ENV)?.as_deref() {
        None | Some("DISABLED") => return Ok(None),
        Some("ACTIVE") => {}
        Some(_) => return Err(LatticedError::new(LatticedErrorKind::Configuration)),
    }

    // Resume/status-only composition intentionally omits delivery executables.
    // An active managed foreman therefore reloads the same process-owned paths
    // from the closed environment; no MCP argument can choose an executable.
    let codex_executable = if config.launcher.as_os_str().is_empty() {
        PathBuf::from(required_environment("LATTICE_DELIVERY_LAUNCHER")?)
    } else {
        config.launcher.clone()
    };
    let git_executable = if config.git_executable.as_os_str().is_empty() {
        PathBuf::from(required_environment("LATTICE_DELIVERY_GIT_EXE")?)
    } else {
        config.git_executable.clone()
    };
    let codex_home = if config.codex_home.as_os_str().is_empty() {
        PathBuf::from(required_environment(DELIVERY_CODEX_HOME_ENV)?)
    } else {
        config.codex_home.clone()
    };
    let node_executable = PathBuf::from(required_environment(MANAGED_FOREMAN_NODE_ENV)?);
    let bridge_path = PathBuf::from(required_environment(MANAGED_FOREMAN_BRIDGE_ENV)?);
    let worktree_bridge_path = bridge_path.with_file_name("managed-worktree-bridge.mjs");
    let worktree_root = PathBuf::from(required_environment(MANAGED_FOREMAN_WORKTREE_ROOT_ENV)?);
    if [
        &codex_executable,
        &codex_home,
        &git_executable,
        &node_executable,
        &bridge_path,
        &worktree_bridge_path,
        &worktree_root,
    ]
    .into_iter()
    .any(|path| !path.is_absolute())
    {
        return Err(LatticedError::new(LatticedErrorKind::Configuration));
    }

    let effect_bundle_guard = managed_foreman_effect_bundle_guard(config, &codex_executable)?;

    let mut service = ManagedForemanServiceConfig::new(
        database.clone(),
        password.to_owned(),
        config.timeout,
        store_authority.clone(),
        task_ingress_peer.clone(),
        process_start_identity.clone(),
        codex_executable,
        codex_home,
        node_executable,
        bridge_path,
        worktree_bridge_path,
        worktree_root,
        git_executable,
        optional_absolute_path_environment(MANAGED_FOREMAN_NPM_ENV)?,
        optional_absolute_path_environment(MANAGED_FOREMAN_CARGO_ENV)?,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
    if let Some(descriptor) =
        optional_unicode_environment(MANAGED_FOREMAN_EXECUTION_ENVIRONMENT_ENV)?
    {
        service = service
            .with_execution_environment_template(&descriptor)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
    }
    service
        .with_effect_bundle_guard(effect_bundle_guard)
        .map(Some)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))
}

fn managed_foreman_effect_bundle_guard(
    config: &LatticedDeliveryConfig,
    codex_executable: &Path,
) -> Result<ManagedEffectBundleGuard, LatticedError> {
    let rejected = || LatticedError::new(LatticedErrorKind::OfficialLiveBlocked);
    match config.runtime {
        DeliveryRuntime::OfficialCodexAppServer => {
            let reloaded;
            let bundle = if let Some(bundle) = config.official_bundle.as_ref() {
                bundle
            } else {
                reloaded = validate_official_codex_identity(
                    codex_executable,
                    &required_environment("LATTICE_DELIVERY_LAUNCHER_VERSION")?,
                    &required_environment("LATTICE_DELIVERY_LAUNCHER_SHA256")?,
                )?;
                &reloaded
            };
            bundle.ensure_current().map_err(|_| rejected())?;
            Ok(bundle.managed_effect_guard())
        }
        DeliveryRuntime::ScriptedAcceptance => {
            if codex_executable.file_name() != Some(OsStr::new("scripted-codex.cmd")) {
                return Err(rejected());
            }
            let server = codex_executable.with_file_name("scripted-codex.ps1");
            // Capture both path-loaded scripts in one process-lifetime bundle
            // before comparing either file. Reading the launcher or its
            // PowerShell dependency before the shared deny-write/delete seal
            // would leave a good-read -> malicious-open substitution window.
            let guard = ManagedEffectBundleGuard::capture([
                (codex_executable.to_path_buf(), MAX_SCRIPTED_LAUNCHER_BYTES),
                (server.clone(), MAX_SCRIPTED_SERVER_BYTES),
            ])
            .map_err(|()| rejected())?;
            let server_sha256 = file_sha256(&server, MAX_SCRIPTED_SERVER_BYTES)?;
            if read_regular_file(&server, MAX_SCRIPTED_SERVER_BYTES)? != SCRIPTED_SERVER_BYTES
                || read_regular_file(codex_executable, MAX_SCRIPTED_LAUNCHER_BYTES)?
                    != scripted_launcher_bytes(&server_sha256)
                || guard.verify().is_err()
            {
                return Err(rejected());
            }
            Ok(guard)
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
        Some("GRAPHIFY_HERMES" | "FULL_CHAIN") => Ok(RuntimeIntegrationMode::GraphifyHermes),
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
    managed_scripted_acceptance: Option<ManagedScriptedAcceptanceBinding>,
    managed_foreman: Option<ManagedForemanServiceConfig>,
    managed_tasks: Arc<Mutex<BTreeSet<String>>>,
    managed_scheduler: Option<ManagedSchedulerOwner>,
}

struct ManagedScheduledTask {
    submission: TaskSubmissionEnvelope,
    repository_path: PathBuf,
}

struct ManagedSchedulerOwner {
    sender: Option<mpsc::SyncSender<ManagedScheduledTask>>,
    cancellation: ManagedWorkerCancellation,
    workers: Vec<ManagedSchedulerWorker>,
    rescan_requested: Arc<AtomicBool>,
    armed: bool,
}

struct ManagedSchedulerWorker {
    completion: mpsc::Receiver<()>,
    handle: Option<thread::JoinHandle<Result<(), &'static str>>>,
}

struct ManagedSchedulerCompletion(Option<mpsc::SyncSender<()>>);

impl Drop for ManagedSchedulerCompletion {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

impl ManagedSchedulerOwner {
    fn sender(&self) -> Option<&mpsc::SyncSender<ManagedScheduledTask>> {
        self.sender.as_ref()
    }

    fn request_rescan(&self) {
        self.rescan_requested.store(true, AtomicOrdering::Release);
    }

    fn shutdown(&mut self) -> Result<(), LatticedError> {
        self.shutdown_with_deadline(MANAGED_SCHEDULER_SHUTDOWN_DEADLINE)
    }

    fn shutdown_with_deadline(&mut self, timeout: Duration) -> Result<(), LatticedError> {
        if !self.armed {
            return Ok(());
        }
        self.cancellation.request();
        self.sender.take();
        // Exact provider interruption/reaping and the durable terminal write
        // are distinct shutdown phases.  Giving both phases the same absolute
        // deadline can reject a healthy worker after the bridge consumed the
        // window while producing its exact terminal receipt.
        let bridges_drained = self.cancellation.wait_for_no_active_bridges(timeout);
        let deadline = Instant::now().checked_add(timeout);
        let mut rejected = !bridges_drained;
        while !self.workers.is_empty() {
            let Some(remaining) =
                deadline.and_then(|value| value.checked_duration_since(Instant::now()))
            else {
                rejected = true;
                break;
            };
            match self.workers[0].completion.recv_timeout(remaining) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let mut worker = self.workers.remove(0);
                    match worker
                        .handle
                        .take()
                        .expect("managed scheduler handle")
                        .join()
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) | Err(_) => rejected = true,
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Keep the JoinHandle owned by this guard. Returning a
                    // teardown failure is bounded; a later retry can join a
                    // now-complete worker, and Drop performs the final
                    // non-detaching join during process teardown.
                    rejected = true;
                    break;
                }
            }
        }
        if rejected && !self.cancellation.wait_for_no_active_bridges(Duration::ZERO) {
            // Every managed bridge is a kill-on-close Job and must reap within
            // its five-second adapter deadline after cancellation. Continuing
            // with a live subtree would violate fencing; fail-stop closes all
            // Job handles instead of returning an unsafe teardown receipt.
            process::abort();
        }
        self.armed = !self.workers.is_empty();
        if rejected {
            Err(LatticedError::new(
                LatticedErrorKind::ManagedTeardownRejected,
            ))
        } else {
            Ok(())
        }
    }
}

impl Drop for ManagedSchedulerOwner {
    fn drop(&mut self) {
        if self.armed {
            // A partially assembled runtime must never detach scheduler
            // threads or their owned worker Jobs. Drop cannot report the
            // teardown error, but it still cancels and joins every worker.
            self.cancellation.request();
            self.sender.take();
            let deadline = Instant::now()
                .checked_add(MANAGED_SCHEDULER_SHUTDOWN_DEADLINE)
                .unwrap_or_else(|| process::abort());
            while !self.workers.is_empty() {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    process::abort();
                };
                match self.workers[0].completion.recv_timeout(remaining) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        let mut worker = self.workers.remove(0);
                        if let Some(handle) = worker.handle.take() {
                            match handle.join() {
                                Ok(Ok(())) => {}
                                Ok(Err(_)) | Err(_) => process::abort(),
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => process::abort(),
                }
            }
            self.armed = false;
        }
    }
}

#[derive(Clone)]
struct ManagedForemanIdentitySource {
    database: DeliveryDatabaseBinding,
    password: String,
    timeout: Duration,
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
        let foreman = {
            let mut coordination = foreman_coordination(self).map_err(foreman_replay_latticed)?;
            let replay = coordination
                .load_runtime_status()
                .map_err(|error| foreman_replay_latticed(ToolExecutionError::new(error.code())))?;
            drop(coordination);
            replay
        };
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
        object.insert(
            "hermes_activation_status".to_owned(),
            Value::String(
                hermes_activation_status(hermes_production_preflight_from_environment()).to_owned(),
            ),
        );
        // Writer readiness is observed only after the Task Ledger replay has
        // been verified. It can degrade write readiness, never replay truth.
        let identity = foreman_coordination_identity()
            .map_err(|_| LatticedError::new(LatticedErrorKind::ForemanReplayCorrupt))?;
        let mut writer = foreman_writer_lease(self).map_err(foreman_replay_latticed)?;
        let writer_active = writer
            .current_authority(identity.project_id())
            .map_err(foreman_writer_observation_error)
            .map_err(foreman_replay_latticed)?
            .is_some();
        let degraded_code = foreman_writer_degraded_code(writer_active)
            .map_or(Value::Null, |code| Value::String(code.to_owned()));
        let dependency = foreman
            .dependency()
            .map_or(Value::Null, dependency_continuation_json);
        object.insert(
            "foreman".to_owned(),
            json!({
                "schema": "lattice.foreman-runtime-projection/1.1",
                "replay_status": "VERIFIED",
                "checkpoint_status": if foreman.latest_generation() == 0 { "NONE" } else { "AVAILABLE" },
                "ledger_digest": foreman.ledger_digest().as_str(),
                "checkpoint_digest": if foreman.latest_generation() == 0 {
                    Value::Null
                } else {
                    Value::String(foreman.checkpoint_digest().as_str().to_owned())
                },
                "latest_generation": foreman.latest_generation(),
                "active_count": foreman.active_count(),
                "blocked_count": foreman.blocked_count(),
                "completed_count": foreman.completed_count(),
                "next_action": foreman.next_action(),
                "degraded_code": degraded_code,
                "dependency": dependency,
            }),
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
        submission: &TaskSpecSubmission,
        binding: &SubjectBinding,
        writer_authority: &WriterLeaseAuthorityHead,
        writer_guard: &mut dyn WriterAuthorityGuardPort,
        delivery_root: &Path,
    ) -> Result<Value, LatticedError> {
        match self.run_mode {
            FullChainRunMode::Fresh => self.delivery.run_task_json(
                submission,
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

    fn status_task_json_at(
        &mut self,
        binding: &SubjectBinding,
        operation_deadline: Instant,
    ) -> Result<Value, LatticedError> {
        self.delivery
            .status_task_json_at(binding, operation_deadline)
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

fn dependency_continuation_json(dependency: &DependencyContinuation) -> Value {
    let binding = DependencyBinding::new(
        dependency.parent_task_id(),
        dependency.dependency_task_id(),
        dependency.dependency_worktree_id(),
        dependency.dependency_branch(),
        dependency.base_sha(),
        "COMPLETE_DEPENDENCY",
    );
    let verification = binding
        .as_ref()
        .map_err(|_| "FOREMAN_REPLAY_CORRUPT")
        .and_then(|binding| {
            verify_dependency_git(
                binding,
                Some(dependency),
                match dependency.state() {
                    DependencyContinuationState::Blocked => DependencyGitPhase::ObserveBlocked,
                    DependencyContinuationState::Resumed => DependencyGitPhase::ObserveResumed,
                },
            )
        });
    let (verification_status, next_action) = match verification {
        Ok(_) => ("VERIFIED", dependency.next_action()),
        Err(_) => ("RECONCILIATION_REQUIRED", "RECONCILE_DEPENDENCY"),
    };
    json!({
        "schema": "lattice.dependency-continuation/1.0",
        "parent_task_id": dependency.parent_task_id(),
        "dependency_task_id": dependency.dependency_task_id(),
        "depends_on": dependency.dependency_task_id(),
        "parent_branch": dependency.parent_branch(),
        "parent_worktree": dependency.parent_worktree(),
        "base_sha": dependency.base_sha(),
        "dependency_branch": dependency.dependency_branch(),
        "dependency_worktree_id": dependency.dependency_worktree_id(),
        "state": match dependency.state() {
            DependencyContinuationState::Blocked => "BLOCKED",
            DependencyContinuationState::Resumed => "RESUMED",
        },
        "next_action": next_action,
        "verification_status": verification_status,
    })
}

struct FullChainTaskExecution<'a, H> {
    core: &'a mut FullChainCore<H>,
    submission: &'a TaskSpecSubmission,
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
            .run_task_json(
                self.submission,
                binding,
                writer_authority,
                writer_guard,
                &delivery_root,
            )
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
    let operation_deadline = deadline(core.delivery.timeout).map_err(|_| {
        TaskLifecycleError::new(
            TaskLifecycleErrorKind::Unavailable,
            "LATTICE_TASK_LEDGER_DEADLINE_REJECTED",
        )
    })?;
    task_lifecycle_at(core, binding, operation_deadline)
}

fn task_lifecycle_at<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    binding: &SubjectBinding,
    operation_deadline: Instant,
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
        operation_deadline,
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

fn connect_control_product<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
) -> Result<PostgresControlProduct, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    let target = StoreMigrationTarget::new(
        core.delivery.database.database_name(),
        core.delivery.database.run_id(),
    )
    .map_err(|_| ToolExecutionError::new("CONTROL_PRODUCT_STORE_REJECTED"))?;
    let client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout).map_err(|error| ToolExecutionError::new(error.code()))?,
    )
    .map_err(|_| ToolExecutionError::new("CONTROL_PRODUCT_DATABASE_UNAVAILABLE"))?;
    PostgresControlProduct::new(client, &target).map_err(ToolExecutionError::new)
}

fn control_product_project<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    project_id: &str,
) -> Result<Value, ToolExecutionError> {
    let id = ProjectId::new(project_id)
        .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_REJECTED"))?;
    let target = StoreMigrationTarget::new(
        core.delivery.database.database_name(),
        core.delivery.database.run_id(),
    )
    .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_REJECTED"))?;
    let client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout).map_err(|error| ToolExecutionError::new(error.code()))?,
    )
    .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_UNAVAILABLE"))?;
    let mut registry = PostgresProjectRegistry::new(client, &target)
        .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_UNAVAILABLE"))?;
    let loaded = registry
        .load()
        .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_UNAVAILABLE"))?;
    let project = loaded
        .state()
        .project(&id)
        .ok_or_else(|| ToolExecutionError::new("PROJECT_IS_NOT_REGISTERED"))?;
    Ok(json!({
        "id":project_id,
        "canonical_path":project.observation().canonical_root(),
        "project_snapshot_id":project.authority().project_snapshot_id().as_str(),
        "active":project.authority().lifecycle()==ProjectLifecycle::Active,
    }))
}

fn general_task_lifecycle<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    submission: &TaskSubmissionEnvelope,
) -> TaskLifecycleResult<PostgresTaskLifecycle> {
    let operation_deadline = deadline(core.delivery.timeout).map_err(|_| {
        TaskLifecycleError::new(
            TaskLifecycleErrorKind::Unavailable,
            "LATTICE_TASK_LEDGER_DEADLINE_REJECTED",
        )
    })?;
    general_task_lifecycle_at(core, submission, operation_deadline)
}

fn general_task_lifecycle_at<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    submission: &TaskSubmissionEnvelope,
    operation_deadline: Instant,
) -> TaskLifecycleResult<PostgresTaskLifecycle> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| {
            TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "LATTICE_MCP_OBSERVED_EFFECT_REJECTED",
            )
        })?;
    PostgresTaskLifecycle::connect_with_ingress_peer_and_admission_profile(
        &core.delivery.database,
        &core.delivery.password,
        operation_deadline,
        submission.identity().clone(),
        core.store_authority.clone(),
        core.task_ingress_peer.clone(),
        TaskAdmissionProfile::GeneralTaskIntake(Box::new(submission.clone())),
    )
}

fn load_general_submission_by_request<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    client_request_id: &str,
) -> Result<Option<TaskSubmissionEnvelope>, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    PostgresTaskLifecycle::load_submission_by_request(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout).map_err(|error| ToolExecutionError::new(error.code()))?,
        GENERAL_TASK_INGRESS_ID,
        client_request_id,
    )
    .map_err(|error| ToolExecutionError::new(error.code()))
}

fn load_task_ingress_request_kind_by_request<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    client_request_id: &str,
) -> Result<Option<TaskIngressRequestKind>, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    PostgresTaskLifecycle::load_ingress_request_kind_by_request(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout).map_err(|error| ToolExecutionError::new(error.code()))?,
        GENERAL_TASK_INGRESS_ID,
        client_request_id,
    )
    .map_err(|error| ToolExecutionError::new(error.code()))
}

fn load_general_submission_by_task_ref<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    task_ref: &ContentDigest,
) -> Result<Option<TaskSubmissionEnvelope>, ToolExecutionError> {
    let operation_deadline =
        deadline(core.delivery.timeout).map_err(|error| ToolExecutionError::new(error.code()))?;
    load_general_submission_by_task_ref_at(core, task_ref, operation_deadline)
}

fn load_general_submission_by_task_ref_at<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    task_ref: &ContentDigest,
    operation_deadline: Instant,
) -> Result<Option<TaskSubmissionEnvelope>, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    PostgresTaskLifecycle::load_submission_by_task_ref(
        &core.delivery.database,
        &core.delivery.password,
        operation_deadline,
        task_ref,
    )
    .map_err(|error| ToolExecutionError::new(error.code()))
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
    task_writer_lease_at(core, foundation, deadline(core.delivery.timeout)?)
}

fn task_writer_lease_at<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    foundation: &TaskPersistenceFoundation,
    operation_deadline: Instant,
) -> Result<PostgresWriterLease, LatticedError> {
    let target = V5ExtensionTarget::new(
        core.delivery.database.database_name(),
        foundation.database_identity_digest().clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))?;
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        operation_deadline,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    PostgresWriterLease::new_v5_v7(client, &target, &core.store_authority, 600)
        .map_err(|_| LatticedError::new(LatticedErrorKind::WriterLease))
}

fn foreman_coordination<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
) -> Result<PostgresForemanCoordination, ToolExecutionError> {
    let operation_deadline = deadline(core.delivery.timeout)
        .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    foreman_coordination_at(core, operation_deadline)
}

fn foreman_coordination_at<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    operation_deadline: Instant,
) -> Result<PostgresForemanCoordination, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let target = StoreMigrationTarget::new(
        core.delivery.database.database_name(),
        core.delivery.database.run_id(),
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?;
    let client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        operation_deadline,
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let ledger = PostgresTaskLedger::new(client, &target).map_err(|error| {
        ToolExecutionError::new(match error.kind() {
            PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema => "FOREMAN_REPLAY_UNSUPPORTED",
            PostgresTaskLedgerErrorKind::Unavailable
            | PostgresTaskLedgerErrorKind::TransactionFailed
            | PostgresTaskLedgerErrorKind::CommitOutcomeUnknown => "FOREMAN_REPLAY_UNAVAILABLE",
            _ => "FOREMAN_REPLAY_CORRUPT",
        })
    })?;
    Ok(PostgresForemanCoordination::new(
        ledger,
        core.store_authority.clone(),
    ))
}

fn foreman_writer_lease<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
) -> Result<PostgresWriterLease, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let store_target = StoreMigrationTarget::new(
        core.delivery.database.database_name(),
        core.delivery.database.run_id(),
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?;
    let foundation_client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout)
            .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?,
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let mut foundation_ledger =
        PostgresTaskLedger::new(foundation_client, &store_target).map_err(|error| {
            ToolExecutionError::new(match error.kind() {
                PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema => {
                    "FOREMAN_REPLAY_UNSUPPORTED"
                }
                PostgresTaskLedgerErrorKind::Unavailable
                | PostgresTaskLedgerErrorKind::TransactionFailed
                | PostgresTaskLedgerErrorKind::CommitOutcomeUnknown => "FOREMAN_REPLAY_UNAVAILABLE",
                _ => "FOREMAN_REPLAY_CORRUPT",
            })
        })?;
    let _foundation = foundation_ledger.load_foreman_replay().map_err(|error| {
        ToolExecutionError::new(match error.kind() {
            PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema => "FOREMAN_REPLAY_UNSUPPORTED",
            PostgresTaskLedgerErrorKind::Unavailable
            | PostgresTaskLedgerErrorKind::TransactionFailed
            | PostgresTaskLedgerErrorKind::CommitOutcomeUnknown => "FOREMAN_REPLAY_UNAVAILABLE",
            _ => "FOREMAN_REPLAY_CORRUPT",
        })
    })?;
    let database_identity =
        ContentDigest::from_sha256(store_target.expected_database_identity_sha256().as_str())
            .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?;
    let target = V5ExtensionTarget::new(core.delivery.database.database_name(), database_identity)
        .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?;
    drop(foundation_ledger);
    let client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout)
            .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?,
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    PostgresWriterLease::new_v5_v7(client, &target, &core.store_authority, 600)
        .map_err(foreman_writer_observation_error)
}

const fn foreman_writer_degraded_code(writer_active: bool) -> Option<&'static str> {
    if writer_active {
        Some("FOREMAN_WRITER_CONTENTION")
    } else {
        None
    }
}

const fn foreman_writer_observation_error(error: WriterLeaseRepositoryError) -> ToolExecutionError {
    let code = match error.kind() {
        WriterLeaseRepositoryErrorKind::Unavailable
        | WriterLeaseRepositoryErrorKind::SerializationExhausted
        | WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown => "FOREMAN_REPLAY_UNAVAILABLE",
        WriterLeaseRepositoryErrorKind::Domain
        | WriterLeaseRepositoryErrorKind::Corrupt
        | WriterLeaseRepositoryErrorKind::AuthorityMismatch => "FOREMAN_REPLAY_CORRUPT",
    };
    ToolExecutionError::new(code)
}

fn foreman_observation_from_environment() -> Result<ForemanServerObservation, &'static str> {
    if let Some(descriptor_json) = env::var_os(MANAGED_FOREMAN_EXECUTION_ENVIRONMENT_ENV) {
        let descriptor_json = descriptor_json
            .into_string()
            .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
        let descriptor = ExecutionEnvironmentDescriptor::from_json(&descriptor_json)
            .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
        return foreman_wsl2_observation_from_environment(&descriptor);
    }
    let root = required_environment("LATTICE_GRAPHIFY_SOURCE_ROOT")
        .map(PathBuf::from)
        .and_then(|path| graph_canonical_directory(&path))
        .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
    let git = required_environment("LATTICE_DELIVERY_GIT_EXE")
        .map(PathBuf::from)
        .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
    graph_executable_sha256(&git).map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
    let top_level = graph_git_stdout(&git, &root, ["rev-parse", "--show-toplevel"])
        .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
    let top_level = graph_canonical_directory(Path::new(&top_level))
        .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
    if top_level != root {
        return Err("FOREMAN_CHECKPOINT_OBSERVATION_FAILED");
    }
    let branch = graph_git_stdout(&git, &root, ["symbolic-ref", "--short", "HEAD"])
        .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
    let head = graph_git_stdout(&git, &root, ["rev-parse", "HEAD"])
        .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")?;
    SoleForemanBinding::observe_git(branch, root.to_string_lossy(), head)
        .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")
}

fn normalized_wsl_unc_path(path: &Path) -> String {
    let normalized = path.as_os_str().to_string_lossy().replace('/', "\\");
    let ordinary = normalized
        .strip_prefix(r"\\?\UNC\")
        .map_or_else(|| normalized.clone(), |tail| format!(r"\\{tail}"));
    ordinary.to_lowercase()
}

fn foreman_wsl2_git_line(
    descriptor: &ExecutionEnvironmentDescriptor,
    arguments: &[&str],
) -> Result<String, &'static str> {
    let rejected = || "FOREMAN_CHECKPOINT_OBSERVATION_FAILED";
    let linux_root = descriptor.linux_repository_path();
    let mut components = linux_root.split('/');
    if components.next() != Some("") || components.next() != Some("home") {
        return Err(rejected());
    }
    let user = components
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(rejected)?;
    let linux_home = format!("/home/{user}");
    let gateway = PathBuf::from(descriptor.gateway().path());
    if graph_executable_sha256(&gateway).map_err(|_| rejected())?
        != descriptor.gateway().digest().as_str()
    {
        return Err(rejected());
    }
    let mut command = process::Command::new(&gateway);
    command.env_clear();
    for key in ["SystemRoot", "WINDIR"] {
        if let Some(value) = env::var_os(key) {
            command.env(key, value);
        }
    }
    let output = command
        .args([
            "-d",
            descriptor.distribution(),
            "--exec",
            "/usr/bin/env",
            "-i",
            &format!("HOME={linux_home}"),
            "PATH=/usr/bin:/bin",
            "LANG=C.UTF-8",
            "LC_ALL=C.UTF-8",
            "GCM_INTERACTIVE=Never",
            "GIT_ATTR_NOSYSTEM=1",
            "GIT_ALLOW_PROTOCOL=",
            "GIT_CONFIG_GLOBAL=/dev/null",
            "GIT_CONFIG_NOSYSTEM=1",
            "GIT_CONFIG_SYSTEM=/dev/null",
            "GIT_OPTIONAL_LOCKS=0",
            "GIT_PAGER=",
            "GIT_TERMINAL_PROMPT=0",
            descriptor.git().path(),
            "--no-optional-locks",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "core.excludesFile=/dev/null",
            "-c",
            "status.submoduleSummary=false",
            "-C",
            linux_root,
        ])
        .args(arguments)
        .output()
        .map_err(|_| rejected())?;
    if !output.status.success()
        || !output.stderr.is_empty()
        || output.stdout.is_empty()
        || output.stdout.len() > 65_536
    {
        return Err(rejected());
    }
    let line = std::str::from_utf8(&output.stdout).map_err(|_| rejected())?;
    let line = line.strip_suffix('\n').ok_or_else(rejected)?;
    if line.is_empty() || line.contains(['\0', '\r', '\n']) {
        return Err(rejected());
    }
    Ok(line.to_owned())
}

fn foreman_wsl2_observation_from_environment(
    descriptor: &ExecutionEnvironmentDescriptor,
) -> Result<ForemanServerObservation, &'static str> {
    let rejected = || "FOREMAN_CHECKPOINT_OBSERVATION_FAILED";
    let configured_root = PathBuf::from(
        required_environment("LATTICE_GRAPHIFY_SOURCE_ROOT").map_err(|_| rejected())?,
    );
    let canonical_root = graph_canonical_directory(&configured_root).map_err(|_| rejected())?;
    let mapped_root = PathBuf::from(descriptor.path_mapping_windows_path());
    if normalized_wsl_unc_path(&configured_root) != normalized_wsl_unc_path(&mapped_root)
        || normalized_wsl_unc_path(&canonical_root) != normalized_wsl_unc_path(&mapped_root)
        || descriptor.path_mapping_linux_path() != descriptor.linux_repository_path()
    {
        return Err(rejected());
    }
    let version = foreman_wsl2_git_line(descriptor, &["--version"])?;
    if version != descriptor.git().version() {
        return Err(rejected());
    }
    let top_level = foreman_wsl2_git_line(descriptor, &["rev-parse", "--show-toplevel"])?;
    let branch = foreman_wsl2_git_line(descriptor, &["symbolic-ref", "--short", "HEAD"])?;
    let head = foreman_wsl2_git_line(descriptor, &["rev-parse", "HEAD"])?;
    if top_level != descriptor.linux_repository_path() || head != descriptor.repository_head() {
        return Err(rejected());
    }
    SoleForemanBinding::observe_git(branch, descriptor.linux_repository_path(), head)
        .map_err(|_| rejected())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DependencyGitPhase {
    Block,
    ObserveBlocked,
    ObserveResumed,
    Resume,
}

fn validate_dependency_checkpoint(
    coordination: &mut impl ForemanCoordinationPort,
    intent: &ForemanCheckpointIntent,
) -> Result<Option<ForemanServerObservation>, &'static str> {
    let snapshots = coordination
        .load_snapshots()
        .map_err(|error| error.code())?;
    let projection = reconstruct(snapshots).map_err(|_| "FOREMAN_REPLAY_CORRUPT")?;
    let proposed = intent
        .blocker_ref()
        .map(DependencyBinding::from_blocker_ref)
        .transpose()
        .map_err(|_| "FOREMAN_CHECKPOINT_INVALID")?
        .flatten();
    match (intent.state(), proposed, projection.dependency()) {
        (ForemanState::Blocked, Some(binding), _) => {
            verify_dependency_git(&binding, None, DependencyGitPhase::Block)
                .and_then(DependencyGitObservation::into_server_observation)
                .map(Some)
        }
        (ForemanState::Active, None, Some(dependency))
            if dependency.state() == DependencyContinuationState::Blocked =>
        {
            let binding = DependencyBinding::new(
                dependency.parent_task_id(),
                dependency.dependency_task_id(),
                dependency.dependency_worktree_id(),
                dependency.dependency_branch(),
                dependency.base_sha(),
                "COMPLETE_DEPENDENCY",
            )
            .map_err(|_| "FOREMAN_REPLAY_CORRUPT")?;
            verify_dependency_git(&binding, Some(dependency), DependencyGitPhase::Resume)
                .and_then(DependencyGitObservation::into_server_observation)
                .map(Some)
        }
        (ForemanState::Completed, None, Some(dependency))
            if dependency.state() == DependencyContinuationState::Blocked =>
        {
            Err("FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED")
        }
        _ => Ok(None),
    }
}

fn verify_dependency_git(
    binding: &DependencyBinding,
    retained: Option<&DependencyContinuation>,
    phase: DependencyGitPhase,
) -> Result<DependencyGitObservation, &'static str> {
    let unsafe_worktree = || "FOREMAN_DEPENDENCY_WORKTREE_UNSAFE";
    let source_root = required_environment("LATTICE_GRAPHIFY_SOURCE_ROOT")
        .map(PathBuf::from)
        .and_then(|path| graph_canonical_directory(&path))
        .map_err(|_| unsafe_worktree())?;
    let dependency_root = required_environment("LATTICE_DEPENDENCY_WORKTREE_ROOT")
        .map(PathBuf::from)
        .and_then(|path| graph_canonical_directory(&path))
        .map_err(|_| unsafe_worktree())?;
    if source_root.starts_with(&dependency_root) || dependency_root.starts_with(&source_root) {
        return Err(unsafe_worktree());
    }
    let git = required_environment("LATTICE_DELIVERY_GIT_EXE")
        .map(PathBuf::from)
        .map_err(|_| unsafe_worktree())?;
    graph_executable_sha256(&git).map_err(|_| unsafe_worktree())?;
    verify_dependency_git_at(
        binding,
        retained,
        phase,
        &source_root,
        &dependency_root,
        &git,
    )
}

fn verify_dependency_git_at(
    binding: &DependencyBinding,
    retained: Option<&DependencyContinuation>,
    phase: DependencyGitPhase,
    source_root: &Path,
    dependency_root: &Path,
    git: &Path,
) -> Result<DependencyGitObservation, &'static str> {
    let binding_mismatch = || "FOREMAN_DEPENDENCY_BINDING_MISMATCH";
    let reconciliation = || "FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED";
    let hooks = dependency_hooks_directory(dependency_root)?;
    let child = dependency_owned_child(binding, source_root, dependency_root)?;
    let observed = observe_dependency_git(binding, source_root, &child, git, &hooks)?;
    match phase {
        DependencyGitPhase::Block => {
            if !dependency_git_clean(git, &hooks, source_root)?
                || !dependency_git_clean(git, &hooks, &child)?
            {
                return Err(reconciliation());
            }
            if retained.is_some()
                || observed.parent_head != binding.base_sha()
                || observed.child_head != binding.base_sha()
                || !observed.base_is_child_ancestor
            {
                return Err(binding_mismatch());
            }
        }
        DependencyGitPhase::ObserveBlocked => {
            let retained = retained.ok_or_else(reconciliation)?;
            if retained.parent_branch() != observed.parent_branch
                || graph_canonical_directory(Path::new(retained.parent_worktree()))
                    .map_err(|_| reconciliation())?
                    != source_root
                || retained.base_sha() != binding.base_sha()
                || observed.parent_head != binding.base_sha()
                || !observed.base_is_child_ancestor
            {
                return Err(binding_mismatch());
            }
        }
        DependencyGitPhase::ObserveResumed => {
            let retained = retained.ok_or_else(reconciliation)?;
            if !dependency_git_clean(git, &hooks, &child)? {
                return Err(reconciliation());
            }
            if retained.parent_branch() != observed.parent_branch
                || graph_canonical_directory(Path::new(retained.parent_worktree()))
                    .map_err(|_| reconciliation())?
                    != source_root
                || retained.base_sha() != binding.base_sha()
                || !observed.base_is_child_ancestor
                || !dependency_git_is_ancestor(
                    git,
                    &hooks,
                    source_root,
                    binding.base_sha(),
                    &observed.parent_head,
                )?
            {
                return Err(binding_mismatch());
            }
            if !dependency_git_is_ancestor(
                git,
                &hooks,
                source_root,
                &observed.child_head,
                &observed.parent_head,
            )? {
                return Err("FOREMAN_DEPENDENCY_NOT_INTEGRATED");
            }
        }
        DependencyGitPhase::Resume => {
            let retained = retained.ok_or_else(reconciliation)?;
            if !dependency_git_clean(git, &hooks, source_root)?
                || !dependency_git_clean(git, &hooks, &child)?
            {
                return Err(reconciliation());
            }
            if retained.parent_branch() != observed.parent_branch
                || graph_canonical_directory(Path::new(retained.parent_worktree()))
                    .map_err(|_| reconciliation())?
                    != source_root
                || retained.base_sha() != binding.base_sha()
            {
                return Err(binding_mismatch());
            }
            if !observed.base_is_child_ancestor
                || !dependency_git_is_ancestor(
                    git,
                    &hooks,
                    source_root,
                    binding.base_sha(),
                    &observed.parent_head,
                )?
            {
                return Err(binding_mismatch());
            }
            if !dependency_git_is_ancestor(
                git,
                &hooks,
                source_root,
                &observed.child_head,
                &observed.parent_head,
            )? {
                return Err("FOREMAN_DEPENDENCY_NOT_INTEGRATED");
            }
        }
    }
    Ok(observed)
}

fn dependency_hooks_directory(dependency_root: &Path) -> Result<PathBuf, &'static str> {
    let rejected = || "FOREMAN_DEPENDENCY_WORKTREE_UNSAFE";
    let expected = dependency_root.join(".lattice-hooks-empty");
    let hooks = graph_canonical_directory(&expected).map_err(|_| rejected())?;
    if hooks != expected
        || fs::read_dir(&hooks)
            .map_err(|_| rejected())?
            .next()
            .is_some()
    {
        return Err(rejected());
    }
    Ok(hooks)
}

struct DependencyGitObservation {
    parent_branch: String,
    parent_worktree: PathBuf,
    parent_head: String,
    child_head: String,
    base_is_child_ancestor: bool,
}

impl DependencyGitObservation {
    fn into_server_observation(self) -> Result<ForemanServerObservation, &'static str> {
        SoleForemanBinding::observe_git(
            self.parent_branch,
            self.parent_worktree.to_string_lossy(),
            self.parent_head,
        )
        .map_err(|_| "FOREMAN_CHECKPOINT_OBSERVATION_FAILED")
    }
}

fn dependency_owned_child(
    binding: &DependencyBinding,
    source_root: &Path,
    dependency_root: &Path,
) -> Result<PathBuf, &'static str> {
    let unsafe_worktree = || "FOREMAN_DEPENDENCY_WORKTREE_UNSAFE";
    let mismatch = || "FOREMAN_DEPENDENCY_BINDING_MISMATCH";
    let expected_child =
        dependency_root.join(binding.dependency_worktree_id().to_ascii_lowercase());
    let child = graph_canonical_directory(&expected_child).map_err(|_| unsafe_worktree())?;
    if child != expected_child {
        return Err(unsafe_worktree());
    }
    let marker = dependency_root.join(".lattice-ownership").join(format!(
        "{}.json",
        binding.dependency_worktree_id().to_ascii_lowercase()
    ));
    let metadata = fs::symlink_metadata(&marker).map_err(|_| unsafe_worktree())?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(unsafe_worktree());
    }
    let value = fs::read_to_string(&marker)
        .ok()
        .and_then(|value| serde_json::from_str::<Value>(&value).ok())
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(unsafe_worktree)?;
    if value.len() != 7
        || value.get("version").and_then(Value::as_u64) != Some(1)
        || value.get("worktree_id").and_then(Value::as_str)
            != Some(binding.dependency_worktree_id())
        || value.get("task_id").and_then(Value::as_str) != Some(binding.dependency_task_id())
        || value.get("branch").and_then(Value::as_str) != Some(binding.dependency_branch())
        || value.get("base_commit_sha").and_then(Value::as_str) != Some(binding.base_sha())
    {
        return Err(mismatch());
    }
    let canonical_field = |field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .and_then(|path| graph_canonical_directory(&path).ok())
            .ok_or_else(unsafe_worktree)
    };
    if canonical_field("repository_root")? != source_root
        || canonical_field("worktree_path")? != child
    {
        return Err(mismatch());
    }
    Ok(child)
}

fn observe_dependency_git(
    binding: &DependencyBinding,
    source_root: &Path,
    child: &Path,
    git: &Path,
    hooks: &Path,
) -> Result<DependencyGitObservation, &'static str> {
    let unsafe_worktree = || "FOREMAN_DEPENDENCY_WORKTREE_UNSAFE";
    let mismatch = || "FOREMAN_DEPENDENCY_BINDING_MISMATCH";
    for (root, expected) in [(source_root, source_root), (child, child)] {
        let top = dependency_git_value(git, hooks, root, ["rev-parse", "--show-toplevel"])?;
        if graph_canonical_directory(Path::new(&top)).map_err(|_| unsafe_worktree())? != expected {
            return Err(mismatch());
        }
    }
    let parent_branch = dependency_git_value(
        git,
        hooks,
        source_root,
        ["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?;
    let child_branch = dependency_git_value(
        git,
        hooks,
        child,
        ["symbolic-ref", "--quiet", "--short", "HEAD"],
    )?;
    let parent_head = dependency_git_value(git, hooks, source_root, ["rev-parse", "HEAD"])?;
    let child_head = dependency_git_value(git, hooks, child, ["rev-parse", "HEAD"])?;
    if child_branch != binding.dependency_branch()
        || !is_lower_hex(&parent_head, 40)
        || !is_lower_hex(&child_head, 40)
    {
        return Err(mismatch());
    }
    let common = |root: &Path| {
        dependency_git_value(git, hooks, root, ["rev-parse", "--git-common-dir"])
            .and_then(|value| fs::canonicalize(root.join(value)).map_err(|_| unsafe_worktree()))
    };
    if common(source_root)? != common(child)? {
        return Err(mismatch());
    }
    let listed =
        dependency_git_output(git, hooks, source_root, ["worktree", "list", "--porcelain"])?;
    let listed = std::str::from_utf8(&listed.stdout).map_err(|_| unsafe_worktree())?;
    let expected_branch = format!("branch refs/heads/{}", binding.dependency_branch());
    let matching = listed
        .replace("\r\n", "\n")
        .split("\n\n")
        .filter(|record| dependency_worktree_record_matches(record, child, &expected_branch))
        .count();
    if matching != 1 {
        return Err(mismatch());
    }
    Ok(DependencyGitObservation {
        parent_branch,
        parent_worktree: source_root.to_path_buf(),
        parent_head,
        base_is_child_ancestor: dependency_git_is_ancestor(
            git,
            hooks,
            child,
            binding.base_sha(),
            &child_head,
        )?,
        child_head,
    })
}

fn dependency_worktree_record_matches(record: &str, child: &Path, branch: &str) -> bool {
    let mut lines = record.lines();
    let path = lines
        .next()
        .and_then(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .and_then(|path| graph_canonical_directory(&path).ok());
    path.as_deref() == Some(child) && lines.any(|line| line == branch)
}

fn dependency_git_output<I, S>(
    git: &Path,
    hooks: &Path,
    root: &Path,
    arguments: I,
) -> Result<process::Output, &'static str>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut hook_config = OsString::from("core.hooksPath=");
    hook_config.push(hooks);
    let output = process::Command::new(git)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([OsStr::new("-c"), hook_config.as_os_str()])
        .args(["-c", "core.fsmonitor=false"])
        .args(arguments)
        .output()
        .map_err(|_| "FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED")?;
    if !output.status.success() || !output.stderr.is_empty() {
        return Err("FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED");
    }
    Ok(output)
}

fn dependency_git_value<I, S>(
    git: &Path,
    hooks: &Path,
    root: &Path,
    arguments: I,
) -> Result<String, &'static str>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = dependency_git_output(git, hooks, root, arguments)?;
    let value = std::str::from_utf8(&output.stdout)
        .map_err(|_| "FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED")?
        .trim();
    if value.is_empty() || value.contains(['\r', '\n', '\0']) {
        return Err("FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED");
    }
    Ok(value.to_owned())
}

fn dependency_git_clean(git: &Path, hooks: &Path, root: &Path) -> Result<bool, &'static str> {
    let output = dependency_git_output(
        git,
        hooks,
        root,
        ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    Ok(output.stdout.is_empty())
}

fn dependency_git_is_ancestor(
    git: &Path,
    hooks: &Path,
    root: &Path,
    ancestor: &str,
    descendant: &str,
) -> Result<bool, &'static str> {
    let mut hook_config = OsString::from("core.hooksPath=");
    hook_config.push(hooks);
    let output = process::Command::new(git)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([OsStr::new("-c"), hook_config.as_os_str()])
        .args(["-c", "core.fsmonitor=false"])
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .output()
        .map_err(|_| "FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED")?;
    if !output.stderr.is_empty() {
        return Err("FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED");
    }
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err("FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED"),
    }
}

fn foreman_writer_acquire<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    checkpoint_id: &str,
) -> Result<WriterLeaseAcquireRequest, ToolExecutionError> {
    let suffix = Sha256::digest(checkpoint_id.as_bytes())[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let suffix = suffix.as_str();
    let identity = foreman_coordination_identity()
        .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?;
    Ok(WriterLeaseAcquireRequest {
        command_id: format!("foreman-acquire-{suffix}"),
        expected_head: None,
        project_id: identity.project_id().clone(),
        project_snapshot_id: identity.project_snapshot_id().clone(),
        task_id: identity.task_id().clone(),
        task_revision: identity.task_revision().to_owned(),
        task_spec_digest: identity
            .task_spec_digest()
            .cloned()
            .ok_or_else(|| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?,
        attempt_id: AttemptId::new(format!("foreman-attempt-{suffix}"))
            .map_err(|_| ToolExecutionError::new("FOREMAN_CHECKPOINT_INVALID"))?,
        lease_id: format!("foreman-lease-{suffix}"),
        lease_holder_id: "latticed-foreman-v1".to_owned(),
        worktree_id: format!("foreman-worktree-{suffix}"),
        holder_process_id: HolderProcessId::new(u64::from(process::id()))
            .map_err(|_| ToolExecutionError::new("FOREMAN_WRITER_CONTENTION"))?,
        holder_process_start_identity: core.process_start_identity.clone(),
    })
}

fn foreman_error_code(error: &ForemanCheckpointOrchestratorError) -> &'static str {
    match error {
        ForemanCheckpointOrchestratorError::Replay(error)
        | ForemanCheckpointOrchestratorError::Append(error) => error.code(),
        ForemanCheckpointOrchestratorError::Observation(code) => code,
        ForemanCheckpointOrchestratorError::WriterAcquire(_)
        | ForemanCheckpointOrchestratorError::WriterContention => "FOREMAN_WRITER_CONTENTION",
        ForemanCheckpointOrchestratorError::Snapshot(_) => "FOREMAN_CHECKPOINT_INVALID",
        ForemanCheckpointOrchestratorError::WriterRelease(error) => {
            if error.kind()
                == lattice_writer_lease::WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown
            {
                "FOREMAN_RELEASE_OUTCOME_UNKNOWN"
            } else {
                "FOREMAN_WRITER_CONTENTION"
            }
        }
        ForemanCheckpointOrchestratorError::ReleaseRejected => "FOREMAN_RELEASE_OUTCOME_UNKNOWN",
    }
}

fn foreman_replay_latticed(error: ToolExecutionError) -> LatticedError {
    let kind = match error.code() {
        "FOREMAN_REPLAY_UNSUPPORTED" => LatticedErrorKind::ForemanReplayUnsupported,
        "FOREMAN_REPLAY_UNAVAILABLE" => LatticedErrorKind::ForemanReplayUnavailable,
        _ => LatticedErrorKind::ForemanReplayCorrupt,
    };
    LatticedError::new(kind)
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

fn general_task_submission(
    client_request_id: &str,
    objective: &str,
    project_display_name: &str,
    authority: &ProjectAuthorityReceipt,
) -> Result<TaskSubmissionEnvelope, LatticedError> {
    let task_id_digest = digest(
        "lattice.task.general-intake-id.v1",
        &CanonicalValue::Object(vec![
            (
                "client_request_id".to_owned(),
                CanonicalValue::String(client_request_id.to_owned()),
            ),
            (
                "ingress_id".to_owned(),
                CanonicalValue::String(GENERAL_TASK_INGRESS_ID.to_owned()),
            ),
        ]),
    )?;
    let intake_digest = digest(
        "lattice.task.general-intake-subject.v1",
        &CanonicalValue::Object(vec![
            (
                "client_request_id".to_owned(),
                CanonicalValue::String(client_request_id.to_owned()),
            ),
            (
                "objective".to_owned(),
                CanonicalValue::String(objective.to_owned()),
            ),
            (
                "project_authority_receipt_digest".to_owned(),
                CanonicalValue::String(authority.receipt_digest().as_str().to_owned()),
            ),
            (
                "project_id".to_owned(),
                CanonicalValue::String(authority.project_id().as_str().to_owned()),
            ),
            (
                "project_snapshot_id".to_owned(),
                CanonicalValue::String(authority.project_snapshot_id().as_str().to_owned()),
            ),
            (
                "schema".to_owned(),
                CanonicalValue::String("lattice.task.general-intake-subject/1.0".to_owned()),
            ),
        ]),
    )?;
    let identity = lattice_contracts::TaskLedgerStreamIdentity::new_general_task_intake(
        authority.project_id().clone(),
        authority.project_snapshot_id().clone(),
        TaskId::new(format!(
            "TASK-GENERAL-{}",
            task_id_digest.as_str()[..40].to_ascii_uppercase()
        ))
        .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?,
        "1",
        intake_digest,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?;
    TaskSubmissionEnvelope::new(
        GENERAL_TASK_INGRESS_ID,
        client_request_id,
        objective,
        project_display_name,
        identity,
        authority.receipt_digest().clone(),
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))
}

fn general_task_binding(
    submission: &TaskSubmissionEnvelope,
) -> Result<TaskIntakeBinding, LatticedError> {
    TaskIntakeBinding::try_from_stream_identity(submission.identity())
        .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))
}

fn general_submission_matches_arguments(
    submission: &TaskSubmissionEnvelope,
    arguments: &TaskSubmitArguments,
) -> bool {
    general_submission_matches_request(
        submission,
        arguments.objective(),
        arguments.project_id(),
        arguments.project_name(),
    )
}

fn general_submission_matches_request(
    submission: &TaskSubmissionEnvelope,
    objective: Option<&str>,
    project_id: Option<&str>,
    project_name: Option<&str>,
) -> bool {
    objective == Some(submission.objective())
        && project_id
            .is_none_or(|project_id| project_id == submission.identity().project_id().as_str())
        && project_name.is_none_or(|project_name| project_name == submission.project_display_name())
}

fn general_submission_matches_effective_project(
    submission: &TaskSubmissionEnvelope,
    arguments: &TaskSubmitArguments,
    effective_project_id: &ProjectId,
) -> bool {
    general_submission_matches_effective_request(
        submission,
        arguments.objective(),
        arguments.project_id(),
        arguments.project_name(),
        effective_project_id,
    )
}

fn general_submission_matches_effective_request(
    submission: &TaskSubmissionEnvelope,
    objective: Option<&str>,
    project_id: Option<&str>,
    project_name: Option<&str>,
    effective_project_id: &ProjectId,
) -> bool {
    submission.identity().project_id() == effective_project_id
        && general_submission_matches_request(submission, objective, project_id, project_name)
}

fn general_submission_after_ingress_preflight<T>(
    retained_request_kind: Option<TaskIngressRequestKind>,
    reload_submission: impl FnOnce() -> Result<Option<T>, ToolExecutionError>,
) -> Result<Option<T>, ToolExecutionError> {
    match retained_request_kind {
        Some(TaskIngressRequestKind::ControlledCodexCanary) => {
            Err(ToolExecutionError::new("LATTICE_TASK_IDEMPOTENCY_CONFLICT"))
        }
        Some(TaskIngressRequestKind::GeneralTask) => reload_submission()?
            .map(Some)
            .ok_or_else(|| ToolExecutionError::new("LATTICE_TASK_LEDGER_CORRUPT")),
        None => Ok(None),
    }
}

fn resolve_registered_project_for_general_submit<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    arguments: &TaskSubmitArguments,
) -> Result<ResolvedProjectAuthority, ToolExecutionError> {
    let selector = ProjectSelector::new(arguments.project_id(), arguments.project_name())
        .map_err(|error| ToolExecutionError::new(error.code()))?;
    record_observed_effect(ObservedEffectKind::Filesystem)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Process))
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .and_then(|()| record_observed_effect(ObservedEffectKind::Database))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    let resolved = resolve_project_authority(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout).map_err(|error| ToolExecutionError::new(error.code()))?,
        &core.store_authority,
        &selector,
    )
    .map_err(|error| ToolExecutionError::new(error.code()))?;
    if &resolved.authority().head() != resolved.current_head() {
        return Err(ToolExecutionError::new(
            "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
        ));
    }
    Ok(resolved)
}

fn resolve_registered_project_for_general_submission<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    submission: &TaskSubmissionEnvelope,
) -> Result<ResolvedProjectAuthority, ToolExecutionError> {
    let selector = ProjectSelector::new(Some(submission.identity().project_id().as_str()), None)
        .map_err(|error| ToolExecutionError::new(error.code()))?;
    record_observed_effect(ObservedEffectKind::Filesystem)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Process))
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .and_then(|()| record_observed_effect(ObservedEffectKind::Database))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    let resolved = resolve_project_authority(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout).map_err(|error| ToolExecutionError::new(error.code()))?,
        &core.store_authority,
        &selector,
    )
    .map_err(|error| ToolExecutionError::new(error.code()))?;
    if !registered_project_matches_general_submission(&resolved, submission) {
        return Err(ToolExecutionError::new(
            "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
        ));
    }
    Ok(resolved)
}

struct ManagedStatusProjectLookup {
    canonical_path: PathBuf,
    project_current: bool,
}

fn load_registered_project_for_general_status<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    submission: &TaskSubmissionEnvelope,
    operation_deadline: Instant,
) -> Result<ManagedStatusProjectLookup, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Network)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Database))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    let target = StoreMigrationTarget::new(
        core.delivery.database.database_name(),
        core.delivery.database.run_id(),
    )
    .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_REJECTED"))?;
    let client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        operation_deadline,
    )
    .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_UNAVAILABLE"))?;
    let mut registry = PostgresProjectRegistry::new(client, &target)
        .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_UNAVAILABLE"))?;
    let loaded = registry
        .load()
        .map_err(|_| ToolExecutionError::new("PROJECT_REGISTRY_UNAVAILABLE"))?;
    let project = loaded
        .state()
        .project(submission.identity().project_id())
        .ok_or_else(|| ToolExecutionError::new("PROJECT_IS_NOT_REGISTERED"))?;
    let canonical_root = project.observation().canonical_root();
    let canonical_path = PathBuf::from(canonical_root);
    if !canonical_path.is_absolute() || canonical_path.to_str() != Some(canonical_root) {
        return Err(ToolExecutionError::new("PROJECT_REGISTRY_REJECTED"));
    }
    let authority = project.authority();
    let project_current = project.project_class() == ProjectClass::UserProject
        && authority.lifecycle() == ProjectLifecycle::Active
        && project.pending_observation().is_none()
        && project.drift().is_empty()
        && authority.project_id() == submission.identity().project_id()
        && authority.project_snapshot_id() == submission.identity().project_snapshot_id()
        && authority.receipt_digest() == submission.project_authority_receipt_digest();
    Ok(ManagedStatusProjectLookup {
        canonical_path,
        project_current,
    })
}

fn registered_project_matches_general_submission(
    resolved: &ResolvedProjectAuthority,
    submission: &TaskSubmissionEnvelope,
) -> bool {
    &resolved.authority().head() == resolved.current_head()
        && resolved.authority().project_id() == submission.identity().project_id()
        && resolved.authority().project_snapshot_id() == submission.identity().project_snapshot_id()
        && resolved.authority().receipt_digest() == submission.project_authority_receipt_digest()
        && resolved.display_name() == submission.project_display_name()
}

fn replay_general_submission<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    arguments: &TaskSubmitArguments,
    existing: &TaskSubmissionEnvelope,
) -> Result<Value, ToolExecutionError> {
    if !general_submission_matches_arguments(existing, arguments) {
        return Err(ToolExecutionError::new("LATTICE_TASK_IDEMPOTENCY_CONFLICT"));
    }
    let binding =
        general_task_binding(existing).map_err(|error| ToolExecutionError::new(error.code()))?;
    let request = GeneralTaskIntakeRequest::new(binding, arguments.client_request_id())
        .map_err(|error| ToolExecutionError::new(general_task_error_code(&error)))?;
    let mut lifecycle = general_task_lifecycle(core, existing)
        .map_err(|error| ToolExecutionError::new(error.code()))?;
    let admission = create_general_task(&request, &mut lifecycle)
        .map_err(|error| ToolExecutionError::new(general_task_error_code(&error)))?;
    general_task_public_status(admission.evidence(), existing)
        .map_err(|error| ToolExecutionError::new(error.code()))
}

fn replay_general_submission_and_schedule<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    arguments: &TaskSubmitArguments,
    existing: &TaskSubmissionEnvelope,
) -> Result<Value, ToolExecutionError> {
    let status = replay_general_submission(core, arguments, existing)?;
    let resolved = resolve_registered_project_for_general_submission(core, existing)?;
    schedule_managed_general_task(
        core,
        existing.clone(),
        resolved.canonical_path().to_path_buf(),
    )?;
    Ok(status)
}

enum GeneralWinnerReplay {
    Absent,
    Conflict,
    Replayed(Value),
}

fn replay_general_winner_after_admission_failure<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    arguments: &TaskSubmitArguments,
    effective_project_id: &ProjectId,
) -> Result<GeneralWinnerReplay, ToolExecutionError> {
    let Some(winner) = load_general_submission_by_request(core, arguments.client_request_id())?
    else {
        return Ok(GeneralWinnerReplay::Absent);
    };
    if !general_submission_matches_effective_project(&winner, arguments, effective_project_id) {
        return Ok(GeneralWinnerReplay::Conflict);
    }
    replay_general_submission_and_schedule(core, arguments, &winner)
        .map(GeneralWinnerReplay::Replayed)
}

fn admit_general_submission<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    arguments: &TaskSubmitArguments,
    resolved: &ResolvedProjectAuthority,
) -> Result<Value, ToolExecutionError> {
    let objective = arguments
        .objective()
        .ok_or_else(|| ToolExecutionError::new("LATTICE_TASK_REQUEST_REJECTED"))?;
    let submission = general_task_submission(
        arguments.client_request_id(),
        objective,
        resolved.display_name(),
        resolved.authority(),
    )
    .map_err(|error| ToolExecutionError::new(error.code()))?;
    let binding =
        general_task_binding(&submission).map_err(|error| ToolExecutionError::new(error.code()))?;
    let request = GeneralTaskIntakeRequest::new(binding, arguments.client_request_id())
        .map_err(|error| ToolExecutionError::new(general_task_error_code(&error)))?;
    let mut lifecycle = general_task_lifecycle(core, &submission)
        .map_err(|error| ToolExecutionError::new(error.code()))?;
    let admission = create_general_task(&request, &mut lifecycle)
        .map_err(|error| ToolExecutionError::new(general_task_error_code(&error)))?;
    general_task_public_status(admission.evidence(), &submission)
        .map_err(|error| ToolExecutionError::new(error.code()))
}

fn general_task_public_status(
    evidence: &TaskIntakeLifecycleEvidence,
    submission: &TaskSubmissionEnvelope,
) -> Result<Value, LatticedError> {
    let binding = general_task_binding(submission)?;
    if evidence.binding() != &binding {
        return Err(LatticedError::new(LatticedErrorKind::TaskControl));
    }
    let objective_digest = task_public_objective_digest(submission.objective())
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::TaskControl))?;
    let (schema_version, status, result_digest) = match (evidence.state(), evidence.result_digest())
    {
        (TaskState::Draft, None) => ("lattice.task.status.v5", "SUBMITTED", Value::Null),
        (TaskState::Completed, Some(result_digest)) => (
            "lattice.task.status.v6",
            "COMPLETED",
            json!(result_digest.as_str()),
        ),
        _ => return Err(LatticedError::new(LatticedErrorKind::TaskControl)),
    };
    Ok(json!({
        "schema_version": schema_version,
        "status": status,
        "task_state": evidence.state().as_str(),
        "task_ref": submission.task_ref().as_str(),
        "ledger_head_digest": evidence.ledger_head_digest().as_str(),
        "result_digest": result_digest,
        "failure_stage": Value::Null,
        "failure_code": Value::Null,
        "objective_summary": TASK_PUBLIC_OBJECTIVE_SUMMARY,
        "objective_digest": objective_digest.as_str(),
        "project_id": submission.identity().project_id().as_str(),
        "project_name": submission.project_display_name(),
        "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
    }))
}

fn formal_managed_foreman_identity<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
) -> Result<FormalForemanIdentity, ToolExecutionError> {
    let foreman = {
        let mut coordination = foreman_coordination(core)?;
        coordination
            .load_runtime_status()
            .map_err(|failure| ToolExecutionError::new(failure.code()))?
    };
    formal_managed_foreman_identity_from_status(&foreman)
}

fn formal_managed_foreman_identity_at<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    operation_deadline: Instant,
) -> Result<FormalForemanIdentity, ToolExecutionError> {
    let foreman = {
        let mut coordination = foreman_coordination_at(core, operation_deadline)?;
        coordination
            .load_runtime_status()
            .map_err(|failure| ToolExecutionError::new(failure.code()))?
    };
    formal_managed_foreman_identity_from_status(&foreman)
}

fn formal_managed_foreman_identity_from_status(
    foreman: &ForemanRuntimeStatus,
) -> Result<FormalForemanIdentity, ToolExecutionError> {
    if foreman.latest_generation() == 0
        || foreman.active_count() != 1
        || foreman.blocked_count() != 0
        || foreman.completed_count() != 0
        || foreman.next_action() != "CONTINUE"
    {
        return Err(ToolExecutionError::new(MANAGED_FOREMAN_NOT_ACTIVE));
    }
    FormalForemanIdentity::new(
        foreman.latest_generation(),
        foreman.checkpoint_digest().clone(),
    )
    .map_err(|failure| ToolExecutionError::new(failure.code()))
}

fn reload_managed_foreman_identity(
    source: &ManagedForemanIdentitySource,
) -> Result<FormalForemanIdentity, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let target =
        StoreMigrationTarget::new(source.database.database_name(), source.database.run_id())
            .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?;
    let client = connect_fixed_runtime_client(
        &source.database,
        &source.password,
        deadline(source.timeout)
            .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?,
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let ledger = PostgresTaskLedger::new(client, &target).map_err(|error| {
        ToolExecutionError::new(match error.kind() {
            PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema => "FOREMAN_REPLAY_UNSUPPORTED",
            PostgresTaskLedgerErrorKind::Unavailable
            | PostgresTaskLedgerErrorKind::TransactionFailed
            | PostgresTaskLedgerErrorKind::CommitOutcomeUnknown => "FOREMAN_REPLAY_UNAVAILABLE",
            _ => "FOREMAN_REPLAY_CORRUPT",
        })
    })?;
    let mut coordination = PostgresForemanCoordination::new(ledger, source.store_authority.clone());
    let foreman = coordination
        .load_runtime_status()
        .map_err(|failure| ToolExecutionError::new(failure.code()))?;
    formal_managed_foreman_identity_from_status(&foreman)
}

fn managed_status_tool_error(
    operation_deadline: Instant,
    fallback_code: &'static str,
) -> ToolExecutionError {
    ToolExecutionError::new(if Instant::now() >= operation_deadline {
        MANAGED_STATUS_TIMEOUT
    } else {
        fallback_code
    })
}

fn managed_general_task_public_status<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    evidence: &TaskIntakeLifecycleEvidence,
    submission: &TaskSubmissionEnvelope,
    status_config: Option<&ManagedForemanServiceConfig>,
) -> Result<Value, ToolExecutionError> {
    let Some(config) = status_config else {
        return general_task_public_status(evidence, submission)
            .map_err(|error| ToolExecutionError::new(error.code()));
    };
    let operation_deadline = config
        .status_request_deadline()
        .ok_or_else(|| ToolExecutionError::new(MANAGED_STATUS_TIMEOUT))?;
    let identity = formal_managed_foreman_identity_at(core, operation_deadline)
        .map_err(|error| managed_status_tool_error(operation_deadline, error.code()))?;
    let project = load_registered_project_for_general_status(core, submission, operation_deadline)
        .map_err(|error| managed_status_tool_error(operation_deadline, error.code()))?;
    let managed_status = managed_task_public_status(
        config,
        submission.clone(),
        &project.canonical_path,
        &identity,
    )
    .map_err(|failure| managed_status_tool_error(operation_deadline, failure.code()))?;
    if Instant::now() >= operation_deadline {
        return Err(ToolExecutionError::new(MANAGED_STATUS_TIMEOUT));
    }
    match managed_status {
        Some(status) => {
            require_managed_project_status_projection(project.project_current, submission, status)
        }
        None if project.project_current => general_task_public_status(evidence, submission)
            .map_err(|error| ToolExecutionError::new(error.code())),
        None => Err(ToolExecutionError::new(
            "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
        )),
    }
}

fn require_managed_project_status_projection(
    project_current: bool,
    submission: &TaskSubmissionEnvelope,
    status: Value,
) -> Result<Value, ToolExecutionError> {
    if status.get("schema_version").and_then(Value::as_str) != Some("lattice.task.status.v4")
        || status.get("task_ref").and_then(Value::as_str) != Some(submission.task_ref().as_str())
        || status.get("project_id").and_then(Value::as_str)
            != Some(submission.identity().project_id().as_str())
        || status.get("project_snapshot_id").and_then(Value::as_str)
            != Some(submission.identity().project_snapshot_id().as_str())
    {
        return Err(ToolExecutionError::new(
            "LATTICE_MANAGED_STATUS_SUBSTITUTION_REJECTED",
        ));
    }
    if project_current {
        return Ok(status);
    }
    let project_blocker_exact = status.get("blocker").and_then(Value::as_str)
        == Some("PROJECT_REGISTRY_CURRENTNESS_CONFLICT")
        && status.get("failure_code").and_then(Value::as_str)
            == Some("PROJECT_REGISTRY_CURRENTNESS_CONFLICT")
        && status.get("next_action").and_then(Value::as_str)
            == Some("Refresh the registered project authority, then retry this task.");
    if !project_blocker_exact && !managed_project_drift_has_stronger_durable_status(&status) {
        return Err(ToolExecutionError::new(
            "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
        ));
    }
    Ok(status)
}

fn managed_project_drift_has_stronger_durable_status(status: &Value) -> bool {
    let blocker = status.get("blocker").and_then(Value::as_str);
    let failure_code = status.get("failure_code").and_then(Value::as_str);
    if blocker != failure_code {
        return false;
    }
    if managed_status_requires_exact_provider_reconciliation(status) {
        return true;
    }
    if status.get("task_state").and_then(Value::as_str) == Some("VERIFYING")
        && status.get("verification_status").and_then(Value::as_str) == Some("RUNNING")
        && matches!(blocker, None | Some("EXECUTION_AUTHORITY_NOT_CURRENT"))
    {
        return true;
    }
    if status.get("task_state").and_then(Value::as_str) == Some("REVIEWING")
        && status.get("status").and_then(Value::as_str) == Some("RUNNING")
        && status.get("verification_status").and_then(Value::as_str) == Some("PASSED")
        && blocker.is_none()
    {
        return true;
    }
    if blocker.is_some_and(|code| !managed_status_has_known_closed_blocker(code)) {
        return false;
    }
    managed_status_is_durably_closed(status)
}

fn schedule_managed_general_task<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
    submission: TaskSubmissionEnvelope,
    mut repository_path: PathBuf,
) -> Result<(), ToolExecutionError> {
    if core.managed_foreman.is_none() {
        return Ok(());
    }
    if let Some(binding) = core.managed_scripted_acceptance.as_ref() {
        let canonical_repository = canonical_directory(&repository_path).map_err(|_| {
            ToolExecutionError::new("LATTICE_MANAGED_SCRIPTED_PROJECT_SUBSTITUTION_REJECTED")
        })?;
        if !same_declared_path(&canonical_repository, &binding.project_root) {
            return Err(ToolExecutionError::new(
                "LATTICE_MANAGED_SCRIPTED_PROJECT_SUBSTITUTION_REJECTED",
            ));
        }
        repository_path = canonical_repository;
    }
    // General intake is already durable PostgreSQL truth. Admission to this
    // bounded local queue must not synchronously run Git/policy/promotion or
    // turn a durable submit into an RPC failure. The supervisor is the sole
    // owner of prepare/promote; a crash or full queue is recovered through the
    // formal DRAFT restart-discovery row for this exact task_ref.
    let scheduler = core
        .managed_scheduler
        .as_ref()
        .ok_or_else(|| ToolExecutionError::new("LATTICE_MANAGED_SCHEDULER_UNAVAILABLE"))?;
    let sender = scheduler
        .sender()
        .ok_or_else(|| ToolExecutionError::new("LATTICE_MANAGED_SCHEDULER_UNAVAILABLE"))?;
    let task_ref = submission.task_ref().as_str().to_owned();
    match accept_durable_scheduler_task(
        sender,
        &core.managed_tasks,
        &scheduler.rescan_requested,
        &task_ref,
        ManagedScheduledTask {
            submission,
            repository_path,
        },
    )? {
        ManagedDurableEnqueueOutcome::Enqueued | ManagedDurableEnqueueOutcome::AlreadyScheduled => {
        }
        ManagedDurableEnqueueOutcome::DeferredCapacity => {
            scheduler.request_rescan();
        }
    }
    Ok(())
}

fn load_managed_scheduled_task_from_durable(
    source: &ManagedForemanIdentitySource,
    task_ref: &ContentDigest,
) -> Result<ManagedScheduledTask, ToolExecutionError> {
    let submission = load_managed_submission_from_durable(source, task_ref)?;
    let selector = ProjectSelector::new(Some(submission.identity().project_id().as_str()), None)
        .map_err(|error| ToolExecutionError::new(error.code()))?;
    record_observed_effect(ObservedEffectKind::Filesystem)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Process))
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .and_then(|()| record_observed_effect(ObservedEffectKind::Database))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    let resolved = resolve_project_authority(
        &source.database,
        &source.password,
        deadline(source.timeout).map_err(|error| ToolExecutionError::new(error.code()))?,
        &source.store_authority,
        &selector,
    )
    .map_err(|error| ToolExecutionError::new(error.code()))?;
    if &resolved.authority().head() != resolved.current_head()
        || resolved.authority().project_id() != submission.identity().project_id()
        || resolved.authority().project_snapshot_id() != submission.identity().project_snapshot_id()
        || resolved.authority().receipt_digest() != submission.project_authority_receipt_digest()
        || resolved.display_name() != submission.project_display_name()
    {
        return Err(ToolExecutionError::new(
            "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
        ));
    }
    Ok(ManagedScheduledTask {
        submission,
        repository_path: resolved.canonical_path().to_path_buf(),
    })
}

fn load_managed_submission_from_durable(
    source: &ManagedForemanIdentitySource,
    task_ref: &ContentDigest,
) -> Result<TaskSubmissionEnvelope, ToolExecutionError> {
    record_observed_effect(ObservedEffectKind::Database)
        .and_then(|()| record_observed_effect(ObservedEffectKind::Network))
        .map_err(|_| ToolExecutionError::new("LATTICE_MCP_OBSERVED_EFFECT_REJECTED"))?;
    PostgresTaskLifecycle::load_submission_by_task_ref(
        &source.database,
        &source.password,
        deadline(source.timeout).map_err(|error| ToolExecutionError::new(error.code()))?,
        task_ref,
    )
    .map_err(|error| ToolExecutionError::new(error.code()))?
    .ok_or_else(|| ToolExecutionError::new("LATTICE_MANAGED_INTAKE_REPLAY_REQUIRED"))
}

fn refill_managed_scheduler_from_durable(
    config: &ManagedForemanServiceConfig,
    source: &ManagedForemanIdentitySource,
    sender: &mpsc::SyncSender<ManagedScheduledTask>,
    scheduled: &Mutex<BTreeSet<String>>,
    rescan_requested: &AtomicBool,
) -> Result<(), ToolExecutionError> {
    let target =
        ForemanExtensionTarget::new(source.database.database_name(), source.database.run_id())
            .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?;
    let client = connect_fixed_runtime_client(
        &source.database,
        &source.password,
        deadline(source.timeout)
            .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?,
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let mut foreman = PostgresForeman::new(client, &target)
        .map_err(|failure| ToolExecutionError::new(failure.code()))?;
    let identity = reload_managed_foreman_identity(source)?;
    let mut observed = 0_usize;
    let mut capacity_deferred = false;
    let scan = walk_restart_keyset_pages(
        256,
        |cursor, page_limit| {
            foreman
                .list_restart_task_refs_page(cursor, page_limit)
                .map_err(|failure| ToolExecutionError::new(failure.code()))
        },
        |retained| retained.cursor(),
        |retained| {
            observed = observed.saturating_add(1);
            if observed > MANAGED_RESTART_TASK_LIMIT {
                return Err(ToolExecutionError::new(
                    "FOREMAN_RESTART_BACKLOG_LIMIT_EXCEEDED",
                ));
            }
            match retained.restart_kind() {
                RestartTaskKind::DraftPendingPromotion
                | RestartTaskKind::DraftProjectReconciliationRequired
                | RestartTaskKind::PromotedNoAttempt
                | RestartTaskKind::CapacityWait
                | RestartTaskKind::ProjectReconciliationRequired
                | RestartTaskKind::AttemptReconcileRequired
                | RestartTaskKind::WriterReconciliationRequired
                | RestartTaskKind::TerminalPendingVerification
                | RestartTaskKind::VerificationReconcileRequired
                | RestartTaskKind::AttemptClosedPendingRelease => {}
            }
            let mut durable_evidence_ready =
                managed_restart_kind_has_durable_evidence(retained.restart_kind());
            let task_ref = retained.task_ref().as_str().to_owned();
            if capacity_deferred {
                return Ok(());
            }
            if scheduled
                .lock()
                .map_err(|_| ToolExecutionError::new("LATTICE_MANAGED_SCHEDULER_UNAVAILABLE"))?
                .contains(&task_ref)
            {
                return Ok(());
            }
            if managed_restart_kind_requires_project_reconciliation(retained.restart_kind()) {
                let submission = load_managed_submission_from_durable(source, retained.task_ref())?;
                if record_managed_restart_project_blocker(config, &submission)
                    .map_err(|failure| ToolExecutionError::new(failure.code()))?
                    == ManagedRestartProjectBlockerOutcome::Persisted
                {
                    return Ok(());
                }
            }
            let task = match load_managed_scheduled_task_from_durable(source, retained.task_ref()) {
                Ok(task) => task,
                Err(failure) if failure.code() == "PROJECT_REGISTRY_CURRENTNESS_CONFLICT" => {
                    let submission =
                        load_managed_submission_from_durable(source, retained.task_ref())?;
                    record_managed_restart_project_blocker(config, &submission)
                        .map_err(|failure| ToolExecutionError::new(failure.code()))?;
                    return Ok(());
                }
                Err(failure) => {
                    let Some(task) = isolate_managed_restart_dependency(Err(failure))? else {
                        return Ok(());
                    };
                    task
                }
            };
            if retained.restart_kind() == RestartTaskKind::WriterReconciliationRequired {
                match record_managed_restart_writer_blocker(
                    config,
                    &task.submission,
                    &task.repository_path,
                    &identity,
                )
                .map_err(|failure| ToolExecutionError::new(failure.code()))?
                {
                    ManagedRestartWriterBlockerOutcome::Persisted
                    | ManagedRestartWriterBlockerOutcome::NoLongerActive => return Ok(()),
                    ManagedRestartWriterBlockerOutcome::AlreadyCurrent => {}
                    ManagedRestartWriterBlockerOutcome::DurableEvidenceReady => {
                        durable_evidence_ready = true;
                    }
                }
            }
            let Some(status) = isolate_managed_restart_dependency(
                managed_task_public_status(
                    config,
                    task.submission.clone(),
                    &task.repository_path,
                    &identity,
                )
                .map_err(|failure| ToolExecutionError::new(failure.code())),
            )?
            else {
                return Ok(());
            };
            if status.as_ref().is_some_and(|status| {
                managed_restart_status_should_skip(durable_evidence_ready, status)
            }) {
                return Ok(());
            }
            match accept_durable_scheduler_task(
                sender,
                scheduled,
                rescan_requested,
                &task_ref,
                task,
            )? {
                ManagedDurableEnqueueOutcome::Enqueued
                | ManagedDurableEnqueueOutcome::AlreadyScheduled => Ok(()),
                ManagedDurableEnqueueOutcome::DeferredCapacity => {
                    capacity_deferred = true;
                    Ok(())
                }
            }
        },
    );
    scan
}

fn managed_scheduler(
    config: ManagedForemanServiceConfig,
    identity_source: ManagedForemanIdentitySource,
    scheduled: Arc<Mutex<BTreeSet<String>>>,
    initial_tasks: Vec<ManagedScheduledTask>,
) -> Result<ManagedSchedulerOwner, LatticedError> {
    let (sender, receiver) =
        mpsc::sync_channel::<ManagedScheduledTask>(MANAGED_SCHEDULER_QUEUE_CAPACITY);
    let receiver = Arc::new(Mutex::new(receiver));
    let cancellation = ManagedWorkerCancellation::default();
    let rescan_requested = Arc::new(AtomicBool::new(false));
    let next_durable_rescan = Arc::new(Mutex::new(
        Instant::now()
            .checked_add(MANAGED_DURABLE_RESCAN_INTERVAL)
            .unwrap_or_else(Instant::now),
    ));
    let config = config.with_cancellation(cancellation.clone());
    // The complete restart scan has already succeeded before this function is
    // entered. Fill the bounded queue before any supervisor exists, so a slow
    // or corrupt startup scan can never race the periodic durable refill.
    for task in initial_tasks {
        let task_ref = task.submission.task_ref().as_str().to_owned();
        match accept_durable_scheduler_task(&sender, &scheduled, &rescan_requested, &task_ref, task)
            .map_err(|_| LatticedError::new(LatticedErrorKind::ManagedTeardownRejected))?
        {
            ManagedDurableEnqueueOutcome::Enqueued
            | ManagedDurableEnqueueOutcome::AlreadyScheduled => {}
            ManagedDurableEnqueueOutcome::DeferredCapacity => {
                rescan_requested.store(true, AtomicOrdering::Release);
            }
        }
    }
    let mut owner = ManagedSchedulerOwner {
        sender: Some(sender),
        cancellation: cancellation.clone(),
        workers: Vec::with_capacity(MANAGED_SUPERVISOR_WORKERS),
        rescan_requested: Arc::clone(&rescan_requested),
        armed: true,
    };
    for worker in 0..MANAGED_SUPERVISOR_WORKERS {
        let receiver = Arc::clone(&receiver);
        let scheduled = Arc::clone(&scheduled);
        let config = config.clone();
        let identity_source = identity_source.clone();
        let cancellation = cancellation.clone();
        let cancellable_wait = cancellation.clone();
        let requeue_sender = owner
            .sender
            .as_ref()
            .expect("managed scheduler sender")
            .clone();
        let rescan_requested = Arc::clone(&rescan_requested);
        let next_durable_rescan = Arc::clone(&next_durable_rescan);
        let (completion_sender, completion_receiver) = mpsc::sync_channel(1);
        let handle = thread::Builder::new()
            .name(format!("lattice-managed-supervisor-{}", worker + 1))
            .spawn(move || -> Result<(), &'static str> {
                let _completion = ManagedSchedulerCompletion(Some(completion_sender));
                loop {
                    if cancellation.is_requested() {
                        return Ok(());
                    }
                    let task = {
                        let Ok(receiver) = receiver.lock() else {
                            return Err("LATTICE_MANAGED_SCHEDULER_RECEIVER_REJECTED");
                        };
                        match receiver.recv_timeout(MANAGED_CAPACITY_RETRY_DELAY) {
                            Ok(task) => task,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                drop(receiver);
                                let should_rescan = claim_managed_durable_rescan(
                                    &rescan_requested,
                                    &next_durable_rescan,
                                    Instant::now(),
                                )
                                .map_err(ToolExecutionError::code)?;
                                if should_rescan {
                                    if let Err(failure) = refill_managed_scheduler_from_durable(
                                        &config,
                                        &identity_source,
                                        &requeue_sender,
                                        &scheduled,
                                        &rescan_requested,
                                    ) {
                                        if !managed_restart_rescan_is_retryable(failure.code()) {
                                            cancellation.request();
                                            return Err(failure.code());
                                        }
                                    }
                                }
                                continue;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                        }
                    };
                    let task_ref = task.submission.task_ref().as_str().to_owned();
                    let exit = supervise_managed_task(
                        &cancellation,
                        || {
                            reload_managed_foreman_identity(&identity_source)
                                .map_err(ToolExecutionError::code)
                        },
                        |identity| {
                            run_managed_task(
                                &config,
                                task.submission.clone(),
                                &task.repository_path,
                                identity,
                            )
                            .map(|_| ())
                            .map_err(|failure| failure.code())
                        },
                        |identity| {
                            managed_task_public_status(
                                &config,
                                task.submission.clone(),
                                &task.repository_path,
                                identity,
                            )
                            .map_err(|failure| failure.code())
                        },
                        |duration| cancellable_wait.wait_timeout(duration),
                    );
                    release_managed_schedule(&scheduled, &task_ref);
                    match exit {
                        ManagedSupervisorExit::ShutdownFailed(code) => return Err(code),
                        ManagedSupervisorExit::ShutdownComplete
                        | ManagedSupervisorExit::ShutdownIdle => return Ok(()),
                        ManagedSupervisorExit::RunCompleted
                        | ManagedSupervisorExit::DurablyClosed
                        | ManagedSupervisorExit::DurablyDeferred => {
                            if cancellation.is_requested() {
                                return Ok(());
                            }
                        }
                    }
                    let should_rescan = claim_managed_durable_rescan(
                        &rescan_requested,
                        &next_durable_rescan,
                        Instant::now(),
                    )
                    .map_err(ToolExecutionError::code)?;
                    if should_rescan {
                        if let Err(failure) = refill_managed_scheduler_from_durable(
                            &config,
                            &identity_source,
                            &requeue_sender,
                            &scheduled,
                            &rescan_requested,
                        ) {
                            if !managed_restart_rescan_is_retryable(failure.code()) {
                                cancellation.request();
                                return Err(failure.code());
                            }
                        }
                    }
                }
            })
            .map_err(|_| LatticedError::new(LatticedErrorKind::Configuration))?;
        owner.workers.push(ManagedSchedulerWorker {
            completion: completion_receiver,
            handle: Some(handle),
        });
    }
    Ok(owner)
}

fn claim_managed_durable_rescan(
    requested: &AtomicBool,
    next_rescan: &Mutex<Instant>,
    now: Instant,
) -> Result<bool, ToolExecutionError> {
    let explicit = requested.swap(false, AtomicOrdering::AcqRel);
    let mut next = next_rescan
        .lock()
        .map_err(|_| ToolExecutionError::new("LATTICE_MANAGED_SCHEDULER_UNAVAILABLE"))?;
    if !explicit && now < *next {
        return Ok(false);
    }
    *next = now
        .checked_add(MANAGED_DURABLE_RESCAN_INTERVAL)
        .ok_or_else(|| ToolExecutionError::new("LATTICE_MANAGED_SCHEDULER_UNAVAILABLE"))?;
    Ok(true)
}

fn managed_restart_rescan_is_retryable(code: &str) -> bool {
    matches!(
        code,
        "FOREMAN_REPLAY_UNAVAILABLE" | "FOREMAN_ADAPTER_DATABASE_FAILED"
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedSupervisorExit {
    RunCompleted,
    DurablyClosed,
    DurablyDeferred,
    ShutdownIdle,
    ShutdownComplete,
    ShutdownFailed(&'static str),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ManagedRecoveryBackoff {
    exponent: u8,
}

impl ManagedRecoveryBackoff {
    fn next_delay(&mut self) -> Duration {
        let exponent = self.exponent.min(MANAGED_RECOVERY_RETRY_MAX_EXPONENT);
        self.exponent = self
            .exponent
            .saturating_add(1)
            .min(MANAGED_RECOVERY_RETRY_MAX_EXPONENT);
        Duration::from_secs(1_u64 << exponent)
    }
}

fn retain_managed_schedule(
    scheduled: &Mutex<BTreeSet<String>>,
    task_ref: &str,
) -> Result<bool, ToolExecutionError> {
    scheduled
        .lock()
        .map(|mut scheduled| scheduled.insert(task_ref.to_owned()))
        .map_err(|_| ToolExecutionError::new("LATTICE_MANAGED_SCHEDULER_UNAVAILABLE"))
}

fn release_managed_schedule(scheduled: &Mutex<BTreeSet<String>>, task_ref: &str) {
    if let Ok(mut scheduled) = scheduled.lock() {
        scheduled.remove(task_ref);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedDurableEnqueueOutcome {
    Enqueued,
    AlreadyScheduled,
    DeferredCapacity,
}

fn accept_durable_scheduler_task<T>(
    sender: &mpsc::SyncSender<T>,
    scheduled: &Mutex<BTreeSet<String>>,
    rescan_requested: &AtomicBool,
    task_ref: &str,
    task: T,
) -> Result<ManagedDurableEnqueueOutcome, ToolExecutionError> {
    // The general intake already exists in the formal PostgreSQL Task Ledger.
    // Queue pressure is a deferred local observation, not a failed submit: the
    // bounded durable rescan reloads this exact task_ref after capacity drains.
    try_enqueue_durable(sender, scheduled, rescan_requested, task_ref, task)
        .map_err(|()| ToolExecutionError::new("LATTICE_MANAGED_SCHEDULER_UNAVAILABLE"))
}

fn try_enqueue_durable<T>(
    sender: &mpsc::SyncSender<T>,
    scheduled: &Mutex<BTreeSet<String>>,
    rescan_requested: &AtomicBool,
    task_ref: &str,
    task: T,
) -> Result<ManagedDurableEnqueueOutcome, ()> {
    if !retain_managed_schedule(scheduled, task_ref).map_err(|_| ())? {
        return Ok(ManagedDurableEnqueueOutcome::AlreadyScheduled);
    }
    match sender.try_send(task) {
        Ok(()) => Ok(ManagedDurableEnqueueOutcome::Enqueued),
        Err(mpsc::TrySendError::Full(_)) => {
            release_managed_schedule(scheduled, task_ref);
            rescan_requested.store(true, AtomicOrdering::Release);
            Ok(ManagedDurableEnqueueOutcome::DeferredCapacity)
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            release_managed_schedule(scheduled, task_ref);
            Err(())
        }
    }
}

fn supervise_managed_task<I, LoadIdentity, RunTask, LoadStatus, Wait>(
    cancellation: &ManagedWorkerCancellation,
    mut load_identity: LoadIdentity,
    mut run_task: RunTask,
    mut load_status: LoadStatus,
    mut wait: Wait,
) -> ManagedSupervisorExit
where
    LoadIdentity: FnMut() -> Result<I, &'static str>,
    RunTask: FnMut(&I) -> Result<(), &'static str>,
    LoadStatus: FnMut(&I) -> Result<Option<Value>, &'static str>,
    Wait: FnMut(Duration),
{
    let mut recovery_backoff = ManagedRecoveryBackoff::default();
    loop {
        if cancellation.is_requested() {
            return ManagedSupervisorExit::ShutdownIdle;
        }
        let identity = match load_identity() {
            Ok(identity) => identity,
            Err(_) => {
                wait(recovery_backoff.next_delay());
                continue;
            }
        };
        match run_task(&identity) {
            Ok(()) => return ManagedSupervisorExit::RunCompleted,
            Err(MANAGED_GRACEFUL_SHUTDOWN_COMPLETE) if cancellation.is_requested() => {
                return ManagedSupervisorExit::ShutdownComplete;
            }
            Err(MANAGED_GRACEFUL_SHUTDOWN_IDLE) if cancellation.is_requested() => {
                return ManagedSupervisorExit::ShutdownIdle;
            }
            Err(code) if cancellation.is_requested() => {
                return ManagedSupervisorExit::ShutdownFailed(code);
            }
            Err("FOREMAN_GLOBAL_CAPACITY_EXHAUSTED" | "FOREMAN_TASK_CAPACITY_EXHAUSTED") => {
                wait(MANAGED_CAPACITY_RETRY_DELAY)
            }
            // A service error is not terminal evidence. Re-read the exact
            // PostgreSQL-backed managed projection before releasing this
            // process-owned de-duplication key; every unavailable, missing,
            // pending, runnable, or active projection stays in reconciliation.
            Err(code) => match load_status(&identity) {
                Ok(Some(status)) if managed_status_is_durably_closed(&status) => {
                    return ManagedSupervisorExit::DurablyClosed;
                }
                Ok(Some(status)) if managed_status_is_durably_deferred(&status) => {
                    return ManagedSupervisorExit::DurablyDeferred;
                }
                Ok(Some(status)) if managed_status_confirms_dependency_deferred(&status, code) => {
                    return ManagedSupervisorExit::DurablyDeferred;
                }
                Ok(Some(_) | None) | Err(_) => wait(recovery_backoff.next_delay()),
            },
        }
    }
}

fn managed_status_is_durably_closed(status: &Value) -> bool {
    if status.get("schema_version").and_then(Value::as_str) != Some("lattice.task.status.v4") {
        return false;
    }
    let Some(task_state) = status.get("task_state").and_then(Value::as_str) else {
        return false;
    };
    if matches!(
        status
            .get("blocker")
            .and_then(Value::as_str)
            .or_else(|| status.get("failure_code").and_then(Value::as_str)),
        Some("LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED")
    ) {
        return false;
    }
    if task_state == "BLOCKED" && managed_status_requires_exact_provider_reconciliation(status) {
        return false;
    }
    matches!(
        task_state,
        "AWAITING_MERGE_APPROVAL" | "COMPLETED" | "FAILED" | "BLOCKED" | "REJECTED" | "CANCELLED"
    )
}

fn managed_status_has_known_closed_blocker(code: &str) -> bool {
    matches!(
        code,
        "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"
            | "LATTICE_MANAGED_PRESTART_CONFIGURATION_REJECTED"
            | "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED"
            | "LATTICE_MANAGED_REVIEW_MODEL_PROBE_TIMEOUT_NO_PROVIDER_EFFECT"
            | "LATTICE_MANAGED_HEARTBEAT_TIMEOUT_WHILE_IN_PROGRESS"
            | "LATTICE_MANAGED_DEADLINE_EXCEEDED"
            | "LATTICE_MANAGED_REVIEW_TIMEOUT"
            | "LATTICE_MANAGED_MODEL_UNAVAILABLE"
            | "MANAGED_CODEX_MODEL_UNAVAILABLE"
            | "LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED"
            | "LATTICE_MANAGED_VERIFICATION_FAILED"
            | "LATTICE_MANAGED_TOKEN_BUDGET_EXHAUSTED"
            | "LATTICE_MANAGED_REVIEW_TOKEN_BUDGET_EXCEEDED"
            | "LATTICE_MANAGED_MODEL_CALL_BUDGET_EXHAUSTED"
            | "LATTICE_MANAGED_REVIEW_BUDGET_EXHAUSTED"
            | "LATTICE_MANAGED_MODEL_USAGE_RECONCILIATION_REQUIRED"
            | "LATTICE_MANAGED_REVIEW_RESOURCE_OBSERVATION_MISSING"
            | "LATTICE_MANAGED_REVIEW_RESULT_REJECTED"
            | "LATTICE_MANAGED_REVIEW_FINAL_REJECTED"
            | "LATTICE_MANAGED_REVIEW_FINAL_DIGEST_MISMATCH"
            | "LATTICE_MANAGED_REVIEW_OUTPUT_REJECTED"
            | "LATTICE_MANAGED_REVIEW_IDENTITY_MISMATCH"
            | "LATTICE_MANAGED_REVIEW_LIFECYCLE_REJECTED"
            | "LATTICE_MANAGED_REVIEW_EVIDENCE_REJECTED"
            | "LATTICE_MANAGED_REVIEW_RESOURCE_REJECTED"
            | "LATTICE_MANAGED_REVIEW_RESULT_LIMIT"
            | "LATTICE_MANAGED_REVIEW_CONFIG_REJECTED"
            | "LATTICE_MANAGED_REVIEW_SUBJECT_REJECTED"
            | "LATTICE_MANAGED_REVIEW_PROMPT_REJECTED"
            | "LATTICE_MANAGED_REVIEW_PATH_REJECTED"
            | "LATTICE_MANAGED_REVIEW_DIGEST_FAILED"
            | "LATTICE_MANAGED_REPOSITORY_LINEAGE_MISMATCH"
            | "LATTICE_MANAGED_WORKTREE_NOT_CLEAN"
            | "LATTICE_MANAGED_BASE_COMMIT_DRIFT"
            | "LATTICE_MANAGED_DISPATCH_BASE_COMMIT_DRIFT"
            | "LATTICE_MANAGED_WORKTREE_BASELINE_REQUIRED"
            | "LATTICE_MANAGED_WORKTREE_BASELINE_REPLAY_REJECTED"
            | "LATTICE_MANAGED_WORKTREE_BASELINE_DRIFT"
            | "LATTICE_MANAGED_WORKTREE_CONTROL_DRIFT"
            | "LATTICE_MANAGED_PROTECTED_REF_REJECTED"
    )
}

const fn managed_restart_kind_has_durable_evidence(kind: RestartTaskKind) -> bool {
    matches!(
        kind,
        RestartTaskKind::AttemptClosedPendingRelease
            | RestartTaskKind::VerificationReconcileRequired
            | RestartTaskKind::TerminalPendingVerification
    )
}

const fn managed_restart_kind_requires_project_reconciliation(kind: RestartTaskKind) -> bool {
    matches!(
        kind,
        RestartTaskKind::DraftProjectReconciliationRequired
            | RestartTaskKind::ProjectReconciliationRequired
    )
}

fn managed_restart_status_should_skip(durable_evidence_ready: bool, status: &Value) -> bool {
    managed_status_is_durably_closed(status)
        || (!durable_evidence_ready && managed_status_is_durably_deferred(status))
}

fn managed_status_requires_exact_provider_reconciliation(status: &Value) -> bool {
    matches!(
        status
            .get("blocker")
            .and_then(Value::as_str)
            .or_else(|| status.get("failure_code").and_then(Value::as_str)),
        Some(
            "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"
                | "LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED"
                | "LATTICE_MANAGED_BRIDGE_HEARTBEAT_TIMEOUT_RECONCILIATION_REQUIRED"
                | "LATTICE_MANAGED_THREAD_START_RPC_INVALID_PARAMS"
                | "LATTICE_MANAGED_THREAD_START_RPC_REJECTED"
                | "LATTICE_MANAGED_TURN_START_RPC_INVALID_PARAMS"
                | "LATTICE_MANAGED_TURN_START_RPC_REJECTED"
                | "LATTICE_MANAGED_REVIEW_MODEL_UNAVAILABLE"
                | "LATTICE_MANAGED_REVIEW_THREAD_START_RPC_INVALID_PARAMS"
                | "LATTICE_MANAGED_REVIEW_THREAD_START_RPC_REJECTED"
                | "LATTICE_MANAGED_REVIEW_TURN_START_RPC_INVALID_PARAMS"
                | "LATTICE_MANAGED_REVIEW_TURN_START_RPC_REJECTED"
                | "LATTICE_MANAGED_REVIEW_RECONCILIATION_REQUIRED"
        )
    )
}

fn managed_status_is_durably_deferred(status: &Value) -> bool {
    if status.get("schema_version").and_then(Value::as_str) != Some("lattice.task.status.v4") {
        return false;
    }
    let task_state = status.get("task_state").and_then(Value::as_str);
    let blocker = status
        .get("blocker")
        .and_then(Value::as_str)
        .or_else(|| status.get("failure_code").and_then(Value::as_str));
    (task_state == Some("AWAITING_EXECUTION_APPROVAL")
        && blocker == Some("LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED"))
        || (matches!(task_state, Some("PREPARING" | "EXECUTING"))
            && blocker == Some("LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED"))
}

fn managed_dependency_not_ready(code: &str) -> bool {
    matches!(
        code,
        "LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED"
            | "LATTICE_MANAGED_WORKTREE_NOT_CLEAN"
            | "PROJECT_REGISTRY_CURRENTNESS_CONFLICT"
            | "LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED"
    )
}

fn managed_status_confirms_dependency_deferred(status: &Value, code: &str) -> bool {
    status.get("schema_version").and_then(Value::as_str) == Some("lattice.task.status.v4")
        && managed_dependency_not_ready(code)
        && (status.get("blocker").and_then(Value::as_str) == Some(code)
            || status.get("failure_code").and_then(Value::as_str) == Some(code))
}

fn isolate_managed_restart_dependency<T>(
    result: Result<T, ToolExecutionError>,
) -> Result<Option<T>, ToolExecutionError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(failure)
            if failure.code() != "PROJECT_REGISTRY_CURRENTNESS_CONFLICT"
                && managed_dependency_not_ready(failure.code()) =>
        {
            Ok(None)
        }
        Err(failure) => Err(failure),
    }
}

fn walk_restart_keyset_pages<T, Cursor, Fetch, CursorFor, Handle>(
    page_limit: u16,
    mut fetch_page: Fetch,
    mut cursor_for: CursorFor,
    mut handle: Handle,
) -> Result<(), ToolExecutionError>
where
    Cursor: Clone + Ord,
    Fetch: FnMut(Option<&Cursor>, u16) -> Result<Vec<T>, ToolExecutionError>,
    CursorFor: FnMut(&T) -> Cursor,
    Handle: FnMut(T) -> Result<(), ToolExecutionError>,
{
    if page_limit == 0 || page_limit > 256 {
        return Err(ToolExecutionError::new(
            "FOREMAN_RESTART_PAGE_LIMIT_INVALID",
        ));
    }
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    loop {
        let page = fetch_page(cursor.as_ref(), page_limit)?;
        let page_len = page.len();
        if page_len > usize::from(page_limit) {
            return Err(ToolExecutionError::new(
                "FOREMAN_RESTART_PAGE_LIMIT_EXCEEDED",
            ));
        }
        if page.is_empty() {
            return Ok(());
        }

        let mut last = cursor.clone();
        for item in page {
            let next = cursor_for(&item);
            if last.as_ref().is_some_and(|previous| next <= *previous) || !seen.insert(next.clone())
            {
                return Err(ToolExecutionError::new("FOREMAN_RESTART_SCAN_NOT_STRICT"));
            }
            handle(item)?;
            last = Some(next);
        }
        cursor = last;
        if page_len < usize::from(page_limit) {
            return Ok(());
        }
    }
}

fn stage_managed_restart_tasks<H: FullChainHermesPort>(
    core: &FullChainCore<H>,
) -> Result<Vec<ManagedScheduledTask>, ToolExecutionError> {
    if core.managed_foreman.is_none() {
        return Ok(Vec::new());
    }
    let managed_config = core
        .managed_foreman
        .as_ref()
        .ok_or_else(|| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let target = ForemanExtensionTarget::new(
        core.delivery.database.database_name(),
        core.delivery.database.run_id(),
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_CORRUPT"))?;
    let client = connect_fixed_runtime_client(
        &core.delivery.database,
        &core.delivery.password,
        deadline(core.delivery.timeout)
            .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?,
    )
    .map_err(|_| ToolExecutionError::new("FOREMAN_REPLAY_UNAVAILABLE"))?;
    let mut foreman = PostgresForeman::new(client, &target)
        .map_err(|failure| ToolExecutionError::new(failure.code()))?;
    let foreman_identity = formal_managed_foreman_identity(core)?;
    let mut retained_tasks = Vec::new();
    walk_restart_keyset_pages(
        256,
        |cursor, page_limit| {
            foreman
                .list_restart_task_refs_page(cursor, page_limit)
                .map_err(|failure| ToolExecutionError::new(failure.code()))
        },
        |retained| retained.cursor(),
        |retained| {
            let submission = load_general_submission_by_task_ref(core, retained.task_ref())?
                .ok_or_else(|| ToolExecutionError::new("LATTICE_MANAGED_INTAKE_REPLAY_REQUIRED"))?;
            if managed_restart_kind_requires_project_reconciliation(retained.restart_kind()) {
                if record_managed_restart_project_blocker(managed_config, &submission)
                    .map_err(|failure| ToolExecutionError::new(failure.code()))?
                    == ManagedRestartProjectBlockerOutcome::Persisted
                {
                    return Ok(());
                }
            }
            let resolved =
                match resolve_registered_project_for_general_submission(core, &submission) {
                    Ok(resolved) => resolved,
                    Err(failure) if failure.code() == "PROJECT_REGISTRY_CURRENTNESS_CONFLICT" => {
                        record_managed_restart_project_blocker(managed_config, &submission)
                            .map_err(|failure| ToolExecutionError::new(failure.code()))?;
                        return Ok(());
                    }
                    Err(failure) => {
                        let Some(resolved) = isolate_managed_restart_dependency(Err(failure))?
                        else {
                            return Ok(());
                        };
                        resolved
                    }
                };
            if retained.restart_kind() == RestartTaskKind::WriterReconciliationRequired {
                match record_managed_restart_writer_blocker(
                    managed_config,
                    &submission,
                    resolved.canonical_path(),
                    &foreman_identity,
                )
                .map_err(|failure| ToolExecutionError::new(failure.code()))?
                {
                    ManagedRestartWriterBlockerOutcome::Persisted
                    | ManagedRestartWriterBlockerOutcome::NoLongerActive => return Ok(()),
                    ManagedRestartWriterBlockerOutcome::AlreadyCurrent
                    | ManagedRestartWriterBlockerOutcome::DurableEvidenceReady => {}
                }
            }
            push_managed_restart_task(
                &mut retained_tasks,
                ManagedScheduledTask {
                    submission,
                    repository_path: resolved.canonical_path().to_path_buf(),
                },
                MANAGED_RESTART_TASK_LIMIT,
            )
        },
    )?;
    // The caller starts supervisors only after this complete bounded scan has
    // returned. A corrupt later row therefore leaves the effect count at zero.
    Ok(retained_tasks)
}

fn push_managed_restart_task<T>(
    retained_tasks: &mut Vec<T>,
    task: T,
    limit: usize,
) -> Result<(), ToolExecutionError> {
    if limit == 0 || retained_tasks.len() >= limit {
        return Err(ToolExecutionError::new(
            "FOREMAN_RESTART_BACKLOG_LIMIT_EXCEEDED",
        ));
    }
    retained_tasks.push(task);
    Ok(())
}

fn start_after_complete_stage<T, U, E, Stage, Start>(stage: Stage, start: Start) -> Result<U, E>
where
    Stage: FnOnce() -> Result<T, E>,
    Start: FnOnce(T) -> Result<U, E>,
{
    let staged = stage()?;
    start(staged)
}

const fn general_task_error_code(error: &GeneralTaskIntakeError) -> &'static str {
    match error {
        GeneralTaskIntakeError::RequestRejected => "LATTICE_TASK_REQUEST_REJECTED",
        GeneralTaskIntakeError::Lifecycle(error) => error.code(),
        GeneralTaskIntakeError::StateMismatch => "LATTICE_TASK_STATE_MISMATCH",
    }
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

fn task_public_status(
    evidence: &TaskLifecycleEvidence,
    task_ref: &ContentDigest,
    failure_stage: Option<&str>,
    failure_code: Option<&str>,
) -> Value {
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
        "failure_code": failure_code,
        "failure_stage": failure_stage,
        "result_digest": evidence.result_digest().map(ContentDigest::as_str),
        "schema_version": "lattice.task.status.v2",
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
    verified_task_status_at(core, evidence, task_ref, None)
}

fn verified_task_status_at<H: FullChainHermesPort>(
    core: &mut FullChainCore<H>,
    evidence: &TaskLifecycleEvidence,
    task_ref: &ContentDigest,
    operation_deadline: Option<Instant>,
) -> Result<Value, LatticedError> {
    let mut failure_stage = None;
    let mut failure_code = None;
    if evidence.state() == TaskState::Completed {
        let mut lifecycle = match operation_deadline {
            Some(operation_deadline) => {
                task_lifecycle_at(core, evidence.binding(), operation_deadline)
            }
            None => task_lifecycle(core, evidence.binding()),
        }
        .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?;
        let foundation = lifecycle
            .persistence_foundation(evidence.binding())
            .map_err(|_| LatticedError::new(LatticedErrorKind::TaskControl))?;
        let mut writer_lease = match operation_deadline {
            Some(operation_deadline) => {
                task_writer_lease_at(core, &foundation, operation_deadline)?
            }
            None => task_writer_lease(core, &foundation)?,
        };
        verify_completed_writer_history(&mut writer_lease, evidence)?;
        let expected = evidence
            .result_digest()
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::ReceiptMismatch))?;
        let receipt = match operation_deadline {
            Some(operation_deadline) => {
                core.status_task_json_at(evidence.binding(), operation_deadline)?
            }
            None => core.status_task_json(evidence.binding())?,
        };
        if &delivery_receipt_digest(&receipt)? != expected {
            return Err(LatticedError::new(LatticedErrorKind::ReceiptMismatch));
        }
    } else if evidence.state() == TaskState::Failed {
        // Failures before Delivery have no durable delivery receipt. When a
        // receipt exists, expose only its closed, payload-free stage and code.
        let receipt = match operation_deadline {
            Some(operation_deadline) => {
                core.status_task_json_at(evidence.binding(), operation_deadline)
            }
            None => core.status_task_json(evidence.binding()),
        };
        if let Ok(receipt) = receipt {
            if let Some((stage, code)) = delivery_failure_projection(&receipt) {
                failure_stage = Some(stage);
                failure_code = Some(code);
            }
        }
    }
    if operation_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(LatticedError::new(LatticedErrorKind::DatabaseConnect));
    }
    Ok(task_public_status(
        evidence,
        task_ref,
        failure_stage.as_deref(),
        failure_code.as_deref(),
    ))
}

fn delivery_failure_projection(receipt: &Value) -> Option<(String, String)> {
    let object = receipt.as_object()?;
    let stage = object.get("failure_stage")?.as_str()?;
    let code = object.get("failure_code")?.as_str()?;
    if valid_public_failure_atom(stage) && valid_public_failure_atom(code) {
        Some((stage.to_owned(), code.to_owned()))
    } else {
        None
    }
}

fn valid_public_failure_atom(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
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
    managed_status_timeout: Option<Duration>,
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
            managed_status_timeout: self.managed_status_timeout,
        }
    }
}

fn lock_task_status_core_until<T>(
    core: &Mutex<T>,
    operation_deadline: Option<Instant>,
) -> Result<MutexGuard<'_, T>, ToolExecutionError> {
    let Some(operation_deadline) = operation_deadline else {
        return core
            .lock()
            .map_err(|_| ToolExecutionError::new(LatticedErrorKind::Transport.code()));
    };
    loop {
        match core.try_lock() {
            Ok(core) => return Ok(core),
            Err(TryLockError::Poisoned(_)) => {
                return Err(ToolExecutionError::new(LatticedErrorKind::Transport.code()));
            }
            Err(TryLockError::WouldBlock) => {
                let now = Instant::now();
                if now >= operation_deadline {
                    return Err(ToolExecutionError::new(MANAGED_STATUS_TIMEOUT));
                }
                thread::sleep(
                    operation_deadline
                        .saturating_duration_since(now)
                        .min(Duration::from_millis(2)),
                );
            }
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
        let managed_result = core
            .managed_scheduler
            .as_mut()
            .map_or(Ok(()), ManagedSchedulerOwner::shutdown);
        let hermes_result = finish_hermes_owner(serve_result, &mut core.hermes);
        match managed_result {
            Ok(()) => hermes_result,
            Err(managed) => Err(managed),
        }
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
                TaskLifecyclePort::load(&mut lifecycle, &binding)
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
        let preexisting = TaskLifecyclePort::load(&mut lifecycle, &binding)
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
                submission,
                task_identity,
            };
            run_controlled_task(&request, &mut lifecycle, &mut writer_lease, &mut execution)
        };
        match outcome {
            Ok(evidence) => Ok(evidence),
            Err(error @ ControlledTaskOrchestratorError::Execution(_)) => {
                let evidence = TaskLifecyclePort::load(&mut lifecycle, &binding)
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
                let evidence = TaskLifecyclePort::load(&mut lifecycle, &binding)
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
        let task_evidence = TaskLifecyclePort::load(&mut lifecycle, &fixed_binding)
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
        let (evidence, historical_terminal) =
            match TaskLifecyclePort::load(&mut lifecycle, &binding) {
                Ok(evidence) => (evidence, false),
                Err(error)
                    if error.code() == "LATTICE_TASK_INGRESS_PROFILE_COMMITMENT_MISMATCH" =>
                {
                    lifecycle
                        .load_historical_terminal_status(&binding)
                        .map(|(evidence, _)| evidence)
                        .map(|evidence| (evidence, true))
                        .map_err(|error| ToolExecutionError::new(error.code()))?
                }
                Err(error) => return Err(ToolExecutionError::new(error.code())),
            };
        if evidence.admitted() {
            if historical_terminal {
                return core
                    .delivery
                    .historical_terminal_status_json(&binding)
                    .map_err(|error| ToolExecutionError::new(error.code()));
            }
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

    fn foreman_checkpoint(
        &mut self,
        arguments: &ForemanCheckpointArguments,
    ) -> Result<Value, ToolExecutionError> {
        let core = self
            .inner
            .lock()
            .map_err(|_| ToolExecutionError::new(LatticedErrorKind::Transport.code()))?;
        let mut coordination = foreman_coordination(&core)?;
        // Task-Ledger-owned preflight must complete before constructing or
        // observing the Writer repository and before the Git probe.
        let _replay_preflight = coordination
            .replay_checkpoint(arguments.intent())
            .map_err(|error| ToolExecutionError::new(error.code()))?;

        let mut writer = foreman_writer_lease(&core)?;
        let acquire = foreman_writer_acquire(&core, arguments.intent().checkpoint_id())?;
        let receipt = checkpoint_foreman(
            &mut coordination,
            &mut writer,
            arguments.intent(),
            acquire,
            || {
                let mut latest = foreman_coordination(&core).map_err(ToolExecutionError::code)?;
                match validate_dependency_checkpoint(&mut latest, arguments.intent())? {
                    Some(observation) => Ok(observation),
                    None => foreman_observation_from_environment(),
                }
            },
        )
        .map_err(|error| ToolExecutionError::new(foreman_error_code(&error)))?;
        Ok(json!({
            "schema": "lattice.foreman-checkpoint-result/1.0",
            "checkpoint_id": arguments.intent().checkpoint_id(),
            "generation": receipt.generation(),
            "status": if receipt.is_exact_retry() { "REPLAYED" } else { "RECORDED" },
            "exact_retry": receipt.is_exact_retry(),
            "ledger_digest": receipt.ledger_digest().as_str(),
            "checkpoint_digest": receipt.checkpoint_digest().as_str(),
        }))
    }

    fn reconcile(
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
        core.delivery
            .reconcile_json()
            .map_err(|error| ToolExecutionError::new(error.code()))
    }

    // Admission, replay, and currentness checks intentionally remain in one
    // visible fail-closed sequence so later effects cannot be reordered around
    // an idempotency winner.
    #[allow(clippy::too_many_lines)]
    fn task_submit(
        &mut self,
        arguments: &TaskSubmitArguments,
    ) -> Result<Value, ToolExecutionError> {
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
        if let Some(input) = arguments.verified_result_adoption() {
            let digest = |value: &str| {
                ContentDigest::from_sha256(value)
                    .map_err(|_| ToolExecutionError::new("LATTICE_TASK_REFERENCE_REJECTED"))
            };
            let adoption = lattice_task_ledger::ExternalVerifiedResultAdoption::new(
                digest(input.task_ref())?,
                arguments.client_request_id(),
                digest(input.expected_ledger_head_digest())?,
                input.source_sha(),
                input.target_sha(),
                input.push_merge_receipt_ref(),
                input.deployment_receipt_ref(),
                input.deployment_artifact_ref(),
                input.independent_acceptance_ref(),
                input.protected_action_approval_refs().to_vec(),
            )
            .map_err(|_| ToolExecutionError::new("LATTICE_EXTERNAL_RESULT_REJECTED"))?;
            let submission = load_general_submission_by_task_ref(&core, adoption.task_ref())?
                .ok_or_else(|| ToolExecutionError::new("LATTICE_TASK_REFERENCE_NOT_FOUND"))?;
            let mut lifecycle = general_task_lifecycle(&core, &submission)
                .map_err(|failure| ToolExecutionError::new(failure.code()))?;
            let occurred_at = time::OffsetDateTime::now_utc()
                .replace_nanosecond(0)
                .map_err(|_| ToolExecutionError::new("LATTICE_EXTERNAL_RESULT_REJECTED"))?
                .format(&time::format_description::well_known::Rfc3339)
                .map_err(|_| ToolExecutionError::new("LATTICE_EXTERNAL_RESULT_REJECTED"))?;
            let evidence = lifecycle
                .adopt_external_result(&adoption, &occurred_at)
                .map_err(|failure| ToolExecutionError::new(failure.code()))?;
            return general_task_public_status(&evidence, &submission)
                .map_err(|failure| ToolExecutionError::new(failure.code()));
        }
        let existing_general =
            load_general_submission_by_request(&core, arguments.client_request_id())?;
        if arguments.is_controlled_canary() && existing_general.is_some() {
            return Err(ToolExecutionError::new("LATTICE_TASK_IDEMPOTENCY_CONFLICT"));
        }
        if !arguments.is_controlled_canary() {
            if let Some(existing) = existing_general {
                return replay_general_submission_and_schedule(&core, arguments, &existing);
            }

            let retained_request_kind =
                load_task_ingress_request_kind_by_request(&core, arguments.client_request_id())?;
            let raced_submission =
                general_submission_after_ingress_preflight(retained_request_kind, || {
                    load_general_submission_by_request(&core, arguments.client_request_id())
                })?;
            if let Some(existing) = raced_submission {
                return replay_general_submission_and_schedule(&core, arguments, &existing);
            }
            let resolved = resolve_registered_project_for_general_submit(&core, arguments)?;
            let effective_project_id = resolved.authority().project_id().clone();
            match admit_general_submission(&core, arguments, &resolved) {
                Ok(value) => {
                    let submission =
                        load_general_submission_by_request(&core, arguments.client_request_id())?
                            .ok_or_else(|| ToolExecutionError::new("LATTICE_TASK_LEDGER_CORRUPT"))?;
                    schedule_managed_general_task(
                        &core,
                        submission,
                        resolved.canonical_path().to_path_buf(),
                    )?;
                    return Ok(value);
                }
                Err(error)
                    if matches!(
                        error.code(),
                        "LATTICE_TASK_IDEMPOTENCY_CONFLICT"
                            | "PROJECT_REGISTRY_CURRENTNESS_CONFLICT"
                    ) =>
                {
                    match replay_general_winner_after_admission_failure(
                        &core,
                        arguments,
                        &effective_project_id,
                    )? {
                        GeneralWinnerReplay::Replayed(value) => return Ok(value),
                        GeneralWinnerReplay::Conflict => {
                            return Err(ToolExecutionError::new(
                                "LATTICE_TASK_IDEMPOTENCY_CONFLICT",
                            ));
                        }
                        GeneralWinnerReplay::Absent => {}
                    }

                    if error.code() != "PROJECT_REGISTRY_CURRENTNESS_CONFLICT" {
                        return Err(error);
                    }

                    let refreshed =
                        resolve_registered_project_for_general_submit(&core, arguments)?;
                    if refreshed.authority().project_id() != &effective_project_id {
                        return Err(error);
                    }
                    match admit_general_submission(&core, arguments, &refreshed) {
                        Ok(value) => {
                            let submission = load_general_submission_by_request(
                                &core,
                                arguments.client_request_id(),
                            )?
                            .ok_or_else(|| {
                                ToolExecutionError::new("LATTICE_TASK_LEDGER_CORRUPT")
                            })?;
                            schedule_managed_general_task(
                                &core,
                                submission,
                                refreshed.canonical_path().to_path_buf(),
                            )?;
                            return Ok(value);
                        }
                        Err(retry_error)
                            if matches!(
                                retry_error.code(),
                                "LATTICE_TASK_IDEMPOTENCY_CONFLICT"
                                    | "PROJECT_REGISTRY_CURRENTNESS_CONFLICT"
                            ) =>
                        {
                            return match replay_general_winner_after_admission_failure(
                                &core,
                                arguments,
                                &effective_project_id,
                            )? {
                                GeneralWinnerReplay::Replayed(value) => Ok(value),
                                GeneralWinnerReplay::Conflict => Err(ToolExecutionError::new(
                                    "LATTICE_TASK_IDEMPOTENCY_CONFLICT",
                                )),
                                GeneralWinnerReplay::Absent => Err(retry_error),
                            };
                        }
                        Err(retry_error) => return Err(retry_error),
                    }
                }
                Err(error) => return Err(error),
            }
        }

        let submission = mcp_gateway_submission(arguments.client_request_id())
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        if core.run_mode == FullChainRunMode::ResumeExisting {
            let binding = submission.binding().clone();
            let mut lifecycle = task_lifecycle(&core, &binding)
                .map_err(|error| ToolExecutionError::new(error.code()))?;
            let evidence = TaskLifecyclePort::load(&mut lifecycle, &binding)
                .map_err(|error| ToolExecutionError::new(error.code()))?;
            if evidence.admitted() && evidence.state() == TaskState::Failed {
                let task_ref = verified_controlled_task_reference(&core, &binding)
                    .map_err(|error| ToolExecutionError::new(error.code()))?;
                return verified_task_status(&mut core, &evidence, &task_ref)
                    .map_err(|error| ToolExecutionError::new(error.code()));
            }
        }
        let existing_completion_policy = match core.run_mode {
            FullChainRunMode::Fresh => ExistingCompletionPolicy::Ignore,
            FullChainRunMode::ResumeExisting => ExistingCompletionPolicy::Require,
        };
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

    fn control_snapshot(
        &mut self,
        arguments: &ControlSnapshotArguments,
    ) -> Result<Value, ToolExecutionError> {
        let mut core = self
            .inner
            .lock()
            .map_err(|_| ToolExecutionError::new("CONTROL_PRODUCT_UNAVAILABLE"))?;
        let run_id = core.delivery.database.run_id().to_owned();
        apply_canonical_hermes_tool_policy(
            &mut core.hermes,
            &run_id,
            CanonicalHermesTool::TaskStatus,
        )?;
        let mut product = connect_control_product(&core)?;
        if let Some(query) = &arguments.decisions {
            let result = query.read(&mut product).map_err(ToolExecutionError::new)?;
            let project_id = query
                .scope()
                .or_else(|| result["decision"]["scope"].as_str())
                .ok_or_else(|| ToolExecutionError::new("CONTROL_PRODUCT_RESPONSE_REJECTED"))?;
            control_product_project(&core, project_id)?;
            return Ok(result);
        }
        let refs = match &arguments.task_ref {
            Some(task_ref) => vec![task_ref.as_str().to_owned()],
            None => product
                .task_refs(&arguments.project_id, &arguments.after_task_ref, 32)
                .map_err(ToolExecutionError::new)?,
        };
        let project = control_product_project(&core, &arguments.project_id)?;
        let status_config = core
            .managed_foreman
            .as_ref()
            .map(|config| config.begin_status_request_at(Instant::now()))
            .transpose()
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let mut tasks = Vec::with_capacity(refs.len());
        for task_ref in &refs {
            let reference = ContentDigest::from_sha256(task_ref)
                .map_err(|_| ToolExecutionError::new("CONTROL_PRODUCT_REFERENCE_REJECTED"))?;
            let submission = load_general_submission_by_task_ref(&core, &reference)?
                .ok_or_else(|| ToolExecutionError::new("CONTROL_PRODUCT_TASK_MISSING"))?;
            if submission.identity().project_id().as_str() != arguments.project_id {
                return Err(ToolExecutionError::new("CONTROL_PRODUCT_SCOPE_REJECTED"));
            }
            let binding = general_task_binding(&submission)
                .map_err(|error| ToolExecutionError::new(error.code()))?;
            let mut lifecycle = general_task_lifecycle(&core, &submission)
                .map_err(|error| ToolExecutionError::new(error.code()))?;
            let evidence = TaskIntakeLifecyclePort::load(&mut lifecycle, &binding)
                .map_err(|error| ToolExecutionError::new(error.code()))?;
            let status = managed_general_task_public_status(
                &core,
                &evidence,
                &submission,
                status_config.as_ref(),
            )?;
            tasks.push(json!({
                "task_ref":task_ref,"client_request_id":submission.client_request_id(),
                "objective":submission.objective(),"ledger":status,
            }));
        }
        let mut facts = product
            .snapshot(&arguments.project_id, &refs)
            .map_err(ToolExecutionError::new)?;
        crate::control_product::bound_snapshot(&mut facts, arguments.task_ref.is_some());
        if let (Some(task_ref), Some(question_id)) = (&arguments.task_ref, &arguments.question_id) {
            facts["question_resolution"] = product
                .question_resolution(&arguments.project_id, task_ref.as_str(), question_id)
                .map_err(ToolExecutionError::new)?;
        }
        let mut response = json!({
            "schema_version":"lattice.control.product-snapshot.v1",
            "source":{"kind":"POSTGRESQL_CONTROL_PRODUCT","authority":"POSTGRESQL_TASK_LEDGER"},
            "project":project,"tasks":tasks,"product":facts,
            "next_task_ref":if arguments.task_ref.is_none()&&refs.len()==32 {refs.last().cloned()} else {None},
        });
        if response.to_string().len() > 500_000 {
            return Err(ToolExecutionError::new(
                "CONTROL_PRODUCT_RESPONSE_LIMIT_EXCEEDED",
            ));
        }
        let revision = digest(
            "lattice.control.product-snapshot.v1",
            &CanonicalValue::String(response.to_string()),
        )
        .map_err(|_| ToolExecutionError::new("CONTROL_PRODUCT_RESPONSE_REJECTED"))?;
        response["revision"] = json!(revision.as_str());
        Ok(response)
    }

    fn control_update(
        &mut self,
        arguments: &ControlUpdateArguments,
    ) -> Result<Value, ToolExecutionError> {
        let mut core = self
            .inner
            .lock()
            .map_err(|_| ToolExecutionError::new("CONTROL_PRODUCT_UNAVAILABLE"))?;
        let run_id = core.delivery.database.run_id().to_owned();
        apply_canonical_hermes_tool_policy(
            &mut core.hermes,
            &run_id,
            CanonicalHermesTool::TaskSubmit,
        )?;
        let mut product = connect_control_product(&core)?;
        let mut repository_path = None;
        let command = match arguments {
            ControlUpdateArguments::Claim {
                task_ref,
                claim_id,
                phase,
                prompt,
            } => {
                let submission = load_general_submission_by_task_ref(&core, task_ref)?
                    .ok_or_else(|| ToolExecutionError::new("CONTROL_PRODUCT_TASK_MISSING"))?;
                let facts = product
                    .snapshot(
                        submission.identity().project_id().as_str(),
                        &[task_ref.as_str().to_owned()],
                    )
                    .map_err(ToolExecutionError::new)?;
                let claims = facts["claims"]
                    .as_array()
                    .ok_or_else(|| ToolExecutionError::new("CONTROL_PRODUCT_RESPONSE_REJECTED"))?;
                let existing = claims
                    .iter()
                    .find(|claim| claim["phase"].as_str() == Some(phase));
                let project = if existing.is_some() {
                    load_registered_project_for_general_status(
                        &core,
                        &submission,
                        deadline(core.delivery.timeout)
                            .map_err(|e| ToolExecutionError::new(e.code()))?,
                    )?
                    .canonical_path
                } else {
                    resolve_registered_project_for_general_submission(&core, &submission)?
                        .canonical_path()
                        .to_owned()
                };
                repository_path = Some(project);
                let execution = claims.iter().find(|claim| claim["phase"] == "EXECUTION");
                let retained_workspace = existing
                    .or(execution)
                    .and_then(|claim| claim["worktree_path"].as_str());
                let workspace = match retained_workspace {
                    Some(path) => PathBuf::from(path),
                    None => {
                        let root = core.delivery.delivery.as_ref().ok_or_else(|| {
                            ToolExecutionError::new("CONTROL_PRODUCT_WORKSPACE_UNAVAILABLE")
                        })?;
                        root.delivery_root
                            .join("control-worktrees")
                            .join(task_ref.as_str())
                    }
                };
                if !workspace.is_absolute() {
                    return Err(ToolExecutionError::new(
                        "CONTROL_PRODUCT_WORKSPACE_REJECTED",
                    ));
                }
                ControlProductCommand::Claim {
                    task_ref: task_ref.clone(),
                    claim_id: claim_id.clone(),
                    phase: phase.clone(),
                    prompt: prompt.clone(),
                    model: existing
                        .and_then(|claim| claim["model"].as_str())
                        .unwrap_or("gpt-6-astra")
                        .to_owned(),
                    worktree_path: workspace
                        .to_str()
                        .ok_or_else(|| {
                            ToolExecutionError::new("CONTROL_PRODUCT_WORKSPACE_REJECTED")
                        })?
                        .to_owned(),
                }
            }
            ControlUpdateArguments::Command(command) => {
                match command {
                    ControlProductCommand::Metadata { task_ref, .. } => {
                        load_general_submission_by_task_ref(&core, task_ref)?.ok_or_else(|| {
                            ToolExecutionError::new("CONTROL_PRODUCT_TASK_MISSING")
                        })?;
                    }
                    ControlProductCommand::Observe {
                        task_ref,
                        claim_id,
                        kind,
                        ..
                    } => {
                        let submission = load_general_submission_by_task_ref(&core, task_ref)?
                            .ok_or_else(|| {
                                ToolExecutionError::new("CONTROL_PRODUCT_TASK_MISSING")
                            })?;
                        let facts = product
                            .snapshot(
                                submission.identity().project_id().as_str(),
                                &[task_ref.as_str().to_owned()],
                            )
                            .map_err(ToolExecutionError::new)?;
                        if !facts["claims"].as_array().is_some_and(|claims| {
                            claims
                                .iter()
                                .any(|claim| claim["claim_id"].as_str() == Some(claim_id))
                        }) {
                            return Err(ToolExecutionError::new("CONTROL_PRODUCT_SCOPE_REJECTED"));
                        }
                        if kind == "DISPATCH_STARTED" {
                            resolve_registered_project_for_general_submission(&core, &submission)?;
                        }
                    }
                    ControlProductCommand::Decision { project_id, .. } => {
                        control_product_project(&core, project_id)?;
                    }
                    ControlProductCommand::Claim { .. } => {
                        return Err(ToolExecutionError::new("CONTROL_PRODUCT_INPUT_REJECTED"));
                    }
                }
                command.clone()
            }
        };
        let mut result = product.execute(&command).map_err(ToolExecutionError::new)?;
        if matches!(&command, ControlProductCommand::Decision { .. }) {
            return Ok(result);
        }
        if let Some(path) = repository_path {
            result["repository_path"] = json!(path);
        }
        Ok(json!({"schema_version":"lattice.control.product-update.v1","record":result}))
    }

    fn task_status(
        &mut self,
        arguments: &TaskStatusArguments,
    ) -> Result<Value, ToolExecutionError> {
        let request_started = Instant::now();
        let status_deadline = self
            .managed_status_timeout
            .map(|timeout| {
                request_started
                    .checked_add(timeout)
                    .ok_or_else(|| ToolExecutionError::new(MANAGED_STATUS_TIMEOUT))
            })
            .transpose()?;
        let mut core = lock_task_status_core_until(&self.inner, status_deadline)?;
        let status_config = core
            .managed_foreman
            .as_ref()
            .map(|config| config.begin_status_request_at(request_started))
            .transpose()
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let configured_status_deadline = status_config
            .as_ref()
            .and_then(ManagedForemanServiceConfig::status_request_deadline);
        if configured_status_deadline != status_deadline {
            return Err(ToolExecutionError::new(MANAGED_STATUS_TIMEOUT));
        }
        let run_id = core.delivery.database.run_id().to_owned();
        apply_canonical_hermes_tool_policy(
            &mut core.hermes,
            &run_id,
            CanonicalHermesTool::TaskStatus,
        )?;
        let parsed_task_ref = ContentDigest::from_sha256(arguments.task_ref())
            .map_err(|_| ToolExecutionError::new("LATTICE_TASK_REFERENCE_REJECTED"))?;
        let general_submission = match configured_status_deadline {
            Some(operation_deadline) => {
                load_general_submission_by_task_ref_at(&core, &parsed_task_ref, operation_deadline)
                    .map_err(|error| managed_status_tool_error(operation_deadline, error.code()))?
            }
            None => load_general_submission_by_task_ref(&core, &parsed_task_ref)?,
        };
        if let Some(submission) = general_submission {
            if arguments
                .client_request_id()
                .is_some_and(|client_request_id| {
                    client_request_id != submission.client_request_id()
                })
            {
                return Err(ToolExecutionError::new("LATTICE_TASK_REFERENCE_REJECTED"));
            }
            let binding = general_task_binding(&submission)
                .map_err(|error| ToolExecutionError::new(error.code()))?;
            let mut lifecycle = match configured_status_deadline {
                Some(operation_deadline) => {
                    general_task_lifecycle_at(&core, &submission, operation_deadline).map_err(
                        |error| managed_status_tool_error(operation_deadline, error.code()),
                    )?
                }
                None => general_task_lifecycle(&core, &submission)
                    .map_err(|error| ToolExecutionError::new(error.code()))?,
            };
            let evidence =
                TaskIntakeLifecyclePort::load(&mut lifecycle, &binding).map_err(|error| {
                    match configured_status_deadline {
                        Some(operation_deadline) => {
                            managed_status_tool_error(operation_deadline, error.code())
                        }
                        None => ToolExecutionError::new(error.code()),
                    }
                })?;
            return managed_general_task_public_status(
                &core,
                &evidence,
                &submission,
                status_config.as_ref(),
            );
        }
        let client_request_id = arguments
            .client_request_id()
            .ok_or_else(|| ToolExecutionError::new("LATTICE_TASK_REFERENCE_NOT_FOUND"))?;
        let submission = mcp_gateway_submission(client_request_id)
            .map_err(|error| ToolExecutionError::new(error.code()))?;
        let binding = submission.binding().clone();
        let mut lifecycle = match configured_status_deadline {
            Some(operation_deadline) => task_lifecycle_at(&core, &binding, operation_deadline),
            None => task_lifecycle(&core, &binding),
        }
        .map_err(|error| match configured_status_deadline {
            Some(operation_deadline) => managed_status_tool_error(operation_deadline, error.code()),
            None => ToolExecutionError::new(error.code()),
        })?;
        let (evidence, historical_admission_command_id) =
            match TaskLifecyclePort::load(&mut lifecycle, &binding) {
                Ok(evidence) => (evidence, None),
                Err(error)
                    if error.code() == "LATTICE_TASK_INGRESS_PROFILE_COMMITMENT_MISMATCH" =>
                {
                    let (evidence, admission_command_id) = lifecycle
                        .load_historical_terminal_status(&binding)
                        .map_err(|error| match configured_status_deadline {
                            Some(operation_deadline) => {
                                managed_status_tool_error(operation_deadline, error.code())
                            }
                            None => ToolExecutionError::new(error.code()),
                        })?;
                    (evidence, Some(admission_command_id))
                }
                Err(error) => {
                    return Err(match configured_status_deadline {
                        Some(operation_deadline) => {
                            managed_status_tool_error(operation_deadline, error.code())
                        }
                        None => ToolExecutionError::new(error.code()),
                    });
                }
            };
        let admission_command_id = match historical_admission_command_id {
            Some(command_id) => command_id,
            None => lifecycle
                .verified_admission_command_id(&binding)
                .map_err(|error| match configured_status_deadline {
                    Some(operation_deadline) => {
                        managed_status_tool_error(operation_deadline, error.code())
                    }
                    None => ToolExecutionError::new(error.code()),
                })?,
        };
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
        verified_task_status_at(&mut core, &evidence, &task_ref, configured_status_deadline)
            .map_err(|error| match configured_status_deadline {
                Some(operation_deadline) => {
                    managed_status_tool_error(operation_deadline, error.code())
                }
                None => ToolExecutionError::new(error.code()),
            })
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

enum FullChainInputChunk {
    Bytes(Vec<u8>),
    Eof,
    Failed,
}

/// Process-lifetime, input-only stdin reader for the full-chain binary.
///
/// This thread receives no service, database, provider, scheduler, guard, or
/// credential capability. That narrow boundary is intentional: a fatal
/// `OpenClaw` result can interrupt the channel-backed MCP reader without waiting
/// for external stdin EOF. After the unique service teardown completes, an OS
/// process return may leave only this blocked input syscall for process exit to
/// discard; no product effect can outlive the returned error.
struct ProcessLifetimeStdinReader {
    receiver: mpsc::Receiver<FullChainInputChunk>,
    handle: Option<thread::JoinHandle<()>>,
    fatal_surface: Arc<AtomicBool>,
    buffer: Vec<u8>,
    offset: usize,
    eof: bool,
    terminal_received: bool,
    detached_after_surfaces: bool,
}

impl ProcessLifetimeStdinReader {
    fn spawn<R>(mut input: R, fatal_surface: Arc<AtomicBool>) -> Result<Self, LatticedError>
    where
        R: Read + Send + 'static,
    {
        let (sender, receiver) = mpsc::sync_channel(1);
        let handle = thread::Builder::new()
            .name("lattice-full-chain-stdin".to_owned())
            .spawn(move || {
                let mut buffer = [0_u8; FULL_CHAIN_STDIN_CHUNK_BYTES];
                loop {
                    match input.read(&mut buffer) {
                        Ok(0) => {
                            let _ = sender.send(FullChainInputChunk::Eof);
                            return;
                        }
                        Ok(length) => {
                            if sender
                                .send(FullChainInputChunk::Bytes(buffer[..length].to_vec()))
                                .is_err()
                            {
                                return;
                            }
                        }
                        Err(_) => {
                            let _ = sender.send(FullChainInputChunk::Failed);
                            return;
                        }
                    }
                }
            })
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
        Ok(Self {
            receiver,
            handle: Some(handle),
            fatal_surface,
            buffer: Vec::new(),
            offset: 0,
            eof: false,
            terminal_received: false,
            detached_after_surfaces: false,
        })
    }

    fn refill(&mut self) -> io::Result<()> {
        while !self.eof && self.offset == self.buffer.len() {
            self.buffer.clear();
            self.offset = 0;
            if self.fatal_surface.load(AtomicOrdering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "full-chain surface terminated",
                ));
            }
            match self.receiver.recv_timeout(FULL_CHAIN_STDIN_POLL_INTERVAL) {
                Ok(FullChainInputChunk::Bytes(bytes)) => self.buffer = bytes,
                Ok(FullChainInputChunk::Eof) => {
                    self.eof = true;
                    self.terminal_received = true;
                }
                Ok(FullChainInputChunk::Failed) => {
                    self.terminal_received = true;
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "full-chain stdin failed",
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.terminal_received = true;
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "full-chain stdin disconnected",
                    ));
                }
            }
        }
        Ok(())
    }

    fn finish_after_surfaces(&mut self) -> Result<(), LatticedError> {
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        if self.terminal_received || handle.is_finished() {
            return handle
                .join()
                .map_err(|_| LatticedError::new(LatticedErrorKind::ManagedTeardownRejected));
        }
        // This is the sole deliberate process-lifetime detach. It happens only
        // after listener join and scheduler/Hermes teardown; the thread owns an
        // input handle and a bounded sender, and therefore cannot perform or
        // retain any provider effect while the binary returns its typed error.
        self.detached_after_surfaces = true;
        drop(handle);
        Ok(())
    }
}

impl Read for ProcessLifetimeStdinReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let length = output.len().min(available.len());
        output[..length].copy_from_slice(&available[..length]);
        self.consume(length);
        Ok(length)
    }
}

impl BufRead for ProcessLifetimeStdinReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.fatal_surface.load(AtomicOrdering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "full-chain surface terminated",
            ));
        }
        self.refill()?;
        if self.fatal_surface.load(AtomicOrdering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "full-chain surface terminated",
            ));
        }
        Ok(&self.buffer[self.offset..])
    }

    fn consume(&mut self, amount: usize) {
        self.offset = self.buffer.len().min(self.offset.saturating_add(amount));
    }
}

fn serve_full_chain_stdio<Serve, StopPump, FinishService>(
    stdin_owner: &mut ProcessLifetimeStdinReader,
    serve: Serve,
    stop_pump: StopPump,
    finish_service: FinishService,
) -> Result<(), LatticedError>
where
    Serve: FnOnce(&mut ProcessLifetimeStdinReader) -> Result<(), LatticedError>,
    StopPump: FnOnce() -> Result<OpenClawPumpExit, LatticedError>,
    FinishService: FnOnce(Result<(), LatticedError>) -> Result<(), LatticedError>,
{
    let serve_result = serve(stdin_owner);
    let surface_result = finish_full_chain_surfaces(serve_result, stop_pump, finish_service);
    let input_result = stdin_owner.finish_after_surfaces();
    match input_result {
        Ok(()) => surface_result,
        Err(input) => Err(input),
    }
}

/// Serves MCP stdio and continuously pumps the authenticated `OpenClaw` listener.
///
/// Both surfaces hold clones of the same [`FullChainService`], so they serialize
/// through one coordinator and share `PostgreSQL` receipts. Stdin EOF and fatal
/// listener failures both use the same typed listener-join, managed-scheduler,
/// and Hermes teardown path.
///
/// # Errors
///
/// Returns a bounded MCP startup, stdio transport, listener, or teardown failure.
pub fn serve_full_chain_runtime<H>(runtime: FullChainRuntime<H>) -> Result<(), LatticedError>
where
    H: FullChainHermesPort + 'static,
{
    let (mcp_service, mcp_binding, openclaw_server) = runtime.into_parts();
    let openclaw_endpoint = openclaw_server
        .local_addr()
        .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
    let shutdown = mcp_service.clone();
    let fatal_shutdown = shutdown.clone();
    let fatal_surface = Arc::new(AtomicBool::new(false));
    let pump_fatal_surface = Arc::clone(&fatal_surface);
    let mut openclaw_owner =
        match OpenClawPumpOwner::spawn(openclaw_server, openclaw_endpoint, move |failure| {
            if fatal_openclaw_pump_error(failure.kind) {
                eprintln!("{}", LatticedErrorKind::Transport.code());
                // Stop admitting new stdin frames immediately. The main owner
                // cannot return until this callback completes the exact service
                // teardown and publishes its typed pump terminal below.
                pump_fatal_surface.store(true, AtomicOrdering::Release);
                let terminal = fatal_shutdown
                    .finish_hermes_session(Err(LatticedError::new(LatticedErrorKind::Transport)))
                    .expect_err("fatal listener shutdown preserves a terminal failure")
                    .kind();
                OpenClawPumpControl::Stop(terminal)
            } else {
                OpenClawPumpControl::Continue
            }
        }) {
            Ok(owner) => owner,
            Err(failure) => return shutdown.finish_hermes_session(Err(failure)),
        };
    let mut stdin_owner = match ProcessLifetimeStdinReader::spawn(io::stdin(), fatal_surface) {
        Ok(owner) => owner,
        Err(failure) => {
            return finish_full_chain_surfaces(
                Err(failure),
                || openclaw_owner.shutdown(),
                |result| shutdown.finish_hermes_session(result),
            );
        }
    };
    serve_full_chain_stdio(
        &mut stdin_owner,
        |input| {
            let output = io::stdout();
            mcp::serve_legacy_delivery_observer(mcp_service, mcp_binding, input, output.lock())
                .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))
        },
        || openclaw_owner.shutdown(),
        |result| shutdown.finish_hermes_session(result),
    )
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
    Stop(LatticedErrorKind),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpenClawPumpExit {
    Cancelled,
    Fatal(LatticedErrorKind),
}

struct OpenClawPumpOwner {
    cancellation: Arc<AtomicBool>,
    wake_endpoint: SocketAddr,
    completion: mpsc::Receiver<OpenClawPumpExit>,
    handle: Option<thread::JoinHandle<OpenClawPumpExit>>,
    armed: bool,
}

impl OpenClawPumpOwner {
    fn spawn<P, F>(pump: P, wake_endpoint: SocketAddr, on_failure: F) -> Result<Self, LatticedError>
    where
        P: FullChainOpenClawPump,
        F: FnMut(OpenClawPumpFailure) -> OpenClawPumpControl + Send + 'static,
    {
        let cancellation = Arc::new(AtomicBool::new(false));
        let thread_cancellation = Arc::clone(&cancellation);
        let (completion_sender, completion) = mpsc::sync_channel(1);
        let handle = thread::Builder::new()
            .name("lattice-openclaw-full-chain".to_owned())
            .spawn(move || {
                let exit = run_openclaw_pump(pump, &thread_cancellation, on_failure);
                let _ = completion_sender.send(exit);
                exit
            })
            .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))?;
        Ok(Self {
            cancellation,
            wake_endpoint,
            completion,
            handle: Some(handle),
            armed: true,
        })
    }

    fn shutdown(&mut self) -> Result<OpenClawPumpExit, LatticedError> {
        if !self.armed {
            return Err(LatticedError::new(
                LatticedErrorKind::ManagedTeardownRejected,
            ));
        }
        self.cancellation.store(true, AtomicOrdering::Release);
        // `serve_once` blocks in its nonblocking-listener polling loop. A local
        // connection that is immediately closed gives it a bounded wakeup; the
        // cancellation check consumes that transport result before any callback.
        let _ = TcpStream::connect_timeout(&self.wake_endpoint, OPENCLAW_PUMP_WAKE_TIMEOUT);
        let observed = match self
            .completion
            .recv_timeout(OPENCLAW_PUMP_SHUTDOWN_DEADLINE)
        {
            Ok(exit) => exit,
            Err(mpsc::RecvTimeoutError::Timeout) => process::abort(),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let joined = self
                    .handle
                    .take()
                    .expect("armed OpenClaw pump owns its join handle")
                    .join();
                self.armed = false;
                return joined
                    .map_err(|_| LatticedError::new(LatticedErrorKind::ManagedTeardownRejected));
            }
        };
        let joined = self
            .handle
            .take()
            .expect("armed OpenClaw pump owns its join handle")
            .join()
            .map_err(|_| LatticedError::new(LatticedErrorKind::ManagedTeardownRejected))?;
        self.armed = false;
        if joined != observed {
            return Err(LatticedError::new(
                LatticedErrorKind::ManagedTeardownRejected,
            ));
        }
        Ok(joined)
    }
}

impl Drop for OpenClawPumpOwner {
    fn drop(&mut self) {
        if self.armed && self.shutdown().is_err() {
            // A Rust thread cannot be detached safely while it still owns the
            // listener/service clone. Fail-stop is the bounded last resort.
            process::abort();
        }
    }
}

fn run_openclaw_pump<P, F>(
    mut pump: P,
    cancellation: &AtomicBool,
    mut on_failure: F,
) -> OpenClawPumpExit
where
    P: FullChainOpenClawPump,
    F: FnMut(OpenClawPumpFailure) -> OpenClawPumpControl,
{
    loop {
        if cancellation.load(AtomicOrdering::Acquire) {
            return OpenClawPumpExit::Cancelled;
        }
        if let Err(failure) = pump.pump_once() {
            if cancellation.load(AtomicOrdering::Acquire) {
                return OpenClawPumpExit::Cancelled;
            }
            match on_failure(failure) {
                OpenClawPumpControl::Continue => {}
                OpenClawPumpControl::Stop(kind) => return OpenClawPumpExit::Fatal(kind),
            }
        }
    }
}

fn finish_full_chain_surfaces<StopPump, FinishService>(
    serve_result: Result<(), LatticedError>,
    stop_pump: StopPump,
    finish_service: FinishService,
) -> Result<(), LatticedError>
where
    StopPump: FnOnce() -> Result<OpenClawPumpExit, LatticedError>,
    FinishService: FnOnce(Result<(), LatticedError>) -> Result<(), LatticedError>,
{
    let joined_result = match stop_pump() {
        Ok(OpenClawPumpExit::Cancelled) => serve_result,
        Ok(OpenClawPumpExit::Fatal(kind)) => Err(LatticedError::new(kind)),
        Err(listener) => Err(listener),
    };
    finish_service(joined_result)
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
    let managed_scripted_acceptance = if config.runtime == DeliveryRuntime::ScriptedAcceptance {
        let binding =
            validate_managed_scripted_active_restart_admission(run_mode, &config, database)?;
        validate_scripted_fixture(&config)?;
        Some(binding)
    } else if config.runtime != DeliveryRuntime::OfficialCodexAppServer {
        return Err(LatticedError::new(LatticedErrorKind::OfficialLiveBlocked));
    } else {
        None
    };
    let is_production_configured =
        production_hermes_sealed::Sealed::is_production_configured(&hermes);
    if require_production_hermes != is_production_configured {
        return Err(LatticedError::new(
            LatticedErrorKind::HermesProductionRunnerRequired,
        ));
    }
    let store_authority = configured_store_authority()?;
    let process_start_identity = daemon_process_start_identity()?;
    let task_ingress_peer = configured_task_ingress_peer(&process_start_identity)?;
    let mcp_binding = submission.binding().clone();
    let managed_foreman = managed_foreman_service_from_environment(
        &config,
        database,
        password,
        &store_authority,
        &task_ingress_peer,
        &process_start_identity,
    )?;
    let managed_status_timeout = managed_foreman
        .as_ref()
        .map(|_| config.timeout.min(MANAGED_STATUS_MAX_DURATION));
    let managed_tasks = Arc::new(Mutex::new(BTreeSet::new()));
    let managed_identity_source = ManagedForemanIdentitySource {
        database: database.clone(),
        password: password.to_owned(),
        timeout: config.timeout,
        store_authority: store_authority.clone(),
    };
    let delivery = full_chain_delivery_service(config, database, password, run_mode)?;
    let mut core = FullChainCore {
        delivery,
        hermes,
        submission,
        run_mode,
        integration_mode,
        process_start_identity,
        task_ingress_peer,
        store_authority,
        managed_scripted_acceptance,
        managed_foreman,
        managed_tasks,
        managed_scheduler: None,
    };
    // Ordinary serving never migrates. It must verify the fixed durable
    // foreman replay before either MCP or OpenClaw can accept a request.
    let mut coordination = foreman_coordination(&core).map_err(foreman_replay_latticed)?;
    let foreman_status = coordination
        .load_runtime_status()
        .map_err(|error| foreman_replay_latticed(ToolExecutionError::new(error.code())))?;
    drop(coordination);
    if core.managed_foreman.is_some() {
        formal_managed_foreman_identity_from_status(&foreman_status)
            .map_err(foreman_replay_latticed)?;
        let scheduler = start_after_complete_stage(
            || stage_managed_restart_tasks(&core).map_err(foreman_replay_latticed),
            |initial_tasks| {
                managed_scheduler(
                    core.managed_foreman
                        .as_ref()
                        .expect("managed foreman checked")
                        .clone(),
                    managed_identity_source,
                    Arc::clone(&core.managed_tasks),
                    initial_tasks,
                )
            },
        )?;
        core.managed_scheduler = Some(scheduler);
    }
    Ok((
        FullChainService {
            inner: Arc::new(Mutex::new(core)),
            managed_status_timeout,
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
    // MCP submissions share the Runtime ledger, but not a Writer Lease. A
    // terminally uncertain task must remain readable without preventing an
    // unrelated idempotency key from doing bounded work.
    let project = format!("lattice-mcp-{}", &request_digest.as_str()[..24]);
    let snapshot = format!("{project}:snapshot:1");
    gateway_submission(GatewaySubmissionIdentity {
        project: &project,
        snapshot: &snapshot,
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

/// Constructs one derived-memory request from a process-owned Git commit.
///
/// Unlike the legacy delivery continuation, this binding does not imply a
/// delivery receipt. It only identifies the immutable source selected at
/// process start and keeps its result isolated from every other commit.
fn runtime_graph_request(
    run_id: &str,
    commit: &str,
    configuration_digest: ContentDigest,
) -> Result<GraphMemoryRunRequest, LatticedError> {
    let commit_id = GitObjectId::new(commit)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let subject_digest = digest(
        "lattice.runtime.graphify-source-request",
        &CanonicalValue::Object(vec![
            (
                "commit".to_owned(),
                CanonicalValue::String(commit.to_owned()),
            ),
            (
                "configuration_digest".to_owned(),
                CanonicalValue::String(configuration_digest.as_str().to_owned()),
            ),
            (
                "run_id".to_owned(),
                CanonicalValue::String(run_id.to_owned()),
            ),
        ]),
    )?;
    let invocation = Invocation::new(
        CONTRACT_VERSION,
        RequestId::new(format!("runtime-graph-request-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        TaskId::new(GRAPH_TASK_ID).map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        AttemptId::new(format!("runtime-graph-attempt-{run_id}"))
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        ProjectSnapshotId::new(GRAPH_PROJECT_SNAPSHOT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        subject_digest,
    )
    .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    GraphMemoryRunRequest::new(
        invocation,
        ProjectId::new(GRAPH_PROJECT_ID)
            .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?,
        commit_id,
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
        | LatticedErrorKind::HermesTeardownRejected
        | LatticedErrorKind::ManagedTeardownRejected => PortErrorKind::Ambiguous,
        LatticedErrorKind::DatabaseConnect
        | LatticedErrorKind::GraphReceiptRead
        | LatticedErrorKind::HermesReceiptRead
        | LatticedErrorKind::HermesProductionLivenessRejected
        | LatticedErrorKind::ForemanReplayUnavailable => PortErrorKind::Unavailable,
        LatticedErrorKind::Configuration
        | LatticedErrorKind::Contract
        | LatticedErrorKind::ReceiptMismatch => PortErrorKind::Malformed,
        LatticedErrorKind::Intent
        | LatticedErrorKind::OutcomePersistence
        | LatticedErrorKind::DeliveryFailed
        | LatticedErrorKind::OfficialLiveBlocked
        | LatticedErrorKind::ScriptedFixtureRejected
        | LatticedErrorKind::GraphExecution
        | LatticedErrorKind::GraphSnapshotExecution
        | LatticedErrorKind::GraphifyExecution
        | LatticedErrorKind::GraphNormalization
        | LatticedErrorKind::GraphPersistence
        | LatticedErrorKind::GraphRetrieval
        | LatticedErrorKind::GraphReceipt
        | LatticedErrorKind::HermesPreparationMissing
        | LatticedErrorKind::HermesPreparationRequired
        | LatticedErrorKind::HermesProductionRunnerRequired
        | LatticedErrorKind::HermesExecution
        | LatticedErrorKind::DatabaseSecret
        | LatticedErrorKind::LedgerConfiguration
        | LatticedErrorKind::RuntimePostgresProvision
        | LatticedErrorKind::RuntimePostgresBoundary
        | LatticedErrorKind::RuntimePostgresMigration
        | LatticedErrorKind::RuntimePostgresExternalAdoption
        | LatticedErrorKind::RuntimePostgresForeman
        | LatticedErrorKind::RuntimePostgresMigrationPermission
        | LatticedErrorKind::RuntimePostgresMigrationUnsafeSetting
        | LatticedErrorKind::RuntimePostgresVerification
        | LatticedErrorKind::WorkspaceConfiguration
        | LatticedErrorKind::CodexConfiguration
        | LatticedErrorKind::ReceiptRead
        | LatticedErrorKind::GraphConfiguration
        | LatticedErrorKind::TaskControl
        | LatticedErrorKind::WriterLease
        | LatticedErrorKind::ForemanReplayCorrupt
        | LatticedErrorKind::ForemanReplayUnsupported
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

/// Process-owned source for ordinary Graphify refreshes.
///
/// This is deliberately separate from the historical delivery fixture: it
/// reads one clean Git worktree at its exact `HEAD` and never accepts a path
/// from an MCP request.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeGraphSource {
    repository_root: PathBuf,
    work_root: PathBuf,
    git_executable: PathBuf,
    git_sha256: String,
}

fn runtime_graph_source_from_environment() -> Result<(RuntimeGraphSource, String), LatticedError> {
    let repository_root = graph_canonical_directory(Path::new(&required_environment(
        "LATTICE_GRAPHIFY_SOURCE_ROOT",
    )?))?;
    let work_root = PathBuf::from(required_environment("LATTICE_GRAPHIFY_WORK_ROOT")?);
    fs::create_dir_all(&work_root)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let work_root = graph_canonical_directory(&work_root)?;
    let git_executable = PathBuf::from(required_environment("LATTICE_DELIVERY_GIT_EXE")?);
    let git_sha256 = graph_executable_sha256(&git_executable)?;

    let top_level = graph_git_stdout(
        &git_executable,
        &repository_root,
        ["rev-parse", "--show-toplevel"],
    )?;
    if graph_canonical_directory(Path::new(&top_level))? != repository_root {
        return Err(LatticedError::new(LatticedErrorKind::GraphConfiguration));
    }
    let clean = graph_git_output(
        &git_executable,
        &repository_root,
        ["status", "--porcelain=v1", "-z"],
    )?;
    if !clean.stdout.is_empty() {
        return Err(LatticedError::new(LatticedErrorKind::GraphConfiguration));
    }
    let commit = graph_git_stdout(&git_executable, &repository_root, ["rev-parse", "HEAD"])?;
    GitObjectId::new(&commit)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    Ok((
        RuntimeGraphSource {
            repository_root,
            work_root,
            git_executable,
            git_sha256,
        },
        commit,
    ))
}

fn graph_canonical_directory(path: &Path) -> Result<PathBuf, LatticedError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(LatticedError::new(LatticedErrorKind::GraphConfiguration));
    }
    fs::canonicalize(path).map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))
}

fn graph_git_output<I, S>(
    executable: &Path,
    repository_root: &Path,
    arguments: I,
) -> Result<process::Output, LatticedError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = process::Command::new(executable)
        .current_dir(repository_root)
        .args(arguments)
        .output()
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(LatticedError::new(LatticedErrorKind::GraphConfiguration));
    }
    Ok(output)
}

fn graph_git_stdout<I, S>(
    executable: &Path,
    repository_root: &Path,
    arguments: I,
) -> Result<String, LatticedError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = graph_git_output(executable, repository_root, arguments)?;
    let value = std::str::from_utf8(&output.stdout)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?
        .trim();
    if value.is_empty() || value.contains(char::is_whitespace) {
        return Err(LatticedError::new(LatticedErrorKind::GraphConfiguration));
    }
    Ok(value.to_owned())
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
        .map_err(graph_memory_execution_error)
}

/// Refreshes derived Graphify memory from the fixed, clean Git source selected
/// in process configuration. This is intentionally a local CLI path rather
/// than an MCP tool: callers cannot supply a source path, commit, command, or
/// database target.
///
/// # Errors
///
/// Returns a bounded configuration, database, Graphify, or persistence error;
/// it never creates a delivery receipt or substitutes a different source.
pub fn refresh_runtime_graphify_from_environment() -> Result<GraphMemoryReceipt, LatticedError> {
    let (_unused_delivery, database, password) =
        delivery_environment_for_mode(FullChainRunMode::ResumeExisting)?;
    let timeout = match env::var("LATTICE_DELIVERY_TIMEOUT_SECONDS") {
        Ok(value) => parse_timeout(&value)?,
        Err(env::VarError::NotPresent) => Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(LatticedError::new(LatticedErrorKind::Configuration));
        }
    };
    let (source, request) = runtime_graph_source_request(&database)?;
    if let Some(receipt) =
        load_runtime_graph_receipt(&database, &password, deadline(timeout)?, &request)?
    {
        return Ok(receipt);
    }
    run_runtime_graph_memory_request(&database, &password, &source, deadline(timeout)?, &request)
}

/// Runs the optional Hermes reflection over the current derived Graphify
/// receipt. This is deliberately independent of the historical delivery lane:
/// it neither creates nor changes a delivery receipt, and always tears down
/// the contained Hermes runner before returning.
///
/// # Errors
///
/// Returns a bounded Graphify-receipt, Hermes, persistence, or teardown error.
/// A failed optional reflection never changes PostgreSQL task or delivery truth.
#[cfg(windows)]
pub fn reflect_runtime_hermes_from_environment() -> Result<HermesReflectionReceipt, LatticedError> {
    let (_unused_delivery, database, password) =
        delivery_environment_for_mode(FullChainRunMode::ResumeExisting)?;
    let timeout = match env::var("LATTICE_DELIVERY_TIMEOUT_SECONDS") {
        Ok(value) => parse_timeout(&value)?,
        Err(env::VarError::NotPresent) => Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
        Err(env::VarError::NotUnicode(_)) => {
            return Err(LatticedError::new(LatticedErrorKind::Configuration));
        }
    };
    let (_source, request) = runtime_graph_source_request(&database)?;
    match load_reflection_from_postgres(&database, &password, timeout, &request) {
        Ok(receipt) => return Ok(receipt),
        Err(error)
            if error.kind() == PortErrorKind::Unavailable
                && error.code() == "MEMORY_RECEIPT_UNAVAILABLE" => {}
        Err(error) => return Err(map_reflection_read_error(&error)),
    }
    let graph_receipt =
        load_runtime_graph_receipt(&database, &password, deadline(timeout)?, &request)?
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::GraphReceiptRead))?;
    let hermes_request = hermes_request_for_graph(database.run_id(), &request, &graph_receipt)?;
    let configuration = DirectCodexHermesEnvironmentConfig::from_environment()?;
    let session_id = new_hermes_session_token()?;
    let job = HermesReflectionJob::new(
        hermes_request.clone(),
        session_id.clone(),
        FULL_CHAIN_CODEX_BROKER_MODEL,
        hermes_job_evidence(&request, &graph_receipt)
            .map_err(|error| map_hermes_research_error(&error))?,
    )
    .map_err(|error| map_hermes_research_error(&map_hermes_adapter_error(&error)))?;
    let direct = configuration
        .broker
        .run_direct_reflection(&job, deadline(configuration.timeout)?)
        .map_err(|error| map_hermes_research_error(&map_hermes_adapter_error(&error)))?;
    persist_direct_codex_reflection(
        &database,
        &password,
        timeout,
        &request,
        &graph_receipt,
        &hermes_request,
        &session_id,
        &job,
        direct,
    )
}

#[cfg(windows)]
struct DirectCodexHermesEnvironmentConfig {
    broker: CodexReflectionBrokerConfig,
    timeout: Duration,
}

#[cfg(windows)]
impl DirectCodexHermesEnvironmentConfig {
    fn from_environment() -> Result<Self, LatticedError> {
        let product_root = PathBuf::from(hermes_environment("LATTICE_HERMES_PRODUCT_ROOT")?);
        let timeout_seconds = hermes_environment("LATTICE_HERMES_DEADLINE_SECONDS")?
            .parse::<u64>()
            .ok()
            .filter(|seconds| (1..=300).contains(seconds))
            .ok_or_else(|| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        let broker = CodexReflectionBrokerConfig::new(
            PathBuf::from(hermes_environment("LATTICE_HERMES_CODEX_LAUNCHER")?),
            PathBuf::from(hermes_environment("LATTICE_HERMES_CODEX_HOME")?),
            PathBuf::from(hermes_environment("LATTICE_HERMES_BROKER_ISOLATION_ROOT")?),
            product_root,
            FULL_CHAIN_CODEX_BROKER_MODEL,
        )
        .map_err(|_| LatticedError::new(LatticedErrorKind::HermesProductionRunnerRequired))?;
        Ok(Self {
            broker,
            timeout: Duration::from_secs(timeout_seconds),
        })
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn persist_direct_codex_reflection(
    database: &DeliveryDatabaseBinding,
    password: &str,
    timeout: Duration,
    graph_request: &GraphMemoryRunRequest,
    graph_receipt: &GraphMemoryReceipt,
    request: &HermesResearchRequest,
    session_id: &str,
    job: &HermesReflectionJob,
    direct: DirectCodexReflection,
) -> Result<HermesReflectionReceipt, LatticedError> {
    let evidence = HermesEvidence::new(
        request.invocation().clone(),
        RuntimeKind::Live,
        direct.reflection().output_digest().clone(),
    );
    let candidate = reflection_candidate(
        request,
        graph_request,
        graph_receipt,
        session_id,
        job.model(),
        job.input_digest(),
        direct.identity_digest(),
        direct.reflection(),
        &evidence,
    )
    .map_err(|error| map_hermes_research_error(&error))?;
    let persisted = persist_reflection_to_postgres(database, password, timeout, &candidate)?;
    let replayed = load_reflection_from_postgres(database, password, timeout, graph_request)
        .map_err(|error| map_reflection_read_error(&error))?;
    if replayed != persisted {
        return Err(LatticedError::new(LatticedErrorKind::HermesReceiptRead));
    }
    Ok(replayed)
}

#[cfg(not(windows))]
pub fn reflect_runtime_hermes_from_environment() -> Result<HermesReflectionReceipt, LatticedError> {
    Err(LatticedError::new(
        LatticedErrorKind::HermesProductionRunnerRequired,
    ))
}

fn runtime_graph_source_request(
    database: &DeliveryDatabaseBinding,
) -> Result<(RuntimeGraphSource, GraphMemoryRunRequest), LatticedError> {
    let (source, commit) = runtime_graph_source_from_environment()?;
    let configuration_digest = digest(
        "lattice.runtime.graphify-source-configuration",
        &CanonicalValue::Object(vec![
            (
                "git_sha256".to_owned(),
                CanonicalValue::String(source.git_sha256.clone()),
            ),
            (
                "repository_root".to_owned(),
                CanonicalValue::String(path_text(&source.repository_root)?),
            ),
            (
                "runtime_root".to_owned(),
                CanonicalValue::String(path_text(&graphify_runtime_root_from_environment(
                    &source.repository_root,
                ))?),
            ),
        ]),
    )?;
    let request = runtime_graph_request(database.run_id(), &commit, configuration_digest)?;
    Ok((source, request))
}

fn load_runtime_graph_receipt(
    database: &DeliveryDatabaseBinding,
    password: &str,
    deadline: Instant,
    request: &GraphMemoryRunRequest,
) -> Result<Option<GraphMemoryReceipt>, LatticedError> {
    let client = connect_fixed_runtime_client(database, password, deadline)
        .map_err(|_| LatticedError::new(LatticedErrorKind::DatabaseConnect))?;
    let target = ExtensionTarget::new(database.database_name(), database.run_id())
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    let mut memory = PostgresCodebaseMemory::new(client, target)
        .map_err(|_| LatticedError::new(LatticedErrorKind::GraphConfiguration))?;
    match graph_memory_status(request, &mut memory) {
        Ok(receipt) => Ok(Some(receipt)),
        Err(GraphMemoryOrchestratorError::Receipt(error))
            if error.code() == "MEMORY_RECEIPT_UNAVAILABLE" =>
        {
            Ok(None)
        }
        Err(_) => Err(LatticedError::new(LatticedErrorKind::GraphReceiptRead)),
    }
}

fn run_runtime_graph_memory_request(
    database: &DeliveryDatabaseBinding,
    password: &str,
    source: &RuntimeGraphSource,
    deadline: Instant,
    request: &GraphMemoryRunRequest,
) -> Result<GraphMemoryReceipt, LatticedError> {
    let query = MemoryQuery::new(request, GRAPH_QUERY, GRAPH_RETRIEVAL_LIMIT)
        .map_err(|_| LatticedError::new(LatticedErrorKind::Contract))?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| LatticedError::new(LatticedErrorKind::GraphExecution))?;
    let graph_root = source.work_root.join(request.commit_id().as_str());
    let bridge = SnapshotBridge::new();
    let snapshot_config = GitSnapshotConfig::new(
        source.git_executable.clone(),
        source.git_sha256.clone(),
        source.repository_root.clone(),
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
        graphify_runtime_root_from_environment(&source.repository_root),
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
        .map_err(graph_memory_execution_error)
}

/// Converts the orchestrator's typed stage into a fixed, payload-free runtime
/// error code.  The underlying adapter diagnostic stays out of the CLI/MCP
/// boundary, while recovery logic can still tell where to look.
fn graph_memory_execution_error(error: GraphMemoryOrchestratorError) -> LatticedError {
    let kind = match error {
        GraphMemoryOrchestratorError::Snapshot(_) => LatticedErrorKind::GraphSnapshotExecution,
        GraphMemoryOrchestratorError::Graphify(_) => LatticedErrorKind::GraphifyExecution,
        GraphMemoryOrchestratorError::Normalize(_) => LatticedErrorKind::GraphNormalization,
        GraphMemoryOrchestratorError::Persistence(_) => LatticedErrorKind::GraphPersistence,
        GraphMemoryOrchestratorError::Retrieval(_) => LatticedErrorKind::GraphRetrieval,
        GraphMemoryOrchestratorError::Receipt(_) => LatticedErrorKind::GraphReceipt,
        GraphMemoryOrchestratorError::EvidenceMismatch(_) => LatticedErrorKind::GraphExecution,
    };
    LatticedError::new(kind)
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
            "if \"%~1\"==\"app-server\" if \"%~2\"==\"--stdio\" if \"%~3\"==\"\" goto server\r\n",
            "exit /b 11\r\n",
            ":version\r\n",
            "echo codex-cli 0.144.6\r\n",
            "exit /b 0\r\n",
            ":schema\r\n",
            "\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"%~dp0scripted-codex.ps1\" -ExpectedSelfSha256 \"{server_sha256}\" -Mode Schema -SchemaRoot \"%~4\"\r\n",
            "exit /b %ERRORLEVEL%\r\n",
            ":server\r\n",
            "set LATTICE_DELIVERY_CODEX_MODE=SCRIPTED_ACCEPTANCE\r\n",
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

    #[test]
    fn foreman_bootstrap_failure_has_a_secret_free_stage_code() {
        assert_eq!(
            LatticedErrorKind::RuntimePostgresForeman.code(),
            "LATTICED_RUNTIME_POSTGRES_FOREMAN_REJECTED"
        );
        assert_eq!(
            LatticedErrorKind::RuntimePostgresExternalAdoption.code(),
            "LATTICED_RUNTIME_POSTGRES_EXTERNAL_ADOPTION_REJECTED"
        );
    }

    use lattice_codebase_memory::{normalize_analysis, plan_retrieval};
    use lattice_contracts::{
        CodeSnapshotEvidence, CodebaseMemoryPersistenceIdentity, DaemonEpoch, GitRefIdentity,
        GraphConfidence, GraphMemoryPersistenceEvidence, GraphSourceProvenance, GraphifyIdentity,
        GraphifyRawEvidence, GraphifyRawNode, MemoryRetrievalDisposition, MemoryRetrievalEvidence,
        MemoryRetrievalPlan, NormalizedGraphAnalysis, PROJECT_AUTHORITY_PRODUCER_ID,
        PROJECT_AUTHORITY_PRODUCER_VERSION, ProjectClass, ProjectLifecycle, TrackedSource,
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

    #[test]
    fn postgres_bootstrap_admission_transition_is_exact_cas_and_stopped_recovery_is_closed() {
        let stopped = RuntimeAdmissionSnapshot {
            mode: "STOPPED".to_owned(),
            daemon_instance_id: None,
            daemon_epoch: None,
            authority_revision: 0,
            observation_digest: None,
            authority_head_digest: None,
        };
        assert!(stopped.is_stopped_no_leader());
        let source = include_str!("composition.rs");
        let boundary = source
            .split_once("impl RuntimeAdmissionSnapshot")
            .expect("admission boundary")
            .1
            .split_once("pub fn serve_stdio_from_environment")
            .expect("admission boundary end")
            .0;
        for required in [
            "daemon_instance_id IS NOT DISTINCT FROM $2",
            "observation_digest IS NOT DISTINCT FROM $5",
            "authority_head_digest IS NOT DISTINCT FROM $6",
            "WHERE singleton AND admission_mode = 'STOPPED'",
            "if affected != 1",
        ] {
            assert!(
                boundary.contains(required),
                "missing admission CAS guard: {required}"
            );
        }
    }

    #[test]
    fn foreman_catalog_measurement_is_test_only_marker_owned_and_reuses_foundation() {
        let source = include_str!("composition.rs").replace("\r\n", "\n");
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production composition");
        assert!(
            production
                .contains("#[cfg(test)]\n    if tests::foreman_catalog_measurement_requested()")
        );
        assert!(production.contains("return tests::measure_foreman_catalog_profile("));
        assert!(!production.contains("LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_V1"));
        assert!(!production.contains("MEASURE_CATALOG_PINS"));

        let tests = source
            .split("#[cfg(test)]\nmod tests")
            .nth(1)
            .expect("test-only composition");
        assert!(tests.contains("fn disposable_store_v7_foreman_catalog_measurement_profile()"));
        assert!(tests.contains(
            "#[ignore = \"requires a coordinator-owned disposable PostgreSQL 17 profile\"]"
        ));
        assert!(tests.contains("LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_V1"));
        assert!(tests.contains("bootstrap_postgres_extensions_from_environment()"));

        let script =
            include_str!("../../../scripts/test-phase4-postgres-foreman.ps1").replace("\r\n", "\n");
        assert!(script.contains("[switch]$MeasureCatalogPins"));
        assert!(script.contains("LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_V1"));
        assert!(script.contains("disposable_store_v7_foreman_catalog_measurement_profile"));

        let latticed = include_str!("bin/latticed.rs");
        assert!(!latticed.contains("catalog-measure"));
        assert!(!latticed.contains("foundation-only"));
    }

    #[cfg(windows)]
    fn measurement_comparable_path(path: &Path) -> String {
        let value = path.to_string_lossy().replace('/', "\\");
        let value = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{rest}")
        } else {
            value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
        };
        value.trim_end_matches('\\').to_lowercase()
    }

    #[cfg(not(windows))]
    fn measurement_comparable_path(path: &Path) -> String {
        path.to_string_lossy().trim_end_matches('/').to_owned()
    }

    fn measurement_same_path(left: &Path, right: &Path) -> bool {
        measurement_comparable_path(left) == measurement_comparable_path(right)
    }

    #[cfg(windows)]
    #[test]
    fn foreman_catalog_measurement_path_identity_accepts_windows_equivalents_only() {
        let ordinary = Path::new(r"C:\Users\Lattice\Temp\catalog-measurement");
        let extended = Path::new(r"\\?\C:\Users\Lattice\Temp\catalog-measurement");
        let case_and_separator = Path::new(r"c:/users/lattice/temp/catalog-measurement/");
        assert!(measurement_same_path(ordinary, extended));
        assert!(measurement_same_path(ordinary, case_and_separator));
        assert!(!measurement_same_path(
            ordinary,
            Path::new(r"C:\Users\Lattice\Temp\catalog-measurement-other")
        ));
        assert!(!measurement_same_path(
            ordinary,
            Path::new(r"C:\Users\Lattice\Elsewhere\catalog-measurement")
        ));
    }

    #[test]
    fn foreman_catalog_measurement_database_identity_comes_from_delivery_binding() {
        let source = include_str!("composition.rs").replace("\r\n", "\n");
        let tests = source
            .split("#[cfg(test)]\nmod tests")
            .nth(1)
            .expect("test-only composition");
        assert!(tests.contains("let database_name = database.database_name();"));
        assert!(tests.contains(
            "marker_object.insert(\"database\".to_owned(), Value::String(database_name.clone()));"
        ));
        assert!(tests.contains("FOREMAN_CATALOG_DATABASE={database_name}"));

        let script =
            include_str!("../../../scripts/test-phase4-postgres-foreman.ps1").replace("\r\n", "\n");
        assert!(!script.contains("'phase4_catalog_'"));
        assert!(script.contains("FOREMAN_CATALOG_DATABASE=([a-z0-9_]{3,63})"));
        assert!(script.contains("$measurementMarker.database -cne $measuredDatabase"));
    }

    #[test]
    fn phase4_postgres_script_pins_cargo_and_closes_password_cleanup() {
        let script =
            include_str!("../../../scripts/test-phase4-postgres-foreman.ps1").replace("\r\n", "\n");
        assert!(script.contains("$requiredRustToolchain = '1.97.1-x86_64-pc-windows-msvc'"));
        assert!(script.contains(
            "$requiredCargoSha256 = 'ddfbad20b31b918d3439d070945ec59bbfe037a6ec0ab5b584459e69c8b37d1b'"
        ));
        assert!(script.contains("which cargo --toolchain $requiredRustToolchain"));
        assert!(script.contains("& $cargo -Vv"));
        assert!(script.contains("PHASE4_POSTGRES_CARGO_IDENTITY_REJECTED"));

        let cleanup = script
            .split("function Remove-InitdbPassword")
            .nth(1)
            .expect("password cleanup helper")
            .split("function ")
            .next()
            .expect("password cleanup body");
        assert!(cleanup.contains("$script:passwordPath"));
        assert!(cleanup.contains("[IO.File]::Delete($resolvedPasswordPath)"));
        assert!(cleanup.contains("PHASE4_POSTGRES_PASSWORD_PATH_REJECTED"));
        assert!(cleanup.contains("PHASE4_POSTGRES_PASSWORD_CLEANUP_FAILED"));
        let cleanup_self_test = script
            .split("function Test-InitdbPasswordCleanup")
            .nth(1)
            .expect("password cleanup self-test")
            .split("function ")
            .next()
            .expect("password cleanup self-test body");
        assert!(cleanup_self_test.contains("PHASE4_POSTGRES_PASSWORD_NEGATIVE_SELF_TEST_FAILED"));
        assert!(cleanup_self_test.contains("-not [IO.File]::Exists($foreignPasswordPath)"));
        assert!(script.contains("if ($StaticPreflight) {"));
        let finally = script.rsplit("\nfinally {").next().expect("script finally");
        assert!(finally.contains("Remove-InitdbPassword"));
        assert!(!script.contains("Remove-Item -LiteralPath $passwordPath"));
    }

    #[test]
    fn phase4_catalog_measurement_receipt_is_distinct_and_shape_is_sql_derived() {
        let script =
            include_str!("../../../scripts/test-phase4-postgres-foreman.ps1").replace("\r\n", "\n");
        assert!(script.contains(
            "$measurementReceiptSchema = 'lattice.phase4-foreman-catalog-measurement.v1'"
        ));
        assert!(script.contains("schema = $receiptSchema"));
        assert!(script.contains("$expectedMeasurementShape = [ordered]@{"));
        assert!(script.contains("CREATE TABLE foreman_execution\\."));
        assert!(script.contains("CREATE FUNCTION foreman_execution\\."));
        assert!(script.contains("GRANT EXECUTE ON FUNCTION foreman_execution\\."));
        assert!(script.contains("$expectedMeasurementShape.function"));
        assert!(script.contains("$expectedMeasurementShape.runtime_execute"));
        assert!(!script.contains("function = 40"));
        assert!(!script.contains("runtime_execute = 37"));
        assert!(!script.contains("$functionCount -ne 38"));
        assert!(!script.contains("$runtimeExecuteCount -ne 35"));
    }

    pub(super) fn foreman_catalog_measurement_requested() -> bool {
        env::var("LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_LIVE").as_deref() == Ok("1")
    }

    fn measurement_error() -> LatticedError {
        LatticedError::new(LatticedErrorKind::RuntimePostgresVerification)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn measure_foreman_catalog_profile(
        migrator: &mut Client,
        database: &DeliveryDatabaseBinding,
        configured_admission: &RuntimeAdmissionSnapshot,
    ) -> Result<(), LatticedError> {
        let root = env::var("LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_ROOT")
            .map(PathBuf::from)
            .map_err(|_| measurement_error())?;
        let root = fs::canonicalize(root).map_err(|_| measurement_error())?;
        let temp = fs::canonicalize(env::temp_dir()).map_err(|_| measurement_error())?;
        let run_id = database.run_id();
        let expected_name = format!("lattice-phase4-catalog-measure-{run_id}");
        if root.parent() != Some(temp.as_path())
            || root.file_name().and_then(OsStr::to_str) != Some(expected_name.as_str())
        {
            return Err(measurement_error());
        }

        let marker_path = root.join(".phase4-catalog-measure-owner.json");
        let mut marker = fs::read_to_string(&marker_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .ok_or_else(measurement_error)?;
        let marker_root = marker
            .get("root")
            .and_then(Value::as_str)
            .ok_or_else(measurement_error)?;
        let port = env::var("LATTICE_TASK019_PORT")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(measurement_error)?;
        let database_name = database.database_name();
        if marker.get("owner").and_then(Value::as_str)
            != Some("LATTICE_PHASE4_FOREMAN_CATALOG_MEASUREMENT_V1")
            || marker.get("run_id").and_then(Value::as_str) != Some(run_id)
            || !measurement_same_path(Path::new(marker_root), &root)
            || marker.get("port").and_then(Value::as_u64) != Some(port)
        {
            return Err(measurement_error());
        }
        let marker_object = marker.as_object_mut().ok_or_else(measurement_error)?;
        marker_object.insert("database".to_owned(), Value::String(database_name.clone()));
        fs::write(
            &marker_path,
            serde_json::to_vec(&marker).map_err(|_| measurement_error())?,
        )
        .map_err(|_| measurement_error())?;

        let manifest = lattice_postgres_foreman::verify_embedded_extension()
            .map_err(|_| measurement_error())?;
        let sql = std::str::from_utf8(manifest.bytes()).map_err(|_| measurement_error())?;
        let admission = RuntimeAdmissionSnapshot::load(migrator)?;
        admission.stop(migrator)?;
        let applied = (|| {
            let mut transaction = migrator
                .build_transaction()
                .isolation_level(postgres::IsolationLevel::Serializable)
                .start()
                .map_err(|_| measurement_error())?;
            transaction
                .batch_execute(
                    "SET LOCAL search_path = pg_catalog; \
                     SET LOCAL row_security = on; \
                     SET LOCAL lock_timeout = '5s'; \
                     SET LOCAL statement_timeout = '30s'",
                )
                .map_err(|_| measurement_error())?;
            transaction
                .batch_execute(sql)
                .map_err(|_| measurement_error())?;
            transaction.commit().map_err(|_| measurement_error())
        })();
        match applied {
            Ok(()) => configured_admission.restore(migrator)?,
            Err(error) => {
                admission.restore(migrator)?;
                return Err(error);
            }
        }

        let shape = migrator
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                     WHERE n.nspname='foreman_execution' AND c.relkind='r'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                     WHERE n.nspname='foreman_execution'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                      JOIN pg_catalog.pg_roles AS r ON r.oid=c.relowner \
                     WHERE n.nspname='foreman_execution' AND c.relkind='r' \
                       AND r.rolname='lattice_migrator'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                      JOIN pg_catalog.pg_roles AS r ON r.oid=p.proowner \
                     WHERE n.nspname='foreman_execution' \
                       AND r.rolname='lattice_migrator' AND p.prosecdef \
                       AND p.proconfig=ARRAY['search_path=pg_catalog']::text[]), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                     WHERE n.nspname='foreman_execution' AND c.relkind='r' \
                       AND pg_catalog.has_table_privilege( \
                           'lattice_runtime',c.oid, \
                           'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                     WHERE n.nspname='foreman_execution' \
                       AND pg_catalog.has_function_privilege( \
                           'lattice_runtime',p.oid,'EXECUTE')), \
                    (SELECT EXISTS ( \
                        SELECT 1 FROM pg_catalog.pg_namespace AS n \
                        CROSS JOIN LATERAL pg_catalog.aclexplode( \
                            COALESCE(n.nspacl,pg_catalog.acldefault('n',n.nspowner))) AS a \
                        WHERE n.nspname='foreman_execution' AND a.grantee=0 \
                          AND a.privilege_type='USAGE')), \
                    pg_catalog.has_schema_privilege( \
                        'lattice_runtime','foreman_execution','USAGE'), \
                    pg_catalog.has_schema_privilege( \
                        'lattice_runtime','foreman_execution','CREATE')",
                &[],
            )
            .map_err(|_| measurement_error())?;
        let tables = shape.get::<_, i64>(0);
        let functions = shape.get::<_, i64>(1);
        let owned_tables = shape.get::<_, i64>(2);
        let hardened_functions = shape.get::<_, i64>(3);
        let runtime_tables = shape.get::<_, i64>(4);
        let runtime_functions = shape.get::<_, i64>(5);
        let public_usage = shape.get::<_, bool>(6);
        let runtime_usage = shape.get::<_, bool>(7);
        let runtime_create = shape.get::<_, bool>(8);
        if tables != owned_tables
            || functions != hardened_functions
            || runtime_tables != 0
            || public_usage
            || !runtime_usage
            || runtime_create
        {
            return Err(measurement_error());
        }

        let result = json!({
            "schema": "lattice.phase4-foreman-catalog-measurement.v1",
            "run_id": run_id,
            "database": database_name,
            "sql_bytes": manifest.byte_length(),
            "sql_sha256": manifest.sql_sha256().as_str(),
            "manifest_sha256": manifest.manifest_sha256().as_str(),
            "table_count": tables,
            "function_count": functions,
            "hardened_function_count": hardened_functions,
            "runtime_execute_count": runtime_functions,
        });
        fs::write(
            root.join("catalog-measurement.json"),
            serde_json::to_vec_pretty(&result).map_err(|_| measurement_error())?,
        )
        .map_err(|_| measurement_error())?;
        println!("FOREMAN_CATALOG_TABLE_COUNT={tables}");
        println!("FOREMAN_CATALOG_DATABASE={database_name}");
        println!("FOREMAN_CATALOG_FUNCTION_COUNT={functions}");
        println!("FOREMAN_CATALOG_HARDENED_FUNCTION_COUNT={hardened_functions}");
        println!("FOREMAN_CATALOG_RUNTIME_EXECUTE_COUNT={runtime_functions}");
        Ok(())
    }

    #[test]
    #[ignore = "requires a coordinator-owned disposable PostgreSQL 17 profile"]
    fn disposable_store_v7_foreman_catalog_measurement_profile() {
        assert!(
            foreman_catalog_measurement_requested(),
            "coordinator measurement admission"
        );
        bootstrap_postgres_extensions_from_environment()
            .expect("verified Store/Memory/Writer-v7 foundation and raw Foreman SQL measurement");
    }

    #[test]
    fn official_launcher_runs_provision_then_explicit_bootstrap_and_avoids_live_ports() {
        let launcher = include_str!("../../../scripts/start-lattice-runtime-postgres.ps1")
            .replace("\r\n", "\n");
        assert!(launcher.contains("$candidate -eq 5432 -or $candidate -eq 58743"));
        assert_eq!(launcher.matches("--postgres-initialize").count(), 2);
        assert_eq!(launcher.matches("--postgres-bootstrap").count(), 3);
        assert_eq!(
            launcher
                .matches(
                    "--postgres-initialize\n    if ($LASTEXITCODE -ne 0) { throw 'LATTICE_RUNTIME_POSTGRES_INITIALIZE_REJECTED' }\n    & $LatticedPath --postgres-bootstrap"
                )
                .count(),
            2
        );
    }

    #[test]
    fn verified_replay_precedes_writer_read_and_writer_state_only_degrades_readiness() {
        assert_eq!(foreman_writer_degraded_code(false), None);
        assert_eq!(
            foreman_writer_degraded_code(true),
            Some("FOREMAN_WRITER_CONTENTION")
        );

        let source = include_str!("composition.rs");
        let runtime_status = source
            .split("fn runtime_status_json")
            .nth(1)
            .expect("runtime status")
            .split("fn status_json")
            .next()
            .expect("runtime status body");
        let replay = runtime_status
            .find("load_runtime_status")
            .expect("verified foreman replay");
        let coordination_drop = runtime_status
            .find("drop(coordination)")
            .expect("foreman coordination release");
        let delivery = runtime_status
            .find("core_status_json")
            .expect("delivery base status read");
        let writer = runtime_status
            .find("foreman_writer_lease")
            .expect("writer adapter construction");
        let current = runtime_status
            .find("current_authority")
            .expect("writer current-authority read");
        assert!(
            replay < coordination_drop
                && coordination_drop < delivery
                && delivery < writer
                && writer < current
        );

        for kind in [
            WriterLeaseRepositoryErrorKind::Unavailable,
            WriterLeaseRepositoryErrorKind::SerializationExhausted,
            WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown,
        ] {
            assert_eq!(
                foreman_writer_observation_error(WriterLeaseRepositoryError::new(kind)).code(),
                "FOREMAN_REPLAY_UNAVAILABLE"
            );
        }
        for kind in [
            WriterLeaseRepositoryErrorKind::Domain,
            WriterLeaseRepositoryErrorKind::Corrupt,
            WriterLeaseRepositoryErrorKind::AuthorityMismatch,
        ] {
            assert_eq!(
                foreman_writer_observation_error(WriterLeaseRepositoryError::new(kind)).code(),
                "FOREMAN_REPLAY_CORRUPT"
            );
        }
        assert!(runtime_status.contains("map_err(foreman_writer_observation_error)"));
    }

    #[test]
    fn dependency_replay_precedes_writer_construction_and_inner_guard() {
        let source = include_str!("composition.rs");
        let checkpoint = source
            .split("fn foreman_checkpoint(")
            .nth(1)
            .expect("foreman checkpoint")
            .split("fn reconcile(")
            .next()
            .expect("foreman checkpoint body");
        let replay = checkpoint
            .find("replay_checkpoint")
            .expect("replay preflight");
        let writer = checkpoint
            .find("foreman_writer_lease")
            .expect("writer construction");
        let orchestrated = checkpoint
            .find("checkpoint_foreman")
            .expect("orchestrated checkpoint");
        let guard = checkpoint
            .rfind("validate_dependency_checkpoint")
            .expect("inner dependency Git guard");
        assert!(replay < writer && writer < orchestrated && orchestrated < guard);
        assert!(checkpoint.contains("let _replay_preflight"));
    }

    #[test]
    fn dependency_git_commands_disable_hooks_fsmonitor_and_optional_locks() {
        let source = include_str!("composition.rs");
        assert_eq!(
            source
                .matches(".env(\"GIT_OPTIONAL_LOCKS\", \"0\")")
                .count(),
            2
        );
        assert!(source.contains("core.hooksPath="));
        assert!(source.contains("core.fsmonitor=false"));
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn dependency_git_guard_blocks_dirty_or_unintegrated_resume_and_accepts_exact_merge() {
        static NEXT_DEPENDENCY_FIXTURE: AtomicUsize = AtomicUsize::new(0);
        let unique = NEXT_DEPENDENCY_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let fixture = env::temp_dir().join(format!(
            "lattice-dependency-guard-{}-{unique}",
            process::id()
        ));
        let repository = fixture.join("repository");
        let dependency_root = fixture.join("dependency-worktrees");
        let child = dependency_root.join("task-107-worktree");
        let marker_root = dependency_root.join(".lattice-ownership");
        let hooks_root = dependency_root.join(".lattice-hooks-empty");
        fs::create_dir_all(&repository).expect("repository root");
        fs::create_dir_all(&marker_root).expect("marker root");
        fs::create_dir_all(&hooks_root).expect("hooks root");
        let git = PathBuf::from("git");
        let run = |root: &Path, arguments: &[&str]| {
            let output = process::Command::new(&git)
                .current_dir(root)
                .args(arguments)
                .output()
                .expect("git process");
            assert!(
                output.status.success(),
                "git {:?}: {}",
                arguments,
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("git stdout")
                .trim()
                .to_owned()
        };
        run(&repository, &["init", "-b", "product-parent"]);
        run(&repository, &["config", "user.name", "LATTICE Test"]);
        run(
            &repository,
            &["config", "user.email", "lattice-test@invalid.example"],
        );
        fs::write(repository.join("base.txt"), b"base\n").expect("base file");
        run(&repository, &["add", "base.txt"]);
        run(&repository, &["commit", "-m", "base"]);
        let base = run(&repository, &["rev-parse", "HEAD"]);
        run(
            &repository,
            &[
                "worktree",
                "add",
                "-b",
                "lattice/task-107",
                child.to_str().expect("child path"),
                &base,
            ],
        );
        run(&child, &["config", "user.name", "LATTICE Test"]);
        run(
            &child,
            &["config", "user.email", "lattice-test@invalid.example"],
        );
        let repository = fs::canonicalize(&repository).expect("canonical repository");
        let dependency_root =
            fs::canonicalize(&dependency_root).expect("canonical dependency root");
        let child = fs::canonicalize(&child).expect("canonical child");
        let marker_path = marker_root.join("task-107-worktree.json");
        let marker_json = format!(
            "{{\"version\":1,\"worktree_id\":\"TASK-107-WORKTREE\",\"task_id\":\"TASK-107\",\"repository_root\":{},\"worktree_path\":{},\"branch\":\"lattice/task-107\",\"base_commit_sha\":\"{}\"}}\n",
            serde_json::to_string(&repository.to_string_lossy()).expect("repository JSON"),
            serde_json::to_string(&child.to_string_lossy()).expect("child JSON"),
            base
        );
        fs::write(&marker_path, &marker_json).expect("ownership marker");
        let binding = DependencyBinding::new(
            "TASK-106",
            "TASK-107",
            "TASK-107-WORKTREE",
            "lattice/task-107",
            &base,
            "COMPLETE_DEPENDENCY",
        )
        .expect("binding");
        fs::write(hooks_root.join("unexpected-hook"), b"prohibited\n").expect("unexpected hook");
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                None,
                DependencyGitPhase::Block,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Err("FOREMAN_DEPENDENCY_WORKTREE_UNSAFE")
        );
        fs::remove_file(hooks_root.join("unexpected-hook")).expect("remove unexpected hook");
        run(
            &repository,
            &["config", "core.fsmonitor", "lattice-missing-fsmonitor"],
        );
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                None,
                DependencyGitPhase::Block,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Ok(())
        );

        let mut drifted_marker = serde_json::from_str::<Value>(&marker_json).expect("marker JSON");
        drifted_marker["repository_root"] = Value::String(child.to_string_lossy().into_owned());
        fs::write(
            &marker_path,
            serde_json::to_vec(&drifted_marker).expect("drifted marker JSON"),
        )
        .expect("drift marker");
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                None,
                DependencyGitPhase::Block,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Err("FOREMAN_DEPENDENCY_BINDING_MISMATCH")
        );
        fs::write(&marker_path, &marker_json).expect("restore marker");

        let mut drifted_marker = serde_json::from_str::<Value>(&marker_json).expect("marker JSON");
        drifted_marker["worktree_path"] = Value::String(repository.to_string_lossy().into_owned());
        fs::write(
            &marker_path,
            serde_json::to_vec(&drifted_marker).expect("path-drift marker JSON"),
        )
        .expect("path-drift marker");
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                None,
                DependencyGitPhase::Block,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Err("FOREMAN_DEPENDENCY_BINDING_MISMATCH")
        );
        fs::write(&marker_path, &marker_json).expect("restore marker after path drift");

        let mut drifted_marker = serde_json::from_str::<Value>(&marker_json).expect("marker JSON");
        drifted_marker["base_commit_sha"] = Value::String("b".repeat(40));
        fs::write(
            &marker_path,
            serde_json::to_vec(&drifted_marker).expect("base-drift marker JSON"),
        )
        .expect("base-drift marker");
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                None,
                DependencyGitPhase::Block,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Err("FOREMAN_DEPENDENCY_BINDING_MISMATCH")
        );
        fs::write(&marker_path, &marker_json).expect("restore marker after base drift");

        run(&child, &["branch", "-m", "lattice/task-107-drift"]);
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                None,
                DependencyGitPhase::Block,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Err("FOREMAN_DEPENDENCY_BINDING_MISMATCH")
        );
        run(&child, &["branch", "-m", "lattice/task-107"]);

        fs::write(child.join("dirty.txt"), b"dirty\n").expect("dirty child");
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                None,
                DependencyGitPhase::Block,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Err("FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED")
        );
        fs::remove_file(child.join("dirty.txt")).expect("remove fixture dirt");

        let blocked = ForemanSnapshot::new(
            SoleForemanBinding::WORKER,
            SoleForemanBinding::THREAD,
            SoleForemanBinding::TASK,
            "product-parent",
            repository.to_string_lossy(),
            &base,
            ForemanState::Blocked,
            Some(binding.as_blocker_ref().to_owned()),
            format!("heartbeat:sha256:{}", "a".repeat(64)),
            format!("authority:sha256:{}", "c".repeat(64)),
            binding.evidence_ref(),
            1,
        )
        .expect("blocked snapshot");
        let dependency = reconstruct([blocked.clone()])
            .expect("projection")
            .dependency()
            .expect("dependency")
            .clone();
        fs::write(child.join("in-progress.txt"), b"in progress\n")
            .expect("in-progress child change");
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                Some(&dependency),
                DependencyGitPhase::ObserveBlocked,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Ok(())
        );
        fs::remove_file(child.join("in-progress.txt")).expect("remove in-progress fixture");
        fs::write(child.join("dependency.txt"), b"dependency\n").expect("dependency change");
        run(&child, &["add", "dependency.txt"]);
        run(&child, &["commit", "-m", "dependency"]);
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                Some(&dependency),
                DependencyGitPhase::Resume,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Err("FOREMAN_DEPENDENCY_NOT_INTEGRATED")
        );
        run(
            &repository,
            &[
                "merge",
                "--no-ff",
                "lattice/task-107",
                "-m",
                "merge dependency",
            ],
        );
        let captured_resume = verify_dependency_git_at(
            &binding,
            Some(&dependency),
            DependencyGitPhase::Resume,
            &repository,
            &dependency_root,
            &git,
        )
        .expect("captured resume observation");
        let merged_head = run(&repository, &["rev-parse", "HEAD"]);
        let resumed_snapshot = ForemanSnapshot::new(
            SoleForemanBinding::WORKER,
            SoleForemanBinding::THREAD,
            SoleForemanBinding::TASK,
            "product-parent",
            repository.to_string_lossy(),
            &merged_head,
            ForemanState::Active,
            None,
            format!("heartbeat:sha256:{}", "d".repeat(64)),
            format!("authority:sha256:{}", "f".repeat(64)),
            format!("evidence:sha256:{}", "e".repeat(64)),
            2,
        )
        .expect("resumed snapshot");
        let resumed_dependency = reconstruct([blocked, resumed_snapshot])
            .expect("resumed projection")
            .dependency()
            .expect("resumed dependency")
            .clone();
        fs::write(repository.join("in-progress-parent.txt"), b"in progress\n")
            .expect("in-progress parent change");
        assert!(
            verify_dependency_git_at(
                &binding,
                Some(&resumed_dependency),
                DependencyGitPhase::ObserveResumed,
                &repository,
                &dependency_root,
                &git,
            )
            .is_ok()
        );
        fs::remove_file(repository.join("in-progress-parent.txt"))
            .expect("remove in-progress parent change");
        fs::write(child.join("post-resume-child.txt"), b"unexpected\n")
            .expect("post-resume child change");
        assert_eq!(
            verify_dependency_git_at(
                &binding,
                Some(&resumed_dependency),
                DependencyGitPhase::ObserveResumed,
                &repository,
                &dependency_root,
                &git,
            )
            .map(|_| ()),
            Err("FOREMAN_DEPENDENCY_RECONCILIATION_REQUIRED")
        );
        fs::remove_file(child.join("post-resume-child.txt"))
            .expect("remove post-resume child change");
        fs::write(repository.join("continued.txt"), b"continued\n").expect("continued parent work");
        run(&repository, &["add", "continued.txt"]);
        run(&repository, &["commit", "-m", "continue parent"]);
        let active_intent = ForemanCheckpointIntent::new(
            "captured-resume",
            2,
            "2026-08-25T01:00:00Z",
            ForemanState::Active,
            None,
            format!("heartbeat:sha256:{}", "d".repeat(64)),
            format!("evidence:sha256:{}", "e".repeat(64)),
        )
        .expect("active intent");
        let captured_snapshot = captured_resume
            .into_server_observation()
            .expect("server observation")
            .bind(
                &active_intent,
                format!("authority:sha256:{}", "f".repeat(64)),
            )
            .expect("captured snapshot");
        assert_eq!(captured_snapshot.head(), merged_head);

        run(
            &repository,
            &["worktree", "remove", child.to_str().expect("child path")],
        );
        fs::remove_dir_all(&fixture).expect("remove dependency fixture");
    }

    #[test]
    fn postgres_bootstrap_cross_product_is_closed_before_effects() {
        use MemoryBootstrapProfile::{Empty, V2 as MemoryV2, V3 as MemoryV3};
        use MigrationBootstrapProfile::{Fresh, LegacyPrefix, V5, V6, V7, V8, V8LegacyPrefix};
        use PostgresBootstrapAction::{
            V4Apply, V5Apply, V6Rebind, V7Apply, V8Apply, V8VerifyOnly, WriterV5Apply,
            WriterV8Rebind,
        };
        use V3BootstrapProfile::{
            V5Bridge, V5FallbackRequired, V6BridgePending, V6Current, V6V4Bridge,
            V6V4BridgeLegacyF252Rebind, V7V4Current, V7V5Current, V7V5RebindPending, V8V5Current,
            V8V5RebindPending,
        };

        let accepted = [
            ((V5, Empty, V5FallbackRequired), V5Apply),
            ((V5, MemoryV2, V5FallbackRequired), V5Apply),
            ((V5, MemoryV3, V5FallbackRequired), V5Apply),
            ((V5, MemoryV3, V5Bridge), V5Apply),
            ((V6, MemoryV3, V6BridgePending), V6Rebind),
            ((V6, MemoryV3, V6Current), V4Apply),
            ((V6, MemoryV3, V6V4Bridge), V7Apply),
            ((V6, MemoryV3, V6V4BridgeLegacyF252Rebind), V4Apply),
            ((V7, MemoryV3, V7V4Current), WriterV5Apply),
            ((V7, MemoryV3, V7V5RebindPending), WriterV8Rebind),
            ((V7, MemoryV3, V7V5Current), V8Apply),
            (
                (V8LegacyPrefix, MemoryV3, V8V5RebindPending),
                WriterV8Rebind,
            ),
            ((V8LegacyPrefix, MemoryV3, V8V5Current), V8Apply),
            ((V8, MemoryV3, V8V5RebindPending), WriterV8Rebind),
            ((V8, MemoryV3, V8V5Current), V8VerifyOnly),
        ];
        for store in [Fresh, LegacyPrefix, V5, V6, V7, V8LegacyPrefix, V8] {
            for memory in [Empty, MemoryV2, MemoryV3] {
                for writer in [
                    V5FallbackRequired,
                    V5Bridge,
                    V6BridgePending,
                    V6Current,
                    V6V4Bridge,
                    V6V4BridgeLegacyF252Rebind,
                    V7V4Current,
                    V7V5RebindPending,
                    V7V5Current,
                    V8V5RebindPending,
                    V8V5Current,
                ] {
                    let expected = accepted.iter().find_map(
                        |((left_store, left_memory, left_writer), action)| {
                            (*left_store == store
                                && *left_memory == memory
                                && *left_writer == writer)
                                .then_some(*action)
                        },
                    );
                    assert_eq!(postgres_bootstrap_action(store, memory, writer), expected);
                }
            }
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn initialize_is_provisioning_only_and_bootstrap_converges_once_through_store_v8() {
        let source = include_str!("composition.rs");
        let initialize = source
            .split("pub fn initialize_runtime_postgres_from_environment")
            .nth(1)
            .expect("initialize")
            .split("fn connect_migrator")
            .next()
            .expect("initialize body");
        assert!(!initialize.contains("apply_store_migrations"));
        assert!(!initialize.contains("bootstrap_postgres_extensions_from_environment()"));

        let bootstrap = source
            .split("pub fn bootstrap_postgres_extensions_from_environment")
            .nth(1)
            .expect("bootstrap")
            .split("pub fn initialize_runtime_postgres_from_environment")
            .next()
            .expect("bootstrap body");
        let first_store = bootstrap
            .find("apply_store_migrations")
            .expect("v5 foundation");
        let writer_absence = bootstrap
            .find("inspect_v3_bootstrap_profile")
            .expect("fresh Writer absence gate");
        let memory_inspection = bootstrap
            .find("inspect_bootstrap_profile")
            .expect("Memory-owned closed bootstrap profile");
        let writer_inspection = memory_inspection
            + bootstrap[memory_inspection..]
                .find("inspect_v3_bootstrap_profile")
                .expect("Writer-owned closed bootstrap profile");
        let writer_v3 = bootstrap
            .find("apply_v3_extension")
            .expect("Writer v3 bridge");
        let writer_v4 = bootstrap
            .find("apply_v4_extension")
            .expect("Writer v4 bridge");
        let writer_v5 = bootstrap
            .find("apply_v5_extension")
            .expect("Writer v5 process-handoff bridge");
        let writer_v8 = bootstrap
            .find("rebind_v5_for_store_v8")
            .expect("Writer v5 dual Store-v7/v8 runtime successor");
        let generic_v5_verifier = bootstrap
            .find("verify_store_schema")
            .expect("generic Store v5 fallback verifier");
        let memory_fallback = bootstrap
            .find("apply_postgres_memory_extension")
            .expect("Memory fresh-install fallback");
        let writer_v2_fallback = bootstrap
            .find("apply_postgres_writer_extension")
            .expect("Writer v2 fresh-install fallback");
        let memory_fallback_verify = bootstrap
            .find("verify_memory_extension")
            .expect("Memory fallback verification");
        let final_store = bootstrap.rfind("apply_store_migrations").expect("Store v8");
        let legacy_rejection = bootstrap
            .find("if profile == MigrationBootstrapProfile::LegacyPrefix")
            .expect("legacy product-bootstrap rejection");
        assert!(legacy_rejection < writer_absence && writer_absence < first_store);
        assert!(first_store < memory_inspection && memory_inspection < writer_inspection);
        assert!(writer_inspection < writer_v3);
        assert!(
            writer_v3 < writer_v4
                && writer_v4 < writer_v5
                && writer_v5 < writer_v8
                && writer_v8 < final_store
        );
        assert!(writer_v3 < generic_v5_verifier && generic_v5_verifier < memory_fallback);
        assert!(memory_fallback < writer_v2_fallback);
        assert!(writer_v2_fallback < memory_fallback_verify);
        assert!(bootstrap.contains("V3BootstrapProfile::V5FallbackRequired"));
        assert!(bootstrap.contains("V3BootstrapProfile::V5Bridge"));
        assert!(bootstrap.contains("V3BootstrapProfile::V6BridgePending"));
        assert!(bootstrap.contains("V3BootstrapProfile::V6Current"));
        assert!(bootstrap.contains("V3BootstrapProfile::V6V4Bridge"));
        assert!(bootstrap.contains("V3BootstrapProfile::V6V4BridgeLegacyF252Rebind"));
        assert!(bootstrap.contains("V3BootstrapProfile::V7V4Current"));
        assert!(bootstrap.contains("V3BootstrapProfile::V7V5RebindPending"));
        assert!(bootstrap.contains("V3BootstrapProfile::V7V5Current"));
        assert!(bootstrap.contains("V3BootstrapProfile::V8V5RebindPending"));
        assert!(bootstrap.contains("MemoryBootstrapGlobalProfile::V7"));
        assert!(bootstrap.contains("MemoryBootstrapGlobalProfile::V8LegacyPrefix"));
        assert!(bootstrap.contains("MemoryBootstrapGlobalProfile::V8"));
        assert!(bootstrap.contains("PostgresBootstrapAction::V7Apply"));
        assert!(bootstrap.contains("PostgresBootstrapAction::WriterV5Apply"));
        assert!(bootstrap.contains("PostgresBootstrapAction::WriterV8Rebind"));
        assert!(bootstrap.contains("PostgresBootstrapAction::V8Apply"));
        assert!(bootstrap.contains("PostgresBootstrapAction::V8VerifyOnly"));
        assert!(bootstrap.contains("final_store.schema_version() != 8"));
        assert!(bootstrap.contains("PostgresBootstrapAction::V4Apply"));
        assert!(bootstrap.contains("for _ in 0..6"));
        assert!(bootstrap.contains("!= WriterExtensionApplyOutcome::Bridged"));
        assert!(bootstrap.contains("admission != configured_admission"));
        assert!(bootstrap.contains("admission.is_stopped_no_leader()"));
        assert_eq!(bootstrap.matches("apply_v3_extension").count(), 2);
        let admission_stop = bootstrap.find("admission.stop").expect("admission stop");
        assert!(legacy_rejection < admission_stop);
        let outer_gate = bootstrap
            .find("acquire_postgres_bootstrap_gate")
            .expect("whole-bootstrap advisory gate");
        let initial_inspection = bootstrap
            .find("inspect_migration_profile")
            .expect("initial Store inspection");
        assert!(outer_gate < initial_inspection);
        let v6_rebind = bootstrap
            .find("rebind_existing_v3_extension")
            .expect("Writer-owned strict existing-v3 rebind");
        assert!(admission_stop < v6_rebind);
        assert!(bootstrap.contains("inspect_migration_profile"));
        assert!(bootstrap.contains("drop(migrator);"));
        let foreman_apply = bootstrap
            .find("apply_postgres_foreman_extension")
            .expect("Foreman V8 install/rebind");
        let runtime_verify = bootstrap
            .rfind("verify_postgres_runtime_gates")
            .expect("fresh Runtime-role replay and Foreman verification");
        let final_restore = bootstrap
            .rfind("configured_admission.restore")
            .expect("single terminal admission restore");
        let outer_release = bootstrap
            .rfind("release_postgres_bootstrap_gate")
            .expect("outer gate release");
        let migrator_drop = bootstrap
            .rfind("drop(migrator)")
            .expect("migrator credential close");
        assert!(final_store < foreman_apply);
        assert!(foreman_apply < final_restore);
        assert!(final_restore < outer_release && outer_release < migrator_drop);
        assert!(migrator_drop < runtime_verify);
        assert_eq!(bootstrap.matches("configured_admission.restore").count(), 1);
        assert_eq!(bootstrap.matches(".restore(&mut migrator)").count(), 1);
        let production = source
            .split("#[cfg(test)]\nmod tests")
            .next()
            .expect("production composition");
        assert_eq!(
            production.matches("PostgresWriterLease::new_v5_v7").count(),
            2
        );

        let ordinary_start = source
            .split("fn assemble_full_chain_service_with_mode")
            .nth(1)
            .expect("ordinary service assembly")
            .split("fn validate_controlled_task_timeout")
            .next()
            .expect("ordinary service assembly body");
        assert!(!ordinary_start.contains("apply_store_migrations"));
        assert!(!ordinary_start.contains("apply_v3_extension"));
        let replay = ordinary_start
            .find("load_runtime_status")
            .expect("foreman replay verification");
        let serve = ordinary_start
            .find("Ok((")
            .expect("service construction result");
        assert!(replay < serve);
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
            "expected_status": task_public_status(evidence, task_ref, None, None),
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
            TaskLifecyclePort::admit(&mut lifecycle, &binding, client_request_id)?;
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
    fn managed_scripted_restart_selector_is_resume_active_and_exact_only() {
        let exact = ManagedScriptedRestartSelectorInput {
            run_mode: FullChainRunMode::ResumeExisting,
            runtime: DeliveryRuntime::ScriptedAcceptance,
            enabled: Some("1"),
            foreman_mode: Some("ACTIVE"),
        };
        assert_eq!(select_managed_scripted_active_restart(exact), Ok(true));
        assert_eq!(
            select_managed_scripted_active_restart(ManagedScriptedRestartSelectorInput {
                runtime: DeliveryRuntime::OfficialCodexAppServer,
                ..exact
            }),
            Ok(false)
        );
        assert_eq!(
            select_managed_scripted_active_restart(ManagedScriptedRestartSelectorInput {
                enabled: None,
                ..exact
            }),
            Ok(false)
        );
        for rejected in [
            ManagedScriptedRestartSelectorInput {
                run_mode: FullChainRunMode::Fresh,
                ..exact
            },
            ManagedScriptedRestartSelectorInput {
                enabled: Some("true"),
                ..exact
            },
            ManagedScriptedRestartSelectorInput {
                foreman_mode: Some("DISABLED"),
                ..exact
            },
            ManagedScriptedRestartSelectorInput {
                foreman_mode: None,
                ..exact
            },
        ] {
            assert_eq!(
                select_managed_scripted_active_restart(rejected),
                Err(LatticedErrorKind::Configuration)
            );
        }
    }

    #[test]
    fn managed_scripted_acceptance_binding_rejects_origin_project_and_worktree_substitution_before_effect()
     {
        use std::cell::Cell;

        let owner_root = Path::new("/phase4-owner");
        let project_root = owner_root.join("repository");
        let worktree_root = owner_root.join("managed-worktrees");
        let exact = ManagedScriptedAcceptanceBindingInput {
            owner_root,
            control_port: 31_337,
            control_origin: "http://127.0.0.1:31337",
            marker_project_root: &project_root,
            configured_project_root: &project_root,
            configured_worktree_root: &worktree_root,
        };
        let binding = select_managed_scripted_acceptance_binding(exact).expect("exact binding");
        assert_eq!(binding.owner_root, owner_root);
        assert_eq!(binding.control_origin, "http://127.0.0.1:31337");
        assert_eq!(binding.project_root, project_root);
        assert_eq!(binding.managed_worktree_root, worktree_root);

        let foreign_project = Path::new("/foreign/repository");
        let foreign_worktrees = Path::new("/foreign/managed-worktrees");
        for substituted in [
            ManagedScriptedAcceptanceBindingInput {
                control_origin: "http://127.0.0.1:31338",
                ..exact
            },
            ManagedScriptedAcceptanceBindingInput {
                configured_project_root: foreign_project,
                ..exact
            },
            ManagedScriptedAcceptanceBindingInput {
                configured_worktree_root: foreign_worktrees,
                ..exact
            },
        ] {
            let mutations = Cell::new(0_u8);
            let result = select_managed_scripted_acceptance_binding(substituted).map(|_| {
                mutations.set(mutations.get() + 1);
            });
            assert_eq!(result, Err(LatticedErrorKind::ScriptedFixtureRejected));
            assert_eq!(mutations.get(), 0);
        }
    }

    #[test]
    fn managed_scripted_admission_wires_actual_run_mode_and_precedes_task_mutation() {
        let source = include_str!("composition.rs");
        let environment = source
            .split("fn delivery_environment_for_mode")
            .nth(1)
            .expect("delivery environment")
            .split("enum PostgresBootstrapAction")
            .next()
            .expect("delivery environment body");
        assert!(environment.contains(
            "validate_managed_scripted_active_restart_admission(\n                run_mode,"
        ));

        let validator = source
            .split("fn validate_managed_scripted_active_restart_admission")
            .nth(1)
            .expect("scripted validator")
            .split("enum RuntimeIntegrationMode")
            .next()
            .expect("scripted validator body");
        assert!(validator.contains("run_mode,"));
        assert!(!validator.contains("run_mode: FullChainRunMode::ResumeExisting"));

        let schedule = source
            .split("fn schedule_managed_general_task")
            .nth(1)
            .expect("managed schedule")
            .split("fn load_managed_scheduled_task_from_durable")
            .next()
            .expect("managed schedule body");
        let binding = schedule
            .find("managed_scripted_acceptance")
            .expect("scripted path binding");
        let enqueue = schedule
            .find("accept_durable_scheduler_task(")
            .expect("bounded durable intake enqueue");
        assert!(binding < enqueue);
        assert!(!schedule.contains("promote_managed_task("));
    }

    #[cfg(windows)]
    fn scripted_effect_bundle_fixture(
        label: &str,
    ) -> (PathBuf, PathBuf, PathBuf, ManagedEffectBundleGuard) {
        static NEXT_SCRIPT_SEAL: AtomicUsize = AtomicUsize::new(0);
        let root = env::temp_dir().join(format!(
            "lattice-scripted-effect-seal-{label}-{}-{}",
            process::id(),
            NEXT_SCRIPT_SEAL.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("script seal fixture");
        let launcher = root.join("scripted-codex.cmd");
        let server = root.join("scripted-codex.ps1");
        fs::write(&server, SCRIPTED_SERVER_BYTES).expect("scripted server");
        let server_sha256 = file_sha256(&server, MAX_SCRIPTED_SERVER_BYTES)
            .expect("embedded scripted server digest");
        fs::write(&launcher, scripted_launcher_bytes(&server_sha256))
            .expect("exact scripted launcher");
        let config = LatticedDeliveryConfig {
            launcher: launcher.clone(),
            version: "scripted".to_owned(),
            launcher_sha256: file_sha256(&launcher, MAX_SCRIPTED_LAUNCHER_BYTES)
                .expect("scripted launcher digest"),
            schema_directory: root.join("schema"),
            codex_home: root.join("codex-home"),
            delivery_root: root.join("delivery"),
            git_executable: root.join("git.exe"),
            timeout: Duration::from_secs(30),
            runtime: DeliveryRuntime::ScriptedAcceptance,
            official_bundle: None,
        };
        let guard = managed_foreman_effect_bundle_guard(&config, &launcher)
            .expect("sealed scripted launcher and server bundle");
        (root, launcher, server, guard)
    }

    #[cfg(windows)]
    fn mark_provider_effect_if_substitution_survived(
        mutation_succeeded: bool,
        guard: &ManagedEffectBundleGuard,
        marker: &Path,
        provider_effects: &AtomicUsize,
    ) {
        if mutation_succeeded && guard.verify().is_ok() {
            provider_effects.fetch_add(1, Ordering::SeqCst);
            fs::write(marker, b"provider-effect").expect("record unsafe provider effect");
        }
    }

    #[cfg(windows)]
    #[test]
    fn managed_scripted_server_is_sealed_from_assembly_through_provider_lifetime() {
        let (root, _launcher, server, guard) = scripted_effect_bundle_fixture("server");
        let marker = root.join("substitution-effect.txt");
        let replacement = fs::write(&server, b"malicious provider effect\n");
        if replacement.is_ok() {
            fs::write(&marker, b"effect").expect("record unsafe replacement");
        }
        assert!(replacement.is_err(), "held server handle must deny write");
        assert!(
            fs::remove_file(&server).is_err(),
            "held server handle must deny delete"
        );
        assert!(
            !marker.exists(),
            "substituted PowerShell effect must be absent"
        );
        guard.verify().expect("scripted dependency remains exact");
        drop(guard);
        fs::remove_dir_all(root).expect("remove script seal fixture");
    }

    #[cfg(windows)]
    #[test]
    fn managed_scripted_launcher_replace_is_denied_before_provider_effect() {
        let (root, launcher, _server, guard) = scripted_effect_bundle_fixture("cmd-replace");
        let marker = root.join("provider-effect.txt");
        let provider_effects = AtomicUsize::new(0);
        let replacement = fs::write(&launcher, b"@echo malicious provider effect\r\n");
        mark_provider_effect_if_substitution_survived(
            replacement.is_ok(),
            &guard,
            &marker,
            &provider_effects,
        );

        assert!(
            replacement.is_err(),
            "held launcher handle must deny replacement writes"
        );
        assert_eq!(provider_effects.load(Ordering::SeqCst), 0);
        assert!(!marker.exists(), "substituted launcher must have no effect");
        guard.verify().expect("scripted bundle remains exact");
        drop(guard);
        fs::remove_dir_all(root).expect("remove launcher replace fixture");
    }

    #[cfg(windows)]
    #[test]
    fn managed_scripted_launcher_delete_is_denied_before_provider_effect() {
        let (root, launcher, _server, guard) = scripted_effect_bundle_fixture("cmd-delete");
        let marker = root.join("provider-effect.txt");
        let provider_effects = AtomicUsize::new(0);
        let deletion = fs::remove_file(&launcher);
        mark_provider_effect_if_substitution_survived(
            deletion.is_ok(),
            &guard,
            &marker,
            &provider_effects,
        );

        assert!(deletion.is_err(), "held launcher handle must deny deletion");
        assert_eq!(provider_effects.load(Ordering::SeqCst), 0);
        assert!(!marker.exists(), "deleted launcher must have no effect");
        guard.verify().expect("scripted bundle remains exact");
        drop(guard);
        fs::remove_dir_all(root).expect("remove launcher delete fixture");
    }

    #[cfg(windows)]
    #[test]
    fn managed_scripted_launcher_rename_is_denied_before_provider_effect() {
        let (root, launcher, _server, guard) = scripted_effect_bundle_fixture("cmd-rename");
        let renamed = root.join("renamed-scripted-codex.cmd");
        let marker = root.join("provider-effect.txt");
        let provider_effects = AtomicUsize::new(0);
        let rename = fs::rename(&launcher, &renamed);
        mark_provider_effect_if_substitution_survived(
            rename.is_ok(),
            &guard,
            &marker,
            &provider_effects,
        );

        assert!(rename.is_err(), "held launcher ancestry must deny rename");
        assert_eq!(provider_effects.load(Ordering::SeqCst), 0);
        assert!(!marker.exists(), "renamed launcher must have no effect");
        assert!(!renamed.exists(), "rename target must not be created");
        guard.verify().expect("scripted bundle remains exact");
        drop(guard);
        fs::remove_dir_all(root).expect("remove launcher rename fixture");
    }

    #[cfg(windows)]
    #[test]
    fn managed_scripted_launcher_aba_is_denied_before_provider_effect() {
        let (root, launcher, _server, guard) = scripted_effect_bundle_fixture("cmd-aba");
        let retained = root.join("retained-scripted-codex.cmd");
        let malicious = root.join("malicious-scripted-codex.cmd");
        let marker = root.join("provider-effect.txt");
        fs::write(&malicious, b"@echo malicious provider effect\r\n")
            .expect("malicious ABA candidate");
        let provider_effects = AtomicUsize::new(0);
        let aba = fs::rename(&launcher, &retained)
            .and_then(|()| fs::rename(&malicious, &launcher))
            .and_then(|()| fs::rename(&retained, &launcher));
        mark_provider_effect_if_substitution_survived(
            aba.is_ok(),
            &guard,
            &marker,
            &provider_effects,
        );

        assert!(
            aba.is_err(),
            "held launcher ancestry must close the ABA lane"
        );
        assert_eq!(provider_effects.load(Ordering::SeqCst), 0);
        assert!(!marker.exists(), "ABA launcher must have no effect");
        assert!(launcher.exists(), "original launcher path remains bound");
        assert!(!retained.exists(), "original launcher was never displaced");
        guard.verify().expect("scripted bundle remains exact");
        drop(guard);
        fs::remove_dir_all(root).expect("remove launcher ABA fixture");
    }

    #[test]
    fn managed_resume_reloads_official_bundle_and_binds_it_into_service_effects() {
        let source = include_str!("composition.rs");
        let helper = source
            .split("fn managed_foreman_effect_bundle_guard")
            .nth(1)
            .expect("managed effect helper")
            .split("fn gateway_submission_from_environment")
            .next()
            .expect("managed effect helper body");
        assert!(helper.contains("validate_official_codex_identity("));
        assert!(helper.contains("LATTICE_DELIVERY_LAUNCHER_VERSION"));
        assert!(helper.contains("LATTICE_DELIVERY_LAUNCHER_SHA256"));
        assert!(helper.contains("bundle.managed_effect_guard()"));
        let assembly = source
            .split("fn managed_foreman_service_from_environment")
            .nth(1)
            .expect("managed service assembly")
            .split("fn managed_foreman_effect_bundle_guard")
            .next()
            .expect("managed service assembly body");
        let capture = assembly
            .find("managed_foreman_effect_bundle_guard")
            .expect("effect guard capture");
        let service = assembly
            .find("ManagedForemanServiceConfig::new")
            .expect("service config construction");
        let bind = assembly
            .find("with_effect_bundle_guard")
            .expect("service guard binding");
        assert!(capture < service && service < bind);
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

    struct IdleOpenClawPump;

    impl FullChainOpenClawPump for IdleOpenClawPump {
        fn pump_once(&mut self) -> Result<(), OpenClawPumpFailure> {
            thread::yield_now();
            Ok(())
        }
    }

    struct NeverEofTestInput {
        started: Option<mpsc::SyncSender<()>>,
        release: mpsc::Receiver<()>,
        completed: mpsc::SyncSender<()>,
    }

    impl Read for NeverEofTestInput {
        fn read(&mut self, _output: &mut [u8]) -> io::Result<usize> {
            if let Some(started) = self.started.take() {
                started.send(()).expect("announce blocked stdin");
            }
            self.release.recv().expect("release blocked stdin");
            self.completed.send(()).expect("announce stdin exit");
            Ok(0)
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
        let immediate_exit = ["process", "::exit(2)"].concat();

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
        assert!(!source.contains(&immediate_exit));
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
    fn retained_ingress_claim_is_key_scoped_and_rechecks_general_commit_before_resolution() {
        use std::cell::Cell;
        use std::collections::BTreeMap;

        let retained = BTreeMap::from([(
            "client-key-a",
            TaskIngressRequestKind::ControlledCodexCanary,
        )]);

        let different_key_reload_calls = Cell::new(0_u8);
        let different_key = general_submission_after_ingress_preflight(
            retained.get("client-key-b").copied(),
            || {
                different_key_reload_calls.set(different_key_reload_calls.get() + 1);
                Ok(Some("must-not-reload"))
            },
        )
        .expect("a canary claim under key A must not block general key B");
        assert_eq!(different_key, None);
        assert_eq!(different_key_reload_calls.get(), 0);

        let same_key_reload_calls = Cell::new(0_u8);
        let same_key = general_submission_after_ingress_preflight(
            retained.get("client-key-a").copied(),
            || {
                same_key_reload_calls.set(same_key_reload_calls.get() + 1);
                Ok(Some("must-not-reload"))
            },
        )
        .expect_err("general-after-canary must reject the shared idempotency key");
        assert_eq!(same_key.code(), "LATTICE_TASK_IDEMPOTENCY_CONFLICT");
        assert_eq!(same_key_reload_calls.get(), 0);

        let raced_general_reload_calls = Cell::new(0_u8);
        let raced_general = general_submission_after_ingress_preflight(
            Some(TaskIngressRequestKind::GeneralTask),
            || {
                raced_general_reload_calls.set(raced_general_reload_calls.get() + 1);
                Ok(Some("existing-task"))
            },
        )
        .expect("a task committed between snapshots must become exact replay");
        assert_eq!(raced_general, Some("existing-task"));
        assert_eq!(raced_general_reload_calls.get(), 1);

        let orphaned_general_reload_calls = Cell::new(0_u8);
        let orphaned_general = general_submission_after_ingress_preflight(
            Some(TaskIngressRequestKind::GeneralTask),
            || {
                orphaned_general_reload_calls.set(orphaned_general_reload_calls.get() + 1);
                Ok(None::<&str>)
            },
        )
        .expect_err("a general claim without its verified envelope is retained corruption");
        assert_eq!(orphaned_general.code(), "LATTICE_TASK_LEDGER_CORRUPT");
        assert_eq!(orphaned_general_reload_calls.get(), 1);

        let source = include_str!("composition.rs");
        let task_submit = source
            .split("    fn task_submit(")
            .nth(1)
            .expect("Task Submit composition")
            .split("    fn task_status(")
            .next()
            .expect("Task Submit body");
        let envelope_lookups = task_submit
            .match_indices("load_general_submission_by_request")
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>();
        assert!(envelope_lookups.len() >= 2);
        let neutral_claim_lookup = task_submit
            .find("load_task_ingress_request_kind_by_request")
            .expect("neutral ingress-claim lookup");
        let guarded_resolution = task_submit
            .find("general_submission_after_ingress_preflight")
            .expect("claim-before-resolution gate");
        let project_resolver = task_submit
            .find("resolve_registered_project_for_general_submit")
            .expect("registered-project resolver");
        assert!(
            envelope_lookups[0] < neutral_claim_lookup
                && neutral_claim_lookup < guarded_resolution
                && guarded_resolution < envelope_lookups[1]
                && envelope_lookups[1] < project_resolver
        );
    }

    #[test]
    fn managed_status_core_lock_wait_is_bounded_by_the_request_deadline() {
        let core = Mutex::new(());
        let _held = core.lock().expect("test lock");
        let started = Instant::now();
        let error = lock_task_status_core_until(&core, Some(started + Duration::from_millis(50)))
            .expect_err("a held core lock must fail closed at the managed status deadline");

        assert_eq!(error.code(), MANAGED_STATUS_TIMEOUT);
        assert!(started.elapsed() >= Duration::from_millis(40));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn external_verified_adoption_never_enters_general_submission_or_scheduler_paths() {
        let source = include_str!("composition.rs");
        let task_submit = source
            .split("    fn task_submit(")
            .nth(1)
            .expect("Task Submit composition")
            .split("    fn task_status(")
            .next()
            .expect("Task Submit body");
        let adoption_gate = task_submit
            .find("verified_result_adoption()")
            .expect("adoption gate");
        let general_lookup = task_submit
            .find("load_general_submission_by_request")
            .expect("general intake path");
        assert!(adoption_gate < general_lookup);
        assert!(!task_submit[..adoption_gate].contains("schedule_managed_general_task"));
    }

    #[test]
    fn managed_status_is_read_only_and_dispatch_uses_a_fixed_four_worker_pool() {
        assert_eq!(MANAGED_SUPERVISOR_WORKERS, 4);
        let source = include_str!("composition.rs");
        let task_status = source
            .split("    fn task_status(")
            .nth(1)
            .expect("Task Status composition")
            .split("impl<H: FullChainHermesPort> GatewayService")
            .next()
            .expect("Task Status body");
        let request_start = task_status
            .find("let request_started = Instant::now()")
            .expect("status request starts before lock acquisition");
        let bounded_lock = task_status
            .find("lock_task_status_core_until")
            .expect("deadline-bound core lock");
        let deadline = task_status
            .find("begin_status_request_at(request_started)")
            .expect("one managed status deadline");
        let intake = task_status
            .find("load_general_submission_by_task_ref_at")
            .expect("deadline-bound intake lookup");
        assert!(request_start < bounded_lock && bounded_lock < deadline && deadline < intake);
        assert!(task_status.contains("general_task_lifecycle_at"));
        assert!(task_status.contains("Some(operation_deadline) => task_lifecycle_at"));
        assert!(task_status.contains("verified_task_status_at"));
        assert!(task_status.contains("status_config.as_ref()"));
        let status = source
            .split("fn managed_general_task_public_status")
            .nth(1)
            .expect("managed status")
            .split("fn schedule_managed_general_task")
            .next()
            .expect("managed status body");
        assert!(!status.contains("schedule_managed_general_task("));
        assert!(!status.contains("run_managed_task("));
        let project = status
            .find("load_registered_project_for_general_status")
            .expect("read-only durable Project Registry lookup for pinned scope replay");
        let replay = status
            .find("managed_task_public_status(")
            .expect("read-only managed status replay");
        assert!(project < replay);
        assert!(status.contains("require_managed_project_status_projection"));

        let schedule = source
            .split("fn schedule_managed_general_task")
            .nth(1)
            .expect("managed scheduler entry")
            .split("fn load_managed_scheduled_task_from_durable")
            .next()
            .expect("scheduler entry body");
        assert!(schedule.contains("managed_scheduler"));
        let enqueue = schedule
            .find("accept_durable_scheduler_task(")
            .expect("bounded supervisor enqueue");
        assert!(enqueue > 0);
        assert!(!schedule.contains("promote_managed_task("));
        assert!(!schedule.contains("thread::Builder::new"));
        let replay_schedule = source
            .split("fn replay_general_submission_and_schedule")
            .nth(1)
            .expect("execution replay schedule")
            .split("enum GeneralWinnerReplay")
            .next()
            .expect("execution replay schedule body");
        assert!(replay_schedule.contains("resolve_registered_project_for_general_submission"));
        let execution_resolver = source
            .split("fn resolve_registered_project_for_general_submission")
            .nth(1)
            .expect("execution Project resolver")
            .split("fn load_registered_project_for_general_status")
            .next()
            .expect("execution Project resolver body");
        assert!(execution_resolver.contains("resolve_project_authority("));
        assert!(execution_resolver.contains("ProjectSelector::new"));
        assert!(execution_resolver.contains("registered_project_matches_general_submission"));
        assert!(execution_resolver.contains("PROJECT_REGISTRY_CURRENTNESS_CONFLICT"));
        let status_lookup = source
            .split("fn load_registered_project_for_general_status")
            .nth(1)
            .expect("status-only Project Registry lookup")
            .split("fn registered_project_matches_general_submission")
            .next()
            .expect("status-only Project Registry lookup body");
        assert!(status_lookup.contains("PostgresProjectRegistry::new"));
        assert!(status_lookup.contains("let loaded = registry"));
        assert!(status_lookup.contains(".load()"));
        assert!(!status_lookup.contains("resolve_project_authority("));
        assert!(!status_lookup.contains("ProjectSelector::new"));
        assert!(!status_lookup.contains("RegistryCommand"));
        assert!(!status_lookup.contains("ObservedEffectKind::Filesystem"));
        assert!(!status_lookup.contains("ObservedEffectKind::Process"));
        assert!(!status_lookup.contains("fs::canonicalize"));

        let supervisor = source
            .split("fn managed_scheduler")
            .nth(1)
            .expect("managed fixed supervisor")
            .split("fn stage_managed_restart_tasks")
            .next()
            .expect("managed supervisor body");
        assert!(supervisor.contains("0..MANAGED_SUPERVISOR_WORKERS"));
        assert!(supervisor.contains("mpsc::sync_channel::<ManagedScheduledTask>("));
        assert_eq!(MANAGED_SCHEDULER_QUEUE_CAPACITY, 64);
        let reload = supervisor
            .find("reload_managed_foreman_identity(&identity_source)")
            .expect("latest durable foreman identity reload before claim");
        let claim = supervisor
            .find("run_managed_task(")
            .expect("managed claim and dispatch");
        let durable_status = supervisor
            .find("managed_task_public_status(")
            .expect("fresh PostgreSQL disposition after a run failure");
        let release = supervisor
            .find("release_managed_schedule(&scheduled, &task_ref)")
            .expect("scheduled key release after supervised recovery");
        let after_release = &supervisor[release..];
        let shutdown_disposition = after_release
            .find("match exit")
            .expect("shutdown disposition before any post-run refill");
        let post_run_rescan = after_release
            .find("claim_managed_durable_rescan(")
            .expect("post-run durable rescan");
        assert!(reload < claim);
        assert!(claim < durable_status && durable_status < release);
        assert!(shutdown_disposition < post_run_rescan);
        assert!(!schedule.contains("foreman_identity"));
        assert!(supervisor.contains("FOREMAN_GLOBAL_CAPACITY_EXHAUSTED"));
        assert!(supervisor.contains("FOREMAN_TASK_CAPACITY_EXHAUSTED"));
        assert!(supervisor.contains("managed_status_is_durably_closed"));
        assert!(supervisor.contains("managed_status_is_durably_deferred"));
        assert!(supervisor.contains("ManagedSchedulerOwner"));
        assert!(supervisor.contains("workers.push("));
        assert!(supervisor.contains("cancellation.is_requested()"));
        assert!(supervisor.contains("claim_managed_durable_rescan("));
        assert!(supervisor.contains("next_durable_rescan"));
        assert!(supervisor.contains("refill_managed_scheduler_from_durable("));
        assert!(!supervisor.contains("_ => break"));

        let owner = source
            .split("impl ManagedSchedulerOwner")
            .nth(1)
            .expect("scheduler owner")
            .split("struct ManagedForemanIdentitySource")
            .next()
            .expect("scheduler owner body");
        assert!(owner.contains("if !self.armed"));
        assert!(owner.contains("MANAGED_SCHEDULER_SHUTDOWN_DEADLINE"));
        assert!(owner.contains("impl Drop for ManagedSchedulerOwner"));
        assert!(owner.contains("completion.recv_timeout(remaining)"));
        assert!(owner.contains("process::abort()"));
        let cancel = owner
            .find("self.cancellation.request()")
            .expect("scheduler cancellation");
        let close = owner.find("self.sender.take()").expect("queue close");
        let join = owner.find("handle.join()").expect("worker join");
        assert!(cancel < close && close < join);

        let restart = source
            .split("fn stage_managed_restart_tasks")
            .nth(1)
            .expect("managed restart discovery")
            .split("fn push_managed_restart_task")
            .next()
            .expect("managed restart discovery body");
        assert!(restart.contains("walk_restart_keyset_pages("));
        assert!(restart.contains("list_restart_task_refs_page(cursor, page_limit)"));
        assert!(restart.contains("MANAGED_RESTART_TASK_LIMIT"));
        assert!(restart.contains("push_managed_restart_task("));
        assert!(restart.contains("Ok(retained_tasks)"));
        assert!(!restart.contains("managed_scheduler("));
        assert!(!restart.contains("thread::Builder::new"));
        assert!(!restart.contains("FOREMAN_RESTART_SCAN_LIMIT_REACHED"));
        assert!(!restart.contains("list_active_task_refs(256)"));

        let assembly = source
            .split("fn assemble_full_chain_service_with_mode")
            .nth(1)
            .expect("full-chain assembly")
            .split("fn validate_controlled_task_timeout")
            .next()
            .expect("full-chain assembly body");
        let stage = assembly
            .find("stage_managed_restart_tasks(&core)")
            .expect("complete durable stage");
        let start = assembly
            .find("managed_scheduler(")
            .expect("scheduler starts after stage");
        assert!(assembly.contains("start_after_complete_stage("));
        assert!(stage < start);

        let refill = source
            .split("fn refill_managed_scheduler_from_durable")
            .nth(1)
            .expect("durable rescan")
            .split("fn managed_scheduler")
            .next()
            .expect("durable rescan body");
        assert!(refill.contains("RestartTaskKind::DraftPendingPromotion"));
        assert!(!refill.contains("promote_managed_task("));
        assert!(refill.contains("isolate_managed_restart_dependency"));
        assert!(refill.contains("managed_restart_status_should_skip"));
        let status_gate = source
            .split("fn managed_restart_status_should_skip")
            .nth(1)
            .expect("durable restart status gate")
            .split("fn managed_status_requires_exact_provider_reconciliation")
            .next()
            .expect("durable restart status gate boundary");
        assert!(status_gate.contains("managed_status_is_durably_closed"));
        assert!(status_gate.contains("managed_status_is_durably_deferred"));
        assert!(!refill.contains("LATTICE_MANAGED_SCHEDULER_CAPACITY_EXHAUSTED"));
    }

    #[test]
    fn outer_managed_status_accepts_only_exact_durable_project_drift_projection() {
        let authority = ProjectAuthorityReceipt::new(
            CONTRACT_VERSION,
            PROJECT_AUTHORITY_PRODUCER_ID,
            PROJECT_AUTHORITY_PRODUCER_VERSION,
            RuntimeKind::Live,
            ProjectId::new("status-project").expect("project"),
            ProjectSnapshotId::new("status-project:snapshot:1").expect("snapshot"),
            1,
            ProjectLifecycle::Active,
            ProjectClass::UserProject,
            GitRefIdentity::new("refs/heads/main", test_content_digest('1')).expect("primary ref"),
            test_content_digest('a'),
            test_content_digest('b'),
        )
        .expect("project authority");
        let submission = general_task_submission(
            "outer-status-project-drift",
            "status only",
            "Status Project",
            &authority,
        )
        .expect("general submission");
        let drift = json!({
            "schema_version": "lattice.task.status.v4",
            "task_ref": submission.task_ref().as_str(),
            "project_id": submission.identity().project_id().as_str(),
            "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
            "blocker": "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
            "failure_code": "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
            "next_action": "Refresh the registered project authority, then retry this task.",
        });
        assert_eq!(
            require_managed_project_status_projection(false, &submission, drift.clone())
                .expect("persisted drift projection"),
            drift
        );

        for stronger in [
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "BLOCKED",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED",
                "failure_code": "LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED",
                "next_action": "The bounded repair budget is exhausted; inspect retained evidence.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "BLOCKED",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL",
                "failure_code": "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL",
                "next_action": "Reconcile the retained exact provider effect.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "AWAITING_MERGE_APPROVAL",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": Value::Null,
                "failure_code": Value::Null,
                "verification_status": "PASSED",
                "next_action": "Approve the verified result for merge.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "VERIFYING",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "EXECUTION_AUTHORITY_NOT_CURRENT",
                "failure_code": "EXECUTION_AUTHORITY_NOT_CURRENT",
                "verification_status": "RUNNING",
                "next_action": "Wait for independent verification to finish.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "BLOCKED",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "LATTICE_MANAGED_VERIFICATION_FAILED",
                "failure_code": "LATTICE_MANAGED_VERIFICATION_FAILED",
                "verification_status": "FAILED",
                "next_action": "Inspect the retained verification evidence.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "status": "RUNNING",
                "task_state": "REVIEWING",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": Value::Null,
                "failure_code": Value::Null,
                "verification_status": "PASSED",
                "next_action": "Wait for independent semantic review to finish.",
            }),
        ] {
            assert_eq!(
                require_managed_project_status_projection(false, &submission, stronger.clone())
                    .expect("stronger durable managed evidence wins over Project drift"),
                stronger
            );
        }

        let cleared = json!({
            "schema_version": "lattice.task.status.v4",
            "task_ref": submission.task_ref().as_str(),
            "project_id": submission.identity().project_id().as_str(),
            "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
            "blocker": Value::Null,
            "failure_code": Value::Null,
            "next_action": "Wait for the foreman to reconcile the retained exact worker turn.",
        });
        assert!(
            require_managed_project_status_projection(true, &submission, cleared.clone()).is_ok()
        );
        assert_eq!(
            require_managed_project_status_projection(false, &submission, cleared)
                .expect_err("drift without its durable blocker must fail closed")
                .code(),
            "PROJECT_REGISTRY_CURRENTNESS_CONFLICT"
        );

        for arbitrary in [
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "BLOCKED",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "ARBITRARY_BLOCKER",
                "failure_code": "ARBITRARY_BLOCKER",
                "next_action": "Trust an unknown projection.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "AWAITING_MERGE_APPROVAL",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "ARBITRARY_BLOCKER",
                "failure_code": "ARBITRARY_BLOCKER",
                "next_action": "Trust an unknown projection.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "status": "RUNNING",
                "task_state": "REVIEWING",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "ARBITRARY_BLOCKER",
                "failure_code": "ARBITRARY_BLOCKER",
                "verification_status": "PASSED",
                "next_action": "Trust an unknown review projection.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "BLOCKED",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED",
                "failure_code": "LATTICE_MANAGED_VERIFICATION_FAILED",
                "next_action": "Trust mismatched durable evidence.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "status": "SUBMITTED",
                "task_state": "REVIEWING",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": Value::Null,
                "failure_code": Value::Null,
                "verification_status": "PASSED",
                "next_action": "Trust an invalid review phase.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "status": "RUNNING",
                "task_state": "REVIEWING",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": Value::Null,
                "failure_code": Value::Null,
                "verification_status": "NOT_STARTED",
                "next_action": "Trust review before verification.",
            }),
        ] {
            assert_eq!(
                require_managed_project_status_projection(false, &submission, arbitrary)
                    .expect_err("unknown blocker must not bypass Project drift")
                    .code(),
                "PROJECT_REGISTRY_CURRENTNESS_CONFLICT"
            );
        }

        for tampered in [
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_ref": test_content_digest('f').as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
                "failure_code": "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
                "next_action": "Refresh the registered project authority, then retry this task.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_ref": submission.task_ref().as_str(),
                "project_id": "foreign-project",
                "project_snapshot_id": submission.identity().project_snapshot_id().as_str(),
                "blocker": "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
                "failure_code": "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
                "next_action": "Refresh the registered project authority, then retry this task.",
            }),
            json!({
                "schema_version": "lattice.task.status.v4",
                "task_ref": submission.task_ref().as_str(),
                "project_id": submission.identity().project_id().as_str(),
                "project_snapshot_id": "status-project:foreign-snapshot",
                "blocker": "LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED",
                "failure_code": "LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED",
                "next_action": "The bounded repair budget is exhausted.",
            }),
        ] {
            assert_eq!(
                require_managed_project_status_projection(false, &submission, tampered)
                    .expect_err("cross-task and cross-project status must fail closed")
                    .code(),
                "LATTICE_MANAGED_STATUS_SUBSTITUTION_REJECTED"
            );
        }
    }

    #[test]
    fn project_drift_restart_candidate_records_a_durable_preparation_blocker() {
        let source = include_str!("composition.rs");
        let stage = source
            .split("fn stage_managed_restart_tasks")
            .nth(1)
            .expect("restart stage")
            .split("fn push_managed_restart_task")
            .next()
            .expect("restart stage body");
        let refill = source
            .split("fn refill_managed_scheduler_from_durable")
            .nth(1)
            .expect("restart refill")
            .split("fn managed_scheduler")
            .next()
            .expect("restart refill body");
        for (body, dispatch) in [
            (stage, "push_managed_restart_task("),
            (refill, "accept_durable_scheduler_task("),
        ] {
            assert!(body.contains("managed_restart_kind_requires_project_reconciliation"));
            assert!(body.contains("record_managed_restart_project_blocker"));
            let persist = body
                .find("PROJECT_REGISTRY_CURRENTNESS_CONFLICT")
                .expect("scan/resolve race persists a typed project blocker");
            let isolate = body
                .find("isolate_managed_restart_dependency")
                .expect("other known gates remain isolated");
            assert!(persist < isolate);
            let race_lane = &body[persist..];
            let recorded = race_lane
                .find("record_managed_restart_project_blocker")
                .expect("race blocker is retained");
            let skipped = race_lane[recorded..]
                .find("return Ok(())")
                .expect("race candidate stops before provider dispatch");
            let dispatch = race_lane
                .find(dispatch)
                .expect("normal current candidate may reach dispatch");
            assert!(skipped > 0 && recorded + skipped < dispatch);
        }
        for kind in [
            RestartTaskKind::DraftProjectReconciliationRequired,
            RestartTaskKind::ProjectReconciliationRequired,
        ] {
            assert!(managed_restart_kind_requires_project_reconciliation(kind));
        }
    }

    #[test]
    fn writer_drift_restart_is_durably_deferred_without_restarting_a_provider() {
        let status = serde_json::json!({
            "schema_version": "lattice.task.status.v4",
            "task_state": "EXECUTING",
            "blocker": "LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED",
            "failure_code": "LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED",
        });
        assert!(managed_status_is_durably_deferred(&status));
        assert!(managed_dependency_not_ready(
            "LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED"
        ));

        let source = include_str!("composition.rs");
        let stage = source
            .split("fn stage_managed_restart_tasks")
            .nth(1)
            .expect("restart stage")
            .split("fn push_managed_restart_task")
            .next()
            .expect("restart stage body");
        let refill = source
            .split("fn refill_managed_scheduler_from_durable")
            .nth(1)
            .expect("restart refill")
            .split("fn managed_scheduler")
            .next()
            .expect("restart refill body");
        for body in [stage, refill] {
            assert!(body.contains("RestartTaskKind::WriterReconciliationRequired"));
            assert!(body.contains("record_managed_restart_writer_blocker"));
        }
    }

    #[test]
    fn durable_restart_evidence_bypasses_writer_defer_but_not_a_closed_result() {
        for kind in [
            RestartTaskKind::AttemptClosedPendingRelease,
            RestartTaskKind::VerificationReconcileRequired,
            RestartTaskKind::TerminalPendingVerification,
        ] {
            assert!(managed_restart_kind_has_durable_evidence(kind));
        }
        for kind in [
            RestartTaskKind::DraftPendingPromotion,
            RestartTaskKind::DraftProjectReconciliationRequired,
            RestartTaskKind::ProjectReconciliationRequired,
            RestartTaskKind::PromotedNoAttempt,
            RestartTaskKind::CapacityWait,
            RestartTaskKind::AttemptReconcileRequired,
            RestartTaskKind::WriterReconciliationRequired,
        ] {
            assert!(!managed_restart_kind_has_durable_evidence(kind));
        }
        let writer_deferred = serde_json::json!({
            "schema_version": "lattice.task.status.v4",
            "task_state": "PREPARING",
            "blocker": "LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED",
            "failure_code": "LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED",
        });
        assert!(managed_restart_status_should_skip(false, &writer_deferred));
        assert!(
            !managed_restart_status_should_skip(true, &writer_deferred),
            "a closure/verification/terminal discovered ahead of Writer health must enter recovery"
        );

        let closed = serde_json::json!({
            "schema_version": "lattice.task.status.v4",
            "task_state": "AWAITING_MERGE_APPROVAL",
            "blocker": null,
            "failure_code": null,
        });
        assert!(
            managed_restart_status_should_skip(true, &closed),
            "a promoted result with no Writer cleanup must remain closed"
        );
    }

    #[test]
    fn blocked_retained_provider_effect_remains_restart_reconcilable() {
        for blocker in [
            "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL",
            "LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
            "LATTICE_MANAGED_THREAD_START_RPC_REJECTED",
            "LATTICE_MANAGED_REVIEW_RECONCILIATION_REQUIRED",
        ] {
            let status = serde_json::json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": "BLOCKED",
                "blocker": blocker,
                "failure_code": blocker,
            });
            assert!(managed_status_requires_exact_provider_reconciliation(
                &status
            ));
            assert!(!managed_status_is_durably_closed(&status));
        }
        for task_state in ["BLOCKED", "AWAITING_MERGE_APPROVAL"] {
            let writer_cleanup = serde_json::json!({
                "schema_version": "lattice.task.status.v4",
                "task_state": task_state,
                "blocker": "LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED",
                "failure_code": "LATTICE_MANAGED_WRITER_RECONCILIATION_REQUIRED",
            });
            assert!(
                !managed_status_is_durably_closed(&writer_cleanup),
                "durable task result does not suppress pending Writer cleanup"
            );
        }
        let closed = serde_json::json!({
            "schema_version": "lattice.task.status.v4",
            "task_state": "BLOCKED",
            "blocker": "LATTICE_MANAGED_VERIFICATION_FAILED",
        });
        assert!(!managed_status_requires_exact_provider_reconciliation(
            &closed
        ));
        assert!(managed_status_is_durably_closed(&closed));
    }

    #[test]
    fn managed_status_catalog_accepts_both_closed_model_probe_timeouts() {
        for code in [
            "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED",
            "LATTICE_MANAGED_REVIEW_MODEL_PROBE_TIMEOUT_NO_PROVIDER_EFFECT",
        ] {
            assert!(
                managed_status_has_known_closed_blocker(code),
                "missing durable closed blocker {code}"
            );
        }
    }

    #[test]
    fn scheduler_shutdown_timeout_retains_the_join_handle_until_cleanup_completes() {
        let (task_sender, _task_receiver) = mpsc::sync_channel(1);
        let cancellation = ManagedWorkerCancellation::default();
        let (completion_sender, completion_receiver) = mpsc::sync_channel(1);
        let (release_sender, release_receiver) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            let _completion = ManagedSchedulerCompletion(Some(completion_sender));
            release_receiver.recv().expect("release blocked worker");
            Ok(())
        });
        let mut owner = ManagedSchedulerOwner {
            sender: Some(task_sender),
            cancellation: cancellation.clone(),
            workers: vec![ManagedSchedulerWorker {
                completion: completion_receiver,
                handle: Some(handle),
            }],
            rescan_requested: Arc::new(AtomicBool::new(false)),
            armed: true,
        };

        let started = Instant::now();
        assert_eq!(
            owner
                .shutdown_with_deadline(Duration::from_millis(20))
                .expect_err("blocked worker must exceed total deadline")
                .kind(),
            LatticedErrorKind::ManagedTeardownRejected
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(cancellation.is_requested());
        assert!(owner.armed);
        assert_eq!(owner.workers.len(), 1);

        release_sender.send(()).expect("release blocked worker");
        owner
            .shutdown_with_deadline(Duration::from_secs(1))
            .expect("second teardown joins the retained handle");
        assert!(!owner.armed);
        assert!(owner.workers.is_empty());
    }

    #[test]
    fn scheduler_shutdown_gives_terminal_persistence_its_own_bounded_join_window() {
        let (task_sender, _task_receiver) = mpsc::sync_channel(1);
        let cancellation = ManagedWorkerCancellation::default();
        let mut bridge = cancellation.register_managed_bridge();
        let (completion_sender, completion_receiver) = mpsc::sync_channel(1);
        let (started_sender, started_receiver) = mpsc::sync_channel(0);
        let handle = thread::spawn(move || {
            let _completion = ManagedSchedulerCompletion(Some(completion_sender));
            started_sender.send(()).expect("announce worker start");
            thread::sleep(Duration::from_millis(60));
            bridge.record_reaped();
            thread::sleep(Duration::from_millis(60));
            Ok(())
        });
        let mut owner = ManagedSchedulerOwner {
            sender: Some(task_sender),
            cancellation,
            workers: vec![ManagedSchedulerWorker {
                completion: completion_receiver,
                handle: Some(handle),
            }],
            rescan_requested: Arc::new(AtomicBool::new(false)),
            armed: true,
        };
        started_receiver.recv().expect("observe worker start");

        owner
            .shutdown_with_deadline(Duration::from_millis(80))
            .expect("exact bridge drain and durable terminal join have separate deadlines");
        assert!(!owner.armed);
        assert!(owner.workers.is_empty());
    }

    #[test]
    fn scheduler_drop_fail_stops_when_a_worker_terminal_is_rejected() {
        const CHILD_ENV: &str = "LATTICE_TEST_SCHEDULER_DROP_WORKER_FAILURE";
        if env::var_os(CHILD_ENV).is_some() {
            let (task_sender, _task_receiver) = mpsc::sync_channel(1);
            let (completion_sender, completion_receiver) = mpsc::sync_channel(1);
            let handle = thread::spawn(move || {
                let _completion = ManagedSchedulerCompletion(Some(completion_sender));
                Err("TEST_WORKER_TERMINAL_REJECTED")
            });
            let owner = ManagedSchedulerOwner {
                sender: Some(task_sender),
                cancellation: ManagedWorkerCancellation::default(),
                workers: vec![ManagedSchedulerWorker {
                    completion: completion_receiver,
                    handle: Some(handle),
                }],
                rescan_requested: Arc::new(AtomicBool::new(false)),
                armed: true,
            };
            drop(owner);
            panic!("worker failure was detached instead of fail-stopping");
        }

        let status = process::Command::new(env::current_exe().expect("current test executable"))
            .args([
                "--exact",
                "composition::tests::scheduler_drop_fail_stops_when_a_worker_terminal_is_rejected",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .status()
            .expect("run isolated scheduler Drop child");
        assert!(
            !status.success(),
            "Drop swallowed a rejected worker terminal"
        );
    }

    #[test]
    fn restart_backlog_over_limit_fails_before_any_dispatch() {
        let mut retained = Vec::new();
        let mut dispatch_count = 0;
        let result = (|| -> Result<(), ToolExecutionError> {
            for task in 0..3 {
                push_managed_restart_task(&mut retained, task, 2)?;
            }
            for _task in retained {
                dispatch_count += 1;
            }
            Ok(())
        })();

        assert_eq!(
            result.expect_err("bounded restart backlog").code(),
            "FOREMAN_RESTART_BACKLOG_LIMIT_EXCEEDED"
        );
        assert_eq!(dispatch_count, 0);
    }

    #[test]
    fn scheduler_start_is_barriered_on_the_complete_restart_stage() {
        let effects = Arc::new(AtomicUsize::new(0));
        let (early_sender, early_receiver) = mpsc::sync_channel(1);
        let (release_sender, release_receiver) = mpsc::sync_channel(1);
        let thread_effects = Arc::clone(&effects);
        let owner = thread::spawn(move || {
            start_after_complete_stage(
                || -> Result<Vec<&'static str>, &'static str> {
                    let staged = vec!["valid-early"];
                    early_sender.send(()).expect("announce early valid row");
                    release_receiver.recv().expect("release late corrupt row");
                    let _ = staged;
                    Err("FOREMAN_REPLAY_CORRUPT")
                },
                |tasks| {
                    thread_effects.fetch_add(tasks.len(), Ordering::SeqCst);
                    Ok(())
                },
            )
        });

        early_receiver.recv().expect("early row staged");
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        release_sender.send(()).expect("release full scan");
        assert_eq!(
            owner
                .join()
                .expect("barrier test thread")
                .expect_err("late corrupt row rejects assembly"),
            "FOREMAN_REPLAY_CORRUPT"
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        let mut starts = 0;
        let accepted = start_after_complete_stage(
            || Ok::<_, &'static str>(vec!["first", "second"]),
            |tasks| {
                starts += 1;
                Ok(tasks.len())
            },
        )
        .expect("complete valid stage starts once");
        assert_eq!(accepted, 2);
        assert_eq!(starts, 1);
    }

    #[test]
    fn bounded_scheduler_queue_accepts_then_refills_the_same_durable_task_once() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.try_send("queue-occupant").expect("fill queue");
        let scheduled = Mutex::new(BTreeSet::new());
        let rescan_requested = AtomicBool::new(false);
        assert_eq!(
            accept_durable_scheduler_task(
                &sender,
                &scheduled,
                &rescan_requested,
                "task-ref-retry",
                "task-ref-retry",
            )
            .expect("durable submit remains accepted while capacity is deferred"),
            ManagedDurableEnqueueOutcome::DeferredCapacity
        );
        assert!(rescan_requested.load(AtomicOrdering::Acquire));
        assert!(
            !scheduled
                .lock()
                .expect("scheduled set")
                .contains("task-ref-retry")
        );
        assert_eq!(receiver.try_recv().expect("drain queue"), "queue-occupant");

        assert!(rescan_requested.swap(false, AtomicOrdering::AcqRel));
        assert_eq!(
            try_enqueue_durable(
                &sender,
                &scheduled,
                &rescan_requested,
                "task-ref-retry",
                "task-ref-retry",
            )
            .expect("durable refill"),
            ManagedDurableEnqueueOutcome::Enqueued
        );
        assert_eq!(
            try_enqueue_durable(
                &sender,
                &scheduled,
                &rescan_requested,
                "task-ref-retry",
                "duplicate",
            )
            .expect("duplicate suppression"),
            ManagedDurableEnqueueOutcome::AlreadyScheduled
        );
        assert_eq!(
            receiver.try_recv().expect("refilled durable task"),
            "task-ref-retry"
        );
        assert!(receiver.try_recv().is_err());

        let source = include_str!("composition.rs");
        let schedule = source
            .split("fn schedule_managed_general_task")
            .nth(1)
            .expect("managed enqueue")
            .split("fn load_managed_scheduled_task_from_durable")
            .next()
            .expect("managed enqueue body");
        assert!(schedule.contains("ManagedDurableEnqueueOutcome::DeferredCapacity"));
        assert!(schedule.contains("request_rescan"));
        let deferred = schedule
            .split("ManagedDurableEnqueueOutcome::DeferredCapacity =>")
            .nth(1)
            .expect("durable accepted deferral")
            .split("}\n")
            .next()
            .expect("deferred branch");
        assert!(deferred.contains("request_rescan"));
        assert!(!deferred.contains("return Err"));
    }

    #[test]
    fn durable_intake_queue_full_is_accepted_without_synchronous_promotion() {
        let source = include_str!("composition.rs");
        let schedule = source
            .split("fn schedule_managed_general_task")
            .nth(1)
            .expect("managed schedule")
            .split("fn load_managed_scheduled_task_from_durable")
            .next()
            .expect("managed schedule body");
        assert!(schedule.contains("accept_durable_scheduler_task("));
        let accepted = schedule
            .find("accept_durable_scheduler_task(")
            .expect("accepted/deferred admission");
        assert!(accepted > 0);
        assert!(!schedule.contains("promote_managed_task("));
        assert!(!schedule.contains("LATTICE_MANAGED_SCHEDULER_CAPACITY_EXHAUSTED"));
    }

    fn managed_foreman_runtime_status(
        latest_generation: u64,
        active_count: usize,
        blocked_count: usize,
        completed_count: usize,
        next_action: &'static str,
    ) -> ForemanRuntimeStatus {
        ForemanRuntimeStatus::new(
            test_content_digest('1'),
            test_content_digest('2'),
            latest_generation,
            active_count,
            blocked_count,
            completed_count,
            next_action,
            None,
        )
    }

    #[test]
    fn managed_foreman_identity_accepts_only_one_active_continuing_generation() {
        let identity = formal_managed_foreman_identity_from_status(
            &managed_foreman_runtime_status(7, 1, 0, 0, "CONTINUE"),
        )
        .expect("one replay-verified active foreman may dispatch");

        assert_eq!(identity.generation(), 7);
        assert_eq!(identity.checkpoint_digest(), &test_content_digest('2'));
    }

    #[test]
    fn managed_foreman_identity_rejects_empty_blocked_and_completed_replay() {
        for status in [
            managed_foreman_runtime_status(0, 0, 0, 0, "NO_DURABLE_SNAPSHOT"),
            managed_foreman_runtime_status(7, 0, 1, 0, "RESOLVE_BLOCKERS"),
            managed_foreman_runtime_status(7, 0, 0, 1, "ALL_COMPLETED"),
        ] {
            let failure = formal_managed_foreman_identity_from_status(&status)
                .expect_err("non-active foreman replay must stop before claim");
            assert_eq!(failure.code(), MANAGED_FOREMAN_NOT_ACTIVE);
        }
    }

    #[test]
    fn managed_foreman_identity_rejects_multiple_active_or_wrong_next_action() {
        for status in [
            managed_foreman_runtime_status(7, 2, 0, 0, "CONTINUE"),
            managed_foreman_runtime_status(7, 1, 0, 0, "RESOLVE_BLOCKERS"),
        ] {
            let failure = formal_managed_foreman_identity_from_status(&status)
                .expect_err("ambiguous active foreman replay must stop before claim");
            assert_eq!(failure.code(), MANAGED_FOREMAN_NOT_ACTIVE);
        }
    }

    #[test]
    fn managed_foreman_identity_loaders_gate_replay_before_formal_identity() {
        let source = include_str!("composition.rs");
        let initial = source
            .split("fn formal_managed_foreman_identity<H")
            .nth(1)
            .expect("initial managed foreman identity loader")
            .split("fn formal_managed_foreman_identity_from_status")
            .next()
            .expect("initial loader body");
        let reload = source
            .split("fn reload_managed_foreman_identity")
            .nth(1)
            .expect("restart managed foreman identity loader")
            .split("fn managed_general_task_public_status")
            .next()
            .expect("restart loader body");

        for loader in [initial, reload] {
            let replay = loader
                .find("load_runtime_status()")
                .expect("replay-verified runtime status");
            let active_gate = loader
                .find("formal_managed_foreman_identity_from_status(&foreman)")
                .expect("sole-active preclaim gate");
            assert!(replay < active_gate);
            assert!(!loader.contains("FormalForemanIdentity::new("));
        }
    }

    #[test]
    fn restart_keyset_pages_cover_exact_256_257_and_513_without_duplicates_or_starvation() {
        for total in [256_usize, 257, 513] {
            let rows = (0..total)
                .map(|index| ((index / 100).min(5) as u8, index as u16))
                .collect::<Vec<_>>();
            let mut page_sizes = Vec::new();
            let mut scheduled = Vec::new();
            walk_restart_keyset_pages(
                256,
                |cursor, page_limit| {
                    let start = cursor
                        .map(|cursor| rows.partition_point(|row| row <= cursor))
                        .unwrap_or(0);
                    let end = (start + usize::from(page_limit)).min(rows.len());
                    let page = rows[start..end].to_vec();
                    page_sizes.push(page.len());
                    Ok(page)
                },
                |row| *row,
                |row| {
                    scheduled.push(row);
                    Ok(())
                },
            )
            .expect("stable keyset walk");

            assert_eq!(scheduled, rows, "all restart candidates must be scheduled");
            assert_eq!(scheduled.iter().collect::<BTreeSet<_>>().len(), total);
            assert!(page_sizes.iter().all(|size| *size <= 256));
            assert_eq!(
                page_sizes,
                match total {
                    256 => vec![256, 0],
                    257 => vec![256, 1],
                    513 => vec![256, 256, 1],
                    _ => unreachable!(),
                }
            );
        }
    }

    #[test]
    fn restart_keyset_pages_reject_cross_page_duplicate_and_nonprogress() {
        let mut pages = VecDeque::from([vec![(0_u8, 1_u16), (0, 2)], vec![(0, 2)]]);
        let failure = walk_restart_keyset_pages(
            2,
            |_cursor, _page_limit| Ok(pages.pop_front().unwrap_or_default()),
            |row| *row,
            |_row| Ok(()),
        )
        .expect_err("duplicate cursor must fail closed");
        assert_eq!(failure.code(), "FOREMAN_RESTART_SCAN_NOT_STRICT");
    }

    #[test]
    fn restart_scan_persists_project_drift_and_isolates_other_known_dependency_gates() {
        let mut admitted = Vec::new();
        for candidate in [
            Err(ToolExecutionError::new(
                "LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED",
            )),
            Ok("ready-task"),
        ] {
            if let Some(task) =
                isolate_managed_restart_dependency(candidate).expect("known gate is isolated")
            {
                admitted.push(task);
            }
        }
        assert_eq!(admitted, vec!["ready-task"]);
        for code in [
            "LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED",
            "LATTICE_MANAGED_WORKTREE_NOT_CLEAN",
        ] {
            assert_eq!(
                isolate_managed_restart_dependency::<()>(Err(ToolExecutionError::new(code)))
                    .expect("known dependency gate"),
                None
            );
        }
        assert_eq!(
            isolate_managed_restart_dependency::<()>(Err(ToolExecutionError::new(
                "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
            )))
            .expect_err("project drift must be persisted before it can be deferred")
            .code(),
            "PROJECT_REGISTRY_CURRENTNESS_CONFLICT"
        );
        assert_eq!(
            isolate_managed_restart_dependency::<()>(Err(ToolExecutionError::new(
                "LATTICE_MANAGED_PROMOTION_REPLAY_REJECTED",
            )))
            .expect_err("unknown lineage error must fail closed")
            .code(),
            "LATTICE_MANAGED_PROMOTION_REPLAY_REJECTED"
        );
    }

    #[test]
    fn managed_scheduler_retries_one_transient_preclaim_failure_without_a_new_queue_entry() {
        let scheduled = Mutex::new(BTreeSet::new());
        assert!(retain_managed_schedule(&scheduled, "task-ref").expect("initial queue entry"));
        let mut identity_reads = 0_u8;
        let mut run_calls = 0_u8;
        let mut status_reads = 0_u8;
        let mut waits = Vec::new();
        let cancellation = ManagedWorkerCancellation::default();
        let exit = supervise_managed_task(
            &cancellation,
            || {
                identity_reads += 1;
                Ok::<_, &'static str>(())
            },
            |_| {
                run_calls += 1;
                if run_calls == 1 {
                    Err("LATTICE_MANAGED_GIT_OBSERVATION_REJECTED")
                } else {
                    Ok(())
                }
            },
            |_| {
                status_reads += 1;
                Ok(Some(managed_scheduler_test_status("PREPARING", None)))
            },
            |delay| {
                assert!(
                    !retain_managed_schedule(&scheduled, "task-ref")
                        .expect("recovery-time duplicate queue check")
                );
                waits.push(delay);
            },
        );

        assert_eq!(exit, ManagedSupervisorExit::RunCompleted);
        assert_eq!(identity_reads, 2);
        assert_eq!(run_calls, 2);
        assert_eq!(status_reads, 1);
        assert_eq!(waits, vec![Duration::from_secs(1)]);
        assert_eq!(scheduled.lock().expect("retained task key").len(), 1);
        release_managed_schedule(&scheduled, "task-ref");
        assert!(scheduled.lock().expect("released task key").is_empty());
    }

    #[test]
    fn current_execution_authority_does_not_defer_an_awaiting_state_after_transient_setup_failure()
    {
        let cancellation = ManagedWorkerCancellation::default();
        let mut run_calls = 0_u8;
        let mut status_reads = 0_u8;
        let mut waits = Vec::new();
        let exit = supervise_managed_task(
            &cancellation,
            || Ok::<_, &'static str>(()),
            |_| {
                run_calls += 1;
                if run_calls == 1 {
                    Err("LATTICE_MANAGED_WORKTREE_BRIDGE_REJECTED")
                } else {
                    Ok(())
                }
            },
            |_| {
                status_reads += 1;
                Ok(Some(managed_scheduler_test_status(
                    "AWAITING_EXECUTION_APPROVAL",
                    None,
                )))
            },
            |delay| waits.push(delay),
        );

        assert_eq!(exit, ManagedSupervisorExit::RunCompleted);
        assert_eq!(run_calls, 2);
        assert_eq!(status_reads, 1);
        assert_eq!(waits, vec![Duration::from_secs(1)]);
    }

    #[test]
    fn awaiting_execution_approval_releases_each_worker_without_busy_retry() {
        let cancellation = ManagedWorkerCancellation::default();
        let mut total_runs = 0_u8;
        let mut total_status_reads = 0_u8;
        for _ in 0..MANAGED_SUPERVISOR_WORKERS {
            let exit = supervise_managed_task(
                &cancellation,
                || Ok::<_, &'static str>(()),
                |_| {
                    total_runs += 1;
                    Err("LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED")
                },
                |_| {
                    total_status_reads += 1;
                    Ok(Some(managed_scheduler_test_status(
                        "AWAITING_EXECUTION_APPROVAL",
                        Some("LATTICE_MANAGED_EXECUTION_APPROVAL_REQUIRED"),
                    )))
                },
                |_| panic!("durable approval gate must release capacity without backoff"),
            );
            assert_eq!(exit, ManagedSupervisorExit::DurablyDeferred);
        }
        assert_eq!(
            total_runs,
            u8::try_from(MANAGED_SUPERVISOR_WORKERS).unwrap()
        );
        assert_eq!(
            total_status_reads,
            u8::try_from(MANAGED_SUPERVISOR_WORKERS).unwrap()
        );

        let mut authorized_runs = 0_u8;
        assert_eq!(
            supervise_managed_task(
                &cancellation,
                || Ok::<_, &'static str>(()),
                |_| {
                    authorized_runs += 1;
                    Ok(())
                },
                |_| panic!("authorized task completes without a status retry"),
                |_| panic!("authorized task must not be starved by approval waiters"),
            ),
            ManagedSupervisorExit::RunCompleted
        );
        assert_eq!(authorized_runs, 1);
    }

    #[test]
    fn durably_deferred_task_is_automatically_redispatched_once_after_dependency_ready() {
        let cancellation = ManagedWorkerCancellation::default();
        let scheduled = Mutex::new(BTreeSet::new());
        assert!(retain_managed_schedule(&scheduled, "task-deferred").unwrap());
        let exit = supervise_managed_task(
            &cancellation,
            || Ok::<_, &'static str>(()),
            |_| Err("LATTICE_MANAGED_WORKTREE_NOT_CLEAN"),
            |_| {
                Ok(Some(managed_scheduler_test_status(
                    "DRAFT",
                    Some("LATTICE_MANAGED_WORKTREE_NOT_CLEAN"),
                )))
            },
            |_| panic!("durably visible dependency gate must not hot-loop"),
        );
        assert_eq!(exit, ManagedSupervisorExit::DurablyDeferred);
        release_managed_schedule(&scheduled, "task-deferred");

        let requested = AtomicBool::new(false);
        let now = Instant::now();
        let due = now.checked_add(MANAGED_DURABLE_RESCAN_INTERVAL).unwrap();
        let next = Mutex::new(due);
        assert!(!claim_managed_durable_rescan(&requested, &next, now).unwrap());
        assert!(claim_managed_durable_rescan(&requested, &next, due).unwrap());
        assert!(!claim_managed_durable_rescan(&requested, &next, due).unwrap());

        let (sender, receiver) = mpsc::sync_channel(1);
        assert_eq!(
            try_enqueue_durable(
                &sender,
                &scheduled,
                &requested,
                "task-deferred",
                "task-deferred",
            )
            .unwrap(),
            ManagedDurableEnqueueOutcome::Enqueued
        );
        assert_eq!(receiver.try_recv().unwrap(), "task-deferred");
        assert!(receiver.try_recv().is_err());

        let mut ready_runs = 0_u8;
        assert_eq!(
            supervise_managed_task(
                &cancellation,
                || Ok::<_, &'static str>(()),
                |_| {
                    ready_runs += 1;
                    Ok(())
                },
                |_| panic!("ready dependency must dispatch without recovery read"),
                |_| panic!("ready dependency must not back off"),
            ),
            ManagedSupervisorExit::RunCompleted
        );
        assert_eq!(ready_runs, 1);
        release_managed_schedule(&scheduled, "task-deferred");
    }

    #[test]
    fn managed_scheduler_stops_on_fresh_durable_terminal_without_spinning() {
        let mut run_calls = 0_u8;
        let mut status_reads = 0_u8;
        let cancellation = ManagedWorkerCancellation::default();
        let exit = supervise_managed_task(
            &cancellation,
            || Ok::<_, &'static str>(()),
            |_| {
                run_calls += 1;
                Err("LATTICE_MANAGED_WRITER_RELEASE_REJECTED")
            },
            |_| {
                status_reads += 1;
                Ok(Some(managed_scheduler_test_status(
                    "AWAITING_MERGE_APPROVAL",
                    None,
                )))
            },
            |_| panic!("durable terminal must not back off or spin"),
        );

        assert_eq!(exit, ManagedSupervisorExit::DurablyClosed);
        assert_eq!(run_calls, 1);
        assert_eq!(status_reads, 1);
    }

    #[test]
    fn managed_scheduler_keeps_reconciling_an_active_state_even_with_a_closed_reason_code() {
        let mut run_calls = 0_u8;
        let mut waits = Vec::new();
        let cancellation = ManagedWorkerCancellation::default();
        let exit = supervise_managed_task(
            &cancellation,
            || Ok::<_, &'static str>(()),
            |_| {
                run_calls += 1;
                if run_calls == 1 {
                    Err("LATTICE_MANAGED_VERIFICATION_FAILED")
                } else {
                    Ok(())
                }
            },
            |_| {
                Ok(Some(managed_scheduler_test_status(
                    "VERIFYING",
                    Some("LATTICE_MANAGED_VERIFICATION_FAILED"),
                )))
            },
            |delay| waits.push(delay),
        );

        assert_eq!(exit, ManagedSupervisorExit::RunCompleted);
        assert_eq!(run_calls, 2);
        assert_eq!(waits, vec![Duration::from_secs(1)]);
    }

    #[test]
    fn managed_scheduler_does_not_release_for_an_unknown_uppercase_blocker() {
        let scheduled = Mutex::new(BTreeSet::new());
        assert!(retain_managed_schedule(&scheduled, "task-ref").expect("initial queue entry"));
        let mut run_calls = 0_u8;
        let mut status_reads = 0_u8;
        let mut waits = Vec::new();
        let cancellation = ManagedWorkerCancellation::default();
        let exit = supervise_managed_task(
            &cancellation,
            || Ok::<_, &'static str>(()),
            |_| {
                run_calls += 1;
                if run_calls == 1 {
                    Err("LATTICE_MANAGED_DATABASE_CONNECT_REJECTED")
                } else {
                    Ok(())
                }
            },
            |_| {
                status_reads += 1;
                Ok(Some(managed_scheduler_test_status(
                    "VERIFYING",
                    Some("UNKNOWN_UPPERCASE_BLOCKER"),
                )))
            },
            |delay| {
                assert!(
                    !retain_managed_schedule(&scheduled, "task-ref")
                        .expect("recovery-time duplicate queue check")
                );
                waits.push(delay);
            },
        );

        assert_eq!(exit, ManagedSupervisorExit::RunCompleted);
        assert_eq!(run_calls, 2);
        assert_eq!(status_reads, 1);
        assert_eq!(waits, vec![Duration::from_secs(1)]);
        assert_eq!(scheduled.lock().expect("retained task key").len(), 1);
        release_managed_schedule(&scheduled, "task-ref");
        assert!(scheduled.lock().expect("released task key").is_empty());
    }

    #[test]
    fn managed_scheduler_retains_work_across_status_and_identity_read_failures() {
        let mut identity_reads = 0_u8;
        let mut run_calls = 0_u8;
        let mut status_reads = 0_u8;
        let mut waits = Vec::new();
        let cancellation = ManagedWorkerCancellation::default();
        let exit = supervise_managed_task(
            &cancellation,
            || {
                identity_reads += 1;
                if identity_reads == 1 {
                    Err("FOREMAN_REPLAY_UNAVAILABLE")
                } else {
                    Ok(())
                }
            },
            |_| {
                run_calls += 1;
                if run_calls <= 2 {
                    Err("LATTICE_MANAGED_DATABASE_CONNECT_REJECTED")
                } else {
                    Ok(())
                }
            },
            |_| {
                status_reads += 1;
                if status_reads == 1 {
                    Err("LATTICE_MANAGED_STATUS_REPLAY_UNAVAILABLE")
                } else {
                    Ok(Some(managed_scheduler_test_status("PREPARING", None)))
                }
            },
            |delay| waits.push(delay),
        );

        assert_eq!(exit, ManagedSupervisorExit::RunCompleted);
        assert_eq!(identity_reads, 4);
        assert_eq!(run_calls, 3);
        assert_eq!(status_reads, 2);
        assert_eq!(
            waits,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ]
        );
    }

    #[test]
    fn managed_scheduler_keeps_one_task_key_and_caps_recovery_backoff() {
        let scheduled = Mutex::new(BTreeSet::new());
        assert!(retain_managed_schedule(&scheduled, "task-ref").expect("first queue entry"));
        assert!(!retain_managed_schedule(&scheduled, "task-ref").expect("duplicate queue entry"));
        assert_eq!(scheduled.lock().expect("scheduled set").len(), 1);

        let mut backoff = ManagedRecoveryBackoff::default();
        assert_eq!(backoff.next_delay(), Duration::from_secs(1));
        assert_eq!(backoff.next_delay(), Duration::from_secs(2));
        assert_eq!(backoff.next_delay(), Duration::from_secs(4));
        assert_eq!(backoff.next_delay(), Duration::from_secs(8));
        assert_eq!(backoff.next_delay(), Duration::from_secs(16));
        assert_eq!(backoff.next_delay(), Duration::from_secs(16));
    }

    #[test]
    fn managed_scheduler_surfaces_graceful_shutdown_receipt_and_failure_without_status_retry() {
        let cancellation = ManagedWorkerCancellation::default();
        let exit = supervise_managed_task(
            &cancellation,
            || Ok::<_, &'static str>(()),
            |_| {
                cancellation.request();
                Err(MANAGED_GRACEFUL_SHUTDOWN_COMPLETE)
            },
            |_| panic!("exact shutdown receipt must not enter status recovery"),
            |_| panic!("exact shutdown receipt must not back off"),
        );
        assert_eq!(exit, ManagedSupervisorExit::ShutdownComplete);

        let cancellation = ManagedWorkerCancellation::default();
        let exit = supervise_managed_task(
            &cancellation,
            || Ok::<_, &'static str>(()),
            |_| {
                cancellation.request();
                Err("LATTICE_MANAGED_INTERRUPT_AMBIGUOUS")
            },
            |_| panic!("failed graceful teardown must not be hidden by status"),
            |_| panic!("failed graceful teardown must not back off"),
        );
        assert_eq!(
            exit,
            ManagedSupervisorExit::ShutdownFailed("LATTICE_MANAGED_INTERRUPT_AMBIGUOUS")
        );
    }

    fn managed_scheduler_test_status(state: &str, blocker: Option<&str>) -> Value {
        json!({
            "schema_version": "lattice.task.status.v4",
            "task_state": state,
            "blocker": blocker,
        })
    }

    #[test]
    fn create_only_general_status_redacts_arbitrary_sensitive_objective_on_submit_and_replay() {
        let authority = ProjectAuthorityReceipt::new(
            CONTRACT_VERSION,
            PROJECT_AUTHORITY_PRODUCER_ID,
            PROJECT_AUTHORITY_PRODUCER_VERSION,
            RuntimeKind::Live,
            ProjectId::new("private-project").expect("project"),
            ProjectSnapshotId::new("private-project:snapshot:1").expect("snapshot"),
            1,
            ProjectLifecycle::Active,
            ProjectClass::UserProject,
            GitRefIdentity::new("refs/heads/main", test_content_digest('1')).expect("primary ref"),
            test_content_digest('a'),
            test_content_digest('b'),
        )
        .expect("project authority");
        let objectives = [
            "Internal acquisition codename Quiet Orchard",
            "Private staffing plan for candidate Juniper",
        ];

        for (index, objective) in objectives.into_iter().enumerate() {
            let submission = general_task_submission(
                &format!("redacted-status-{index}"),
                objective,
                "Confidential Planning",
                &authority,
            )
            .expect("general submission");
            let evidence = TaskIntakeLifecycleEvidence::new(
                general_task_binding(&submission).expect("binding"),
                test_content_digest(if index == 0 { 'c' } else { 'd' }),
            )
            .expect("intake evidence");
            let submitted =
                general_task_public_status(&evidence, &submission).expect("submitted projection");
            let replayed = general_task_public_status(&evidence, &submission)
                .expect("fresh replay projection");

            assert_eq!(submitted, replayed);
            assert_eq!(submitted["schema_version"], "lattice.task.status.v5");
            assert_eq!(
                submitted["objective_summary"],
                TASK_PUBLIC_OBJECTIVE_SUMMARY
            );
            assert_eq!(
                submitted["objective_digest"],
                task_public_objective_digest(objective)
                    .expect("objective digest")
                    .as_str()
            );
            assert!(submitted.get("objective").is_none());
            assert!(!submitted.to_string().contains(objective));
        }

        let submission = general_task_submission(
            "external-adoption-status",
            "Confidential verified closure",
            "Confidential Planning",
            &authority,
        )
        .expect("general submission");
        let evidence = TaskIntakeLifecycleEvidence::externally_adopted(
            general_task_binding(&submission).expect("binding"),
            test_content_digest('e'),
            test_content_digest('f'),
        )
        .expect("external adoption evidence");
        let adopted =
            general_task_public_status(&evidence, &submission).expect("adopted projection");
        assert_eq!(adopted["schema_version"], "lattice.task.status.v6");
        assert_eq!(adopted["status"], "COMPLETED");
        assert_eq!(adopted["task_state"], "COMPLETED");
        assert_eq!(adopted["result_digest"], test_content_digest('f').as_str());

        let source = include_str!("composition.rs");
        let producer = source
            .split("fn general_task_public_status")
            .nth(1)
            .expect("create-only status producer")
            .split("fn formal_managed_foreman_identity")
            .next()
            .expect("create-only producer body");
        assert!(!producer.contains("lattice.task.status.v3"));
        assert!(!producer.contains("submission.objective(),"));
    }

    #[test]
    fn same_request_replays_the_winner_across_project_snapshot_race_only_for_same_project() {
        let authority = |project_id: &str, snapshot_id: &str, fill: char| {
            ProjectAuthorityReceipt::new(
                CONTRACT_VERSION,
                PROJECT_AUTHORITY_PRODUCER_ID,
                PROJECT_AUTHORITY_PRODUCER_VERSION,
                RuntimeKind::Live,
                ProjectId::new(project_id).expect("project"),
                ProjectSnapshotId::new(snapshot_id).expect("snapshot"),
                1,
                ProjectLifecycle::Active,
                ProjectClass::UserProject,
                GitRefIdentity::new("refs/heads/main", test_content_digest('1'))
                    .expect("primary ref"),
                test_content_digest(fill),
                test_content_digest(match fill {
                    'a' => 'd',
                    'b' => 'e',
                    'c' => 'f',
                    _ => '9',
                }),
            )
            .expect("project authority")
        };
        let first_authority = authority("project-a", "snapshot-a-1", 'a');
        let later_authority = authority("project-a", "snapshot-a-2", 'b');
        let other_authority = authority("project-b", "snapshot-b-1", 'c');
        let winner = general_task_submission(
            "same-request-race",
            "完成角色系統",
            "AI 劇本",
            &first_authority,
        )
        .expect("winner envelope");
        let raced = general_task_submission(
            "same-request-race",
            "完成角色系統",
            "AI 劇本",
            &later_authority,
        )
        .expect("raced envelope");
        assert_ne!(winner.task_ref(), raced.task_ref());

        assert!(general_submission_matches_effective_request(
            &winner,
            Some("完成角色系統"),
            Some("project-a"),
            None,
            later_authority.project_id(),
        ));
        assert!(!general_submission_matches_effective_request(
            &winner,
            Some("完成角色系統"),
            None,
            None,
            other_authority.project_id(),
        ));
        assert!(!general_submission_matches_effective_request(
            &winner,
            Some("different objective"),
            Some("project-a"),
            None,
            later_authority.project_id(),
        ));

        let source = include_str!("composition.rs");
        let task_submit = source
            .split("    fn task_submit(")
            .nth(1)
            .expect("Task Submit composition")
            .split("    fn task_status(")
            .next()
            .expect("Task Submit body");
        let admissions = task_submit
            .match_indices("admit_general_submission")
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>();
        assert_eq!(admissions.len(), 2, "initial and bounded retry admissions");
        let winner_reload = task_submit
            .find("replay_general_winner_after_admission_failure")
            .expect("post-conflict winner reload");
        let project_resolutions = task_submit
            .match_indices("resolve_registered_project_for_general_submit")
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>();
        assert_eq!(
            project_resolutions.len(),
            2,
            "initial and one bounded currentness re-resolution"
        );
        assert!(
            project_resolutions[0] < admissions[0]
                && admissions[0] < winner_reload
                && winner_reload < project_resolutions[1]
                && project_resolutions[1] < admissions[1]
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
    fn runtime_graph_request_is_bound_to_the_configured_git_commit() {
        let configuration = test_content_digest('a');
        let first = runtime_graph_request(
            "core-61152",
            "1111111111111111111111111111111111111111",
            configuration.clone(),
        )
        .expect("first configured source request");
        let repeated = runtime_graph_request(
            "core-61152",
            "1111111111111111111111111111111111111111",
            configuration,
        )
        .expect("same configured source request");
        let changed = runtime_graph_request(
            "core-61152",
            "2222222222222222222222222222222222222222",
            test_content_digest('a'),
        )
        .expect("changed configured source request");

        assert_eq!(first, repeated);
        assert_ne!(first.commit_id(), changed.commit_id());
        assert_ne!(
            first.invocation().subject_digest(),
            changed.invocation().subject_digest()
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
            None,
            None,
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
        assert_ne!(first.binding().project_id(), second.binding().project_id());
        assert_ne!(
            first.binding().project_snapshot_id(),
            second.binding().project_snapshot_id()
        );
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
    fn public_failure_projection_rejects_payload_like_receipt_fields() {
        let valid = json!({
            "failure_stage": "CODEX",
            "failure_code": "LATTICE_DELIVERY_FAILED"
        });
        assert_eq!(
            delivery_failure_projection(&valid),
            Some(("CODEX".to_owned(), "LATTICE_DELIVERY_FAILED".to_owned()))
        );
        let invalid = json!({
            "failure_stage": "CODEX",
            "failure_code": "C:\\\\runtime\\\\stderr"
        });
        assert_eq!(delivery_failure_projection(&invalid), None);
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
        let status = task_public_status(&evidence, &test_content_digest('9'), None, None);

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
    fn hermes_activation_status_requires_only_real_configuration() {
        assert_eq!(
            hermes_activation_status(HermesProductionPreflight::MissingConfiguration(vec![
                "LATTICE_HERMES_CODEX_HOME",
            ])),
            "CONFIGURATION_REQUIRED"
        );
        assert_eq!(
            hermes_activation_status(HermesProductionPreflight::MissingConfiguration(vec![
                "LATTICE_HERMES_CODEX_HOME",
            ])),
            "CONFIGURATION_REQUIRED"
        );
        assert_eq!(
            hermes_activation_status(HermesProductionPreflight::ConfigurationPresentUnverified),
            "PREPARED"
        );
    }

    #[cfg(windows)]
    #[test]
    fn hermes_loopback_session_tokens_are_fresh() {
        let first = new_hermes_session_token().expect("generate first loopback token");
        let second = new_hermes_session_token().expect("generate second loopback token");

        assert_eq!(first.len(), 64);
        assert_eq!(second.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(second.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
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
    fn graph_memory_failure_keeps_its_stage_without_exposing_adapter_output() {
        let error = GraphMemoryOrchestratorError::Graphify(GraphMemoryPortError::new(
            GraphMemoryStage::Graphify,
            PortErrorKind::Unavailable,
            GraphMemoryFailureCertainty::Known,
            "GRAPHIFY_PRIVATE_EXTRACT_STDERR_REJECTED",
        ));

        assert_eq!(
            graph_memory_execution_error(error).code(),
            "LATTICE_GRAPH_MEMORY_GRAPHIFY_REJECTED"
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
        let cancellation = AtomicBool::new(false);
        let exit = run_openclaw_pump(pump, &cancellation, |failure| {
            observed.push(failure);
            if fatal_openclaw_pump_error(failure.kind) {
                OpenClawPumpControl::Stop(LatticedErrorKind::Transport)
            } else {
                OpenClawPumpControl::Continue
            }
        });

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(exit, OpenClawPumpExit::Fatal(LatticedErrorKind::Transport));
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
    fn full_chain_eof_joins_listener_before_service_teardown() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let stop_order = Arc::clone(&order);
        let finish_order = Arc::clone(&order);
        let active_subtree = Arc::new(AtomicUsize::new(1));
        let finish_active_subtree = Arc::clone(&active_subtree);

        finish_full_chain_surfaces(
            Ok(()),
            move || {
                stop_order.lock().expect("shutdown order").push("listener");
                Ok(OpenClawPumpExit::Cancelled)
            },
            move |result| {
                assert_eq!(
                    finish_order.lock().expect("shutdown order").as_slice(),
                    &["listener"]
                );
                finish_order.lock().expect("shutdown order").push("service");
                finish_active_subtree.store(0, Ordering::SeqCst);
                result
            },
        )
        .expect("idle EOF teardown");

        assert_eq!(active_subtree.load(Ordering::SeqCst), 0);
        assert_eq!(
            order.lock().expect("shutdown order").as_slice(),
            &["listener", "service"]
        );
    }

    #[test]
    fn fatal_openclaw_exit_uses_the_same_service_teardown_path() {
        let teardown_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&teardown_calls);
        let failure = finish_full_chain_surfaces(
            Ok(()),
            || Ok(OpenClawPumpExit::Fatal(LatticedErrorKind::Transport)),
            move |result| {
                calls.fetch_add(1, Ordering::SeqCst);
                result
            },
        )
        .expect_err("fatal listener failure remains terminal after teardown");

        assert_eq!(failure.kind(), LatticedErrorKind::Transport);
        assert_eq!(teardown_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn openclaw_pump_owner_cancels_and_joins_without_detaching() {
        let mut owner = OpenClawPumpOwner::spawn(
            IdleOpenClawPump,
            SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
            |_| panic!("idle pump has no transport failure"),
        )
        .expect("spawn typed pump owner");

        assert_eq!(
            owner.shutdown().expect("cancel and join pump"),
            OpenClawPumpExit::Cancelled
        );
        assert!(!owner.armed);
        assert!(owner.handle.is_none());
    }

    #[test]
    fn fatal_surface_without_stdin_eof_returns_after_exact_teardown() {
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let (release_sender, release_receiver) = mpsc::sync_channel(1);
        let (completed_sender, completed_receiver) = mpsc::sync_channel(1);
        let (teardown_sender, teardown_receiver) = mpsc::sync_channel(1);
        let fatal_surface = Arc::new(AtomicBool::new(false));
        let mut stdin_owner = ProcessLifetimeStdinReader::spawn(
            NeverEofTestInput {
                started: Some(started_sender),
                release: release_receiver,
                completed: completed_sender,
            },
            Arc::clone(&fatal_surface),
        )
        .expect("spawn input-only reader");
        let active_provider = Arc::new(AtomicUsize::new(1));
        let active_subtree = Arc::new(AtomicUsize::new(1));
        let trigger_provider = Arc::clone(&active_provider);
        let trigger_subtree = Arc::clone(&active_subtree);
        let trigger_fatal = Arc::clone(&fatal_surface);
        let order = Arc::new(Mutex::new(Vec::new()));
        let trigger_order = Arc::clone(&order);
        let stop_order = Arc::clone(&order);
        let finish_order = Arc::clone(&order);
        let trigger = thread::spawn(move || {
            started_receiver.recv().expect("stdin is blocked");
            // This mirrors the production fatal callback: exact provider and
            // subtree teardown is completed by the pump before its terminal can
            // satisfy the main owner's join. Stdin may unblock earlier, but main
            // cannot return until `teardown_receiver` proves the exact terminal.
            trigger_fatal.store(true, AtomicOrdering::Release);
            trigger_provider.store(0, Ordering::SeqCst);
            trigger_subtree.store(0, Ordering::SeqCst);
            trigger_order
                .lock()
                .expect("fatal order")
                .push("exact-teardown");
            teardown_sender.send(()).expect("publish exact teardown");
        });

        let failure = serve_full_chain_stdio(
            &mut stdin_owner,
            |input| {
                let mut byte = [0_u8; 1];
                input
                    .read(&mut byte)
                    .map(|_| ())
                    .map_err(|_| LatticedError::new(LatticedErrorKind::Transport))
            },
            move || {
                teardown_receiver
                    .recv()
                    .expect("pump terminal follows exact teardown");
                stop_order
                    .lock()
                    .expect("fatal order")
                    .push("listener-joined");
                Ok(OpenClawPumpExit::Fatal(LatticedErrorKind::Transport))
            },
            move |result| {
                assert_eq!(active_provider.load(Ordering::SeqCst), 0);
                assert_eq!(active_subtree.load(Ordering::SeqCst), 0);
                finish_order
                    .lock()
                    .expect("fatal order")
                    .push("service-finished");
                result
            },
        )
        .expect_err("fatal surface returns without stdin EOF");
        trigger.join().expect("fatal trigger");

        assert_eq!(failure.kind(), LatticedErrorKind::Transport);
        assert!(stdin_owner.detached_after_surfaces);
        assert_eq!(
            order.lock().expect("fatal order").as_slice(),
            &["exact-teardown", "listener-joined", "service-finished"]
        );
        release_sender.send(()).expect("release input-only thread");
        completed_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("input-only thread exits after test release");
    }

    #[test]
    fn stdin_eof_joins_input_reader_after_surface_teardown() {
        let fatal_surface = Arc::new(AtomicBool::new(false));
        let mut stdin_owner =
            ProcessLifetimeStdinReader::spawn(io::Cursor::new(Vec::<u8>::new()), fatal_surface)
                .expect("spawn EOF reader");

        serve_full_chain_stdio(
            &mut stdin_owner,
            |input| {
                let mut byte = [0_u8; 1];
                assert_eq!(input.read(&mut byte).expect("read EOF"), 0);
                Ok(())
            },
            || Ok(OpenClawPumpExit::Cancelled),
            |result| result,
        )
        .expect("EOF joins all owned surfaces");

        assert!(!stdin_owner.detached_after_surfaces);
        assert!(stdin_owner.handle.is_none());
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

    #[test]
    fn historical_terminal_routes_only_exact_delivery_states() {
        assert_eq!(
            historical_delivery_status_action(DeliveryStatus::NotStarted),
            HistoricalDeliveryStatusAction::NotStarted,
        );
        assert_eq!(
            historical_delivery_status_action(DeliveryStatus::Failed),
            HistoricalDeliveryStatusAction::Failed,
        );
        assert_eq!(
            historical_delivery_status_action(DeliveryStatus::ReconciliationRequired),
            HistoricalDeliveryStatusAction::ReconciliationRequired,
        );
        assert_eq!(
            historical_delivery_status_action(DeliveryStatus::Completed),
            HistoricalDeliveryStatusAction::ReceiptMismatch,
        );
    }

    #[test]
    fn historical_delivery_terminal_exposes_only_a_closed_failure_projection() {
        let status = historical_terminal_status_json(
            "FAILED",
            "CODEX",
            "CODEX_APP_SERVER_INVALID_CODEX_HOME",
        );

        assert_eq!(
            status,
            json!({
                "component": "delivery-receipt",
                "failure_code": "CODEX_APP_SERVER_INVALID_CODEX_HOME",
                "failure_stage": "CODEX",
                "scope": "receipt-only",
                "status": "FAILED",
            })
        );
        for forbidden in [
            "profile",
            "request_id",
            "configuration_digest",
            "intent_digest",
            "outcome_digest",
            "receipt_digest",
        ] {
            assert!(status.get(forbidden).is_none(), "{forbidden}");
        }
    }
}
