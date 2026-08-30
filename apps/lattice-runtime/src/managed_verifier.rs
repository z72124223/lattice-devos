//! Concrete, process-isolated verifier for one bounded managed-task worktree.
//!
//! Every Git invocation uses a fixed argument vector. Native verification uses
//! `std::process` directly; WSL2 verification crosses only the sealed GIT-role
//! bridge and retains one independent process-fence receipt per invocation.
//! Natural-language objectives are absent from this API. Project checks are
//! selected only from files captured at the exact base commit.

use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
use std::ffi::OsStr;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt as _;
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, RawHandle};

#[cfg(windows)]
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
};

use lattice_artifact_store::{ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence};
use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_codex_adapter::SupervisedDuplexChild;
use lattice_contracts::{ContentDigest, ProjectId, task_ingress_text_contains_recognized_secret};
use lattice_ports::{
    ManagedPortError, ManagedPortErrorKind, ManagedPortResult, ManagedReviewEvidenceSink,
    ManagedVerificationEvidence, ManagedVerificationPort, ManagedVerificationPreparation,
    ManagedVerificationRequest,
};
use lattice_postgres_foreman::ExecutionEnvironmentDescriptor;
use lattice_task_ledger::{
    VerificationOutcome, VerifiedTaskExecutionBinding, VerifiedWorkerAttemptRecord,
    VerifiedWorkerObservationRecord,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::managed_file_identity::ManagedEffectBundleGuard;
use crate::managed_semantic_reviewer::{
    ManagedSemanticReviewResult, ManagedSemanticReviewRunner, ManagedSemanticReviewSubject,
};
use crate::managed_task_spec::{managed_protected_control_path, managed_scope_rule_valid};

const SNAPSHOT_SCHEMA: &str = "lattice.managed-git-snapshot/1.0";
const WSL_GIT_TRANSPORT_FAILURE_SCHEMA: &str = "lattice.managed-wsl2-git-transport-failure/1.0";
const PRODUCER_ID: &str = "lattice-runtime-managed-verifier";
const PRODUCER_VERSION: &str = "1.0";
const COMMIT_MESSAGE: &[u8] = b"LATTICE managed verification candidate\n";
const MAX_GIT_OUTPUT_BYTES: usize = 8 * 1_048_576;
const MAX_GIT_INDEX_BYTES: u64 = 16 * 1_024 * 1_024;
const MAX_GIT_CONTROL_FILE_BYTES: u64 = 1_024 * 1_024;
const MAX_GIT_CONTROL_AGGREGATE_BYTES: u64 = 8 * 1_024 * 1_024;
const MAX_GITFILE_BYTES: u64 = 4 * 1_024;
const MAX_CHANGED_PATHS: usize = 4_096;
const MAX_CANDIDATE_FILE_BYTES: u64 = 32 * 1_024 * 1_024;
const MAX_CANDIDATE_AGGREGATE_BYTES: u64 = 128 * 1_024 * 1_024;
const MAX_REPOSITORY_PATHS: usize = 32_768;
const MAX_IGNORED_STATE_BYTES: usize = 8 * 1_024 * 1_024;
const MAX_TRUSTED_RULE_FILES: usize = 256;
const MAX_TRUSTED_CONTROL_FILES: usize = 1_024;
const MAX_TRUSTED_CONTROL_BYTES: usize = 8 * 1_024 * 1_024;
const MAX_TRUSTED_NPM_SCRIPTS: usize = 128;
const MAX_TRUSTED_EXECUTABLE_BYTES: u64 = 512 * 1_024 * 1_024;
// Every vendored input is held by a deny-write/delete handle while Cargo can
// execute it. Keep the inventory explicitly bounded so handle and memory use
// cannot be driven by an untrusted repository.
const MAX_CARGO_VENDOR_FILES: usize = 4_095;
const MAX_CARGO_VENDOR_BYTES: u64 = 512 * 1_024 * 1_024;
const MAX_CARGO_VENDOR_FILE_BYTES: u64 = 64 * 1_024 * 1_024;
const MAX_CARGO_VENDOR_CONFIG_BYTES: u64 = 16 * 1_024;
const MAX_ANCESTOR_DIRECTORIES: usize = 64;
const MAX_TIMEOUT: Duration = Duration::from_mins(15);
const PROCESS_PIPE_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WSL_GIT_INVOCATIONS: u64 = 10_000;
// A Git hash-object request carries one already-bounded candidate file as
// base64. Keep transport large enough for MAX_CANDIDATE_FILE_BYTES plus JSON
// framing while still rejecting aggregate or unbounded stdin.
const MAX_WSL_GIT_REQUEST_BYTES: usize = 48 * 1_048_576;
const MAX_WSL_GIT_RESULT_BYTES: usize = 1_048_576;
const MAX_WSL_GIT_RECEIPT_BUNDLE_BYTES: usize = 524_288;
const MAX_WSL_GIT_ARGUMENTS: usize = 256;
const MAX_WSL_GIT_ARGUMENT_BYTES: usize = 65_536;
const CANDIDATE_INDEX_FILE: &str = "candidate-index";
const WSL2_SUPERVISOR_BOOTSTRAP_SHA256: &str =
    "446b12b3d83b8619d8c2da532d3dc5cef9d1833e636d4645570a750e438d7e9c";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
type GitPreSpawnFailpoint = (PathBuf, Box<dyn FnOnce() -> bool + Send>);
#[cfg(test)]
static GIT_PRE_SPAWN_FAILPOINT: std::sync::Mutex<Option<GitPreSpawnFailpoint>> =
    std::sync::Mutex::new(None);
#[cfg(test)]
type ToolchainPreSealFailpoint = (PathBuf, Box<dyn FnOnce(&[PathBuf]) + Send>);
#[cfg(test)]
static TOOLCHAIN_PRE_SEAL_FAILPOINT: std::sync::Mutex<Option<ToolchainPreSealFailpoint>> =
    std::sync::Mutex::new(None);
#[cfg(test)]
static PROCESS_TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Trusted, objective-free configuration captured before worker execution.
#[derive(Clone, Debug)]
pub struct ManagedVerifierConfig {
    project_id: ProjectId,
    repository: PathBuf,
    git_executable: PathBuf,
    sandbox_executable: Option<PathBuf>,
    node_executable: Option<PathBuf>,
    npm_executable: Option<PathBuf>,
    cargo_executable: Option<PathBuf>,
    worktree_digest: ContentDigest,
    allowed_paths: Vec<String>,
    created_at: String,
    command_timeout: Duration,
    effect_bundle_guard: Option<ManagedEffectBundleGuard>,
    runtime_effect_bundle_guard: Option<ManagedEffectBundleGuard>,
    execution_environment: Option<ExecutionEnvironmentDescriptor>,
    execution_preflight: Option<VerifiedManagedEvidence>,
    wsl_verifier_bridge_path: Option<PathBuf>,
}

impl ManagedVerifierConfig {
    /// Constructs one closed verifier configuration. Executables and the
    /// repository must be absolute; allowed paths come from trusted Task Spec
    /// policy and never from the objective.
    ///
    /// # Errors
    ///
    /// Returns a closed configuration error when an executable, repository,
    /// scope rule, timestamp, or timeout cannot be safely admitted.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_id: ProjectId,
        repository: impl Into<PathBuf>,
        git_executable: impl Into<PathBuf>,
        sandbox_executable: Option<PathBuf>,
        npm_executable: Option<PathBuf>,
        cargo_executable: Option<PathBuf>,
        worktree_digest: ContentDigest,
        allowed_paths: Vec<String>,
        created_at: impl Into<String>,
        command_timeout: Duration,
    ) -> ManagedPortResult<Self> {
        let repository = repository.into();
        let git_executable = git_executable.into();
        let created_at = created_at.into();
        if !repository.is_absolute()
            || !git_executable.is_absolute()
            || sandbox_executable
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
            || npm_executable
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
            || cargo_executable
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
            || command_timeout.is_zero()
            || command_timeout > MAX_TIMEOUT
            || allowed_paths.is_empty()
            || allowed_paths.iter().any(|rule| !valid_scope_rule(rule))
            || OffsetDateTime::parse(&created_at, &Rfc3339)
                .ok()
                .is_none_or(|value| value.offset() != time::UtcOffset::UTC)
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_CONFIG_REJECTED"));
        }
        Ok(Self {
            project_id,
            repository,
            git_executable,
            sandbox_executable,
            node_executable: None,
            npm_executable,
            cargo_executable,
            worktree_digest,
            allowed_paths,
            created_at,
            command_timeout,
            effect_bundle_guard: None,
            runtime_effect_bundle_guard: None,
            execution_environment: None,
            execution_preflight: None,
            wsl_verifier_bridge_path: None,
        })
    }

    pub(crate) fn with_effect_bundle_guard(mut self, guard: ManagedEffectBundleGuard) -> Self {
        self.effect_bundle_guard = Some(guard);
        self
    }

    pub(crate) fn with_runtime_effect_bundle_guard(
        mut self,
        guard: ManagedEffectBundleGuard,
    ) -> Self {
        self.runtime_effect_bundle_guard = Some(guard);
        self
    }

    /// Binds the exact Node executable admitted by the server-owned runtime
    /// configuration. Npm is never spawned by the verifier; it is only an
    /// ecosystem availability signal while the captured package scripts are
    /// compiled into an exact [`TrustedNodePlan`].
    pub fn with_node_executable(
        mut self,
        node_executable: impl Into<PathBuf>,
    ) -> ManagedPortResult<Self> {
        let node_executable = node_executable.into();
        if !node_executable.is_absolute() {
            return Err(known("LATTICE_MANAGED_VERIFIER_CONFIG_REJECTED"));
        }
        self.node_executable = Some(node_executable);
        Ok(self)
    }

    /// Binds every production verifier command to one durable WSL2
    /// descriptor and its exact zero-model preflight receipt.
    pub(crate) fn with_wsl_execution_domain(
        mut self,
        descriptor: ExecutionEnvironmentDescriptor,
        preflight: VerifiedManagedEvidence,
        bridge_path: impl Into<PathBuf>,
    ) -> ManagedPortResult<Self> {
        let bridge_path = bridge_path.into();
        let receipt: Value = serde_json::from_slice(preflight.bytes())
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
        if self.execution_environment.is_some()
            || self.execution_preflight.is_some()
            || self.wsl_verifier_bridge_path.is_some()
            || !bridge_path.is_absolute()
            || descriptor.path_mapping_windows_path()
                != self.repository.to_str().unwrap_or_default()
            || descriptor.path_mapping_linux_path() != descriptor.linux_repository_path()
            || preflight.kind() != ManagedEvidenceKind::WorkerLifecycle
            || preflight.payload_schema() != "lattice.wsl2-zero-model-preflight/1.0"
            || receipt.get("status").and_then(Value::as_str) != Some("PASS")
            || receipt
                .get("execution_environment_ref")
                .and_then(Value::as_str)
                != Some(descriptor.environment_ref().as_str())
            || receipt.get("repository_head").and_then(Value::as_str)
                != Some(descriptor.repository_head())
            || receipt.get("linux_cwd").and_then(Value::as_str)
                != Some(descriptor.linux_repository_path())
            || receipt.get("provider_effect_count").and_then(Value::as_u64) != Some(0)
        {
            return Err(known(
                "LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED",
            ));
        }
        self.execution_environment = Some(descriptor);
        self.execution_preflight = Some(preflight);
        self.wsl_verifier_bridge_path = Some(bridge_path);
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrustedCheckKind {
    NpmVerify,
    CargoTest,
}

impl TrustedCheckKind {
    const fn id(self) -> &'static str {
        match self {
            Self::NpmVerify => "trusted-node-plan-v1",
            Self::CargoTest => "cargo-test-locked-offline-v1",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedCheck {
    kind: TrustedCheckKind,
    control_files: Vec<TrustedControlFile>,
    control_profile: &'static str,
    node_plan: Option<TrustedNodePlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedNodePlan {
    invocations: Vec<TrustedNodeInvocation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedNodeInvocation {
    script: String,
    arguments: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedControlFile {
    path: String,
    canonical_path_digest: ContentDigest,
    base_file_digest: ContentDigest,
    byte_len: u64,
    file_identity: TrustedFileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedExecutable {
    role: &'static str,
    path: PathBuf,
    canonical_path_digest: ContentDigest,
    content_digest: ContentDigest,
    byte_len: u64,
    file_identity: TrustedFileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedFileIdentity {
    namespace: &'static str,
    volume_or_device: u64,
    file: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedPathAnchor {
    path: PathBuf,
    canonical_path_digest: ContentDigest,
    file_identity: TrustedFileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TrustedGitEntry {
    Directory(TrustedPathAnchor),
    File {
        path: PathBuf,
        facts: CapturedFileFacts,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CapturedFileFacts {
    canonical_path_digest: ContentDigest,
    content_digest: ContentDigest,
    byte_len: u64,
    file_identity: TrustedFileIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedTreeFacts {
    root: TrustedPathAnchor,
    content_digest: ContentDigest,
    identity_digest: ContentDigest,
    file_count: usize,
    byte_len: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedCargoSourceSnapshot {
    vendor: TrustedTreeFacts,
    config_path: PathBuf,
    config: CapturedFileFacts,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedAmbientGuard {
    profile: &'static str,
    digest: ContentDigest,
}

struct ControlDirectoryGuard {
    path: PathBuf,
    armed: bool,
}

impl ControlDirectoryGuard {
    const fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ControlDirectoryGuard {
    fn drop(&mut self) {
        if self.armed {
            remove_owned_control_directory(&self.path);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedRule {
    path: String,
    base_file_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CheckResult {
    id: &'static str,
    passed: bool,
    wsl_receipt_json: Option<String>,
}

#[derive(Clone, Debug)]
struct WslGitReceiptRecord {
    sequence: u64,
    invocation_digest: String,
    result: Value,
}

#[derive(Clone, Debug)]
struct WslGitPreflightContext {
    worktree_ref: String,
    preflight_fence: String,
    credential_seal_digest: String,
    timeout_ms: u64,
    stdout_limit_bytes: u64,
    stderr_limit_bytes: u64,
    retry_of: Option<String>,
    reconnect_of: Option<String>,
    unit_prefix: String,
}

#[derive(Clone, Debug)]
struct PreparedCandidate {
    preparation: ManagedVerificationPreparation,
    binding_digest: ContentDigest,
    attempt_digest: ContentDigest,
    terminal_digest: ContentDigest,
    commit_oid: String,
    tree_oid: String,
    diff_digest: ContentDigest,
    changed_paths: Vec<String>,
    checks: Vec<CheckResult>,
    semantic_review: Option<ManagedSemanticReviewResult>,
}

/// Concrete implementation of [`ManagedVerificationPort`].
pub struct ManagedVerificationAdapter {
    config: ManagedVerifierConfig,
    repository: PathBuf,
    repository_anchor: TrustedPathAnchor,
    git_entry: TrustedGitEntry,
    git_executable: PathBuf,
    sandbox_executable: Option<PathBuf>,
    node_executable: Option<PathBuf>,
    cargo_executable: Option<PathBuf>,
    trusted_executables: Vec<TrustedExecutable>,
    base_effect_guard: Option<ManagedEffectBundleGuard>,
    active_toolchain_guard: Option<ManagedEffectBundleGuard>,
    cargo_source_guard: Option<ManagedEffectBundleGuard>,
    npm_ancestor_guard: Option<TrustedAmbientGuard>,
    cargo_ancestor_guard: Option<TrustedAmbientGuard>,
    cargo_source_snapshot: Option<TrustedCargoSourceSnapshot>,
    fixed_path: OsString,
    control_directory: PathBuf,
    hooks_directory: PathBuf,
    global_config: PathBuf,
    git_directory: PathBuf,
    common_git_directory: PathBuf,
    object_directory: PathBuf,
    git_directory_anchor: Option<TrustedPathAnchor>,
    common_git_directory_anchor: Option<TrustedPathAnchor>,
    object_directory_anchor: Option<TrustedPathAnchor>,
    git_guard_ready: bool,
    base_commit_oid: String,
    base_commit_digest: ContentDigest,
    command_identity: ContentDigest,
    trusted_checks: Vec<TrustedCheck>,
    trusted_rules: Vec<TrustedRule>,
    initial_refs: Vec<u8>,
    initial_index: Option<CapturedFileFacts>,
    initial_control_digest: ContentDigest,
    initial_ignored_state: ContentDigest,
    wsl_git_sequence: AtomicU64,
    wsl_git_receipts: Mutex<Vec<WslGitReceiptRecord>>,
    prepared: Option<PreparedCandidate>,
    semantic_reviewer: Option<Box<dyn ManagedSemanticReviewRunner>>,
}

impl ManagedVerificationAdapter {
    /// Captures the exact current base commit, trusted check policy, Git refs,
    /// real index, and security-sensitive Git control files.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed verifier error when repository state, trusted
    /// controls, or any executable identity cannot be captured exactly.
    pub fn new(config: ManagedVerifierConfig) -> ManagedPortResult<Self> {
        let wsl_execution = config.execution_environment.is_some();
        let repository = canonical_directory(&config.repository)?;
        let repository_anchor =
            capture_path_anchor(&repository, "LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED")?;
        let git_entry = capture_git_entry(
            &repository.join(".git"),
            "LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED",
        )?;
        let git_executable = canonical_file(
            config
                .execution_environment
                .as_ref()
                .map_or(config.git_executable.as_path(), |descriptor| {
                    Path::new(descriptor.gateway().path())
                }),
        )?;
        let sandbox_executable = (!wsl_execution)
            .then_some(config.sandbox_executable.as_deref())
            .flatten()
            .map(canonical_file)
            .transpose()?;
        let _npm_executable = (!wsl_execution)
            .then_some(config.npm_executable.as_deref())
            .flatten()
            .map(canonical_file)
            .transpose()?;
        let cargo_executable = (!wsl_execution)
            .then_some(config.cargo_executable.as_deref())
            .flatten()
            .map(canonical_file)
            .transpose()?;
        let (control_directory, hooks_directory, global_config) = if wsl_execution {
            create_wsl_control_directory(
                config.execution_environment.as_ref().ok_or_else(|| {
                    known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REQUIRED")
                })?,
                config.execution_preflight.as_ref().ok_or_else(|| {
                    known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REQUIRED")
                })?,
            )?
        } else {
            create_control_directory()?
        };
        // Own the directory immediately: failures while resolving the remaining
        // executable closure must not leak verifier-controlled state.
        let mut control_directory_guard = ControlDirectoryGuard::new(control_directory.clone());
        let guards = [
            config.effect_bundle_guard.as_ref(),
            config.runtime_effect_bundle_guard.as_ref(),
        ];
        let mut fallback_effect_paths = Vec::new();
        let (git_identity, git_sealed) =
            capture_trusted_executable_with_guards("git", &git_executable, &guards)?;
        if !git_sealed {
            fallback_effect_paths.push(git_executable.clone());
        }
        let mut trusted_executables = vec![git_identity];
        if let Some(sandbox) = sandbox_executable.as_deref() {
            let (identity, sealed) =
                capture_trusted_executable_with_guards("sandbox", sandbox, &guards)?;
            if !sealed {
                fallback_effect_paths.push(sandbox.to_path_buf());
            }
            trusted_executables.push(identity);
        }
        let node_executable = config
            .node_executable
            .as_deref()
            .map(canonical_file)
            .transpose()?;
        if let Some(node) = node_executable.as_deref() {
            let (identity, sealed) = capture_trusted_executable_with_guards("node", node, &guards)?;
            if !sealed {
                fallback_effect_paths.push(node.to_path_buf());
            }
            trusted_executables.push(identity);
        }
        if let Some(bridge) = config.wsl_verifier_bridge_path.as_deref() {
            let bridge = canonical_file(bridge)?;
            let (identity, sealed) =
                capture_trusted_executable_with_guards("wsl-verifier-bridge", &bridge, &guards)?;
            if !sealed {
                fallback_effect_paths.push(bridge);
            }
            trusted_executables.push(identity);
        }
        if let Some(cargo) = cargo_executable.as_deref() {
            let (identity, sealed) =
                capture_trusted_executable_with_guards("cargo", cargo, &guards)?;
            if !sealed {
                fallback_effect_paths.push(cargo.to_path_buf());
            }
            trusted_executables.push(identity);
            let rustc = resolve_required_program(cargo.parent(), "rustc")?;
            let (identity, sealed) =
                capture_trusted_executable_with_guards("rustc-proxy", &rustc, &guards)?;
            if !sealed {
                fallback_effect_paths.push(rustc.clone());
            }
            trusted_executables.push(identity);
            let rustdoc = resolve_required_program(cargo.parent(), "rustdoc")?;
            let (identity, sealed) =
                capture_trusted_executable_with_guards("rustdoc-proxy", &rustdoc, &guards)?;
            if !sealed {
                fallback_effect_paths.push(rustdoc.clone());
            }
            trusted_executables.push(identity);
        }
        let fixed_path = trusted_path(&trusted_executables)?;
        let base_effect_guard = (!fallback_effect_paths.is_empty())
            .then(|| {
                ManagedEffectBundleGuard::capture(
                    fallback_effect_paths
                        .into_iter()
                        .map(|path| (path, MAX_TRUSTED_EXECUTABLE_BYTES)),
                )
            })
            .transpose()
            .map_err(|()| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
        if let Some(guard) = config.effect_bundle_guard.as_ref() {
            guard
                .verify()
                .map_err(|()| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
        }

        let mut adapter = Self {
            config,
            repository,
            repository_anchor,
            git_entry,
            git_executable,
            sandbox_executable,
            node_executable,
            cargo_executable,
            trusted_executables,
            base_effect_guard,
            active_toolchain_guard: None,
            cargo_source_guard: None,
            npm_ancestor_guard: None,
            cargo_ancestor_guard: None,
            cargo_source_snapshot: None,
            fixed_path,
            control_directory,
            hooks_directory,
            global_config,
            git_directory: PathBuf::new(),
            common_git_directory: PathBuf::new(),
            object_directory: PathBuf::new(),
            git_directory_anchor: None,
            common_git_directory_anchor: None,
            object_directory_anchor: None,
            git_guard_ready: false,
            base_commit_oid: String::new(),
            base_commit_digest: zero_digest()?,
            command_identity: zero_digest()?,
            trusted_checks: Vec::new(),
            trusted_rules: Vec::new(),
            initial_refs: Vec::new(),
            initial_index: None,
            initial_control_digest: zero_digest()?,
            initial_ignored_state: zero_digest()?,
            wsl_git_sequence: AtomicU64::new(0),
            wsl_git_receipts: Mutex::new(Vec::new()),
            prepared: None,
            semantic_reviewer: None,
        };
        // From this point `ManagedVerificationAdapter::drop` owns cleanup,
        // including every fail-closed exit from base capture or verification.
        control_directory_guard.disarm();
        adapter.capture_base()?;
        Ok(adapter)
    }

    /// Installs the separate read-only semantic reviewer selected by the
    /// trusted runtime composition. Without one, mechanically valid work
    /// still fails closed rather than manufacturing a review result.
    #[must_use]
    pub fn with_semantic_reviewer(
        mut self,
        reviewer: Box<dyn ManagedSemanticReviewRunner>,
    ) -> Self {
        self.semantic_reviewer = Some(reviewer);
        self
    }

    /// Returns the captured raw local Git commit object ID.
    #[must_use]
    pub fn base_commit_oid(&self) -> &str {
        &self.base_commit_oid
    }

    /// Returns the SHA-256 commitment used by worker-attempt records.
    #[must_use]
    pub const fn base_commit_digest(&self) -> &ContentDigest {
        &self.base_commit_digest
    }

    /// Returns the candidate commit only after the complete independent
    /// verifier and semantic reviewer have both produced a passing verdict.
    /// Callers must additionally require `finish_managed_attempt` success,
    /// which proves the corresponding verification record is durable, before
    /// protecting this otherwise-unreferenced commit object.
    #[must_use]
    pub fn verified_result_commit_oid(&self) -> Option<&str> {
        self.prepared.as_ref().and_then(|candidate| {
            (candidate.checks.iter().all(|check| check.passed)
                && candidate
                    .semantic_review
                    .as_ref()
                    .is_some_and(|review| review.verdict().passed()))
            .then_some(candidate.commit_oid.as_str())
        })
    }

    /// Returns the closed identity of base-captured verification commands.
    #[must_use]
    pub const fn command_identity(&self) -> &ContentDigest {
        &self.command_identity
    }

    fn capture_base(&mut self) -> ManagedPortResult<()> {
        let top = self.git_success(&["rev-parse", "--show-toplevel"], None, None)?;
        if let Some(descriptor) = self.config.execution_environment.as_ref() {
            if trim_ascii(&top)? != descriptor.linux_repository_path() {
                return Err(known("LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED"));
            }
        } else {
            let top = path_from_git_stdout(&top)?;
            if canonical_directory(&top)? != self.repository {
                return Err(known("LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED"));
            }
        }
        self.base_commit_oid = oid_from_output(&self.git_success(
            &["rev-parse", "--verify", "HEAD^{commit}"],
            None,
            None,
        )?)?;
        self.base_commit_digest = sha256_bytes(self.base_commit_oid.as_bytes())?;
        let git_directory = path_from_git_stdout(&self.git_success(
            &["rev-parse", "--absolute-git-dir"],
            None,
            None,
        )?)?;
        let common_git_directory = path_from_git_stdout(&self.git_success(
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            None,
            None,
        )?)?;
        self.git_directory = canonical_directory(&self.execution_path_to_host(&git_directory)?)?;
        self.common_git_directory =
            canonical_directory(&self.execution_path_to_host(&common_git_directory)?)?;
        self.git_directory_anchor = Some(capture_path_anchor(
            &self.git_directory,
            "LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED",
        )?);
        self.common_git_directory_anchor = Some(capture_path_anchor(
            &self.common_git_directory,
            "LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED",
        )?);
        self.object_directory = canonical_directory(&self.common_git_directory.join("objects"))
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED"))?;
        self.object_directory_anchor = Some(capture_path_anchor(
            &self.object_directory,
            "LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED",
        )?);
        self.initial_index = capture_optional_file_facts(
            &self.git_directory.join("index"),
            MAX_GIT_INDEX_BYTES,
            "LATTICE_MANAGED_VERIFIER_GIT_INDEX_REJECTED",
        )?;
        self.initial_control_digest =
            git_control_digest(&self.git_directory, &self.common_git_directory)?;
        self.git_guard_ready = true;
        self.initial_refs = self.capture_refs()?;
        self.initial_ignored_state = self.capture_ignored_state()?;
        self.trusted_checks = self.capture_trusted_checks()?;
        if self.config.execution_environment.is_none()
            && self
                .trusted_checks
                .iter()
                .any(|check| check.kind == TrustedCheckKind::NpmVerify)
        {
            self.npm_ancestor_guard = Some(capture_ancestor_absence_guard(
                &self.repository,
                TrustedCheckKind::NpmVerify,
                "LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED",
            )?);
        }
        if self.config.execution_environment.is_none()
            && self
                .trusted_checks
                .iter()
                .any(|check| check.kind == TrustedCheckKind::CargoTest)
        {
            self.cargo_ancestor_guard = Some(capture_ancestor_absence_guard(
                &self.repository,
                TrustedCheckKind::CargoTest,
                "LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED",
            )?);
        }
        self.capture_active_cargo_toolchain()?;
        self.fixed_path = trusted_path(&self.trusted_executables)?;
        self.cargo_source_snapshot = self.capture_cargo_source_snapshot()?;
        self.trusted_rules = self.capture_trusted_rules()?;
        self.command_identity = command_identity(
            &self.trusted_checks,
            &self.trusted_rules,
            &self.config.allowed_paths,
            &self.trusted_executables,
            (
                &self.repository_anchor,
                &self.git_entry,
                self.git_directory_anchor
                    .as_ref()
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED"))?,
                self.common_git_directory_anchor
                    .as_ref()
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED"))?,
                self.object_directory_anchor
                    .as_ref()
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_REPOSITORY_REJECTED"))?,
            ),
            self.cargo_source_snapshot.as_ref(),
            self.npm_ancestor_guard.as_ref(),
            self.cargo_ancestor_guard.as_ref(),
            self.config.execution_environment.as_ref(),
        )?;
        Ok(())
    }

    fn execution_path_to_host(&self, path: &Path) -> ManagedPortResult<PathBuf> {
        let Some(descriptor) = self.config.execution_environment.as_ref() else {
            return Ok(path.to_path_buf());
        };
        wsl_linux_to_unc(path, descriptor.distribution())
    }

    fn capture_active_cargo_toolchain(&mut self) -> ManagedPortResult<()> {
        if self.config.execution_environment.is_some() {
            return Ok(());
        }
        if !self
            .trusted_checks
            .iter()
            .any(|check| check.kind == TrustedCheckKind::CargoTest)
        {
            return Ok(());
        }
        let rustc_proxy = self
            .trusted_executables
            .iter()
            .find(|trusted| trusted.role == "rustc-proxy")
            .map(|trusted| trusted.path.clone())
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
        let probe_root = self.control_directory.join("toolchain-probe");
        let codex_home = probe_root.join("codex-home");
        let process_temp = probe_root.join("temp");
        let process_home = probe_root.join("home");
        let cargo_home = probe_root.join("cargo-home");
        for directory in [&codex_home, &process_temp, &process_home, &cargo_home] {
            fs::create_dir_all(directory)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
        }
        let environment = safe_process_environment(
            &codex_home,
            &process_temp,
            &process_home,
            &cargo_home,
            &self.fixed_path,
        );
        let mut environment = environment;
        environment.push((
            OsString::from("RUSTUP_HOME"),
            source_rustup_home()?.into_os_string(),
        ));
        self.verify_effect_guards()?;
        let result = run_process(
            &rustc_proxy,
            &[OsString::from("--print"), OsString::from("sysroot")],
            &self.repository,
            &environment,
            None,
            self.config.command_timeout,
            &self.control_directory,
            true,
            true,
        )?;
        self.verify_effect_guards()?;
        if !result.status.success() {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"));
        }
        let sysroot = std::str::from_utf8(&result.stdout)
            .ok()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
        let sysroot = canonical_directory(&sysroot)?;
        let bin = canonical_directory(&sysroot.join("bin"))?;
        let mut active_paths = Vec::with_capacity(3);
        for (role, name) in [
            (
                "cargo-toolchain",
                if cfg!(windows) { "cargo.exe" } else { "cargo" },
            ),
            (
                "rustc-toolchain",
                if cfg!(windows) { "rustc.exe" } else { "rustc" },
            ),
            (
                "rustdoc-toolchain",
                if cfg!(windows) {
                    "rustdoc.exe"
                } else {
                    "rustdoc"
                },
            ),
        ] {
            let path = bin.join(name);
            self.trusted_executables
                .push(capture_trusted_executable(role, &path)?);
            active_paths.push((path, MAX_TRUSTED_EXECUTABLE_BYTES));
        }
        #[cfg(test)]
        {
            let action = TOOLCHAIN_PRE_SEAL_FAILPOINT
                .lock()
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"))?
                .take_if(|(repository, _)| same_path(repository, &self.repository))
                .map(|(_, action)| action);
            if let Some(action) = action {
                let paths = active_paths
                    .iter()
                    .map(|(path, _)| path.clone())
                    .collect::<Vec<_>>();
                action(&paths);
            }
        }
        self.active_toolchain_guard = Some(
            ManagedEffectBundleGuard::capture(active_paths)
                .map_err(|()| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?,
        );
        self.verify_effect_guards()?;
        if !self.trusted_executables_match() {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"));
        }
        // The proxy result is only a locator. Re-run the query through the now
        // sealed active compiler and require the exact same sysroot before any
        // vendor or test effect can be admitted.
        let sealed_rustc = self.trusted_executable_path("rustc-toolchain")?;
        let sealed_probe = run_process(
            sealed_rustc,
            &[OsString::from("--print"), OsString::from("sysroot")],
            &self.repository,
            &environment,
            None,
            self.config.command_timeout,
            &self.control_directory,
            true,
            true,
        )?;
        self.verify_effect_guards()?;
        if !sealed_probe.status.success()
            || std::str::from_utf8(&sealed_probe.stdout)
                .ok()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .and_then(|path| canonical_directory(&path).ok())
                .is_none_or(|observed| observed != sysroot)
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"));
        }
        Ok(())
    }

    fn capture_cargo_source_snapshot(
        &mut self,
    ) -> ManagedPortResult<Option<TrustedCargoSourceSnapshot>> {
        if self.config.execution_environment.is_some() {
            return Ok(None);
        }
        if !self
            .trusted_checks
            .iter()
            .any(|check| check.kind == TrustedCheckKind::CargoTest)
        {
            return Ok(None);
        }
        let lock = self
            .base_control_file("Cargo.lock")?
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
        let lock = std::str::from_utf8(&lock)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
        let mut has_registry_sources = false;
        for source in lock
            .lines()
            .map(str::trim)
            .filter_map(|line| line.strip_prefix("source = "))
        {
            if source != "\"registry+https://github.com/rust-lang/crates.io-index\"" {
                return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
            }
            has_registry_sources = true;
        }

        let source_root = self.control_directory.join("cargo-source-snapshot");
        let vendor = source_root.join("vendor");
        fs::create_dir(&source_root)
            .and_then(|()| fs::create_dir(&vendor))
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_OFFLINE_CACHE_UNAVAILABLE"))?;
        if has_registry_sources {
            self.materialize_cargo_vendor(&vendor)?;
        }

        let before_readonly = capture_bounded_tree(&vendor)?;
        set_bounded_tree_readonly(&vendor, before_readonly.file_count)?;
        let vendor = capture_bounded_tree(&vendor)?;
        let vendor_path = vendor
            .root
            .path
            .to_str()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
        let encoded_path = serde_json::to_string(vendor_path)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
        let config_path = source_root.join("config.toml");
        let config_bytes = format!(
            "[source.crates-io]\nreplace-with = \"lattice-vendored\"\n\n[source.lattice-vendored]\ndirectory = {encoded_path}\n\n[net]\noffline = true\n"
        );
        fs::write(&config_path, config_bytes.as_bytes())
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
        let mut permissions = fs::metadata(&config_path)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&config_path, permissions)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
        let config = capture_file_facts(
            &config_path,
            MAX_CARGO_VENDOR_CONFIG_BYTES,
            "LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED",
        )?;
        let mut sealed_inputs = bounded_tree_files(&vendor.root.path)?
            .into_iter()
            .map(|path| {
                let length = fs::metadata(&path)
                    .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?
                    .len();
                Ok((path, length.max(1)))
            })
            .collect::<ManagedPortResult<Vec<_>>>()?;
        sealed_inputs.push((config_path.clone(), MAX_CARGO_VENDOR_CONFIG_BYTES));
        self.cargo_source_guard = Some(
            ManagedEffectBundleGuard::capture_bounded(
                sealed_inputs,
                MAX_CARGO_VENDOR_FILES.saturating_add(1),
            )
            .map_err(|()| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?,
        );
        self.assert_git_pre_spawn()?;
        Ok(Some(TrustedCargoSourceSnapshot {
            vendor,
            config_path,
            config,
        }))
    }

    fn materialize_cargo_vendor(&self, vendor: &Path) -> ManagedPortResult<()> {
        self.verify_effect_guards()?;
        self.assert_ambient_guard_unchanged(TrustedCheckKind::CargoTest)?;
        let cargo_home = source_cargo_home()?;
        let executable = self.trusted_executable_path("cargo-toolchain")?;
        let rustc = self.trusted_executable_text("rustc-toolchain")?;
        let rustdoc = self.trusted_executable_text("rustdoc-toolchain")?;
        let probe_root = self.control_directory.join("cargo-vendor-probe");
        let codex_home = probe_root.join("codex-home");
        let process_temp = probe_root.join("temp");
        let process_home = probe_root.join("home");
        for directory in [&codex_home, &process_temp, &process_home] {
            fs::create_dir_all(directory)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_OFFLINE_CACHE_UNAVAILABLE"))?;
        }
        let mut environment = safe_process_environment(
            &codex_home,
            &process_temp,
            &process_home,
            &cargo_home,
            &self.fixed_path,
        );
        environment.extend([
            (OsString::from("CARGO_NET_OFFLINE"), OsString::from("true")),
            (OsString::from("RUSTC"), OsString::from(rustc)),
            (OsString::from("RUSTDOC"), OsString::from(rustdoc)),
        ]);
        self.assert_ambient_guard_unchanged(TrustedCheckKind::CargoTest)?;
        let result = run_process(
            executable,
            &[
                OsString::from("vendor"),
                OsString::from("--locked"),
                OsString::from("--offline"),
                OsString::from("--versioned-dirs"),
                vendor.as_os_str().to_owned(),
                OsString::from("--manifest-path"),
                self.repository.join("Cargo.toml").into_os_string(),
            ],
            &self.repository,
            &environment,
            None,
            self.config.command_timeout,
            &self.control_directory,
            false,
            true,
        )?;
        self.verify_effect_guards()?;
        self.assert_ambient_guard_unchanged(TrustedCheckKind::CargoTest)?;
        if !result.status.success() {
            return Err(known(
                "LATTICE_MANAGED_VERIFIER_CARGO_OFFLINE_CACHE_UNAVAILABLE",
            ));
        }
        Ok(())
    }

    fn capture_trusted_rules(&self) -> ManagedPortResult<Vec<TrustedRule>> {
        let listing = self.git_success(
            &[
                "ls-tree",
                "-r",
                "-z",
                "--name-only",
                &self.base_commit_oid,
                "--",
            ],
            None,
            None,
        )?;
        let mut paths = listing
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(path_text)
            .filter_map(|path| match path {
                Ok(path) if trusted_rule_path(&path) => Some(Ok(path)),
                Ok(_) => None,
                Err(failure) => Some(Err(failure)),
            })
            .collect::<ManagedPortResult<Vec<_>>>()?;
        paths.sort();
        paths.dedup();
        if paths.len() > MAX_TRUSTED_RULE_FILES {
            return Err(known("LATTICE_MANAGED_VERIFIER_RULE_LIMIT"));
        }
        paths
            .into_iter()
            .map(|path| {
                let bytes = self
                    .base_file(&path)?
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
                Ok(TrustedRule {
                    path,
                    base_file_digest: sha256_bytes(&bytes)?,
                })
            })
            .collect()
    }

    fn capture_trusted_checks(&self) -> ManagedPortResult<Vec<TrustedCheck>> {
        let mut checks = Vec::new();
        let tracked_paths = self.base_tracked_paths()?;
        self.validate_base_git_attributes(&tracked_paths)?;
        if let Some(bytes) = self.base_control_file("package.json")? {
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
            if value
                .get("scripts")
                .and_then(|scripts| scripts.get("verify"))
                .and_then(Value::as_str)
                .is_some_and(|script| !script.trim().is_empty())
            {
                if self.node_executable.is_none() {
                    return Err(known("LATTICE_MANAGED_VERIFIER_NPM_UNAVAILABLE"));
                }
                if self.sandbox_executable.is_none() && self.config.execution_environment.is_none()
                {
                    return Err(known("LATTICE_MANAGED_VERIFIER_SANDBOX_UNAVAILABLE"));
                }
                let profile = trusted_npm_profile(&tracked_paths, &value)?;
                checks.push(TrustedCheck {
                    kind: TrustedCheckKind::NpmVerify,
                    control_files: self.capture_control_files(&profile.control_paths)?,
                    control_profile: "npm-static-node-plan-v3",
                    node_plan: Some(profile.plan),
                });
            }
        }
        if self.base_control_file("Cargo.toml")?.is_some() {
            if self.cargo_executable.is_none() && self.config.execution_environment.is_none() {
                return Err(known("LATTICE_MANAGED_VERIFIER_CARGO_UNAVAILABLE"));
            }
            if self.sandbox_executable.is_none() && self.config.execution_environment.is_none() {
                return Err(known("LATTICE_MANAGED_VERIFIER_SANDBOX_UNAVAILABLE"));
            }
            let control_paths = tracked_paths
                .iter()
                .filter(|path| cargo_control_path(path))
                .cloned()
                .collect::<Vec<_>>();
            for path in control_paths.iter().filter(|path| cargo_config_path(path)) {
                let bytes = self
                    .base_control_file(path)?
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
                if !supported_cargo_config(&bytes) {
                    return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
                }
            }
            checks.push(TrustedCheck {
                kind: TrustedCheckKind::CargoTest,
                control_files: self.capture_control_files(&control_paths)?,
                control_profile: "cargo-closed-controls-v2",
                node_plan: None,
            });
        }
        if checks
            .iter()
            .any(|check| !self.trusted_control_inventory_matches(check))
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
        Ok(checks)
    }

    fn validate_base_git_attributes(&self, tracked_paths: &[String]) -> ManagedPortResult<()> {
        let attribute_paths = tracked_paths
            .iter()
            .filter(|path| path.rsplit('/').next() == Some(".gitattributes"))
            .collect::<Vec<_>>();
        if attribute_paths.len() > MAX_TRUSTED_RULE_FILES {
            return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
        }
        let mut total_bytes = 0u64;
        for path in attribute_paths {
            let bytes = self
                .base_control_file(path)?
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?;
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_GIT_CONTROL_FILE_BYTES {
                return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
            }
            total_bytes = total_bytes
                .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?;
            if total_bytes > MAX_GIT_CONTROL_AGGREGATE_BYTES || !git_attributes_are_closed(&bytes) {
                return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
            }
        }
        Ok(())
    }

    fn base_tracked_paths(&self) -> ManagedPortResult<Vec<String>> {
        let listing = self.git_success(
            &[
                "ls-tree",
                "-r",
                "-z",
                "--name-only",
                &self.base_commit_oid,
                "--",
            ],
            None,
            None,
        )?;
        let mut paths = listing
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(path_text)
            .collect::<ManagedPortResult<Vec<_>>>()?;
        paths.sort();
        paths.dedup();
        if paths.len() > MAX_REPOSITORY_PATHS {
            return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"));
        }
        Ok(paths)
    }

    fn capture_control_files(
        &self,
        control_paths: &[String],
    ) -> ManagedPortResult<Vec<TrustedControlFile>> {
        let mut paths = control_paths.to_vec();
        paths.sort();
        paths.dedup();
        if paths.is_empty() || paths.len() > MAX_TRUSTED_CONTROL_FILES {
            return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"));
        }
        let mut total = 0usize;
        let mut controls = Vec::with_capacity(paths.len());
        for path in paths {
            let bytes = self
                .base_control_file(&path)?
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
            let remaining = MAX_TRUSTED_CONTROL_BYTES
                .checked_sub(total)
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"))?;
            let current = self.capture_repository_file(
                &path,
                u64::try_from(remaining)
                    .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"))?,
            )?;
            let base_file_digest = sha256_bytes(&bytes)?;
            if current.byte_len != u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                || current.content_digest != base_file_digest
            {
                return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
            }
            total = total
                .checked_add(path.len())
                .and_then(|value| value.checked_add(bytes.len()))
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"))?;
            if total > MAX_TRUSTED_CONTROL_BYTES {
                return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"));
            }
            controls.push(TrustedControlFile {
                path,
                canonical_path_digest: current.canonical_path_digest,
                base_file_digest,
                byte_len: current.byte_len,
                file_identity: current.file_identity,
            });
        }
        Ok(controls)
    }

    fn capture_repository_file(
        &self,
        path: &str,
        max_bytes: u64,
    ) -> ManagedPortResult<CapturedFileFacts> {
        let candidate = safe_repository_file(&self.repository, path)?;
        capture_file_facts(
            &candidate,
            max_bytes,
            "LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED",
        )
    }

    fn base_control_file(&self, path: &str) -> ManagedPortResult<Option<Vec<u8>>> {
        let listing = self.git_success(
            &["ls-tree", "-z", &self.base_commit_oid, "--", path],
            None,
            None,
        )?;
        if listing.is_empty() {
            return Ok(None);
        }
        let expected_suffix = format!("\t{path}\0");
        if !(listing.starts_with(b"100644 blob ") || listing.starts_with(b"100755 blob "))
            || !listing.ends_with(expected_suffix.as_bytes())
            || listing
                .strip_suffix(expected_suffix.as_bytes())
                .is_none_or(|prefix| prefix.contains(&b'\t'))
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
        self.git_success(
            &["show", &format!("{}:{path}", self.base_commit_oid)],
            None,
            None,
        )
        .map(Some)
    }

    fn base_file(&self, path: &str) -> ManagedPortResult<Option<Vec<u8>>> {
        let listing = self.git_success(
            &[
                "ls-tree",
                "-z",
                "--name-only",
                &self.base_commit_oid,
                "--",
                path,
            ],
            None,
            None,
        )?;
        if listing.is_empty() {
            return Ok(None);
        }
        let expected = format!("{path}\0");
        if listing != expected.as_bytes() {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
        self.git_success(
            &["show", &format!("{}:{path}", self.base_commit_oid)],
            None,
            None,
        )
        .map(Some)
    }

    fn validate_records(
        &self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
    ) -> ManagedPortResult<()> {
        if attempt.task_ref() != binding.task_ref()
            || attempt.successor_stream_id() != binding.successor_stream_id()
            || attempt.task_spec_digest() != binding.task_spec_digest()
            || attempt.binding_digest() != binding.binding_digest()
            || attempt.worktree_digest() != &self.config.worktree_digest
            || attempt.base_commit_digest() != &self.base_commit_digest
            || terminal.task_ref() != binding.task_ref()
            || terminal.successor_stream_id() != binding.successor_stream_id()
            || terminal.binding_digest() != binding.binding_digest()
            || terminal.attempt_id() != attempt.attempt_id()
            || terminal.attempt_number() != attempt.attempt_number()
            || terminal.kind() != lattice_task_ledger::WorkerObservationKind::TerminalCompleted
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_BINDING_REJECTED"));
        }
        Ok(())
    }

    fn assert_repository_control_unchanged(&self) -> ManagedPortResult<()> {
        let head = oid_from_output(&self.git_success(
            &["rev-parse", "--verify", "HEAD^{commit}"],
            None,
            None,
        )?)?;
        if head != self.base_commit_oid
            || self.capture_refs()? != self.initial_refs
            || capture_optional_file_facts(
                &self.git_directory.join("index"),
                MAX_GIT_INDEX_BYTES,
                "LATTICE_MANAGED_VERIFIER_GIT_INDEX_REJECTED",
            )? != self.initial_index
            || git_control_digest(&self.git_directory, &self.common_git_directory)?
                != self.initial_control_digest
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_DRIFT"));
        }
        Ok(())
    }

    fn assert_git_pre_spawn(&self) -> ManagedPortResult<()> {
        let repository = capture_path_anchor(
            &self.repository_anchor.path,
            "LATTICE_MANAGED_VERIFIER_GIT_LAYOUT_DRIFT",
        )?;
        let git_entry = capture_git_entry(
            &self.repository.join(".git"),
            "LATTICE_MANAGED_VERIFIER_GIT_LAYOUT_DRIFT",
        )?;
        if repository != self.repository_anchor || git_entry != self.git_entry {
            return Err(known("LATTICE_MANAGED_VERIFIER_GIT_LAYOUT_DRIFT"));
        }
        for anchor in [
            self.git_directory_anchor.as_ref(),
            self.common_git_directory_anchor.as_ref(),
            self.object_directory_anchor.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if capture_path_anchor(&anchor.path, "LATTICE_MANAGED_VERIFIER_GIT_LAYOUT_DRIFT")?
                != *anchor
            {
                return Err(known("LATTICE_MANAGED_VERIFIER_GIT_LAYOUT_DRIFT"));
            }
        }
        if self.git_guard_ready {
            let index = capture_optional_file_facts(
                &self.git_directory.join("index"),
                MAX_GIT_INDEX_BYTES,
                "LATTICE_MANAGED_VERIFIER_GIT_INDEX_REJECTED",
            )?;
            if index != self.initial_index {
                return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_DRIFT"));
            }
            let control = git_control_digest(&self.git_directory, &self.common_git_directory)?;
            if control != self.initial_control_digest {
                return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_DRIFT"));
            }
        }
        Ok(())
    }

    /// The verifier executes trusted checks in a candidate worktree, so an
    /// ignored worker-created runner is still an execution input. Capture the
    /// full ignored-file state before dispatch and reject any later mutation.
    fn assert_ignored_state_unchanged(&self) -> ManagedPortResult<()> {
        if self.capture_ignored_state()? != self.initial_ignored_state {
            return Err(known("LATTICE_MANAGED_VERIFIER_IGNORED_STATE_DRIFT"));
        }
        Ok(())
    }

    fn capture_ignored_state(&self) -> ManagedPortResult<ContentDigest> {
        let listing = self.git_success(
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
                "--",
            ],
            None,
            None,
        )?;
        let mut paths = listing
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(path_text)
            .collect::<ManagedPortResult<Vec<_>>>()?;
        paths.sort();
        paths.dedup();
        if paths.len() > MAX_CHANGED_PATHS {
            return Err(known("LATTICE_MANAGED_VERIFIER_IGNORED_STATE_REJECTED"));
        }
        let mut hasher = Sha256::new();
        let mut total = 0u64;
        for path in paths {
            if forbidden_git_path(&path) {
                return Err(known("LATTICE_MANAGED_VERIFIER_IGNORED_STATE_REJECTED"));
            }
            let candidate = safe_repository_file(&self.repository, &path)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_IGNORED_STATE_REJECTED"))?;
            let remaining = u64::try_from(MAX_IGNORED_STATE_BYTES)
                .unwrap_or(u64::MAX)
                .checked_sub(total)
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_IGNORED_STATE_REJECTED"))?;
            let facts = capture_file_facts(
                &candidate,
                remaining,
                "LATTICE_MANAGED_VERIFIER_IGNORED_STATE_REJECTED",
            )?;
            total = total
                .checked_add(u64::try_from(path.len()).unwrap_or(u64::MAX))
                .and_then(|value| value.checked_add(facts.byte_len))
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_IGNORED_STATE_REJECTED"))?;
            if total > u64::try_from(MAX_IGNORED_STATE_BYTES).unwrap_or(u64::MAX) {
                return Err(known("LATTICE_MANAGED_VERIFIER_IGNORED_STATE_REJECTED"));
            }
            hasher.update(u64::try_from(path.len()).unwrap_or(u64::MAX).to_be_bytes());
            hasher.update(path.as_bytes());
            hasher.update(facts.byte_len.to_be_bytes());
            hasher.update(facts.content_digest.as_str().as_bytes());
            update_file_identity_hash(&mut hasher, &facts.file_identity);
        }
        digest_from_sha256(hasher.finalize().as_slice())
    }

    fn assert_trusted_rules_unchanged(&self) -> ManagedPortResult<()> {
        for rule in &self.trusted_rules {
            let path = self.repository_path(&rule.path);
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_RULE_DRIFT"))?;
            if !metadata.file_type().is_file()
                || metadata.file_type().is_symlink()
                || sha256_file(&path)? != rule.base_file_digest
            {
                return Err(known("LATTICE_MANAGED_VERIFIER_RULE_DRIFT"));
            }
        }
        Ok(())
    }

    fn capture_refs(&self) -> ManagedPortResult<Vec<u8>> {
        self.git_success(
            &["for-each-ref", "--format=%(refname)%00%(objectname)%00"],
            None,
            None,
        )
    }

    fn collect_changes(&self) -> ManagedPortResult<Vec<String>> {
        let changed = self.git_success(
            &[
                "diff",
                "--name-status",
                "-z",
                "--no-renames",
                &self.base_commit_oid,
                "--",
            ],
            None,
            None,
        )?;
        let chunks = changed.split(|byte| *byte == 0).collect::<Vec<_>>();
        let mut paths = BTreeSet::new();
        let mut index = 0;
        while index < chunks.len() && !chunks[index].is_empty() {
            let status = std::str::from_utf8(chunks[index])
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_DIFF_REJECTED"))?;
            let path = chunks
                .get(index + 1)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_DIFF_REJECTED"))?;
            if status == "D" {
                return Err(known("LATTICE_MANAGED_VERIFIER_DELETE_REJECTED"));
            }
            if !matches!(status, "A" | "M") {
                return Err(known("LATTICE_MANAGED_VERIFIER_DIFF_REJECTED"));
            }
            paths.insert(path_text(path)?);
            index += 2;
        }
        let untracked = self.git_success(
            &["ls-files", "--others", "--exclude-standard", "-z", "--"],
            None,
            None,
        )?;
        for path in untracked
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            paths.insert(path_text(path)?);
        }
        if paths.is_empty() || paths.len() > MAX_CHANGED_PATHS {
            return Err(known("LATTICE_MANAGED_VERIFIER_SCOPE_REJECTED"));
        }
        let paths = paths.into_iter().collect::<Vec<_>>();
        for path in &paths {
            if managed_protected_control_path(path) {
                return Err(known(
                    "LATTICE_MANAGED_VERIFIER_PROTECTED_PATH_CAPABILITY_REQUIRED",
                ));
            }
            if forbidden_git_path(path)
                || trusted_rule_path(path)
                || !self
                    .config
                    .allowed_paths
                    .iter()
                    .any(|rule| scope_rule_matches(rule, path))
            {
                return Err(known("LATTICE_MANAGED_VERIFIER_SCOPE_REJECTED"));
            }
            let metadata = fs::symlink_metadata(self.repository_path(path))
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_SCOPE_REJECTED"))?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(known("LATTICE_MANAGED_VERIFIER_SCOPE_REJECTED"));
            }
        }
        Ok(paths)
    }

    fn repository_path(&self, path: &str) -> PathBuf {
        path.split('/')
            .fold(self.repository.clone(), |root, part| root.join(part))
    }

    fn materialize_tree(&self, paths: &[String]) -> ManagedPortResult<String> {
        let index_path = self.control_directory.join(CANDIDATE_INDEX_FILE);
        match fs::symlink_metadata(&index_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) | Err(_) => {
                return Err(known("LATTICE_MANAGED_VERIFIER_GIT_INDEX_REJECTED"));
            }
        }
        let candidates = self.capture_secret_free_candidate_files(paths)?;
        let guard = TempIndex(index_path.clone());
        self.git_success(
            &["read-tree", &self.base_commit_oid],
            Some(&index_path),
            None,
        )?;
        for (path, bytes) in &candidates {
            let blob = oid_from_output(&self.git_success(
                &["hash-object", "-w", "--stdin"],
                None,
                Some(bytes),
            )?)?;
            let mode = self.base_mode(path)?;
            let cache = format!("{mode},{blob},{path}");
            self.git_success(
                &["update-index", "--add", "--cacheinfo", &cache],
                Some(&index_path),
                None,
            )?;
        }
        let tree = oid_from_output(&self.git_success(&["write-tree"], Some(&index_path), None)?)?;
        drop(guard);
        Ok(tree)
    }

    /// No candidate byte is admitted into Git's object database before this
    /// deterministic scan approves its path, raw content and pre-object diff.
    fn capture_secret_free_candidate_files(
        &self,
        paths: &[String],
    ) -> ManagedPortResult<Vec<(String, Vec<u8>)>> {
        let mut diff_args = vec![
            "diff".to_owned(),
            "--binary".to_owned(),
            "--no-ext-diff".to_owned(),
            "--no-textconv".to_owned(),
            self.base_commit_oid.clone(),
            "--".to_owned(),
        ];
        diff_args.extend(paths.iter().cloned());
        let diff_refs = diff_args.iter().map(String::as_str).collect::<Vec<_>>();
        let diff = self.git_success(&diff_refs, None, None)?;
        if contains_secret_material(&diff) {
            return Err(known("LATTICE_MANAGED_VERIFIER_SECRET_REJECTED"));
        }
        let mut aggregate_bytes = 0u64;
        let mut candidates = Vec::with_capacity(paths.len());
        for path in paths {
            if contains_secret_material(path.as_bytes()) {
                return Err(known("LATTICE_MANAGED_VERIFIER_SECRET_REJECTED"));
            }
            let candidate = safe_repository_file(&self.repository, path)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CANDIDATE_LIMIT"))?;
            let remaining = MAX_CANDIDATE_AGGREGATE_BYTES
                .checked_sub(aggregate_bytes)
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CANDIDATE_LIMIT"))?;
            let bytes = read_bounded_file(
                &candidate,
                MAX_CANDIDATE_FILE_BYTES.min(remaining),
                "LATTICE_MANAGED_VERIFIER_CANDIDATE_LIMIT",
            )?;
            aggregate_bytes = aggregate_bytes
                .checked_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CANDIDATE_LIMIT"))?;
            if contains_secret_material(&bytes) {
                return Err(known("LATTICE_MANAGED_VERIFIER_SECRET_REJECTED"));
            }
            candidates.push((path.clone(), bytes));
        }
        Ok(candidates)
    }

    fn base_mode(&self, path: &str) -> ManagedPortResult<&'static str> {
        let output =
            self.git_success(&["ls-tree", &self.base_commit_oid, "--", path], None, None)?;
        if output.is_empty() {
            return Ok("100644");
        }
        if output.starts_with(b"100644 ") {
            Ok("100644")
        } else if output.starts_with(b"100755 ") {
            Ok("100755")
        } else {
            Err(known("LATTICE_MANAGED_VERIFIER_FILE_MODE_REJECTED"))
        }
    }

    fn candidate_diff(&self, tree_oid: &str) -> ManagedPortResult<Vec<u8>> {
        self.git_success(
            &[
                "diff",
                "--binary",
                "--no-ext-diff",
                "--no-textconv",
                &self.base_commit_oid,
                tree_oid,
                "--",
            ],
            None,
            None,
        )
    }

    fn run_diff_check(&self, tree_oid: &str) -> ManagedPortResult<bool> {
        let result = self.git_status(
            &["diff", "--check", &self.base_commit_oid, tree_oid, "--"],
            None,
            None,
        )?;
        Ok(result.status.success())
    }

    fn run_trusted_checks(&self) -> ManagedPortResult<Vec<CheckResult>> {
        let cargo_target = self.control_directory.join("cargo-target");
        fs::create_dir(&cargo_target)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
        let cargo_target = cargo_target
            .to_str()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
        let mut results = Vec::with_capacity(self.trusted_checks.len());
        for check in &self.trusted_checks {
            if !self.trusted_controls_match(check) {
                results.push(CheckResult {
                    id: check.kind.id(),
                    passed: false,
                    wsl_receipt_json: None,
                });
                continue;
            }
            if self.assert_ambient_guard_unchanged(check.kind).is_err() {
                results.push(CheckResult {
                    id: check.kind.id(),
                    passed: false,
                    wsl_receipt_json: None,
                });
                continue;
            }
            if !self.trusted_executables_match() {
                return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"));
            }
            // Worker changes are allowed to exist, but no control file may
            // change between the last base comparison and the child opening
            // it. Capture deny-write/delete handles now, then replay the base
            // comparison while those exact handles are held.
            let _control_guard = self.capture_check_effect_guard(check)?;
            if !self.trusted_controls_match(check) {
                results.push(CheckResult {
                    id: check.kind.id(),
                    passed: false,
                    wsl_receipt_json: None,
                });
                continue;
            }
            let (passed, wsl_receipt_json) = if self.config.execution_environment.is_some() {
                let (passed, receipt) = self.run_wsl_verifier_check(check.kind)?;
                (passed, Some(receipt))
            } else {
                let passed = match check.kind {
                    TrustedCheckKind::NpmVerify => {
                        self.run_trusted_node_plan(check.node_plan.as_ref().ok_or_else(|| {
                            known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED")
                        })?)?
                    }
                    TrustedCheckKind::CargoTest => {
                        self.assert_cargo_source_snapshot_unchanged()?;
                        let rustc = self.trusted_executable_text("rustc-toolchain")?;
                        let rustdoc = self.trusted_executable_text("rustdoc-toolchain")?;
                        let cargo_config = self
                            .cargo_source_snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.config_path.to_str())
                            .ok_or_else(|| {
                                known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED")
                            })?;
                        self.sandboxed_process_status(
                            TrustedCheckKind::CargoTest,
                            self.trusted_executable_path("cargo-toolchain")?,
                            &["--config", cargo_config, "test", "--locked", "--offline"],
                            &[
                                ("CARGO_NET_OFFLINE", "true"),
                                ("CARGO_TARGET_DIR", cargo_target),
                                ("RUSTC", rustc),
                                ("RUSTDOC", rustdoc),
                            ],
                        )?
                        .success()
                    }
                };
                (passed, None)
            };
            results.push(CheckResult {
                id: check.kind.id(),
                passed,
                wsl_receipt_json,
            });
        }
        Ok(results)
    }

    fn run_wsl_verifier_check(&self, kind: TrustedCheckKind) -> ManagedPortResult<(bool, String)> {
        let descriptor = self
            .config
            .execution_environment
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REQUIRED"))?;
        let preflight = self
            .config
            .execution_preflight
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REQUIRED"))?;
        let bridge = self
            .config
            .wsl_verifier_bridge_path
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_BRIDGE_REQUIRED"))?;
        let node = self
            .node_executable
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_NPM_UNAVAILABLE"))?;
        let receipt: Value = serde_json::from_slice(preflight.bytes())
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
        let worktree_ref = receipt
            .get("worktree_ref")
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
        let (role, arguments) = match kind {
            TrustedCheckKind::NpmVerify => (
                "NODE",
                vec!["run", "verify", "--offline", "--no-audit", "--no-fund"],
            ),
            TrustedCheckKind::CargoTest => ("CARGO", vec!["test", "--locked", "--offline"]),
        };
        let expected_process_fence = wsl_regular_verifier_fence(
            descriptor,
            preflight.task_ref().as_str(),
            preflight.attempt(),
            worktree_ref,
            role,
            &arguments,
            &receipt,
        )?;
        let expected_command_digest = wsl_regular_verifier_command_digest(
            descriptor,
            &receipt,
            role,
            &arguments,
            &expected_process_fence,
        )?;
        let request = serde_json::json!({
            "schema": "lattice.wsl2-verifier-request/1.0",
            "environment": serde_json::from_str::<Value>(descriptor.as_json())
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REJECTED"))?,
            "preflight_receipt": receipt,
            "task_ref": preflight.task_ref().as_str(),
            "attempt": preflight.attempt(),
            "worktree_ref": worktree_ref,
            "role": role,
            "args": arguments,
        });
        let mut input = serde_json::to_vec(&request)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REQUEST_REJECTED"))?;
        if input.is_empty() || input.len() > 262_144 {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REQUEST_REJECTED"));
        }
        input.push(b'\n');
        let environment = ["SystemRoot", "WINDIR"]
            .into_iter()
            .filter_map(|key| env::var_os(key).map(|value| (OsString::from(key), value)))
            .collect::<Vec<_>>();
        self.verify_effect_guards()?;
        let result = run_process(
            node,
            &[bridge.as_os_str().to_owned()],
            &self.repository,
            &environment,
            Some(&input),
            self.config.command_timeout,
            &self.control_directory,
            true,
            true,
        )?;
        self.verify_effect_guards()?;
        if !result.status.success() || result.stdout.is_empty() || result.stdout.len() > 1_048_576 {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
        }
        let output = std::str::from_utf8(&result.stdout)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        if !output.ends_with('\n')
            || output.contains('\r')
            || output[..output.len() - 1].contains('\n')
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
        }
        let result_value: Value = serde_json::from_str(&output[..output.len() - 1])
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        if result_value.get("schema").and_then(Value::as_str)
            == Some("lattice.wsl2-verifier-transport-failure/1.0")
        {
            let credential_seal = receipt
                .get("credential_seal_digest")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
            validate_wsl_transport_failure(
                &result_value,
                descriptor,
                preflight.task_ref().as_str(),
                preflight.attempt(),
                worktree_ref,
                role,
                credential_seal,
                receipt
                    .pointer("/continuation/retry_of")
                    .and_then(Value::as_str),
                receipt
                    .pointer("/continuation/reconnect_of")
                    .and_then(Value::as_str),
                None,
                Some(expected_process_fence.as_str()),
                Some(expected_command_digest.as_str()),
            )?;
            return Ok((false, canonical_json_value(&result_value)?));
        }
        if !exact_json_keys(
            &result_value,
            &[
                "schema",
                "status",
                "outcome",
                "task_ref",
                "attempt",
                "worktree_ref",
                "role",
                "repository_head",
                "verifier_identity",
                "process_marker",
                "exit_receipt",
                "outer_cleanup",
                "outer_post_exit",
                "output",
                "provider_effect_count",
                "result_digest",
            ],
        ) {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
        }
        let result_object = result_value
            .as_object()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let verifier_identity = result_value
            .get("verifier_identity")
            .filter(|value| {
                exact_json_keys(
                    value,
                    &[
                        "schema",
                        "command_digest",
                        "execution_environment_ref",
                        "verification_toolchain_ref",
                        "credential_seal_digest",
                        "process_fence",
                        "linux_cwd",
                        "repository_head",
                        "provider_effect_count",
                    ],
                )
            })
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let marker = result_value
            .get("process_marker")
            .filter(|value| {
                exact_json_keys(
                    value,
                    &[
                        "schema",
                        "fence",
                        "unit",
                        "execution_environment_ref",
                        "credential_seal_digest",
                        "boot_id_digest",
                        "pid",
                        "process_start_ticks",
                        "process_group_id",
                        "cgroup_path",
                        "cgroup_version",
                        "delegated",
                        "attempt",
                        "retry_of",
                        "reconnect_of",
                    ],
                )
            })
            .and_then(Value::as_object)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let exit_receipt = result_value
            .get("exit_receipt")
            .filter(|value| {
                exact_json_keys(
                    value,
                    &[
                        "schema",
                        "fence",
                        "unit",
                        "execution_environment_ref",
                        "credential_seal_digest",
                        "cgroup_path",
                        "zero_descendants",
                        "credential_seal_intact",
                        "credential_watch_intact",
                        "keyring_daemon_sha256",
                        "keyring_library_manifest_digest",
                        "tool_input_identities",
                        "stdout_bytes",
                        "stderr_bytes",
                        "stdout_limit_bytes",
                        "stderr_limit_bytes",
                        "output_bound_exceeded",
                        "timeout_ms",
                        "timed_out",
                        "interrupted",
                        "stdin_bytes",
                        "stdin_sha256",
                        "stdin_complete",
                        "attempt",
                        "retry_of",
                        "reconnect_of",
                        "exit_code",
                        "exit_signal",
                    ],
                )
            })
            .and_then(Value::as_object)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let outcome = wsl_verifier_terminal_outcome(result_object, exit_receipt)?;
        let unit = marker
            .get("unit")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 255)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let process_fence = marker
            .get("fence")
            .and_then(Value::as_str)
            .filter(|value| plain_sha256(value))
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let cgroup_path = marker
            .get("cgroup_path")
            .and_then(Value::as_str)
            .filter(|value| value.starts_with('/') && value.len() <= 4_096)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let descriptor_value: Value = serde_json::from_str(descriptor.as_json())
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REJECTED"))?;
        let credential_seal = receipt
            .get("credential_seal_digest")
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
        let retry_of = receipt
            .pointer("/continuation/retry_of")
            .and_then(Value::as_str);
        let reconnect_of = receipt
            .pointer("/continuation/reconnect_of")
            .and_then(Value::as_str);
        let expected_unit = format!(
            "{}-{}-{}.service",
            descriptor_value
                .pointer("/process_fence/unit_prefix")
                .and_then(Value::as_str)
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?,
            role.to_ascii_lowercase(),
            &expected_process_fence[..12]
        );
        let owner_uid = descriptor_value
            .pointer("/verification_toolchain/owner_uid")
            .and_then(Value::as_u64)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let expected_cgroup_path = format!(
            "/user.slice/user-{owner_uid}.slice/user@{owner_uid}.service/app.slice/{expected_unit}"
        );
        validate_wsl_verifier_cleanup(
            result_object
                .get("outer_cleanup")
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?,
            &descriptor_value,
            outcome,
            unit,
            process_fence,
            u64::from(preflight.attempt()),
            retry_of,
            reconnect_of,
        )?;
        validate_wsl_verifier_outer_exit(
            result_object
                .get("outer_post_exit")
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?,
            outcome,
            unit,
            cgroup_path,
        )?;
        let output_evidence = result_object
            .get("output")
            .filter(|value| {
                exact_json_keys(
                    value,
                    &[
                        "stdout_observed_bytes",
                        "stderr_observed_bytes",
                        "stdout_sha256",
                        "stderr_sha256",
                    ],
                )
            })
            .and_then(Value::as_object)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let stdout_limit = exit_receipt
            .get("stdout_limit_bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let stderr_limit = exit_receipt
            .get("stderr_limit_bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let stdout_bytes = exit_receipt
            .get("stdout_bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let stderr_bytes = exit_receipt
            .get("stderr_bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let output_bound = exit_receipt
            .get("output_bound_exceeded")
            .and_then(Value::as_bool)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let output_counts_valid = if output_bound {
            stdout_bytes <= stdout_limit.saturating_add(1)
                && stderr_bytes <= stderr_limit.saturating_add(1)
                && (stdout_bytes == stdout_limit.saturating_add(1)
                    || stderr_bytes == stderr_limit.saturating_add(1))
                && output_evidence
                    .get("stdout_observed_bytes")
                    .and_then(Value::as_u64)
                    .is_some_and(|bytes| bytes <= stdout_limit)
                && output_evidence
                    .get("stderr_observed_bytes")
                    .and_then(Value::as_u64)
                    .is_some_and(|bytes| bytes <= stderr_limit)
        } else {
            stdout_bytes <= stdout_limit
                && stderr_bytes <= stderr_limit
                && output_evidence
                    .get("stdout_observed_bytes")
                    .and_then(Value::as_u64)
                    == Some(stdout_bytes)
                && output_evidence
                    .get("stderr_observed_bytes")
                    .and_then(Value::as_u64)
                    == Some(stderr_bytes)
        };
        if result_value.get("schema").and_then(Value::as_str)
            != Some("lattice.wsl2-verifier-result/1.0")
            || result_value.get("task_ref").and_then(Value::as_str)
                != Some(preflight.task_ref().as_str())
            || result_value.get("attempt").and_then(Value::as_u64)
                != Some(u64::from(preflight.attempt()))
            || result_value.get("worktree_ref").and_then(Value::as_str) != Some(worktree_ref)
            || result_value.get("role").and_then(Value::as_str) != Some(role)
            || result_value.get("repository_head").and_then(Value::as_str)
                != Some(descriptor.repository_head())
            || result_value
                .get("provider_effect_count")
                .and_then(Value::as_u64)
                != Some(0)
            || !output_counts_valid
            || output_evidence
                .get("stderr_observed_bytes")
                .and_then(Value::as_u64)
                .is_none_or(|bytes| bytes > 1_310_720)
            || output_evidence
                .get("stdout_sha256")
                .and_then(Value::as_str)
                .is_none_or(|digest| !plain_sha256(digest))
            || output_evidence
                .get("stderr_sha256")
                .and_then(Value::as_str)
                .is_none_or(|digest| !plain_sha256(digest))
            || verifier_identity.get("schema").and_then(Value::as_str)
                != Some("lattice.wsl2-verifier-launch/1.0")
            || verifier_identity
                .get("command_digest")
                .and_then(Value::as_str)
                != Some(expected_command_digest.as_str())
            || verifier_identity
                .get("execution_environment_ref")
                .and_then(Value::as_str)
                != Some(descriptor.environment_ref().as_str())
            || verifier_identity
                .get("verification_toolchain_ref")
                .and_then(Value::as_str)
                != Some(descriptor.verification_toolchain_identity_ref())
            || verifier_identity.get("linux_cwd").and_then(Value::as_str)
                != Some(descriptor.linux_repository_path())
            || verifier_identity
                .get("credential_seal_digest")
                .and_then(Value::as_str)
                != Some(credential_seal)
            || verifier_identity
                .get("process_fence")
                .and_then(Value::as_str)
                != Some(expected_process_fence.as_str())
            || verifier_identity
                .get("repository_head")
                .and_then(Value::as_str)
                != Some(descriptor.repository_head())
            || verifier_identity
                .get("provider_effect_count")
                .and_then(Value::as_u64)
                != Some(0)
            || marker.get("schema").and_then(Value::as_str)
                != Some("lattice.wsl2-process-fence/1.1")
            || process_fence != expected_process_fence
            || unit != expected_unit
            || cgroup_path != expected_cgroup_path
            || marker
                .get("execution_environment_ref")
                .and_then(Value::as_str)
                != Some(descriptor.environment_ref().as_str())
            || marker.get("credential_seal_digest").and_then(Value::as_str) != Some(credential_seal)
            || marker
                .get("boot_id_digest")
                .and_then(Value::as_str)
                .is_none_or(|value| !typed_sha256(value, "wsl-boot"))
            || marker
                .get("pid")
                .and_then(Value::as_u64)
                .is_none_or(|value| value == 0)
            || marker
                .get("process_start_ticks")
                .and_then(Value::as_str)
                .is_none_or(|value| {
                    value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
                })
            || marker
                .get("process_group_id")
                .and_then(Value::as_u64)
                .is_none_or(|value| value == 0)
            || marker.get("cgroup_version").and_then(Value::as_u64) != Some(2)
            || marker.get("delegated").and_then(Value::as_bool) != Some(false)
            || marker.get("attempt").and_then(Value::as_u64) != Some(u64::from(preflight.attempt()))
            || !continuation_marker_matches(marker.get("retry_of"), retry_of)
            || !continuation_marker_matches(marker.get("reconnect_of"), reconnect_of)
            || exit_receipt.get("schema").and_then(Value::as_str)
                != Some("lattice.wsl2-subtree-exit/1.2")
            || exit_receipt
                .get("zero_descendants")
                .and_then(Value::as_bool)
                != Some(true)
            || exit_receipt
                .get("credential_seal_intact")
                .and_then(Value::as_bool)
                != Some(true)
            || exit_receipt
                .get("credential_watch_intact")
                .and_then(Value::as_bool)
                != Some(true)
            || exit_receipt.get("fence").and_then(Value::as_str) != Some(process_fence)
            || exit_receipt.get("unit").and_then(Value::as_str) != Some(unit)
            || exit_receipt.get("cgroup_path").and_then(Value::as_str) != Some(cgroup_path)
            || exit_receipt
                .get("execution_environment_ref")
                .and_then(Value::as_str)
                != Some(descriptor.environment_ref().as_str())
            || exit_receipt
                .get("credential_seal_digest")
                .and_then(Value::as_str)
                != Some(credential_seal)
            || !wsl_regular_receipt_tool_inputs_match(&descriptor_value, exit_receipt, role)
            || exit_receipt
                .get("stdout_limit_bytes")
                .and_then(Value::as_u64)
                != receipt
                    .pointer("/bounds/stdout_limit_bytes")
                    .and_then(Value::as_u64)
            || exit_receipt
                .get("stderr_limit_bytes")
                .and_then(Value::as_u64)
                != receipt
                    .pointer("/bounds/stderr_limit_bytes")
                    .and_then(Value::as_u64)
            || exit_receipt.get("timeout_ms").and_then(Value::as_u64)
                != receipt
                    .pointer("/timeout/timeout_ms")
                    .and_then(Value::as_u64)
            || exit_receipt.get("stdin_bytes").and_then(Value::as_u64) != Some(0)
            || exit_receipt.get("stdin_sha256").and_then(Value::as_str)
                != Some(sha256_bytes(&[])?.as_str())
            || exit_receipt.get("stdin_complete").and_then(Value::as_bool) != Some(true)
            || exit_receipt.get("attempt").and_then(Value::as_u64)
                != Some(u64::from(preflight.attempt()))
            || !continuation_marker_matches(exit_receipt.get("retry_of"), retry_of)
            || !continuation_marker_matches(exit_receipt.get("reconnect_of"), reconnect_of)
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
        }
        let supplied_digest = result_value
            .get("result_digest")
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
        let mut subject = result_value.clone();
        subject
            .as_object_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?
            .remove("result_digest");
        let canonical_subject = canonical_json_value(&subject)?;
        let expected_digest = format!(
            "wsl2-verifier-result:sha256:{}",
            sha256_bytes(canonical_subject.as_bytes())?.as_str()
        );
        if supplied_digest != expected_digest {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
        }
        Ok((outcome == "PASS", canonical_json_value(&result_value)?))
    }

    fn run_trusted_node_plan(&self, plan: &TrustedNodePlan) -> ManagedPortResult<bool> {
        let node = self
            .node_executable
            .as_deref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_NPM_UNAVAILABLE"))?;
        if plan.invocations.is_empty() || plan.invocations.len() > MAX_TRUSTED_NPM_SCRIPTS {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
        for invocation in &plan.invocations {
            let arguments = invocation
                .arguments
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            if !self
                .sandboxed_process_status(TrustedCheckKind::NpmVerify, node, &arguments, &[])?
                .success()
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn trusted_controls_match(&self, check: &TrustedCheck) -> bool {
        !check.control_files.is_empty()
            && self.trusted_control_inventory_matches(check)
            && check.control_files.iter().all(|control| {
                self.capture_repository_file(&control.path, control.byte_len)
                    .ok()
                    .is_some_and(|current| {
                        current.canonical_path_digest == control.canonical_path_digest
                            && current.content_digest == control.base_file_digest
                            && current.byte_len == control.byte_len
                            && current.file_identity == control.file_identity
                    })
            })
    }

    fn trusted_executables_match(&self) -> bool {
        self.trusted_executables.iter().all(|trusted| {
            capture_trusted_executable_with_guards(
                trusted.role,
                &trusted.path,
                &[
                    self.config.effect_bundle_guard.as_ref(),
                    self.config.runtime_effect_bundle_guard.as_ref(),
                ],
            )
            .ok()
            .is_some_and(|(current, _)| current == *trusted)
        })
    }

    fn capture_check_effect_guard(
        &self,
        check: &TrustedCheck,
    ) -> ManagedPortResult<ManagedEffectBundleGuard> {
        ManagedEffectBundleGuard::capture_bounded(
            check
                .control_files
                .iter()
                .map(|control| (self.repository.join(&control.path), control.byte_len.max(1))),
            MAX_TRUSTED_CONTROL_FILES,
        )
        .map_err(|()| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))
    }

    fn verify_effect_guards(&self) -> ManagedPortResult<()> {
        if let Some(guard) = self.base_effect_guard.as_ref() {
            guard
                .verify()
                .map_err(|()| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"))?;
        }
        if let Some(guard) = self.config.effect_bundle_guard.as_ref() {
            guard
                .verify()
                .map_err(|()| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"))?;
        }
        if let Some(guard) = self.config.runtime_effect_bundle_guard.as_ref() {
            guard
                .verify()
                .map_err(|()| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"))?;
        }
        if let Some(guard) = self.active_toolchain_guard.as_ref() {
            guard
                .verify()
                .map_err(|()| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"))?;
        }
        if let Some(guard) = self.cargo_source_guard.as_ref() {
            guard
                .verify()
                .map_err(|()| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_DRIFT"))?;
        }
        Ok(())
    }

    fn assert_trusted_executable_role(&self, role: &str) -> ManagedPortResult<()> {
        let trusted = self
            .trusted_executables
            .iter()
            .find(|trusted| trusted.role == role)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"))?;
        if capture_trusted_executable(trusted.role, &trusted.path)? != *trusted {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"));
        }
        Ok(())
    }

    fn trusted_executable_text(&self, role: &str) -> ManagedPortResult<&str> {
        self.trusted_executable_path(role)?
            .to_str()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))
    }

    fn trusted_executable_path(&self, role: &str) -> ManagedPortResult<&Path> {
        self.trusted_executables
            .iter()
            .find(|trusted| trusted.role == role)
            .map(|trusted| trusted.path.as_path())
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))
    }

    fn assert_cargo_source_snapshot_unchanged(&self) -> ManagedPortResult<()> {
        let snapshot = self
            .cargo_source_snapshot
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
        let vendor = capture_bounded_tree(&snapshot.vendor.root.path)?;
        let config = capture_file_facts(
            &snapshot.config_path,
            MAX_CARGO_VENDOR_CONFIG_BYTES,
            "LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED",
        )?;
        if vendor != snapshot.vendor || config != snapshot.config {
            return Err(known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_DRIFT"));
        }
        self.cargo_source_guard
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?
            .verify()
            .map_err(|()| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_DRIFT"))?;
        Ok(())
    }

    fn assert_ambient_guard_unchanged(&self, kind: TrustedCheckKind) -> ManagedPortResult<()> {
        let expected = match kind {
            TrustedCheckKind::NpmVerify => self.npm_ancestor_guard.as_ref(),
            TrustedCheckKind::CargoTest => self.cargo_ancestor_guard.as_ref(),
        }
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_AMBIENT_CONTROL_DRIFT"))?;
        let current = capture_ancestor_absence_guard(
            &self.repository,
            kind,
            "LATTICE_MANAGED_VERIFIER_AMBIENT_CONTROL_DRIFT",
        )?;
        if current != *expected {
            return Err(known("LATTICE_MANAGED_VERIFIER_AMBIENT_CONTROL_DRIFT"));
        }
        Ok(())
    }

    fn trusted_control_inventory_matches(&self, check: &TrustedCheck) -> bool {
        let expected = check
            .control_files
            .iter()
            .map(|control| control.path.clone())
            .collect::<BTreeSet<_>>();
        self.current_repository_paths().ok().is_some_and(|paths| {
            paths
                .into_iter()
                .filter(|path| match check.kind {
                    TrustedCheckKind::NpmVerify => {
                        npm_static_control_path(path)
                            || npm_shadow_path(path)
                            || expected.contains(path)
                    }
                    TrustedCheckKind::CargoTest => cargo_control_path(path),
                })
                .collect::<BTreeSet<_>>()
                == expected
        })
    }

    fn current_repository_paths(&self) -> ManagedPortResult<BTreeSet<String>> {
        let mut paths = BTreeSet::new();
        for args in [
            [
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
            ]
            .as_slice(),
            [
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z",
                "--",
            ]
            .as_slice(),
        ] {
            let listing = self.git_success(args, None, None)?;
            for path in listing
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
            {
                paths.insert(path_text(path)?);
                if paths.len() > MAX_REPOSITORY_PATHS {
                    return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"));
                }
            }
        }
        Ok(paths)
    }

    fn create_commit(&self, tree_oid: &str) -> ManagedPortResult<String> {
        let result = self.git_status_with_env(
            &["commit-tree", tree_oid, "-p", &self.base_commit_oid],
            None,
            Some(COMMIT_MESSAGE),
            &[
                ("GIT_AUTHOR_NAME", "LATTICE Foreman"),
                ("GIT_AUTHOR_EMAIL", "lattice@invalid.local"),
                ("GIT_AUTHOR_DATE", &self.config.created_at),
                ("GIT_COMMITTER_NAME", "LATTICE Foreman"),
                ("GIT_COMMITTER_EMAIL", "lattice@invalid.local"),
                ("GIT_COMMITTER_DATE", &self.config.created_at),
            ],
        )?;
        if !result.status.success() {
            return Err(known("LATTICE_MANAGED_VERIFIER_COMMIT_OBJECT_FAILED"));
        }
        oid_from_output(&result.stdout)
    }

    fn verify_commit_object(
        &self,
        commit_oid: &str,
        tree_oid: &str,
        diff_digest: &ContentDigest,
    ) -> ManagedPortResult<()> {
        if trim_ascii(&self.git_success(&["cat-file", "-t", commit_oid], None, None)?)? != "commit"
            || oid_from_output(&self.git_success(
                &["rev-parse", "--verify", &format!("{commit_oid}^{{tree}}")],
                None,
                None,
            )?)? != tree_oid
            || oid_from_output(&self.git_success(
                &["rev-parse", "--verify", &format!("{commit_oid}^1")],
                None,
                None,
            )?)? != self.base_commit_oid
            || sha256_bytes(&self.candidate_diff(commit_oid)?)? != *diff_digest
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_COMMIT_OBJECT_REJECTED"));
        }
        Ok(())
    }

    fn verify_prepared_candidate(&self, candidate: &PreparedCandidate) -> ManagedPortResult<()> {
        self.assert_repository_control_unchanged()?;
        self.assert_trusted_rules_unchanged()?;
        self.assert_ignored_state_unchanged()?;
        let paths = self.collect_changes()?;
        if paths != candidate.changed_paths {
            return Err(known("LATTICE_MANAGED_VERIFIER_WORKTREE_DRIFT"));
        }
        let tree = self.materialize_tree(&paths)?;
        if tree != candidate.tree_oid
            || sha256_bytes(&self.candidate_diff(&tree)?)? != candidate.diff_digest
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_WORKTREE_DRIFT"));
        }
        self.verify_commit_object(
            &candidate.commit_oid,
            &candidate.tree_oid,
            &candidate.diff_digest,
        )?;
        self.assert_repository_control_unchanged()?;
        self.assert_trusted_rules_unchanged()
    }

    fn git_success(
        &self,
        args: &[&str],
        index: Option<&Path>,
        stdin: Option<&[u8]>,
    ) -> ManagedPortResult<Vec<u8>> {
        let result = self.git_status(args, index, stdin)?;
        if result.status.success() {
            Ok(result.stdout)
        } else {
            Err(known(match args.first().copied() {
                Some("rev-parse") => match args.get(1).copied() {
                    Some("--show-toplevel") => "LATTICE_MANAGED_VERIFIER_GIT_SHOW_TOPLEVEL_FAILED",
                    Some("--absolute-git-dir") => {
                        "LATTICE_MANAGED_VERIFIER_GIT_ABSOLUTE_GIT_DIR_FAILED"
                    }
                    Some("--verify") => "LATTICE_MANAGED_VERIFIER_GIT_REV_VERIFY_FAILED",
                    _ => "LATTICE_MANAGED_VERIFIER_GIT_REV_PARSE_FAILED",
                },
                Some("for-each-ref") => "LATTICE_MANAGED_VERIFIER_GIT_REFS_FAILED",
                Some("ls-tree") => "LATTICE_MANAGED_VERIFIER_GIT_LS_TREE_FAILED",
                Some("show") => "LATTICE_MANAGED_VERIFIER_GIT_SHOW_FAILED",
                Some("diff") => "LATTICE_MANAGED_VERIFIER_GIT_DIFF_FAILED",
                Some("ls-files") => "LATTICE_MANAGED_VERIFIER_GIT_LS_FILES_FAILED",
                Some("read-tree") => "LATTICE_MANAGED_VERIFIER_GIT_READ_TREE_FAILED",
                Some("hash-object") => "LATTICE_MANAGED_VERIFIER_GIT_HASH_OBJECT_FAILED",
                Some("update-index") => "LATTICE_MANAGED_VERIFIER_GIT_UPDATE_INDEX_FAILED",
                Some("write-tree") => "LATTICE_MANAGED_VERIFIER_GIT_WRITE_TREE_FAILED",
                Some("cat-file") => "LATTICE_MANAGED_VERIFIER_GIT_CAT_FILE_FAILED",
                _ => "LATTICE_MANAGED_VERIFIER_GIT_FAILED",
            }))
        }
    }

    fn git_status(
        &self,
        args: &[&str],
        index: Option<&Path>,
        stdin: Option<&[u8]>,
    ) -> ManagedPortResult<ProcessResult> {
        self.git_status_with_env(args, index, stdin, &[])
    }

    fn git_status_with_env(
        &self,
        args: &[&str],
        index: Option<&Path>,
        stdin: Option<&[u8]>,
        environment: &[(&str, &str)],
    ) -> ManagedPortResult<ProcessResult> {
        self.assert_trusted_executable_role("git")?;
        self.verify_effect_guards()?;
        self.assert_git_pre_spawn()?;
        #[cfg(test)]
        {
            let action = GIT_PRE_SPAWN_FAILPOINT
                .lock()
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_GIT_LAYOUT_DRIFT"))?
                .take_if(|(repository, _)| same_path(repository, &self.repository))
                .map(|(_, action)| action);
            if let Some(action) = action {
                if action() {
                    return Err(known("LATTICE_MANAGED_VERIFIER_GIT_LAYOUT_DRIFT"));
                }
            }
        }
        // The second identity check is intentionally adjacent to process
        // creation. It closes deterministic check/use substitutions; an
        // unprivileged same-user swap after this point remains a narrow OS
        // scheduling race, while explicit Git stores still bind any effect.
        self.assert_git_pre_spawn()?;
        let git_home = self.control_directory.join("git-home");
        let git_temp = self.control_directory.join("git-temp");
        let mut directory_anchors = Vec::with_capacity(2);
        for directory in [&git_home, &git_temp] {
            fs::create_dir_all(directory)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
            directory_anchors.push(capture_path_anchor(
                directory,
                "LATTICE_MANAGED_VERIFIER_CONTROL_FAILED",
            )?);
        }
        let mut fixed = vec![
            OsString::from("--no-pager"),
            OsString::from("--no-replace-objects"),
            OsString::from("--literal-pathspecs"),
            OsString::from("-c"),
            OsString::from(format!(
                "core.hooksPath={}",
                self.hooks_directory.to_string_lossy()
            )),
            OsString::from("-c"),
            OsString::from("core.fsmonitor=false"),
            OsString::from("-c"),
            OsString::from("protocol.allow=never"),
            OsString::from("-c"),
            OsString::from("commit.gpgSign=false"),
        ];
        fixed.extend(args.iter().map(OsString::from));
        let default_index = self
            .git_guard_ready
            .then(|| self.git_directory.join("index"));
        let effective_index = index.or(default_index.as_deref());
        let mut env = safe_git_environment(
            &git_home,
            &git_temp,
            &self.fixed_path,
            &self.global_config,
            self.git_guard_ready.then_some(self.repository.as_path()),
            self.git_guard_ready.then_some(self.git_directory.as_path()),
            self.git_guard_ready
                .then_some(self.common_git_directory.as_path()),
            self.git_guard_ready
                .then_some(self.object_directory.as_path()),
            effective_index,
        );
        env.extend(
            environment
                .iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value))),
        );
        let result = if self.config.execution_environment.is_some() {
            self.run_wsl_git_verifier(&fixed, &env, stdin)?
        } else {
            let clear_environment = true;
            run_process(
                &self.git_executable,
                &fixed,
                &self.repository,
                &env,
                stdin,
                self.config.command_timeout,
                &self.control_directory,
                true,
                clear_environment,
            )?
        };
        for anchor in directory_anchors {
            if capture_path_anchor(&anchor.path, "LATTICE_MANAGED_VERIFIER_CONTROL_FAILED")?
                != anchor
            {
                return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"));
            }
        }
        self.assert_git_pre_spawn()?;
        self.verify_effect_guards()?;
        Ok(result)
    }

    #[allow(clippy::too_many_lines)]
    fn run_wsl_git_verifier(
        &self,
        args: &[OsString],
        environment: &[(OsString, OsString)],
        stdin: Option<&[u8]>,
    ) -> ManagedPortResult<ProcessResult> {
        let descriptor = self
            .config
            .execution_environment
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REQUIRED"))?;
        let preflight = self
            .config
            .execution_preflight
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REQUIRED"))?;
        let bridge = self
            .config
            .wsl_verifier_bridge_path
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_BRIDGE_REQUIRED"))?;
        let node = self
            .node_executable
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_NPM_UNAVAILABLE"))?;
        let receipt: Value = serde_json::from_slice(preflight.bytes())
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
        let context = validate_wsl_git_preflight(descriptor, preflight, &receipt)?;
        let mapped_args = wsl_git_arguments(args, descriptor.distribution())?;
        let mapped_environment = wsl_git_environment(environment, descriptor.distribution())?;
        let sequence = self
            .wsl_git_sequence
            .fetch_add(1, Ordering::AcqRel)
            .checked_add(1)
            .filter(|value| *value <= MAX_WSL_GIT_INVOCATIONS)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"))?;
        let stdin_value = stdin.map_or_else(
            || Ok(Value::Null),
            |bytes| {
                if bytes.len() > usize::try_from(MAX_CANDIDATE_FILE_BYTES).unwrap_or(usize::MAX) {
                    return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REQUEST_REJECTED"));
                }
                let digest = sha256_bytes(bytes)?;
                Ok(serde_json::json!({
                    "byte_len": bytes.len(),
                    "sha256": digest.as_str(),
                    "base64": base64_encode(bytes),
                }))
            },
        )?;
        let invocation_subject = serde_json::json!({
            "schema": "lattice.wsl2-git-invocation/1.0",
            "sequence": sequence,
            "environment": mapped_environment.clone(),
            "args": mapped_args.clone(),
            "stdin": stdin_value,
        });
        let invocation_digest = typed_json_sha256("wsl2-git-invocation", &invocation_subject)?;
        let process_fence =
            wsl_git_process_fence(&context.preflight_fence, &invocation_digest, sequence)?;
        let descriptor_value = serde_json::from_str::<Value>(descriptor.as_json())
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REJECTED"))?;
        let command_digest = wsl_git_command_digest(
            &descriptor_value,
            descriptor.environment_ref().as_str(),
            descriptor.linux_repository_path(),
            &context,
            u64::from(preflight.attempt()),
            &mapped_args,
            &mapped_environment,
            &invocation_digest,
            &process_fence,
        )?;
        let mut git_invocation = invocation_subject;
        let invocation = git_invocation
            .as_object_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"))?;
        invocation.insert(
            "invocation_digest".to_owned(),
            Value::String(invocation_digest.clone()),
        );
        invocation.insert(
            "process_fence".to_owned(),
            Value::String(process_fence.clone()),
        );
        let request = serde_json::json!({
            "schema": "lattice.wsl2-verifier-request/1.0",
            "environment": descriptor_value,
            "preflight_receipt": receipt,
            "task_ref": preflight.task_ref().as_str(),
            "attempt": preflight.attempt(),
            "worktree_ref": context.worktree_ref.as_str(),
            "role": "GIT",
            "args": mapped_args,
            "git_invocation": git_invocation,
        });
        let mut input = serde_json::to_vec(&request)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REQUEST_REJECTED"))?;
        if input.is_empty() || input.len() > MAX_WSL_GIT_REQUEST_BYTES {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REQUEST_REJECTED"));
        }
        input.push(b'\n');
        let bridge_environment = ["SystemRoot", "WINDIR"]
            .into_iter()
            .filter_map(|key| env::var_os(key).map(|value| (OsString::from(key), value)))
            .collect::<Vec<_>>();
        self.verify_effect_guards()?;
        let bridge_result = run_process(
            node,
            &[bridge.as_os_str().to_owned()],
            &self.repository,
            &bridge_environment,
            Some(&input),
            self.config.command_timeout,
            &self.control_directory,
            true,
            true,
        )?;
        self.verify_effect_guards()?;
        if !bridge_result.status.success()
            || bridge_result.stdout.is_empty()
            || bridge_result.stdout.len() > MAX_WSL_GIT_RESULT_BYTES
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
        }
        let result_value = single_line_json(&bridge_result.stdout)?;
        let process_result = validate_wsl_git_result(
            descriptor,
            preflight,
            &context,
            sequence,
            &invocation_digest,
            &process_fence,
            &command_digest,
            stdin,
            &result_value,
        )?;
        let mut receipts = self
            .wsl_git_receipts
            .lock()
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"))?;
        if receipts.len() != usize::try_from(sequence - 1).unwrap_or(usize::MAX) {
            return Err(known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"));
        }
        receipts.push(WslGitReceiptRecord {
            sequence,
            invocation_digest,
            result: compact_wsl_git_result(&result_value)?,
        });
        Ok(process_result)
    }

    fn wsl_git_receipt_bundle(&self) -> ManagedPortResult<Option<String>> {
        let Some(descriptor) = self.config.execution_environment.as_ref() else {
            return Ok(None);
        };
        let records = self
            .wsl_git_receipts
            .lock()
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"))?
            .clone();
        let observed = self.wsl_git_sequence.load(Ordering::Acquire);
        let invocation_digests = records
            .iter()
            .map(|record| record.invocation_digest.as_str())
            .collect::<BTreeSet<_>>();
        let process_fences = records
            .iter()
            .filter_map(|record| {
                record
                    .result
                    .pointer("/process_marker/fence")
                    .and_then(Value::as_str)
                    .or_else(|| record.result.get("process_fence").and_then(Value::as_str))
            })
            .collect::<BTreeSet<_>>();
        let command_digests = records
            .iter()
            .filter_map(|record| {
                record
                    .result
                    .pointer("/verifier_identity/command_digest")
                    .and_then(Value::as_str)
            })
            .collect::<BTreeSet<_>>();
        if records.is_empty()
            || observed != u64::try_from(records.len()).unwrap_or(u64::MAX)
            || invocation_digests.len() != records.len()
            || process_fences.len() != records.len()
            || command_digests.len() != records.len()
            || records
                .iter()
                .enumerate()
                .any(|(index, record)| record.sequence != u64::try_from(index + 1).unwrap_or(0))
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"));
        }
        let subject = serde_json::json!({
            "schema": "lattice.wsl2-git-receipt-bundle/1.0",
            "execution_environment_ref": descriptor.environment_ref().as_str(),
            "repository_head": descriptor.repository_head(),
            "operation_count": records.len(),
            "records": records.into_iter().map(|record| serde_json::json!({
                "sequence": record.sequence,
                "invocation_digest": record.invocation_digest,
                "result": record.result,
            })).collect::<Vec<_>>(),
        });
        let bundle_digest = typed_json_sha256("wsl2-git-receipt-bundle", &subject)?;
        let mut bundle = subject;
        bundle
            .as_object_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"))?
            .insert("bundle_digest".to_owned(), Value::String(bundle_digest));
        let bundle = canonical_json_value(&bundle)?;
        if bundle.len() > MAX_WSL_GIT_RECEIPT_BUNDLE_BYTES {
            return Err(known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"));
        }
        Ok(Some(bundle))
    }

    fn wsl_git_transport_failure_evidence(
        &self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
        failure: &ManagedPortError,
    ) -> ManagedPortResult<Option<VerifiedManagedEvidence>> {
        let Some(descriptor) = self.config.execution_environment.as_ref() else {
            return Ok(None);
        };
        let Some(preflight) = self.config.execution_preflight.as_ref() else {
            return Err(known(
                "LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED",
            ));
        };
        let final_receipt_is_transport_failure = self
            .wsl_git_receipts
            .lock()
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"))?
            .last()
            .is_some_and(|record| {
                record.result.get("status").and_then(Value::as_str) == Some("FAILED")
                    && record.result.get("outcome").and_then(Value::as_str)
                        == Some("TRANSPORT_ERROR")
                    && record
                        .result
                        .get("provider_effect_count")
                        .and_then(Value::as_u64)
                        == Some(0)
            });
        if !final_receipt_is_transport_failure {
            return Ok(None);
        }
        let bundle_json = self
            .wsl_git_receipt_bundle()?
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"))?;
        let bundle: Value = serde_json::from_str(&bundle_json)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"))?;
        let subject = serde_json::json!({
            "schema": WSL_GIT_TRANSPORT_FAILURE_SCHEMA,
            "task_ref": binding.task_ref().as_str(),
            "attempt": attempt.attempt_number(),
            "binding_digest": binding.binding_digest().as_str(),
            "attempt_payload_digest": attempt.payload_digest().as_str(),
            "terminal_payload_digest": terminal.payload_digest().as_str(),
            "failure_code": failure.code(),
            "execution_environment_ref": descriptor.environment_ref().as_str(),
            "execution_environment_descriptor_digest": descriptor.descriptor_digest().as_str(),
            "execution_preflight_descriptor_digest": preflight.descriptor_digest().as_str(),
            "provider_effect_count": 0,
            "receipt_bundle": bundle,
        });
        let bytes = canonical_json_value(&subject)?.into_bytes();
        let input = ManagedEvidenceInput::new(
            self.config.project_id.clone(),
            binding.task_ref().clone(),
            u8::try_from(attempt.attempt_number())
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EVIDENCE_FAILED"))?,
            ManagedEvidenceKind::VerificationResult,
            "application/json",
            WSL_GIT_TRANSPORT_FAILURE_SCHEMA,
            PRODUCER_ID,
            PRODUCER_VERSION,
            sha256_bytes(b"lattice-runtime-managed-verifier/1.0")?,
            self.config.created_at.clone(),
            bytes,
        )
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EVIDENCE_FAILED"))?;
        VerifiedManagedEvidence::new(input)
            .map(Some)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EVIDENCE_FAILED"))
    }

    fn sandboxed_process_status(
        &self,
        kind: TrustedCheckKind,
        executable: &Path,
        args: &[&str],
        environment: &[(&str, &str)],
    ) -> ManagedPortResult<ExitStatus> {
        self.verify_effect_guards()?;
        if !self.trusted_executables_match() {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT"));
        }
        let sandbox = self
            .sandbox_executable
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_SANDBOX_UNAVAILABLE"))?;
        let codex_home = self.control_directory.join("codex-home");
        let process_temp = self.control_directory.join("process-temp");
        let process_home = self.control_directory.join("process-home");
        let cargo_home = self.control_directory.join("cargo-home");
        for directory in [&codex_home, &process_temp, &process_home, &cargo_home] {
            fs::create_dir_all(directory)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
            let metadata = fs::symlink_metadata(directory)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"));
            }
        }
        let repository = self
            .repository
            .to_str()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PATH_REJECTED"))?;
        let mut fixed = vec![
            OsString::from("sandbox"),
            OsString::from("-P"),
            OsString::from(":workspace"),
            OsString::from("--sandbox-state-disable-network"),
            OsString::from("-C"),
            OsString::from(repository),
            OsString::from("--"),
            executable.as_os_str().to_owned(),
        ];
        fixed.extend(args.iter().map(OsString::from));
        let mut safe_environment = safe_process_environment(
            &codex_home,
            &process_temp,
            &process_home,
            &cargo_home,
            &self.fixed_path,
        );
        safe_environment.extend(
            environment
                .iter()
                .map(|(key, value)| (OsString::from(key), OsString::from(value))),
        );
        self.assert_ambient_guard_unchanged(kind)?;
        let result = run_process(
            sandbox,
            &fixed,
            &self.repository,
            &safe_environment,
            None,
            self.config.command_timeout,
            &self.control_directory,
            false,
            true,
        )?;
        self.assert_ambient_guard_unchanged(kind)?;
        self.verify_effect_guards()?;
        Ok(result.status)
    }
}

impl ManagedVerificationPort for ManagedVerificationAdapter {
    fn prepare(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
    ) -> ManagedPortResult<ManagedVerificationPreparation> {
        self.validate_records(binding, attempt, terminal)?;
        if let Some(candidate) = &self.prepared {
            if candidate.binding_digest != *binding.binding_digest()
                || candidate.attempt_digest != *attempt.payload_digest()
                || candidate.terminal_digest != *terminal.payload_digest()
            {
                return Err(known("LATTICE_MANAGED_VERIFIER_PREPARATION_SUBSTITUTED"));
            }
            self.verify_prepared_candidate(candidate)?;
            return Ok(candidate.preparation.clone());
        }
        self.assert_repository_control_unchanged()?;
        self.assert_trusted_rules_unchanged()?;
        self.assert_ignored_state_unchanged()?;
        let changed_paths = self.collect_changes()?;
        let initial_tree = self.materialize_tree(&changed_paths)?;
        let diff_check = self.run_diff_check(&initial_tree)?;
        let mut checks = vec![CheckResult {
            id: "git-diff-check-v1",
            passed: diff_check,
            wsl_receipt_json: None,
        }];
        if self.trusted_checks.is_empty() {
            checks.push(CheckResult {
                id: "trusted-project-test-required-v1",
                passed: false,
                wsl_receipt_json: None,
            });
        } else {
            checks.extend(self.run_trusted_checks()?);
        }

        self.assert_repository_control_unchanged()?;
        self.assert_trusted_rules_unchanged()?;
        self.assert_ignored_state_unchanged()?;
        let after_checks = self.collect_changes()?;
        if after_checks != changed_paths {
            return Err(known("LATTICE_MANAGED_VERIFIER_CHECK_MUTATED_WORKTREE"));
        }
        let tree_oid = self.materialize_tree(&changed_paths)?;
        if tree_oid != initial_tree {
            return Err(known("LATTICE_MANAGED_VERIFIER_CHECK_MUTATED_WORKTREE"));
        }
        let diff = self.candidate_diff(&tree_oid)?;
        let diff_digest = sha256_bytes(&diff)?;
        let commit_oid = self.create_commit(&tree_oid)?;
        self.verify_commit_object(&commit_oid, &tree_oid, &diff_digest)?;
        self.assert_repository_control_unchanged()?;
        self.assert_ignored_state_unchanged()?;
        checks[0].wsl_receipt_json = self.wsl_git_receipt_bundle()?;

        let evidence = git_snapshot_evidence(
            &self.config,
            attempt,
            &self.base_commit_oid,
            &commit_oid,
            &tree_oid,
            &diff_digest,
            &changed_paths,
            &checks,
            &self.command_identity,
        )?;
        let request = ManagedVerificationRequest::new(
            binding.verification_policy_digest().clone(),
            self.command_identity.clone(),
            self.base_commit_digest.clone(),
            sha256_bytes(commit_oid.as_bytes())?,
            sha256_bytes(tree_oid.as_bytes())?,
            diff_digest.clone(),
            terminal.evidence_digest().clone(),
            &evidence,
        )?;
        let mechanical_outcome = if checks.iter().all(|check| check.passed) {
            VerificationOutcome::Passed
        } else {
            VerificationOutcome::Failed
        };
        let preparation = ManagedVerificationPreparation::new(binding, attempt, evidence, request)?
            .with_mechanical_outcome(mechanical_outcome);
        let candidate = PreparedCandidate {
            preparation: preparation.clone(),
            binding_digest: binding.binding_digest().clone(),
            attempt_digest: attempt.payload_digest().clone(),
            terminal_digest: terminal.payload_digest().clone(),
            commit_oid,
            tree_oid,
            diff_digest,
            changed_paths,
            checks,
            semantic_review: None,
        };
        self.prepared = Some(candidate);
        Ok(preparation)
    }

    fn preparation_failure_evidence(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
        failure: &ManagedPortError,
    ) -> ManagedPortResult<Option<VerifiedManagedEvidence>> {
        self.validate_records(binding, attempt, terminal)?;
        self.wsl_git_transport_failure_evidence(binding, attempt, terminal, failure)
    }

    fn review(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
        request: &ManagedVerificationRequest,
        sink: &mut dyn ManagedReviewEvidenceSink,
    ) -> ManagedPortResult<()> {
        self.validate_records(binding, attempt, terminal)?;
        let candidate = self
            .prepared
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PREPARATION_REQUIRED"))?;
        if candidate.binding_digest != *binding.binding_digest()
            || candidate.attempt_digest != *attempt.payload_digest()
            || candidate.terminal_digest != *terminal.payload_digest()
            || candidate.preparation.request() != request
            || candidate.preparation.mechanical_outcome() != VerificationOutcome::Passed
            || !candidate.checks.iter().all(|check| check.passed)
        {
            return Err(known("LATTICE_MANAGED_REVIEW_STAGE_REJECTED"));
        }
        if candidate.semantic_review.is_some() {
            return Ok(());
        }
        let subject = ManagedSemanticReviewSubject::new(
            binding.project_authority_receipt_digest().clone(),
            binding.task_ref().clone(),
            u8::try_from(attempt.attempt_number())
                .map_err(|_| known("LATTICE_MANAGED_REVIEW_SUBJECT_REJECTED"))?,
            binding.task_spec_digest().clone(),
            binding.verification_policy_digest().clone(),
            self.base_commit_oid.clone(),
            candidate.commit_oid.clone(),
            candidate.tree_oid.clone(),
            candidate.diff_digest.clone(),
            candidate.changed_paths.clone(),
        )?;
        let reviewer = self
            .semantic_reviewer
            .as_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_REVIEWER_REQUIRED"))?;
        let result = reviewer.review(&subject, sink)?;
        if result.subject_digest() != subject.subject_digest()
            || result.review_evidence().project_id() != &self.config.project_id
            || result.review_evidence().task_ref() != binding.task_ref()
            || u64::from(result.review_evidence().attempt()) != attempt.attempt_number()
            || result.review_evidence().kind() != ManagedEvidenceKind::ReviewResult
            || result.resource_evidence().project_id() != &self.config.project_id
            || result.resource_evidence().task_ref() != binding.task_ref()
            || u64::from(result.resource_evidence().attempt()) != attempt.attempt_number()
            || result.resource_evidence().kind() != ManagedEvidenceKind::ResourceObservation
        {
            return Err(known("LATTICE_MANAGED_REVIEW_EVIDENCE_REJECTED"));
        }
        for evidence in result.supplemental_evidence() {
            let receipt = sink.record(&evidence)?;
            if !receipt.matches(&evidence) {
                return Err(known("LATTICE_MANAGED_REVIEW_EVIDENCE_REJECTED"));
            }
        }
        self.prepared
            .as_mut()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PREPARATION_REQUIRED"))?
            .semantic_review = Some(result);
        Ok(())
    }

    fn verify(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
        request: &ManagedVerificationRequest,
    ) -> ManagedPortResult<ManagedVerificationEvidence> {
        self.validate_records(binding, attempt, terminal)?;
        let candidate = self
            .prepared
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PREPARATION_REQUIRED"))?;
        if candidate.binding_digest != *binding.binding_digest()
            || candidate.attempt_digest != *attempt.payload_digest()
            || candidate.terminal_digest != *terminal.payload_digest()
            || candidate.preparation.request() != request
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_REQUEST_SUBSTITUTED"));
        }
        self.verify_prepared_candidate(candidate)?;
        let review_digest = candidate
            .semantic_review
            .as_ref()
            .map(|review| review.review_digest().clone());
        let outcome = if candidate.checks.iter().all(|check| check.passed)
            && candidate
                .semantic_review
                .as_ref()
                .is_some_and(|review| review.verdict().passed())
        {
            VerificationOutcome::Passed
        } else {
            VerificationOutcome::Failed
        };
        let result_digest = verification_result_digest(
            request,
            outcome,
            &candidate.checks,
            review_digest.as_ref(),
        )?;
        ManagedVerificationEvidence::new(request.clone(), outcome, result_digest, review_digest)
    }
}

impl Drop for ManagedVerificationAdapter {
    fn drop(&mut self) {
        // The source guard deliberately holds deny-write/delete handles inside
        // the verifier-owned control directory. Release those handles before
        // deleting the owned snapshot; all other guards protect external files
        // and remain live until field drop completes.
        self.cargo_source_guard.take();
        remove_owned_control_directory(&self.control_directory);
    }
}

struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
        let _ = fs::remove_file(self.0.with_extension("lock"));
    }
}

#[derive(Debug)]
struct ProcessResult {
    status: ExitStatus,
    stdout: Vec<u8>,
}

enum ProcessOutputRead {
    Captured(Vec<u8>),
    Drained,
    Limit,
    Failed,
}

enum ProcessInputWrite {
    Written,
    Failed,
}

static ACTIVE_PROCESS_IO_THREADS: AtomicU64 = AtomicU64::new(0);

struct ActiveProcessIoThread;

impl ActiveProcessIoThread {
    fn enter() -> Self {
        ACTIVE_PROCESS_IO_THREADS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ActiveProcessIoThread {
    fn drop(&mut self) {
        ACTIVE_PROCESS_IO_THREADS.fetch_sub(1, Ordering::AcqRel);
    }
}

#[allow(clippy::too_many_arguments)]
fn safe_git_environment(
    process_home: &Path,
    process_temp: &Path,
    fixed_path: &OsString,
    global_config: &Path,
    work_tree: Option<&Path>,
    git_directory: Option<&Path>,
    common_git_directory: Option<&Path>,
    object_directory: Option<&Path>,
    index: Option<&Path>,
) -> Vec<(OsString, OsString)> {
    let mut environment = vec![
        (OsString::from("HOME"), git_process_path(process_home)),
        (
            OsString::from("USERPROFILE"),
            git_process_path(process_home),
        ),
        (OsString::from("TEMP"), git_process_path(process_temp)),
        (OsString::from("TMP"), git_process_path(process_temp)),
        (OsString::from("PATH"), fixed_path.clone()),
        (OsString::from("NO_COLOR"), OsString::from("1")),
        (OsString::from("CI"), OsString::from("1")),
        (OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1")),
        (
            OsString::from("GIT_CONFIG_GLOBAL"),
            git_process_path(global_config),
        ),
        (OsString::from("GIT_CONFIG_COUNT"), OsString::from("0")),
        (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
        (OsString::from("GIT_OPTIONAL_LOCKS"), OsString::from("0")),
        (OsString::from("GIT_ATTR_NOSYSTEM"), OsString::from("1")),
    ];
    for key in [
        "SystemRoot",
        "WINDIR",
        "PROCESSOR_ARCHITECTURE",
        "NUMBER_OF_PROCESSORS",
    ] {
        if let Some(value) = env::var_os(key) {
            environment.push((OsString::from(key), value));
        }
    }
    #[cfg(windows)]
    environment.push((
        OsString::from("PATHEXT"),
        OsString::from(".COM;.EXE;.BAT;.CMD"),
    ));
    for (key, value) in [
        ("GIT_WORK_TREE", work_tree),
        ("GIT_DIR", git_directory),
        ("GIT_COMMON_DIR", common_git_directory),
        ("GIT_OBJECT_DIRECTORY", object_directory),
        ("GIT_INDEX_FILE", index),
    ] {
        if let Some(value) = value {
            environment.push((OsString::from(key), git_process_path(value)));
        }
    }
    environment
}

#[cfg(windows)]
fn git_process_path(path: &Path) -> OsString {
    let text = path.as_os_str().to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return OsString::from(format!(r"\\{unc}"));
    }
    OsString::from(text.strip_prefix(r"\\?\").unwrap_or(&text))
}

#[cfg(not(windows))]
fn git_process_path(path: &Path) -> OsString {
    path.as_os_str().to_owned()
}

fn safe_process_environment(
    codex_home: &Path,
    process_temp: &Path,
    process_home: &Path,
    cargo_home: &Path,
    fixed_path: &OsString,
) -> Vec<(OsString, OsString)> {
    let mut environment = vec![
        (
            OsString::from("CODEX_HOME"),
            codex_home.as_os_str().to_owned(),
        ),
        (OsString::from("TEMP"), process_temp.as_os_str().to_owned()),
        (OsString::from("TMP"), process_temp.as_os_str().to_owned()),
        (OsString::from("NO_COLOR"), OsString::from("1")),
        (OsString::from("CI"), OsString::from("1")),
        (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
        (
            OsString::from("USERPROFILE"),
            process_home.as_os_str().to_owned(),
        ),
        (OsString::from("HOME"), process_home.as_os_str().to_owned()),
        (
            OsString::from("CARGO_HOME"),
            cargo_home.as_os_str().to_owned(),
        ),
        (OsString::from("PATH"), fixed_path.clone()),
    ];
    // Only process/runtime discovery values cross the boundary. Arbitrary
    // parent variables (database passwords, API tokens, credentials) do not.
    for key in [
        "SystemRoot",
        "WINDIR",
        "ProgramData",
        "PROCESSOR_ARCHITECTURE",
        "NUMBER_OF_PROCESSORS",
    ] {
        if let Some(value) = env::var_os(key) {
            environment.push((OsString::from(key), value));
        }
    }
    #[cfg(windows)]
    environment.push((
        OsString::from("PATHEXT"),
        OsString::from(".COM;.EXE;.BAT;.CMD"),
    ));
    environment
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_process(
    executable: &Path,
    args: &[OsString],
    current_directory: &Path,
    environment: &[(OsString, OsString)],
    stdin: Option<&[u8]>,
    timeout: Duration,
    _control_directory: &Path,
    capture_stdout: bool,
    clear_environment: bool,
) -> ManagedPortResult<ProcessResult> {
    let mut command = Command::new(executable);
    command.args(args).current_dir(current_directory);
    if clear_environment {
        command.env_clear();
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_KEY_0",
        "GIT_CONFIG_VALUE_0",
        "GIT_CONFIG_PARAMETERS",
        "GIT_TRACE",
        "GIT_TRACE_PACKET",
        "GIT_TRACE_CURL",
        "GIT_SSH",
        "GIT_SSH_COMMAND",
    ] {
        command.env_remove(name);
    }
    for (key, value) in environment {
        command.env(key, value);
    }
    let child = if clear_environment {
        SupervisedDuplexChild::spawn_cleared(&mut command)
    } else {
        SupervisedDuplexChild::spawn(&mut command)
    };
    let mut child = child.map_err(|_| known("LATTICE_MANAGED_VERIFIER_PROCESS_SPAWN_FAILED"))?;
    let Some(stdout) = child.take_stdout() else {
        child
            .terminate_and_reap()
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_PROCESS_WAIT_FAILED"))?;
        return Err(known(
            "LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_CREATE_FAILED",
        ));
    };
    let (output_sender, output_receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let _active = ActiveProcessIoThread::enter();
        let result = if capture_stdout {
            let mut stdout = stdout;
            let mut captured = Vec::new();
            let mut buffer = vec![0_u8; 64 * 1_024].into_boxed_slice();
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break ProcessOutputRead::Captured(captured),
                    Ok(read)
                        if captured
                            .len()
                            .checked_add(read)
                            .is_some_and(|length| length <= MAX_GIT_OUTPUT_BYTES) =>
                    {
                        captured.extend_from_slice(&buffer[..read]);
                    }
                    Ok(_) => break ProcessOutputRead::Limit,
                    Err(_) => break ProcessOutputRead::Failed,
                }
            }
        } else {
            let mut stdout = stdout;
            match std::io::copy(&mut stdout, &mut std::io::sink()) {
                Ok(_) => ProcessOutputRead::Drained,
                Err(_) => ProcessOutputRead::Failed,
            }
        };
        let _ = output_sender.send(result);
    });
    let Some(mut child_stdin) = child.take_stdin() else {
        finish_process_io(
            &mut child,
            &output_receiver,
            None,
            None,
            Some(ProcessInputWrite::Written),
            reader,
            None,
        )?;
        return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_STDIN_FAILED"));
    };
    let (input_receiver, writer) = if let Some(bytes) = stdin {
        let bytes = bytes.to_vec();
        let (input_sender, input_receiver) = mpsc::sync_channel(1);
        let writer = thread::spawn(move || {
            let _active = ActiveProcessIoThread::enter();
            let result = child_stdin
                .write_all(&bytes)
                .and_then(|()| child_stdin.flush())
                .map_or(ProcessInputWrite::Failed, |()| ProcessInputWrite::Written);
            drop(child_stdin);
            let _ = input_sender.send(result);
        });
        (Some(input_receiver), Some(writer))
    } else {
        drop(child_stdin);
        (None, None)
    };
    let mut output_observation = None;
    let mut input_observation = input_receiver
        .is_none()
        .then_some(ProcessInputWrite::Written);
    let mut reader = Some(reader);
    let mut writer = writer;
    let Some(deadline) = Instant::now().checked_add(timeout) else {
        finish_process_io(
            &mut child,
            &output_receiver,
            input_receiver.as_ref(),
            output_observation.take(),
            input_observation.take(),
            reader
                .take()
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED"))?,
            writer.take(),
        )?;
        return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_FAILED"));
    };
    let status = loop {
        if output_observation.is_none() {
            match output_receiver.try_recv() {
                Ok(observation) => {
                    let failed = matches!(
                        observation,
                        ProcessOutputRead::Limit | ProcessOutputRead::Failed
                    );
                    output_observation = Some(observation);
                    if failed {
                        finish_process_io(
                            &mut child,
                            &output_receiver,
                            input_receiver.as_ref(),
                            output_observation.take(),
                            input_observation.take(),
                            reader.take().ok_or_else(|| {
                                known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED")
                            })?,
                            writer.take(),
                        )?;
                        return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED"));
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    finish_process_io(
                        &mut child,
                        &output_receiver,
                        input_receiver.as_ref(),
                        Some(ProcessOutputRead::Failed),
                        input_observation.take(),
                        reader.take().ok_or_else(|| {
                            known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED")
                        })?,
                        writer.take(),
                    )?;
                    return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED"));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if input_observation.is_none() {
            let Some(receiver) = input_receiver.as_ref() else {
                input_observation = Some(ProcessInputWrite::Written);
                continue;
            };
            match receiver.try_recv() {
                Ok(observation) => {
                    let failed = matches!(observation, ProcessInputWrite::Failed);
                    input_observation = Some(observation);
                    if failed {
                        finish_process_io(
                            &mut child,
                            &output_receiver,
                            input_receiver.as_ref(),
                            output_observation.take(),
                            input_observation.take(),
                            reader.take().ok_or_else(|| {
                                known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED")
                            })?,
                            writer.take(),
                        )?;
                        return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_STDIN_FAILED"));
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    finish_process_io(
                        &mut child,
                        &output_receiver,
                        input_receiver.as_ref(),
                        output_observation.take(),
                        Some(ProcessInputWrite::Failed),
                        reader.take().ok_or_else(|| {
                            known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED")
                        })?,
                        writer.take(),
                    )?;
                    return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_STDIN_FAILED"));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let Ok(observed_status) = child.try_wait() else {
            finish_process_io(
                &mut child,
                &output_receiver,
                input_receiver.as_ref(),
                output_observation.take(),
                input_observation.take(),
                reader
                    .take()
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED"))?,
                writer.take(),
            )?;
            return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_WAIT_FAILED"));
        };
        if let Some(status) = observed_status {
            break status;
        }
        if Instant::now() >= deadline {
            finish_process_io(
                &mut child,
                &output_receiver,
                input_receiver.as_ref(),
                output_observation.take(),
                input_observation.take(),
                reader
                    .take()
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED"))?,
                writer.take(),
            )?;
            return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_TIMEOUT"));
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = finish_process_io(
        &mut child,
        &output_receiver,
        input_receiver.as_ref(),
        output_observation,
        input_observation,
        reader.ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED"))?,
        writer,
    )?;
    Ok(ProcessResult { status, stdout })
}

#[allow(clippy::too_many_lines)]
fn validate_wsl_git_preflight(
    descriptor: &ExecutionEnvironmentDescriptor,
    preflight: &VerifiedManagedEvidence,
    receipt: &Value,
) -> ManagedPortResult<WslGitPreflightContext> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED");
    let object = receipt.as_object().ok_or_else(rejected)?;
    let process_fence = object
        .get("process_fence")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let continuation = object
        .get("continuation")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let bounds = object
        .get("bounds")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let timeout = object
        .get("timeout")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let worktree_ref = object
        .get("worktree_ref")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "worktree"))
        .ok_or_else(rejected)?;
    let fence = process_fence
        .get("fence")
        .and_then(Value::as_str)
        .filter(|value| plain_sha256(value))
        .ok_or_else(rejected)?;
    let credential_seal_digest = object
        .get("credential_seal_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "credential-seal"))
        .ok_or_else(rejected)?;
    let retry_of = continuation_marker_value(continuation.get("retry_of"), rejected)?;
    let reconnect_of = continuation_marker_value(continuation.get("reconnect_of"), rejected)?;
    let attempt = u64::from(preflight.attempt());
    let continuation_attempt = continuation
        .get("attempt")
        .and_then(Value::as_u64)
        .ok_or_else(rejected)?;
    let stdout_limit_bytes = bounds
        .get("stdout_limit_bytes")
        .and_then(Value::as_u64)
        .filter(|value| (1_024..=1_048_576).contains(value))
        .ok_or_else(rejected)?;
    let stderr_limit_bytes = bounds
        .get("stderr_limit_bytes")
        .and_then(Value::as_u64)
        .filter(|value| (1_024..=1_048_576).contains(value))
        .ok_or_else(rejected)?;
    let timeout_ms = timeout
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .filter(|value| (1_000..=300_000).contains(value))
        .ok_or_else(rejected)?;
    let descriptor_value: Value = serde_json::from_str(descriptor.as_json())
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REJECTED"))?;
    let process_authority_ref = descriptor_value
        .pointer("/process_fence/identity_digest")
        .and_then(Value::as_str)
        .ok_or_else(rejected)?;
    let unit_prefix = descriptor_value
        .pointer("/process_fence/unit_prefix")
        .and_then(Value::as_str)
        .filter(|value| {
            value.len() == "lattice-wsl2-".len() + 16
                && value.starts_with("lattice-wsl2-")
                && value["lattice-wsl2-".len()..]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(rejected)?;
    if object.get("schema").and_then(Value::as_str) != Some("lattice.wsl2-zero-model-preflight/1.0")
        || object.get("status").and_then(Value::as_str) != Some("PASS")
        || object.get("task_ref").and_then(Value::as_str) != Some(preflight.task_ref().as_str())
        || object.get("attempt").and_then(Value::as_u64) != Some(attempt)
        || object
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(descriptor.environment_ref().as_str())
        || object.get("descriptor_digest").and_then(Value::as_str)
            != Some(descriptor.environment_ref().as_str())
        || object.get("linux_cwd").and_then(Value::as_str)
            != Some(descriptor.linux_repository_path())
        || object.get("repository_head").and_then(Value::as_str)
            != Some(descriptor.repository_head())
        || object
            .get("verification_toolchain_ref")
            .and_then(Value::as_str)
            != Some(descriptor.verification_toolchain_identity_ref())
        || object.get("provider_effect_count").and_then(Value::as_u64) != Some(0)
        || process_fence.get("authority_ref").and_then(Value::as_str) != Some(process_authority_ref)
        || process_fence
            .get("supervisor_zero_descendants")
            .and_then(Value::as_bool)
            != Some(true)
        || continuation_attempt != attempt
        || !continuation_shape_valid(attempt, retry_of.as_deref(), reconnect_of.as_deref())
        || bounds
            .get("stdout_observed_bytes")
            .and_then(Value::as_u64)
            .is_none_or(|value| value > stdout_limit_bytes)
        || bounds
            .get("stderr_observed_bytes")
            .and_then(Value::as_u64)
            .is_none_or(|value| value > stderr_limit_bytes)
        || timeout.get("timed_out").and_then(Value::as_bool) != Some(false)
        || timeout.get("interrupted").and_then(Value::as_bool) != Some(false)
        || object
            .get("receipt_digest")
            .and_then(Value::as_str)
            .is_none_or(|value| !typed_sha256(value, "wsl2-preflight"))
    {
        return Err(rejected());
    }
    let supplied_receipt_digest = object["receipt_digest"].as_str().ok_or_else(rejected)?;
    let mut receipt_subject = receipt.clone();
    receipt_subject
        .as_object_mut()
        .ok_or_else(rejected)?
        .remove("receipt_digest");
    if typed_json_sha256("wsl2-preflight", &receipt_subject)? != supplied_receipt_digest {
        return Err(rejected());
    }
    Ok(WslGitPreflightContext {
        worktree_ref: worktree_ref.to_owned(),
        preflight_fence: fence.to_owned(),
        credential_seal_digest: credential_seal_digest.to_owned(),
        timeout_ms,
        stdout_limit_bytes,
        stderr_limit_bytes,
        retry_of,
        reconnect_of,
        unit_prefix: unit_prefix.to_owned(),
    })
}

fn continuation_marker_value(
    value: Option<&Value>,
    rejected: impl Fn() -> ManagedPortError,
) -> ManagedPortResult<Option<String>> {
    match value {
        Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if typed_sha256(value, "verifier-receipt") => {
            Ok(Some(value.clone()))
        }
        _ => Err(rejected()),
    }
}

fn continuation_shape_valid(
    attempt: u64,
    retry_of: Option<&str>,
    reconnect_of: Option<&str>,
) -> bool {
    match attempt {
        1 => retry_of.is_none(),
        2.. => !(retry_of.is_some() && reconnect_of.is_some()),
        _ => false,
    }
}

fn continuation_marker_matches(value: Option<&Value>, expected: Option<&str>) -> bool {
    match (value, expected) {
        (Some(Value::Null), None) => true,
        (Some(Value::String(value)), Some(expected)) => value == expected,
        _ => false,
    }
}

fn wsl_verifier_terminal_outcome<'a>(
    result: &'a serde_json::Map<String, Value>,
    exit: &serde_json::Map<String, Value>,
) -> ManagedPortResult<&'a str> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED");
    let exit_code = match exit.get("exit_code") {
        Some(Value::Number(value)) => value.as_u64().filter(|value| *value <= 255),
        Some(Value::Null) => None,
        _ => return Err(rejected()),
    };
    let exit_signal = match exit.get("exit_signal") {
        Some(Value::String(value))
            if !value.is_empty()
                && value.len() <= 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()) =>
        {
            Some(value.as_str())
        }
        Some(Value::Null) => None,
        _ => return Err(rejected()),
    };
    if exit_code.is_some() == exit_signal.is_some() {
        return Err(rejected());
    }
    let interrupted = exit
        .get("interrupted")
        .and_then(Value::as_bool)
        .ok_or_else(rejected)?;
    let timed_out = exit
        .get("timed_out")
        .and_then(Value::as_bool)
        .ok_or_else(rejected)?;
    let output_bound_exceeded = exit
        .get("output_bound_exceeded")
        .and_then(Value::as_bool)
        .ok_or_else(rejected)?;
    let expected = if interrupted {
        "INTERRUPTED"
    } else if timed_out {
        "TIMED_OUT"
    } else if output_bound_exceeded {
        "OUTPUT_BOUND_EXCEEDED"
    } else if exit_code == Some(0) {
        "PASS"
    } else {
        "FAILED"
    };
    let outcome = result
        .get("outcome")
        .and_then(Value::as_str)
        .filter(|value| {
            matches!(
                *value,
                "PASS" | "FAILED" | "TIMED_OUT" | "INTERRUPTED" | "OUTPUT_BOUND_EXCEEDED"
            )
        })
        .ok_or_else(rejected)?;
    let expected_status = if outcome == "PASS" { "PASS" } else { "FAILED" };
    if outcome != expected || result.get("status").and_then(Value::as_str) != Some(expected_status)
    {
        return Err(rejected());
    }
    Ok(outcome)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn validate_wsl_verifier_cleanup(
    value: &Value,
    descriptor: &Value,
    outcome: &str,
    unit: &str,
    process_fence: &str,
    attempt: u64,
    retry_of: Option<&str>,
    reconnect_of: Option<&str>,
) -> ManagedPortResult<()> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED");
    let watchdog = matches!(
        outcome,
        "TIMED_OUT" | "INTERRUPTED" | "OUTPUT_BOUND_EXCEEDED" | "TRANSPORT_ERROR"
    );
    if !watchdog {
        return if value.is_null() {
            Ok(())
        } else {
            Err(rejected())
        };
    }
    if !exact_json_keys(
        value,
        &[
            "schema",
            "reason",
            "unit",
            "process_fence",
            "systemctl_identity",
            "attempt",
            "retry_of",
            "reconnect_of",
            "attempts",
            "cleanup_digest",
        ],
    ) {
        return Err(rejected());
    }
    let object = value.as_object().ok_or_else(rejected)?;
    let systemctl = object
        .get("systemctl_identity")
        .filter(|value| exact_json_keys(value, &["path", "version", "sha256"]))
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let expected_systemctl_path = descriptor
        .pointer("/process_fence/systemctl_path")
        .and_then(Value::as_str)
        .ok_or_else(rejected)?;
    let expected_systemctl_version = descriptor
        .pointer("/process_fence/systemctl_version")
        .and_then(Value::as_str)
        .ok_or_else(rejected)?;
    let expected_systemctl_sha256 = descriptor
        .pointer("/process_fence/systemctl_sha256")
        .and_then(Value::as_str)
        .ok_or_else(rejected)?;
    let attempts = object
        .get("attempts")
        .and_then(Value::as_array)
        .filter(|attempts| matches!(attempts.len(), 2 | 4))
        .ok_or_else(rejected)?;
    if object.get("schema").and_then(Value::as_str) != Some("lattice.wsl2-verifier-cleanup/1.0")
        || object.get("reason").and_then(Value::as_str) != Some(outcome)
        || object.get("unit").and_then(Value::as_str) != Some(unit)
        || object.get("process_fence").and_then(Value::as_str) != Some(process_fence)
        || object.get("attempt").and_then(Value::as_u64) != Some(attempt)
        || !continuation_marker_matches(object.get("retry_of"), retry_of)
        || !continuation_marker_matches(object.get("reconnect_of"), reconnect_of)
        || systemctl.get("path").and_then(Value::as_str) != Some(expected_systemctl_path)
        || systemctl.get("version").and_then(Value::as_str) != Some(expected_systemctl_version)
        || systemctl.get("sha256").and_then(Value::as_str) != Some(expected_systemctl_sha256)
        || !plain_sha256(expected_systemctl_sha256)
    {
        return Err(rejected());
    }
    let actions = if attempts.len() == 2 {
        ["TERM_KILL", "STOP", "", ""]
    } else {
        ["TERM_KILL", "STOP", "KILL", "FORCE_STOP"]
    };
    for (index, attempt_value) in attempts.iter().enumerate() {
        if !exact_json_keys(
            attempt_value,
            &[
                "sequence",
                "action",
                "result",
                "exit_code",
                "signal",
                "timed_out",
                "output_bound_exceeded",
                "stdout_captured_bytes",
                "stderr_captured_bytes",
                "stdout_sha256",
                "stderr_sha256",
            ],
        ) {
            return Err(rejected());
        }
        let attempt_object = attempt_value.as_object().ok_or_else(rejected)?;
        let result = attempt_object
            .get("result")
            .and_then(Value::as_str)
            .filter(|result| {
                matches!(
                    *result,
                    "SUCCESS"
                        | "EXIT_NONZERO"
                        | "TIMED_OUT"
                        | "OUTPUT_BOUND_EXCEEDED"
                        | "TRANSPORT_ERROR"
                )
            })
            .ok_or_else(rejected)?;
        let exit_code = match attempt_object.get("exit_code") {
            Some(Value::Number(value)) => value.as_u64(),
            Some(Value::Null) => None,
            _ => return Err(rejected()),
        };
        let signal = match attempt_object.get("signal") {
            Some(Value::String(value))
                if !value.is_empty()
                    && value.len() <= 32
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()) =>
            {
                Some(value.as_str())
            }
            Some(Value::Null) => None,
            _ => return Err(rejected()),
        };
        let timed_out = attempt_object
            .get("timed_out")
            .and_then(Value::as_bool)
            .ok_or_else(rejected)?;
        let output_bound_exceeded = attempt_object
            .get("output_bound_exceeded")
            .and_then(Value::as_bool)
            .ok_or_else(rejected)?;
        let result_matches = match result {
            "SUCCESS" => {
                exit_code == Some(0) && signal.is_none() && !timed_out && !output_bound_exceeded
            }
            "EXIT_NONZERO" => {
                exit_code.is_some_and(|code| code != 0)
                    && signal.is_none()
                    && !timed_out
                    && !output_bound_exceeded
            }
            "TIMED_OUT" => timed_out && !output_bound_exceeded,
            "OUTPUT_BOUND_EXCEEDED" => output_bound_exceeded,
            "TRANSPORT_ERROR" => exit_code.is_none() && !timed_out && !output_bound_exceeded,
            _ => false,
        };
        if attempt_object.get("sequence").and_then(Value::as_u64)
            != Some(u64::try_from(index + 1).unwrap_or(u64::MAX))
            || attempt_object.get("action").and_then(Value::as_str) != Some(actions[index])
            || attempt_object
                .get("stdout_captured_bytes")
                .and_then(Value::as_u64)
                .is_none_or(|bytes| bytes > 65_536)
            || attempt_object
                .get("stderr_captured_bytes")
                .and_then(Value::as_u64)
                .is_none_or(|bytes| bytes > 65_536)
            || attempt_object
                .get("stdout_sha256")
                .and_then(Value::as_str)
                .is_none_or(|digest| !plain_sha256(digest))
            || attempt_object
                .get("stderr_sha256")
                .and_then(Value::as_str)
                .is_none_or(|digest| !plain_sha256(digest))
            || !result_matches
        {
            return Err(rejected());
        }
    }
    if (attempts.len() == 4)
        != (attempts[0].get("result").and_then(Value::as_str) != Some("SUCCESS")
            || attempts[1].get("result").and_then(Value::as_str) != Some("SUCCESS"))
    {
        return Err(rejected());
    }
    let supplied = object
        .get("cleanup_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "wsl2-verifier-cleanup"))
        .ok_or_else(rejected)?;
    let mut subject = value.clone();
    subject
        .as_object_mut()
        .ok_or_else(rejected)?
        .remove("cleanup_digest");
    if typed_json_sha256("wsl2-verifier-cleanup", &subject)? != supplied {
        return Err(rejected());
    }
    Ok(())
}

fn validate_wsl_verifier_outer_exit(
    outer: &Value,
    outcome: &str,
    unit: &str,
    cgroup_path: &str,
) -> ManagedPortResult<()> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED");
    if !exact_json_keys(
        outer,
        &[
            "unit",
            "active_state",
            "sub_state",
            "result",
            "cgroup_path",
            "delegate",
            "cgroup_exists",
            "populated",
        ],
    ) {
        return Err(rejected());
    }
    let object = outer.as_object().ok_or_else(rejected)?;
    let cgroup_closed = match (
        object.get("cgroup_exists").and_then(Value::as_bool),
        object.get("populated"),
    ) {
        (Some(false), Some(Value::Null)) => true,
        (Some(true), Some(value)) => value.as_u64() == Some(0),
        _ => false,
    };
    let result = object.get("result").and_then(Value::as_str);
    let result_valid = if outcome == "PASS" {
        result == Some("success")
    } else if outcome == "FAILED" {
        result == Some("exit-code")
    } else {
        matches!(result, Some("success" | "exit-code" | "signal"))
    };
    if object.get("unit").and_then(Value::as_str) != Some(unit)
        || object.get("active_state").and_then(Value::as_str) != Some("inactive")
        || object.get("sub_state").and_then(Value::as_str) != Some("dead")
        || object.get("cgroup_path").and_then(Value::as_str) != Some(cgroup_path)
        || object.get("delegate").and_then(Value::as_str) != Some("no")
        || !result_valid
        || !cgroup_closed
    {
        return Err(rejected());
    }
    Ok(())
}

fn wsl_regular_verifier_fence(
    descriptor: &ExecutionEnvironmentDescriptor,
    task_ref: &str,
    attempt: u8,
    worktree_ref: &str,
    role: &str,
    args: &[&str],
    receipt: &Value,
) -> ManagedPortResult<String> {
    if !matches!(role, "NODE" | "CARGO") {
        return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
    }
    let receipt_ref = receipt
        .get("receipt_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "wsl2-preflight"))
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
    let preflight_fence = receipt
        .pointer("/process_fence/fence")
        .and_then(Value::as_str)
        .filter(|value| plain_sha256(value))
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
    let retry_of = receipt
        .pointer("/continuation/retry_of")
        .cloned()
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
    let reconnect_of = receipt
        .pointer("/continuation/reconnect_of")
        .cloned()
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"))?;
    let subject = serde_json::json!({
        "schema": "lattice.wsl2-verifier-fence/1.0",
        "task_ref": task_ref,
        "worktree_ref": worktree_ref,
        "execution_environment_ref": descriptor.environment_ref().as_str(),
        "preflight_receipt_ref": receipt_ref,
        "preflight_fence": preflight_fence,
        "role": role,
        "args": args,
        "attempt": attempt,
        "retry_of": retry_of,
        "reconnect_of": reconnect_of,
    });
    sha256_bytes(canonical_json_value(&subject)?.as_bytes())
        .map(|digest| digest.as_str().to_owned())
}

#[allow(clippy::too_many_lines)]
fn wsl_regular_verifier_command_digest(
    descriptor: &ExecutionEnvironmentDescriptor,
    receipt: &Value,
    role: &str,
    args: &[&str],
    process_fence: &str,
) -> ManagedPortResult<String> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED");
    if !matches!(role, "NODE" | "CARGO") || !plain_sha256(process_fence) {
        return Err(rejected());
    }
    let value: Value = serde_json::from_str(descriptor.as_json())
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REJECTED"))?;
    let toolchain = value
        .get("verification_toolchain")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let linux = value
        .get("linux")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let process = value
        .get("process_fence")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let string = |object: &serde_json::Map<String, Value>, key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .filter(|candidate| wsl_linux_command_path(candidate))
            .map(str::to_owned)
            .ok_or_else(rejected)
    };
    let tool_path = |key: &str| {
        toolchain
            .get(key)
            .and_then(Value::as_object)
            .and_then(|tool| tool.get("path"))
            .and_then(Value::as_str)
            .filter(|candidate| wsl_linux_command_path(candidate))
            .map(str::to_owned)
            .ok_or_else(rejected)
    };
    let task_root = string(toolchain, "task_root")?;
    let home_dir = string(toolchain, "home_dir")?;
    let temp_dir = string(toolchain, "temp_dir")?;
    let npm_cache = string(toolchain, "npm_cache")?;
    let cargo_home = string(toolchain, "cargo_home")?;
    let cargo_target = string(toolchain, "cargo_target_dir")?;
    let codex_home = string(linux, "codex_home")?;
    let linux_cwd = string(linux, "cwd")?;
    let user_runtime_dir = string(process, "user_runtime_dir")?;
    let linux_home = wsl_linux_home_from_task_root(&task_root).ok_or_else(rejected)?;
    let writable_roots = if role == "NODE" {
        vec![home_dir.clone(), temp_dir.clone(), npm_cache.clone()]
    } else {
        vec![
            home_dir.clone(),
            temp_dir.clone(),
            cargo_home.clone(),
            cargo_target.clone(),
        ]
    };
    let mut denied_roots = Vec::with_capacity(4);
    for candidate in [
        codex_home.clone(),
        format!("{linux_home}/.codex"),
        "/mnt".to_owned(),
        user_runtime_dir.clone(),
    ] {
        if !denied_roots.contains(&candidate) {
            denied_roots.push(candidate);
        }
    }
    if writable_roots
        .iter()
        .any(|candidate| denied_roots.iter().any(|denied| candidate == denied))
    {
        return Err(rejected());
    }
    let mut entries = vec![
        serde_json::json!({
            "path": { "type": "special", "value": { "kind": "minimal" } },
            "access": "read",
        }),
        serde_json::json!({
            "path": { "type": "path", "path": task_root },
            "access": "read",
        }),
    ];
    entries.extend(writable_roots.iter().map(|candidate| {
        serde_json::json!({
            "path": { "type": "path", "path": candidate },
            "access": "write",
        })
    }));
    entries.extend(denied_roots.iter().map(|candidate| {
        serde_json::json!({
            "path": { "type": "path", "path": candidate },
            "access": "deny",
            "missing_path_behavior": "skip",
        })
    }));
    let sandbox_state = serde_json::json!({
        "permissionProfile": {
            "type": "managed",
            "file_system": { "type": "restricted", "entries": entries },
            "network": "restricted",
        },
        "codexLinuxSandboxExe": Value::Null,
        "sandboxCwd": wsl_linux_file_uri(&linux_cwd)?,
        "useLegacyLandlock": false,
    });
    let node_path = string(linux, "node_path")?;
    let cargo_path = tool_path("cargo")?;
    let fixed_path = format!(
        "{}:{}:/usr/bin:/bin",
        wsl_posix_dirname(&node_path).ok_or_else(rejected)?,
        wsl_posix_dirname(&cargo_path).ok_or_else(rejected)?
    );
    let mut sandbox_environment = vec![
        format!("HOME={home_dir}"),
        format!("TMPDIR={temp_dir}"),
        format!("npm_config_cache={npm_cache}"),
        format!("CARGO_HOME={cargo_home}"),
        format!("CARGO_TARGET_DIR={cargo_target}"),
        format!("PATH={fixed_path}"),
        "LANG=C.UTF-8".to_owned(),
        "LC_ALL=C.UTF-8".to_owned(),
    ];
    if role == "NODE" {
        sandbox_environment.extend([
            "npm_config_offline=true".to_owned(),
            "npm_config_audit=false".to_owned(),
            "npm_config_fund=false".to_owned(),
        ]);
    } else {
        let rustc_path = tool_path("rustc")?;
        let rustdoc_path = tool_path("rustdoc")?;
        sandbox_environment.extend([
            format!("RUSTC={rustc_path}"),
            format!("RUSTDOC={rustdoc_path}"),
            "CARGO_NET_OFFLINE=true".to_owned(),
        ]);
    }
    let tool_key = if role == "NODE" { "npm" } else { "cargo" };
    let executable = toolchain.get(tool_key).cloned().ok_or_else(rejected)?;
    let sandbox = toolchain.get("sandbox").cloned().ok_or_else(rejected)?;
    let credential_seal = receipt
        .get("credential_seal_digest")
        .and_then(Value::as_str)
        .ok_or_else(rejected)?;
    let timeout_ms = receipt
        .pointer("/timeout/timeout_ms")
        .and_then(Value::as_u64)
        .ok_or_else(rejected)?;
    let stdout_limit = receipt
        .pointer("/bounds/stdout_limit_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(rejected)?;
    let stderr_limit = receipt
        .pointer("/bounds/stderr_limit_bytes")
        .and_then(Value::as_u64)
        .ok_or_else(rejected)?;
    let attempt = receipt
        .get("attempt")
        .and_then(Value::as_u64)
        .ok_or_else(rejected)?;
    let retry_of = receipt
        .pointer("/continuation/retry_of")
        .cloned()
        .ok_or_else(rejected)?;
    let reconnect_of = receipt
        .pointer("/continuation/reconnect_of")
        .cloned()
        .ok_or_else(rejected)?;
    let unit_prefix = process
        .get("unit_prefix")
        .and_then(Value::as_str)
        .ok_or_else(rejected)?;
    let service_unit = format!(
        "{}-{}-{}.service",
        unit_prefix,
        role.to_ascii_lowercase(),
        &process_fence[..12]
    );
    typed_json_sha256(
        "wsl2-verifier-command",
        &serde_json::json!({
            "role": role,
            "executable": executable,
            "sandbox": sandbox,
            "sandbox_state": sandbox_state,
            "sandbox_environment": sandbox_environment,
            "args": args,
            "cwd": linux_cwd,
            "process_fence": process_fence,
            "service_unit": service_unit,
            "execution_environment_ref": descriptor.environment_ref().as_str(),
            "credential_seal_digest": credential_seal,
            "supervisor_bootstrap_sha256": WSL2_SUPERVISOR_BOOTSTRAP_SHA256,
            "timeout_ms": timeout_ms,
            "stdout_limit_bytes": stdout_limit,
            "stderr_limit_bytes": stderr_limit,
            "attempt": attempt,
            "retry_of": retry_of,
            "reconnect_of": reconnect_of,
        }),
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn validate_wsl_transport_failure(
    result: &Value,
    descriptor: &ExecutionEnvironmentDescriptor,
    expected_task_ref: &str,
    expected_attempt: u8,
    expected_worktree_ref: &str,
    expected_role: &str,
    expected_credential_seal: &str,
    expected_retry_of: Option<&str>,
    expected_reconnect_of: Option<&str>,
    expected_invocation_digest: Option<&str>,
    expected_process_fence: Option<&str>,
    expected_command_digest: Option<&str>,
) -> ManagedPortResult<()> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED");
    let mut keys = vec![
        "schema",
        "status",
        "outcome",
        "retryable",
        "task_ref",
        "attempt",
        "worktree_ref",
        "role",
        "execution_environment_ref",
        "repository_head",
        "credential_seal_digest",
        "verifier_identity",
        "unit",
        "process_fence",
        "continuation",
        "transport_evidence",
        "outer_cleanup",
        "outer_post_exit",
        "provider_effect_count",
        "result_digest",
    ];
    if expected_invocation_digest.is_some() {
        keys.push("invocation_digest");
    }
    if !exact_json_keys(result, &keys) {
        return Err(rejected());
    }
    let object = result.as_object().ok_or_else(rejected)?;
    let descriptor_value: Value = serde_json::from_str(descriptor.as_json())
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REJECTED"))?;
    let process_fence = object
        .get("process_fence")
        .and_then(Value::as_str)
        .filter(|value| plain_sha256(value))
        .ok_or_else(rejected)?;
    if expected_process_fence.is_some_and(|expected| expected != process_fence) {
        return Err(rejected());
    }
    let unit = object
        .get("unit")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 255)
        .ok_or_else(rejected)?;
    let unit_prefix = descriptor_value
        .pointer("/process_fence/unit_prefix")
        .and_then(Value::as_str)
        .ok_or_else(rejected)?;
    let expected_unit = format!(
        "{}-{}-{}.service",
        unit_prefix,
        expected_role.to_ascii_lowercase(),
        &process_fence[..12]
    );
    let continuation = object
        .get("continuation")
        .filter(|value| exact_json_keys(value, &["retry_of", "reconnect_of"]))
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let identity = object
        .get("verifier_identity")
        .filter(|value| {
            exact_json_keys(
                value,
                &[
                    "schema",
                    "command_digest",
                    "execution_environment_ref",
                    "verification_toolchain_ref",
                    "credential_seal_digest",
                    "process_fence",
                    "linux_cwd",
                    "repository_head",
                    "provider_effect_count",
                ],
            )
        })
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let command_digest = identity
        .get("command_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "wsl2-verifier-command"))
        .ok_or_else(rejected)?;
    if expected_command_digest.is_some_and(|expected| expected != command_digest)
        || object.get("schema").and_then(Value::as_str)
            != Some("lattice.wsl2-verifier-transport-failure/1.0")
        || object.get("status").and_then(Value::as_str) != Some("FAILED")
        || object.get("outcome").and_then(Value::as_str) != Some("TRANSPORT_ERROR")
        || object.get("retryable").and_then(Value::as_bool) != Some(true)
        || object.get("task_ref").and_then(Value::as_str) != Some(expected_task_ref)
        || object.get("attempt").and_then(Value::as_u64) != Some(u64::from(expected_attempt))
        || object.get("worktree_ref").and_then(Value::as_str) != Some(expected_worktree_ref)
        || object.get("role").and_then(Value::as_str) != Some(expected_role)
        || object
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(descriptor.environment_ref().as_str())
        || object.get("repository_head").and_then(Value::as_str)
            != Some(descriptor.repository_head())
        || object.get("credential_seal_digest").and_then(Value::as_str)
            != Some(expected_credential_seal)
        || object.get("provider_effect_count").and_then(Value::as_u64) != Some(0)
        || object.get("invocation_digest").and_then(Value::as_str) != expected_invocation_digest
        || unit != expected_unit
        || !continuation_marker_matches(continuation.get("retry_of"), expected_retry_of)
        || !continuation_marker_matches(continuation.get("reconnect_of"), expected_reconnect_of)
        || identity.get("schema").and_then(Value::as_str)
            != Some("lattice.wsl2-verifier-launch/1.0")
        || identity
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(descriptor.environment_ref().as_str())
        || identity
            .get("verification_toolchain_ref")
            .and_then(Value::as_str)
            != Some(descriptor.verification_toolchain_identity_ref())
        || identity
            .get("credential_seal_digest")
            .and_then(Value::as_str)
            != Some(expected_credential_seal)
        || identity.get("process_fence").and_then(Value::as_str) != Some(process_fence)
        || identity.get("linux_cwd").and_then(Value::as_str)
            != Some(descriptor.linux_repository_path())
        || identity.get("repository_head").and_then(Value::as_str)
            != Some(descriptor.repository_head())
        || identity
            .get("provider_effect_count")
            .and_then(Value::as_u64)
            != Some(0)
    {
        return Err(rejected());
    }

    let evidence_value = object.get("transport_evidence").ok_or_else(rejected)?;
    if !exact_json_keys(
        evidence_value,
        &["schema", "error", "process", "output", "evidence_digest"],
    ) {
        return Err(rejected());
    }
    let evidence = evidence_value.as_object().ok_or_else(rejected)?;
    let error_value = evidence.get("error").ok_or_else(rejected)?;
    let process_value = evidence.get("process").ok_or_else(rejected)?;
    let output_value = evidence.get("output").ok_or_else(rejected)?;
    if !exact_json_keys(
        error_value,
        &[
            "source",
            "error_name",
            "error_code",
            "message_sha256",
            "error_type_digest",
        ],
    ) || !exact_json_keys(
        process_value,
        &["spawn_observed", "close_observed", "exit_code", "signal"],
    ) || !exact_json_keys(
        output_value,
        &[
            "stdout_captured_bytes",
            "stderr_captured_bytes",
            "stdout_seen_bytes",
            "stderr_seen_bytes",
            "stdout_bound_exceeded",
            "stderr_bound_exceeded",
            "stdout_sha256",
            "stderr_sha256",
        ],
    ) {
        return Err(rejected());
    }
    let error = error_value.as_object().ok_or_else(rejected)?;
    let process = process_value.as_object().ok_or_else(rejected)?;
    let output = output_value.as_object().ok_or_else(rejected)?;
    let source = error
        .get("source")
        .and_then(Value::as_str)
        .filter(|source| matches!(*source, "SPAWN" | "STDIN" | "STDOUT" | "STDERR" | "CHILD"))
        .ok_or_else(rejected)?;
    let error_name = error
        .get("error_name")
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value.bytes().enumerate().all(|(index, byte)| {
                    if index == 0 {
                        byte.is_ascii_alphabetic()
                    } else {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
                    }
                })
        })
        .ok_or_else(rejected)?;
    let error_code = match error.get("error_code") {
        Some(Value::String(value))
            if !value.is_empty()
                && value.len() <= 128
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-')
                }) =>
        {
            Some(value.as_str())
        }
        Some(Value::Null) => None,
        _ => return Err(rejected()),
    };
    let error_type = serde_json::json!({
        "source": source,
        "error_name": error_name,
        "error_code": error_code,
    });
    if error
        .get("message_sha256")
        .and_then(Value::as_str)
        .is_none_or(|value| !plain_sha256(value))
        || error.get("error_type_digest").and_then(Value::as_str)
            != Some(typed_json_sha256("wsl2-verifier-transport-error", &error_type)?.as_str())
    {
        return Err(rejected());
    }
    let spawn_observed = process
        .get("spawn_observed")
        .and_then(Value::as_bool)
        .ok_or_else(rejected)?;
    let close_observed = process
        .get("close_observed")
        .and_then(Value::as_bool)
        .ok_or_else(rejected)?;
    let exit_code = match process.get("exit_code") {
        Some(Value::Number(value)) => value.as_u64().filter(|value| *value <= 255),
        Some(Value::Null) => None,
        _ => return Err(rejected()),
    };
    let signal = match process.get("signal") {
        Some(Value::String(value))
            if !value.is_empty()
                && value.len() <= 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()) =>
        {
            Some(value.as_str())
        }
        Some(Value::Null) => None,
        _ => return Err(rejected()),
    };
    if (!close_observed && (exit_code.is_some() || signal.is_some()))
        || (!spawn_observed && (exit_code.is_some() || signal.is_some()))
    {
        return Err(rejected());
    }
    for channel in ["stdout", "stderr"] {
        let captured = output
            .get(&format!("{channel}_captured_bytes"))
            .and_then(Value::as_u64)
            .ok_or_else(rejected)?;
        let seen = output
            .get(&format!("{channel}_seen_bytes"))
            .and_then(Value::as_u64)
            .ok_or_else(rejected)?;
        let exceeded = output
            .get(&format!("{channel}_bound_exceeded"))
            .and_then(Value::as_bool)
            .ok_or_else(rejected)?;
        if captured > 1_310_720
            || (exceeded && (seen != 1_310_721 || captured != 1_310_720))
            || (!exceeded && seen != captured)
            || output
                .get(&format!("{channel}_sha256"))
                .and_then(Value::as_str)
                .is_none_or(|value| !plain_sha256(value))
        {
            return Err(rejected());
        }
    }
    if evidence.get("schema").and_then(Value::as_str)
        != Some("lattice.wsl2-verifier-transport-evidence/1.0")
    {
        return Err(rejected());
    }
    let evidence_digest = evidence
        .get("evidence_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "wsl2-verifier-transport-evidence"))
        .ok_or_else(rejected)?;
    let mut evidence_subject = evidence_value.clone();
    evidence_subject
        .as_object_mut()
        .ok_or_else(rejected)?
        .remove("evidence_digest");
    if typed_json_sha256("wsl2-verifier-transport-evidence", &evidence_subject)? != evidence_digest
    {
        return Err(rejected());
    }

    validate_wsl_verifier_cleanup(
        object.get("outer_cleanup").ok_or_else(rejected)?,
        &descriptor_value,
        "TRANSPORT_ERROR",
        unit,
        process_fence,
        u64::from(expected_attempt),
        expected_retry_of,
        expected_reconnect_of,
    )?;
    let owner_uid = descriptor_value
        .pointer("/verification_toolchain/owner_uid")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(rejected)?;
    let cgroup_path =
        format!("/user.slice/user-{owner_uid}.slice/user@{owner_uid}.service/app.slice/{unit}");
    validate_wsl_verifier_outer_exit(
        object.get("outer_post_exit").ok_or_else(rejected)?,
        "TRANSPORT_ERROR",
        unit,
        &cgroup_path,
    )?;
    let result_digest = object
        .get("result_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "wsl2-verifier-transport-failure"))
        .ok_or_else(rejected)?;
    let mut subject = result.clone();
    subject
        .as_object_mut()
        .ok_or_else(rejected)?
        .remove("result_digest");
    if typed_json_sha256("wsl2-verifier-transport-failure", &subject)? != result_digest {
        return Err(rejected());
    }
    Ok(())
}

fn wsl_git_arguments(args: &[OsString], distribution: &str) -> ManagedPortResult<Vec<String>> {
    if args.len() < 12 || args.len() > MAX_WSL_GIT_ARGUMENTS {
        return Err(known("LATTICE_MANAGED_VERIFIER_GIT_ARGUMENTS_REJECTED"));
    }
    let mut total = 0usize;
    let mut mapped = Vec::with_capacity(args.len());
    for arg in args {
        let text = arg
            .to_str()
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 8_192
                    && !value
                        .chars()
                        .any(|character| matches!(character, '\0' | '\r' | '\n'))
            })
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_ARGUMENTS_REJECTED"))?;
        total = total
            .checked_add(text.len())
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_ARGUMENTS_REJECTED"))?;
        if total > MAX_WSL_GIT_ARGUMENT_BYTES {
            return Err(known("LATTICE_MANAGED_VERIFIER_GIT_ARGUMENTS_REJECTED"));
        }
        mapped.push(if let Some(path) = text.strip_prefix("core.hooksPath=") {
            format!(
                "core.hooksPath={}",
                wsl_unc_text_to_linux(path, distribution)?
            )
        } else {
            text.to_owned()
        });
    }
    if mapped[0] != "--no-pager"
        || mapped[1] != "--no-replace-objects"
        || mapped[2] != "--literal-pathspecs"
        || mapped[3] != "-c"
        || !mapped[4].starts_with("core.hooksPath=/")
        || mapped[5] != "-c"
        || mapped[6] != "core.fsmonitor=false"
        || mapped[7] != "-c"
        || mapped[8] != "protocol.allow=never"
        || mapped[9] != "-c"
        || mapped[10] != "commit.gpgSign=false"
    {
        return Err(known("LATTICE_MANAGED_VERIFIER_GIT_ARGUMENTS_REJECTED"));
    }
    Ok(mapped)
}

fn wsl_git_environment(
    environment: &[(OsString, OsString)],
    distribution: &str,
) -> ManagedPortResult<Value> {
    let mut mapped = serde_json::Map::new();
    for (key, value) in environment {
        let key = key
            .to_str()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_ENVIRONMENT_REJECTED"))?;
        let value = value
            .to_str()
            .filter(|value| {
                value.len() <= 8_192
                    && !value
                        .chars()
                        .any(|character| matches!(character, '\0' | '\r' | '\n'))
            })
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_ENVIRONMENT_REJECTED"))?;
        let admitted = match key {
            "HOME"
            | "GIT_CONFIG_GLOBAL"
            | "GIT_WORK_TREE"
            | "GIT_DIR"
            | "GIT_COMMON_DIR"
            | "GIT_OBJECT_DIRECTORY"
            | "GIT_INDEX_FILE" => Some((key, wsl_unc_text_to_linux(value, distribution)?)),
            "TEMP" | "TMP" => Some(("TMPDIR", wsl_unc_text_to_linux(value, distribution)?)),
            "NO_COLOR"
            | "CI"
            | "GIT_CONFIG_NOSYSTEM"
            | "GIT_CONFIG_COUNT"
            | "GIT_TERMINAL_PROMPT"
            | "GIT_OPTIONAL_LOCKS"
            | "GIT_ATTR_NOSYSTEM"
            | "GIT_AUTHOR_NAME"
            | "GIT_AUTHOR_EMAIL"
            | "GIT_AUTHOR_DATE"
            | "GIT_COMMITTER_NAME"
            | "GIT_COMMITTER_EMAIL"
            | "GIT_COMMITTER_DATE" => Some((key, value.to_owned())),
            "USERPROFILE"
            | "PATH"
            | "SystemRoot"
            | "WINDIR"
            | "PROCESSOR_ARCHITECTURE"
            | "NUMBER_OF_PROCESSORS"
            | "PATHEXT" => None,
            _ => return Err(known("LATTICE_MANAGED_VERIFIER_GIT_ENVIRONMENT_REJECTED")),
        };
        if let Some((output_key, admitted)) = admitted {
            if let Some(existing) = mapped.get(output_key) {
                if output_key == "TMPDIR" && existing.as_str() == Some(admitted.as_str()) {
                    continue;
                }
                return Err(known("LATTICE_MANAGED_VERIFIER_GIT_ENVIRONMENT_REJECTED"));
            }
            mapped.insert(output_key.to_owned(), Value::String(admitted));
        }
    }
    Ok(Value::Object(mapped))
}

#[allow(clippy::too_many_lines)]
fn validate_wsl_git_result(
    descriptor: &ExecutionEnvironmentDescriptor,
    preflight: &VerifiedManagedEvidence,
    context: &WslGitPreflightContext,
    sequence: u64,
    invocation_digest: &str,
    process_fence: &str,
    command_digest: &str,
    stdin: Option<&[u8]>,
    result: &Value,
) -> ManagedPortResult<ProcessResult> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED");
    if !(1..=MAX_WSL_GIT_INVOCATIONS).contains(&sequence) {
        return Err(rejected());
    }
    if result.get("schema").and_then(Value::as_str)
        == Some("lattice.wsl2-verifier-transport-failure/1.0")
    {
        validate_wsl_transport_failure(
            result,
            descriptor,
            preflight.task_ref().as_str(),
            preflight.attempt(),
            context.worktree_ref.as_str(),
            "GIT",
            context.credential_seal_digest.as_str(),
            context.retry_of.as_deref(),
            context.reconnect_of.as_deref(),
            Some(invocation_digest),
            Some(process_fence),
            Some(command_digest),
        )?;
        return Ok(ProcessResult {
            status: exit_status_from_code(1),
            stdout: Vec::new(),
        });
    }
    if !exact_json_keys(
        result,
        &[
            "schema",
            "status",
            "outcome",
            "task_ref",
            "attempt",
            "worktree_ref",
            "role",
            "repository_head",
            "verifier_identity",
            "process_marker",
            "exit_receipt",
            "outer_cleanup",
            "outer_post_exit",
            "output",
            "provider_effect_count",
            "invocation_digest",
            "result_digest",
        ],
    ) {
        return Err(rejected());
    }
    let object = result.as_object().ok_or_else(rejected)?;
    let identity = object
        .get("verifier_identity")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let marker = object
        .get("process_marker")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let exit = object
        .get("exit_receipt")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let outer = object
        .get("outer_post_exit")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let output = object
        .get("output")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let descriptor_value: Value = serde_json::from_str(descriptor.as_json())
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REJECTED"))?;
    if !exact_json_keys(
        object.get("verifier_identity").ok_or_else(rejected)?,
        &[
            "schema",
            "command_digest",
            "execution_environment_ref",
            "verification_toolchain_ref",
            "credential_seal_digest",
            "process_fence",
            "linux_cwd",
            "repository_head",
            "provider_effect_count",
        ],
    ) || !exact_json_keys(
        object.get("process_marker").ok_or_else(rejected)?,
        &[
            "schema",
            "fence",
            "unit",
            "execution_environment_ref",
            "credential_seal_digest",
            "boot_id_digest",
            "pid",
            "process_start_ticks",
            "process_group_id",
            "cgroup_path",
            "cgroup_version",
            "delegated",
            "attempt",
            "retry_of",
            "reconnect_of",
        ],
    ) || !exact_json_keys(
        object.get("exit_receipt").ok_or_else(rejected)?,
        &[
            "schema",
            "fence",
            "unit",
            "execution_environment_ref",
            "credential_seal_digest",
            "cgroup_path",
            "zero_descendants",
            "credential_seal_intact",
            "credential_watch_intact",
            "keyring_daemon_sha256",
            "keyring_library_manifest_digest",
            "tool_input_identities",
            "stdout_bytes",
            "stderr_bytes",
            "stdout_limit_bytes",
            "stderr_limit_bytes",
            "output_bound_exceeded",
            "timeout_ms",
            "timed_out",
            "interrupted",
            "stdin_bytes",
            "stdin_sha256",
            "stdin_complete",
            "attempt",
            "retry_of",
            "reconnect_of",
            "exit_code",
            "exit_signal",
        ],
    ) || !exact_json_keys(
        object.get("outer_post_exit").ok_or_else(rejected)?,
        &[
            "unit",
            "active_state",
            "sub_state",
            "result",
            "cgroup_path",
            "delegate",
            "cgroup_exists",
            "populated",
        ],
    ) || !exact_json_keys(
        object.get("output").ok_or_else(rejected)?,
        &[
            "stdout_observed_bytes",
            "stderr_observed_bytes",
            "stdout_sha256",
            "stderr_sha256",
            "stdout_base64",
        ],
    ) {
        return Err(rejected());
    }
    let stdout = output
        .get("stdout_base64")
        .and_then(Value::as_str)
        .ok_or_else(rejected)
        .and_then(base64_decode)?;
    let stdout_digest = sha256_bytes(&stdout)?;
    let stdin = stdin.unwrap_or_default();
    let stdin_digest = sha256_bytes(stdin)?;
    let exit_code = exit
        .get("exit_code")
        .and_then(Value::as_u64)
        .filter(|value| *value <= 255);
    let outcome = wsl_verifier_terminal_outcome(object, exit)?;
    let unit = marker
        .get("unit")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 255)
        .ok_or_else(rejected)?;
    let cgroup_path = marker
        .get("cgroup_path")
        .and_then(Value::as_str)
        .filter(|value| value.starts_with('/') && value.len() <= 4_096)
        .ok_or_else(rejected)?;
    let outer_cgroup_closed = match (
        outer.get("cgroup_exists").and_then(Value::as_bool),
        outer.get("populated"),
    ) {
        (Some(false), Some(Value::Null)) => true,
        (Some(true), Some(value)) => value.as_u64() == Some(0),
        _ => false,
    };
    validate_wsl_verifier_cleanup(
        object.get("outer_cleanup").ok_or_else(rejected)?,
        &descriptor_value,
        outcome,
        unit,
        process_fence,
        u64::from(preflight.attempt()),
        context.retry_of.as_deref(),
        context.reconnect_of.as_deref(),
    )?;
    validate_wsl_verifier_outer_exit(
        object.get("outer_post_exit").ok_or_else(rejected)?,
        outcome,
        unit,
        cgroup_path,
    )?;
    let stdout_limit = context.stdout_limit_bytes;
    let stderr_limit = context.stderr_limit_bytes;
    let stdout_receipt_bytes = exit.get("stdout_bytes").and_then(Value::as_u64);
    let stderr_receipt_bytes = exit.get("stderr_bytes").and_then(Value::as_u64);
    let output_bound = exit.get("output_bound_exceeded").and_then(Value::as_bool);
    let output_counts_valid = match (stdout_receipt_bytes, stderr_receipt_bytes, output_bound) {
        (Some(stdout_bytes), Some(stderr_bytes), Some(true)) => {
            stdout_bytes <= stdout_limit.saturating_add(1)
                && stderr_bytes <= stderr_limit.saturating_add(1)
                && (stdout_bytes == stdout_limit.saturating_add(1)
                    || stderr_bytes == stderr_limit.saturating_add(1))
                && u64::try_from(stdout.len()).unwrap_or(u64::MAX) <= stdout_limit
        }
        (Some(stdout_bytes), Some(stderr_bytes), Some(false)) => {
            stdout_bytes == u64::try_from(stdout.len()).unwrap_or(u64::MAX)
                && stdout_bytes <= stdout_limit
                && stderr_bytes <= stderr_limit
        }
        _ => false,
    };
    if object.get("schema").and_then(Value::as_str) != Some("lattice.wsl2-verifier-result/1.0")
        || object.get("task_ref").and_then(Value::as_str) != Some(preflight.task_ref().as_str())
        || object.get("attempt").and_then(Value::as_u64) != Some(u64::from(preflight.attempt()))
        || object.get("worktree_ref").and_then(Value::as_str) != Some(context.worktree_ref.as_str())
        || object.get("role").and_then(Value::as_str) != Some("GIT")
        || object.get("repository_head").and_then(Value::as_str)
            != Some(descriptor.repository_head())
        || object.get("provider_effect_count").and_then(Value::as_u64) != Some(0)
        || object.get("invocation_digest").and_then(Value::as_str) != Some(invocation_digest)
        || identity.get("schema").and_then(Value::as_str)
            != Some("lattice.wsl2-verifier-launch/1.0")
        || identity.get("command_digest").and_then(Value::as_str) != Some(command_digest)
        || identity
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(descriptor.environment_ref().as_str())
        || identity
            .get("verification_toolchain_ref")
            .and_then(Value::as_str)
            != Some(descriptor.verification_toolchain_identity_ref())
        || identity
            .get("credential_seal_digest")
            .and_then(Value::as_str)
            != Some(context.credential_seal_digest.as_str())
        || identity.get("process_fence").and_then(Value::as_str) != Some(process_fence)
        || identity.get("linux_cwd").and_then(Value::as_str)
            != Some(descriptor.linux_repository_path())
        || identity.get("repository_head").and_then(Value::as_str)
            != Some(descriptor.repository_head())
        || identity
            .get("provider_effect_count")
            .and_then(Value::as_u64)
            != Some(0)
        || marker.get("schema").and_then(Value::as_str) != Some("lattice.wsl2-process-fence/1.1")
        || marker.get("fence").and_then(Value::as_str) != Some(process_fence)
        || marker
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(descriptor.environment_ref().as_str())
        || marker.get("credential_seal_digest").and_then(Value::as_str)
            != Some(context.credential_seal_digest.as_str())
        || marker
            .get("boot_id_digest")
            .and_then(Value::as_str)
            .is_none_or(|value| !typed_sha256(value, "wsl-boot"))
        || marker
            .get("pid")
            .and_then(Value::as_u64)
            .is_none_or(|value| value == 0)
        || marker
            .get("process_start_ticks")
            .and_then(Value::as_str)
            .is_none_or(|value| {
                value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
            })
        || marker
            .get("process_group_id")
            .and_then(Value::as_u64)
            .is_none_or(|value| value == 0)
        || marker.get("cgroup_version").and_then(Value::as_u64) != Some(2)
        || marker.get("delegated").and_then(Value::as_bool) != Some(false)
        || marker.get("attempt").and_then(Value::as_u64) != Some(u64::from(preflight.attempt()))
        || !continuation_marker_matches(marker.get("retry_of"), context.retry_of.as_deref())
        || !continuation_marker_matches(marker.get("reconnect_of"), context.reconnect_of.as_deref())
        || exit.get("schema").and_then(Value::as_str) != Some("lattice.wsl2-subtree-exit/1.2")
        || exit.get("fence").and_then(Value::as_str) != Some(process_fence)
        || exit.get("unit").and_then(Value::as_str) != Some(unit)
        || exit
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(descriptor.environment_ref().as_str())
        || exit.get("credential_seal_digest").and_then(Value::as_str)
            != Some(context.credential_seal_digest.as_str())
        || exit.get("cgroup_path").and_then(Value::as_str) != Some(cgroup_path)
        || exit.get("zero_descendants").and_then(Value::as_bool) != Some(true)
        || exit.get("credential_seal_intact").and_then(Value::as_bool) != Some(true)
        || exit.get("credential_watch_intact").and_then(Value::as_bool) != Some(true)
        || !wsl_git_receipt_tool_inputs_match(&descriptor_value, exit)
        || !output_counts_valid
        || exit.get("stdout_limit_bytes").and_then(Value::as_u64)
            != Some(context.stdout_limit_bytes)
        || exit.get("stderr_limit_bytes").and_then(Value::as_u64)
            != Some(context.stderr_limit_bytes)
        || exit.get("timeout_ms").and_then(Value::as_u64) != Some(context.timeout_ms)
        || exit.get("stdin_bytes").and_then(Value::as_u64)
            != Some(u64::try_from(stdin.len()).unwrap_or(u64::MAX))
        || exit.get("stdin_sha256").and_then(Value::as_str) != Some(stdin_digest.as_str())
        || exit.get("stdin_complete").and_then(Value::as_bool) != Some(true)
        || exit.get("attempt").and_then(Value::as_u64) != Some(u64::from(preflight.attempt()))
        || !continuation_marker_matches(exit.get("retry_of"), context.retry_of.as_deref())
        || !continuation_marker_matches(exit.get("reconnect_of"), context.reconnect_of.as_deref())
        || outer.get("unit").and_then(Value::as_str) != Some(unit)
        || outer.get("active_state").and_then(Value::as_str) != Some("inactive")
        || outer.get("sub_state").and_then(Value::as_str) != Some("dead")
        || outer.get("cgroup_path").and_then(Value::as_str) != Some(cgroup_path)
        || outer.get("delegate").and_then(Value::as_str) != Some("no")
        || !outer_cgroup_closed
        || output.get("stdout_observed_bytes").and_then(Value::as_u64)
            != Some(u64::try_from(stdout.len()).unwrap_or(u64::MAX))
        || output
            .get("stderr_observed_bytes")
            .and_then(Value::as_u64)
            .is_none_or(|value| value > u64::try_from(MAX_WSL_GIT_RESULT_BYTES).unwrap_or(u64::MAX))
        || output.get("stdout_sha256").and_then(Value::as_str) != Some(stdout_digest.as_str())
        || output
            .get("stderr_sha256")
            .and_then(Value::as_str)
            .is_none_or(|value| !plain_sha256(value))
        || unit
            != format!(
                "{}-git-{}.service",
                context.unit_prefix,
                &process_fence[..12]
            )
    {
        return Err(rejected());
    }
    let supplied_result_digest = object
        .get("result_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "wsl2-verifier-result"))
        .ok_or_else(rejected)?;
    let mut result_subject = result.clone();
    result_subject
        .as_object_mut()
        .ok_or_else(rejected)?
        .remove("result_digest");
    if typed_json_sha256("wsl2-verifier-result", &result_subject)? != supplied_result_digest {
        return Err(rejected());
    }
    Ok(ProcessResult {
        status: exit_status_from_code(if outcome == "PASS" {
            0
        } else if outcome == "FAILED" {
            u32::try_from(exit_code.unwrap_or(1)).unwrap_or(1).max(1)
        } else {
            1
        }),
        stdout,
    })
}

fn wsl_git_receipt_tool_inputs_match(
    descriptor: &Value,
    exit: &serde_json::Map<String, Value>,
) -> bool {
    let Some(linux) = descriptor.get("linux").and_then(Value::as_object) else {
        return false;
    };
    let Some(toolchain) = descriptor
        .get("verification_toolchain")
        .and_then(Value::as_object)
    else {
        return false;
    };
    let Some(sandbox) = toolchain.get("sandbox").and_then(Value::as_object) else {
        return false;
    };
    let Some(sandbox_helper) = toolchain.get("sandbox_helper").and_then(Value::as_object) else {
        return false;
    };
    let Some(expected_keyring_digest) =
        json_object_string(linux, "keyring_daemon_sha256").filter(|value| plain_sha256(value))
    else {
        return false;
    };
    let Some(expected_manifest_digest) =
        json_object_string(linux, "keyring_library_manifest_digest")
            .filter(|value| typed_sha256(value, "keyring-library-manifest"))
    else {
        return false;
    };
    if exit.get("keyring_daemon_sha256").and_then(Value::as_str) != Some(expected_keyring_digest)
        || exit
            .get("keyring_library_manifest_digest")
            .and_then(Value::as_str)
            != Some(expected_manifest_digest)
    {
        return false;
    }
    let Some(inputs) = exit
        .get("tool_input_identities")
        .filter(|value| {
            exact_json_keys(
                value,
                &[
                    "executable",
                    "verifier_tool",
                    "sandbox_helper",
                    "node_runtime",
                    "rustc",
                    "rustdoc",
                    "keyring_daemon",
                    "keyring_libraries",
                ],
            )
        })
        .and_then(Value::as_object)
    else {
        return false;
    };
    if !inputs.get("node_runtime").is_some_and(Value::is_null)
        || !inputs.get("rustc").is_some_and(Value::is_null)
        || !inputs.get("rustdoc").is_some_and(Value::is_null)
        || !wsl_git_tool_seal_matches(
            inputs.get("executable"),
            json_object_string(sandbox, "path"),
            json_object_string(sandbox, "sha256"),
        )
        || !wsl_git_tool_seal_matches(
            inputs.get("verifier_tool"),
            json_object_string(linux, "git_path"),
            json_object_string(linux, "git_sha256"),
        )
        || !wsl_git_tool_seal_matches(
            inputs.get("sandbox_helper"),
            json_object_string(sandbox_helper, "path"),
            json_object_string(sandbox_helper, "sha256"),
        )
        || !wsl_git_tool_seal_matches(
            inputs.get("keyring_daemon"),
            json_object_string(linux, "keyring_daemon_path"),
            Some(expected_keyring_digest),
        )
    {
        return false;
    }
    let Some(library_root) = json_object_string(linux, "keyring_library_path") else {
        return false;
    };
    let Some(libraries) = inputs.get("keyring_libraries").and_then(Value::as_array) else {
        return false;
    };
    const LIBRARIES: [&str; 2] = ["libgck-1.so.0.0.0", "libgcr-base-3.so.1.0.0"];
    libraries.len() == LIBRARIES.len()
        && libraries
            .iter()
            .zip(LIBRARIES)
            .all(|(identity, manifest_path)| {
                wsl_git_library_seal_matches(identity, library_root, manifest_path)
            })
}

fn wsl_regular_receipt_tool_inputs_match(
    descriptor: &Value,
    exit: &serde_json::Map<String, Value>,
    role: &str,
) -> bool {
    if !matches!(role, "NODE" | "CARGO") {
        return false;
    }
    let Some(linux) = descriptor.get("linux").and_then(Value::as_object) else {
        return false;
    };
    let Some(toolchain) = descriptor
        .get("verification_toolchain")
        .and_then(Value::as_object)
    else {
        return false;
    };
    let Some(sandbox) = toolchain.get("sandbox").and_then(Value::as_object) else {
        return false;
    };
    let Some(sandbox_helper) = toolchain.get("sandbox_helper").and_then(Value::as_object) else {
        return false;
    };
    let verifier_key = if role == "NODE" { "npm" } else { "cargo" };
    let Some(verifier) = toolchain.get(verifier_key).and_then(Value::as_object) else {
        return false;
    };
    let Some(expected_keyring_digest) =
        json_object_string(linux, "keyring_daemon_sha256").filter(|value| plain_sha256(value))
    else {
        return false;
    };
    let Some(expected_manifest_digest) =
        json_object_string(linux, "keyring_library_manifest_digest")
            .filter(|value| typed_sha256(value, "keyring-library-manifest"))
    else {
        return false;
    };
    if exit.get("keyring_daemon_sha256").and_then(Value::as_str) != Some(expected_keyring_digest)
        || exit
            .get("keyring_library_manifest_digest")
            .and_then(Value::as_str)
            != Some(expected_manifest_digest)
    {
        return false;
    }
    let Some(inputs) = exit
        .get("tool_input_identities")
        .filter(|value| {
            exact_json_keys(
                value,
                &[
                    "executable",
                    "verifier_tool",
                    "sandbox_helper",
                    "node_runtime",
                    "rustc",
                    "rustdoc",
                    "keyring_daemon",
                    "keyring_libraries",
                ],
            )
        })
        .and_then(Value::as_object)
    else {
        return false;
    };
    if !wsl_git_tool_seal_matches(
        inputs.get("executable"),
        json_object_string(sandbox, "path"),
        json_object_string(sandbox, "sha256"),
    ) || !wsl_git_tool_seal_matches(
        inputs.get("verifier_tool"),
        json_object_string(verifier, "path"),
        json_object_string(verifier, "sha256"),
    ) || !wsl_git_tool_seal_matches(
        inputs.get("sandbox_helper"),
        json_object_string(sandbox_helper, "path"),
        json_object_string(sandbox_helper, "sha256"),
    ) || !wsl_git_tool_seal_matches(
        inputs.get("keyring_daemon"),
        json_object_string(linux, "keyring_daemon_path"),
        Some(expected_keyring_digest),
    ) {
        return false;
    }
    if role == "NODE" {
        if !wsl_git_tool_seal_matches(
            inputs.get("node_runtime"),
            json_object_string(linux, "node_path"),
            json_object_string(linux, "node_sha256"),
        ) || !inputs.get("rustc").is_some_and(Value::is_null)
            || !inputs.get("rustdoc").is_some_and(Value::is_null)
        {
            return false;
        }
    } else if !inputs.get("node_runtime").is_some_and(Value::is_null) {
        return false;
    } else {
        for (key, identity) in [("rustc", "rustc"), ("rustdoc", "rustdoc")] {
            let Some(expected) = toolchain.get(identity).and_then(Value::as_object) else {
                return false;
            };
            if !wsl_git_tool_seal_matches(
                inputs.get(key),
                json_object_string(expected, "path"),
                json_object_string(expected, "sha256"),
            ) {
                return false;
            }
        }
    }
    let Some(library_root) = json_object_string(linux, "keyring_library_path") else {
        return false;
    };
    let Some(libraries) = inputs.get("keyring_libraries").and_then(Value::as_array) else {
        return false;
    };
    const LIBRARIES: [&str; 2] = ["libgck-1.so.0.0.0", "libgcr-base-3.so.1.0.0"];
    libraries.len() == LIBRARIES.len()
        && libraries
            .iter()
            .zip(LIBRARIES)
            .all(|(identity, manifest_path)| {
                wsl_git_library_seal_matches(identity, library_root, manifest_path)
            })
}

fn wsl_git_library_seal_matches(identity: &Value, library_root: &str, manifest_path: &str) -> bool {
    if !exact_json_keys(
        identity,
        &[
            "manifest_path",
            "path",
            "resolved_path",
            "sha256",
            "device",
            "inode",
            "owner_uid",
            "mode",
            "size",
        ],
    ) {
        return false;
    }
    let Some(object) = identity.as_object() else {
        return false;
    };
    let expected_path = format!("{library_root}/{manifest_path}");
    object.get("manifest_path").and_then(Value::as_str) == Some(manifest_path)
        && object.get("resolved_path").and_then(Value::as_str) == Some(expected_path.as_str())
        && wsl_git_seal_object_matches(
            object,
            Some(expected_path.as_str()),
            object.get("sha256").and_then(Value::as_str),
        )
}

fn wsl_git_tool_seal_matches(
    value: Option<&Value>,
    expected_path: Option<&str>,
    expected_sha256: Option<&str>,
) -> bool {
    let Some(value) = value else {
        return false;
    };
    if !exact_json_keys(
        value,
        &[
            "path",
            "resolved_path",
            "sha256",
            "device",
            "inode",
            "owner_uid",
            "mode",
            "size",
        ],
    ) {
        return false;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    wsl_git_seal_object_matches(object, expected_path, expected_sha256)
}

fn wsl_git_seal_object_matches(
    object: &serde_json::Map<String, Value>,
    expected_path: Option<&str>,
    expected_sha256: Option<&str>,
) -> bool {
    expected_path.is_some_and(wsl_linux_command_path)
        && expected_sha256.is_some_and(plain_sha256)
        && object.get("path").and_then(Value::as_str) == expected_path
        && object.get("sha256").and_then(Value::as_str) == expected_sha256
        && object
            .get("resolved_path")
            .and_then(Value::as_str)
            .is_some_and(wsl_linux_command_path)
        && ["device", "inode"].into_iter().all(|key| {
            object
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                })
        })
        && object.get("owner_uid").and_then(Value::as_u64) == Some(0)
        && object
            .get("mode")
            .and_then(Value::as_u64)
            .is_some_and(|value| value > 0 && value <= 0o7777 && value & 0o022 == 0)
        && object
            .get("size")
            .and_then(Value::as_u64)
            .is_some_and(|value| value > 0)
}

fn compact_wsl_git_result(result: &Value) -> ManagedPortResult<Value> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED");
    let object = result.as_object().ok_or_else(rejected)?;
    if object.get("schema").and_then(Value::as_str)
        == Some("lattice.wsl2-verifier-transport-failure/1.0")
    {
        for key in [
            "status",
            "outcome",
            "retryable",
            "task_ref",
            "attempt",
            "worktree_ref",
            "role",
            "execution_environment_ref",
            "repository_head",
            "credential_seal_digest",
            "verifier_identity",
            "unit",
            "process_fence",
            "continuation",
            "transport_evidence",
            "outer_cleanup",
            "outer_post_exit",
            "provider_effect_count",
            "invocation_digest",
            "result_digest",
        ] {
            if !object.contains_key(key) {
                return Err(rejected());
            }
        }
        return Ok(serde_json::json!({
            "schema": "lattice.wsl2-git-operation-receipt/1.0",
            "result_schema": object["schema"],
            "status": object["status"],
            "outcome": object["outcome"],
            "retryable": object["retryable"],
            "task_ref": object["task_ref"],
            "attempt": object["attempt"],
            "worktree_ref": object["worktree_ref"],
            "role": object["role"],
            "execution_environment_ref": object["execution_environment_ref"],
            "repository_head": object["repository_head"],
            "credential_seal_digest": object["credential_seal_digest"],
            "verifier_identity": object["verifier_identity"],
            "unit": object["unit"],
            "process_fence": object["process_fence"],
            "continuation": object["continuation"],
            "transport_evidence": object["transport_evidence"],
            "outer_cleanup": object["outer_cleanup"],
            "outer_post_exit": object["outer_post_exit"],
            "provider_effect_count": object["provider_effect_count"],
            "invocation_digest": object["invocation_digest"],
            "result_digest": object["result_digest"],
        }));
    }
    let output = object
        .get("output")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    for key in [
        "schema",
        "status",
        "outcome",
        "task_ref",
        "attempt",
        "worktree_ref",
        "role",
        "repository_head",
        "verifier_identity",
        "process_marker",
        "exit_receipt",
        "outer_cleanup",
        "outer_post_exit",
        "provider_effect_count",
        "invocation_digest",
        "result_digest",
    ] {
        if !object.contains_key(key) {
            return Err(rejected());
        }
    }
    for key in [
        "stdout_observed_bytes",
        "stderr_observed_bytes",
        "stdout_sha256",
        "stderr_sha256",
        "stdout_base64",
    ] {
        if !output.contains_key(key) {
            return Err(rejected());
        }
    }
    Ok(serde_json::json!({
        "schema": "lattice.wsl2-git-operation-receipt/1.0",
        "result_schema": object["schema"],
        "status": object["status"],
        "outcome": object["outcome"],
        "task_ref": object["task_ref"],
        "attempt": object["attempt"],
        "worktree_ref": object["worktree_ref"],
        "role": object["role"],
        "repository_head": object["repository_head"],
        "verifier_identity": object["verifier_identity"],
        "process_marker": object["process_marker"],
        "exit_receipt": object["exit_receipt"],
        "outer_cleanup": object["outer_cleanup"],
        "outer_post_exit": object["outer_post_exit"],
        "output": {
            "stdout_observed_bytes": output["stdout_observed_bytes"],
            "stderr_observed_bytes": output["stderr_observed_bytes"],
            "stdout_sha256": output["stdout_sha256"],
            "stderr_sha256": output["stderr_sha256"],
            "stdout_payload_retained": false,
        },
        "provider_effect_count": object["provider_effect_count"],
        "invocation_digest": object["invocation_digest"],
        "result_digest": object["result_digest"],
    }))
}

fn exact_json_keys(value: &Value, expected: &[&str]) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn json_object_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

fn plain_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn typed_sha256(value: &str, domain: &str) -> bool {
    value
        .strip_prefix(&format!("{domain}:sha256:"))
        .is_some_and(plain_sha256)
}

fn typed_json_sha256(domain: &str, value: &Value) -> ManagedPortResult<String> {
    Ok(format!(
        "{domain}:sha256:{}",
        sha256_bytes(canonical_json_value(value)?.as_bytes())?.as_str()
    ))
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn wsl_git_command_digest(
    descriptor: &Value,
    execution_environment_ref: &str,
    linux_cwd: &str,
    context: &WslGitPreflightContext,
    attempt: u64,
    args: &[String],
    environment: &Value,
    invocation_digest: &str,
    process_fence: &str,
) -> ManagedPortResult<String> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED");
    let linux = descriptor
        .get("linux")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let toolchain = descriptor
        .get("verification_toolchain")
        .and_then(Value::as_object)
        .ok_or_else(rejected)?;
    let git_path = json_object_string(linux, "git_path").ok_or_else(rejected)?;
    let git_version = json_object_string(linux, "git_version")
        .filter(|value| !value.is_empty())
        .ok_or_else(rejected)?;
    let git_sha256 = json_object_string(linux, "git_sha256")
        .filter(|value| plain_sha256(value))
        .ok_or_else(rejected)?;
    let codex_home = json_object_string(linux, "codex_home")
        .filter(|value| wsl_linux_command_path(value))
        .ok_or_else(rejected)?;
    let task_root = json_object_string(toolchain, "task_root")
        .filter(|value| wsl_linux_command_path(value))
        .ok_or_else(rejected)?;
    let user_runtime_dir = descriptor
        .pointer("/process_fence/user_runtime_dir")
        .and_then(Value::as_str)
        .filter(|value| wsl_linux_command_path(value))
        .ok_or_else(rejected)?;
    if !wsl_linux_command_path(git_path)
        || !wsl_linux_command_path(linux_cwd)
        || !typed_sha256(invocation_digest, "wsl2-git-invocation")
        || !plain_sha256(process_fence)
        || !(1..=100).contains(&attempt)
    {
        return Err(rejected());
    }
    let sandbox = toolchain
        .get("sandbox")
        .filter(|value| exact_json_keys(value, &["path", "version", "sha256"]))
        .cloned()
        .ok_or_else(rejected)?;
    let sandbox_object = sandbox.as_object().ok_or_else(rejected)?;
    if json_object_string(sandbox_object, "path").is_none_or(|value| !wsl_linux_command_path(value))
        || json_object_string(sandbox_object, "version").is_none_or(str::is_empty)
        || json_object_string(sandbox_object, "sha256").is_none_or(|value| !plain_sha256(value))
    {
        return Err(rejected());
    }
    const BOOTSTRAP_ENVIRONMENT_KEYS: &[&str] = &[
        "HOME",
        "TMPDIR",
        "GIT_CONFIG_GLOBAL",
        "NO_COLOR",
        "CI",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_COUNT",
        "GIT_TERMINAL_PROMPT",
        "GIT_OPTIONAL_LOCKS",
        "GIT_ATTR_NOSYSTEM",
    ];
    const GUARDED_ENVIRONMENT_KEYS: &[&str] = &[
        "HOME",
        "TMPDIR",
        "GIT_CONFIG_GLOBAL",
        "GIT_WORK_TREE",
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_INDEX_FILE",
        "NO_COLOR",
        "CI",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_COUNT",
        "GIT_TERMINAL_PROMPT",
        "GIT_OPTIONAL_LOCKS",
        "GIT_ATTR_NOSYSTEM",
    ];
    const GUARDED_IDENTITY_ENVIRONMENT_KEYS: &[&str] = &[
        "HOME",
        "TMPDIR",
        "GIT_CONFIG_GLOBAL",
        "GIT_WORK_TREE",
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_INDEX_FILE",
        "NO_COLOR",
        "CI",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_CONFIG_COUNT",
        "GIT_TERMINAL_PROMPT",
        "GIT_OPTIONAL_LOCKS",
        "GIT_ATTR_NOSYSTEM",
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_AUTHOR_DATE",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "GIT_COMMITTER_DATE",
    ];
    let bootstrap_phase = exact_json_keys(environment, BOOTSTRAP_ENVIRONMENT_KEYS);
    let guarded_phase = exact_json_keys(environment, GUARDED_ENVIRONMENT_KEYS)
        || exact_json_keys(environment, GUARDED_IDENTITY_ENVIRONMENT_KEYS);
    if !bootstrap_phase && !guarded_phase {
        return Err(rejected());
    }
    let environment = environment.as_object().ok_or_else(rejected)?;
    let environment_string = |key: &str| environment.get(key).and_then(Value::as_str);
    let git_command = args.get(11).map(String::as_str).ok_or_else(rejected)?;
    let object_write = matches!(git_command, "hash-object" | "commit-tree");
    let index_write = matches!(git_command, "read-tree" | "update-index" | "write-tree");
    let mut writable_roots = Vec::with_capacity(4);
    for path in [environment_string("HOME"), environment_string("TMPDIR")] {
        let path = path
            .filter(|value| wsl_linux_command_path(value))
            .ok_or_else(rejected)?;
        if !writable_roots.iter().any(|existing| existing == path) {
            writable_roots.push(path.to_owned());
        }
    }
    if guarded_phase && object_write {
        let object_directory = environment_string("GIT_OBJECT_DIRECTORY")
            .filter(|value| wsl_linux_command_path(value))
            .ok_or_else(rejected)?;
        if !writable_roots
            .iter()
            .any(|existing| existing == object_directory)
        {
            writable_roots.push(object_directory.to_owned());
        }
    }
    if guarded_phase && index_write {
        let index_path = environment_string("GIT_INDEX_FILE")
            .filter(|value| wsl_linux_command_path(value))
            .ok_or_else(rejected)?;
        let index_parent = wsl_posix_dirname(index_path).ok_or_else(rejected)?;
        if !writable_roots
            .iter()
            .any(|existing| existing == index_parent)
        {
            writable_roots.push(index_parent.to_owned());
        }
    }
    let linux_home = wsl_linux_home_from_task_root(task_root).ok_or_else(rejected)?;
    let mut denied_roots = Vec::with_capacity(4);
    for path in [
        codex_home.to_owned(),
        format!("{linux_home}/.codex"),
        "/mnt".to_owned(),
        user_runtime_dir.to_owned(),
    ] {
        if !wsl_linux_command_path(&path) {
            return Err(rejected());
        }
        if !denied_roots.iter().any(|existing| existing == &path) {
            denied_roots.push(path);
        }
    }
    if writable_roots
        .iter()
        .any(|path| denied_roots.iter().any(|denied| denied == path))
    {
        return Err(rejected());
    }
    let mut entries = vec![
        serde_json::json!({
            "path": { "type": "special", "value": { "kind": "minimal" } },
            "access": "read",
        }),
        serde_json::json!({
            "path": { "type": "path", "path": task_root },
            "access": "read",
        }),
    ];
    entries.extend(writable_roots.into_iter().map(|path| {
        serde_json::json!({
            "path": { "type": "path", "path": path },
            "access": "write",
        })
    }));
    entries.extend(denied_roots.into_iter().map(|path| {
        serde_json::json!({
            "path": { "type": "path", "path": path },
            "access": "deny",
        })
    }));
    let sandbox_state = serde_json::json!({
        "permissionProfile": {
            "type": "managed",
            "file_system": {
                "type": "restricted",
                "entries": entries,
            },
            "network": "restricted",
        },
        "codexLinuxSandboxExe": Value::Null,
        "sandboxCwd": wsl_linux_file_uri(linux_cwd)?,
        "useLegacyLandlock": false,
    });
    let mut environment_keys = environment.keys().collect::<Vec<_>>();
    environment_keys.sort_unstable();
    let mut sandbox_environment = Vec::with_capacity(environment_keys.len() + 3);
    for key in environment_keys {
        let value = environment
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(rejected)?;
        sandbox_environment.push(format!("{key}={value}"));
    }
    sandbox_environment.extend([
        "PATH=/usr/bin:/bin".to_owned(),
        "LANG=C.UTF-8".to_owned(),
        "LC_ALL=C.UTF-8".to_owned(),
    ]);
    let service_unit = format!(
        "{}-git-{}.service",
        context.unit_prefix,
        &process_fence[..12]
    );
    let retry_of = context
        .retry_of
        .as_ref()
        .map_or(Value::Null, |value| Value::String(value.clone()));
    let reconnect_of = context
        .reconnect_of
        .as_ref()
        .map_or(Value::Null, |value| Value::String(value.clone()));
    typed_json_sha256(
        "wsl2-verifier-command",
        &serde_json::json!({
            "role": "GIT",
            "executable": {
                "path": git_path,
                "version": git_version,
                "sha256": git_sha256,
            },
            "sandbox": sandbox,
            "sandbox_state": sandbox_state,
            "sandbox_environment": sandbox_environment,
            "args": args,
            "cwd": linux_cwd,
            "process_fence": process_fence,
            "service_unit": service_unit,
            "execution_environment_ref": execution_environment_ref,
            "credential_seal_digest": context.credential_seal_digest,
            "supervisor_bootstrap_sha256": WSL2_SUPERVISOR_BOOTSTRAP_SHA256,
            "timeout_ms": context.timeout_ms,
            "stdout_limit_bytes": context.stdout_limit_bytes,
            "stderr_limit_bytes": context.stderr_limit_bytes,
            "attempt": attempt,
            "retry_of": retry_of,
            "reconnect_of": reconnect_of,
            "git_invocation_digest": invocation_digest,
        }),
    )
}

fn wsl_linux_home_from_task_root(task_root: &str) -> Option<String> {
    if !wsl_linux_command_path(task_root) {
        return None;
    }
    let mut components = task_root.split('/');
    if components.next() != Some("") || components.next() != Some("home") {
        return None;
    }
    let user = components.next().filter(|value| !value.is_empty())?;
    Some(format!("/home/{user}"))
}

fn wsl_linux_command_path(value: &str) -> bool {
    if value == "/" {
        return true;
    }
    value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains(['\\', '\0'])
        && value
            .split('/')
            .skip(1)
            .all(|component| !component.is_empty() && component != "." && component != "..")
}

fn wsl_posix_dirname(value: &str) -> Option<&str> {
    if !wsl_linux_command_path(value) || value == "/" {
        return None;
    }
    value
        .rfind('/')
        .map(|index| if index == 0 { "/" } else { &value[..index] })
}

fn wsl_linux_file_uri(value: &str) -> ManagedPortResult<String> {
    if !wsl_linux_command_path(value) {
        return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len() + "file://".len());
    encoded.push_str("file://");
    for byte in value.bytes() {
        if byte == b'/'
            || byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    Ok(encoded)
}

fn wsl_git_process_fence(
    preflight_fence: &str,
    invocation_digest: &str,
    sequence: u64,
) -> ManagedPortResult<String> {
    if !plain_sha256(preflight_fence)
        || !typed_sha256(invocation_digest, "wsl2-git-invocation")
        || !(1..=MAX_WSL_GIT_INVOCATIONS).contains(&sequence)
    {
        return Err(known("LATTICE_MANAGED_VERIFIER_GIT_FENCE_REJECTED"));
    }
    Ok(
        sha256_bytes(format!("{preflight_fence}\n{invocation_digest}\n{sequence}").as_bytes())?
            .as_str()
            .to_owned(),
    )
}

fn single_line_json(bytes: &[u8]) -> ManagedPortResult<Value> {
    let output = std::str::from_utf8(bytes)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))?;
    if !output.ends_with('\n') || output.contains('\r') || output[..output.len() - 1].contains('\n')
    {
        return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
    }
    serde_json::from_str(&output[..output.len() - 1])
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        encoded.push(char::from(TABLE[usize::from(first >> 2)]));
        encoded.push(char::from(
            TABLE[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        encoded.push(if chunk.len() >= 2 {
            char::from(TABLE[usize::from(((second & 0x0f) << 2) | (third >> 6))])
        } else {
            '='
        });
        encoded.push(if chunk.len() == 3 {
            char::from(TABLE[usize::from(third & 0x3f)])
        } else {
            '='
        });
    }
    encoded
}

fn base64_decode(encoded: &str) -> ManagedPortResult<Vec<u8>> {
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    if !encoded.len().is_multiple_of(4) || encoded.len() > MAX_WSL_GIT_RESULT_BYTES * 2 {
        return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
    }
    let mut decoded = Vec::with_capacity(encoded.len() / 4 * 3);
    for (index, chunk) in encoded.as_bytes().chunks_exact(4).enumerate() {
        let last = index + 1 == encoded.len() / 4;
        let first = value(chunk[0]);
        let second = value(chunk[1]);
        let third = (chunk[2] != b'=').then(|| value(chunk[2])).flatten();
        let fourth = (chunk[3] != b'=').then(|| value(chunk[3])).flatten();
        if first.is_none()
            || second.is_none()
            || (!last && (third.is_none() || fourth.is_none()))
            || (chunk[2] == b'=' && chunk[3] != b'=')
            || (chunk[2] == b'=' && !last)
            || (chunk[3] == b'=' && !last)
            || (chunk[2] != b'=' && third.is_none())
            || (chunk[3] != b'=' && fourth.is_none())
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
        }
        let first = first.unwrap_or_default();
        let second = second.unwrap_or_default();
        let third = third.unwrap_or_default();
        let fourth = fourth.unwrap_or_default();
        decoded.push((first << 2) | (second >> 4));
        if chunk[2] != b'=' {
            decoded.push((second << 4) | (third >> 2));
        }
        if chunk[3] != b'=' {
            decoded.push((third << 6) | fourth);
        }
    }
    if base64_encode(&decoded) != encoded || decoded.len() > MAX_GIT_OUTPUT_BYTES {
        return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"));
    }
    Ok(decoded)
}

fn exit_status_from_code(code: u32) -> ExitStatus {
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt as _;
        ExitStatus::from_raw(code)
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        ExitStatus::from_raw(i32::try_from(code).unwrap_or(255) << 8)
    }
}

fn wsl_unc_text_to_linux(value: &str, distribution: &str) -> ManagedPortResult<String> {
    let normalized = value.replace('/', "\\");
    let normalized = normalized
        .strip_prefix(r"\\?\UNC\")
        .map_or(normalized.clone(), |tail| format!(r"\\{tail}"));
    let prefix = format!(r"\\wsl.localhost\{distribution}\");
    if !normalized
        .to_lowercase()
        .starts_with(&prefix.to_lowercase())
    {
        return Err(known("LATTICE_MANAGED_WSL_PATH_MAPPING_REJECTED"));
    }
    let relative = &normalized[prefix.len()..];
    if relative.is_empty()
        || relative
            .split('\\')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(known("LATTICE_MANAGED_WSL_PATH_MAPPING_REJECTED"));
    }
    Ok(format!("/{}", relative.replace('\\', "/")))
}

fn wsl_linux_to_unc(path: &Path, distribution: &str) -> ManagedPortResult<PathBuf> {
    let text = path.to_string_lossy().replace('\\', "/");
    if distribution.is_empty()
        || distribution.contains(['\\', '/'])
        || !text.starts_with('/')
        || text.starts_with("/mnt/c/")
        || text
            .split('/')
            .skip(1)
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(known("LATTICE_MANAGED_WSL_PATH_MAPPING_REJECTED"));
    }
    Ok(PathBuf::from(format!(
        r"\\wsl.localhost\{}\{}",
        distribution,
        text.trim_start_matches('/').replace('/', "\\")
    )))
}

#[allow(clippy::too_many_arguments)]
fn finish_process_io(
    child: &mut SupervisedDuplexChild,
    output_receiver: &mpsc::Receiver<ProcessOutputRead>,
    input_receiver: Option<&mpsc::Receiver<ProcessInputWrite>>,
    output_observation: Option<ProcessOutputRead>,
    input_observation: Option<ProcessInputWrite>,
    reader: thread::JoinHandle<()>,
    writer: Option<thread::JoinHandle<()>>,
) -> ManagedPortResult<Vec<u8>> {
    child
        .terminate_and_reap()
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_PROCESS_WAIT_FAILED"))?;
    let deadline = Instant::now()
        .checked_add(PROCESS_PIPE_DRAIN_TIMEOUT)
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_PROCESS_WAIT_FAILED"))?;
    let mut cleanup_error = None;
    let output_observation = match output_observation {
        Some(observation) => observation,
        None => {
            if let Ok(observation) = output_receiver.recv_timeout(
                deadline
                    .checked_duration_since(Instant::now())
                    .unwrap_or_default(),
            ) {
                observation
            } else {
                cleanup_error = Some("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED");
                ProcessOutputRead::Failed
            }
        }
    };
    let input_observation = match (input_observation, input_receiver) {
        (Some(observation), _) => observation,
        (None, Some(receiver)) => {
            if let Ok(observation) = receiver.recv_timeout(
                deadline
                    .checked_duration_since(Instant::now())
                    .unwrap_or_default(),
            ) {
                observation
            } else {
                cleanup_error = Some("LATTICE_MANAGED_VERIFIER_PROCESS_STDIN_FAILED");
                ProcessInputWrite::Failed
            }
        }
        (None, None) => ProcessInputWrite::Written,
    };
    while (!reader.is_finished() || writer.as_ref().is_some_and(|writer| !writer.is_finished()))
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(1));
    }
    if reader.is_finished() {
        if reader.join().is_err() {
            cleanup_error = Some("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED");
        }
    } else {
        cleanup_error = Some("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED");
    }
    if let Some(writer) = writer {
        if writer.is_finished() {
            if writer.join().is_err() {
                cleanup_error = Some("LATTICE_MANAGED_VERIFIER_PROCESS_STDIN_FAILED");
            }
        } else {
            cleanup_error = Some("LATTICE_MANAGED_VERIFIER_PROCESS_STDIN_FAILED");
        }
    }
    if let Some(code) = cleanup_error {
        return Err(known(code));
    }
    if matches!(input_observation, ProcessInputWrite::Failed) {
        return Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_STDIN_FAILED"));
    }
    match output_observation {
        ProcessOutputRead::Captured(bytes) => Ok(bytes),
        ProcessOutputRead::Drained => Ok(Vec::new()),
        ProcessOutputRead::Limit => Err(known("LATTICE_MANAGED_VERIFIER_OUTPUT_LIMIT")),
        ProcessOutputRead::Failed => Err(known("LATTICE_MANAGED_VERIFIER_PROCESS_OUTPUT_FAILED")),
    }
}

fn create_control_directory() -> ManagedPortResult<(PathBuf, PathBuf, PathBuf)> {
    let temp_path = env::temp_dir();
    if !temp_path.is_absolute() {
        return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"));
    }
    create_control_directory_under(&temp_path)
}

fn wsl_git_control_root_identity(
    descriptor: &ExecutionEnvironmentDescriptor,
    preflight: &VerifiedManagedEvidence,
) -> ManagedPortResult<(String, String)> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED");
    let descriptor_value: Value =
        serde_json::from_str(descriptor.as_json()).map_err(|_| rejected())?;
    let receipt: Value = serde_json::from_slice(preflight.bytes()).map_err(|_| rejected())?;
    let isolation_root = descriptor_value
        .pointer("/verification_toolchain/isolation_root")
        .and_then(Value::as_str)
        .filter(|value| wsl_linux_command_path(value))
        .ok_or_else(rejected)?;
    let worktree_ref = receipt
        .get("worktree_ref")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "worktree"))
        .ok_or_else(rejected)?;
    let preflight_receipt_ref = receipt
        .get("receipt_digest")
        .and_then(Value::as_str)
        .filter(|value| typed_sha256(value, "wsl2-preflight"))
        .ok_or_else(rejected)?;
    if receipt.get("task_ref").and_then(Value::as_str) != Some(preflight.task_ref().as_str())
        || receipt.get("attempt").and_then(Value::as_u64) != Some(u64::from(preflight.attempt()))
        || receipt
            .get("execution_environment_ref")
            .and_then(Value::as_str)
            != Some(descriptor.environment_ref().as_str())
        || receipt.get("repository_head").and_then(Value::as_str)
            != Some(descriptor.repository_head())
    {
        return Err(rejected());
    }
    let binding = serde_json::json!({
        "schema": "lattice.wsl2-git-control-root/1.0",
        "task_ref": preflight.task_ref().as_str(),
        "attempt": preflight.attempt(),
        "worktree_ref": worktree_ref,
        "execution_environment_ref": descriptor.environment_ref().as_str(),
        "preflight_receipt_ref": preflight_receipt_ref,
        "repository_head": descriptor.repository_head(),
        "isolation_root": isolation_root,
    });
    let locator_key = sha256_bytes(canonical_json_value(&binding)?.as_bytes())?;
    let locator = format!(
        "{isolation_root}/git-control/attempt-{}-{}",
        preflight.attempt(),
        locator_key.as_str()
    );
    let mut subject = binding;
    subject
        .as_object_mut()
        .ok_or_else(rejected)?
        .insert("locator".to_owned(), Value::String(locator.clone()));
    let identity_ref = typed_json_sha256("wsl2-git-control-root", &subject)?;
    Ok((locator, identity_ref))
}

fn create_wsl_control_directory(
    descriptor: &ExecutionEnvironmentDescriptor,
    preflight: &VerifiedManagedEvidence,
) -> ManagedPortResult<(PathBuf, PathBuf, PathBuf)> {
    let rejected = || known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED");
    let (linux_locator, _identity_ref) = wsl_git_control_root_identity(descriptor, preflight)?;
    let directory = wsl_linux_to_unc(Path::new(&linux_locator), descriptor.distribution())?;
    let parent = directory.parent().ok_or_else(rejected)?;
    match fs::create_dir(parent) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            canonical_directory(parent)?;
        }
        Err(_) => return Err(rejected()),
    }
    fs::create_dir(&directory).map_err(|_| rejected())?;
    let canonical = canonical_directory(&directory)?;
    if !same_path(&canonical, &directory)
        || canonical.parent() != Some(canonical_directory(parent)?.as_path())
    {
        return Err(rejected());
    }
    let git_home = directory.join("git-home");
    let git_temp = directory.join("git-temp");
    let hooks = directory.join("empty-hooks");
    for child in [&git_home, &git_temp, &hooks] {
        fs::create_dir(child).map_err(|_| rejected())?;
    }
    let config = directory.join("empty-global.gitconfig");
    File::create(&config).map_err(|_| rejected())?;
    Ok((directory, hooks, config))
}

fn create_control_directory_under(root: &Path) -> ManagedPortResult<(PathBuf, PathBuf, PathBuf)> {
    if !root.is_absolute() {
        return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"));
    }
    let canonical_root = canonical_directory(root)?;
    let directory = root.join(format!(
        "lattice-managed-verifier-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
    let canonical_directory = canonical_directory(&directory)?;
    if canonical_directory.parent() != Some(canonical_root.as_path()) {
        return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"));
    }
    // Git for Windows does not consistently accept the `\\?\` verbatim
    // prefix returned by `std::fs::canonicalize` in config and index
    // environment values.  The ordinary absolute path remains safe because
    // the canonical containment check above binds it to the system temp root.
    let hooks = directory.join("empty-hooks");
    fs::create_dir(&hooks).map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
    let config = directory.join("empty-global.gitconfig");
    File::create(&config).map_err(|_| known("LATTICE_MANAGED_VERIFIER_CONTROL_FAILED"))?;
    Ok((directory, hooks, config))
}

fn canonical_directory(path: &Path) -> ManagedPortResult<PathBuf> {
    assert_path_components_safe(path, "LATTICE_MANAGED_VERIFIER_PATH_REJECTED")?;
    let canonical =
        fs::canonicalize(path).map_err(|_| known("LATTICE_MANAGED_VERIFIER_PATH_REJECTED"))?;
    if !canonical.is_absolute()
        || !fs::metadata(&canonical)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_PATH_REJECTED"))?
            .is_dir()
        || !same_path(&canonical, path)
    {
        return Err(known("LATTICE_MANAGED_VERIFIER_PATH_REJECTED"));
    }
    Ok(canonical)
}

fn canonical_file(path: &Path) -> ManagedPortResult<PathBuf> {
    assert_path_components_safe(path, "LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED")?;
    let canonical = fs::canonicalize(path)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
    let metadata = fs::symlink_metadata(&canonical)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
    if !canonical.is_absolute()
        || !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || unsafe_file_type(&metadata)
        || !same_path(&canonical, path)
    {
        return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"));
    }
    Ok(canonical)
}

fn capture_path_anchor(path: &Path, code: &'static str) -> ManagedPortResult<TrustedPathAnchor> {
    assert_path_components_safe(path, code)?;
    let canonical = fs::canonicalize(path).map_err(|_| known(code))?;
    let metadata = fs::symlink_metadata(path).map_err(|_| known(code))?;
    if !metadata.file_type().is_dir() || unsafe_file_type(&metadata) || !same_path(path, &canonical)
    {
        return Err(known(code));
    }
    let file_identity = trusted_path_identity(path).ok_or_else(|| known(code))?;
    Ok(TrustedPathAnchor {
        path: canonical.clone(),
        canonical_path_digest: sha256_bytes(comparable_path(&canonical).as_bytes())?,
        file_identity,
    })
}

fn capture_ancestor_absence_guard(
    repository: &Path,
    kind: TrustedCheckKind,
    code: &'static str,
) -> ManagedPortResult<TrustedAmbientGuard> {
    let (profile, control_directory, forbidden_names): (&'static str, &str, &[&str]) = match kind {
        TrustedCheckKind::NpmVerify => (
            "npm-ancestor-node-modules-bin-absence-v1",
            "node_modules",
            &[".bin"],
        ),
        TrustedCheckKind::CargoTest => (
            "cargo-ancestor-config-absence-v1",
            ".cargo",
            &["config", "config.toml"],
        ),
    };
    let canonical_repository = fs::canonicalize(repository).map_err(|_| known(code))?;
    if !same_path(repository, &canonical_repository) {
        return Err(known(code));
    }

    let mut hasher = Sha256::new();
    hasher.update(profile.as_bytes());
    let mut ancestor_count = 0usize;
    for ancestor in canonical_repository.ancestors().skip(1) {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        ancestor_count = ancestor_count.checked_add(1).ok_or_else(|| known(code))?;
        if ancestor_count > MAX_ANCESTOR_DIRECTORIES {
            return Err(known(code));
        }
        let anchor = capture_path_anchor(ancestor, code)?;
        hasher.update(anchor.canonical_path_digest.as_str().as_bytes());
        update_file_identity_hash(&mut hasher, &anchor.file_identity);

        let control_root = ancestor.join(control_directory);
        match fs::symlink_metadata(&control_root) {
            Ok(metadata) => {
                if unsafe_file_type(&metadata) || !metadata.file_type().is_dir() {
                    return Err(known(code));
                }
                let control_anchor = capture_path_anchor(&control_root, code)?;
                hasher.update(b"present");
                hasher.update(control_anchor.canonical_path_digest.as_str().as_bytes());
                update_file_identity_hash(&mut hasher, &control_anchor.file_identity);
                for name in forbidden_names {
                    let forbidden = control_root.join(name);
                    match fs::symlink_metadata(&forbidden) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            hasher.update(name.as_bytes());
                            hasher.update(b"absent");
                        }
                        _ => return Err(known(code)),
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(control_directory.as_bytes());
                hasher.update(b"absent");
            }
            Err(_) => return Err(known(code)),
        }
        hasher.update([0xff]);
    }
    if ancestor_count == 0 {
        return Err(known(code));
    }
    if kind == TrustedCheckKind::CargoTest {
        let cargo_home = source_cargo_home().map_err(|_| known(code))?;
        let cargo_home_anchor = capture_path_anchor(&cargo_home, code)?;
        hasher.update(b"source-cargo-home\0");
        hasher.update(cargo_home_anchor.canonical_path_digest.as_str().as_bytes());
        update_file_identity_hash(&mut hasher, &cargo_home_anchor.file_identity);
        for name in ["config", "config.toml"] {
            match fs::symlink_metadata(cargo_home.join(name)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    hasher.update(name.as_bytes());
                    hasher.update(b"absent");
                }
                _ => return Err(known(code)),
            }
        }
        hasher.update([0xff]);
    }
    Ok(TrustedAmbientGuard {
        profile,
        digest: digest_from_sha256(hasher.finalize().as_slice())?,
    })
}

fn capture_git_entry(path: &Path, code: &'static str) -> ManagedPortResult<TrustedGitEntry> {
    let metadata = fs::symlink_metadata(path).map_err(|_| known(code))?;
    if unsafe_file_type(&metadata) {
        return Err(known(code));
    }
    if metadata.file_type().is_dir() {
        return capture_path_anchor(path, code).map(TrustedGitEntry::Directory);
    }
    if metadata.file_type().is_file() {
        return capture_file_facts(path, MAX_GITFILE_BYTES, code).map(|facts| {
            TrustedGitEntry::File {
                path: path.to_path_buf(),
                facts,
            }
        });
    }
    Err(known(code))
}

fn safe_repository_file(repository: &Path, relative: &str) -> ManagedPortResult<PathBuf> {
    if !valid_relative_path(relative) || forbidden_git_path(relative) {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    assert_path_components_safe(repository, "LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED")?;
    let current_root = fs::canonicalize(repository)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
    if !same_path(repository, &current_root) {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    let mut candidate = repository.to_path_buf();
    let components = Path::new(relative).components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        };
        candidate.push(component);
        let metadata = fs::symlink_metadata(&candidate)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
        if unsafe_file_type(&metadata)
            || (index + 1 == components.len() && !metadata.file_type().is_file())
            || (index + 1 < components.len() && !metadata.file_type().is_dir())
        {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
    }
    let canonical = fs::canonicalize(&candidate)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
    if !same_path(&canonical, &candidate) || !path_is_contained(repository, &canonical) {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    Ok(canonical)
}

fn assert_path_components_safe(path: &Path, code: &'static str) -> ManagedPortResult<()> {
    if !path.is_absolute() {
        return Err(known(code));
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if matches!(component, Component::Prefix(_) | Component::RootDir) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current).map_err(|_| known(code))?;
        if unsafe_file_type(&metadata) {
            return Err(known(code));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn unsafe_file_type(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn unsafe_file_type(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn comparable_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    let value = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned()
    };
    value.trim_end_matches('\\').to_lowercase()
}

#[cfg(not(windows))]
fn comparable_path(path: &Path) -> String {
    path.to_string_lossy().trim_end_matches('/').to_owned()
}

fn same_path(left: &Path, right: &Path) -> bool {
    comparable_path(left) == comparable_path(right)
}

fn path_is_contained(root: &Path, candidate: &Path) -> bool {
    let root = comparable_path(root);
    let candidate = comparable_path(candidate);
    candidate == root
        || candidate
            .strip_prefix(&root)
            .is_some_and(|tail| tail.starts_with(['/', '\\']))
}

fn capture_trusted_executable(
    role: &'static str,
    path: &Path,
) -> ManagedPortResult<TrustedExecutable> {
    let canonical = canonical_file(path)?;
    let facts = capture_file_facts(
        &canonical,
        MAX_TRUSTED_EXECUTABLE_BYTES,
        "LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED",
    )?;
    Ok(TrustedExecutable {
        role,
        path: canonical,
        canonical_path_digest: facts.canonical_path_digest,
        content_digest: facts.content_digest,
        byte_len: facts.byte_len,
        file_identity: facts.file_identity,
    })
}

fn capture_trusted_executable_with_guards(
    role: &'static str,
    path: &Path,
    guards: &[Option<&ManagedEffectBundleGuard>],
) -> ManagedPortResult<(TrustedExecutable, bool)> {
    for guard in guards.iter().flatten() {
        let snapshot = guard
            .sealed_file_snapshot(path)
            .map_err(|()| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
        let Some(snapshot) = snapshot else {
            continue;
        };
        let canonical = canonical_file(path)?;
        if !same_path(&canonical, snapshot.canonical_path()) {
            return Err(known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"));
        }
        #[cfg(windows)]
        let namespace = "windows-volume-file-index-v1";
        #[cfg(unix)]
        let namespace = "unix-device-inode-v1";
        return Ok((
            TrustedExecutable {
                role,
                path: canonical.clone(),
                canonical_path_digest: sha256_bytes(comparable_path(&canonical).as_bytes())?,
                content_digest: snapshot.content_digest().clone(),
                byte_len: snapshot.length(),
                file_identity: TrustedFileIdentity {
                    namespace,
                    volume_or_device: snapshot.volume_or_device(),
                    file: snapshot.file(),
                },
            },
            true,
        ));
    }
    capture_trusted_executable(role, path).map(|identity| (identity, false))
}

fn capture_file_facts(
    path: &Path,
    max_bytes: u64,
    code: &'static str,
) -> ManagedPortResult<CapturedFileFacts> {
    let metadata = fs::symlink_metadata(path).map_err(|_| known(code))?;
    if unsafe_file_type(&metadata) || !metadata.file_type().is_file() || metadata.len() > max_bytes
    {
        return Err(known(code));
    }
    let canonical = fs::canonicalize(path).map_err(|_| known(code))?;
    if !same_path(&canonical, path) {
        return Err(known(code));
    }
    let path_identity = trusted_path_identity(path).ok_or_else(|| known(code))?;
    let mut file = File::open(path).map_err(|_| known(code))?;
    let opened = file.metadata().map_err(|_| known(code))?;
    if opened.len() != metadata.len() || opened.len() > max_bytes {
        return Err(known(code));
    }
    let identity = trusted_file_identity(&file).ok_or_else(|| known(code))?;
    if identity != path_identity {
        return Err(known(code));
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1_024].into_boxed_slice();
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer).map_err(|_| known(code))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).map_err(|_| known(code))?)
            .ok_or_else(|| known(code))?;
        if total > max_bytes {
            return Err(known(code));
        }
        hasher.update(&buffer[..read]);
    }
    let after = file.metadata().map_err(|_| known(code))?;
    if total != opened.len()
        || after.len() != opened.len()
        || trusted_file_identity(&file).as_ref() != Some(&identity)
        || trusted_path_identity(path).as_ref() != Some(&identity)
        || fs::canonicalize(path).ok().is_none_or(|after_path| {
            !same_path(&canonical, &after_path) || !same_path(path, &after_path)
        })
    {
        return Err(known(code));
    }
    Ok(CapturedFileFacts {
        canonical_path_digest: sha256_bytes(comparable_path(&canonical).as_bytes())?,
        content_digest: digest_from_sha256(hasher.finalize().as_slice())?,
        byte_len: total,
        file_identity: identity,
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn trusted_file_identity(file: &File) -> Option<TrustedFileIdentity> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &raw mut information) } == 0
    {
        return None;
    }
    Some(TrustedFileIdentity {
        namespace: "windows-volume-file-index-v1",
        volume_or_device: u64::from(information.dwVolumeSerialNumber),
        file: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    })
}

#[cfg(unix)]
fn trusted_file_identity(file: &File) -> Option<TrustedFileIdentity> {
    let metadata = file.metadata().ok()?;
    Some(TrustedFileIdentity {
        namespace: "unix-device-inode-v1",
        volume_or_device: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn trusted_path_identity(path: &Path) -> Option<TrustedFileIdentity> {
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return None;
    }
    let file = unsafe { File::from_raw_handle(handle.cast::<std::ffi::c_void>() as RawHandle) };
    trusted_file_identity(&file)
}

#[cfg(unix)]
fn trusted_path_identity(path: &Path) -> Option<TrustedFileIdentity> {
    let metadata = fs::metadata(path).ok()?;
    Some(TrustedFileIdentity {
        namespace: "unix-device-inode-v1",
        volume_or_device: metadata.dev(),
        file: metadata.ino(),
    })
}

fn resolve_required_program(preferred: Option<&Path>, name: &str) -> ManagedPortResult<PathBuf> {
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    let directory =
        preferred.ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
    canonical_file(&directory.join(file_name))
}

#[cfg(test)]
fn resolve_test_program(name: &str) -> ManagedPortResult<PathBuf> {
    let file_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    env::var_os("PATH")
        .into_iter()
        .flat_map(|path| env::split_paths(&path).collect::<Vec<_>>())
        .find_map(|directory| canonical_file(&directory.join(&file_name)).ok())
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))
}

fn trusted_path(executables: &[TrustedExecutable]) -> ManagedPortResult<OsString> {
    let mut directories: Vec<PathBuf> = Vec::new();
    for executable in executables {
        let parent = executable
            .path
            .parent()
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
        if !directories
            .iter()
            .any(|existing| same_path(existing, parent))
        {
            directories.push(parent.to_path_buf());
        }
    }
    env::join_paths(directories).map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))
}

fn source_cargo_home() -> ManagedPortResult<PathBuf> {
    let path = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from)
                .map(|home| home.join(".cargo"))
        })
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CARGO_OFFLINE_CACHE_UNAVAILABLE"))?;
    canonical_directory(&path)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_OFFLINE_CACHE_UNAVAILABLE"))
}

fn source_rustup_home() -> ManagedPortResult<PathBuf> {
    let path = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .map(PathBuf::from)
                .map(|home| home.join(".rustup"))
        })
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED"))?;
    canonical_directory(&path)
}

fn capture_bounded_tree(root: &Path) -> ManagedPortResult<TrustedTreeFacts> {
    let root = capture_path_anchor(root, "LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED")?;
    let files = bounded_tree_files(&root.path)?;
    let mut content_hasher = Sha256::new();
    let mut identity_hasher = Sha256::new();
    let mut byte_len = 0u64;
    for file in &files {
        let relative = file
            .strip_prefix(&root.path)
            .ok()
            .and_then(Path::to_str)
            .map(|path| path.replace('\\', "/"))
            .filter(|path| valid_relative_path(path))
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
        let remaining = MAX_CARGO_VENDOR_BYTES
            .checked_sub(byte_len)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_LIMIT"))?;
        let facts = capture_file_facts(
            file,
            MAX_CARGO_VENDOR_FILE_BYTES.min(remaining),
            "LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_LIMIT",
        )?;
        byte_len = byte_len
            .checked_add(facts.byte_len)
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_LIMIT"))?;
        for hasher in [&mut content_hasher, &mut identity_hasher] {
            hasher.update(
                u64::try_from(relative.len())
                    .unwrap_or(u64::MAX)
                    .to_be_bytes(),
            );
            hasher.update(relative.as_bytes());
        }
        content_hasher.update(facts.byte_len.to_be_bytes());
        content_hasher.update(facts.content_digest.as_str().as_bytes());
        update_file_identity_hash(&mut identity_hasher, &facts.file_identity);
    }
    Ok(TrustedTreeFacts {
        root,
        content_digest: digest_from_sha256(content_hasher.finalize().as_slice())?,
        identity_digest: digest_from_sha256(identity_hasher.finalize().as_slice())?,
        file_count: files.len(),
        byte_len,
    })
}

fn bounded_tree_files(root: &Path) -> ManagedPortResult<Vec<PathBuf>> {
    let mut directories = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut entry_count = 0usize;
    while let Some(directory) = directories.pop() {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&directory)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?
        {
            entry_count = entry_count
                .checked_add(1)
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_LIMIT"))?;
            if entry_count > MAX_CARGO_VENDOR_FILES.saturating_mul(2) {
                return Err(known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_LIMIT"));
            }
            entries
                .push(entry.map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?);
        }
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
            if unsafe_file_type(&metadata) {
                return Err(known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"));
            }
            let canonical = fs::canonicalize(&path)
                .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
            if !same_path(&canonical, &path) || !path_is_contained(root, &canonical) {
                return Err(known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"));
            }
            if metadata.file_type().is_dir() {
                directories.push(path);
            } else if metadata.file_type().is_file() {
                files.push(path);
                if files.len() > MAX_CARGO_VENDOR_FILES {
                    return Err(known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_LIMIT"));
                }
            } else {
                return Err(known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"));
            }
        }
    }
    files.sort_by_key(|path| comparable_path(path));
    Ok(files)
}

fn remove_owned_control_directory(path: &Path) {
    let Ok(temp) = canonical_directory(&env::temp_dir()) else {
        return;
    };
    let Ok(control) = canonical_directory(path) else {
        return;
    };
    let owned_prefix = format!("lattice-managed-verifier-{}-", std::process::id());
    if control.parent() != Some(temp.as_path())
        || !control
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(&owned_prefix))
    {
        return;
    }
    clear_owned_readonly_files(&control);
    let _ = fs::remove_dir_all(control);
}

#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn clear_owned_readonly_files(root: &Path) {
    let mut directories = vec![root.to_path_buf()];
    let mut entry_count = 0usize;
    while let Some(directory) = directories.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            return;
        };
        for entry in entries {
            entry_count = entry_count.saturating_add(1);
            if entry_count
                > MAX_CARGO_VENDOR_FILES
                    .saturating_mul(2)
                    .saturating_add(4_096)
            {
                return;
            }
            let Ok(entry) = entry else {
                return;
            };
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            // Never traverse or mutate a replacement link/reparse target.
            if unsafe_file_type(&metadata) {
                continue;
            }
            if metadata.file_type().is_dir() {
                directories.push(path);
            } else if metadata.file_type().is_file() && metadata.permissions().readonly() {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                let _ = fs::set_permissions(path, permissions);
            }
        }
    }
}

#[cfg(not(windows))]
const fn clear_owned_readonly_files(_root: &Path) {}

fn set_bounded_tree_readonly(root: &Path, expected_files: usize) -> ManagedPortResult<()> {
    let files = bounded_tree_files(root)?;
    if files.len() != expected_files {
        return Err(known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_DRIFT"));
    }
    for file in files {
        let mut permissions = fs::metadata(&file)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(file, permissions)
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_CARGO_SOURCE_REJECTED"))?;
    }
    Ok(())
}

fn valid_scope_rule(rule: &str) -> bool {
    let path = rule.strip_suffix("/**").unwrap_or(rule);
    managed_scope_rule_valid(rule)
        && valid_relative_path(path)
        && !forbidden_git_path(path)
        && !managed_protected_control_path(path)
}

fn trusted_rule_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    let file_name = normalized.rsplit('/').next().unwrap_or_default();
    file_name.eq_ignore_ascii_case("AGENTS.md")
        || file_name.eq_ignore_ascii_case("instructions.md")
        || normalized.eq_ignore_ascii_case(".github/copilot-instructions.md")
}

fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0', ':', '*', '?'])
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn forbidden_git_path(path: &str) -> bool {
    path.eq_ignore_ascii_case(".git")
        || path
            .split('/')
            .next()
            .is_some_and(|part| part.eq_ignore_ascii_case(".git"))
}

struct TrustedNpmProfile {
    control_paths: Vec<String>,
    plan: TrustedNodePlan,
}

fn trusted_npm_profile(
    tracked_paths: &[String],
    package: &Value,
) -> ManagedPortResult<TrustedNpmProfile> {
    if tracked_paths.iter().any(|path| npm_shadow_path(path)) {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    let scripts = package
        .get("scripts")
        .and_then(Value::as_object)
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
    let mut controls = tracked_paths
        .iter()
        .filter(|path| npm_static_control_path(path))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut plan = TrustedNodePlan {
        invocations: Vec::new(),
    };
    let mut visiting = BTreeSet::new();
    let mut expanded_scripts = 0_usize;
    expand_npm_lifecycle(
        "verify",
        scripts,
        tracked_paths,
        &mut controls,
        &mut plan,
        &mut visiting,
        &mut expanded_scripts,
    )?;
    if controls.len() > MAX_TRUSTED_CONTROL_FILES {
        return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"));
    }
    if plan.invocations.is_empty() || plan.invocations.len() > MAX_TRUSTED_NPM_SCRIPTS {
        return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"));
    }
    Ok(TrustedNpmProfile {
        control_paths: controls.into_iter().collect(),
        plan,
    })
}

fn expand_npm_lifecycle(
    name: &str,
    scripts: &serde_json::Map<String, Value>,
    tracked_paths: &[String],
    controls: &mut BTreeSet<String>,
    plan: &mut TrustedNodePlan,
    visiting: &mut BTreeSet<String>,
    expanded_scripts: &mut usize,
) -> ManagedPortResult<()> {
    if !safe_npm_script_name(name) || !visiting.insert(name.to_owned()) {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    *expanded_scripts = expanded_scripts
        .checked_add(1)
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"))?;
    if *expanded_scripts > MAX_TRUSTED_NPM_SCRIPTS {
        return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"));
    }
    for lifecycle_name in [
        Some(format!("pre{name}")).filter(|hook| scripts.contains_key(hook)),
        Some(name.to_owned()),
        Some(format!("post{name}")).filter(|hook| scripts.contains_key(hook)),
    ]
    .into_iter()
    .flatten()
    {
        expand_npm_script_body(
            &lifecycle_name,
            scripts,
            tracked_paths,
            controls,
            plan,
            visiting,
            expanded_scripts,
        )?;
    }
    visiting.remove(name);
    Ok(())
}

fn expand_npm_script_body(
    name: &str,
    scripts: &serde_json::Map<String, Value>,
    tracked_paths: &[String],
    controls: &mut BTreeSet<String>,
    plan: &mut TrustedNodePlan,
    visiting: &mut BTreeSet<String>,
    expanded_scripts: &mut usize,
) -> ManagedPortResult<()> {
    let command = scripts
        .get(name)
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
    for segment in shell_segments(command)? {
        let words = shell_words(&segment)?;
        let executable = words
            .first()
            .filter(|word| !word.contains(['/', '\\']))
            .map(|word| word.to_ascii_lowercase())
            .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
        match executable.as_str() {
            "npm" | "npm.cmd" | "npm.exe" => {
                let referenced = if words.len() == 2 && words[1] == "test" {
                    "test"
                } else if words.len() == 3
                    && matches!(words[1].as_str(), "run" | "run-script")
                    && safe_npm_script_name(&words[2])
                {
                    words[2].as_str()
                } else {
                    // Workspace/prefix aliases and option-dependent resolution
                    // are deliberately outside the closed verifier profile.
                    return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
                };
                if !scripts.contains_key(referenced) {
                    return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
                }
                expand_npm_lifecycle(
                    referenced,
                    scripts,
                    tracked_paths,
                    controls,
                    plan,
                    visiting,
                    expanded_scripts,
                )?;
            }
            "node" | "node.exe" => {
                let runners = resolve_node_runners(&words[1..], tracked_paths)?;
                controls.extend(runners);
                plan.invocations.push(TrustedNodeInvocation {
                    script: name.to_owned(),
                    arguments: words[1..].to_vec(),
                });
                if plan.invocations.len() > MAX_TRUSTED_NPM_SCRIPTS {
                    return Err(known("LATTICE_MANAGED_VERIFIER_CONTROL_LIMIT"));
                }
            }
            _ => return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED")),
        }
    }
    Ok(())
}

fn resolve_node_runners(
    arguments: &[String],
    tracked_paths: &[String],
) -> ManagedPortResult<Vec<String>> {
    if arguments.is_empty() {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    let mut runners = Vec::new();
    let mut test_mode = false;
    let mut test_input = false;
    let mut index = 0usize;
    while index < arguments.len() {
        let argument = &arguments[index];
        if matches!(argument.as_str(), "-e" | "--eval") {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
        if argument == "--test" {
            test_mode = true;
            index += 1;
            continue;
        }
        if matches!(
            argument.as_str(),
            "-r" | "--require" | "--import" | "--loader"
        ) {
            let path = arguments
                .get(index + 1)
                .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"))?;
            runners.push(explicit_node_runner(path, tracked_paths)?);
            index += 2;
            continue;
        }
        if let Some(path) = ["--require=", "--import=", "--loader="]
            .iter()
            .find_map(|prefix| argument.strip_prefix(prefix))
        {
            runners.push(explicit_node_runner(path, tracked_paths)?);
            index += 1;
            continue;
        }
        if argument.starts_with('-') {
            if matches!(
                argument.as_str(),
                "--no-warnings" | "--enable-source-maps" | "--test-only"
            ) {
                index += 1;
                continue;
            }
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
        if test_mode {
            // `node --test` inputs are candidate code: they remain bounded by
            // task scope, the exact tree/diff digest and independent review.
            // They are not promoted to immutable verification runners.
            if !safe_candidate_test_input(argument) {
                return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
            }
            test_input = true;
            index += 1;
            continue;
        }
        runners.push(explicit_node_runner(argument, tracked_paths)?);
        break;
    }
    if test_mode && !test_input {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    Ok(runners)
}

fn explicit_node_runner(path: &str, tracked_paths: &[String]) -> ManagedPortResult<String> {
    let path = path.strip_prefix("./").unwrap_or(path);
    let extension = Path::new(path).extension().and_then(|value| value.to_str());
    if !valid_relative_path(path)
        || path.contains(['$', '%', '{', '}', '@'])
        || !matches!(extension, Some("js" | "mjs" | "cjs"))
        || !tracked_paths.iter().any(|tracked| tracked == path)
    {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    Ok(path.to_owned())
}

fn safe_candidate_test_input(path: &str) -> bool {
    let path = path.strip_prefix("./").unwrap_or(path);
    !path.is_empty()
        && !Path::new(path).is_absolute()
        && !path.contains(['\\', '\0', ':', '$', '%', '{', '}', '@'])
        && !path.split('/').any(|part| matches!(part, "" | "." | ".."))
        && path
            .rsplit('.')
            .next()
            .is_some_and(|extension| matches!(extension, "js" | "mjs" | "cjs"))
}

fn safe_npm_script_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.'))
}

fn shell_segments(command: &str) -> ManagedPortResult<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut characters = command.chars().peekable();
    while let Some(character) = characters.next() {
        // Both POSIX shells and cmd.exe have expansion/escape forms that can
        // turn a static-looking package script into a different executable.
        // The closed profile rejects those forms even inside quoted text.
        if matches!(character, '$' | '%' | '!' | '^' | '`') {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            }
            current.push(character);
            continue;
        }
        if quote.is_none() && matches!(character, '&' | '|') {
            // A trusted plan can reproduce a success-ordered `&&` chain. `||`
            // and single shell operators require shell-dependent branching and
            // are outside the exact no-shell profile.
            if character != '&' || characters.next_if_eq(&character).is_none() {
                return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
            }
            if current.trim().is_empty() {
                return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
            }
            segments.push(current.trim().to_owned());
            current.clear();
            continue;
        }
        if quote.is_none() && matches!(character, ';' | '\n' | '\r' | '<' | '>' | '(' | ')') {
            return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
        }
        current.push(character);
    }
    if quote.is_some() || current.trim().is_empty() {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    segments.push(current.trim().to_owned());
    Ok(segments)
}

fn shell_words(segment: &str) -> ManagedPortResult<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    for character in segment.chars() {
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
            } else {
                current.push(character);
            }
        } else if quote.is_none() && character.is_ascii_whitespace() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if quote.is_some() {
        return Err(known("LATTICE_MANAGED_VERIFIER_BASE_POLICY_REJECTED"));
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

fn npm_static_control_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "package.json" | "package-lock.json" | "npm-shrinkwrap.json" | ".npmrc"
    ) || path.starts_with("scripts/")
}

fn npm_shadow_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or_default();
    let bare_shadow = matches!(
        name,
        "node" | "node.exe" | "node.cmd" | "node.bat" | "npm" | "npm.exe" | "npm.cmd" | "npm.bat"
    );
    bare_shadow
        && (!normalized.contains('/')
            || normalized.contains("/node_modules/.bin/")
            || normalized.starts_with("node_modules/.bin/"))
}

fn cargo_control_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "Cargo.toml" | "Cargo.lock" | "build.rs" | "rust-toolchain" | "rust-toolchain.toml"
    ) || path.split('/').any(|component| component == ".cargo")
}

fn cargo_config_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    normalized == ".cargo/config"
        || normalized == ".cargo/config.toml"
        || normalized.ends_with("/.cargo/config")
        || normalized.ends_with("/.cargo/config.toml")
}

fn supported_cargo_config(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let normalized = text.to_ascii_lowercase().replace('_', "-");
    ![
        "rustc-wrapper",
        "rustc-workspace-wrapper",
        "rustc =",
        "rustdoc =",
        "runner =",
        "linker =",
        "rustflags",
        "rustdocflags",
        "[env]",
        "[target.",
        "credential",
    ]
    .iter()
    .any(|forbidden| normalized.contains(forbidden))
}

fn git_local_config_is_closed(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    if text.contains('\0') {
        return false;
    }
    let mut section = None::<String>;
    let mut continued = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if continued {
            continued = unescaped_trailing_backslash(line);
            continue;
        }
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        if line.starts_with('[') {
            let Some(close) = line.find(']') else {
                return false;
            };
            let tail = line[close + 1..].trim_start();
            if !tail.is_empty() && !tail.starts_with(['#', ';']) {
                return false;
            }
            let header = line[1..close].trim();
            let name_end = header
                .find(|character: char| character.is_ascii_whitespace() || character == '"')
                .unwrap_or(header.len());
            let name = header[..name_end]
                .split('.')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return false;
            }
            if matches!(name.as_str(), "include" | "includeif") {
                return false;
            }
            section = Some(name);
            continue;
        }
        let Some(active_section) = section.as_deref() else {
            return false;
        };
        let key_end = line
            .find(|character: char| character.is_ascii_whitespace() || character == '=')
            .unwrap_or(line.len());
        let key = line[..key_end].to_ascii_lowercase();
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return false;
        }
        if (active_section == "filter"
            && matches!(key.as_str(), "clean" | "smudge" | "process" | "required"))
            || (active_section == "diff"
                && matches!(key.as_str(), "command" | "external" | "textconv"))
            || (active_section == "core"
                && matches!(key.as_str(), "attributesfile" | "excludesfile"))
        {
            return false;
        }
        continued = unescaped_trailing_backslash(line);
    }
    !continued
}

fn unescaped_trailing_backslash(value: &str) -> bool {
    value
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count()
        % 2
        == 1
}

fn git_attributes_are_closed(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    if text.contains('\0') {
        return false;
    }
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_ascii_whitespace();
        if fields.next().is_none() {
            return false;
        }
        for field in fields {
            let attribute = field
                .trim_matches('"')
                .trim_start_matches(['!', '-'])
                .split('=')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            if matches!(attribute.as_str(), "filter" | "diff") {
                return false;
            }
        }
    }
    true
}

fn scope_rule_matches(rule: &str, path: &str) -> bool {
    if rule == "**/*" {
        return true;
    }
    if let Some(prefix) = rule.strip_suffix("/**") {
        return path == prefix
            || path
                .strip_prefix(prefix)
                .is_some_and(|tail| tail.starts_with('/'));
    }
    rule == path
}

fn path_text(bytes: &[u8]) -> ManagedPortResult<String> {
    let path =
        std::str::from_utf8(bytes).map_err(|_| known("LATTICE_MANAGED_VERIFIER_PATH_REJECTED"))?;
    if !valid_relative_path(path) || forbidden_git_path(path) {
        return Err(known("LATTICE_MANAGED_VERIFIER_PATH_REJECTED"));
    }
    Ok(path.to_owned())
}

fn path_from_git_stdout(bytes: &[u8]) -> ManagedPortResult<PathBuf> {
    Ok(PathBuf::from(trim_ascii(bytes)?))
}

fn trim_ascii(bytes: &[u8]) -> ManagedPortResult<&str> {
    std::str::from_utf8(bytes)
        .map(str::trim)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_GIT_OUTPUT_REJECTED"))
}

fn oid_from_output(bytes: &[u8]) -> ManagedPortResult<String> {
    let oid = trim_ascii(bytes)?;
    if oid.len() != 40
        || !oid
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(known("LATTICE_MANAGED_VERIFIER_GIT_OBJECT_REJECTED"));
    }
    Ok(oid.to_owned())
}

fn capture_optional_file_facts(
    path: &Path,
    max_bytes: u64,
    code: &'static str,
) -> ManagedPortResult<Option<CapturedFileFacts>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if unsafe_file_type(&metadata)
                || !metadata.file_type().is_file()
                || metadata.len() > max_bytes
            {
                return Err(known(code));
            }
            capture_file_facts(path, max_bytes, code).map(Some)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(known(code)),
    }
}

fn read_bounded_file(
    path: &Path,
    max_bytes: u64,
    code: &'static str,
) -> ManagedPortResult<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|_| known(code))?;
    if unsafe_file_type(&metadata) || !metadata.file_type().is_file() || metadata.len() > max_bytes
    {
        return Err(known(code));
    }
    let canonical = fs::canonicalize(path).map_err(|_| known(code))?;
    if !same_path(&canonical, path) {
        return Err(known(code));
    }
    let expected_identity = trusted_path_identity(path).ok_or_else(|| known(code))?;
    let capacity = usize::try_from(metadata.len()).map_err(|_| known(code))?;
    let mut bytes = Vec::with_capacity(capacity);
    let mut file = File::open(path).map_err(|_| known(code))?;
    if trusted_file_identity(&file).as_ref() != Some(&expected_identity) {
        return Err(known(code));
    }
    std::io::Read::by_ref(&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| known(code))?;
    let after = file.metadata().map_err(|_| known(code))?;
    let after_path = fs::symlink_metadata(path).map_err(|_| known(code))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != metadata.len()
        || after.len() != metadata.len()
        || unsafe_file_type(&after_path)
        || !after_path.file_type().is_file()
        || trusted_file_identity(&file).as_ref() != Some(&expected_identity)
        || trusted_path_identity(path).as_ref() != Some(&expected_identity)
        || fs::canonicalize(path).ok().is_none_or(|after_canonical| {
            !same_path(&canonical, &after_canonical) || !same_path(path, &after_canonical)
        })
    {
        return Err(known(code));
    }
    Ok(bytes)
}

fn contains_secret_material(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    if task_ingress_text_contains_recognized_secret(text.as_ref()) {
        return true;
    }

    // Git candidates may be arbitrary bytes. Preserve a byte-oriented pass so
    // invalid UTF-8 cannot bypass the pre-object credential URL and legacy
    // assignment/private-key checks.
    let folded = bytes.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>();
    let contains = |needle: &[u8]| folded.windows(needle.len()).any(|window| window == needle);
    let assignment = [
        b"password=".as_slice(),
        b"password:".as_slice(),
        b"passwd=".as_slice(),
        b"passwd:".as_slice(),
        b"token=".as_slice(),
        b"token:".as_slice(),
        b"api_key=".as_slice(),
        b"api_key:".as_slice(),
        b"api-key=".as_slice(),
        b"api-key:".as_slice(),
        b"secret=".as_slice(),
        b"secret:".as_slice(),
    ];
    if assignment.iter().any(|needle| contains(needle))
        || (contains(b"-----begin") && contains(b"private key-----"))
    {
        return true;
    }
    // Reject an RFC-style credential URL before it reaches `hash-object -w`.
    folded.windows(3).enumerate().any(|(start, window)| {
        window == b"://"
            && folded[start + 3..]
                .iter()
                .take_while(|byte| !matches!(**byte, b'/' | b'?' | b'#' | b'\n' | b'\r'))
                .any(|byte| *byte == b'@')
    })
}

#[allow(clippy::too_many_lines)]
fn git_control_digest(
    git_directory: &Path,
    common_git_directory: &Path,
) -> ManagedPortResult<ContentDigest> {
    let mut hasher = Sha256::new();
    let mut total_bytes = 0u64;
    let mut file_count = 0usize;
    let roots = if git_directory == common_git_directory {
        vec![("common", common_git_directory)]
    } else {
        vec![
            ("worktree", git_directory),
            ("common", common_git_directory),
        ]
    };
    for (root_name, root) in roots {
        for relative in [
            "HEAD",
            "commondir",
            "gitdir",
            "config",
            "config.worktree",
            "packed-refs",
            "shallow",
            "info/attributes",
            "info/exclude",
            "info/grafts",
            "objects/info/alternates",
            "objects/info/http-alternates",
        ] {
            hasher.update(root_name.as_bytes());
            hasher.update([0]);
            hasher.update(relative.as_bytes());
            hasher.update([0]);
            if let Some(facts) = capture_optional_file_facts(
                &root.join(relative),
                MAX_GIT_CONTROL_FILE_BYTES,
                "LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED",
            )? {
                if matches!(relative, "config" | "config.worktree" | "info/attributes") {
                    let bytes = read_bounded_file(
                        &root.join(relative),
                        MAX_GIT_CONTROL_FILE_BYTES,
                        "LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED",
                    )?;
                    if facts.byte_len != u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                        || facts.content_digest != sha256_bytes(&bytes)?
                        || (matches!(relative, "config" | "config.worktree")
                            && !git_local_config_is_closed(&bytes))
                        || (relative == "info/attributes" && !git_attributes_are_closed(&bytes))
                    {
                        return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
                    }
                }
                file_count = file_count
                    .checked_add(1)
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?;
                total_bytes = total_bytes
                    .checked_add(facts.byte_len)
                    .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?;
                if file_count > MAX_TRUSTED_CONTROL_FILES
                    || total_bytes > MAX_GIT_CONTROL_AGGREGATE_BYTES
                {
                    return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
                }
                hasher.update(facts.byte_len.to_be_bytes());
                hasher.update(facts.content_digest.as_str().as_bytes());
                update_file_identity_hash(&mut hasher, &facts.file_identity);
            }
            hasher.update([0xff]);
        }
        let hooks = root.join("hooks");
        match fs::symlink_metadata(&hooks) {
            Ok(metadata) => {
                if unsafe_file_type(&metadata) || !metadata.file_type().is_dir() {
                    return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
                }
                let hooks_anchor =
                    capture_path_anchor(&hooks, "LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED")?;
                hasher.update(root_name.as_bytes());
                hasher.update(b"\0hooks\0present\0");
                hasher.update(hooks_anchor.canonical_path_digest.as_str().as_bytes());
                update_file_identity_hash(&mut hasher, &hooks_anchor.file_identity);
                hasher.update([0xff]);

                let entries = fs::read_dir(&hooks)
                    .map_err(|_| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?;
                let mut names = Vec::new();
                for entry in entries {
                    if names.len() >= MAX_TRUSTED_CONTROL_FILES {
                        return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
                    }
                    names.push(
                        entry
                            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?
                            .file_name(),
                    );
                }
                names.sort();
                for name in names {
                    let path = hooks.join(&name);
                    let remaining = MAX_GIT_CONTROL_AGGREGATE_BYTES
                        .checked_sub(total_bytes)
                        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?;
                    let facts = capture_file_facts(
                        &path,
                        MAX_GIT_CONTROL_FILE_BYTES.min(remaining),
                        "LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED",
                    )?;
                    file_count = file_count
                        .checked_add(1)
                        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?;
                    total_bytes = total_bytes
                        .checked_add(facts.byte_len)
                        .ok_or_else(|| known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"))?;
                    if file_count > MAX_TRUSTED_CONTROL_FILES {
                        return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
                    }
                    hasher.update(root_name.as_bytes());
                    hasher.update([0]);
                    hasher.update(name.to_string_lossy().as_bytes());
                    hasher.update([0]);
                    hasher.update(facts.byte_len.to_be_bytes());
                    hasher.update(facts.content_digest.as_str().as_bytes());
                    update_file_identity_hash(&mut hasher, &facts.file_identity);
                    hasher.update([0xff]);
                }
                if capture_path_anchor(&hooks, "LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED")?
                    != hooks_anchor
                {
                    return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(root_name.as_bytes());
                hasher.update(b"\0hooks\0absent\0");
                hasher.update([0xff]);
            }
            Err(_) => return Err(known("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED")),
        }
    }
    digest_from_sha256(hasher.finalize().as_slice())
}

fn update_file_identity_hash(hasher: &mut Sha256, identity: &TrustedFileIdentity) {
    hasher.update(identity.namespace.as_bytes());
    hasher.update(identity.volume_or_device.to_be_bytes());
    hasher.update(identity.file.to_be_bytes());
}

fn command_identity(
    checks: &[TrustedCheck],
    rules: &[TrustedRule],
    allowed_paths: &[String],
    executables: &[TrustedExecutable],
    git_layout: (
        &TrustedPathAnchor,
        &TrustedGitEntry,
        &TrustedPathAnchor,
        &TrustedPathAnchor,
        &TrustedPathAnchor,
    ),
    cargo_source_snapshot: Option<&TrustedCargoSourceSnapshot>,
    npm_ancestor_guard: Option<&TrustedAmbientGuard>,
    cargo_ancestor_guard: Option<&TrustedAmbientGuard>,
    execution_environment: Option<&ExecutionEnvironmentDescriptor>,
) -> ManagedPortResult<ContentDigest> {
    let mut commands = vec![
        CanonicalValue::String("git-object-and-scope-v1".to_owned()),
        CanonicalValue::String("git-diff-check-v1".to_owned()),
        CanonicalValue::String("exact-commit-tree-diff-review-v1".to_owned()),
        CanonicalValue::String("minimal-path-from-trusted-file-parents-v1".to_owned()),
    ];
    commands.push(trusted_executables_identity_value(executables));
    let (repository, git_entry, git_directory, common_git_directory, object_directory) = git_layout;
    commands.push(git_layout_identity_value(
        repository,
        git_entry,
        git_directory,
        common_git_directory,
        object_directory,
    ));
    if let Some(snapshot) = cargo_source_snapshot {
        commands.push(cargo_source_identity_value(snapshot));
    }
    commands.extend(
        [npm_ancestor_guard, cargo_ancestor_guard]
            .into_iter()
            .flatten()
            .map(ambient_guard_identity_value),
    );
    if let Some(descriptor) = execution_environment {
        commands.push(CanonicalValue::Object(vec![
            (
                "execution_profile".to_owned(),
                CanonicalValue::String("wsl2-linux-same-domain-verifier-v2".to_owned()),
            ),
            (
                "git_execution_profile".to_owned(),
                CanonicalValue::String("wsl2-git-per-invocation-fence-v1".to_owned()),
            ),
            (
                "execution_environment_ref".to_owned(),
                CanonicalValue::String(descriptor.environment_ref().as_str().to_owned()),
            ),
            (
                "descriptor_digest".to_owned(),
                CanonicalValue::String(descriptor.descriptor_digest().as_str().to_owned()),
            ),
            (
                "execution_domain_digest".to_owned(),
                CanonicalValue::String(descriptor.execution_domain_digest().as_str().to_owned()),
            ),
            (
                "verification_toolchain_ref".to_owned(),
                CanonicalValue::String(descriptor.verification_toolchain_identity_ref().to_owned()),
            ),
        ]));
    }
    commands.extend(checks.iter().map(trusted_check_identity_value));
    commands.push(trusted_rules_identity_value(rules));
    commands.push(CanonicalValue::Object(vec![
        (
            "scope_profile".to_owned(),
            CanonicalValue::String("managed-protected-controls-v1".to_owned()),
        ),
        (
            "allowed_paths".to_owned(),
            CanonicalValue::Array(
                allowed_paths
                    .iter()
                    .cloned()
                    .map(CanonicalValue::String)
                    .collect(),
            ),
        ),
    ]));
    digest_canonical(
        "lattice.managed-verifier.command-identity",
        &CanonicalValue::Array(commands),
    )
}

fn cargo_source_identity_value(snapshot: &TrustedCargoSourceSnapshot) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "id".to_owned(),
            CanonicalValue::String("bounded-read-only-cargo-vendor-v1".to_owned()),
        ),
        (
            "vendor_digest".to_owned(),
            CanonicalValue::String(snapshot.vendor.content_digest.as_str().to_owned()),
        ),
        (
            "vendor_identity_digest".to_owned(),
            CanonicalValue::String(snapshot.vendor.identity_digest.as_str().to_owned()),
        ),
        (
            "vendor_root".to_owned(),
            path_anchor_value(&snapshot.vendor.root),
        ),
        (
            "vendor_file_count".to_owned(),
            CanonicalValue::String(snapshot.vendor.file_count.to_string()),
        ),
        (
            "vendor_byte_len".to_owned(),
            CanonicalValue::String(snapshot.vendor.byte_len.to_string()),
        ),
        (
            "config_profile".to_owned(),
            CanonicalValue::String("replace-crates-io-with-exact-vendor-offline-v1".to_owned()),
        ),
        (
            "config_canonical_path_digest".to_owned(),
            CanonicalValue::String(snapshot.config.canonical_path_digest.as_str().to_owned()),
        ),
        (
            "config_content_digest".to_owned(),
            CanonicalValue::String(snapshot.config.content_digest.as_str().to_owned()),
        ),
        (
            "config_byte_len".to_owned(),
            CanonicalValue::String(snapshot.config.byte_len.to_string()),
        ),
        (
            "config_file_identity".to_owned(),
            trusted_file_identity_value(&snapshot.config.file_identity),
        ),
    ])
}

fn git_layout_identity_value(
    repository: &TrustedPathAnchor,
    git_entry: &TrustedGitEntry,
    git_directory: &TrustedPathAnchor,
    common_git_directory: &TrustedPathAnchor,
    object_directory: &TrustedPathAnchor,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "id".to_owned(),
            CanonicalValue::String("exact-git-layout-v1".to_owned()),
        ),
        ("repository".to_owned(), path_anchor_value(repository)),
        ("git_entry".to_owned(), git_entry_value(git_entry)),
        ("git_directory".to_owned(), path_anchor_value(git_directory)),
        (
            "common_git_directory".to_owned(),
            path_anchor_value(common_git_directory),
        ),
        (
            "object_directory".to_owned(),
            path_anchor_value(object_directory),
        ),
    ])
}

fn ambient_guard_identity_value(guard: &TrustedAmbientGuard) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "profile".to_owned(),
            CanonicalValue::String(guard.profile.to_owned()),
        ),
        (
            "digest".to_owned(),
            CanonicalValue::String(guard.digest.as_str().to_owned()),
        ),
    ])
}

fn path_anchor_value(anchor: &TrustedPathAnchor) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "canonical_path_digest".to_owned(),
            CanonicalValue::String(anchor.canonical_path_digest.as_str().to_owned()),
        ),
        (
            "file_identity".to_owned(),
            trusted_file_identity_value(&anchor.file_identity),
        ),
    ])
}

fn git_entry_value(entry: &TrustedGitEntry) -> CanonicalValue {
    match entry {
        TrustedGitEntry::Directory(anchor) => CanonicalValue::Object(vec![
            (
                "kind".to_owned(),
                CanonicalValue::String("directory".to_owned()),
            ),
            ("anchor".to_owned(), path_anchor_value(anchor)),
        ]),
        TrustedGitEntry::File { facts, .. } => CanonicalValue::Object(vec![
            (
                "kind".to_owned(),
                CanonicalValue::String("gitfile".to_owned()),
            ),
            (
                "canonical_path_digest".to_owned(),
                CanonicalValue::String(facts.canonical_path_digest.as_str().to_owned()),
            ),
            (
                "content_digest".to_owned(),
                CanonicalValue::String(facts.content_digest.as_str().to_owned()),
            ),
            (
                "byte_len".to_owned(),
                CanonicalValue::String(facts.byte_len.to_string()),
            ),
            (
                "file_identity".to_owned(),
                trusted_file_identity_value(&facts.file_identity),
            ),
        ]),
    }
}

fn trusted_executables_identity_value(executables: &[TrustedExecutable]) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "id".to_owned(),
            CanonicalValue::String("trusted-external-files-v2".to_owned()),
        ),
        (
            "files".to_owned(),
            CanonicalValue::Array(
                executables
                    .iter()
                    .map(|trusted| {
                        CanonicalValue::Object(vec![
                            (
                                "role".to_owned(),
                                CanonicalValue::String(trusted.role.to_owned()),
                            ),
                            (
                                "canonical_path_digest".to_owned(),
                                CanonicalValue::String(
                                    trusted.canonical_path_digest.as_str().to_owned(),
                                ),
                            ),
                            (
                                "content_digest".to_owned(),
                                CanonicalValue::String(trusted.content_digest.as_str().to_owned()),
                            ),
                            (
                                "byte_len".to_owned(),
                                CanonicalValue::String(trusted.byte_len.to_string()),
                            ),
                            (
                                "file_identity".to_owned(),
                                trusted_file_identity_value(&trusted.file_identity),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

fn trusted_check_identity_value(check: &TrustedCheck) -> CanonicalValue {
    let node_plan = check
        .node_plan
        .as_ref()
        .map_or(CanonicalValue::Null, |plan| {
            CanonicalValue::Array(
                plan.invocations
                    .iter()
                    .map(|invocation| {
                        CanonicalValue::Object(vec![
                            (
                                "script".to_owned(),
                                CanonicalValue::String(invocation.script.clone()),
                            ),
                            (
                                "arguments".to_owned(),
                                CanonicalValue::Array(
                                    invocation
                                        .arguments
                                        .iter()
                                        .cloned()
                                        .map(CanonicalValue::String)
                                        .collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            )
        });
    CanonicalValue::Object(vec![
        (
            "id".to_owned(),
            CanonicalValue::String(check.kind.id().to_owned()),
        ),
        (
            "control_profile".to_owned(),
            CanonicalValue::String(check.control_profile.to_owned()),
        ),
        (
            "control_files".to_owned(),
            CanonicalValue::Array(
                check
                    .control_files
                    .iter()
                    .map(|control| {
                        CanonicalValue::Object(vec![
                            (
                                "path".to_owned(),
                                CanonicalValue::String(control.path.clone()),
                            ),
                            (
                                "canonical_path_digest".to_owned(),
                                CanonicalValue::String(
                                    control.canonical_path_digest.as_str().to_owned(),
                                ),
                            ),
                            (
                                "base_file_digest".to_owned(),
                                CanonicalValue::String(
                                    control.base_file_digest.as_str().to_owned(),
                                ),
                            ),
                            (
                                "byte_len".to_owned(),
                                CanonicalValue::String(control.byte_len.to_string()),
                            ),
                            (
                                "file_identity".to_owned(),
                                trusted_file_identity_value(&control.file_identity),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("node_plan".to_owned(), node_plan),
    ])
}

fn trusted_rules_identity_value(rules: &[TrustedRule]) -> CanonicalValue {
    CanonicalValue::Object(vec![(
        "project_rules".to_owned(),
        CanonicalValue::Array(
            rules
                .iter()
                .map(|rule| {
                    CanonicalValue::Object(vec![
                        ("path".to_owned(), CanonicalValue::String(rule.path.clone())),
                        (
                            "base_file_digest".to_owned(),
                            CanonicalValue::String(rule.base_file_digest.as_str().to_owned()),
                        ),
                    ])
                })
                .collect(),
        ),
    )])
}

fn trusted_file_identity_value(identity: &TrustedFileIdentity) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "namespace".to_owned(),
            CanonicalValue::String(identity.namespace.to_owned()),
        ),
        (
            "volume_or_device".to_owned(),
            CanonicalValue::String(identity.volume_or_device.to_string()),
        ),
        (
            "file".to_owned(),
            CanonicalValue::String(identity.file.to_string()),
        ),
    ])
}

#[allow(clippy::too_many_arguments)]
fn git_snapshot_evidence(
    config: &ManagedVerifierConfig,
    attempt: &VerifiedWorkerAttemptRecord,
    base_commit: &str,
    result_commit: &str,
    tree: &str,
    diff_digest: &ContentDigest,
    changed_paths: &[String],
    checks: &[CheckResult],
    command_identity: &ContentDigest,
) -> ManagedPortResult<VerifiedManagedEvidence> {
    let bytes = canonicalize(&CanonicalValue::Object(vec![
        (
            "schema".to_owned(),
            CanonicalValue::String(SNAPSHOT_SCHEMA.to_owned()),
        ),
        (
            "base_commit".to_owned(),
            CanonicalValue::String(base_commit.to_owned()),
        ),
        (
            "result_commit".to_owned(),
            CanonicalValue::String(result_commit.to_owned()),
        ),
        ("tree".to_owned(), CanonicalValue::String(tree.to_owned())),
        (
            "diff_digest".to_owned(),
            CanonicalValue::String(diff_digest.as_str().to_owned()),
        ),
        (
            "command_identity".to_owned(),
            CanonicalValue::String(command_identity.as_str().to_owned()),
        ),
        (
            "execution_environment_ref".to_owned(),
            config
                .execution_environment
                .as_ref()
                .map_or(CanonicalValue::Null, |descriptor| {
                    CanonicalValue::String(descriptor.environment_ref().as_str().to_owned())
                }),
        ),
        (
            "execution_environment_descriptor_digest".to_owned(),
            config
                .execution_environment
                .as_ref()
                .map_or(CanonicalValue::Null, |descriptor| {
                    CanonicalValue::String(descriptor.descriptor_digest().as_str().to_owned())
                }),
        ),
        (
            "changed_paths".to_owned(),
            CanonicalValue::Array(
                changed_paths
                    .iter()
                    .map(|path| CanonicalValue::String(path.clone()))
                    .collect(),
            ),
        ),
        (
            "checks".to_owned(),
            CanonicalValue::Array(
                checks
                    .iter()
                    .map(|check| {
                        CanonicalValue::Object(vec![
                            ("id".to_owned(), CanonicalValue::String(check.id.to_owned())),
                            ("passed".to_owned(), CanonicalValue::Bool(check.passed)),
                            (
                                "wsl_execution_receipt_json".to_owned(),
                                check
                                    .wsl_receipt_json
                                    .as_ref()
                                    .map_or(CanonicalValue::Null, |receipt| {
                                        CanonicalValue::String(receipt.clone())
                                    }),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
    .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EVIDENCE_FAILED"))?
    .into_vec();
    let attempt_number = u8::try_from(attempt.attempt_number())
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EVIDENCE_FAILED"))?;
    let producer_digest = sha256_bytes(b"lattice-runtime-managed-verifier/1.0")?;
    let input = ManagedEvidenceInput::new(
        config.project_id.clone(),
        attempt.task_ref().clone(),
        attempt_number,
        ManagedEvidenceKind::GitSnapshot,
        "application/json",
        SNAPSHOT_SCHEMA,
        PRODUCER_ID,
        PRODUCER_VERSION,
        producer_digest,
        config.created_at.clone(),
        bytes,
    )
    .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EVIDENCE_FAILED"))?;
    VerifiedManagedEvidence::new(input)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EVIDENCE_FAILED"))
}

fn verification_result_digest(
    request: &ManagedVerificationRequest,
    outcome: VerificationOutcome,
    checks: &[CheckResult],
    review_digest: Option<&ContentDigest>,
) -> ManagedPortResult<ContentDigest> {
    digest_canonical(
        "lattice.managed-verifier.result",
        &CanonicalValue::Object(vec![
            (
                "command_identity".to_owned(),
                CanonicalValue::String(request.command_identity().as_str().to_owned()),
            ),
            (
                "result_commit_digest".to_owned(),
                CanonicalValue::String(request.result_commit_digest().as_str().to_owned()),
            ),
            (
                "diff_digest".to_owned(),
                CanonicalValue::String(request.diff_digest().as_str().to_owned()),
            ),
            (
                "outcome".to_owned(),
                CanonicalValue::String(outcome.as_str().to_owned()),
            ),
            (
                "checks".to_owned(),
                CanonicalValue::Array(
                    checks
                        .iter()
                        .map(|check| {
                            CanonicalValue::Object(vec![
                                ("id".to_owned(), CanonicalValue::String(check.id.to_owned())),
                                ("passed".to_owned(), CanonicalValue::Bool(check.passed)),
                                (
                                    "wsl_execution_receipt_json".to_owned(),
                                    check
                                        .wsl_receipt_json
                                        .as_ref()
                                        .map_or(CanonicalValue::Null, |receipt| {
                                            CanonicalValue::String(receipt.clone())
                                        }),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "review_digest".to_owned(),
                review_digest.map_or(CanonicalValue::Null, |digest| {
                    CanonicalValue::String(digest.as_str().to_owned())
                }),
            ),
        ]),
    )
}

fn digest_canonical(schema: &str, value: &CanonicalValue) -> ManagedPortResult<ContentDigest> {
    let domain = HashDomain::new(schema, "1.0")
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_DIGEST_FAILED"))?;
    let digest = canonical_sha256(&domain, value)
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_DIGEST_FAILED"))?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_DIGEST_FAILED"))
}

fn canonical_json_value(value: &Value) -> ManagedPortResult<String> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), sorted(&object[key])))
                        .collect(),
                )
            }
            Value::Array(values) => Value::Array(values.iter().map(sorted).collect()),
            _ => value.clone(),
        }
    }
    serde_json::to_string(&sorted(value))
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_EXECUTION_REJECTED"))
}

fn sha256_bytes(bytes: &[u8]) -> ManagedPortResult<ContentDigest> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    digest_from_sha256(hasher.finalize().as_slice())
}

fn sha256_file(path: &Path) -> ManagedPortResult<ContentDigest> {
    capture_file_facts(
        path,
        MAX_TRUSTED_EXECUTABLE_BYTES,
        "LATTICE_MANAGED_VERIFIER_EXECUTABLE_REJECTED",
    )
    .map(|facts| facts.content_digest)
}

fn digest_from_sha256(bytes: &[u8]) -> ManagedPortResult<ContentDigest> {
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| known("LATTICE_MANAGED_VERIFIER_DIGEST_FAILED"))?;
    }
    ContentDigest::from_sha256(encoded).map_err(|_| known("LATTICE_MANAGED_VERIFIER_DIGEST_FAILED"))
}

fn zero_digest() -> ManagedPortResult<ContentDigest> {
    ContentDigest::from_sha256("0".repeat(64))
        .map_err(|_| known("LATTICE_MANAGED_VERIFIER_DIGEST_FAILED"))
}

const fn known(code: &'static str) -> ManagedPortError {
    ManagedPortError::new(ManagedPortErrorKind::Known, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_node_plan_expands_exact_lifecycle_order_without_a_shell() {
        let package: Value = serde_json::from_str(
            r#"{
                "scripts": {
                    "preverify": "node preverify.mjs",
                    "verify": "npm run check && node verify.mjs --fixed",
                    "postverify": "node postverify.mjs",
                    "precheck": "node precheck.mjs",
                    "check": "node check.mjs",
                    "postcheck": "node postcheck.mjs"
                }
            }"#,
        )
        .expect("package");
        let tracked = [
            "package.json",
            "preverify.mjs",
            "verify.mjs",
            "postverify.mjs",
            "precheck.mjs",
            "check.mjs",
            "postcheck.mjs",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let profile = trusted_npm_profile(&tracked, &package).expect("closed node plan");
        assert_eq!(
            profile
                .plan
                .invocations
                .iter()
                .map(|invocation| invocation.script.as_str())
                .collect::<Vec<_>>(),
            [
                "preverify",
                "precheck",
                "check",
                "postcheck",
                "verify",
                "postverify"
            ]
        );
        assert_eq!(
            profile.plan.invocations[4].arguments,
            ["verify.mjs", "--fixed"]
        );
        assert!(profile.control_paths.contains(&"package.json".to_owned()));
    }

    #[test]
    fn trusted_node_plan_rejects_eval_cycles_workspace_and_dynamic_shell_resolution() {
        for (label, scripts) in [
            ("eval", r#"{"verify":"node --eval \"process.exit(0)\""}"#),
            (
                "cycle",
                r#"{"verify":"npm run check","check":"npm run verify"}"#,
            ),
            (
                "workspace",
                r#"{"verify":"npm --workspace app run check","check":"node check.mjs"}"#,
            ),
            ("bin", r#"{"verify":"node_modules/.bin/node check.mjs"}"#),
            (
                "branch",
                r#"{"verify":"node check.mjs || node fallback.mjs"}"#,
            ),
            ("dynamic", r#"{"verify":"node $VERIFY_RUNNER"}"#),
        ] {
            let package: Value =
                serde_json::from_str(&format!(r#"{{"scripts":{scripts}}}"#)).expect("package");
            let tracked = vec![
                "package.json".to_owned(),
                "check.mjs".to_owned(),
                "fallback.mjs".to_owned(),
            ];
            assert!(
                trusted_npm_profile(&tracked, &package).is_err(),
                "unsupported profile admitted: {label}"
            );
        }
    }

    #[test]
    fn trusted_node_plan_arguments_are_part_of_command_identity() {
        let control = TrustedControlFile {
            path: "package.json".to_owned(),
            canonical_path_digest: sha256_bytes(b"path").expect("digest"),
            base_file_digest: sha256_bytes(b"package").expect("digest"),
            byte_len: 7,
            file_identity: TrustedFileIdentity {
                namespace: "test",
                volume_or_device: 1,
                file: 2,
            },
        };
        let check = |argument: &str| TrustedCheck {
            kind: TrustedCheckKind::NpmVerify,
            control_files: vec![control.clone()],
            control_profile: "npm-static-node-plan-v3",
            node_plan: Some(TrustedNodePlan {
                invocations: vec![TrustedNodeInvocation {
                    script: "verify".to_owned(),
                    arguments: vec![argument.to_owned()],
                }],
            }),
        };
        assert_ne!(
            trusted_check_identity_value(&check("runner-a.mjs")),
            trusted_check_identity_value(&check("runner-b.mjs"))
        );
    }

    #[test]
    fn configured_tool_siblings_never_fall_back_to_ambient_path() {
        if resolve_test_program("node").is_err() {
            return;
        }
        let root = env::temp_dir().join(format!(
            "lattice-managed-verifier-hostile-path-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("empty configured tool directory");
        let marker = root.join("ambient-node-ran.txt");
        assert!(
            resolve_required_program(Some(&root), "node").is_err(),
            "a missing configured sibling fell back to ambient PATH"
        );
        assert!(!marker.exists(), "ambient executable produced an effect");
        fs::remove_dir(root).expect("remove hostile PATH fixture");
    }

    #[test]
    fn secret_scan_rejects_credentials_private_keys_and_credential_urls() {
        for sample in [
            b"password=not-for-git".as_slice(),
            b"token: not-for-git".as_slice(),
            b"-----BEGIN PRIVATE KEY-----\nnot-for-git\n-----END PRIVATE KEY-----".as_slice(),
            b"https://worker:not-for-git@example.invalid/path".as_slice(),
            b"\xffhttps://worker:not-for-git@example.invalid/path".as_slice(),
            b"diff --git a/ghp_do-not-write.txt b/ghp_do-not-write.txt".as_slice(),
            b"bare github_pat_do-not-write".as_slice(),
            b"\xffbinary\x00sk-do-not-write".as_slice(),
            b"use AKIAIOSFODNN7EXAMPLE here".as_slice(),
        ] {
            assert!(contains_secret_material(sample));
        }
        assert!(!contains_secret_material(b"const proof = 'verified';\n"));
    }
    #[test]
    fn sandbox_environment_is_an_explicit_non_secret_allowlist() {
        let environment = safe_process_environment(
            Path::new(r"C:\safe\codex-home"),
            Path::new(r"C:\safe\temp"),
            Path::new(r"C:\safe\home"),
            Path::new(r"C:\safe\cargo-home"),
            &OsString::from(r"C:\safe\bin"),
        );
        let keys = environment
            .iter()
            .map(|(key, _)| key.to_string_lossy().to_ascii_uppercase())
            .collect::<BTreeSet<_>>();
        assert!(keys.contains("CODEX_HOME"));
        assert!(keys.contains("CARGO_HOME"));
        assert!(keys.contains("GIT_TERMINAL_PROMPT"));
        assert!(keys.contains("PATH"));
        #[cfg(windows)]
        assert!(keys.contains("PROGRAMDATA"));
        let values = environment
            .iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_ascii_uppercase(),
                    value.to_string_lossy().to_string(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            values.get("USERPROFILE").map(String::as_str),
            Some(r"C:\safe\home")
        );
        assert_eq!(
            values.get("CARGO_HOME").map(String::as_str),
            Some(r"C:\safe\cargo-home")
        );
        assert_eq!(values.get("PATH").map(String::as_str), Some(r"C:\safe\bin"));
        for forbidden in [
            "COMSPEC",
            "LOCALAPPDATA",
            "PROGRAMFILES",
            "PROGRAMFILES(X86)",
            "RUSTUP_HOME",
            "RUSTC_WRAPPER",
            "RUSTDOCFLAGS",
            "RUSTFLAGS",
        ] {
            assert!(
                !keys.contains(forbidden),
                "ambient {forbidden} crossed the verifier boundary"
            );
        }
        assert!(!keys.iter().any(|key| {
            key.contains("TOKEN")
                || key.contains("PASSWORD")
                || key.contains("SECRET")
                || key.contains("DATABASE_URL")
        }));
    }

    #[test]
    fn git_environment_is_closed_and_binds_the_exact_object_store() {
        let environment = safe_git_environment(
            Path::new(r"C:\safe\home"),
            Path::new(r"C:\safe\temp"),
            &OsString::from(r"C:\safe\bin"),
            Path::new(r"C:\safe\global.gitconfig"),
            Some(Path::new(r"C:\safe\repo")),
            Some(Path::new(r"C:\safe\repo\.git\worktrees\candidate")),
            Some(Path::new(r"C:\safe\repo\.git")),
            Some(Path::new(r"C:\safe\repo\.git\objects")),
            Some(Path::new(r"C:\safe\repo\.git\worktrees\candidate\index")),
        );
        let values = environment
            .iter()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().to_ascii_uppercase(),
                    value.to_string_lossy().to_string(),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        for key in [
            "GIT_WORK_TREE",
            "GIT_DIR",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_INDEX_FILE",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_NOSYSTEM",
        ] {
            assert!(values.contains_key(key), "missing closed Git key {key}");
        }
        for hostile in [
            "GIT_CONFIG_PARAMETERS",
            "GIT_TRACE",
            "GIT_SSH_COMMAND",
            "DATABASE_URL",
            "OPENAI_API_KEY",
        ] {
            assert!(
                !values.contains_key(hostile),
                "inherited hostile key {hostile}"
            );
        }
    }

    #[test]
    fn git_filter_config_and_base_attributes_fail_before_helper_execution() {
        assert!(git_local_config_is_closed(
            b"[core]\n\trepositoryformatversion = 0\n"
        ));
        for rejected in [
            b"[include]\npath = /tmp/hostile\n".as_slice(),
            b"[includeIf \"gitdir:/home/\"]\npath = /tmp/hostile\n".as_slice(),
            b"[filter \"driver\"]\nprocess = hostile\n".as_slice(),
            b"[diff \"driver\"]\ncommand = hostile\n".as_slice(),
            b"[core]\nattributesFile = /tmp/hostile\n".as_slice(),
            b"[core]\nexcludesFile = /tmp/hostile\n".as_slice(),
        ] {
            assert!(!git_local_config_is_closed(rejected));
        }
        assert!(git_attributes_are_closed(b"*.txt text eol=lf\n"));
        assert!(!git_attributes_are_closed(
            b"*.txt filter=lattice-sentinel\n"
        ));
        assert!(!git_attributes_are_closed(b"*.txt diff=external\n"));
        let _serial = PROCESS_TEST_SERIAL.lock().expect("process test serial");
        let git = resolve_test_program("git").expect("Git executable");
        let root = env::temp_dir().join(format!(
            "lattice-managed-verifier-filter-driver-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let repository = root.join("repo");
        let marker = root.join("filter-helper-ran.txt");
        fs::create_dir_all(&repository).expect("filter fixture repository");
        let git_success = |args: &[&str]| {
            assert!(
                Command::new(&git)
                    .args(args)
                    .current_dir(&repository)
                    .env("GIT_AUTHOR_NAME", "LATTICE Test")
                    .env("GIT_AUTHOR_EMAIL", "lattice-test@invalid.local")
                    .env("GIT_COMMITTER_NAME", "LATTICE Test")
                    .env("GIT_COMMITTER_EMAIL", "lattice-test@invalid.local")
                    .status()
                    .expect("run Git")
                    .success(),
                "Git failed: {args:?}"
            );
        };
        git_success(&["init", "--initial-branch=main"]);
        fs::write(
            repository.join(".gitattributes"),
            b"*.txt filter=lattice-sentinel\n",
        )
        .expect("base attributes");
        fs::write(repository.join("base.txt"), b"base\n").expect("base file");
        git_success(&["add", "--", ".gitattributes", "base.txt"]);
        git_success(&["commit", "--no-verify", "--no-gpg-sign", "-m", "base"]);
        let marker_command = format!(
            "printf sentinel > '{}'",
            marker.to_string_lossy().replace('\\', "/")
        );
        git_success(&[
            "config",
            "--local",
            "filter.lattice-sentinel.clean",
            &marker_command,
        ]);
        git_success(&[
            "config",
            "--local",
            "filter.lattice-sentinel.required",
            "true",
        ]);
        let config = ManagedVerifierConfig::new(
            ProjectId::new("project-filter-driver").expect("project"),
            repository.clone(),
            git,
            None,
            None,
            None,
            sha256_bytes(b"filter-driver-worktree").expect("worktree digest"),
            vec!["base.txt".to_owned()],
            "2026-08-28T00:00:00Z",
            Duration::from_secs(30),
        )
        .expect("config");
        let result = ManagedVerificationAdapter::new(config);
        let code = match result {
            Ok(adapter) => {
                drop(adapter);
                None
            }
            Err(failure) => Some(failure.code()),
        };
        let helper_ran = marker.exists();
        fs::remove_dir_all(&root).expect("remove filter fixture");
        assert_eq!(code, Some("LATTICE_MANAGED_VERIFIER_GIT_CONTROL_REJECTED"));
        assert!(!helper_ran, "Git filter helper executed before rejection");
    }

    #[test]
    fn candidate_index_is_fixed_absent_before_start_and_removed_after_use() {
        let _serial = PROCESS_TEST_SERIAL.lock().expect("process test serial");
        let git = resolve_test_program("git").expect("Git executable");
        let root = env::temp_dir().join(format!(
            "lattice-managed-verifier-candidate-index-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let repository = root.join("repo");
        fs::create_dir_all(&repository).expect("candidate fixture repository");
        let git_success = |args: &[&str]| {
            assert!(
                Command::new(&git)
                    .args(args)
                    .current_dir(&repository)
                    .env("GIT_AUTHOR_NAME", "LATTICE Test")
                    .env("GIT_AUTHOR_EMAIL", "lattice-test@invalid.local")
                    .env("GIT_COMMITTER_NAME", "LATTICE Test")
                    .env("GIT_COMMITTER_EMAIL", "lattice-test@invalid.local")
                    .status()
                    .expect("run Git")
                    .success(),
                "Git failed: {args:?}"
            );
        };
        git_success(&["init", "--initial-branch=main"]);
        fs::write(repository.join("proof.txt"), b"base\n").expect("base file");
        git_success(&["add", "--", "proof.txt"]);
        git_success(&["commit", "--no-verify", "--no-gpg-sign", "-m", "base"]);
        let config = ManagedVerifierConfig::new(
            ProjectId::new("project-candidate-index").expect("project"),
            repository.clone(),
            git,
            None,
            None,
            None,
            sha256_bytes(b"candidate-index-worktree").expect("worktree digest"),
            vec!["proof.txt".to_owned()],
            "2026-08-28T00:00:00Z",
            Duration::from_secs(30),
        )
        .expect("config");
        let adapter = ManagedVerificationAdapter::new(config).expect("candidate adapter");
        fs::write(repository.join("proof.txt"), b"candidate\n").expect("candidate file");
        let index = adapter.control_directory.join(CANDIDATE_INDEX_FILE);
        fs::write(&index, b"sentinel").expect("pre-existing candidate index");
        let failure = adapter
            .materialize_tree(&["proof.txt".to_owned()])
            .expect_err("pre-existing candidate index must fail closed");
        assert_eq!(
            failure.code(),
            "LATTICE_MANAGED_VERIFIER_GIT_INDEX_REJECTED"
        );
        assert_eq!(fs::read(&index).expect("sentinel retained"), b"sentinel");
        fs::remove_file(&index).expect("remove sentinel");
        let tree = adapter
            .materialize_tree(&["proof.txt".to_owned()])
            .expect("fixed candidate index materialization");
        assert_eq!(tree.len(), 40);
        assert!(!index.exists(), "candidate index leaked after use");
        assert!(!index.with_extension("lock").exists(), "index lock leaked");
        drop(adapter);
        fs::remove_dir_all(&root).expect("remove candidate fixture");
    }

    #[test]
    fn gitfile_aba_between_guard_and_spawn_fails_before_object_write() {
        let _serial = PROCESS_TEST_SERIAL.lock().expect("process test serial");
        let git = resolve_test_program("git").expect("Git executable");
        let root = env::temp_dir().join(format!(
            "lattice-managed-verifier-gitfile-aba-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let owner = root.join("owner");
        let linked = root.join("linked");
        fs::create_dir_all(&owner).expect("owner repository");
        let git_success = |repository: &Path, args: &[&str]| {
            let status = Command::new(&git)
                .args(args)
                .current_dir(repository)
                .env("GIT_AUTHOR_NAME", "LATTICE Test")
                .env("GIT_AUTHOR_EMAIL", "lattice-test@invalid.local")
                .env("GIT_COMMITTER_NAME", "LATTICE Test")
                .env("GIT_COMMITTER_EMAIL", "lattice-test@invalid.local")
                .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
                .status()
                .expect("run Git");
            assert!(status.success(), "Git failed: {args:?}");
        };
        git_success(&owner, &["init", "--initial-branch=main"]);
        fs::write(owner.join("base.txt"), b"base\n").expect("base file");
        git_success(&owner, &["add", "--", "base.txt"]);
        git_success(
            &owner,
            &["commit", "--no-verify", "--no-gpg-sign", "-m", "base"],
        );
        git_success(
            &owner,
            &[
                "worktree",
                "add",
                "-b",
                "candidate",
                linked.to_str().expect("linked path"),
            ],
        );
        let config = ManagedVerifierConfig::new(
            ProjectId::new("project-gitfile-aba").expect("project"),
            linked.clone(),
            git.clone(),
            None,
            None,
            None,
            sha256_bytes(b"gitfile-aba-worktree").expect("worktree digest"),
            vec!["proof.txt".to_owned()],
            "2026-08-26T04:00:00Z",
            Duration::from_secs(30),
        )
        .expect("config");
        let adapter = ManagedVerificationAdapter::new(config).expect("linked adapter");
        let gitfile = linked.join(".git");
        let displaced = linked.join(".git.aba-original");
        let original = fs::read(&gitfile).expect("gitfile bytes");
        *GIT_PRE_SPAWN_FAILPOINT.lock().expect("failpoint lock") = Some((
            adapter.repository.clone(),
            Box::new(move || {
                if fs::rename(&gitfile, &displaced).is_ok() {
                    let _ = fs::write(&gitfile, original);
                }
                true
            }),
        ));
        let candidate = b"aba-object-must-not-be-written-unique-2026-08-27\n";
        let failure = adapter
            .git_success(&["hash-object", "-w", "--stdin"], None, Some(candidate))
            .expect_err("gitfile ABA must fail at the adjacent pre-spawn guard");
        assert_eq!(failure.code(), "LATTICE_MANAGED_VERIFIER_GIT_LAYOUT_DRIFT");

        let mut hash = Command::new(&git)
            .args(["hash-object", "--stdin"])
            .current_dir(&owner)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("compute candidate object id");
        hash.stdin
            .take()
            .expect("hash stdin")
            .write_all(candidate)
            .expect("hash bytes");
        let oid = String::from_utf8(hash.wait_with_output().expect("hash output").stdout)
            .expect("hash UTF-8")
            .trim()
            .to_owned();
        assert!(
            !Command::new(&git)
                .args(["cat-file", "-e", &format!("{oid}^{{blob}}")])
                .current_dir(&owner)
                .status()
                .expect("inspect candidate object")
                .success(),
            "the substituted Git layout was allowed to write an object"
        );
        drop(adapter);
        fs::remove_dir_all(&root).expect("remove owned ABA fixture");
    }

    #[test]
    fn toolchain_probe_to_seal_substitution_fails_before_substitute_execution() {
        let _serial = PROCESS_TEST_SERIAL.lock().expect("process test serial");
        let git = resolve_test_program("git").expect("Git executable");
        let rustc = resolve_test_program("rustc").expect("Rust compiler");
        let root = env::temp_dir().join(format!(
            "lattice-managed-verifier-toolchain-aba-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let repository = root.join("repo");
        let proxy_bin = root.join("proxy-bin");
        let sysroot = root.join("toolchain");
        let active_bin = sysroot.join("bin");
        fs::create_dir_all(repository.join("src")).expect("repository");
        fs::create_dir_all(&proxy_bin).expect("proxy bin");
        fs::create_dir_all(&active_bin).expect("active bin");
        fs::write(
            repository.join("Cargo.toml"),
            b"[package]\nname='toolchain_aba'\nversion='0.1.0'\nedition='2024'\n",
        )
        .expect("manifest");
        fs::write(repository.join("Cargo.lock"), b"version = 4\n").expect("lock");
        fs::write(repository.join("src/lib.rs"), b"pub fn proof() {}\n").expect("source");
        let proxy_source = root.join("proxy.rs");
        let proxy = root.join(if cfg!(windows) {
            "toolchain-proxy.exe"
        } else {
            "toolchain-proxy"
        });
        fs::write(
            &proxy_source,
            format!(
                "fn main() {{ if std::env::args().skip(1).eq([\"--print\".to_owned(), \"sysroot\".to_owned()]) {{ println!({:?}); }} }}\n",
                sysroot.to_string_lossy()
            ),
        )
        .expect("proxy source");
        assert!(
            Command::new(&rustc)
                .args([
                    proxy_source.as_os_str(),
                    OsStr::new("-o"),
                    proxy.as_os_str()
                ])
                .status()
                .expect("compile proxy")
                .success()
        );
        let executable_name = |name: &str| {
            if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            }
        };
        for name in ["cargo", "rustc", "rustdoc"] {
            fs::copy(&proxy, proxy_bin.join(executable_name(name))).expect("proxy executable");
            fs::copy(&proxy, active_bin.join(executable_name(name))).expect("active executable");
        }
        let marker = root.join("malicious-rustc-ran.txt");
        let malicious_source = root.join("malicious.rs");
        let malicious = root.join(if cfg!(windows) {
            "malicious-rustc.exe"
        } else {
            "malicious-rustc"
        });
        fs::write(
            &malicious_source,
            format!(
                "fn main() {{ std::fs::write({:?}, b\"effect\").unwrap(); println!({:?}); }}\n",
                marker.to_string_lossy(),
                sysroot.to_string_lossy()
            ),
        )
        .expect("malicious source");
        assert!(
            Command::new(&rustc)
                .args([
                    malicious_source.as_os_str(),
                    OsStr::new("-o"),
                    malicious.as_os_str(),
                ])
                .status()
                .expect("compile malicious fixture")
                .success()
        );
        let git_success = |args: &[&str]| {
            assert!(
                Command::new(&git)
                    .args(args)
                    .current_dir(&repository)
                    .env("GIT_AUTHOR_NAME", "LATTICE Test")
                    .env("GIT_AUTHOR_EMAIL", "lattice-test@invalid.local")
                    .env("GIT_COMMITTER_NAME", "LATTICE Test")
                    .env("GIT_COMMITTER_EMAIL", "lattice-test@invalid.local")
                    .status()
                    .expect("run Git")
                    .success(),
                "Git failed: {args:?}"
            );
        };
        git_success(&["init", "--initial-branch=main"]);
        git_success(&["add", "--", "."]);
        git_success(&["commit", "--no-verify", "--no-gpg-sign", "-m", "base"]);
        let cargo = proxy_bin.join(executable_name("cargo"));
        let config = ManagedVerifierConfig::new(
            ProjectId::new("project-toolchain-aba").expect("project"),
            repository.clone(),
            git,
            Some(proxy.clone()),
            None,
            Some(cargo),
            sha256_bytes(b"toolchain-aba-worktree").expect("worktree digest"),
            vec!["proof.txt".to_owned()],
            "2026-08-26T04:00:00Z",
            Duration::from_secs(30),
        )
        .expect("config");
        *TOOLCHAIN_PRE_SEAL_FAILPOINT
            .lock()
            .expect("toolchain failpoint lock") = Some((
            repository.clone(),
            Box::new(move |paths| {
                let rustc_path = paths
                    .iter()
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.eq_ignore_ascii_case(&executable_name("rustc"))
                            })
                    })
                    .expect("active rustc path");
                fs::copy(&malicious, rustc_path).expect("substitute active rustc before seal");
            }),
        ));
        let failure = match ManagedVerificationAdapter::new(config) {
            Ok(_) => panic!("substituted active toolchain was admitted"),
            Err(failure) => failure,
        };
        assert_eq!(failure.code(), "LATTICE_MANAGED_VERIFIER_EXECUTABLE_DRIFT");
        assert!(
            !marker.exists(),
            "substituted rustc executed before rejection"
        );
        fs::remove_dir_all(root).expect("remove toolchain ABA fixture");
    }

    #[test]
    fn wsl_git_invocation_is_canonical_mapped_and_independently_fenced() {
        let args = vec![
            OsString::from("--no-pager"),
            OsString::from("--no-replace-objects"),
            OsString::from("--literal-pathspecs"),
            OsString::from("-c"),
            OsString::from(r"core.hooksPath=\\wsl.localhost\Ubuntu\home\zk\task\control\hooks"),
            OsString::from("-c"),
            OsString::from("core.fsmonitor=false"),
            OsString::from("-c"),
            OsString::from("protocol.allow=never"),
            OsString::from("-c"),
            OsString::from("commit.gpgSign=false"),
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("HEAD^{commit}"),
        ];
        let mapped_args = wsl_git_arguments(&args, "Ubuntu").expect("closed Git args");
        assert_eq!(mapped_args[4], "core.hooksPath=/home/zk/task/control/hooks");
        let environment = vec![
            (
                OsString::from("HOME"),
                OsString::from(r"\\wsl.localhost\Ubuntu\home\zk\task\control\home"),
            ),
            (
                OsString::from("GIT_WORK_TREE"),
                OsString::from(r"\\wsl.localhost\Ubuntu\home\zk\task\repository"),
            ),
            (
                OsString::from("TEMP"),
                OsString::from(r"\\wsl.localhost\Ubuntu\home\zk\task\control\temp"),
            ),
            (
                OsString::from("TMP"),
                OsString::from(r"\\wsl.localhost\Ubuntu\home\zk\task\control\temp"),
            ),
            (OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1")),
            (
                OsString::from("GIT_AUTHOR_NAME"),
                OsString::from("LATTICE Foreman"),
            ),
            (OsString::from("PATH"), OsString::from(r"C:\must\not\cross")),
        ];
        let mapped_environment =
            wsl_git_environment(&environment, "Ubuntu").expect("closed Git environment");
        assert_eq!(
            mapped_environment.pointer("/HOME").and_then(Value::as_str),
            Some("/home/zk/task/control/home")
        );
        assert_eq!(
            mapped_environment
                .pointer("/GIT_WORK_TREE")
                .and_then(Value::as_str),
            Some("/home/zk/task/repository")
        );
        assert_eq!(
            mapped_environment
                .pointer("/TMPDIR")
                .and_then(Value::as_str),
            Some("/home/zk/task/control/temp")
        );
        assert!(mapped_environment.get("TEMP").is_none());
        assert!(mapped_environment.get("TMP").is_none());
        assert!(mapped_environment.get("PATH").is_none());
        let subject = serde_json::json!({
            "schema": "lattice.wsl2-git-invocation/1.0",
            "sequence": 1,
            "environment": mapped_environment,
            "args": mapped_args,
            "stdin": Value::Null,
        });
        let invocation =
            typed_json_sha256("wsl2-git-invocation", &subject).expect("invocation digest");
        let preflight = "a".repeat(64);
        let first = wsl_git_process_fence(&preflight, &invocation, 1).expect("first fence");
        let second = wsl_git_process_fence(&preflight, &invocation, 2).expect("second fence");
        assert!(plain_sha256(&first));
        assert_ne!(first, second, "a Git process fence was reused");
        assert!(continuation_marker_matches(Some(&Value::Null), None));
        assert!(!continuation_marker_matches(
            Some(&Value::String("NONE".to_owned())),
            None,
        ));
        let retry = format!("verifier-receipt:sha256:{}", "b".repeat(64));
        assert_eq!(
            continuation_marker_value(Some(&Value::String(retry.clone())), || known(
                "LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"
            ),)
            .expect("verifier-owned continuation"),
            Some(retry.clone()),
        );
        assert!(
            continuation_marker_value(
                Some(&Value::String(format!(
                    "attempt-receipt:sha256:{}",
                    "c".repeat(64)
                ))),
                || known("LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED"),
            )
            .is_err(),
            "provider attempt receipt must not enter the verifier domain",
        );
        assert!(continuation_marker_matches(
            Some(&Value::String(retry.clone())),
            Some(&retry),
        ));
        assert!(continuation_shape_valid(1, None, None));
        assert!(continuation_shape_valid(1, None, Some(&retry)));
        assert!(!continuation_shape_valid(1, Some(&retry), None));
        assert!(continuation_shape_valid(2, Some(&retry), None));
        assert!(continuation_shape_valid(2, None, Some(&retry)));
        assert!(continuation_shape_valid(2, None, None));
        assert!(!continuation_shape_valid(2, Some(&retry), Some(&retry)));
    }

    #[test]
    fn wsl_git_command_digest_matches_the_node_canonical_subject() {
        let descriptor = serde_json::json!({
            "linux": {
                "git_path": "/usr/bin/git",
                "git_version": "git version 2.43.0",
                "git_sha256": "a".repeat(64),
                "codex_home": "/home/lattice/credential/codex-home",
            },
            "process_fence": {
                "user_runtime_dir": "/run/user/1000",
            },
            "verification_toolchain": {
                "task_root": "/home/lattice/task",
                "sandbox": {
                    "path": "/home/lattice/tool/codex",
                    "version": "codex-cli 0.146.0",
                    "sha256": "b".repeat(64),
                },
            },
        });
        let environment = serde_json::json!({
            "HOME": "/home/lattice/control/home",
            "TMPDIR": "/home/lattice/control/tmp",
            "GIT_CONFIG_GLOBAL": "/home/lattice/control/config",
            "GIT_WORK_TREE": "/home/lattice/task/repository",
            "GIT_DIR": "/home/lattice/task/repository/.git",
            "GIT_COMMON_DIR": "/home/lattice/common",
            "GIT_OBJECT_DIRECTORY": "/home/lattice/common/objects",
            "GIT_INDEX_FILE": "/home/lattice/control/candidate-index",
            "NO_COLOR": "1",
            "CI": "1",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_COUNT": "0",
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_OPTIONAL_LOCKS": "0",
            "GIT_ATTR_NOSYSTEM": "1",
            "GIT_AUTHOR_NAME": "LATTICE Foreman",
            "GIT_AUTHOR_EMAIL": "lattice@invalid.local",
            "GIT_AUTHOR_DATE": "2026-08-28T00:00:00Z",
            "GIT_COMMITTER_NAME": "LATTICE Foreman",
            "GIT_COMMITTER_EMAIL": "lattice@invalid.local",
            "GIT_COMMITTER_DATE": "2026-08-28T00:00:00Z",
        });
        let args = [
            "--no-pager",
            "--no-replace-objects",
            "--literal-pathspecs",
            "-c",
            "core.hooksPath=/home/lattice/control/hooks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.allow=never",
            "-c",
            "commit.gpgSign=false",
            "rev-parse",
            "--verify",
            "HEAD^{commit}",
        ]
        .map(str::to_owned);
        let context = WslGitPreflightContext {
            worktree_ref: format!("worktree:sha256:{}", "1".repeat(64)),
            preflight_fence: "2".repeat(64),
            credential_seal_digest: format!("credential-seal:sha256:{}", "c".repeat(64)),
            timeout_ms: 120_000,
            stdout_limit_bytes: 262_144,
            stderr_limit_bytes: 262_144,
            retry_of: None,
            reconnect_of: None,
            unit_prefix: "lattice-wsl2-1234567890abcdef".to_owned(),
        };
        let digest = wsl_git_command_digest(
            &descriptor,
            &format!("execution-environment:sha256:{}", "f".repeat(64)),
            "/home/lattice/task/repository",
            &context,
            1,
            &args,
            &environment,
            &format!("wsl2-git-invocation:sha256:{}", "e".repeat(64)),
            &"d".repeat(64),
        )
        .expect("canonical command digest");
        assert_eq!(
            digest,
            "wsl2-verifier-command:sha256:0781ff9bf37769b6e9adc20005df51bf87b57afc7ddfadc8fbd3690127582e66"
        );
        let bootstrap_environment = serde_json::json!({
            "HOME": "/home/lattice/control/home",
            "TMPDIR": "/home/lattice/control/tmp",
            "GIT_CONFIG_GLOBAL": "/home/lattice/control/config",
            "NO_COLOR": "1",
            "CI": "1",
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_CONFIG_COUNT": "0",
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_OPTIONAL_LOCKS": "0",
            "GIT_ATTR_NOSYSTEM": "1",
        });
        let bootstrap_digest = wsl_git_command_digest(
            &descriptor,
            &format!("execution-environment:sha256:{}", "f".repeat(64)),
            "/home/lattice/task/repository",
            &context,
            1,
            &args,
            &bootstrap_environment,
            &format!("wsl2-git-invocation:sha256:{}", "e".repeat(64)),
            &"d".repeat(64),
        )
        .expect("canonical bootstrap command digest");
        assert_eq!(
            bootstrap_digest,
            "wsl2-verifier-command:sha256:0f80f45bf0cf6607b632429addb620bedcd274d1af661c207729c40c523231ff"
        );
        let mut partial_layout = bootstrap_environment.clone();
        partial_layout["GIT_INDEX_FILE"] =
            Value::String("/home/lattice/control/candidate-index".to_owned());
        assert!(
            wsl_git_command_digest(
                &descriptor,
                &format!("execution-environment:sha256:{}", "f".repeat(64)),
                "/home/lattice/task/repository",
                &context,
                1,
                &args,
                &partial_layout,
                &format!("wsl2-git-invocation:sha256:{}", "e".repeat(64)),
                &"d".repeat(64),
            )
            .is_err(),
            "a partial guarded layout crossed the bootstrap phase boundary"
        );
        let object_write_args = args[..11]
            .iter()
            .cloned()
            .chain(["hash-object", "-w", "--stdin"].map(str::to_owned))
            .collect::<Vec<_>>();
        let object_write_digest = wsl_git_command_digest(
            &descriptor,
            &format!("execution-environment:sha256:{}", "f".repeat(64)),
            "/home/lattice/task/repository",
            &context,
            1,
            &object_write_args,
            &environment,
            &format!("wsl2-git-invocation:sha256:{}", "e".repeat(64)),
            &"d".repeat(64),
        )
        .expect("object-write command digest");
        assert_eq!(
            object_write_digest,
            "wsl2-verifier-command:sha256:22b29621015fae4a07da1cc3a4635963ae6d5bb8f0d04ec4f8366aa965ec5975"
        );
        let index_write_args = args[..11]
            .iter()
            .cloned()
            .chain(["read-tree", &"1".repeat(40)].map(str::to_owned))
            .collect::<Vec<_>>();
        let index_write_digest = wsl_git_command_digest(
            &descriptor,
            &format!("execution-environment:sha256:{}", "f".repeat(64)),
            "/home/lattice/task/repository",
            &context,
            1,
            &index_write_args,
            &environment,
            &format!("wsl2-git-invocation:sha256:{}", "e".repeat(64)),
            &"d".repeat(64),
        )
        .expect("index-write command digest");
        assert_eq!(
            index_write_digest,
            "wsl2-verifier-command:sha256:4a132d96b4ba6801c3020d1d24f805cb831a74cfa7d7861e7a16f18be0f331f3"
        );
        let mut changed_descriptor = descriptor.clone();
        changed_descriptor["linux"]["git_sha256"] = Value::String("9".repeat(64));
        let substituted = wsl_git_command_digest(
            &changed_descriptor,
            &format!("execution-environment:sha256:{}", "f".repeat(64)),
            "/home/lattice/task/repository",
            &context,
            1,
            &args,
            &environment,
            &format!("wsl2-git-invocation:sha256:{}", "e".repeat(64)),
            &"d".repeat(64),
        )
        .expect("substituted command digest");
        assert_ne!(digest, substituted, "Git tool substitution was not bound");
        let mut changed_fence_descriptor = descriptor.clone();
        changed_fence_descriptor["process_fence"]["user_runtime_dir"] =
            Value::String("/run/user/1001".to_owned());
        assert_ne!(
            digest,
            wsl_git_command_digest(
                &changed_fence_descriptor,
                &format!("execution-environment:sha256:{}", "f".repeat(64)),
                "/home/lattice/task/repository",
                &context,
                1,
                &args,
                &environment,
                &format!("wsl2-git-invocation:sha256:{}", "e".repeat(64)),
                &"d".repeat(64),
            )
            .expect("runtime deny substitution digest"),
            "the process-fence runtime deny was not command-bound"
        );
        assert_eq!(CANDIDATE_INDEX_FILE, "candidate-index");
        assert_eq!(
            wsl_linux_file_uri("/home/lattice/a b/括號(x)!").expect("encoded Linux file URI"),
            "file:///home/lattice/a%20b/%E6%8B%AC%E8%99%9F(x)!"
        );
    }

    #[test]
    fn wsl_git_exit_tool_identities_are_exactly_descriptor_bound() {
        let descriptor = serde_json::json!({
            "linux": {
                "git_path": "/usr/bin/git",
                "git_sha256": "a".repeat(64),
                "node_path": "/home/lattice/task/node/bin/node",
                "node_sha256": "2".repeat(64),
                "keyring_daemon_path": "/usr/bin/gnome-keyring-daemon",
                "keyring_daemon_sha256": "b".repeat(64),
                "keyring_library_path": "/home/lattice/keyring-libraries",
                "keyring_library_manifest_digest": format!(
                    "keyring-library-manifest:sha256:{}",
                    "c".repeat(64)
                ),
            },
            "verification_toolchain": {
                "sandbox": {
                    "path": "/home/lattice/task/codex",
                    "sha256": "d".repeat(64),
                },
                "sandbox_helper": {
                    "path": "/usr/bin/bwrap",
                    "sha256": "e".repeat(64),
                },
                "npm": {
                    "path": "/home/lattice/task/node/npm-cli.js",
                    "sha256": "3".repeat(64),
                },
                "cargo": {
                    "path": "/home/lattice/task/rust/bin/cargo",
                    "sha256": "4".repeat(64),
                },
                "rustc": {
                    "path": "/home/lattice/task/rust/bin/rustc",
                    "sha256": "5".repeat(64),
                },
                "rustdoc": {
                    "path": "/home/lattice/task/rust/bin/rustdoc",
                    "sha256": "6".repeat(64),
                },
            },
        });
        let seal = |path: &str, sha256: String| {
            serde_json::json!({
                "path": path,
                "resolved_path": path,
                "sha256": sha256,
                "device": "8",
                "inode": "42",
                "owner_uid": 0,
                "mode": 0o755,
                "size": 4096,
            })
        };
        let library = |manifest_path: &str, sha256: &str| {
            let path = format!("/home/lattice/keyring-libraries/{manifest_path}");
            let mut value = seal(&path, sha256.to_owned());
            value.as_object_mut().expect("library seal").insert(
                "manifest_path".to_owned(),
                Value::String(manifest_path.to_owned()),
            );
            value
        };
        let mut exit = serde_json::json!({
            "keyring_daemon_sha256": "b".repeat(64),
            "keyring_library_manifest_digest": format!(
                "keyring-library-manifest:sha256:{}",
                "c".repeat(64)
            ),
            "tool_input_identities": {
                "executable": seal("/home/lattice/task/codex", "d".repeat(64)),
                "verifier_tool": seal("/usr/bin/git", "a".repeat(64)),
                "sandbox_helper": seal("/usr/bin/bwrap", "e".repeat(64)),
                "node_runtime": Value::Null,
                "rustc": Value::Null,
                "rustdoc": Value::Null,
                "keyring_daemon": seal("/usr/bin/gnome-keyring-daemon", "b".repeat(64)),
                "keyring_libraries": [
                    library("libgck-1.so.0.0.0", &"f".repeat(64)),
                    library("libgcr-base-3.so.1.0.0", &"1".repeat(64)),
                ],
            },
        });
        assert!(wsl_git_receipt_tool_inputs_match(
            &descriptor,
            exit.as_object().expect("exit receipt")
        ));
        exit["tool_input_identities"]["verifier_tool"]["sha256"] = Value::String("9".repeat(64));
        assert!(!wsl_git_receipt_tool_inputs_match(
            &descriptor,
            exit.as_object().expect("substituted exit receipt")
        ));

        exit["tool_input_identities"]["verifier_tool"] =
            seal("/home/lattice/task/node/npm-cli.js", "3".repeat(64));
        exit["tool_input_identities"]["node_runtime"] =
            seal("/home/lattice/task/node/bin/node", "2".repeat(64));
        assert!(wsl_regular_receipt_tool_inputs_match(
            &descriptor,
            exit.as_object().expect("NODE exit receipt"),
            "NODE"
        ));
        exit["tool_input_identities"]["node_runtime"]["owner_uid"] = Value::from(1000);
        assert!(!wsl_regular_receipt_tool_inputs_match(
            &descriptor,
            exit.as_object().expect("user-owned NODE exit receipt"),
            "NODE"
        ));

        exit["tool_input_identities"]["verifier_tool"] =
            seal("/home/lattice/task/rust/bin/cargo", "4".repeat(64));
        exit["tool_input_identities"]["node_runtime"] = Value::Null;
        exit["tool_input_identities"]["rustc"] =
            seal("/home/lattice/task/rust/bin/rustc", "5".repeat(64));
        exit["tool_input_identities"]["rustdoc"] =
            seal("/home/lattice/task/rust/bin/rustdoc", "6".repeat(64));
        assert!(wsl_regular_receipt_tool_inputs_match(
            &descriptor,
            exit.as_object().expect("CARGO exit receipt"),
            "CARGO"
        ));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires explicit real WSL2 descriptor, preflight, repository, Node, and bridge inputs"]
    #[allow(clippy::too_many_lines)]
    fn live_wsl_capture_base_uses_bootstrap_then_guarded_read_only_git_receipts() {
        const DESCRIPTOR_FILE: &str = "LATTICE_WSL2_GIT_LIVE_DESCRIPTOR_FILE";
        const PREFLIGHT_FILE: &str = "LATTICE_WSL2_GIT_LIVE_PREFLIGHT_FILE";
        const REPOSITORY_UNC: &str = "LATTICE_WSL2_GIT_LIVE_REPOSITORY_UNC";
        const NODE_EXE: &str = "LATTICE_WSL2_GIT_LIVE_NODE_EXE";
        const BRIDGE_FILE: &str = "LATTICE_WSL2_GIT_LIVE_BRIDGE_FILE";
        const MAX_LIVE_INPUT_BYTES: u64 = 1_048_576;

        let live_path = |name: &str| {
            env::var_os(name)
                .filter(|value| !value.as_os_str().is_empty())
                .map(PathBuf::from)
        };
        let (
            Some(descriptor_file),
            Some(preflight_file),
            Some(repository),
            Some(node_executable),
            Some(bridge_file),
        ) = (
            live_path(DESCRIPTOR_FILE),
            live_path(PREFLIGHT_FILE),
            live_path(REPOSITORY_UNC),
            live_path(NODE_EXE),
            live_path(BRIDGE_FILE),
        )
        else {
            eprintln!(
                "skipped: set {DESCRIPTOR_FILE}, {PREFLIGHT_FILE}, {REPOSITORY_UNC}, {NODE_EXE}, and {BRIDGE_FILE}"
            );
            return;
        };
        let repository_text = repository
            .to_str()
            .expect("live WSL repository UNC is UTF-8");
        assert!(
            repository.is_absolute()
                && (repository_text.starts_with(r"\\wsl.localhost\")
                    || repository_text.starts_with(r"\\wsl$\")),
            "live repository must be an explicit WSL UNC path"
        );
        let descriptor_file = canonical_file(&descriptor_file).expect("descriptor file");
        let descriptor_bytes = read_bounded_file(
            &descriptor_file,
            MAX_LIVE_INPUT_BYTES,
            "LATTICE_MANAGED_VERIFIER_EXECUTION_ENVIRONMENT_REJECTED",
        )
        .expect("bounded descriptor JSON");
        let descriptor = ExecutionEnvironmentDescriptor::from_json(
            std::str::from_utf8(&descriptor_bytes).expect("descriptor UTF-8"),
        )
        .expect("exact execution descriptor");
        assert_eq!(
            descriptor.path_mapping_windows_path(),
            repository_text,
            "descriptor/UNC repository substitution"
        );

        let preflight_file = canonical_file(&preflight_file).expect("preflight file");
        let preflight_bytes = read_bounded_file(
            &preflight_file,
            MAX_LIVE_INPUT_BYTES,
            "LATTICE_MANAGED_VERIFIER_EXECUTION_PREFLIGHT_REJECTED",
        )
        .expect("bounded preflight JSON");
        let receipt: Value =
            serde_json::from_slice(&preflight_bytes).expect("preflight receipt JSON");
        let task_ref = ContentDigest::from_sha256(
            receipt
                .get("task_ref")
                .and_then(Value::as_str)
                .expect("preflight task ref"),
        )
        .expect("preflight task ref digest");
        let attempt = receipt
            .get("attempt")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .filter(|value| (1..=3).contains(value))
            .expect("bounded preflight attempt");
        let worktree_digest = ContentDigest::from_sha256(
            receipt
                .get("worktree_ref")
                .and_then(Value::as_str)
                .and_then(|value| value.strip_prefix("worktree:sha256:"))
                .expect("preflight worktree ref"),
        )
        .expect("preflight worktree digest");
        let project_id = ProjectId::new("phase4-wsl2-git-live").expect("live project id");
        let node_executable = canonical_file(&node_executable).expect("Windows node.exe");
        let bridge_file = canonical_file(&bridge_file).expect("Node verifier bridge");
        let preflight = VerifiedManagedEvidence::new(
            ManagedEvidenceInput::new(
                project_id.clone(),
                task_ref,
                attempt,
                ManagedEvidenceKind::WorkerLifecycle,
                "application/json",
                "lattice.wsl2-zero-model-preflight/1.0",
                "lattice-runtime-wsl2-live-test",
                "1.0",
                sha256_file(&bridge_file).expect("bridge producer digest"),
                "2026-08-28T00:00:00Z",
                canonical_json_value(&receipt)
                    .expect("canonical preflight")
                    .into_bytes(),
            )
            .expect("preflight evidence input"),
        )
        .expect("verified preflight evidence");
        let config = ManagedVerifierConfig::new(
            project_id,
            repository,
            PathBuf::from(descriptor.gateway().path()),
            None,
            None,
            None,
            worktree_digest,
            vec!["proof.txt".to_owned()],
            "2026-08-28T00:00:00Z",
            Duration::from_secs(300),
        )
        .expect("live verifier config")
        .with_node_executable(node_executable)
        .expect("live Node binding")
        .with_wsl_execution_domain(descriptor.clone(), preflight.clone(), bridge_file)
        .expect("live WSL execution binding");

        let _serial = PROCESS_TEST_SERIAL.lock().expect("process test serial");
        let adapter = ManagedVerificationAdapter::new(config).expect("live WSL base capture");
        assert_eq!(adapter.base_commit_oid(), descriptor.repository_head());
        let bundle: Value = serde_json::from_str(
            &adapter
                .wsl_git_receipt_bundle()
                .expect("Git receipt bundle")
                .expect("WSL receipt bundle"),
        )
        .expect("Git receipt bundle JSON");
        assert_eq!(
            bundle.get("schema").and_then(Value::as_str),
            Some("lattice.wsl2-git-receipt-bundle/1.0")
        );
        assert!(
            bundle
                .get("bundle_digest")
                .and_then(Value::as_str)
                .is_some_and(|value| typed_sha256(value, "wsl2-git-receipt-bundle"))
        );
        let records = bundle
            .get("records")
            .and_then(Value::as_array)
            .expect("Git receipt records");
        assert!(
            records.len() >= 5,
            "base capture did not cross from four bootstrap calls into guarded Git"
        );
        assert_eq!(
            bundle.get("operation_count").and_then(Value::as_u64),
            Some(u64::try_from(records.len()).expect("bounded operation count"))
        );
        let mut invocation_digests = BTreeSet::new();
        let mut command_digests = BTreeSet::new();
        let mut result_digests = BTreeSet::new();
        let mut process_fences = BTreeSet::new();
        for (index, record) in records.iter().enumerate() {
            let result = record.get("result").expect("operation result");
            assert_eq!(
                record.get("sequence").and_then(Value::as_u64),
                Some(u64::try_from(index + 1).expect("bounded sequence"))
            );
            assert_eq!(result.get("status").and_then(Value::as_str), Some("PASS"));
            assert_eq!(
                result.get("provider_effect_count").and_then(Value::as_u64),
                Some(0)
            );
            assert_eq!(
                result
                    .pointer("/verifier_identity/provider_effect_count")
                    .and_then(Value::as_u64),
                Some(0)
            );
            let invocation_digest = record
                .get("invocation_digest")
                .and_then(Value::as_str)
                .expect("invocation digest");
            assert_eq!(
                result.get("invocation_digest").and_then(Value::as_str),
                Some(invocation_digest)
            );
            assert!(invocation_digests.insert(invocation_digest));
            assert!(
                command_digests.insert(
                    result
                        .pointer("/verifier_identity/command_digest")
                        .and_then(Value::as_str)
                        .expect("command digest")
                )
            );
            assert!(
                result_digests.insert(
                    result
                        .get("result_digest")
                        .and_then(Value::as_str)
                        .expect("result digest")
                )
            );
            let process_fence = result
                .pointer("/process_marker/fence")
                .and_then(Value::as_str)
                .expect("process fence");
            assert!(process_fences.insert(process_fence));
            assert_eq!(
                result
                    .pointer("/verifier_identity/process_fence")
                    .and_then(Value::as_str),
                Some(process_fence)
            );
            assert_eq!(
                result
                    .pointer("/exit_receipt/zero_descendants")
                    .and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                result
                    .pointer("/exit_receipt/schema")
                    .and_then(Value::as_str),
                Some("lattice.wsl2-subtree-exit/1.2")
            );
            assert_eq!(
                result
                    .pointer("/exit_receipt/credential_watch_intact")
                    .and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                result
                    .pointer("/exit_receipt/stdin_complete")
                    .and_then(Value::as_bool),
                Some(true)
            );
            assert!(
                result
                    .pointer("/exit_receipt/tool_input_identities")
                    .is_some(),
                "sealed tool inputs missing"
            );
            assert_eq!(
                result
                    .pointer("/outer_post_exit/active_state")
                    .and_then(Value::as_str),
                Some("inactive")
            );
            assert_eq!(
                result
                    .pointer("/outer_post_exit/sub_state")
                    .and_then(Value::as_str),
                Some("dead")
            );
        }

        let descriptor_value: Value =
            serde_json::from_str(descriptor.as_json()).expect("descriptor value");
        let context = validate_wsl_git_preflight(&descriptor, &preflight, &receipt)
            .expect("Git preflight context");
        let guarded_index = adapter.git_directory.join("index");
        let operations = [
            vec!["rev-parse", "--show-toplevel"],
            vec!["rev-parse", "--verify", "HEAD^{commit}"],
            vec!["rev-parse", "--absolute-git-dir"],
            vec!["rev-parse", "--path-format=absolute", "--git-common-dir"],
            vec!["for-each-ref", "--format=%(refname)%00%(objectname)%00"],
        ];
        for (index, operation) in operations.iter().enumerate() {
            let guarded = index >= 4;
            let mut args = vec![
                OsString::from("--no-pager"),
                OsString::from("--no-replace-objects"),
                OsString::from("--literal-pathspecs"),
                OsString::from("-c"),
                OsString::from(format!(
                    "core.hooksPath={}",
                    adapter.hooks_directory.to_string_lossy()
                )),
                OsString::from("-c"),
                OsString::from("core.fsmonitor=false"),
                OsString::from("-c"),
                OsString::from("protocol.allow=never"),
                OsString::from("-c"),
                OsString::from("commit.gpgSign=false"),
            ];
            args.extend(operation.iter().map(OsString::from));
            let environment = safe_git_environment(
                &adapter.control_directory.join("git-home"),
                &adapter.control_directory.join("git-temp"),
                &adapter.fixed_path,
                &adapter.global_config,
                guarded.then_some(adapter.repository.as_path()),
                guarded.then_some(adapter.git_directory.as_path()),
                guarded.then_some(adapter.common_git_directory.as_path()),
                guarded.then_some(adapter.object_directory.as_path()),
                guarded.then_some(guarded_index.as_path()),
            );
            let mapped_args =
                wsl_git_arguments(&args, descriptor.distribution()).expect("mapped Git args");
            let mapped_environment = wsl_git_environment(&environment, descriptor.distribution())
                .expect("mapped Git environment");
            let record = &records[index];
            let expected_command_digest = wsl_git_command_digest(
                &descriptor_value,
                descriptor.environment_ref().as_str(),
                descriptor.linux_repository_path(),
                &context,
                u64::from(preflight.attempt()),
                &mapped_args,
                &mapped_environment,
                record["invocation_digest"]
                    .as_str()
                    .expect("record invocation digest"),
                record["result"]["process_marker"]["fence"]
                    .as_str()
                    .expect("record process fence"),
            )
            .expect("phase-specific command digest");
            assert_eq!(
                record["result"]["verifier_identity"]["command_digest"].as_str(),
                Some(expected_command_digest.as_str()),
                "Git phase mismatch at sequence {}",
                index + 1
            );
        }
    }

    #[test]
    fn wsl_verifier_typed_outcomes_and_cleanup_are_exact() {
        for (outcome, status, exit_code, interrupted, timed_out, output_bound_exceeded) in [
            ("PASS", "PASS", 0, false, false, false),
            ("FAILED", "FAILED", 9, false, false, false),
            ("OUTPUT_BOUND_EXCEEDED", "FAILED", 9, false, false, true),
            ("TIMED_OUT", "FAILED", 9, false, true, true),
            ("INTERRUPTED", "FAILED", 9, true, true, true),
        ] {
            let result = serde_json::json!({"status": status, "outcome": outcome});
            let exit = serde_json::json!({
                "exit_code": exit_code,
                "exit_signal": null,
                "interrupted": interrupted,
                "timed_out": timed_out,
                "output_bound_exceeded": output_bound_exceeded,
            });
            assert_eq!(
                wsl_verifier_terminal_outcome(
                    result.as_object().expect("result"),
                    exit.as_object().expect("exit")
                )
                .expect("typed outcome"),
                outcome
            );
        }

        let descriptor = serde_json::json!({
            "process_fence": {
                "systemctl_path": "/usr/bin/systemctl",
                "systemctl_version": "systemd 259",
                "systemctl_sha256": "a".repeat(64),
            }
        });
        let attempt = |sequence, action| {
            serde_json::json!({
                "sequence": sequence,
                "action": action,
                "result": "SUCCESS",
                "exit_code": 0,
                "signal": null,
                "timed_out": false,
                "output_bound_exceeded": false,
                "stdout_captured_bytes": 0,
                "stderr_captured_bytes": 0,
                "stdout_sha256": "b".repeat(64),
                "stderr_sha256": "c".repeat(64),
            })
        };
        let mut cleanup = serde_json::json!({
            "schema": "lattice.wsl2-verifier-cleanup/1.0",
            "reason": "TIMED_OUT",
            "unit": "lattice-wsl2-unit.service",
            "process_fence": "d".repeat(64),
            "systemctl_identity": {
                "path": "/usr/bin/systemctl",
                "version": "systemd 259",
                "sha256": "a".repeat(64),
            },
            "attempt": 1,
            "retry_of": null,
            "reconnect_of": null,
            "attempts": [attempt(1, "TERM_KILL"), attempt(2, "STOP")],
            "cleanup_digest": null,
        });
        let cleanup_digest = {
            let mut subject = cleanup.clone();
            subject
                .as_object_mut()
                .expect("cleanup")
                .remove("cleanup_digest");
            typed_json_sha256("wsl2-verifier-cleanup", &subject).expect("cleanup digest")
        };
        cleanup["cleanup_digest"] = Value::String(cleanup_digest);
        validate_wsl_verifier_cleanup(
            &cleanup,
            &descriptor,
            "TIMED_OUT",
            "lattice-wsl2-unit.service",
            &"d".repeat(64),
            1,
            None,
            None,
        )
        .expect("typed cleanup");
        let mut substituted = cleanup;
        substituted["attempts"][0]["action"] = Value::String("KILL".to_owned());
        assert!(
            validate_wsl_verifier_cleanup(
                &substituted,
                &descriptor,
                "TIMED_OUT",
                "lattice-wsl2-unit.service",
                &"d".repeat(64),
                1,
                None,
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn wsl_git_base64_is_bounded_canonical_and_binary_safe() {
        let largest_candidate =
            usize::try_from(MAX_CANDIDATE_FILE_BYTES).expect("candidate bound fits usize");
        assert!(
            MAX_WSL_GIT_REQUEST_BYTES
                > largest_candidate.div_ceil(3) * 4 + MAX_WSL_GIT_ARGUMENT_BYTES + 262_144,
            "the bounded Git transport cannot carry one admitted candidate file"
        );
        let bytes = b"\0git\r\nstdout\xff";
        let encoded = base64_encode(bytes);
        assert_eq!(base64_decode(&encoded).expect("canonical base64"), bytes);
        assert_eq!(base64_decode("").expect("empty output"), b"");
        assert!(base64_decode("Zh==").is_err(), "non-canonical tail bits");
        assert!(base64_decode("Zm9v\n").is_err(), "framing whitespace");
        assert!(base64_decode("====").is_err(), "padding-only input");
        let compact = compact_wsl_git_result(&serde_json::json!({
            "schema": "lattice.wsl2-verifier-result/1.0",
            "status": "PASS",
            "outcome": "PASS",
            "task_ref": "task",
            "attempt": 1,
            "worktree_ref": "worktree",
            "role": "GIT",
            "repository_head": "head",
            "verifier_identity": {},
            "process_marker": {},
            "exit_receipt": {},
            "outer_cleanup": null,
            "outer_post_exit": {},
            "output": {
                "stdout_observed_bytes": bytes.len(),
                "stderr_observed_bytes": 0,
                "stdout_sha256": "digest",
                "stderr_sha256": "digest",
                "stdout_base64": encoded,
            },
            "provider_effect_count": 0,
            "invocation_digest": "invocation",
            "result_digest": "result",
        }))
        .expect("compact validated receipt");
        assert!(compact.pointer("/output/stdout_base64").is_none());
        assert_eq!(
            compact
                .pointer("/output/stdout_payload_retained")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            compact.get("result_digest").and_then(Value::as_str),
            Some("result")
        );
        assert_eq!(compact.get("outcome").and_then(Value::as_str), Some("PASS"));
        assert!(compact.get("outer_cleanup").is_some_and(Value::is_null));

        let transport = compact_wsl_git_result(&serde_json::json!({
            "schema": "lattice.wsl2-verifier-transport-failure/1.0",
            "status": "FAILED",
            "outcome": "TRANSPORT_ERROR",
            "retryable": true,
            "task_ref": "task",
            "attempt": 1,
            "worktree_ref": "worktree",
            "role": "GIT",
            "execution_environment_ref": "environment",
            "repository_head": "head",
            "credential_seal_digest": "credential",
            "verifier_identity": {"command_digest": "command"},
            "unit": "unit",
            "process_fence": "fence",
            "continuation": {"retry_of": null, "reconnect_of": null},
            "transport_evidence": {"evidence_digest": "evidence"},
            "outer_cleanup": {"reason": "TRANSPORT_ERROR"},
            "outer_post_exit": {"populated": null},
            "provider_effect_count": 0,
            "invocation_digest": "invocation",
            "result_digest": "result",
        }))
        .expect("compact transport receipt");
        assert_eq!(
            transport.get("outcome").and_then(Value::as_str),
            Some("TRANSPORT_ERROR")
        );
        assert_eq!(
            transport.get("process_fence").and_then(Value::as_str),
            Some("fence")
        );
        assert_eq!(
            transport.get("retryable").and_then(Value::as_bool),
            Some(true)
        );
        assert!(transport.get("output").is_none());
    }

    #[test]
    fn wsl_git_environment_rejects_unclosed_credential_keys() {
        let failure = wsl_git_environment(
            &[(
                OsString::from("OPENAI_API_KEY"),
                OsString::from("must-not-cross"),
            )],
            "Ubuntu",
        )
        .expect_err("ambient credential key must fail closed");
        assert_eq!(
            failure.code(),
            "LATTICE_MANAGED_VERIFIER_GIT_ENVIRONMENT_REJECTED"
        );
    }

    #[test]
    fn run_process_child_fixture() {
        let Ok(mode) = env::var("LATTICE_MANAGED_VERIFIER_PROCESS_FIXTURE") else {
            return;
        };
        match mode.as_str() {
            "bounded" => print!("bounded-process-output"),
            "secret" => print!("token=must-never-reach-a-temp-file"),
            "overflow" => {
                std::io::stdout()
                    .write_all(&vec![b'x'; MAX_GIT_OUTPUT_BYTES + 1])
                    .expect("fixture stdout");
            }
            "timeout" => thread::sleep(Duration::from_secs(30)),
            other => panic!("unknown process fixture {other}"),
        }
    }

    #[test]
    fn process_output_is_memory_only_bounded_and_timeout_reaped() {
        let _serial = PROCESS_TEST_SERIAL.lock().expect("process test serial");
        assert_eq!(ACTIVE_PROCESS_IO_THREADS.load(Ordering::Acquire), 0);
        let executable = env::current_exe().expect("current test executable");
        let current_directory = env::current_dir().expect("current directory");
        let control = env::temp_dir().join(format!(
            "lattice-managed-verifier-process-test-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&control).expect("process test control");
        let args = [
            OsString::from("--exact"),
            OsString::from("managed_verifier::tests::run_process_child_fixture"),
            OsString::from("--nocapture"),
        ];
        let runtime_environment = |mode: &str| {
            let mut environment = vec![(
                OsString::from("LATTICE_MANAGED_VERIFIER_PROCESS_FIXTURE"),
                OsString::from(mode),
            )];
            for key in ["SystemRoot", "WINDIR"] {
                if let Some(value) = env::var_os(key) {
                    environment.push((OsString::from(key), value));
                }
            }
            environment
        };

        let bounded = run_process(
            &executable,
            &args,
            &current_directory,
            &runtime_environment("bounded"),
            None,
            Duration::from_secs(10),
            &control,
            true,
            true,
        )
        .expect("bounded child");
        assert!(bounded.status.success());
        assert!(String::from_utf8_lossy(&bounded.stdout).contains("bounded-process-output"));
        let secret = run_process(
            &executable,
            &args,
            &current_directory,
            &runtime_environment("secret"),
            None,
            Duration::from_secs(10),
            &control,
            true,
            true,
        )
        .expect("secret stays in bounded memory");
        assert!(secret.status.success());
        assert!(
            fs::read_dir(&control)
                .expect("control listing")
                .next()
                .is_none(),
            "process output leaked to a verifier temp file"
        );
        let overflow = run_process(
            &executable,
            &args,
            &current_directory,
            &runtime_environment("overflow"),
            None,
            Duration::from_secs(10),
            &control,
            true,
            true,
        )
        .expect_err("oversized output");
        assert_eq!(overflow.code(), "LATTICE_MANAGED_VERIFIER_OUTPUT_LIMIT");
        let timeout = run_process(
            &executable,
            &args,
            &current_directory,
            &runtime_environment("timeout"),
            None,
            Duration::from_millis(100),
            &control,
            true,
            true,
        )
        .expect_err("timed out child");
        assert_eq!(timeout.code(), "LATTICE_MANAGED_VERIFIER_PROCESS_TIMEOUT");
        assert_eq!(
            ACTIVE_PROCESS_IO_THREADS.load(Ordering::Acquire),
            0,
            "reader/writer thread remained active after bounded cleanup"
        );
        fs::remove_dir(&control).expect("empty process test control");
    }
}
