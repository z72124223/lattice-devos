use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

use lattice_approval_verifier::{
    ApprovalVerifierCheckpoint, EXECUTION_AUTHORITY_SCHEMA, ExecutionAuthorityInput,
    ExecutionAuthoritySource, ExecutionCapability, FakeApprovalVerifier,
    UntrustedExecutionAuthority, VerifiedApprovalExecutionContext, VerifiedExecutionAuthority,
    reverify_verified_approval_execution_authority, verify_untrusted_execution_authority,
};
use lattice_artifact_store::{
    MANAGED_EVIDENCE_RECORD_SCHEMA, ManagedEvidenceInput, ManagedEvidenceKind,
    UntrustedManagedEvidence, VerifiedManagedEvidence, verify_untrusted_managed_evidence,
};
use lattice_contracts::{
    ApprovalAuthorityReceipt, AttemptId, CONTRACT_VERSION, ContentDigest, ProjectId,
    ProjectSnapshotId, RuntimeKind, TASK_LEDGER_PRODUCER_ID, TASK_LEDGER_PRODUCER_VERSION, TaskId,
    TaskLedgerStreamHead, TaskLedgerStreamIdentity, task_ingress_text_contains_recognized_secret,
};
use lattice_foreman_state::{ExternalCostBudget, WorkerBudget};
use lattice_task_ledger::{
    CommandId, CorrelationId, ModelReason, ReasoningEffort, TASK_EXECUTION_BINDING_RECORD_SCHEMA,
    TASK_VERIFICATION_RECORD_SCHEMA, TaskRuntimeAppendMetadata, TaskRuntimeEventLink,
    UntrustedTaskExecutionBinding, UntrustedTaskVerificationRow, UntrustedWorkerAttemptRow,
    UntrustedWorkerObservationRow, VerificationOutcome, VerifiedStream,
    VerifiedTaskExecutionBinding, VerifiedTaskVerificationRecord, VerifiedWorkerAttemptRecord,
    VerifiedWorkerObservationRecord, WORKER_ATTEMPT_RECORD_SCHEMA,
    WORKER_OBSERVATION_RECORD_SCHEMA, WorkerModel, WorkerObservationKind,
    foreman_coordination_identity,
};
use postgres::{Client, Row};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::MAX_ACTIVE_TASK_REPLAY_ROWS;
use crate::setup::{
    ExtensionCatalogEvidence, ExtensionDatabaseRole, ExtensionSetupError, ExtensionTarget,
    verify_extension,
};

/// Closed adapter failure class without database text or parameters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterErrorKind {
    Setup,
    InvalidInput,
    /// A fixed server-owned claim policy rejected the request before mutation.
    ClaimRejected,
    /// A fixed server-owned evidence quota rejected the append before mutation.
    QuotaRejected,
    Database,
    CorruptReplay,
}

/// Closed operation labels permitted on a database failure diagnostic.
///
/// These labels never carry a query, parameter, connection target, or server message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterDatabaseStage {
    TaskPromotion,
    PreparationObservation,
}

impl AdapterDatabaseStage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TaskPromotion => "RECORD_TASK_PROMOTION",
            Self::PreparationObservation => "RECORD_PREPARATION_OBSERVATION",
        }
    }
}

/// Secret-free adapter error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdapterError {
    kind: AdapterErrorKind,
    code: &'static str,
    database_stage: Option<AdapterDatabaseStage>,
    sqlstate: Option<&'static str>,
}

impl AdapterError {
    #[must_use]
    pub const fn kind(self) -> AdapterErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }

    /// Returns the fixed operation label for a sanitized database diagnostic.
    #[must_use]
    pub const fn database_stage(self) -> Option<AdapterDatabaseStage> {
        self.database_stage
    }

    /// Returns one allowlisted SQLSTATE, or `OTHER` for an unclassified database state.
    #[must_use]
    pub const fn sqlstate(self) -> Option<&'static str> {
        self.sqlstate
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for AdapterError {}

impl From<ExtensionSetupError> for AdapterError {
    fn from(_: ExtensionSetupError) -> Self {
        adapter_error(AdapterErrorKind::Setup, "FOREMAN_ADAPTER_SETUP_FAILED")
    }
}

/// Append outcome shared by non-claim child records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendDisposition {
    Inserted,
    ExactReplay,
}

const WSL2_EXECUTION_ENVIRONMENT_SCHEMA: &str = "lattice.execution-environment.wsl2-linux/1.1";
const EXECUTION_ENVIRONMENT_REF_PREFIX: &str = "execution-environment:sha256:";
const MAX_EXECUTION_ENVIRONMENT_JSON_BYTES: usize = 16_384;
const MAX_EXECUTION_ENVIRONMENT_STRING_SCAN_DEPTH: usize = 16;
const MAX_EXECUTION_ENVIRONMENT_STRING_SCAN_NODES: usize = 512;
const MAX_EXECUTION_ENVIRONMENT_STRING_LEAF_BYTES: usize = 4_096;
pub const NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF: &str =
    "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001";

/// Closed execution-domain kind currently admitted by the durable WSL2 lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionEnvironmentKind {
    Wsl2Linux,
}

impl ExecutionEnvironmentKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wsl2Linux => "WSL2_LINUX",
        }
    }
}

/// Secret-free credential authority kind retained with one execution domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialAuthorityKind {
    LinuxKeyring,
}

impl CredentialAuthorityKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LinuxKeyring => "LINUX_KEYRING",
        }
    }

    /// Parses the closed persisted credential-authority kind.
    ///
    /// # Errors
    ///
    /// Rejects every unknown or differently-cased value.
    pub fn from_persisted(value: &str) -> Result<Self, AdapterError> {
        match value {
            "LINUX_KEYRING" => Ok(Self::LinuxKeyring),
            _ => Err(corrupt_error()),
        }
    }
}

/// Closed process-subtree authority kind admitted by WSL2 production.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionProcessFenceKind {
    SystemdUserServiceCgroupV2,
}

impl ExecutionProcessFenceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SystemdUserServiceCgroupV2 => "SYSTEMD_USER_SERVICE_CGROUP_V2",
        }
    }

    fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "SYSTEMD_USER_SERVICE_CGROUP_V2" => Ok(Self::SystemdUserServiceCgroupV2),
            _ => Err(corrupt_error()),
        }
    }
}

/// Typed digest reference for one exact execution-environment descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionEnvironmentRef(String);

impl ExecutionEnvironmentRef {
    fn from_descriptor_digest(digest: &ContentDigest) -> Result<Self, AdapterError> {
        if is_zero(digest) {
            return Err(input_error());
        }
        Ok(Self(format!(
            "{EXECUTION_ENVIRONMENT_REF_PREFIX}{}",
            digest.as_str()
        )))
    }

    fn parse(value: String) -> Result<Self, AdapterError> {
        let Some(digest) = value.strip_prefix(EXECUTION_ENVIRONMENT_REF_PREFIX) else {
            return Err(corrupt_error());
        };
        let digest = ContentDigest::from_sha256(digest.to_owned()).map_err(|_| corrupt_error())?;
        let expected = Self::from_descriptor_digest(&digest).map_err(|_| corrupt_error())?;
        if expected.0 != value {
            return Err(corrupt_error());
        }
        Ok(expected)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exact path, version, and content identity for one execution-domain tool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionToolIdentity {
    path: String,
    version: String,
    digest: ContentDigest,
}

/// Exact canonical path and digest for a versionless helper/script identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionFileIdentity {
    path: String,
    digest: ContentDigest,
}

impl ExecutionFileIdentity {
    fn new(path: impl Into<String>, digest: ContentDigest) -> Result<Self, AdapterError> {
        let path = path.into();
        if !canonical_linux_path(&path) || is_zero(&digest) {
            return Err(input_error());
        }
        Ok(Self { path, digest })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

impl ExecutionToolIdentity {
    /// Builds one bounded, secret-free executable or script identity.
    ///
    /// Linux/Windows canonical path rules are applied by the owning descriptor.
    ///
    /// # Errors
    ///
    /// Rejects empty, control-bearing, URL-shaped, overlong, or zero-digest input.
    pub fn new(
        path: impl Into<String>,
        version: impl Into<String>,
        digest: ContentDigest,
    ) -> Result<Self, AdapterError> {
        let path = path.into();
        let version = version.into();
        if !bounded_identity_text(&path, 1_024)
            || !bounded_identity_text(&version, 128)
            || is_zero(&digest)
        {
            return Err(input_error());
        }
        Ok(Self {
            path,
            version,
            digest,
        })
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// Immutable WSL2/Linux execution-environment descriptor selected by one attempt.
///
/// It retains only typed locators, tool identities, and credential-authority
/// kind/digest. It never contains credentials, account identity, prompts, or
/// ambient environment values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionEnvironmentDescriptor {
    kind: ExecutionEnvironmentKind,
    distribution: String,
    distribution_version: String,
    distribution_identity_ref: String,
    distribution_identity_digest: ContentDigest,
    gateway: ExecutionToolIdentity,
    linux_repository_path: String,
    linux_codex_home_path: String,
    codex_config_digest: ContentDigest,
    repository_head: String,
    repository_identity_digest: ContentDigest,
    launcher: ExecutionToolIdentity,
    node: ExecutionToolIdentity,
    npm: ExecutionToolIdentity,
    git: ExecutionToolIdentity,
    supervisor: ExecutionFileIdentity,
    sandbox: ExecutionToolIdentity,
    sandbox_helper: ExecutionToolIdentity,
    cargo: ExecutionToolIdentity,
    rustc: ExecutionToolIdentity,
    rustdoc: ExecutionToolIdentity,
    keyring_library_manifest_ref: String,
    keyring_library_manifest_digest: ContentDigest,
    credential_authority_kind: CredentialAuthorityKind,
    credential_authority_ref: String,
    credential_authority_digest: ContentDigest,
    process_fence_kind: ExecutionProcessFenceKind,
    systemd_run: ExecutionToolIdentity,
    systemctl: ExecutionToolIdentity,
    supervisor_bootstrap_node: ExecutionToolIdentity,
    immutable_probe_lsattr: ExecutionToolIdentity,
    noninteractive_root_probe: ExecutionToolIdentity,
    process_fence_identity_ref: String,
    process_fence_identity_digest: ContentDigest,
    verification_toolchain_identity_ref: String,
    verification_toolchain_identity_digest: ContentDigest,
    verification_task_ref: ContentDigest,
    immutable_snapshot_ref: String,
    immutable_snapshot_digest: ContentDigest,
    sandbox_policy_ref: String,
    sandbox_policy_digest: ContentDigest,
    privilege_boundary_ref: String,
    privilege_boundary_digest: ContentDigest,
    path_mapping_windows_path: String,
    path_mapping_linux_path: String,
    path_mapping_digest_ref: String,
    path_mapping_digest: ContentDigest,
    canonical_json: String,
    execution_domain_digest: ContentDigest,
    environment_ref: ExecutionEnvironmentRef,
}

impl ExecutionEnvironmentDescriptor {
    /// Validates, canonicalizes, and rehashes one production WSL2 descriptor.
    ///
    /// # Errors
    ///
    /// Rejects legacy/unknown shape, noncanonical paths, inconsistent nested
    /// identities, secrets/extra keys, or a top-level identity mismatch.
    #[allow(clippy::too_many_lines)]
    pub fn from_json(json: &str) -> Result<Self, AdapterError> {
        if json.is_empty() || json.len() > MAX_EXECUTION_ENVIRONMENT_JSON_BYTES {
            return Err(input_error());
        }
        let value: Value = serde_json::from_str(json).map_err(|_| input_error())?;
        if !execution_environment_string_leaves_are_secret_free(&value) {
            return Err(input_error());
        }
        let root = exact_json_object(
            &value,
            &[
                "schema",
                "kind",
                "distribution",
                "distribution_identity",
                "gateway",
                "linux",
                "credential_authority",
                "process_fence",
                "verification_toolchain",
                "immutable_snapshot",
                "sandbox_policy",
                "privilege_boundary",
                "path_mapping",
                "identity_digest",
            ],
        )?;
        if json_string(root, "schema")? != WSL2_EXECUTION_ENVIRONMENT_SCHEMA
            || json_string(root, "kind")? != ExecutionEnvironmentKind::Wsl2Linux.as_str()
        {
            return Err(input_error());
        }
        let distribution = json_string(root, "distribution")?.to_owned();
        if !safe_distribution(&distribution) {
            return Err(input_error());
        }

        let distribution_value = root.get("distribution_identity").ok_or_else(input_error)?;
        let distribution_object = exact_json_object(
            distribution_value,
            &[
                "os_id",
                "os_version_id",
                "os_version_codename",
                "os_release_sha256",
                "kernel_release",
                "identity_digest",
            ],
        )?;
        let os_id = json_string(distribution_object, "os_id")?;
        let distribution_version = json_string(distribution_object, "os_version_id")?.to_owned();
        let os_version_codename = json_string(distribution_object, "os_version_codename")?;
        let kernel_release = json_string(distribution_object, "kernel_release")?;
        parse_raw_digest(json_string(distribution_object, "os_release_sha256")?)?;
        if !safe_lower_identity(os_id)
            || !numeric_dotted_version(&distribution_version)
            || !safe_lower_identity(os_version_codename)
            || !bounded_identity_text(kernel_release, 128)
            || !kernel_release.ends_with("microsoft-standard-WSL2")
        {
            return Err(input_error());
        }
        let distribution_identity_ref =
            json_string(distribution_object, "identity_digest")?.to_owned();
        let distribution_identity_digest =
            parse_typed_digest(&distribution_identity_ref, "wsl2-distribution")?;
        let mut distribution_subject = distribution_object.clone();
        distribution_subject.remove("identity_digest");
        distribution_subject.insert(
            "distribution".to_owned(),
            Value::String(distribution.clone()),
        );
        if distribution_identity_ref
            != typed_json_identity("wsl2-distribution", &Value::Object(distribution_subject))?
        {
            return Err(input_error());
        }

        let gateway_object = exact_json_object(
            root.get("gateway").ok_or_else(input_error)?,
            &["windows_path", "version", "sha256"],
        )?;
        let gateway = ExecutionToolIdentity::new(
            json_string(gateway_object, "windows_path")?,
            json_string(gateway_object, "version")?,
            parse_raw_digest(json_string(gateway_object, "sha256")?)?,
        )?;
        if !canonical_windows_wsl_gateway_path(gateway.path())
            || !closed_tool_version(ExecutionToolVersionKind::WslGateway, gateway.version())
        {
            return Err(input_error());
        }

        let linux_object = exact_json_object(
            root.get("linux").ok_or_else(input_error)?,
            &[
                "launcher_path",
                "launcher_version",
                "launcher_sha256",
                "node_path",
                "node_version",
                "node_sha256",
                "git_path",
                "git_version",
                "git_sha256",
                "supervisor_path",
                "supervisor_sha256",
                "codex_home",
                "config_digest",
                "cwd",
                "repository_head",
                "repository_identity",
                "dbus_run_session_path",
                "dbus_run_session_sha256",
                "setsid_path",
                "setsid_sha256",
                "keyring_daemon_path",
                "keyring_daemon_sha256",
                "keyring_library_path",
                "keyring_library_manifest_digest",
                "xdg_runtime_dir",
            ],
        )?;
        let launcher = json_tool_identity(linux_object, "launcher")?;
        let node = json_tool_identity(linux_object, "node")?;
        let git = json_tool_identity(linux_object, "git")?;
        if !closed_tool_version(ExecutionToolVersionKind::Codex, launcher.version())
            || !closed_tool_version(ExecutionToolVersionKind::Node, node.version())
            || !closed_tool_version(ExecutionToolVersionKind::Git, git.version())
        {
            return Err(input_error());
        }
        let supervisor = ExecutionFileIdentity::new(
            json_string(linux_object, "supervisor_path")?,
            parse_raw_digest(json_string(linux_object, "supervisor_sha256")?)?,
        )?;
        for prefix in ["dbus_run_session", "setsid", "keyring_daemon"] {
            json_file_identity(linux_object, prefix)?;
        }
        let linux_repository_path = json_string(linux_object, "cwd")?.to_owned();
        let linux_codex_home_path = json_string(linux_object, "codex_home")?.to_owned();
        let keyring_library_path = json_string(linux_object, "keyring_library_path")?;
        let keyring_library_manifest_ref =
            json_string(linux_object, "keyring_library_manifest_digest")?.to_owned();
        let keyring_library_manifest_digest =
            parse_typed_digest(&keyring_library_manifest_ref, "keyring-library-manifest")?;
        let xdg_runtime_dir = json_string(linux_object, "xdg_runtime_dir")?;
        let codex_config_digest =
            parse_typed_digest(json_string(linux_object, "config_digest")?, "codex-config")?;
        let repository_identity_digest = parse_typed_digest(
            json_string(linux_object, "repository_identity")?,
            "repository",
        )?;
        let repository_head = json_string(linux_object, "repository_head")?.to_owned();
        if !canonical_linux_home_path(&linux_repository_path)
            || !canonical_linux_home_path(&linux_codex_home_path)
            || !canonical_linux_home_path(keyring_library_path)
            || !canonical_linux_path(xdg_runtime_dir)
            || !canonical_linux_path(git.path())
            || !lower_hex(&repository_head, 40)
        {
            return Err(input_error());
        }

        let credential_object = exact_json_object(
            root.get("credential_authority").ok_or_else(input_error)?,
            &["kind", "authority_digest"],
        )?;
        let credential_authority_kind = match json_string(credential_object, "kind")? {
            "LINUX_KEYRING" => CredentialAuthorityKind::LinuxKeyring,
            _ => return Err(input_error()),
        };
        let credential_authority_ref =
            json_string(credential_object, "authority_digest")?.to_owned();
        let credential_authority_digest =
            parse_typed_digest(&credential_authority_ref, "wsl2-credential-authority")?;
        let credential_subject = json_object([
            (
                "kind",
                Value::String(credential_authority_kind.as_str().to_owned()),
            ),
            (
                "distribution_identity_ref",
                Value::String(distribution_identity_ref.clone()),
            ),
            ("codex_home", Value::String(linux_codex_home_path.clone())),
            (
                "config_digest",
                Value::String(json_string(linux_object, "config_digest")?.to_owned()),
            ),
            (
                "keyring_daemon_path",
                Value::String(json_string(linux_object, "keyring_daemon_path")?.to_owned()),
            ),
            (
                "keyring_daemon_sha256",
                Value::String(json_string(linux_object, "keyring_daemon_sha256")?.to_owned()),
            ),
            (
                "keyring_library_path",
                Value::String(keyring_library_path.to_owned()),
            ),
            (
                "keyring_library_manifest_digest",
                Value::String(keyring_library_manifest_ref.clone()),
            ),
            ("xdg_runtime_dir", Value::String(xdg_runtime_dir.to_owned())),
        ]);
        if credential_authority_ref
            != typed_json_identity("wsl2-credential-authority", &credential_subject)?
        {
            return Err(input_error());
        }

        let process_fence_value = root.get("process_fence").ok_or_else(input_error)?;
        let process_fence_object = exact_json_object(
            process_fence_value,
            &[
                "schema",
                "kind",
                "systemd_run_path",
                "systemd_run_version",
                "systemd_run_sha256",
                "systemctl_path",
                "systemctl_version",
                "systemctl_sha256",
                "cgroup_mount",
                "user_runtime_dir",
                "unit_prefix",
                "supervisor_bootstrap_node",
                "immutable_probe_lsattr",
                "noninteractive_root_probe",
                "identity_digest",
            ],
        )?;
        if json_string(process_fence_object, "schema")? != "lattice.wsl2-cgroup-v2-fence/1.0"
            || json_string(process_fence_object, "cgroup_mount")? != "/sys/fs/cgroup"
            || json_string(process_fence_object, "user_runtime_dir")? != xdg_runtime_dir
            || !valid_user_runtime_dir(xdg_runtime_dir)
            || !valid_unit_prefix(json_string(process_fence_object, "unit_prefix")?)
        {
            return Err(input_error());
        }
        let process_fence_kind =
            ExecutionProcessFenceKind::parse(json_string(process_fence_object, "kind")?)
                .map_err(|_| input_error())?;
        let systemd_run = json_tool_identity(process_fence_object, "systemd_run")?;
        let systemctl = json_tool_identity(process_fence_object, "systemctl")?;
        let supervisor_bootstrap_node =
            json_nested_tool_identity(process_fence_object, "supervisor_bootstrap_node")?;
        let immutable_probe_lsattr =
            json_nested_tool_identity(process_fence_object, "immutable_probe_lsattr")?;
        let noninteractive_root_probe =
            json_nested_tool_identity(process_fence_object, "noninteractive_root_probe")?;
        if !canonical_linux_path(systemd_run.path())
            || !canonical_linux_path(systemctl.path())
            || !closed_tool_version(ExecutionToolVersionKind::Systemd, systemd_run.version())
            || !closed_tool_version(ExecutionToolVersionKind::Systemd, systemctl.version())
            || supervisor_bootstrap_node.path() != "/usr/bin/node"
            || !closed_tool_version(
                ExecutionToolVersionKind::Node,
                supervisor_bootstrap_node.version(),
            )
            || immutable_probe_lsattr.path() != "/usr/bin/lsattr"
            || !closed_tool_version(
                ExecutionToolVersionKind::Lsattr,
                immutable_probe_lsattr.version(),
            )
            || noninteractive_root_probe.path() != "/usr/bin/sudo"
            || !closed_tool_version(
                ExecutionToolVersionKind::Sudo,
                noninteractive_root_probe.version(),
            )
        {
            return Err(input_error());
        }
        let process_fence_identity_ref =
            json_string(process_fence_object, "identity_digest")?.to_owned();
        let process_fence_identity_digest =
            parse_typed_digest(&process_fence_identity_ref, "wsl2-process-fence-authority")?;
        let mut process_fence_subject = process_fence_object.clone();
        process_fence_subject.remove("identity_digest");
        process_fence_subject.insert(
            "distribution_identity_ref".to_owned(),
            Value::String(distribution_identity_ref.clone()),
        );
        if process_fence_identity_ref
            != typed_json_identity(
                "wsl2-process-fence-authority",
                &Value::Object(process_fence_subject),
            )?
        {
            return Err(input_error());
        }

        let toolchain_value = root.get("verification_toolchain").ok_or_else(input_error)?;
        let toolchain_object = exact_json_object(
            toolchain_value,
            &[
                "schema",
                "task_ref",
                "task_root",
                "isolation_root",
                "owner_uid",
                "home_dir",
                "temp_dir",
                "npm_cache",
                "cargo_home",
                "cargo_target_dir",
                "cargo_host",
                "npm",
                "cargo",
                "rustc",
                "rustdoc",
                "sandbox",
                "sandbox_helper",
                "identity_digest",
            ],
        )?;
        let verification_owner_uid = json_u64(toolchain_object, "owner_uid")?;
        if json_string(toolchain_object, "schema")? != "lattice.wsl2-verification-toolchain/1.0"
            || !lower_hex(json_string(toolchain_object, "task_ref")?, 64)
            || verification_owner_uid == 0
            || !safe_distribution(json_string(toolchain_object, "cargo_host")?)
        {
            return Err(input_error());
        }
        let verification_task_ref = parse_raw_digest(json_string(toolchain_object, "task_ref")?)?;
        let task_root = json_string(toolchain_object, "task_root")?;
        let isolation_root = json_string(toolchain_object, "isolation_root")?;
        if !canonical_linux_home_path(task_root)
            || !linux_descendant(task_root, isolation_root)
            || !linux_descendant(task_root, &linux_repository_path)
            || !linux_repository_path.starts_with(&format!("{task_root}/managed-worktrees/"))
            || linux_codex_home_path != format!("{task_root}/codex-home")
            || !linux_descendant(task_root, launcher.path())
            || !linux_descendant(task_root, node.path())
            || !node_version_at_least(node.version(), [24, 15, 0])
            || !linux_descendant(task_root, supervisor.path())
        {
            return Err(input_error());
        }
        for field in [
            "home_dir",
            "temp_dir",
            "npm_cache",
            "cargo_home",
            "cargo_target_dir",
        ] {
            if !linux_descendant(isolation_root, json_string(toolchain_object, field)?) {
                return Err(input_error());
            }
        }
        let npm = json_nested_tool_identity(toolchain_object, "npm")?;
        let cargo = json_nested_tool_identity(toolchain_object, "cargo")?;
        let rustc = json_nested_tool_identity(toolchain_object, "rustc")?;
        let rustdoc = json_nested_tool_identity(toolchain_object, "rustdoc")?;
        let sandbox = json_nested_tool_identity(toolchain_object, "sandbox")?;
        let sandbox_helper = json_nested_tool_identity(toolchain_object, "sandbox_helper")?;
        if !closed_tool_version(ExecutionToolVersionKind::Npm, npm.version())
            || !closed_tool_version(ExecutionToolVersionKind::Cargo, cargo.version())
            || !closed_tool_version(ExecutionToolVersionKind::Rustc, rustc.version())
            || !closed_tool_version(ExecutionToolVersionKind::Rustdoc, rustdoc.version())
            || !closed_tool_version(ExecutionToolVersionKind::Codex, sandbox.version())
            || !closed_tool_version(
                ExecutionToolVersionKind::Bubblewrap,
                sandbox_helper.version(),
            )
        {
            return Err(input_error());
        }
        for tool in [&npm, &cargo, &rustc, &rustdoc, &sandbox] {
            if !linux_descendant(task_root, tool.path()) {
                return Err(input_error());
            }
        }
        if sandbox.path() != launcher.path()
            || sandbox.version() != launcher.version()
            || sandbox.digest() != launcher.digest()
            || sandbox_helper.path() != "/usr/bin/bwrap"
        {
            return Err(input_error());
        }
        let verification_toolchain_identity_ref =
            json_string(toolchain_object, "identity_digest")?.to_owned();
        let verification_toolchain_identity_digest = parse_typed_digest(
            &verification_toolchain_identity_ref,
            "wsl2-verification-toolchain",
        )?;
        let mut toolchain_subject = toolchain_object.clone();
        toolchain_subject.remove("identity_digest");
        if verification_toolchain_identity_ref
            != typed_json_identity(
                "wsl2-verification-toolchain",
                &Value::Object(toolchain_subject),
            )?
        {
            return Err(input_error());
        }

        let immutable_snapshot_value = root.get("immutable_snapshot").ok_or_else(input_error)?;
        let immutable_snapshot_object = exact_json_object(
            immutable_snapshot_value,
            &[
                "schema",
                "task_root_path",
                "task_root_device",
                "task_root_inode",
                "task_root_owner_uid",
                "task_root_owner_gid",
                "task_root_mode",
                "task_root_immutable",
                "trees",
                "snapshot_digest",
            ],
        )?;
        if json_string(immutable_snapshot_object, "schema")?
            != "lattice.wsl2-immutable-snapshot/1.0"
            || json_string(immutable_snapshot_object, "task_root_path")? != task_root
            || !canonical_nonzero_u64_text(json_string(
                immutable_snapshot_object,
                "task_root_device",
            )?)
            || !canonical_nonzero_u64_text(json_string(
                immutable_snapshot_object,
                "task_root_inode",
            )?)
            || json_u64(immutable_snapshot_object, "task_root_owner_uid")? != 0
            || json_u64(immutable_snapshot_object, "task_root_owner_gid")? != 0
            || json_string(immutable_snapshot_object, "task_root_mode")? != "0555"
            || immutable_snapshot_object
                .get("task_root_immutable")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(input_error());
        }
        let trees_object = exact_json_object(
            immutable_snapshot_object
                .get("trees")
                .ok_or_else(input_error)?,
            &["codex", "supervisor_runtime", "node", "rust", "keyring"],
        )?;
        let (codex_tree_root, _) = json_tree_manifest(trees_object, "codex")?;
        let (supervisor_tree_root, _) = json_tree_manifest(trees_object, "supervisor_runtime")?;
        let (node_tree_root, _) = json_tree_manifest(trees_object, "node")?;
        let (rust_tree_root, _) = json_tree_manifest(trees_object, "rust")?;
        let (keyring_tree_root, _) = json_tree_manifest(trees_object, "keyring")?;
        let tree_roots = [
            codex_tree_root,
            supervisor_tree_root,
            node_tree_root,
            rust_tree_root,
            keyring_tree_root,
        ];
        if tree_roots.iter().enumerate().any(|(index, candidate)| {
            !linux_direct_child(task_root, candidate)
                || tree_roots[..index].iter().any(|prior| {
                    candidate == prior
                        || linux_descendant(prior, candidate)
                        || linux_descendant(candidate, prior)
                })
        }) || launcher.path() != format!("{codex_tree_root}/bin/codex")
            || !linux_descendant(codex_tree_root, sandbox.path())
            || !linux_descendant(supervisor_tree_root, supervisor.path())
            || !linux_descendant(node_tree_root, node.path())
            || !linux_descendant(node_tree_root, npm.path())
            || !linux_descendant(rust_tree_root, cargo.path())
            || !linux_descendant(rust_tree_root, rustc.path())
            || !linux_descendant(rust_tree_root, rustdoc.path())
            || json_string(linux_object, "keyring_daemon_path")?
                != format!("{keyring_tree_root}/root/usr/bin/gnome-keyring-daemon")
            || keyring_library_path != format!("{keyring_tree_root}/packages")
        {
            return Err(input_error());
        }
        let immutable_snapshot_ref =
            json_string(immutable_snapshot_object, "snapshot_digest")?.to_owned();
        let immutable_snapshot_digest =
            parse_typed_digest(&immutable_snapshot_ref, "wsl2-immutable-snapshot")?;
        let mut immutable_snapshot_subject = immutable_snapshot_object.clone();
        immutable_snapshot_subject.remove("snapshot_digest");
        if immutable_snapshot_ref
            != typed_json_identity(
                "wsl2-immutable-snapshot",
                &Value::Object(immutable_snapshot_subject),
            )?
        {
            return Err(input_error());
        }

        let sandbox_policy_object = exact_json_object(
            root.get("sandbox_policy").ok_or_else(input_error)?,
            &["schema", "policy_digest"],
        )?;
        if json_string(sandbox_policy_object, "schema")? != "lattice.wsl2-sandbox-policy/1.0" {
            return Err(input_error());
        }
        let sandbox_policy_ref = json_string(sandbox_policy_object, "policy_digest")?.to_owned();
        let sandbox_policy_digest = parse_typed_digest(&sandbox_policy_ref, "wsl2-sandbox-policy")?;
        let expected_sandbox_policy_ref = typed_json_identity(
            "wsl2-sandbox-policy",
            &wsl2_sandbox_policy_template(
                &linux_repository_path,
                &linux_codex_home_path,
                xdg_runtime_dir,
                task_root,
                toolchain_object,
            )?,
        )?;
        if sandbox_policy_ref != expected_sandbox_policy_ref {
            return Err(input_error());
        }

        let privilege_boundary_object = exact_json_object(
            root.get("privilege_boundary").ok_or_else(input_error)?,
            &[
                "schema",
                "effective_uid",
                "effective_gid",
                "effective_capabilities_digest",
                "noninteractive_root_unavailable",
                "boundary_digest",
            ],
        )?;
        if json_string(privilege_boundary_object, "schema")?
            != "lattice.wsl2-privilege-boundary/1.0"
            || json_u64(privilege_boundary_object, "effective_uid")? != verification_owner_uid
            || json_u64(privilege_boundary_object, "effective_gid")? == 0
            || privilege_boundary_object
                .get("noninteractive_root_unavailable")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(input_error());
        }
        parse_typed_digest(
            json_string(privilege_boundary_object, "effective_capabilities_digest")?,
            "linux-capabilities",
        )?;
        let privilege_boundary_ref =
            json_string(privilege_boundary_object, "boundary_digest")?.to_owned();
        let privilege_boundary_digest =
            parse_typed_digest(&privilege_boundary_ref, "wsl2-privilege-boundary")?;
        let mut privilege_boundary_subject = privilege_boundary_object.clone();
        privilege_boundary_subject.remove("boundary_digest");
        if privilege_boundary_ref
            != typed_json_identity(
                "wsl2-privilege-boundary",
                &Value::Object(privilege_boundary_subject),
            )?
        {
            return Err(input_error());
        }

        let path_mapping_object = exact_json_object(
            root.get("path_mapping").ok_or_else(input_error)?,
            &["windows_path", "linux_path", "digest"],
        )?;
        let path_mapping_windows_path =
            json_string(path_mapping_object, "windows_path")?.to_owned();
        let path_mapping_linux_path = json_string(path_mapping_object, "linux_path")?.to_owned();
        if path_mapping_linux_path != linux_repository_path
            || windows_wsl_path_to_linux(&path_mapping_windows_path, &distribution).as_deref()
                != Some(linux_repository_path.as_str())
        {
            return Err(input_error());
        }
        let path_mapping_digest_ref = json_string(path_mapping_object, "digest")?.to_owned();
        let path_mapping_digest = parse_typed_digest(&path_mapping_digest_ref, "path-mapping")?;
        let expected_path_mapping_ref = typed_json_identity(
            "path-mapping",
            &json_object([
                ("distribution", Value::String(distribution.clone())),
                (
                    "windows_path",
                    Value::String(path_mapping_windows_path.clone()),
                ),
                ("linux_path", Value::String(path_mapping_linux_path.clone())),
                (
                    "repository_identity",
                    Value::String(json_string(linux_object, "repository_identity")?.to_owned()),
                ),
                ("repository_head", Value::String(repository_head.clone())),
            ]),
        )?;
        if path_mapping_digest_ref != expected_path_mapping_ref {
            return Err(input_error());
        }

        let canonical_json = canonical_json_value(&value)?;
        let mut subject = root.clone();
        subject.remove("identity_digest");
        let execution_domain_digest =
            sha256_content(canonical_json_value(&Value::Object(subject))?.as_bytes())?;
        let environment_ref =
            ExecutionEnvironmentRef::parse(json_string(root, "identity_digest")?.to_owned())
                .map_err(|_| input_error())?;
        let expected_ref =
            ExecutionEnvironmentRef::from_descriptor_digest(&execution_domain_digest)?;
        if environment_ref != expected_ref {
            return Err(input_error());
        }
        let kind = ExecutionEnvironmentKind::Wsl2Linux;
        Ok(Self {
            kind,
            distribution,
            distribution_version,
            distribution_identity_ref,
            distribution_identity_digest,
            gateway,
            linux_repository_path,
            linux_codex_home_path,
            codex_config_digest,
            repository_head,
            repository_identity_digest,
            launcher,
            node,
            npm,
            git,
            supervisor,
            sandbox,
            sandbox_helper,
            cargo,
            rustc,
            rustdoc,
            keyring_library_manifest_ref,
            keyring_library_manifest_digest,
            credential_authority_kind,
            credential_authority_ref,
            credential_authority_digest,
            process_fence_kind,
            systemd_run,
            systemctl,
            supervisor_bootstrap_node,
            immutable_probe_lsattr,
            noninteractive_root_probe,
            process_fence_identity_ref,
            process_fence_identity_digest,
            verification_toolchain_identity_ref,
            verification_toolchain_identity_digest,
            verification_task_ref,
            immutable_snapshot_ref,
            immutable_snapshot_digest,
            sandbox_policy_ref,
            sandbox_policy_digest,
            privilege_boundary_ref,
            privilege_boundary_digest,
            path_mapping_windows_path,
            path_mapping_linux_path,
            path_mapping_digest_ref,
            path_mapping_digest,
            canonical_json,
            execution_domain_digest,
            environment_ref,
        })
    }

    #[must_use]
    pub const fn descriptor_schema(&self) -> &'static str {
        WSL2_EXECUTION_ENVIRONMENT_SCHEMA
    }

    #[must_use]
    pub const fn kind(&self) -> ExecutionEnvironmentKind {
        self.kind
    }

    #[must_use]
    pub fn distribution(&self) -> &str {
        &self.distribution
    }

    #[must_use]
    pub fn distribution_version(&self) -> &str {
        &self.distribution_version
    }

    #[must_use]
    pub fn distribution_identity_ref(&self) -> &str {
        &self.distribution_identity_ref
    }

    #[must_use]
    pub const fn distribution_identity_digest(&self) -> &ContentDigest {
        &self.distribution_identity_digest
    }

    #[must_use]
    pub const fn gateway(&self) -> &ExecutionToolIdentity {
        &self.gateway
    }

    #[must_use]
    pub fn linux_repository_path(&self) -> &str {
        &self.linux_repository_path
    }

    #[must_use]
    pub fn linux_codex_home_path(&self) -> &str {
        &self.linux_codex_home_path
    }

    #[must_use]
    pub const fn codex_config_digest(&self) -> &ContentDigest {
        &self.codex_config_digest
    }

    #[must_use]
    pub fn repository_head(&self) -> &str {
        &self.repository_head
    }

    #[must_use]
    pub const fn repository_identity_digest(&self) -> &ContentDigest {
        &self.repository_identity_digest
    }

    #[must_use]
    pub const fn launcher(&self) -> &ExecutionToolIdentity {
        &self.launcher
    }

    #[must_use]
    pub const fn node(&self) -> &ExecutionToolIdentity {
        &self.node
    }

    #[must_use]
    pub const fn npm(&self) -> &ExecutionToolIdentity {
        &self.npm
    }

    #[must_use]
    pub const fn git(&self) -> &ExecutionToolIdentity {
        &self.git
    }

    #[must_use]
    pub const fn supervisor(&self) -> &ExecutionFileIdentity {
        &self.supervisor
    }

    #[must_use]
    pub const fn sandbox(&self) -> &ExecutionToolIdentity {
        &self.sandbox
    }

    #[must_use]
    pub const fn sandbox_helper(&self) -> &ExecutionToolIdentity {
        &self.sandbox_helper
    }

    #[must_use]
    pub const fn cargo(&self) -> &ExecutionToolIdentity {
        &self.cargo
    }

    #[must_use]
    pub const fn rustc(&self) -> &ExecutionToolIdentity {
        &self.rustc
    }

    #[must_use]
    pub const fn rustdoc(&self) -> &ExecutionToolIdentity {
        &self.rustdoc
    }

    #[must_use]
    pub fn keyring_library_manifest_ref(&self) -> &str {
        &self.keyring_library_manifest_ref
    }

    #[must_use]
    pub const fn keyring_library_manifest_digest(&self) -> &ContentDigest {
        &self.keyring_library_manifest_digest
    }

    #[must_use]
    pub const fn credential_authority_kind(&self) -> CredentialAuthorityKind {
        self.credential_authority_kind
    }

    #[must_use]
    pub fn credential_authority_ref(&self) -> &str {
        &self.credential_authority_ref
    }

    #[must_use]
    pub const fn credential_authority_digest(&self) -> &ContentDigest {
        &self.credential_authority_digest
    }

    #[must_use]
    pub const fn process_fence_kind(&self) -> ExecutionProcessFenceKind {
        self.process_fence_kind
    }

    #[must_use]
    pub const fn systemd_run(&self) -> &ExecutionToolIdentity {
        &self.systemd_run
    }

    #[must_use]
    pub const fn systemctl(&self) -> &ExecutionToolIdentity {
        &self.systemctl
    }

    #[must_use]
    pub const fn supervisor_bootstrap_node(&self) -> &ExecutionToolIdentity {
        &self.supervisor_bootstrap_node
    }

    #[must_use]
    pub const fn immutable_probe_lsattr(&self) -> &ExecutionToolIdentity {
        &self.immutable_probe_lsattr
    }

    #[must_use]
    pub const fn noninteractive_root_probe(&self) -> &ExecutionToolIdentity {
        &self.noninteractive_root_probe
    }

    #[must_use]
    pub fn process_fence_identity_ref(&self) -> &str {
        &self.process_fence_identity_ref
    }

    #[must_use]
    pub const fn process_fence_identity_digest(&self) -> &ContentDigest {
        &self.process_fence_identity_digest
    }

    #[must_use]
    pub fn verification_toolchain_identity_ref(&self) -> &str {
        &self.verification_toolchain_identity_ref
    }

    #[must_use]
    pub const fn verification_toolchain_identity_digest(&self) -> &ContentDigest {
        &self.verification_toolchain_identity_digest
    }

    #[must_use]
    pub const fn verification_task_ref(&self) -> &ContentDigest {
        &self.verification_task_ref
    }

    #[must_use]
    pub fn immutable_snapshot_ref(&self) -> &str {
        &self.immutable_snapshot_ref
    }

    #[must_use]
    pub const fn immutable_snapshot_digest(&self) -> &ContentDigest {
        &self.immutable_snapshot_digest
    }

    #[must_use]
    pub fn sandbox_policy_ref(&self) -> &str {
        &self.sandbox_policy_ref
    }

    #[must_use]
    pub const fn sandbox_policy_digest(&self) -> &ContentDigest {
        &self.sandbox_policy_digest
    }

    #[must_use]
    pub fn privilege_boundary_ref(&self) -> &str {
        &self.privilege_boundary_ref
    }

    #[must_use]
    pub const fn privilege_boundary_digest(&self) -> &ContentDigest {
        &self.privilege_boundary_digest
    }

    #[must_use]
    pub fn path_mapping_windows_path(&self) -> &str {
        &self.path_mapping_windows_path
    }

    #[must_use]
    pub fn path_mapping_linux_path(&self) -> &str {
        &self.path_mapping_linux_path
    }

    #[must_use]
    pub fn path_mapping_digest_ref(&self) -> &str {
        &self.path_mapping_digest_ref
    }

    #[must_use]
    pub const fn path_mapping_digest(&self) -> &ContentDigest {
        &self.path_mapping_digest
    }

    /// Returns the exact recursively key-sorted production JSON.
    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }

    /// Alias used when forwarding the exact descriptor to a preflight or child.
    #[must_use]
    pub fn as_json(&self) -> &str {
        self.canonical_json()
    }

    #[must_use]
    pub const fn execution_domain_digest(&self) -> &ContentDigest {
        &self.execution_domain_digest
    }

    #[must_use]
    pub const fn descriptor_digest(&self) -> &ContentDigest {
        &self.execution_domain_digest
    }

    #[must_use]
    pub const fn environment_ref(&self) -> &ExecutionEnvironmentRef {
        &self.environment_ref
    }
}

/// Fresh-process reconstruction of one descriptor and its exact attempt anchor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedExecutionEnvironment {
    task_ref: ContentDigest,
    attempt_number: u8,
    attempt_id: AttemptId,
    packet_digest: ContentDigest,
    descriptor: ExecutionEnvironmentDescriptor,
    recorded_at: String,
}

impl PersistedExecutionEnvironment {
    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }

    #[must_use]
    pub const fn attempt_number(&self) -> u8 {
        self.attempt_number
    }

    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    #[must_use]
    pub const fn packet_digest(&self) -> &ContentDigest {
        &self.packet_digest
    }

    #[must_use]
    pub const fn descriptor(&self) -> &ExecutionEnvironmentDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub fn recorded_at(&self) -> &str {
        &self.recorded_at
    }
}

/// Bounded Git source locator captured atomically with one task promotion.
///
/// This value is only a restart candidate. The Task Spec owner must rebuild
/// and verify the canonical spec/binding before using it for execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedPromotionSource {
    base_ref: String,
    base_commit: String,
}

/// Immutable owner-bound source/spec intent retained before any successor
/// Task-Ledger effect. It is lineage evidence only and grants no execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedPromotionIntent {
    task_ref: ContentDigest,
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    project_authority_receipt_digest: ContentDigest,
    successor_stream_id: ContentDigest,
    task_spec_digest: ContentDigest,
    approval_subject_digest: ContentDigest,
    budget: WorkerBudget,
    verification_policy_digest: ContentDigest,
    source: ManagedPromotionSource,
    source_clean: bool,
    issued_at: String,
    intent_digest: ContentDigest,
}

/// Latest bounded, rebuttable preparation observation for an admitted intake.
/// It is evidence about a dependency and is never a Task state or authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedPreparationObservationKind {
    WorktreeNotClean,
    ProjectRegistryCurrentnessConflict,
    Cleared,
}

impl ManagedPreparationObservationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorktreeNotClean => "WORKTREE_NOT_CLEAN",
            Self::ProjectRegistryCurrentnessConflict => "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
            Self::Cleared => "CLEARED",
        }
    }

    fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "WORKTREE_NOT_CLEAN" => Ok(Self::WorktreeNotClean),
            "PROJECT_REGISTRY_CURRENTNESS_CONFLICT" => Ok(Self::ProjectRegistryCurrentnessConflict),
            "CLEARED" => Ok(Self::Cleared),
            _ => Err(corrupt_error()),
        }
    }

    #[must_use]
    pub const fn blocker_code(self) -> Option<&'static str> {
        match self {
            Self::WorktreeNotClean => Some("LATTICE_MANAGED_WORKTREE_NOT_CLEAN"),
            Self::ProjectRegistryCurrentnessConflict => {
                Some("PROJECT_REGISTRY_CURRENTNESS_CONFLICT")
            }
            Self::Cleared => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedPreparationObservation {
    task_ref: ContentDigest,
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    project_authority_receipt_digest: ContentDigest,
    kind: ManagedPreparationObservationKind,
    subject_digest: ContentDigest,
    observed_at: String,
    observation_digest: ContentDigest,
}

impl ManagedPreparationObservation {
    pub fn new(
        task_ref: ContentDigest,
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        project_authority_receipt_digest: ContentDigest,
        kind: ManagedPreparationObservationKind,
        subject_digest: ContentDigest,
        observed_at: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        let observed_at = observed_at.into();
        if is_zero(&task_ref)
            || is_zero(&project_authority_receipt_digest)
            || is_zero(&subject_digest)
            || observed_at.is_empty()
            || observed_at.len() > 40
            || observed_at.chars().any(char::is_control)
        {
            return Err(input_error());
        }
        let observation_digest = preparation_observation_digest(
            &task_ref,
            &project_id,
            &project_snapshot_id,
            &project_authority_receipt_digest,
            kind,
            &subject_digest,
            &observed_at,
        )?;
        Ok(Self {
            task_ref,
            project_id,
            project_snapshot_id,
            project_authority_receipt_digest,
            kind,
            subject_digest,
            observed_at,
            observation_digest,
        })
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }
    #[must_use]
    pub const fn project_authority_receipt_digest(&self) -> &ContentDigest {
        &self.project_authority_receipt_digest
    }
    #[must_use]
    pub const fn kind(&self) -> ManagedPreparationObservationKind {
        self.kind
    }
    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }
    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }
    #[must_use]
    pub const fn observation_digest(&self) -> &ContentDigest {
        &self.observation_digest
    }
}

impl ManagedPromotionIntent {
    /// Builds the exact pre-successor intent and its replay commitment.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_ref: ContentDigest,
        project_id: ProjectId,
        project_snapshot_id: ProjectSnapshotId,
        project_authority_receipt_digest: ContentDigest,
        successor_stream_id: ContentDigest,
        task_spec_digest: ContentDigest,
        approval_subject_digest: ContentDigest,
        budget: WorkerBudget,
        verification_policy_digest: ContentDigest,
        source: ManagedPromotionSource,
        source_clean: bool,
        issued_at: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        let issued_at = issued_at.into();
        if is_zero(&task_ref)
            || is_zero(&project_authority_receipt_digest)
            || is_zero(&successor_stream_id)
            || is_zero(&task_spec_digest)
            || is_zero(&approval_subject_digest)
            || is_zero(&verification_policy_digest)
            || !source_clean
            || issued_at.is_empty()
            || issued_at.len() > 40
            || issued_at.chars().any(char::is_control)
        {
            return Err(input_error());
        }
        let intent_digest = promotion_intent_digest(
            &task_ref,
            &project_id,
            &project_snapshot_id,
            &project_authority_receipt_digest,
            &successor_stream_id,
            &task_spec_digest,
            &approval_subject_digest,
            &budget,
            &verification_policy_digest,
            &source,
            source_clean,
            &issued_at,
        )?;
        Ok(Self {
            task_ref,
            project_id,
            project_snapshot_id,
            project_authority_receipt_digest,
            successor_stream_id,
            task_spec_digest,
            approval_subject_digest,
            budget,
            verification_policy_digest,
            source,
            source_clean,
            issued_at,
            intent_digest,
        })
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        &self.project_snapshot_id
    }
    #[must_use]
    pub const fn project_authority_receipt_digest(&self) -> &ContentDigest {
        &self.project_authority_receipt_digest
    }
    #[must_use]
    pub const fn successor_stream_id(&self) -> &ContentDigest {
        &self.successor_stream_id
    }
    #[must_use]
    pub const fn task_spec_digest(&self) -> &ContentDigest {
        &self.task_spec_digest
    }
    #[must_use]
    pub const fn approval_subject_digest(&self) -> &ContentDigest {
        &self.approval_subject_digest
    }
    #[must_use]
    pub const fn budget(&self) -> &WorkerBudget {
        &self.budget
    }
    #[must_use]
    pub const fn verification_policy_digest(&self) -> &ContentDigest {
        &self.verification_policy_digest
    }
    #[must_use]
    pub const fn source(&self) -> &ManagedPromotionSource {
        &self.source
    }
    #[must_use]
    pub const fn source_clean(&self) -> bool {
        self.source_clean
    }
    #[must_use]
    pub fn issued_at(&self) -> &str {
        &self.issued_at
    }
    #[must_use]
    pub const fn intent_digest(&self) -> &ContentDigest {
        &self.intent_digest
    }
}

impl ManagedPromotionSource {
    /// Constructs one bounded non-remote ref and exact lower-case SHA-1 object ID.
    ///
    /// # Errors
    ///
    /// Empty, oversized, control-bearing, whitespace-bearing, URL-like, remote,
    /// or non-canonical values fail closed.
    pub fn new(
        base_ref: impl Into<String>,
        base_commit: impl Into<String>,
    ) -> Result<Self, AdapterError> {
        let base_ref = base_ref.into();
        let base_commit = base_commit.into();
        if base_ref.is_empty()
            || base_ref.len() > 255
            || base_ref.starts_with("refs/remotes/")
            || base_ref.contains("://")
            || base_ref
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
            || base_commit.len() != 40
            || !base_commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(input_error());
        }
        Ok(Self {
            base_ref,
            base_commit,
        })
    }

    #[must_use]
    pub fn base_ref(&self) -> &str {
        &self.base_ref
    }

    #[must_use]
    pub fn base_commit(&self) -> &str {
        &self.base_commit
    }
}

/// Atomic attempt-claim outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimDisposition {
    Claimed,
    ExactReplay,
}

/// Closed one-shot provider effects admitted beneath one durable attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderDispatchKind {
    WorkerThread,
    WorkerTurn,
    ReviewThread,
    ReviewTurn,
}

impl ProviderDispatchKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkerThread => "WORKER_THREAD",
            Self::WorkerTurn => "WORKER_TURN",
            Self::ReviewThread => "REVIEW_THREAD",
            Self::ReviewTurn => "REVIEW_TURN",
        }
    }

    fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "WORKER_THREAD" => Ok(Self::WorkerThread),
            "WORKER_TURN" => Ok(Self::WorkerTurn),
            "REVIEW_THREAD" => Ok(Self::ReviewThread),
            "REVIEW_TURN" => Ok(Self::ReviewTurn),
            _ => Err(corrupt_error()),
        }
    }
}

/// Exact durable receipt for one provider dispatch claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDispatchClaim {
    kind: ProviderDispatchKind,
    task_ref: ContentDigest,
    attempt_number: u8,
    attempt_id: AttemptId,
    binding_digest: ContentDigest,
    writer_fence: u64,
    foreman_generation: u64,
    foreman_checkpoint_digest: ContentDigest,
    anchor_digest: ContentDigest,
    supporting_digest: ContentDigest,
    subject_digest: ContentDigest,
    dispatch_digest: ContentDigest,
    claim_receipt_digest: ContentDigest,
    claimed_at: String,
}

impl ProviderDispatchClaim {
    #[must_use]
    pub const fn kind(&self) -> ProviderDispatchKind {
        self.kind
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }

    #[must_use]
    pub const fn attempt_number(&self) -> u8 {
        self.attempt_number
    }

    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    #[must_use]
    pub const fn binding_digest(&self) -> &ContentDigest {
        &self.binding_digest
    }

    #[must_use]
    pub const fn writer_fence(&self) -> u64 {
        self.writer_fence
    }

    #[must_use]
    pub const fn foreman_generation(&self) -> u64 {
        self.foreman_generation
    }

    #[must_use]
    pub const fn foreman_checkpoint_digest(&self) -> &ContentDigest {
        &self.foreman_checkpoint_digest
    }

    #[must_use]
    pub const fn anchor_digest(&self) -> &ContentDigest {
        &self.anchor_digest
    }

    #[must_use]
    pub const fn supporting_digest(&self) -> &ContentDigest {
        &self.supporting_digest
    }

    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }

    #[must_use]
    pub const fn dispatch_digest(&self) -> &ContentDigest {
        &self.dispatch_digest
    }

    /// Returns the database-time-bound receipt digest for this exact claim.
    #[must_use]
    pub const fn claim_receipt_digest(&self) -> &ContentDigest {
        &self.claim_receipt_digest
    }

    #[must_use]
    pub fn claimed_at(&self) -> &str {
        &self.claimed_at
    }
}

/// Durable reservation outcome before active-capacity admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimReservationDisposition {
    Reserved,
    ExactReplay,
}

/// Capacity observation returned by the atomic claim transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClaimOutcome {
    disposition: ClaimDisposition,
    global_active: u8,
    task_active: u8,
}

/// Closed state for one restart-discovery candidate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartTaskKind {
    /// General intake retained in the formal Task Ledger but not yet promoted
    /// when the prior Runtime process stopped.
    DraftPendingPromotion,
    DraftProjectReconciliationRequired,
    PromotedNoAttempt,
    CapacityWait,
    /// A pending or retained exact attempt whose pinned Project Registry
    /// authority is no longer current.
    ProjectReconciliationRequired,
    AttemptReconcileRequired,
    WriterReconciliationRequired,
    TerminalPendingVerification,
    VerificationReconcileRequired,
    AttemptClosedPendingRelease,
}

impl RestartTaskKind {
    fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "DRAFT_PENDING_PROMOTION" => Ok(Self::DraftPendingPromotion),
            "DRAFT_PROJECT_RECONCILIATION_REQUIRED" => Ok(Self::DraftProjectReconciliationRequired),
            "PROMOTED_NO_ATTEMPT" => Ok(Self::PromotedNoAttempt),
            "CAPACITY_WAIT" => Ok(Self::CapacityWait),
            "PROJECT_RECONCILIATION_REQUIRED" => Ok(Self::ProjectReconciliationRequired),
            "ATTEMPT_RECONCILE_REQUIRED" => Ok(Self::AttemptReconcileRequired),
            "WRITER_RECONCILIATION_REQUIRED" => Ok(Self::WriterReconciliationRequired),
            "TERMINAL_PENDING_VERIFICATION" => Ok(Self::TerminalPendingVerification),
            "VERIFICATION_RECONCILE_REQUIRED" => Ok(Self::VerificationReconcileRequired),
            "ATTEMPT_CLOSED_PENDING_RELEASE" => Ok(Self::AttemptClosedPendingRelease),
            _ => Err(corrupt_error()),
        }
    }

    const fn priority(self) -> u8 {
        match self {
            Self::AttemptClosedPendingRelease => 0,
            Self::VerificationReconcileRequired => 1,
            Self::TerminalPendingVerification => 2,
            Self::ProjectReconciliationRequired => 3,
            Self::AttemptReconcileRequired => 3,
            Self::WriterReconciliationRequired => 3,
            Self::CapacityWait => 4,
            Self::PromotedNoAttempt => 5,
            Self::DraftPendingPromotion => 6,
            Self::DraftProjectReconciliationRequired => 6,
        }
    }
}

/// Typed durable proof that an attempt no longer owns live provider capacity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptClosure {
    blocker_code: String,
    blocker_descriptor_digest: ContentDigest,
    reconciliation_proof_descriptor_digest: Option<ContentDigest>,
    writer_fence: u64,
    closed_at: String,
}

impl AttemptClosure {
    #[must_use]
    pub fn blocker_code(&self) -> &str {
        &self.blocker_code
    }

    #[must_use]
    pub const fn blocker_descriptor_digest(&self) -> &ContentDigest {
        &self.blocker_descriptor_digest
    }

    #[must_use]
    pub const fn reconciliation_proof_descriptor_digest(&self) -> Option<&ContentDigest> {
        self.reconciliation_proof_descriptor_digest.as_ref()
    }

    #[must_use]
    pub const fn writer_fence(&self) -> u64 {
        self.writer_fence
    }

    #[must_use]
    pub fn closed_at(&self) -> &str {
        &self.closed_at
    }
}

/// Stable keyset cursor for restart discovery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestartTaskCursor {
    restart_priority: u8,
    task_ref: ContentDigest,
}

impl RestartTaskCursor {
    #[must_use]
    pub const fn restart_priority(&self) -> u8 {
        self.restart_priority
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }
}

impl Ord for RestartTaskCursor {
    fn cmp(&self, other: &Self) -> Ordering {
        self.restart_priority
            .cmp(&other.restart_priority)
            .then_with(|| self.task_ref.as_str().cmp(other.task_ref.as_str()))
    }
}

impl PartialOrd for RestartTaskCursor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One bounded restart locator. Promoted tasks have no attempt identity yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestartTaskRef {
    task_ref: ContentDigest,
    attempt_number: Option<u8>,
    attempt_id: Option<AttemptId>,
    restart_kind: RestartTaskKind,
    restart_priority: u8,
    last_observed_at: Option<String>,
}

impl RestartTaskRef {
    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }

    #[must_use]
    pub const fn attempt_number(&self) -> Option<u8> {
        self.attempt_number
    }

    #[must_use]
    pub const fn attempt_id(&self) -> Option<&AttemptId> {
        self.attempt_id.as_ref()
    }

    #[must_use]
    pub const fn restart_kind(&self) -> RestartTaskKind {
        self.restart_kind
    }

    #[must_use]
    pub const fn restart_priority(&self) -> u8 {
        self.restart_priority
    }

    #[must_use]
    pub fn cursor(&self) -> RestartTaskCursor {
        RestartTaskCursor {
            restart_priority: self.restart_priority,
            task_ref: self.task_ref.clone(),
        }
    }

    #[must_use]
    pub fn last_observed_at(&self) -> Option<&str> {
        self.last_observed_at.as_deref()
    }
}

impl ClaimOutcome {
    #[must_use]
    pub const fn disposition(self) -> ClaimDisposition {
        self.disposition
    }

    #[must_use]
    pub const fn global_active(self) -> u8 {
        self.global_active
    }

    #[must_use]
    pub const fn task_active(self) -> u8 {
        self.task_active
    }
}

/// Closed reason an unverified latest attempt must be recovered after restart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActiveTaskRestartKind {
    AttemptReconcileRequired,
    TerminalPendingVerification,
}

impl ActiveTaskRestartKind {
    fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "ATTEMPT_RECONCILE_REQUIRED" => Ok(Self::AttemptReconcileRequired),
            "TERMINAL_PENDING_VERIFICATION" => Ok(Self::TerminalPendingVerification),
            _ => Err(corrupt_error()),
        }
    }
}

/// One bounded, deterministically ordered restart-discovery row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveTaskRef {
    task_ref: ContentDigest,
    attempt_number: u8,
    attempt_id: AttemptId,
    restart_kind: ActiveTaskRestartKind,
    last_observed_at: Option<String>,
}

impl ActiveTaskRef {
    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }

    #[must_use]
    pub const fn attempt_number(&self) -> u8 {
        self.attempt_number
    }

    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    #[must_use]
    pub const fn restart_kind(&self) -> ActiveTaskRestartKind {
        self.restart_kind
    }

    #[must_use]
    pub fn last_observed_at(&self) -> Option<&str> {
        self.last_observed_at.as_deref()
    }
}

/// One append-only replay index entry returned through a fixed read function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayRecord {
    record_kind: String,
    record_state: ReplayRecordState,
    attempt_number: Option<u8>,
    record_ordinal: u64,
    record_digest: ContentDigest,
    ledger_stream_id: ContentDigest,
    ledger_event_sequence: u64,
    ledger_event_digest: ContentDigest,
    recorded_at: String,
}

/// Physical replay state for one exact Ledger-linked record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayRecordState {
    Retained,
    PendingClaim,
}

impl ReplayRecordState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Retained => "RETAINED",
            Self::PendingClaim => "PENDING_CLAIM",
        }
    }

    fn parse(value: &str) -> Result<Self, AdapterError> {
        match value {
            "RETAINED" => Ok(Self::Retained),
            "PENDING_CLAIM" => Ok(Self::PendingClaim),
            _ => Err(corrupt_error()),
        }
    }
}

impl ReplayRecord {
    #[must_use]
    pub fn record_kind(&self) -> &str {
        &self.record_kind
    }

    #[must_use]
    pub const fn record_state(&self) -> ReplayRecordState {
        self.record_state
    }

    #[must_use]
    pub const fn attempt_number(&self) -> Option<u8> {
        self.attempt_number
    }

    #[must_use]
    pub const fn record_ordinal(&self) -> u64 {
        self.record_ordinal
    }

    #[must_use]
    pub const fn record_digest(&self) -> &ContentDigest {
        &self.record_digest
    }

    #[must_use]
    pub const fn ledger_stream_id(&self) -> &ContentDigest {
        &self.ledger_stream_id
    }

    #[must_use]
    pub const fn ledger_event_sequence(&self) -> u64 {
        self.ledger_event_sequence
    }

    #[must_use]
    pub const fn ledger_event_digest(&self) -> &ContentDigest {
        &self.ledger_event_digest
    }

    #[must_use]
    pub fn recorded_at(&self) -> &str {
        &self.recorded_at
    }
}

/// Owner-verifiable worker-attempt candidate retained while capacity is full.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingWorkerAttempt {
    row: UntrustedWorkerAttemptRow,
    execution_environment_ref: String,
    max_attempts: u8,
    reserved_at: String,
}

impl PendingWorkerAttempt {
    #[must_use]
    pub const fn row(&self) -> &UntrustedWorkerAttemptRow {
        &self.row
    }

    #[must_use]
    pub const fn max_attempts(&self) -> u8 {
        self.max_attempts
    }

    #[must_use]
    pub fn execution_environment_ref(&self) -> &str {
        &self.execution_environment_ref
    }

    #[must_use]
    pub fn reserved_at(&self) -> &str {
        &self.reserved_at
    }
}

/// Stable restart projection of every child-record digest for one task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskReplay {
    task_ref: ContentDigest,
    records: Vec<ReplayRecord>,
    evidence_digest: ContentDigest,
}

/// Persistence-shaped Task Ledger child rows for owner-side restart verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedTaskRuntimeRows {
    binding: UntrustedTaskExecutionBinding,
    attempts: Vec<UntrustedWorkerAttemptRow>,
    observations: Vec<UntrustedWorkerObservationRow>,
    verifications: Vec<UntrustedTaskVerificationRow>,
}

impl PersistedTaskRuntimeRows {
    #[must_use]
    pub const fn binding(&self) -> &UntrustedTaskExecutionBinding {
        &self.binding
    }

    #[must_use]
    pub fn attempts(&self) -> &[UntrustedWorkerAttemptRow] {
        &self.attempts
    }

    #[must_use]
    pub fn observations(&self) -> &[UntrustedWorkerObservationRow] {
        &self.observations
    }

    #[must_use]
    pub fn verifications(&self) -> &[UntrustedTaskVerificationRow] {
        &self.verifications
    }
}

/// One artifact descriptor bound to its exact appended Task Ledger event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedArtifactReferenceLink {
    attempt_number: u8,
    descriptor_digest: ContentDigest,
    link: TaskRuntimeEventLink,
}

/// One owner-reverified durable artifact outbox entry awaiting exact Ledger
/// append and/or subordinate finalization.
///
/// This is physical recovery state only. It contains no task phase and grants
/// no provider, Writer, verification, or protected-effect authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedArtifactReference {
    evidence: VerifiedManagedEvidence,
    link: TaskRuntimeEventLink,
    correlation_id: CorrelationId,
    command_occurred_at: String,
    staged_at: String,
}

impl StagedArtifactReference {
    #[must_use]
    pub const fn evidence(&self) -> &VerifiedManagedEvidence {
        &self.evidence
    }

    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }

    #[must_use]
    pub const fn correlation_id(&self) -> &CorrelationId {
        &self.correlation_id
    }

    #[must_use]
    pub fn command_occurred_at(&self) -> &str {
        &self.command_occurred_at
    }

    #[must_use]
    pub fn staged_at(&self) -> &str {
        &self.staged_at
    }
}

impl PersistedArtifactReferenceLink {
    #[must_use]
    pub const fn attempt_number(&self) -> u8 {
        self.attempt_number
    }

    #[must_use]
    pub const fn descriptor_digest(&self) -> &ContentDigest {
        &self.descriptor_digest
    }

    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }
}

/// One approval authority bound to its exact appended Task Ledger event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedApprovalReferenceLink {
    authority_digest: ContentDigest,
    link: TaskRuntimeEventLink,
}

impl PersistedApprovalReferenceLink {
    #[must_use]
    pub const fn authority_digest(&self) -> &ContentDigest {
        &self.authority_digest
    }

    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }
}

/// Typed restart projection for subordinate artifact and approval event links.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedReferenceLinks {
    artifact_links: Vec<PersistedArtifactReferenceLink>,
    approval_links: Vec<PersistedApprovalReferenceLink>,
}

impl PersistedReferenceLinks {
    #[must_use]
    pub fn artifact_links(&self) -> &[PersistedArtifactReferenceLink] {
        &self.artifact_links
    }

    #[must_use]
    pub fn approval_links(&self) -> &[PersistedApprovalReferenceLink] {
        &self.approval_links
    }
}

impl TaskReplay {
    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }

    #[must_use]
    pub fn records(&self) -> &[ReplayRecord] {
        &self.records
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }
}

/// Live synchronous adapter. It owns no scheduler, process, or domain state.
pub struct PostgresForeman {
    client: Client,
    evidence: ExtensionCatalogEvidence,
    foreman_coordination_stream_id: ContentDigest,
}

impl PostgresForeman {
    /// Verifies an exact runtime profile before retaining the connection.
    ///
    /// # Errors
    ///
    /// Fails closed if setup, identity, catalog, or ACL verification fails.
    pub fn new(client: Client, target: &ExtensionTarget) -> Result<Self, AdapterError> {
        Self::new_with_role(client, target, ExtensionDatabaseRole::Runtime)
    }

    /// Verifies one exact role-specific profile before retaining the connection.
    ///
    /// # Errors
    ///
    /// Fails closed if setup, identity, catalog, or ACL verification fails.
    pub fn new_with_role(
        mut client: Client,
        target: &ExtensionTarget,
        role: ExtensionDatabaseRole,
    ) -> Result<Self, AdapterError> {
        let evidence = verify_extension(&mut client, target, role)?;
        let foreman_coordination_stream_id = VerifiedStream::vacant(
            foreman_coordination_identity().map_err(|_| corrupt_error())?,
            RuntimeKind::Live,
        )
        .map_err(|_| corrupt_error())?
        .head()
        .stream_id()
        .clone();
        Ok(Self {
            client,
            evidence,
            foreman_coordination_stream_id,
        })
    }

    #[must_use]
    pub const fn extension_evidence(&self) -> &ExtensionCatalogEvidence {
        &self.evidence
    }

    #[must_use]
    pub fn into_client(self) -> Client {
        self.client
    }

    /// Lists latest unverified attempts through a fixed, bounded SQL reader.
    ///
    /// Rows are sorted by binary `task_ref`. A non-terminal attempt requires
    /// connector reconciliation; a terminal attempt requires verification.
    ///
    /// # Errors
    ///
    /// Zero/oversized limits, malformed rows, or database failures fail closed.
    pub fn list_active_task_refs(
        &mut self,
        limit: u16,
    ) -> Result<Vec<ActiveTaskRef>, AdapterError> {
        if limit == 0 || limit > MAX_ACTIVE_TASK_REPLAY_ROWS {
            return Err(input_error());
        }
        let sql_limit = i16::try_from(limit).map_err(|_| input_error())?;
        let rows = self
            .client
            .query(
                "SELECT pg_catalog.encode(task_ref,'hex'), attempt_number, \
                        attempt_id, restart_kind, last_observed_at \
                   FROM foreman_execution.list_active_task_refs_v1($1)",
                &[&sql_limit],
            )
            .map_err(|_| database_error())?;
        let mut active = Vec::with_capacity(rows.len());
        for row in rows {
            let attempt: i16 = row.get(1);
            let attempt_number = u8::try_from(attempt).map_err(|_| corrupt_error())?;
            if !(1..=3).contains(&attempt_number) {
                return Err(corrupt_error());
            }
            active.push(ActiveTaskRef {
                task_ref: parse_digest(row.get::<_, String>(0))?,
                attempt_number,
                attempt_id: AttemptId::new(row.get::<_, String>(2)).map_err(|_| corrupt_error())?,
                restart_kind: ActiveTaskRestartKind::parse(row.get::<_, String>(3).as_str())?,
                last_observed_at: row.get(4),
            });
        }
        Ok(active)
    }

    /// Lists every bounded restart candidate, including pre-claim capacity wait.
    ///
    /// Promoted tasks without an attempt have no attempt number or ID. All
    /// other kinds carry both fields. Rows are sorted by binary `task_ref`.
    ///
    /// # Errors
    ///
    /// Zero/oversized limits, malformed rows, or database failures fail closed.
    pub fn list_restart_task_refs(
        &mut self,
        limit: u16,
    ) -> Result<Vec<RestartTaskRef>, AdapterError> {
        self.list_restart_task_refs_page(None, limit)
    }

    /// Lists one bounded restart-discovery page after an exact stable cursor.
    ///
    /// The keyset is `(restart_priority, task_ref)` and never uses an offset.
    /// Callers must continue with the cursor of the final returned row.
    ///
    /// # Errors
    ///
    /// Zero/oversized limits, malformed rows, or database failures fail closed.
    pub fn list_restart_task_refs_page(
        &mut self,
        after: Option<&RestartTaskCursor>,
        limit: u16,
    ) -> Result<Vec<RestartTaskRef>, AdapterError> {
        if limit == 0 || limit > MAX_ACTIVE_TASK_REPLAY_ROWS {
            return Err(input_error());
        }
        let sql_limit = i16::try_from(limit).map_err(|_| input_error())?;
        let after_priority = after.map(|cursor| i16::from(cursor.restart_priority));
        let after_task_ref = after
            .map(|cursor| digest_bytes(&cursor.task_ref))
            .transpose()?;
        let rows = self
            .client
            .query(
                "SELECT pg_catalog.encode(task_ref,'hex'), attempt_number, \
                        attempt_id, restart_kind, last_observed_at, restart_priority \
                   FROM foreman_execution.list_restart_task_refs_v1($1,$2,$3)",
                &[&after_priority, &after_task_ref, &sql_limit],
            )
            .map_err(|_| database_error())?;
        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows {
            let attempt = row
                .get::<_, Option<i16>>(1)
                .map(|value| u8::try_from(value).map_err(|_| corrupt_error()))
                .transpose()?;
            if attempt.is_some_and(|value| !(1..=3).contains(&value)) {
                return Err(corrupt_error());
            }
            let attempt_id = row
                .get::<_, Option<String>>(2)
                .map(AttemptId::new)
                .transpose()
                .map_err(|_| corrupt_error())?;
            let restart_kind = RestartTaskKind::parse(row.get::<_, String>(3).as_str())?;
            let restart_priority =
                u8::try_from(row.get::<_, i16>(5)).map_err(|_| corrupt_error())?;
            let has_no_attempt = matches!(
                restart_kind,
                RestartTaskKind::DraftPendingPromotion
                    | RestartTaskKind::DraftProjectReconciliationRequired
                    | RestartTaskKind::PromotedNoAttempt
            );
            if has_no_attempt != (attempt.is_none() && attempt_id.is_none())
                || !has_no_attempt && (attempt.is_none() || attempt_id.is_none())
                || restart_priority != restart_kind.priority()
            {
                return Err(corrupt_error());
            }
            candidates.push(RestartTaskRef {
                task_ref: parse_digest(row.get::<_, String>(0))?,
                attempt_number: attempt,
                attempt_id,
                restart_kind,
                restart_priority,
                last_observed_at: row.get(4),
            });
        }
        Ok(candidates)
    }

    /// Atomically retains the exact source/spec intent before successor
    /// admission. Exact retries are immutable and never re-sample Git.
    pub fn record_promotion_intent(
        &mut self,
        intent: &ManagedPromotionIntent,
    ) -> Result<AppendDisposition, AdapterError> {
        let budget = intent.budget();
        let budget_digest = budget
            .digest()
            .strip_prefix("budget:sha256:")
            .ok_or_else(input_error)?;
        let budget_digest = ContentDigest::from_sha256(budget_digest).map_err(|_| input_error())?;
        let (external_cost_status, external_cost_limit_micros) = match budget.external_cost() {
            ExternalCostBudget::Unavailable => ("UNAVAILABLE", None),
            ExternalCostBudget::LimitMicros(value) => ("LIMIT_MICROS", Some(to_i64(value)?)),
        };
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.record_promotion_intent_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16, \
                    $17,$18,$19,$20,$21,$22,$23,$24)",
                &[
                    &digest_bytes(intent.task_ref())?,
                    &intent.project_id().as_str(),
                    &intent.project_snapshot_id().as_str(),
                    &digest_bytes(intent.project_authority_receipt_digest())?,
                    &digest_bytes(intent.successor_stream_id())?,
                    &digest_bytes(intent.task_spec_digest())?,
                    &digest_bytes(intent.approval_subject_digest())?,
                    &digest_bytes(&budget_digest)?,
                    &i16::from(budget.global_active_limit()),
                    &i16::from(budget.per_task_active_limit()),
                    &i16::from(budget.repair_retry_limit()),
                    &to_i64(budget.max_duration_seconds())?,
                    &to_i64(budget.max_total_tokens())?,
                    &i64::from(budget.max_model_calls()),
                    &external_cost_status,
                    &external_cost_limit_micros,
                    &intent.issued_at(),
                    &budget.deadline_at(),
                    &budget.digest(),
                    &digest_bytes(intent.verification_policy_digest())?,
                    &intent.source().base_ref(),
                    &intent.source().base_commit(),
                    &intent.source_clean(),
                    &digest_bytes(intent.intent_digest())?,
                ],
            )
            .map_err(|error| database_error_at(AdapterDatabaseStage::TaskPromotion, &error))?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Records or supersedes the one bounded preparation observation for an
    /// admitted intake. This never changes Task Ledger state or grants work.
    pub fn record_preparation_observation(
        &mut self,
        observation: &ManagedPreparationObservation,
    ) -> Result<AppendDisposition, AdapterError> {
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.record_preparation_observation_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8)",
                &[
                    &digest_bytes(observation.task_ref())?,
                    &observation.project_id().as_str(),
                    &observation.project_snapshot_id().as_str(),
                    &digest_bytes(observation.project_authority_receipt_digest())?,
                    &observation.kind().as_str(),
                    &digest_bytes(observation.subject_digest())?,
                    &observation.observed_at(),
                    &digest_bytes(observation.observation_digest())?,
                ],
            )
            .map_err(|error| {
                database_error_at(AdapterDatabaseStage::PreparationObservation, &error)
            })?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Reads and independently verifies the latest bounded dependency
    /// observation. The read is side-effect free.
    pub fn load_preparation_observation(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<Option<ManagedPreparationObservation>, AdapterError> {
        let row = self
            .client
            .query_opt(
                "SELECT project_id, project_snapshot_id, \
                        pg_catalog.encode(project_authority_receipt_digest,'hex'), \
                        observation_kind, pg_catalog.encode(subject_digest,'hex'), \
                        observed_at, pg_catalog.encode(observation_digest,'hex') \
                   FROM foreman_execution.read_preparation_observation_v1($1)",
                &[&digest_bytes(task_ref)?],
            )
            .map_err(|_| database_error())?;
        let Some(row) = row else {
            return Ok(None);
        };
        let observation = ManagedPreparationObservation::new(
            task_ref.clone(),
            ProjectId::new(row.get::<_, String>(0)).map_err(|_| corrupt_error())?,
            ProjectSnapshotId::new(row.get::<_, String>(1)).map_err(|_| corrupt_error())?,
            parse_digest(row.get(2))?,
            ManagedPreparationObservationKind::parse(row.get::<_, String>(3).as_str())?,
            parse_digest(row.get(4))?,
            row.get::<_, String>(5),
        )
        .map_err(|_| corrupt_error())?;
        if observation.observation_digest() != &parse_digest(row.get(6))? {
            return Err(corrupt_error());
        }
        Ok(Some(observation))
    }

    /// Loads and independently re-hashes one immutable promotion intent.
    pub fn load_promotion_intent(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<Option<ManagedPromotionIntent>, AdapterError> {
        let row = self
            .client
            .query_opt(
                "SELECT project_id, project_snapshot_id, \
                        pg_catalog.encode(project_authority_receipt_digest,'hex'), \
                        pg_catalog.encode(successor_stream_id,'hex'), \
                        pg_catalog.encode(task_spec_digest,'hex'), \
                        pg_catalog.encode(approval_subject_digest,'hex'), \
                        pg_catalog.encode(budget_digest,'hex'), global_active_limit, \
                        per_task_active_limit, repair_retry_limit, max_duration_seconds, \
                        max_total_tokens, max_model_calls, external_cost_status, \
                        external_cost_limit_micros, issued_at, deadline_at, budget_pointer, \
                        pg_catalog.encode(verification_policy_digest,'hex'), base_ref, base_commit, \
                        source_clean, pg_catalog.encode(intent_digest,'hex'), recorded_at \
                   FROM foreman_execution.read_promotion_intent_v1($1)",
                &[&digest_bytes(task_ref)?],
            )
            .map_err(|_| database_error())?;
        let Some(row) = row else {
            return Ok(None);
        };
        let external_cost = match (
            row.get::<_, String>(13).as_str(),
            row.get::<_, Option<i64>>(14),
        ) {
            ("UNAVAILABLE", None) => ExternalCostBudget::Unavailable,
            ("LIMIT_MICROS", Some(value)) => {
                ExternalCostBudget::LimitMicros(u64::try_from(value).map_err(|_| corrupt_error())?)
            }
            _ => return Err(corrupt_error()),
        };
        let budget = WorkerBudget::new(
            u8::try_from(row.get::<_, i16>(7)).map_err(|_| corrupt_error())?,
            u8::try_from(row.get::<_, i16>(8)).map_err(|_| corrupt_error())?,
            u8::try_from(row.get::<_, i16>(9)).map_err(|_| corrupt_error())?,
            u64::try_from(row.get::<_, i64>(10)).map_err(|_| corrupt_error())?,
            u64::try_from(row.get::<_, i64>(11)).map_err(|_| corrupt_error())?,
            u32::try_from(row.get::<_, i64>(12)).map_err(|_| corrupt_error())?,
            external_cost,
            row.get::<_, String>(16),
        )
        .map_err(|_| corrupt_error())?;
        if budget.digest() != row.get::<_, String>(17)
            || budget
                .digest()
                .strip_prefix("budget:sha256:")
                .is_none_or(|digest| digest != row.get::<_, String>(6))
        {
            return Err(corrupt_error());
        }
        let intent = ManagedPromotionIntent::new(
            task_ref.clone(),
            ProjectId::new(row.get::<_, String>(0)).map_err(|_| corrupt_error())?,
            ProjectSnapshotId::new(row.get::<_, String>(1)).map_err(|_| corrupt_error())?,
            parse_digest(row.get(2))?,
            parse_digest(row.get(3))?,
            parse_digest(row.get(4))?,
            parse_digest(row.get(5))?,
            budget,
            parse_digest(row.get(18))?,
            ManagedPromotionSource::new(row.get::<_, String>(19), row.get::<_, String>(20))
                .map_err(|_| corrupt_error())?,
            row.get::<_, bool>(21),
            row.get::<_, String>(15),
        )
        .map_err(|_| corrupt_error())?;
        if intent.intent_digest() != &parse_digest(row.get(22))?
            || row.get::<_, String>(23).is_empty()
        {
            return Err(corrupt_error());
        }
        Ok(Some(intent))
    }

    /// Persists one owner-verified immutable promotion.
    ///
    /// # Errors
    ///
    /// Database rejection, changed retry, or out-of-range values fail closed.
    pub fn record_task_promotion(
        &mut self,
        record: &VerifiedTaskExecutionBinding,
        budget_value: &WorkerBudget,
        source: &ManagedPromotionSource,
    ) -> Result<AppendDisposition, AdapterError> {
        let link = record.link();
        let identity = link.expected_head().identity();
        let task_ref = digest_bytes(record.task_ref())?;
        let intake_stream = digest_bytes(record.intake_stream_id())?;
        let intake_event = digest_bytes(record.intake_event_digest())?;
        let project_receipt = digest_bytes(record.project_authority_receipt_digest())?;
        let successor_stream = digest_bytes(record.successor_stream_id())?;
        let successor_created = digest_bytes(record.successor_task_created_event_digest())?;
        let task_spec = digest_bytes(record.task_spec_digest())?;
        let approval = digest_bytes(record.approval_subject_digest())?;
        let budget = digest_bytes(record.budget_digest())?;
        let verification = digest_bytes(record.verification_policy_digest())?;
        let binding = digest_bytes(record.binding_digest())?;
        let budget_pointer = budget_value.digest();
        let budget_suffix = budget_pointer
            .strip_prefix("budget:sha256:")
            .ok_or_else(input_error)?;
        if budget_suffix != record.budget_digest().as_str()
            || budget_value.global_active_limit() != 4
            || budget_value.per_task_active_limit() != 1
        {
            return Err(input_error());
        }
        let (external_cost_status, external_cost_limit_micros) = match budget_value.external_cost()
        {
            ExternalCostBudget::Unavailable => ("UNAVAILABLE", None),
            ExternalCostBudget::LimitMicros(value) => ("LIMIT_MICROS", Some(to_i64(value)?)),
        };
        let global_active_limit = i16::from(budget_value.global_active_limit());
        let per_task_active_limit = i16::from(budget_value.per_task_active_limit());
        let repair_retry_limit = i16::from(budget_value.repair_retry_limit());
        let max_duration_seconds = to_i64(budget_value.max_duration_seconds())?;
        let max_total_tokens = to_i64(budget_value.max_total_tokens())?;
        let max_model_calls = i64::from(budget_value.max_model_calls());
        let event = event_sql(link)?;
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.record_task_promotion_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15, \
                    $16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27::text::numeric, \
                    $28,$29,$30,$31)",
                &[
                    &task_ref,
                    &identity.project_id().as_str(),
                    &identity.project_snapshot_id().as_str(),
                    &intake_stream,
                    &intake_event,
                    &project_receipt,
                    &successor_stream,
                    &successor_created,
                    &task_spec,
                    &approval,
                    &budget,
                    &global_active_limit,
                    &per_task_active_limit,
                    &repair_retry_limit,
                    &max_duration_seconds,
                    &max_total_tokens,
                    &max_model_calls,
                    &external_cost_status,
                    &external_cost_limit_micros,
                    &budget_value.deadline_at(),
                    &budget_pointer,
                    &verification,
                    &binding,
                    &source.base_ref(),
                    &source.base_commit(),
                    &event.stream_id,
                    &event.sequence,
                    &event.event_digest,
                    &event.command_id,
                    &event.request_digest,
                    &event.payload_digest,
                ],
            )
            .map_err(|error| database_error_at(AdapterDatabaseStage::TaskPromotion, &error))?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Loads the bounded source candidate captured by one promotion.
    ///
    /// The returned locator is structural evidence only. Callers must rebuild
    /// the canonical Task Spec and verify its binding before resuming work.
    ///
    /// # Errors
    ///
    /// Database failure or a malformed retained ref/commit fails closed.
    pub fn load_task_promotion_source(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<Option<ManagedPromotionSource>, AdapterError> {
        let task_ref = digest_bytes(task_ref)?;
        self.client
            .query_opt(
                "SELECT base_ref, base_commit \
                   FROM foreman_execution.read_task_promotion_source_v1($1)",
                &[&task_ref],
            )
            .map_err(|_| database_error())?
            .map(|row| {
                ManagedPromotionSource::new(row.get::<_, String>(0), row.get::<_, String>(1))
                    .map_err(|_| corrupt_error())
            })
            .transpose()
    }

    /// Durably reserves one exact Ledger-linked attempt before capacity admission.
    ///
    /// The reservation is not an active worker. An exact retry returns replay;
    /// a successful later claim atomically moves it into `worker_attempts`.
    ///
    /// # Errors
    ///
    /// Retry, model, binding, event, sequence, or substitution failures fail closed.
    pub fn reserve_worker_attempt(
        &mut self,
        record: &VerifiedWorkerAttemptRecord,
        maximum_attempts: u8,
    ) -> Result<ClaimReservationDisposition, AdapterError> {
        self.reserve_worker_attempt_with_execution_environment_ref(
            record,
            maximum_attempts,
            NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
        )
    }

    /// Durably reserves a packet and its exact native/typed environment ref.
    ///
    /// This is the production WSL2 surface. The native-compatible wrapper is
    /// retained for callers whose packet uses the frozen native sentinel.
    ///
    /// # Errors
    ///
    /// Invalid refs or any changed field on exact replay fail closed.
    pub fn reserve_worker_attempt_with_execution_environment_ref(
        &mut self,
        record: &VerifiedWorkerAttemptRecord,
        maximum_attempts: u8,
        execution_environment_ref: &str,
    ) -> Result<ClaimReservationDisposition, AdapterError> {
        validate_attempt_execution_environment_ref(execution_environment_ref)?;
        let task_ref = digest_bytes(record.task_ref())?;
        let successor = digest_bytes(record.successor_stream_id())?;
        let task_spec = digest_bytes(record.task_spec_digest())?;
        let binding = digest_bytes(record.binding_digest())?;
        let budget = digest_bytes(record.budget_digest())?;
        let checkpoint = digest_bytes(record.foreman_checkpoint_digest())?;
        let approval = digest_bytes(record.approval_receipt_digest())?;
        let packet = digest_bytes(record.packet_digest())?;
        let worktree = digest_bytes(record.worktree_digest())?;
        let base_commit = digest_bytes(record.base_commit_digest())?;
        let model_reason_digest = digest_bytes(record.model_reason_digest())?;
        let payload = digest_bytes(record.payload_digest())?;
        let event = event_sql(record.link())?;
        let attempt = i16::from(u8::try_from(record.attempt_number()).map_err(|_| input_error())?);
        let generation = to_i64(record.foreman_generation())?;
        let writer_fence = to_i64(record.writer_fence())?;
        let max_attempts = i16::from(maximum_attempts);
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.reserve_worker_attempt_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15, \
                    $16,$17,$18,$19,$20,$21,$22,$23,$24::text::numeric,$25,$26,$27)",
                &[
                    &task_ref,
                    &successor,
                    &task_spec,
                    &binding,
                    &budget,
                    &record.attempt_id().as_str(),
                    &attempt,
                    &generation,
                    &record.model().as_str(),
                    &record.reasoning().as_str(),
                    &writer_fence,
                    &checkpoint,
                    &approval,
                    &packet,
                    &execution_environment_ref,
                    &worktree,
                    &base_commit,
                    &record.model_reason().as_str(),
                    &model_reason_digest,
                    &record.claimed_at(),
                    &payload,
                    &max_attempts,
                    &event.stream_id,
                    &event.sequence,
                    &event.event_digest,
                    &event.command_id,
                    &event.request_digest,
                ],
            )
            .map_err(|error| claim_database_error(&error))?;
        match row.get::<_, String>(0).as_str() {
            "RESERVED" => Ok(ClaimReservationDisposition::Reserved),
            "EXACT_REPLAY" => Ok(ClaimReservationDisposition::ExactReplay),
            _ => Err(corrupt_error()),
        }
    }

    /// Loads the single untrusted pending attempt candidate for owner verification.
    ///
    /// # Errors
    ///
    /// Malformed fields, event-link drift, overflow, or duplicate rows fail closed.
    pub fn load_pending_worker_attempt(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<Option<PendingWorkerAttempt>, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let row = self
            .client
            .query_opt(
                "SELECT pg_catalog.encode(successor_stream_id,'hex'), \
                        pg_catalog.encode(task_spec_digest,'hex'), \
                        pg_catalog.encode(binding_digest,'hex'), \
                        pg_catalog.encode(budget_digest,'hex'), attempt_id, \
                        attempt_number, foreman_generation, model, reasoning, \
                        writer_fence, pg_catalog.encode(foreman_checkpoint_digest,'hex'), \
                        pg_catalog.encode(approval_receipt_digest,'hex'), \
                        pg_catalog.encode(packet_digest,'hex'), \
                        execution_environment_ref, \
                        pg_catalog.encode(worktree_digest,'hex'), \
                        pg_catalog.encode(base_commit_digest,'hex'), \
                        model_reason, pg_catalog.encode(model_reason_digest,'hex'), claimed_at, \
                        pg_catalog.encode(payload_digest,'hex'), max_attempts, \
                        reserved_at, ledger_event_digest \
                   FROM foreman_execution.read_pending_worker_attempt_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?;
        let Some(row) = row else {
            return Ok(None);
        };
        let attempt_number = u8::try_from(row.get::<_, i16>(5)).map_err(|_| corrupt_error())?;
        let execution_environment_ref = row.get::<_, String>(13);
        validate_attempt_execution_environment_ref(&execution_environment_ref)
            .map_err(|_| corrupt_error())?;
        let max_attempts = u8::try_from(row.get::<_, i16>(20)).map_err(|_| corrupt_error())?;
        if !(1..=3).contains(&attempt_number)
            || !(1..=3).contains(&max_attempts)
            || attempt_number > max_attempts
        {
            return Err(corrupt_error());
        }
        let event_digest: Vec<u8> = row.get(22);
        let persisted = UntrustedWorkerAttemptRow::new(
            WORKER_ATTEMPT_RECORD_SCHEMA,
            self.load_event_link(&event_digest)?,
            task_ref.clone(),
            parse_digest(row.get(0))?,
            parse_digest(row.get(1))?,
            parse_digest(row.get(2))?,
            parse_digest(row.get(3))?,
            AttemptId::new(row.get::<_, String>(4)).map_err(|_| corrupt_error())?,
            u64::from(attempt_number),
            u64::try_from(row.get::<_, i64>(6)).map_err(|_| corrupt_error())?,
            WorkerModel::from_persisted(row.get::<_, String>(7).as_str())
                .map_err(|_| corrupt_error())?,
            ReasoningEffort::from_persisted(row.get::<_, String>(8).as_str())
                .map_err(|_| corrupt_error())?,
            ModelReason::from_persisted(row.get::<_, String>(16).as_str())
                .map_err(|_| corrupt_error())?,
            u64::try_from(row.get::<_, i64>(9)).map_err(|_| corrupt_error())?,
            parse_digest(row.get(10))?,
            parse_digest(row.get(11))?,
            parse_digest(row.get(12))?,
            parse_digest(row.get(14))?,
            parse_digest(row.get(15))?,
            parse_digest(row.get(17))?,
            row.get::<_, String>(18),
            parse_digest(row.get(19))?,
        );
        Ok(Some(PendingWorkerAttempt {
            row: persisted,
            execution_environment_ref,
            max_attempts,
            reserved_at: row.get(21),
        }))
    }

    /// Persists or exactly replays the immutable environment selected by a
    /// reserved/claimed attempt.
    ///
    /// The database independently recomputes both the execution-domain and
    /// descriptor digests. A changed path, tool, credential-authority digest,
    /// descriptor ref, attempt ID, or packet digest fails closed.
    ///
    /// # Errors
    ///
    /// Missing attempt authority, malformed input, digest mismatch,
    /// substitution, or database failure returns a closed adapter error.
    #[allow(clippy::too_many_lines)]
    pub fn record_execution_environment(
        &mut self,
        record: &VerifiedWorkerAttemptRecord,
        descriptor: &ExecutionEnvironmentDescriptor,
    ) -> Result<AppendDisposition, AdapterError> {
        if descriptor.verification_task_ref() != record.task_ref()
            || descriptor.path_mapping_linux_path() != descriptor.linux_repository_path()
        {
            return Err(input_error());
        }
        let task_ref = digest_bytes(record.task_ref())?;
        let packet_digest = digest_bytes(record.packet_digest())?;
        let attempt_number = u8::try_from(record.attempt_number()).map_err(|_| input_error())?;
        let attempt = i16::from(attempt_number);
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.record_execution_environment_v1( \
                    $1,$2,$3,$4,$5,$6)",
                &[
                    &task_ref,
                    &attempt,
                    &record.attempt_id().as_str(),
                    &packet_digest,
                    &descriptor.canonical_json(),
                    &descriptor.environment_ref().as_str(),
                ],
            )
            .map_err(|error| execution_environment_database_error(&error))?;
        let disposition = parse_append(row.get::<_, String>(0).as_str())?;
        let retained = self
            .load_execution_environment(record.task_ref(), u64::from(attempt_number))?
            .ok_or_else(corrupt_error)?;
        if retained.attempt_id() != record.attempt_id()
            || retained.packet_digest() != record.packet_digest()
            || retained.descriptor() != descriptor
        {
            return Err(corrupt_error());
        }
        Ok(disposition)
    }

    /// Reconstructs and rehashes every retained environment for one task.
    ///
    /// This is the fresh-process/restart surface. Returned rows remain bound to
    /// the exact pending or active attempt ID and packet digest.
    ///
    /// # Errors
    ///
    /// Malformed paths/tools, digest/ref substitution, orphaned rows,
    /// duplicate attempts, or database failure fails closed.
    pub fn load_execution_environments(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<Vec<PersistedExecutionEnvironment>, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let rows = self
            .client
            .query(
                "SELECT attempt_number, attempt_id, \
                        pg_catalog.encode(packet_digest,'hex'), canonical_descriptor, \
                        pg_catalog.encode(execution_domain_digest,'hex'), environment_ref, \
                        pg_catalog.to_char( \
                            recorded_at AT TIME ZONE 'UTC', \
                            'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"' \
                        ) \
                   FROM foreman_execution.read_execution_environment_rows_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|error| execution_environment_read_database_error(&error))?;
        let mut environments = Vec::with_capacity(rows.len());
        for row in rows {
            environments.push(execution_environment_from_row(task_ref, &row)?);
        }
        if environments
            .windows(2)
            .any(|pair| pair[0].attempt_number() >= pair[1].attempt_number())
        {
            return Err(corrupt_error());
        }
        Ok(environments)
    }

    /// Reconstructs one exact attempt environment through the bounded task reader.
    ///
    /// # Errors
    ///
    /// Invalid attempt number, duplicate/malformed persisted rows, digest/ref
    /// substitution, or database failure fails closed.
    pub fn load_execution_environment(
        &mut self,
        task_ref: &ContentDigest,
        attempt_number: u64,
    ) -> Result<Option<PersistedExecutionEnvironment>, AdapterError> {
        let attempt_number = u8::try_from(attempt_number).map_err(|_| input_error())?;
        if !(1..=3).contains(&attempt_number) {
            return Err(input_error());
        }
        let mut matching = self
            .load_execution_environments(task_ref)?
            .into_iter()
            .filter(|environment| environment.attempt_number() == attempt_number);
        let result = matching.next();
        if matching.next().is_some() {
            return Err(corrupt_error());
        }
        Ok(result)
    }

    /// Atomically persists one owner-verified attempt under the closed capacities.
    ///
    /// # Errors
    ///
    /// Capacity, retry, model, binding, event, database, or substitution failures
    /// are returned without launching a worker.
    pub fn claim_worker_attempt(
        &mut self,
        record: &VerifiedWorkerAttemptRecord,
        maximum_attempts: u8,
    ) -> Result<ClaimOutcome, AdapterError> {
        self.claim_worker_attempt_with_execution_environment_ref(
            record,
            maximum_attempts,
            NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
        )
    }

    /// Claims one exact packet under its durably reserved environment ref.
    ///
    /// Non-native refs require the exact descriptor row before capacity or
    /// provider authority can be acquired. The native sentinel requires that
    /// no WSL2 descriptor row exists.
    ///
    /// # Errors
    ///
    /// Missing/substituted refs or normal claim policy failures fail closed.
    pub fn claim_worker_attempt_with_execution_environment_ref(
        &mut self,
        record: &VerifiedWorkerAttemptRecord,
        maximum_attempts: u8,
        execution_environment_ref: &str,
    ) -> Result<ClaimOutcome, AdapterError> {
        validate_attempt_execution_environment_ref(execution_environment_ref)?;
        let task_ref = digest_bytes(record.task_ref())?;
        let successor = digest_bytes(record.successor_stream_id())?;
        let task_spec = digest_bytes(record.task_spec_digest())?;
        let binding = digest_bytes(record.binding_digest())?;
        let budget = digest_bytes(record.budget_digest())?;
        let checkpoint = digest_bytes(record.foreman_checkpoint_digest())?;
        let approval = digest_bytes(record.approval_receipt_digest())?;
        let packet = digest_bytes(record.packet_digest())?;
        let worktree = digest_bytes(record.worktree_digest())?;
        let base_commit = digest_bytes(record.base_commit_digest())?;
        let model_reason_digest = digest_bytes(record.model_reason_digest())?;
        let payload = digest_bytes(record.payload_digest())?;
        let event = event_sql(record.link())?;
        let attempt = i16::from(u8::try_from(record.attempt_number()).map_err(|_| input_error())?);
        let generation = to_i64(record.foreman_generation())?;
        let writer_fence = to_i64(record.writer_fence())?;
        let max_attempts = i16::from(maximum_attempts);
        let row = self
            .client
            .query_one(
                "SELECT disposition, global_active, task_active \
                   FROM foreman_execution.claim_worker_attempt_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15, \
                    $16,$17,$18,$19,$20,$21,$22,$23,$24::text::numeric,$25,$26,$27)",
                &[
                    &task_ref,
                    &successor,
                    &task_spec,
                    &binding,
                    &budget,
                    &record.attempt_id().as_str(),
                    &attempt,
                    &generation,
                    &record.model().as_str(),
                    &record.reasoning().as_str(),
                    &writer_fence,
                    &checkpoint,
                    &approval,
                    &packet,
                    &execution_environment_ref,
                    &worktree,
                    &base_commit,
                    &record.model_reason().as_str(),
                    &model_reason_digest,
                    &record.claimed_at(),
                    &payload,
                    &max_attempts,
                    &event.stream_id,
                    &event.sequence,
                    &event.event_digest,
                    &event.command_id,
                    &event.request_digest,
                ],
            )
            .map_err(|error| claim_database_error(&error))?;
        let disposition = match row.get::<_, String>(0).as_str() {
            "CLAIMED" => ClaimDisposition::Claimed,
            "EXACT_REPLAY" => ClaimDisposition::ExactReplay,
            _ => return Err(corrupt_error()),
        };
        let global_active: i64 = row.get(1);
        let task_active: i64 = row.get(2);
        Ok(ClaimOutcome {
            disposition,
            global_active: u8::try_from(global_active).map_err(|_| corrupt_error())?,
            task_active: u8::try_from(task_active).map_err(|_| corrupt_error())?,
        })
    }

    /// Atomically claims one exact provider effect beneath a retained attempt.
    ///
    /// `anchor_digest` and `supporting_digest` must identify the operation's
    /// already-retained lifecycle/artifact rows. An identical retry returns
    /// `ExactReplay`; any changed field fails closed inside the locked function.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when input validation, current Writer/Foreman
    /// fencing, retained evidence, database execution, or exact replay fails.
    pub fn claim_provider_dispatch(
        &mut self,
        record: &VerifiedWorkerAttemptRecord,
        kind: ProviderDispatchKind,
        anchor_digest: &ContentDigest,
        supporting_digest: &ContentDigest,
        subject_digest: &ContentDigest,
    ) -> Result<ClaimDisposition, AdapterError> {
        let task_ref = digest_bytes(record.task_ref())?;
        let binding = digest_bytes(record.binding_digest())?;
        let checkpoint = digest_bytes(record.foreman_checkpoint_digest())?;
        let anchor = digest_bytes(anchor_digest)?;
        let supporting = digest_bytes(supporting_digest)?;
        let subject = digest_bytes(subject_digest)?;
        let foreman_stream = digest_bytes(&self.foreman_coordination_stream_id)?;
        let dispatch_digest = provider_dispatch_digest(
            kind,
            record.task_ref(),
            u8::try_from(record.attempt_number()).map_err(|_| input_error())?,
            record.attempt_id(),
            record.binding_digest(),
            record.writer_fence(),
            record.foreman_generation(),
            record.foreman_checkpoint_digest(),
            anchor_digest,
            supporting_digest,
            subject_digest,
        )?;
        let dispatch = digest_bytes(&dispatch_digest)?;
        let attempt = i16::from(u8::try_from(record.attempt_number()).map_err(|_| input_error())?);
        let writer_fence = to_i64(record.writer_fence())?;
        let generation = to_i64(record.foreman_generation())?;
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.claim_provider_dispatch_v1(\
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                &[
                    &task_ref,
                    &attempt,
                    &kind.as_str(),
                    &record.attempt_id().as_str(),
                    &binding,
                    &writer_fence,
                    &generation,
                    &checkpoint,
                    &anchor,
                    &supporting,
                    &subject,
                    &dispatch,
                    &foreman_stream,
                ],
            )
            .map_err(|error| provider_dispatch_database_error(&error))?;
        let disposition = match row.get::<_, String>(0).as_str() {
            "CLAIMED" => ClaimDisposition::Claimed,
            "EXACT_REPLAY" => ClaimDisposition::ExactReplay,
            _ => return Err(corrupt_error()),
        };
        let retained = self
            .load_provider_dispatch_claim(record.task_ref(), record.attempt_number(), kind)?
            .ok_or_else(corrupt_error)?;
        if retained.attempt_id != *record.attempt_id()
            || retained.binding_digest != *record.binding_digest()
            || retained.writer_fence != record.writer_fence()
            || retained.foreman_generation != record.foreman_generation()
            || retained.foreman_checkpoint_digest != *record.foreman_checkpoint_digest()
            || retained.anchor_digest != *anchor_digest
            || retained.supporting_digest != *supporting_digest
            || retained.subject_digest != *subject_digest
            || retained.dispatch_digest != dispatch_digest
        {
            return Err(corrupt_error());
        }
        Ok(disposition)
    }

    /// Loads and rehashes one exact provider dispatch receipt for restart/status.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when the identity is invalid, the database is
    /// unavailable, or retained dispatch evidence is malformed or tampered.
    pub fn load_provider_dispatch_claim(
        &mut self,
        task_ref: &ContentDigest,
        attempt_number: u64,
        kind: ProviderDispatchKind,
    ) -> Result<Option<ProviderDispatchClaim>, AdapterError> {
        let attempt = u8::try_from(attempt_number).map_err(|_| input_error())?;
        if !(1..=3).contains(&attempt) {
            return Err(input_error());
        }
        let task_ref_bytes = digest_bytes(task_ref)?;
        let row = self
            .client
            .query_opt(
                "SELECT attempt_id, pg_catalog.encode(binding_digest,'hex'),\
                        writer_fence, foreman_generation,\
                        pg_catalog.encode(foreman_checkpoint_digest,'hex'),\
                        pg_catalog.encode(anchor_digest,'hex'),\
                        pg_catalog.encode(supporting_digest,'hex'),\
                        pg_catalog.encode(subject_digest,'hex'),\
                        pg_catalog.encode(dispatch_digest,'hex'), \
                        pg_catalog.encode(claim_receipt_digest,'hex'), claimed_at \
                   FROM foreman_execution.read_provider_dispatch_claim_v1($1,$2,$3)",
                &[&task_ref_bytes, &i16::from(attempt), &kind.as_str()],
            )
            .map_err(|_| database_error())?;
        let Some(row) = row else {
            return Ok(None);
        };
        let claim = ProviderDispatchClaim {
            kind: ProviderDispatchKind::parse(kind.as_str())?,
            task_ref: task_ref.clone(),
            attempt_number: attempt,
            attempt_id: AttemptId::new(row.get::<_, String>(0)).map_err(|_| corrupt_error())?,
            binding_digest: parse_digest(row.get(1))?,
            writer_fence: u64::try_from(row.get::<_, i64>(2)).map_err(|_| corrupt_error())?,
            foreman_generation: u64::try_from(row.get::<_, i64>(3)).map_err(|_| corrupt_error())?,
            foreman_checkpoint_digest: parse_digest(row.get(4))?,
            anchor_digest: parse_digest(row.get(5))?,
            supporting_digest: parse_digest(row.get(6))?,
            subject_digest: parse_digest(row.get(7))?,
            dispatch_digest: parse_digest(row.get(8))?,
            claim_receipt_digest: parse_digest(row.get(9))?,
            claimed_at: row.get(10),
        };
        let expected = provider_dispatch_digest(
            claim.kind,
            &claim.task_ref,
            claim.attempt_number,
            &claim.attempt_id,
            &claim.binding_digest,
            claim.writer_fence,
            claim.foreman_generation,
            &claim.foreman_checkpoint_digest,
            &claim.anchor_digest,
            &claim.supporting_digest,
            &claim.subject_digest,
        )?;
        let expected_receipt =
            provider_dispatch_receipt_digest(&claim.dispatch_digest, &claim.claimed_at)?;
        if claim.dispatch_digest != expected
            || claim.claim_receipt_digest != expected_receipt
            || claim.claimed_at.is_empty()
            || claim.claimed_at.len() > 40
        {
            return Err(corrupt_error());
        }
        Ok(Some(claim))
    }

    /// Persists one owner-verified exact thread/turn lifecycle observation.
    ///
    /// # Errors
    ///
    /// Lifecycle order, exact-ID substitution, or database failure fails closed.
    pub fn record_worker_observation(
        &mut self,
        record: &VerifiedWorkerObservationRecord,
    ) -> Result<(AppendDisposition, u64), AdapterError> {
        let task_ref = digest_bytes(record.task_ref())?;
        let successor = digest_bytes(record.successor_stream_id())?;
        let binding = digest_bytes(record.binding_digest())?;
        let app_server_identity = digest_bytes(record.app_server_identity_digest())?;
        let evidence = digest_bytes(record.evidence_digest())?;
        let payload = digest_bytes(record.payload_digest())?;
        let event = event_sql(record.link())?;
        let attempt = i16::from(u8::try_from(record.attempt_number()).map_err(|_| input_error())?);
        let generation = to_i64(record.app_server_generation())?;
        let row = self
            .client
            .query_one(
                "SELECT disposition, observation_ordinal \
                   FROM foreman_execution.record_worker_observation_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14, \
                    $15::text::numeric,$16,$17,$18)",
                &[
                    &task_ref,
                    &successor,
                    &binding,
                    &record.attempt_id().as_str(),
                    &attempt,
                    &record.kind().as_str(),
                    &record.thread_id(),
                    &record.turn_id(),
                    &generation,
                    &app_server_identity,
                    &record.observed_at(),
                    &evidence,
                    &payload,
                    &event.stream_id,
                    &event.sequence,
                    &event.event_digest,
                    &event.command_id,
                    &event.request_digest,
                ],
            )
            .map_err(|_| database_error())?;
        let disposition = parse_append(row.get::<_, String>(0).as_str())?;
        let ordinal: i64 = row.get(1);
        Ok((
            disposition,
            u64::try_from(ordinal).map_err(|_| corrupt_error())?,
        ))
    }

    /// Persists one owner-verified independent verification row.
    ///
    /// # Errors
    ///
    /// A non-terminal attempt, changed retry, or database failure fails closed.
    pub fn record_verification(
        &mut self,
        record: &VerifiedTaskVerificationRecord,
    ) -> Result<AppendDisposition, AdapterError> {
        let task_ref = digest_bytes(record.task_ref())?;
        let successor = digest_bytes(record.successor_stream_id())?;
        let task_spec = digest_bytes(record.task_spec_digest())?;
        let binding = digest_bytes(record.binding_digest())?;
        let profile = digest_bytes(record.verification_profile_digest())?;
        let base_commit = digest_bytes(record.base_commit_digest())?;
        let result_commit = digest_bytes(record.result_commit_digest())?;
        let tree = digest_bytes(record.tree_digest())?;
        let diff = digest_bytes(record.diff_digest())?;
        let result = digest_bytes(record.result_digest())?;
        let artifact = digest_bytes(record.evidence_artifact_digest())?;
        let review = record.review_digest().map(digest_bytes).transpose()?;
        let payload = digest_bytes(record.payload_digest())?;
        let event = event_sql(record.link())?;
        let attempt = i16::from(u8::try_from(record.attempt_number()).map_err(|_| input_error())?);
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.record_verification_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15, \
                    $16,$17,$18,$19::text::numeric,$20,$21,$22)",
                &[
                    &task_ref,
                    &successor,
                    &task_spec,
                    &binding,
                    &record.attempt_id().as_str(),
                    &attempt,
                    &record.outcome().as_str(),
                    &profile,
                    &base_commit,
                    &result_commit,
                    &tree,
                    &diff,
                    &result,
                    &artifact,
                    &review,
                    &record.verified_at(),
                    &payload,
                    &event.stream_id,
                    &event.sequence,
                    &event.event_digest,
                    &event.command_id,
                    &event.request_digest,
                ],
            )
            .map_err(|_| database_error())?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Durably stages owner-verified evidence and its exact planned Ledger link.
    ///
    /// # Errors
    ///
    /// Cross-task/head binding, changed same-task pending input, quota overflow,
    /// or database failure fails closed without appending a child row.
    pub fn stage_artifact_reference(
        &mut self,
        evidence: &VerifiedManagedEvidence,
        link: &TaskRuntimeEventLink,
        correlation_id: &CorrelationId,
        command_occurred_at: &str,
    ) -> Result<AppendDisposition, AdapterError> {
        if link.payload_digest() != evidence.descriptor_digest()
            || command_occurred_at.is_empty()
            || command_occurred_at.len() > 40
        {
            return Err(input_error());
        }
        let task_ref = digest_bytes(evidence.task_ref())?;
        let producer = digest_bytes(evidence.producer_digest())?;
        let content = digest_bytes(evidence.content_digest())?;
        let descriptor_bytes = evidence
            .canonical_descriptor_bytes()
            .map_err(|_| input_error())?;
        let descriptor = digest_bytes(evidence.descriptor_digest())?;
        let event = event_sql(link)?;
        let attempt = i16::from(evidence.attempt());
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.stage_artifact_reference_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15, \
                    $16::text::numeric,$17,$18::text::numeric,$19,$20, \
                    $21::text::numeric,$22,$23,$24,$25,$26,$27)",
                &[
                    &evidence.project_id().as_str(),
                    &task_ref,
                    &attempt,
                    &evidence.kind().as_str(),
                    &evidence.media_type(),
                    &evidence.payload_schema(),
                    &evidence.producer_id(),
                    &evidence.producer_version(),
                    &producer,
                    &evidence.created_at(),
                    &evidence.bytes(),
                    &content,
                    &descriptor_bytes,
                    &descriptor,
                    &event.stream_id,
                    &event.before_sequence,
                    &event.before_last_event_digest,
                    &event.before_resource_revision,
                    &event.before_resource_projection_digest,
                    &event.before_head_digest,
                    &event.sequence,
                    &event.event_digest,
                    &event.command_id,
                    &event.request_digest,
                    &event.payload_digest,
                    &correlation_id.as_str(),
                    &command_occurred_at,
                ],
            )
            .map_err(|error| artifact_database_error(&error))?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Atomically finalizes one exact staged artifact after its Ledger event.
    ///
    /// The fixed SQL function verifies the exact retained event, inserts the
    /// child row and artifact bytes, and deletes the stage in one transaction.
    /// A commit-unknown retry resolves from the retained artifact as exact
    /// replay even though the stage has already been consumed.
    ///
    /// # Errors
    ///
    /// A missing or substituted stage/event, malformed identity, or database
    /// failure fails closed.
    pub fn finalize_staged_artifact_reference(
        &mut self,
        task_ref: &ContentDigest,
        attempt: u8,
        descriptor_digest: &ContentDigest,
    ) -> Result<AppendDisposition, AdapterError> {
        if !(1..=3).contains(&attempt) {
            return Err(input_error());
        }
        let task_ref = digest_bytes(task_ref)?;
        let descriptor = digest_bytes(descriptor_digest)?;
        let attempt = i16::from(attempt);
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.finalize_staged_artifact_reference_v1($1,$2,$3)",
                &[&task_ref, &attempt, &descriptor],
            )
            .map_err(|error| artifact_database_error(&error))?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Acquires the Foreman-wide session serialization guard used only while
    /// deciding and appending a restart Writer blocker. Every durable attempt
    /// lane uses the matching transaction advisory key, so none can commit
    /// between the guarded predicate reload and Artifact outbox finalize.
    ///
    /// # Errors
    ///
    /// Invalid identity or database failure fails closed.
    pub fn begin_restart_writer_blocker_guard(
        &mut self,
        task_ref: &ContentDigest,
        attempt: u8,
    ) -> Result<(), AdapterError> {
        if !(1..=3).contains(&attempt) {
            return Err(input_error());
        }
        let task_ref = digest_bytes(task_ref)?;
        let attempt = i16::from(attempt);
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.begin_restart_writer_blocker_guard_v1($1,$2)",
                &[&task_ref, &attempt],
            )
            .map_err(|_| database_error())?;
        if !row.get::<_, bool>(0) {
            return Err(corrupt_error());
        }
        Ok(())
    }

    /// Releases the exact session guard acquired by
    /// [`Self::begin_restart_writer_blocker_guard`].
    ///
    /// # Errors
    ///
    /// Missing lock ownership or database failure fails closed.
    pub fn end_restart_writer_blocker_guard(
        &mut self,
        task_ref: &ContentDigest,
        attempt: u8,
    ) -> Result<(), AdapterError> {
        if !(1..=3).contains(&attempt) {
            return Err(input_error());
        }
        let task_ref = digest_bytes(task_ref)?;
        let attempt = i16::from(attempt);
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.end_restart_writer_blocker_guard_v1($1,$2)",
                &[&task_ref, &attempt],
            )
            .map_err(|_| database_error())?;
        if !row.get::<_, bool>(0) {
            return Err(corrupt_error());
        }
        Ok(())
    }

    /// Records one exact blocker-backed proof that no provider effect remains live.
    ///
    /// This is deliberately separate from verification: closing failed work never
    /// creates a verification row. `PostgreSQL` revalidates the blocker artifact,
    /// attempt fence, and no-effect/exact-terminal precondition.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when the closure input, current Writer fence,
    /// blocker evidence, provider disposition, or database effect is invalid.
    pub fn record_attempt_closure(
        &mut self,
        task_ref: &ContentDigest,
        attempt: u8,
        blocker_code: &str,
        blocker_descriptor_digest: &ContentDigest,
        writer_fence: u64,
    ) -> Result<AppendDisposition, AdapterError> {
        if !(1..=3).contains(&attempt) || writer_fence == 0 || !closed_attempt_blocker(blocker_code)
        {
            return Err(input_error());
        }
        let task_ref = digest_bytes(task_ref)?;
        let blocker_descriptor = digest_bytes(blocker_descriptor_digest)?;
        let attempt = i16::from(attempt);
        let writer_fence = to_i64(writer_fence)?;
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.record_attempt_closure_v1($1,$2,$3,$4,$5)",
                &[
                    &task_ref,
                    &attempt,
                    &blocker_code,
                    &blocker_descriptor,
                    &writer_fence,
                ],
            )
            .map_err(|error| attempt_closure_database_error(&error))?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Atomically materializes and closes one exact pending worker attempt
    /// after its Ledger-linked no-provider-effect blocker has been staged.
    ///
    /// # Errors
    ///
    /// A changed pending packet, missing staged blocker, retained provider
    /// effect, stale fence, or database ambiguity fails closed.
    pub fn close_pending_worker_attempt(
        &mut self,
        task_ref: &ContentDigest,
        attempt: u8,
        blocker_code: &str,
        blocker_descriptor_digest: &ContentDigest,
        writer_fence: u64,
    ) -> Result<AppendDisposition, AdapterError> {
        if !(1..=3).contains(&attempt) || writer_fence == 0 || !closed_attempt_blocker(blocker_code)
        {
            return Err(input_error());
        }
        let task_ref = digest_bytes(task_ref)?;
        let blocker_descriptor = digest_bytes(blocker_descriptor_digest)?;
        let attempt = i16::from(attempt);
        let writer_fence = to_i64(writer_fence)?;
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.close_pending_worker_attempt_v1($1,$2,$3,$4,$5)",
                &[
                    &task_ref,
                    &attempt,
                    &blocker_code,
                    &blocker_descriptor,
                    &writer_fence,
                ],
            )
            .map_err(|error| attempt_closure_database_error(&error))?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Atomically binds one immutable retained provider blocker to a separate
    /// exact no-effect reconciliation proof and closes the attempt.
    ///
    /// The SQL owner serializes this with provider claims and validates both
    /// Artifact Store descriptors, their canonical bounded payloads, the
    /// current attempt fence, and the exact durable dispatch/observation shape.
    ///
    /// # Errors
    ///
    /// A substituted blocker/proof, stale fence, still-possible provider
    /// effect, malformed identity, or database ambiguity fails closed.
    #[allow(clippy::too_many_arguments)]
    pub fn close_retained_worker_without_provider_effect(
        &mut self,
        task_ref: &ContentDigest,
        attempt: u8,
        blocker_code: &str,
        blocker_descriptor_digest: &ContentDigest,
        reconciliation_proof_descriptor_digest: &ContentDigest,
        writer_fence: u64,
    ) -> Result<AppendDisposition, AdapterError> {
        if !(1..=3).contains(&attempt)
            || writer_fence == 0
            || !retained_worker_blocker(blocker_code)
            || blocker_descriptor_digest == reconciliation_proof_descriptor_digest
        {
            return Err(input_error());
        }
        let task_ref = digest_bytes(task_ref)?;
        let blocker_descriptor = digest_bytes(blocker_descriptor_digest)?;
        let proof_descriptor = digest_bytes(reconciliation_proof_descriptor_digest)?;
        let attempt = i16::from(attempt);
        let writer_fence = to_i64(writer_fence)?;
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.close_retained_worker_without_provider_effect_v1(\
                    $1,$2,$3,$4,$5,$6)",
                &[
                    &task_ref,
                    &attempt,
                    &blocker_code,
                    &blocker_descriptor,
                    &proof_descriptor,
                    &writer_fence,
                ],
            )
            .map_err(|error| attempt_closure_database_error(&error))?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Loads one exact typed attempt closure for restart reconciliation.
    ///
    /// # Errors
    ///
    /// Returns an adapter error when the attempt identity is invalid, the
    /// database is unavailable, or the retained closure is malformed.
    pub fn load_attempt_closure(
        &mut self,
        task_ref: &ContentDigest,
        attempt: u8,
    ) -> Result<Option<AttemptClosure>, AdapterError> {
        if !(1..=3).contains(&attempt) {
            return Err(input_error());
        }
        let task_ref = digest_bytes(task_ref)?;
        self.client
            .query_opt(
                "SELECT provider_disposition, blocker_code, \
                        pg_catalog.encode(blocker_descriptor_digest,'hex'), \
                        pg_catalog.encode(reconciliation_proof_descriptor_digest,'hex'), \
                        writer_fence, closed_at \
                   FROM foreman_execution.read_attempt_closure_v1($1,$2)",
                &[&task_ref, &i16::from(attempt)],
            )
            .map_err(|_| database_error())?
            .map(|row| {
                let disposition: String = row.get(0);
                let blocker_code: String = row.get(1);
                let proof = row
                    .get::<_, Option<String>>(3)
                    .map(parse_digest)
                    .transpose()?;
                let writer_fence =
                    u64::try_from(row.get::<_, i64>(4)).map_err(|_| corrupt_error())?;
                let regular = closed_attempt_blocker(&blocker_code) && proof.is_none();
                let reconciled = retained_worker_blocker(&blocker_code) && proof.is_some();
                if disposition != "PROVEN_INACTIVE"
                    || (!regular && !reconciled)
                    || writer_fence == 0
                {
                    return Err(corrupt_error());
                }
                Ok(AttemptClosure {
                    blocker_code,
                    blocker_descriptor_digest: parse_digest(row.get(2))?,
                    reconciliation_proof_descriptor_digest: proof,
                    writer_fence,
                    closed_at: row.get(5),
                })
            })
            .transpose()
    }

    /// Persists one owner-verified, task/spec/budget-bound local authority.
    ///
    /// # Errors
    ///
    /// Cross-binding, external capability, changed retry, or database failure
    /// fails closed.
    pub fn record_approval_evidence(
        &mut self,
        authority: &VerifiedExecutionAuthority,
        link: &TaskRuntimeEventLink,
    ) -> Result<AppendDisposition, AdapterError> {
        if authority.source() != ExecutionAuthoritySource::ClosedPolicyNoApprovalRequired {
            return Err(input_error());
        }
        self.record_approval_evidence_with_owner(authority, link, None)
    }

    /// Persists one verified-approval authority together with the complete
    /// Approval-owner snapshot and independent checkpoint in the same
    /// transaction. The Task authority row retains only the snapshot digest.
    ///
    /// # Errors
    ///
    /// Missing binding receipts, snapshot rollback/substitution, cross-bound
    /// approval state, or database failure fails closed.
    pub fn record_verified_approval_evidence(
        &mut self,
        authority: &VerifiedExecutionAuthority,
        link: &TaskRuntimeEventLink,
        approval_verifier: &FakeApprovalVerifier,
    ) -> Result<AppendDisposition, AdapterError> {
        if authority.source() != ExecutionAuthoritySource::VerifiedApproval {
            return Err(input_error());
        }
        let owner = approval_owner_snapshot_sql(authority, approval_verifier, input_error)?;
        self.record_approval_evidence_with_owner(authority, link, Some(owner))
    }

    fn record_approval_evidence_with_owner(
        &mut self,
        authority: &VerifiedExecutionAuthority,
        link: &TaskRuntimeEventLink,
        owner: Option<ApprovalOwnerSnapshotSql>,
    ) -> Result<AppendDisposition, AdapterError> {
        let task_ref = digest_bytes(authority.task_ref())?;
        let successor = digest_bytes(authority.successor_stream_id())?;
        let task_spec = digest_bytes(authority.task_spec_digest())?;
        let approval_subject = digest_bytes(authority.approval_subject_digest())?;
        let budget = digest_bytes(authority.budget_digest())?;
        let authority_evidence = digest_bytes(authority.authority_evidence_digest())?;
        let receipt = authority
            .approval_receipt_digest()
            .map(digest_bytes)
            .transpose()?;
        let authority_digest = digest_bytes(authority.authority_digest())?;
        let owner_snapshot_digest = owner
            .as_ref()
            .map(|value| digest_bytes(&value.checkpoint_snapshot_digest))
            .transpose()?;
        let owner_snapshot_content_digest = owner
            .as_ref()
            .map(|value| digest_bytes(&value.snapshot_content_digest))
            .transpose()?;
        let owner_snapshot_bytes = owner.as_ref().map(|value| value.snapshot_bytes.clone());
        let owner_command_high_water = owner
            .as_ref()
            .map(|value| i64::try_from(value.command_high_water).map_err(|_| input_error()))
            .transpose()?;
        let owner_command_tail_digest = owner
            .as_ref()
            .map(|value| digest_bytes(&value.command_tail_digest))
            .transpose()?;
        let owner_nonce_bindings_digest = owner
            .as_ref()
            .map(|value| digest_bytes(&value.nonce_bindings_digest))
            .transpose()?;
        let event = event_sql(link)?;
        let row = self
            .client
            .query_one(
                "SELECT foreman_execution.record_approval_evidence_v1( \
                    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13, \
                    $14,$15,$16,$17,$18,$19, \
                    $20::text::numeric,$21,$22,$23,$24)",
                &[
                    &task_ref,
                    &successor,
                    &task_spec,
                    &approval_subject,
                    &budget,
                    &authority.source().as_str(),
                    &authority.capability().as_str(),
                    &authority_evidence,
                    &receipt,
                    &authority.issued_at(),
                    &authority.expires_at(),
                    &authority_digest,
                    &owner_snapshot_digest,
                    &owner_snapshot_content_digest,
                    &owner_snapshot_bytes,
                    &owner_command_high_water,
                    &owner_command_tail_digest,
                    &owner_nonce_bindings_digest,
                    &event.stream_id,
                    &event.sequence,
                    &event.event_digest,
                    &event.command_id,
                    &event.request_digest,
                    &event.payload_digest,
                ],
            )
            .map_err(|_| database_error())?;
        parse_append(row.get::<_, String>(0).as_str())
    }

    /// Reads the stable append-only replay index for status/reconciliation.
    ///
    /// # Errors
    ///
    /// Malformed rows, numeric overflow, or database failure fails closed.
    pub fn read_task_replay(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<TaskReplay, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let rows = self
            .client
            .query(
                "SELECT record_kind, record_state, attempt_number, record_ordinal, \
                        pg_catalog.encode(record_digest,'hex'), \
                        pg_catalog.encode(ledger_stream_id,'hex'), \
                        ledger_event_sequence::text, \
                        pg_catalog.encode(ledger_event_digest,'hex'), recorded_at \
                   FROM foreman_execution.read_task_replay_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?;
        let mut records = Vec::with_capacity(rows.len());
        for row in rows {
            // Treat a torn or physically corrupted projection as typed replay
            // corruption. `Row::get` panics on NULL/type drift, which would let
            // an orphaned child row terminate the foreman instead of failing
            // closed and entering reconciliation.
            let record_kind: String = row.try_get(0).map_err(|_| corrupt_error())?;
            let record_state: String = row.try_get(1).map_err(|_| corrupt_error())?;
            let attempt: Option<i16> = row.try_get(2).map_err(|_| corrupt_error())?;
            let ordinal: i64 = row.try_get(3).map_err(|_| corrupt_error())?;
            let record_digest: String = row.try_get(4).map_err(|_| corrupt_error())?;
            let ledger_stream_id: String = row.try_get(5).map_err(|_| corrupt_error())?;
            let sequence: String = row.try_get(6).map_err(|_| corrupt_error())?;
            let ledger_event_digest: String = row.try_get(7).map_err(|_| corrupt_error())?;
            let recorded_at: String = row.try_get(8).map_err(|_| corrupt_error())?;
            records.push(ReplayRecord {
                record_kind,
                record_state: ReplayRecordState::parse(&record_state)?,
                attempt_number: attempt
                    .map(|value| u8::try_from(value).map_err(|_| corrupt_error()))
                    .transpose()?,
                record_ordinal: u64::try_from(ordinal).map_err(|_| corrupt_error())?,
                record_digest: parse_digest(record_digest)?,
                ledger_stream_id: parse_digest(ledger_stream_id)?,
                ledger_event_sequence: sequence.parse().map_err(|_| corrupt_error())?,
                ledger_event_digest: parse_digest(ledger_event_digest)?,
                recorded_at,
            });
        }
        let evidence_digest = replay_digest(task_ref, &records)?;
        Ok(TaskReplay {
            task_ref: task_ref.clone(),
            records,
            evidence_digest,
        })
    }

    /// Reconstructs and rehashes the immutable owner budget retained at promotion.
    ///
    /// # Errors
    ///
    /// Missing, malformed, overflowed, or digest-substituted budget rows fail closed.
    pub fn load_worker_budget(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<WorkerBudget, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let row = self
            .client
            .query_opt(
                "SELECT global_active_limit, per_task_active_limit, repair_retry_limit, \
                        max_duration_seconds, max_total_tokens, max_model_calls, \
                        external_cost_status, external_cost_limit_micros, deadline_at, \
                        budget_pointer, pg_catalog.encode(budget_digest,'hex') \
                   FROM foreman_execution.read_worker_budget_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?
            .ok_or_else(corrupt_error)?;
        let global_active: i16 = row.get(0);
        let per_task_active: i16 = row.get(1);
        let retry_limit: i16 = row.get(2);
        let duration: i64 = row.get(3);
        let tokens: i64 = row.get(4);
        let model_calls: i64 = row.get(5);
        let cost_status: String = row.get(6);
        let cost_limit: Option<i64> = row.get(7);
        let deadline: String = row.get(8);
        let budget_pointer: String = row.get(9);
        let stored_digest: String = row.get(10);
        let external_cost = match (cost_status.as_str(), cost_limit) {
            ("UNAVAILABLE", None) => ExternalCostBudget::Unavailable,
            ("LIMIT_MICROS", Some(value)) => {
                ExternalCostBudget::LimitMicros(u64::try_from(value).map_err(|_| corrupt_error())?)
            }
            _ => return Err(corrupt_error()),
        };
        let budget = WorkerBudget::new(
            u8::try_from(global_active).map_err(|_| corrupt_error())?,
            u8::try_from(per_task_active).map_err(|_| corrupt_error())?,
            u8::try_from(retry_limit).map_err(|_| corrupt_error())?,
            u64::try_from(duration).map_err(|_| corrupt_error())?,
            u64::try_from(tokens).map_err(|_| corrupt_error())?,
            u32::try_from(model_calls).map_err(|_| corrupt_error())?,
            external_cost,
            deadline,
        )
        .map_err(|_| corrupt_error())?;
        if budget.digest() != budget_pointer
            || budget_pointer.strip_prefix("budget:sha256:") != Some(stored_digest.as_str())
        {
            return Err(corrupt_error());
        }
        Ok(budget)
    }

    /// Loads and owner-reverifies the one bounded staged artifact for a task.
    ///
    /// The returned entry binds exact evidence bytes to the pre-append Ledger
    /// head, request/event digests, correlation, and occurred-at value needed
    /// for deterministic recovery before a full reference projection.
    ///
    /// # Errors
    ///
    /// Malformed evidence, head/link substitution, more than one row, or a
    /// database failure fails closed.
    #[allow(clippy::too_many_lines)]
    pub fn load_staged_artifact_reference(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<Option<StagedArtifactReference>, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let row = self
            .client
            .query_opt(
                "SELECT project_id, attempt_number, evidence_kind, media_type, \
                        payload_schema, producer_id, producer_version, \
                        pg_catalog.encode(producer_digest,'hex'), created_at, \
                        evidence_bytes, pg_catalog.encode(content_digest,'hex'), \
                        pg_catalog.encode(descriptor_digest,'hex'), stream_project_id, \
                        project_snapshot_id, task_id, task_revision, \
                        pg_catalog.encode(task_spec_digest,'hex'), accounting_currency, \
                        pg_catalog.encode(ledger_stream_id,'hex'), before_sequence, \
                        pg_catalog.encode(before_last_event_digest,'hex'), \
                        before_resource_revision, \
                        pg_catalog.encode(before_resource_projection_digest,'hex'), \
                        pg_catalog.encode(before_head_digest,'hex'), ledger_event_sequence, \
                        pg_catalog.encode(ledger_event_digest,'hex'), ledger_command_id, \
                        pg_catalog.encode(ledger_request_digest,'hex'), \
                        pg_catalog.encode(ledger_payload_digest,'hex'), correlation_id, \
                        command_occurred_at, staged_at \
                   FROM foreman_execution.read_staged_artifact_reference_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?;
        let Some(row) = row else {
            return Ok(None);
        };

        let evidence_project =
            ProjectId::new(row.try_get::<_, String>(0).map_err(|_| corrupt_error())?)
                .map_err(|_| corrupt_error())?;
        let attempt = u8::try_from(row.try_get::<_, i16>(1).map_err(|_| corrupt_error())?)
            .map_err(|_| corrupt_error())?;
        let kind = ManagedEvidenceKind::parse(
            row.try_get::<_, String>(2)
                .map_err(|_| corrupt_error())?
                .as_str(),
        )
        .map_err(|_| corrupt_error())?;
        let evidence_input = ManagedEvidenceInput::new(
            evidence_project.clone(),
            task_ref.clone(),
            attempt,
            kind,
            row.try_get::<_, String>(3).map_err(|_| corrupt_error())?,
            row.try_get::<_, String>(4).map_err(|_| corrupt_error())?,
            row.try_get::<_, String>(5).map_err(|_| corrupt_error())?,
            row.try_get::<_, String>(6).map_err(|_| corrupt_error())?,
            parse_digest(row.try_get::<_, String>(7).map_err(|_| corrupt_error())?)?,
            row.try_get::<_, String>(8).map_err(|_| corrupt_error())?,
            row.try_get::<_, Vec<u8>>(9).map_err(|_| corrupt_error())?,
        )
        .map_err(|_| corrupt_error())?;
        let evidence = verify_untrusted_managed_evidence(&UntrustedManagedEvidence::new(
            MANAGED_EVIDENCE_RECORD_SCHEMA,
            evidence_input,
            parse_digest(row.try_get::<_, String>(10).map_err(|_| corrupt_error())?)?,
            parse_digest(row.try_get::<_, String>(11).map_err(|_| corrupt_error())?)?,
        ))
        .map_err(|_| corrupt_error())?;

        let stream_project =
            ProjectId::new(row.try_get::<_, String>(12).map_err(|_| corrupt_error())?)
                .map_err(|_| corrupt_error())?;
        let identity = TaskLedgerStreamIdentity::new(
            stream_project.clone(),
            ProjectSnapshotId::new(row.try_get::<_, String>(13).map_err(|_| corrupt_error())?)
                .map_err(|_| corrupt_error())?,
            TaskId::new(row.try_get::<_, String>(14).map_err(|_| corrupt_error())?)
                .map_err(|_| corrupt_error())?,
            row.try_get::<_, String>(15).map_err(|_| corrupt_error())?,
            parse_digest(row.try_get::<_, String>(16).map_err(|_| corrupt_error())?)?,
            row.try_get::<_, String>(17).map_err(|_| corrupt_error())?,
        )
        .map_err(|_| corrupt_error())?;
        let stream_id = parse_digest(row.try_get::<_, String>(18).map_err(|_| corrupt_error())?)?;
        let before_sequence = row
            .try_get::<_, String>(19)
            .map_err(|_| corrupt_error())?
            .parse::<u64>()
            .map_err(|_| corrupt_error())?;
        let expected_head = TaskLedgerStreamHead::new(
            CONTRACT_VERSION,
            TASK_LEDGER_PRODUCER_ID,
            TASK_LEDGER_PRODUCER_VERSION,
            RuntimeKind::Live,
            identity,
            stream_id.clone(),
            before_sequence,
            parse_digest(row.try_get::<_, String>(20).map_err(|_| corrupt_error())?)?,
            row.try_get::<_, String>(21)
                .map_err(|_| corrupt_error())?
                .parse::<u64>()
                .map_err(|_| corrupt_error())?,
            parse_digest(row.try_get::<_, String>(22).map_err(|_| corrupt_error())?)?,
            parse_digest(row.try_get::<_, String>(23).map_err(|_| corrupt_error())?)?,
        )
        .map_err(|_| corrupt_error())?;
        let link = TaskRuntimeEventLink::new(
            expected_head,
            stream_id,
            row.try_get::<_, String>(24)
                .map_err(|_| corrupt_error())?
                .parse::<u64>()
                .map_err(|_| corrupt_error())?,
            parse_digest(row.try_get::<_, String>(25).map_err(|_| corrupt_error())?)?,
            CommandId::new(row.try_get::<_, String>(26).map_err(|_| corrupt_error())?)
                .map_err(|_| corrupt_error())?,
            parse_digest(row.try_get::<_, String>(27).map_err(|_| corrupt_error())?)?,
            parse_digest(row.try_get::<_, String>(28).map_err(|_| corrupt_error())?)?,
        );
        let correlation_id =
            CorrelationId::new(row.try_get::<_, String>(29).map_err(|_| corrupt_error())?)
                .map_err(|_| corrupt_error())?;
        let command_occurred_at = row.try_get::<_, String>(30).map_err(|_| corrupt_error())?;
        TaskRuntimeAppendMetadata::new(
            link.command_id().clone(),
            correlation_id.clone(),
            command_occurred_at.clone(),
        )
        .map_err(|_| corrupt_error())?;
        let staged_at = row.try_get::<_, String>(31).map_err(|_| corrupt_error())?;
        if evidence_project != stream_project
            || link.expected_head().stream_id() != link.stream_id()
            || link.event_sequence() != before_sequence.saturating_add(1)
            || link.payload_digest() != evidence.descriptor_digest()
            || !(20..=40).contains(&staged_at.len())
            || !staged_at.ends_with('Z')
        {
            return Err(corrupt_error());
        }
        Ok(Some(StagedArtifactReference {
            evidence,
            link,
            correlation_id,
            command_occurred_at,
            staged_at,
        }))
    }

    /// Loads exact bounded evidence bytes and re-verifies them through Artifact Store.
    ///
    /// # Errors
    ///
    /// Missing metadata, forbidden/tampered bytes, digest substitution, overflow,
    /// or database failure fails closed.
    pub fn load_managed_evidence(
        &mut self,
        task_ref: &ContentDigest,
        attempt: u8,
    ) -> Result<Vec<VerifiedManagedEvidence>, AdapterError> {
        if !(1..=3).contains(&attempt) {
            return Err(input_error());
        }
        let task_ref_bytes = digest_bytes(task_ref)?;
        let attempt_sql = i16::from(attempt);
        let rows = self
            .client
            .query(
                "SELECT project_id, evidence_kind, media_type, payload_schema, \
                        producer_id, producer_version, \
                        pg_catalog.encode(producer_digest,'hex'), created_at, \
                        evidence_bytes, pg_catalog.encode(content_digest,'hex'), \
                        pg_catalog.encode(descriptor_digest,'hex') \
                   FROM foreman_execution.read_managed_evidence_v1($1,$2)",
                &[&task_ref_bytes, &attempt_sql],
            )
            .map_err(|_| database_error())?;
        let mut evidence = Vec::with_capacity(rows.len());
        for row in rows {
            let project_id =
                ProjectId::new(row.get::<_, String>(0)).map_err(|_| corrupt_error())?;
            let kind = ManagedEvidenceKind::parse(row.get::<_, String>(1).as_str())
                .map_err(|_| corrupt_error())?;
            let input = ManagedEvidenceInput::new(
                project_id,
                task_ref.clone(),
                attempt,
                kind,
                row.get::<_, String>(2),
                row.get::<_, String>(3),
                row.get::<_, String>(4),
                row.get::<_, String>(5),
                parse_digest(row.get::<_, String>(6))?,
                row.get::<_, String>(7),
                row.get::<_, Vec<u8>>(8),
            )
            .map_err(|_| corrupt_error())?;
            let untrusted = UntrustedManagedEvidence::new(
                MANAGED_EVIDENCE_RECORD_SCHEMA,
                input,
                parse_digest(row.get::<_, String>(9))?,
                parse_digest(row.get::<_, String>(10))?,
            );
            evidence
                .push(verify_untrusted_managed_evidence(&untrusted).map_err(|_| corrupt_error())?);
        }
        Ok(evidence)
    }

    /// Loads persistence-shaped Task Ledger child rows with exact pre-event heads.
    ///
    /// The returned values are deliberately untrusted. The Task Ledger owner must
    /// replay the successor stream and call its verification surface before use.
    ///
    /// # Errors
    ///
    /// Missing promotion, malformed types, event/head substitution, overflow, or
    /// database failure fails closed.
    #[allow(clippy::too_many_lines)]
    pub fn load_task_runtime_rows(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<PersistedTaskRuntimeRows, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let promotion = self
            .client
            .query_opt(
                "SELECT pg_catalog.encode(intake_stream_id,'hex'), \
                        pg_catalog.encode(intake_event_digest,'hex'), \
                        pg_catalog.encode(project_authority_receipt_digest,'hex'), \
                        pg_catalog.encode(successor_stream_id,'hex'), \
                        pg_catalog.encode(successor_task_created_event_digest,'hex'), \
                        pg_catalog.encode(task_spec_digest,'hex'), \
                        pg_catalog.encode(approval_subject_digest,'hex'), \
                        pg_catalog.encode(budget_digest,'hex'), \
                        pg_catalog.encode(verification_policy_digest,'hex'), \
                        pg_catalog.encode(binding_digest,'hex'), ledger_event_digest \
                   FROM foreman_execution.read_task_promotion_row_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?
            .ok_or_else(corrupt_error)?;
        let promotion_event: Vec<u8> = promotion.get(10);
        let binding = UntrustedTaskExecutionBinding::new(
            TASK_EXECUTION_BINDING_RECORD_SCHEMA,
            self.load_event_link(&promotion_event)?,
            task_ref.clone(),
            parse_digest(promotion.get(0))?,
            parse_digest(promotion.get(1))?,
            parse_digest(promotion.get(2))?,
            parse_digest(promotion.get(3))?,
            parse_digest(promotion.get(4))?,
            parse_digest(promotion.get(5))?,
            parse_digest(promotion.get(6))?,
            parse_digest(promotion.get(7))?,
            parse_digest(promotion.get(8))?,
            parse_digest(promotion.get(9))?,
        );

        let attempt_rows = self
            .client
            .query(
                "SELECT pg_catalog.encode(successor_stream_id,'hex'), \
                        pg_catalog.encode(task_spec_digest,'hex'), \
                        pg_catalog.encode(binding_digest,'hex'), \
                        pg_catalog.encode(budget_digest,'hex'), attempt_id, \
                        attempt_number, foreman_generation, model, reasoning, \
                        writer_fence, pg_catalog.encode(foreman_checkpoint_digest,'hex'), \
                        pg_catalog.encode(approval_receipt_digest,'hex'), \
                        pg_catalog.encode(packet_digest,'hex'), \
                        pg_catalog.encode(worktree_digest,'hex'), \
                        pg_catalog.encode(base_commit_digest,'hex'), \
                        model_reason, pg_catalog.encode(model_reason_digest,'hex'), claimed_at, \
                        pg_catalog.encode(payload_digest,'hex'), ledger_event_digest \
                   FROM foreman_execution.read_worker_attempt_rows_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?;
        let mut attempts = Vec::with_capacity(attempt_rows.len());
        for row in attempt_rows {
            let event_digest: Vec<u8> = row.get(19);
            let attempt_number: i16 = row.get(5);
            let foreman_generation: i64 = row.get(6);
            let writer_fence: i64 = row.get(9);
            attempts.push(UntrustedWorkerAttemptRow::new(
                WORKER_ATTEMPT_RECORD_SCHEMA,
                self.load_event_link(&event_digest)?,
                task_ref.clone(),
                parse_digest(row.get(0))?,
                parse_digest(row.get(1))?,
                parse_digest(row.get(2))?,
                parse_digest(row.get(3))?,
                AttemptId::new(row.get::<_, String>(4)).map_err(|_| corrupt_error())?,
                u64::try_from(attempt_number).map_err(|_| corrupt_error())?,
                u64::try_from(foreman_generation).map_err(|_| corrupt_error())?,
                WorkerModel::from_persisted(row.get::<_, String>(7).as_str())
                    .map_err(|_| corrupt_error())?,
                ReasoningEffort::from_persisted(row.get::<_, String>(8).as_str())
                    .map_err(|_| corrupt_error())?,
                ModelReason::from_persisted(row.get::<_, String>(15).as_str())
                    .map_err(|_| corrupt_error())?,
                u64::try_from(writer_fence).map_err(|_| corrupt_error())?,
                parse_digest(row.get(10))?,
                parse_digest(row.get(11))?,
                parse_digest(row.get(12))?,
                parse_digest(row.get(13))?,
                parse_digest(row.get(14))?,
                parse_digest(row.get(16))?,
                row.get::<_, String>(17),
                parse_digest(row.get(18))?,
            ));
        }

        let observation_rows = self
            .client
            .query(
                "SELECT pg_catalog.encode(successor_stream_id,'hex'), \
                        pg_catalog.encode(binding_digest,'hex'), attempt_id, \
                        attempt_number, observation_kind, thread_id, turn_id, \
                        app_server_generation, \
                        pg_catalog.encode(app_server_identity_digest,'hex'), observed_at, \
                        pg_catalog.encode(evidence_digest,'hex'), \
                        pg_catalog.encode(payload_digest,'hex'), ledger_event_digest \
                   FROM foreman_execution.read_worker_observation_rows_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?;
        let mut observations = Vec::with_capacity(observation_rows.len());
        for row in observation_rows {
            let event_digest: Vec<u8> = row.get(12);
            let attempt_number: i16 = row.get(3);
            let generation: i64 = row.get(7);
            observations.push(UntrustedWorkerObservationRow::new(
                WORKER_OBSERVATION_RECORD_SCHEMA,
                self.load_event_link(&event_digest)?,
                task_ref.clone(),
                parse_digest(row.get(0))?,
                parse_digest(row.get(1))?,
                AttemptId::new(row.get::<_, String>(2)).map_err(|_| corrupt_error())?,
                u64::try_from(attempt_number).map_err(|_| corrupt_error())?,
                WorkerObservationKind::parse(row.get::<_, String>(4).as_str())
                    .map_err(|_| corrupt_error())?,
                row.get::<_, String>(5),
                row.get::<_, Option<String>>(6),
                u64::try_from(generation).map_err(|_| corrupt_error())?,
                parse_digest(row.get(8))?,
                row.get::<_, String>(9),
                parse_digest(row.get(10))?,
                parse_digest(row.get(11))?,
            ));
        }

        let verification_rows = self
            .client
            .query(
                "SELECT pg_catalog.encode(successor_stream_id,'hex'), \
                        pg_catalog.encode(task_spec_digest,'hex'), \
                        pg_catalog.encode(binding_digest,'hex'), attempt_id, \
                        attempt_number, outcome, \
                        pg_catalog.encode(verification_profile_digest,'hex'), \
                        pg_catalog.encode(base_commit_digest,'hex'), \
                        pg_catalog.encode(result_commit_digest,'hex'), \
                        pg_catalog.encode(tree_digest,'hex'), \
                        pg_catalog.encode(diff_digest,'hex'), \
                        pg_catalog.encode(result_digest,'hex'), \
                        pg_catalog.encode(evidence_artifact_digest,'hex'), \
                        CASE WHEN review_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(review_digest,'hex') END, \
                        verified_at, pg_catalog.encode(payload_digest,'hex'), \
                        ledger_event_digest \
                   FROM foreman_execution.read_verification_rows_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?;
        let mut verifications = Vec::with_capacity(verification_rows.len());
        for row in verification_rows {
            let event_digest: Vec<u8> = row.get(16);
            let attempt_number: i16 = row.get(4);
            verifications.push(UntrustedTaskVerificationRow::new(
                TASK_VERIFICATION_RECORD_SCHEMA,
                self.load_event_link(&event_digest)?,
                task_ref.clone(),
                parse_digest(row.get(0))?,
                parse_digest(row.get(1))?,
                parse_digest(row.get(2))?,
                AttemptId::new(row.get::<_, String>(3)).map_err(|_| corrupt_error())?,
                u64::try_from(attempt_number).map_err(|_| corrupt_error())?,
                VerificationOutcome::parse(row.get::<_, String>(5).as_str())
                    .map_err(|_| corrupt_error())?,
                parse_digest(row.get(6))?,
                parse_digest(row.get(7))?,
                parse_digest(row.get(8))?,
                parse_digest(row.get(9))?,
                parse_digest(row.get(10))?,
                parse_digest(row.get(11))?,
                parse_digest(row.get(12))?,
                row.get::<_, Option<String>>(13)
                    .map(parse_digest)
                    .transpose()?,
                row.get::<_, String>(14),
                parse_digest(row.get(15))?,
            ));
        }
        Ok(PersistedTaskRuntimeRows {
            binding,
            attempts,
            observations,
            verifications,
        })
    }

    /// Loads and owner-reverifies every retained local execution authority.
    ///
    /// # Errors
    ///
    /// Malformed, cross-bound, receipt/source-inconsistent, expired-shape, or
    /// digest-substituted rows fail closed. Currentness remains caller-owned.
    pub fn load_execution_authorities(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<Vec<VerifiedExecutionAuthority>, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let rows = self
            .client
            .query(
                "SELECT pg_catalog.encode(successor_stream_id,'hex'), \
                        pg_catalog.encode(task_spec_digest,'hex'), \
                        pg_catalog.encode(approval_subject_digest,'hex'), \
                        pg_catalog.encode(budget_digest,'hex'), authority_source, \
                        capability, pg_catalog.encode(authority_evidence_digest,'hex'), \
                        CASE WHEN approval_receipt_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_receipt_digest,'hex') END, \
                        issued_at, expires_at, pg_catalog.encode(authority_digest,'hex'), \
                        CASE WHEN approval_owner_snapshot_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_owner_snapshot_digest,'hex') END, \
                        CASE WHEN approval_owner_snapshot_content_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_owner_snapshot_content_digest,'hex') END, \
                        approval_owner_snapshot_bytes, approval_command_high_water, \
                        CASE WHEN approval_command_tail_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_command_tail_digest,'hex') END, \
                        CASE WHEN approval_nonce_bindings_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_nonce_bindings_digest,'hex') END \
                   FROM foreman_execution.read_execution_authority_rows_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?;
        let mut authorities = Vec::with_capacity(rows.len());
        for row in rows {
            authorities.push(reverify_execution_authority_row(task_ref, &row)?);
        }
        Ok(authorities)
    }

    /// Loads one digest-addressed authority and re-verifies it with its owner.
    ///
    /// # Errors
    ///
    /// Missing, substituted, malformed, or owner-invalid evidence fails closed.
    /// Currentness remains caller-owned.
    pub fn load_execution_authority(
        &mut self,
        task_ref: &ContentDigest,
        authority_digest: &ContentDigest,
    ) -> Result<VerifiedExecutionAuthority, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let authority_digest_bytes = digest_bytes(authority_digest)?;
        let row = self
            .client
            .query_opt(
                "SELECT pg_catalog.encode(successor_stream_id,'hex'), \
                        pg_catalog.encode(task_spec_digest,'hex'), \
                        pg_catalog.encode(approval_subject_digest,'hex'), \
                        pg_catalog.encode(budget_digest,'hex'), authority_source, \
                        capability, pg_catalog.encode(authority_evidence_digest,'hex'), \
                        CASE WHEN approval_receipt_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_receipt_digest,'hex') END, \
                        issued_at, expires_at, pg_catalog.encode(authority_digest,'hex'), \
                        CASE WHEN approval_owner_snapshot_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_owner_snapshot_digest,'hex') END, \
                        CASE WHEN approval_owner_snapshot_content_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_owner_snapshot_content_digest,'hex') END, \
                        approval_owner_snapshot_bytes, approval_command_high_water, \
                        CASE WHEN approval_command_tail_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_command_tail_digest,'hex') END, \
                        CASE WHEN approval_nonce_bindings_digest IS NULL THEN NULL \
                             ELSE pg_catalog.encode(approval_nonce_bindings_digest,'hex') END \
                   FROM foreman_execution.read_execution_authority_v1($1,$2)",
                &[&task_ref_bytes, &authority_digest_bytes],
            )
            .map_err(|_| database_error())?
            .ok_or_else(corrupt_error)?;
        reverify_execution_authority_row(task_ref, &row)
    }

    /// Loads exact Task Ledger event links for every retained approval/artifact row.
    ///
    /// Reference digests are cross-checked against each event payload so callers
    /// can rejoin owner-verified bytes/authority without relying on row order.
    ///
    /// # Errors
    ///
    /// Unknown kinds, invalid attempts, missing links, or payload substitution
    /// fail closed.
    pub fn load_reference_links(
        &mut self,
        task_ref: &ContentDigest,
    ) -> Result<PersistedReferenceLinks, AdapterError> {
        let task_ref_bytes = digest_bytes(task_ref)?;
        let rows = self
            .client
            .query(
                "SELECT record_kind, attempt_number, \
                        pg_catalog.encode(reference_digest,'hex'), ledger_event_digest \
                   FROM foreman_execution.read_reference_event_rows_v1($1)",
                &[&task_ref_bytes],
            )
            .map_err(|_| database_error())?;
        let mut artifact_links = Vec::new();
        let mut approval_links = Vec::new();
        for row in rows {
            let record_kind: String = row.get(0);
            let attempt: Option<i16> = row.get(1);
            let reference_digest = parse_digest(row.get::<_, String>(2))?;
            let event_digest: Vec<u8> = row.get(3);
            let link = self.load_event_link(&event_digest)?;
            if link.payload_digest() != &reference_digest {
                return Err(corrupt_error());
            }
            match (record_kind.as_str(), attempt) {
                ("ARTIFACT_REFERENCE", Some(value)) => {
                    let attempt_number = u8::try_from(value).map_err(|_| corrupt_error())?;
                    if !(1..=3).contains(&attempt_number) {
                        return Err(corrupt_error());
                    }
                    artifact_links.push(PersistedArtifactReferenceLink {
                        attempt_number,
                        descriptor_digest: reference_digest,
                        link,
                    });
                }
                ("APPROVAL_EVIDENCE", None) => {
                    approval_links.push(PersistedApprovalReferenceLink {
                        authority_digest: reference_digest,
                        link,
                    });
                }
                _ => return Err(corrupt_error()),
            }
        }
        Ok(PersistedReferenceLinks {
            artifact_links,
            approval_links,
        })
    }

    fn load_event_link(
        &mut self,
        event_digest: &[u8],
    ) -> Result<TaskRuntimeEventLink, AdapterError> {
        let row = self
            .client
            .query_opt(
                "SELECT project_id, project_snapshot_id, task_id, task_revision, \
                        pg_catalog.encode(task_spec_digest,'hex'), accounting_currency, \
                        pg_catalog.encode(stream_id,'hex'), before_sequence, \
                        pg_catalog.encode(before_last_event_digest,'hex'), \
                        before_resource_revision, \
                        pg_catalog.encode(before_resource_projection_digest,'hex'), \
                        pg_catalog.encode(before_head_digest,'hex'), event_sequence, \
                        pg_catalog.encode(event_digest,'hex'), command_id, \
                        pg_catalog.encode(request_digest,'hex'), \
                        pg_catalog.encode(payload_digest,'hex') \
                   FROM foreman_execution.read_child_event_link_v1($1)",
                &[&event_digest],
            )
            .map_err(|_| database_error())?
            .ok_or_else(corrupt_error)?;
        let identity = TaskLedgerStreamIdentity::new(
            ProjectId::new(row.get::<_, String>(0)).map_err(|_| corrupt_error())?,
            ProjectSnapshotId::new(row.get::<_, String>(1)).map_err(|_| corrupt_error())?,
            TaskId::new(row.get::<_, String>(2)).map_err(|_| corrupt_error())?,
            row.get::<_, String>(3),
            parse_digest(row.get(4))?,
            row.get::<_, String>(5),
        )
        .map_err(|_| corrupt_error())?;
        let stream_id = parse_digest(row.get::<_, String>(6))?;
        let expected_head = TaskLedgerStreamHead::new(
            CONTRACT_VERSION,
            TASK_LEDGER_PRODUCER_ID,
            TASK_LEDGER_PRODUCER_VERSION,
            RuntimeKind::Live,
            identity,
            stream_id.clone(),
            row.get::<_, String>(7)
                .parse()
                .map_err(|_| corrupt_error())?,
            parse_digest(row.get(8))?,
            row.get::<_, String>(9)
                .parse()
                .map_err(|_| corrupt_error())?,
            parse_digest(row.get(10))?,
            parse_digest(row.get(11))?,
        )
        .map_err(|_| corrupt_error())?;
        Ok(TaskRuntimeEventLink::new(
            expected_head,
            stream_id,
            row.get::<_, String>(12)
                .parse()
                .map_err(|_| corrupt_error())?,
            parse_digest(row.get(13))?,
            CommandId::new(row.get::<_, String>(14)).map_err(|_| corrupt_error())?,
            parse_digest(row.get(15))?,
            parse_digest(row.get(16))?,
        ))
    }
}

struct ApprovalOwnerSnapshotSql {
    checkpoint_snapshot_digest: ContentDigest,
    snapshot_content_digest: ContentDigest,
    snapshot_bytes: Vec<u8>,
    command_high_water: u64,
    command_tail_digest: ContentDigest,
    nonce_bindings_digest: ContentDigest,
}

fn approval_receipt_for_authority<'a>(
    authority: &VerifiedExecutionAuthority,
    approval_verifier: &'a FakeApprovalVerifier,
    error: fn() -> AdapterError,
) -> Result<&'a ApprovalAuthorityReceipt, AdapterError> {
    let expected = authority.approval_receipt_digest().ok_or_else(error)?;
    let mut matches = approval_verifier
        .command_receipts()
        .iter()
        .filter_map(|receipt| receipt.authority_receipt.as_ref())
        .filter(|receipt| receipt.receipt_digest() == expected);
    let retained = matches.next().ok_or_else(error)?;
    if matches.next().is_some() {
        return Err(error());
    }
    Ok(retained)
}

#[allow(clippy::too_many_lines)]
fn execution_environment_from_row(
    task_ref: &ContentDigest,
    row: &Row,
) -> Result<PersistedExecutionEnvironment, AdapterError> {
    let attempt_number = u8::try_from(row.get::<_, i16>(0)).map_err(|_| corrupt_error())?;
    if !(1..=3).contains(&attempt_number) {
        return Err(corrupt_error());
    }
    let attempt_id = AttemptId::new(row.get::<_, String>(1)).map_err(|_| corrupt_error())?;
    let packet_digest = parse_digest(row.get(2))?;
    let descriptor = ExecutionEnvironmentDescriptor::from_json(row.get::<_, String>(3).as_str())
        .map_err(|_| corrupt_error())?;
    let persisted_execution_domain_digest = parse_digest(row.get(4))?;
    let persisted_environment_ref = ExecutionEnvironmentRef::parse(row.get(5))?;
    let recorded_at: String = row.get(6);
    if persisted_execution_domain_digest != *descriptor.execution_domain_digest()
        || persisted_environment_ref != *descriptor.environment_ref()
        || recorded_at.is_empty()
        || recorded_at.len() > 40
    {
        return Err(corrupt_error());
    }
    Ok(PersistedExecutionEnvironment {
        task_ref: task_ref.clone(),
        attempt_number,
        attempt_id,
        packet_digest,
        descriptor,
        recorded_at,
    })
}

fn approval_owner_snapshot_sql(
    authority: &VerifiedExecutionAuthority,
    approval_verifier: &FakeApprovalVerifier,
    error: fn() -> AdapterError,
) -> Result<ApprovalOwnerSnapshotSql, AdapterError> {
    if authority.source() != ExecutionAuthoritySource::VerifiedApproval {
        return Err(error());
    }
    let approval_receipt = approval_receipt_for_authority(authority, approval_verifier, error)?;
    let approval_id = approval_receipt.identity().approval_id();
    let binding_receipt = approval_verifier
        .execution_binding_receipt(approval_id)
        .ok_or_else(error)?;
    let current_head = approval_verifier
        .current_head_at(approval_id, authority.issued_at())
        .map_err(|_| error())?
        .ok_or_else(error)?;
    let context = VerifiedApprovalExecutionContext::new_with_binding_receipt(
        binding_receipt.clone(),
        approval_receipt.clone(),
        current_head,
    )
    .map_err(|_| error())?;
    reverify_verified_approval_execution_authority(authority, &context, authority.issued_at())
        .map_err(|_| error())?;

    let snapshot_bytes = approval_verifier
        .export_snapshot_bytes()
        .map_err(|_| error())?;
    let checkpoint = approval_verifier
        .current_checkpoint()
        .map_err(|_| error())?;
    let command_tail_digest = checkpoint
        .command_tail_digest()
        .cloned()
        .ok_or_else(error)?;
    let snapshot_content_digest = sha256_content_digest(snapshot_bytes.as_slice(), error)?;
    Ok(ApprovalOwnerSnapshotSql {
        checkpoint_snapshot_digest: checkpoint.snapshot_digest().clone(),
        snapshot_content_digest,
        snapshot_bytes,
        command_high_water: checkpoint.command_high_water(),
        command_tail_digest,
        nonce_bindings_digest: checkpoint.nonce_bindings_digest().clone(),
    })
}

fn reverify_execution_authority_row(
    task_ref: &ContentDigest,
    row: &Row,
) -> Result<VerifiedExecutionAuthority, AdapterError> {
    let input = ExecutionAuthorityInput::new(
        task_ref.clone(),
        parse_digest(row.get(0))?,
        parse_digest(row.get(1))?,
        parse_digest(row.get(2))?,
        parse_digest(row.get(3))?,
        ExecutionAuthoritySource::parse(row.get::<_, String>(4).as_str())
            .map_err(|_| corrupt_error())?,
        ExecutionCapability::parse(row.get::<_, String>(5).as_str())
            .map_err(|_| corrupt_error())?,
        parse_digest(row.get(6))?,
        row.get::<_, Option<String>>(7)
            .map(parse_digest)
            .transpose()?,
        row.get::<_, String>(8),
        row.get::<_, String>(9),
    )
    .map_err(|_| corrupt_error())?;
    let untrusted = UntrustedExecutionAuthority::new(
        EXECUTION_AUTHORITY_SCHEMA,
        input,
        parse_digest(row.get(10))?,
    );
    let authority =
        verify_untrusted_execution_authority(&untrusted).map_err(|_| corrupt_error())?;
    reverify_approval_owner_row(&authority, row)?;
    Ok(authority)
}

fn reverify_approval_owner_row(
    authority: &VerifiedExecutionAuthority,
    row: &Row,
) -> Result<(), AdapterError> {
    let snapshot_digest: Option<String> = row.try_get(11).map_err(|_| corrupt_error())?;
    let snapshot_content_digest: Option<String> = row.try_get(12).map_err(|_| corrupt_error())?;
    let snapshot_bytes: Option<Vec<u8>> = row.try_get(13).map_err(|_| corrupt_error())?;
    let command_high_water: Option<i64> = row.try_get(14).map_err(|_| corrupt_error())?;
    let command_tail_digest: Option<String> = row.try_get(15).map_err(|_| corrupt_error())?;
    let nonce_bindings_digest: Option<String> = row.try_get(16).map_err(|_| corrupt_error())?;
    if authority.source() == ExecutionAuthoritySource::ClosedPolicyNoApprovalRequired {
        if snapshot_digest.is_some()
            || snapshot_content_digest.is_some()
            || snapshot_bytes.is_some()
            || command_high_water.is_some()
            || command_tail_digest.is_some()
            || nonce_bindings_digest.is_some()
        {
            return Err(corrupt_error());
        }
        return Ok(());
    }

    let snapshot_digest = parse_digest(snapshot_digest.ok_or_else(corrupt_error)?)?;
    let snapshot_content_digest = parse_digest(snapshot_content_digest.ok_or_else(corrupt_error)?)?;
    let snapshot_bytes = snapshot_bytes.ok_or_else(corrupt_error)?;
    let command_high_water = u64::try_from(command_high_water.ok_or_else(corrupt_error)?)
        .map_err(|_| corrupt_error())?;
    let command_tail_digest = parse_digest(command_tail_digest.ok_or_else(corrupt_error)?)?;
    let nonce_bindings_digest = parse_digest(nonce_bindings_digest.ok_or_else(corrupt_error)?)?;
    let actual_content_digest = sha256_content_digest(snapshot_bytes.as_slice(), corrupt_error)?;
    if actual_content_digest != snapshot_content_digest {
        return Err(corrupt_error());
    }
    let checkpoint = ApprovalVerifierCheckpoint::new(
        command_high_water,
        Some(command_tail_digest),
        nonce_bindings_digest,
        snapshot_digest,
    )
    .map_err(|_| corrupt_error())?;
    let mut approval_verifier = FakeApprovalVerifier::new();
    approval_verifier
        .restore_snapshot_bytes(snapshot_bytes.as_slice(), &checkpoint)
        .map_err(|_| corrupt_error())?;
    let normalized = approval_owner_snapshot_sql(authority, &approval_verifier, corrupt_error)?;
    if normalized.snapshot_bytes != snapshot_bytes
        || normalized.snapshot_content_digest != snapshot_content_digest
        || normalized.command_high_water != command_high_water
        || normalized.command_tail_digest
            != *checkpoint.command_tail_digest().ok_or_else(corrupt_error)?
        || normalized.nonce_bindings_digest != *checkpoint.nonce_bindings_digest()
        || normalized.checkpoint_snapshot_digest != *checkpoint.snapshot_digest()
    {
        return Err(corrupt_error());
    }
    Ok(())
}

fn sha256_content_digest(
    bytes: &[u8],
    error: fn() -> AdapterError,
) -> Result<ContentDigest, AdapterError> {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").map_err(|_| error())?;
    }
    ContentDigest::from_sha256(value).map_err(|_| error())
}

struct EventSql {
    stream_id: Vec<u8>,
    before_sequence: String,
    before_last_event_digest: Vec<u8>,
    before_resource_revision: String,
    before_resource_projection_digest: Vec<u8>,
    before_head_digest: Vec<u8>,
    sequence: String,
    event_digest: Vec<u8>,
    command_id: String,
    request_digest: Vec<u8>,
    payload_digest: Vec<u8>,
}

fn event_sql(link: &TaskRuntimeEventLink) -> Result<EventSql, AdapterError> {
    Ok(EventSql {
        stream_id: digest_bytes(link.stream_id())?,
        before_sequence: link.expected_head().sequence().to_string(),
        before_last_event_digest: digest_bytes(link.expected_head().last_event_digest())?,
        before_resource_revision: link.expected_head().resource_revision().to_string(),
        before_resource_projection_digest: digest_bytes(
            link.expected_head().resource_projection_digest(),
        )?,
        before_head_digest: digest_bytes(link.expected_head().head_digest())?,
        sequence: link.event_sequence().to_string(),
        event_digest: digest_bytes(link.event_digest())?,
        command_id: link.command_id().as_str().to_owned(),
        request_digest: digest_bytes(link.request_digest())?,
        payload_digest: digest_bytes(link.payload_digest())?,
    })
}

fn parse_append(value: &str) -> Result<AppendDisposition, AdapterError> {
    match value {
        "INSERTED" => Ok(AppendDisposition::Inserted),
        "EXACT_REPLAY" => Ok(AppendDisposition::ExactReplay),
        _ => Err(corrupt_error()),
    }
}

fn digest_bytes(value: &ContentDigest) -> Result<Vec<u8>, AdapterError> {
    let bytes = value.as_str().as_bytes();
    if bytes.len() != 64 {
        return Err(input_error());
    }
    let mut output = Vec::with_capacity(32);
    for pair in bytes.chunks_exact(2) {
        let high = hex_nibble(pair[0]).ok_or_else(input_error)?;
        let low = hex_nibble(pair[1]).ok_or_else(input_error)?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn parse_digest(value: String) -> Result<ContentDigest, AdapterError> {
    ContentDigest::from_sha256(value).map_err(|_| corrupt_error())
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn to_i64(value: u64) -> Result<i64, AdapterError> {
    i64::try_from(value).map_err(|_| input_error())
}

fn is_zero(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}

fn bounded_identity_text(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value.trim() == value
        && !value.contains("://")
        && value
            .chars()
            .all(|character| !character.is_control() && character != '\0')
}

#[derive(Clone, Copy)]
enum ExecutionToolVersionKind {
    WslGateway,
    Codex,
    Node,
    Git,
    Systemd,
    Lsattr,
    Sudo,
    Npm,
    Cargo,
    Rustc,
    Rustdoc,
    Bubblewrap,
}

fn closed_tool_version(kind: ExecutionToolVersionKind, value: &str) -> bool {
    if !bounded_identity_text(value, 128) || !value.is_ascii() {
        return false;
    }
    match kind {
        ExecutionToolVersionKind::WslGateway => numeric_dotted_components(value, 3, 4),
        ExecutionToolVersionKind::Codex => value
            .strip_prefix("codex-cli ")
            .is_some_and(semantic_version),
        ExecutionToolVersionKind::Node => value.strip_prefix('v').is_some_and(semantic_version),
        ExecutionToolVersionKind::Git => value
            .strip_prefix("git version ")
            .is_some_and(|version| numeric_dotted_components(version, 3, 3)),
        ExecutionToolVersionKind::Systemd => {
            value.strip_prefix("systemd ").is_some_and(systemd_version)
        }
        ExecutionToolVersionKind::Lsattr => {
            value.strip_prefix("lsattr ").is_some_and(lsattr_version)
        }
        ExecutionToolVersionKind::Sudo => sudo_version(value),
        ExecutionToolVersionKind::Npm => semantic_version(value),
        ExecutionToolVersionKind::Cargo => {
            value.strip_prefix("cargo ").is_some_and(rust_tool_version)
        }
        ExecutionToolVersionKind::Rustc => {
            value.strip_prefix("rustc ").is_some_and(rust_tool_version)
        }
        ExecutionToolVersionKind::Rustdoc => value
            .strip_prefix("rustdoc ")
            .is_some_and(rust_tool_version),
        ExecutionToolVersionKind::Bubblewrap => value
            .strip_prefix("bubblewrap ")
            .is_some_and(semantic_version),
    }
}

fn execution_environment_string_leaves_are_secret_free(root: &Value) -> bool {
    let mut pending = vec![(root, 0usize)];
    let mut nodes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes = nodes.saturating_add(1);
        if nodes > MAX_EXECUTION_ENVIRONMENT_STRING_SCAN_NODES {
            return false;
        }
        match value {
            Value::String(value) => {
                if value.len() > MAX_EXECUTION_ENVIRONMENT_STRING_LEAF_BYTES
                    || execution_environment_string_contains_recognized_secret(value)
                {
                    return false;
                }
            }
            Value::Array(values) => {
                if depth >= MAX_EXECUTION_ENVIRONMENT_STRING_SCAN_DEPTH
                    || nodes
                        .saturating_add(pending.len())
                        .saturating_add(values.len())
                        > MAX_EXECUTION_ENVIRONMENT_STRING_SCAN_NODES
                {
                    return false;
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if depth >= MAX_EXECUTION_ENVIRONMENT_STRING_SCAN_DEPTH
                    || nodes
                        .saturating_add(pending.len())
                        .saturating_add(values.len())
                        > MAX_EXECUTION_ENVIRONMENT_STRING_SCAN_NODES
                {
                    return false;
                }
                pending.extend(values.values().map(|value| (value, depth + 1)));
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    true
}

fn execution_environment_string_contains_recognized_secret(value: &str) -> bool {
    if task_ingress_text_contains_recognized_secret(value) {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    [
        "access token",
        "refresh token",
        "id token",
        "session token",
        "api key",
        "client secret",
    ]
    .into_iter()
    .any(|key| contains_execution_environment_sensitive_assignment(&lower, key))
}

fn contains_execution_environment_sensitive_assignment(value: &str, key: &str) -> bool {
    value.match_indices(key).any(|(start, matched)| {
        let boundary_before = value[..start].chars().next_back().is_none_or(|character| {
            !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-')
        });
        if !boundary_before {
            return false;
        }
        let mut suffix = value[start + matched.len()..].chars().peekable();
        if suffix.peek().is_some_and(|character| {
            character.is_ascii_alphanumeric() || matches!(*character, '_' | '-')
        }) {
            return false;
        }
        while suffix
            .peek()
            .is_some_and(|character| character.is_whitespace())
        {
            suffix.next();
        }
        if suffix
            .peek()
            .is_some_and(|character| matches!(*character, '"' | '\''))
        {
            suffix.next();
        }
        while suffix
            .peek()
            .is_some_and(|character| character.is_whitespace())
        {
            suffix.next();
        }
        matches!(suffix.next(), Some(':' | '='))
    })
}

fn numeric_dotted_components(value: &str, minimum: usize, maximum: usize) -> bool {
    let components = value.split('.').collect::<Vec<_>>();
    (minimum..=maximum).contains(&components.len())
        && components.iter().all(|component| {
            !component.is_empty()
                && component.len() <= 6
                && component.bytes().all(|byte| byte.is_ascii_digit())
        })
}

fn semantic_version(value: &str) -> bool {
    numeric_dotted_components(value, 3, 3)
}

fn systemd_version(value: &str) -> bool {
    if value.bytes().all(|byte| byte.is_ascii_digit()) && (2..=4).contains(&value.len()) {
        return true;
    }
    let Some((number, package)) = value.split_once(" (") else {
        return false;
    };
    number.bytes().all(|byte| byte.is_ascii_digit())
        && (2..=4).contains(&number.len())
        && package.ends_with(')')
        && !package[..package.len() - 1].is_empty()
        && package[..package.len() - 1].bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b':' | b'~' | b'_' | b'-')
        })
}

fn lsattr_version(value: &str) -> bool {
    let Some((version, detail)) = value.split_once(" (") else {
        return false;
    };
    if !semantic_version(version) {
        return false;
    }
    let Some(date) = detail.strip_suffix(')') else {
        return false;
    };
    let parts = date.split('-').collect::<Vec<_>>();
    parts.len() == 3
        && (1..=2).contains(&parts[0].len())
        && parts[0].bytes().all(|byte| byte.is_ascii_digit())
        && parts[1].len() == 3
        && parts[1].bytes().all(|byte| byte.is_ascii_alphabetic())
        && parts[2].len() == 4
        && parts[2].bytes().all(|byte| byte.is_ascii_digit())
}

fn rust_tool_version(value: &str) -> bool {
    let Some((version, detail)) = value.split_once(" (") else {
        return false;
    };
    if !semantic_version(version) {
        return false;
    }
    let Some(detail) = detail.strip_suffix(')') else {
        return false;
    };
    let Some((commit, date)) = detail.split_once(' ') else {
        return false;
    };
    (7..=40).contains(&commit.len())
        && commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        && iso_date(date)
}

fn sudo_version(value: &str) -> bool {
    if let Some(version) = value.strip_prefix("sudo-rs ") {
        let Some((semantic, suffix)) = version.split_once('-') else {
            return false;
        };
        return semantic_version(semantic)
            && !suffix.is_empty()
            && suffix.len() <= 64
            && suffix.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'~' | b'_' | b'-')
            });
    }
    let Some(version) = value.strip_prefix("Sudo version ") else {
        return false;
    };
    let base_len = version
        .bytes()
        .take_while(|byte| byte.is_ascii_digit() || *byte == b'.')
        .count();
    let (semantic, suffix) = version.split_at(base_len);
    semantic_version(semantic)
        && (suffix.is_empty()
            || suffix.strip_prefix('p').is_some_and(|patch| {
                !patch.is_empty()
                    && patch.len() <= 64
                    && patch.bytes().all(|byte| byte.is_ascii_digit())
            }))
}

fn iso_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn safe_distribution(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        })
}

fn canonical_linux_path(value: &str) -> bool {
    if value.len() < 2
        || value.len() > 1_024
        || !value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('\\')
        || value.contains("://")
        || value.starts_with("/mnt/")
        || value.chars().any(char::is_control)
    {
        return false;
    }
    value
        .split('/')
        .skip(1)
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn canonical_linux_home_path(value: &str) -> bool {
    value.starts_with("/home/") && canonical_linux_path(value)
}

fn sandbox_policy_uri_safe_linux_path(value: &str) -> bool {
    canonical_linux_path(value)
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'~' | b'-')
        })
}

#[allow(clippy::too_many_lines)]
fn wsl2_sandbox_policy_template(
    linux_repository_path: &str,
    linux_codex_home_path: &str,
    user_runtime_dir: &str,
    task_root: &str,
    toolchain: &Map<String, Value>,
) -> Result<Value, AdapterError> {
    let home_dir = json_string(toolchain, "home_dir")?;
    let temp_dir = json_string(toolchain, "temp_dir")?;
    let npm_cache = json_string(toolchain, "npm_cache")?;
    let cargo_home = json_string(toolchain, "cargo_home")?;
    let cargo_target_dir = json_string(toolchain, "cargo_target_dir")?;
    let dynamic_paths = [
        linux_repository_path,
        linux_codex_home_path,
        user_runtime_dir,
        task_root,
        home_dir,
        temp_dir,
        npm_cache,
        cargo_home,
        cargo_target_dir,
    ];
    if dynamic_paths
        .iter()
        .any(|path| !sandbox_policy_uri_safe_linux_path(path))
    {
        return Err(input_error());
    }
    let task_root_suffix = task_root.strip_prefix("/home/").ok_or_else(input_error)?;
    let home_user = task_root_suffix.split('/').next().ok_or_else(input_error)?;
    if home_user.is_empty() {
        return Err(input_error());
    }
    let linux_home = format!("/home/{home_user}");
    let mut deny_paths = Vec::new();
    for candidate in [
        linux_codex_home_path.to_owned(),
        format!("{linux_home}/.codex"),
        "/mnt".to_owned(),
        user_runtime_dir.to_owned(),
    ] {
        if !deny_paths.contains(&candidate) {
            deny_paths.push(candidate);
        }
    }
    let strings = |values: &[&str]| {
        Value::Array(
            values
                .iter()
                .map(|value| Value::String((*value).to_owned()))
                .collect(),
        )
    };
    let base_entries = Value::Array(vec![
        json_object([
            (
                "path",
                json_object([
                    ("type", Value::String("special".to_owned())),
                    (
                        "value",
                        json_object([("kind", Value::String("minimal".to_owned()))]),
                    ),
                ]),
            ),
            ("access", Value::String("read".to_owned())),
        ]),
        json_object([
            (
                "path",
                json_object([
                    ("type", Value::String("path".to_owned())),
                    ("path", Value::String(task_root.to_owned())),
                ]),
            ),
            ("access", Value::String("read".to_owned())),
        ]),
    ]);
    let git_writes = json_object([
        (
            "bootstrap",
            strings(&["$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR"]),
        ),
        (
            "guarded_object_write",
            strings(&[
                "$GIT_CONTROL_HOME",
                "$GIT_CONTROL_TMPDIR",
                "$GIT_COMMON_DIR/objects",
            ]),
        ),
        (
            "guarded_index_write",
            strings(&[
                "$GIT_CONTROL_HOME",
                "$GIT_CONTROL_TMPDIR",
                "$GIT_CONTROL_ROOT/candidate-index",
            ]),
        ),
    ]);
    let role_writes = json_object([
        (
            "PREFLIGHT",
            strings(&[
                linux_repository_path,
                home_dir,
                temp_dir,
                npm_cache,
                cargo_home,
                cargo_target_dir,
            ]),
        ),
        ("NODE", strings(&[home_dir, temp_dir, npm_cache])),
        (
            "CARGO",
            strings(&[home_dir, temp_dir, cargo_home, cargo_target_dir]),
        ),
        ("GIT", git_writes),
    ]);
    let deny_entries = Value::Array(
        deny_paths
            .into_iter()
            .map(|path| {
                json_object([
                    ("path", Value::String(path)),
                    ("missing_path_behavior", Value::String("skip".to_owned())),
                ])
            })
            .collect(),
    );
    Ok(json_object([
        (
            "schema",
            Value::String("lattice.wsl2-sandbox-template/1.0".to_owned()),
        ),
        (
            "permission_profile_type",
            Value::String("managed".to_owned()),
        ),
        ("filesystem_type", Value::String("restricted".to_owned())),
        ("network", Value::String("restricted".to_owned())),
        ("base_entries", base_entries),
        ("role_writes", role_writes),
        ("deny_entries", deny_entries),
        ("codex_linux_sandbox_exe", Value::Null),
        (
            "sandbox_cwd",
            Value::String(format!("file://{linux_repository_path}")),
        ),
        ("use_legacy_landlock", Value::Bool(false)),
    ]))
}

fn linux_descendant(root: &str, candidate: &str) -> bool {
    canonical_linux_home_path(root)
        && canonical_linux_home_path(candidate)
        && candidate.starts_with(&format!("{root}/"))
}

fn linux_direct_child(root: &str, candidate: &str) -> bool {
    linux_descendant(root, candidate)
        && candidate
            .strip_prefix(&format!("{root}/"))
            .is_some_and(|suffix| !suffix.is_empty() && !suffix.contains('/'))
}

fn canonical_windows_wsl_gateway_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    if value.len() < 8
        || value.len() > 1_024
        || bytes.get(1) != Some(&b':')
        || bytes.get(2) != Some(&b'\\')
        || !bytes[0].is_ascii_alphabetic()
        || value.contains('/')
        || value.contains("\\\\")
        || value.chars().any(char::is_control)
        || !value.to_ascii_lowercase().ends_with("\\wsl.exe")
    {
        return false;
    }
    value
        .split('\\')
        .skip(1)
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn exact_json_object<'a>(
    value: &'a Value,
    expected_keys: &[&str],
) -> Result<&'a Map<String, Value>, AdapterError> {
    let object = value.as_object().ok_or_else(input_error)?;
    if object.len() != expected_keys.len()
        || expected_keys.iter().any(|key| !object.contains_key(*key))
    {
        return Err(input_error());
    }
    Ok(object)
}

fn json_string<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, AdapterError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(input_error)
}

fn json_u64(object: &Map<String, Value>, field: &str) -> Result<u64, AdapterError> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(input_error)
}

fn canonical_nonzero_u64_text(value: &str) -> bool {
    !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|number| number > 0)
}

fn json_tree_manifest<'a>(
    trees: &'a Map<String, Value>,
    field: &str,
) -> Result<(&'a str, ContentDigest), AdapterError> {
    let object = exact_json_object(
        trees.get(field).ok_or_else(input_error)?,
        &["root", "manifest_digest"],
    )?;
    let root = json_string(object, "root")?;
    let digest = parse_typed_digest(
        json_string(object, "manifest_digest")?,
        "immutable-tree-manifest",
    )?;
    Ok((root, digest))
}

fn json_object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonicalize_json(&object[key])))
                    .collect(),
            )
        }
        _ => value.clone(),
    }
}

fn canonical_json_value(value: &Value) -> Result<String, AdapterError> {
    serde_json::to_string(&canonicalize_json(value)).map_err(|_| input_error())
}

fn typed_json_identity(domain: &str, subject: &Value) -> Result<String, AdapterError> {
    let digest = sha256_content(canonical_json_value(subject)?.as_bytes())?;
    Ok(format!("{domain}:sha256:{}", digest.as_str()))
}

fn sha256_content(bytes: &[u8]) -> Result<ContentDigest, AdapterError> {
    sha256_content_digest(bytes, input_error)
}

fn parse_raw_digest(value: &str) -> Result<ContentDigest, AdapterError> {
    if !lower_hex(value, 64) {
        return Err(input_error());
    }
    let digest = ContentDigest::from_sha256(value.to_owned()).map_err(|_| input_error())?;
    if is_zero(&digest) {
        return Err(input_error());
    }
    Ok(digest)
}

fn validate_attempt_execution_environment_ref(value: &str) -> Result<(), AdapterError> {
    ExecutionEnvironmentRef::parse(value.to_owned())
        .map(|_| ())
        .map_err(|_| input_error())
}

fn parse_typed_digest(value: &str, domain: &str) -> Result<ContentDigest, AdapterError> {
    let Some(hex) = value.strip_prefix(&format!("{domain}:sha256:")) else {
        return Err(input_error());
    };
    parse_raw_digest(hex)
}

fn json_tool_identity(
    object: &Map<String, Value>,
    prefix: &str,
) -> Result<ExecutionToolIdentity, AdapterError> {
    ExecutionToolIdentity::new(
        json_string(object, &format!("{prefix}_path"))?,
        json_string(object, &format!("{prefix}_version"))?,
        parse_raw_digest(json_string(object, &format!("{prefix}_sha256"))?)?,
    )
}

fn json_file_identity(
    object: &Map<String, Value>,
    prefix: &str,
) -> Result<ExecutionFileIdentity, AdapterError> {
    ExecutionFileIdentity::new(
        json_string(object, &format!("{prefix}_path"))?,
        parse_raw_digest(json_string(object, &format!("{prefix}_sha256"))?)?,
    )
}

fn json_nested_tool_identity(
    object: &Map<String, Value>,
    field: &str,
) -> Result<ExecutionToolIdentity, AdapterError> {
    let tool = exact_json_object(
        object.get(field).ok_or_else(input_error)?,
        &["path", "version", "sha256"],
    )?;
    let identity = ExecutionToolIdentity::new(
        json_string(tool, "path")?,
        json_string(tool, "version")?,
        parse_raw_digest(json_string(tool, "sha256")?)?,
    )?;
    if !canonical_linux_path(identity.path()) {
        return Err(input_error());
    }
    Ok(identity)
}

fn safe_lower_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn node_version_at_least(value: &str, minimum: [u64; 3]) -> bool {
    let Some(version) = value.strip_prefix('v') else {
        return false;
    };
    let mut segments = version.split('.');
    let mut actual = [0_u64; 3];
    for slot in &mut actual {
        let Some(segment) = segments.next() else {
            return false;
        };
        let Ok(number) = segment.parse::<u64>() else {
            return false;
        };
        *slot = number;
    }
    segments.next().is_none() && actual >= minimum
}

fn numeric_dotted_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_user_runtime_dir(value: &str) -> bool {
    value
        .strip_prefix("/run/user/")
        .is_some_and(|uid| !uid.is_empty() && uid.bytes().all(|byte| byte.is_ascii_digit()))
}

fn valid_unit_prefix(value: &str) -> bool {
    value
        .strip_prefix("lattice-wsl2-")
        .is_some_and(|suffix| lower_hex(suffix, 16))
}

fn windows_wsl_path_to_linux(value: &str, distribution: &str) -> Option<String> {
    let prefix = format!(r"\\wsl.localhost\{distribution}\");
    if value.len() <= prefix.len()
        || !value[..prefix.len()].eq_ignore_ascii_case(&prefix)
        || value[prefix.len()..].contains('/')
    {
        return None;
    }
    let linux = format!("/{}", value[prefix.len()..].replace('\\', "/"));
    canonical_linux_home_path(&linux).then_some(linux)
}

#[allow(clippy::too_many_arguments)]
fn promotion_intent_digest(
    task_ref: &ContentDigest,
    project_id: &ProjectId,
    project_snapshot_id: &ProjectSnapshotId,
    project_authority_receipt_digest: &ContentDigest,
    successor_stream_id: &ContentDigest,
    task_spec_digest: &ContentDigest,
    approval_subject_digest: &ContentDigest,
    budget: &WorkerBudget,
    verification_policy_digest: &ContentDigest,
    source: &ManagedPromotionSource,
    source_clean: bool,
    issued_at: &str,
) -> Result<ContentDigest, AdapterError> {
    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_FOREMAN_PROMOTION_INTENT_V1\0");
    for value in [
        task_ref.as_str(),
        project_id.as_str(),
        project_snapshot_id.as_str(),
        project_authority_receipt_digest.as_str(),
        successor_stream_id.as_str(),
        task_spec_digest.as_str(),
        approval_subject_digest.as_str(),
        budget.digest(),
        verification_policy_digest.as_str(),
        source.base_ref(),
        source.base_commit(),
        if source_clean { "true" } else { "false" },
        issued_at,
    ] {
        update_framed(&mut hasher, value.as_bytes());
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        use fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| input_error())?;
    }
    ContentDigest::from_sha256(output).map_err(|_| input_error())
}

#[allow(clippy::too_many_arguments)]
fn preparation_observation_digest(
    task_ref: &ContentDigest,
    project_id: &ProjectId,
    project_snapshot_id: &ProjectSnapshotId,
    project_authority_receipt_digest: &ContentDigest,
    kind: ManagedPreparationObservationKind,
    subject_digest: &ContentDigest,
    observed_at: &str,
) -> Result<ContentDigest, AdapterError> {
    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_FOREMAN_PREPARATION_OBSERVATION_V1\0");
    for value in [
        task_ref.as_str(),
        project_id.as_str(),
        project_snapshot_id.as_str(),
        project_authority_receipt_digest.as_str(),
        kind.as_str(),
        subject_digest.as_str(),
        observed_at,
    ] {
        update_framed(&mut hasher, value.as_bytes());
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        use fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| input_error())?;
    }
    ContentDigest::from_sha256(output).map_err(|_| input_error())
}

fn replay_digest(
    task_ref: &ContentDigest,
    records: &[ReplayRecord],
) -> Result<ContentDigest, AdapterError> {
    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_FOREMAN_TASK_REPLAY_V1\0");
    update_framed(&mut hasher, task_ref.as_str().as_bytes());
    update_framed(&mut hasher, records.len().to_string().as_bytes());
    for record in records {
        update_framed(&mut hasher, record.record_kind.as_bytes());
        update_framed(&mut hasher, record.record_state.as_str().as_bytes());
        update_framed(
            &mut hasher,
            record
                .attempt_number
                .map_or_else(|| "-".to_owned(), |value| value.to_string())
                .as_bytes(),
        );
        update_framed(&mut hasher, record.record_ordinal.to_string().as_bytes());
        update_framed(&mut hasher, record.record_digest.as_str().as_bytes());
        update_framed(&mut hasher, record.ledger_stream_id.as_str().as_bytes());
        update_framed(
            &mut hasher,
            record.ledger_event_sequence.to_string().as_bytes(),
        );
        update_framed(&mut hasher, record.ledger_event_digest.as_str().as_bytes());
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        use fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| corrupt_error())?;
    }
    ContentDigest::from_sha256(output).map_err(|_| corrupt_error())
}

#[allow(clippy::too_many_arguments)]
fn provider_dispatch_digest(
    kind: ProviderDispatchKind,
    task_ref: &ContentDigest,
    attempt_number: u8,
    attempt_id: &AttemptId,
    binding_digest: &ContentDigest,
    writer_fence: u64,
    foreman_generation: u64,
    foreman_checkpoint_digest: &ContentDigest,
    anchor_digest: &ContentDigest,
    supporting_digest: &ContentDigest,
    subject_digest: &ContentDigest,
) -> Result<ContentDigest, AdapterError> {
    if !(1..=3).contains(&attempt_number) || writer_fence == 0 || foreman_generation == 0 {
        return Err(input_error());
    }
    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_FOREMAN_PROVIDER_DISPATCH_V1\0");
    let attempt_number = attempt_number.to_string();
    let writer_fence = writer_fence.to_string();
    let foreman_generation = foreman_generation.to_string();
    for value in [
        kind.as_str(),
        task_ref.as_str(),
        &attempt_number,
        attempt_id.as_str(),
        binding_digest.as_str(),
        &writer_fence,
        &foreman_generation,
        foreman_checkpoint_digest.as_str(),
        anchor_digest.as_str(),
        supporting_digest.as_str(),
        subject_digest.as_str(),
    ] {
        update_framed(&mut hasher, value.as_bytes());
    }
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        use fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| input_error())?;
    }
    ContentDigest::from_sha256(output).map_err(|_| input_error())
}

fn provider_dispatch_receipt_digest(
    dispatch_digest: &ContentDigest,
    claimed_at: &str,
) -> Result<ContentDigest, AdapterError> {
    if claimed_at.is_empty() || claimed_at.len() > 40 {
        return Err(corrupt_error());
    }
    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_FOREMAN_PROVIDER_DISPATCH_RECEIPT_V1");
    hasher.update([0]);
    hasher.update(dispatch_digest.as_str().as_bytes());
    hasher.update([0]);
    hasher.update(claimed_at.as_bytes());
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        use fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| corrupt_error())?;
    }
    ContentDigest::from_sha256(output).map_err(|_| corrupt_error())
}

fn update_framed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u64::try_from(value.len())
            .expect("bounded replay field")
            .to_be_bytes(),
    );
    hasher.update(value);
}

const fn adapter_error(kind: AdapterErrorKind, code: &'static str) -> AdapterError {
    AdapterError {
        kind,
        code,
        database_stage: None,
        sqlstate: None,
    }
}

const fn input_error() -> AdapterError {
    adapter_error(
        AdapterErrorKind::InvalidInput,
        "FOREMAN_ADAPTER_INVALID_INPUT",
    )
}

const fn database_error() -> AdapterError {
    adapter_error(
        AdapterErrorKind::Database,
        "FOREMAN_ADAPTER_DATABASE_FAILED",
    )
}

fn database_error_at(stage: AdapterDatabaseStage, error: &postgres::Error) -> AdapterError {
    AdapterError {
        kind: AdapterErrorKind::Database,
        code: "FOREMAN_ADAPTER_DATABASE_FAILED",
        database_stage: Some(stage),
        sqlstate: Some(sanitized_sqlstate(
            error.as_db_error().map(|database| database.code().code()),
        )),
    }
}

fn sanitized_sqlstate(value: Option<&str>) -> &'static str {
    match value {
        Some("22P02") => "22P02",
        Some("42501") => "42501",
        Some("42601") => "42601",
        Some("42804") => "42804",
        Some("42883") => "42883",
        Some("42P01") => "42P01",
        Some("P0001") => "P0001",
        Some("XX000") => "XX000",
        _ => "OTHER",
    }
}

fn claim_database_error(error: &postgres::Error) -> AdapterError {
    classify_claim_database_message(error.as_db_error().map(postgres::error::DbError::message))
}

fn execution_environment_database_error(error: &postgres::Error) -> AdapterError {
    let Some(database) = error.as_db_error() else {
        return database_error();
    };
    if database.message() == "FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH" {
        return corrupt_error();
    }
    let code = match database.message() {
        "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED" => {
            "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED"
        }
        "FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH" => {
            "FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH"
        }
        "FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH" => {
            "FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH"
        }
        "FOREMAN_EXECUTION_ENVIRONMENT_SUBSTITUTION" => {
            "FOREMAN_EXECUTION_ENVIRONMENT_SUBSTITUTION"
        }
        _ => return database_error(),
    };
    adapter_error(AdapterErrorKind::ClaimRejected, code)
}

fn execution_environment_read_database_error(error: &postgres::Error) -> AdapterError {
    match error.as_db_error().map(postgres::error::DbError::message) {
        Some("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH") => corrupt_error(),
        _ => database_error(),
    }
}

fn provider_dispatch_database_error(error: &postgres::Error) -> AdapterError {
    classify_provider_dispatch_database_message(
        error.as_db_error().map(postgres::error::DbError::message),
    )
}

fn classify_provider_dispatch_database_message(message: Option<&str>) -> AdapterError {
    match message {
        Some(
            "FOREMAN_PROVIDER_DISPATCH_INPUT_REJECTED"
            | "FOREMAN_PROVIDER_DISPATCH_ATTEMPT_MISMATCH"
            | "FOREMAN_PROVIDER_DISPATCH_ANCHOR_MISMATCH"
            | "FOREMAN_PROVIDER_DISPATCH_AUTHORITY_NOT_CURRENT"
            | "FOREMAN_PROVIDER_DISPATCH_WRITER_FENCE_STALE"
            | "FOREMAN_PROVIDER_DISPATCH_FOREMAN_FENCE_STALE"
            | "FOREMAN_PROVIDER_DISPATCH_SUBSTITUTION"
            | "FOREMAN_PROVIDER_DISPATCH_ATTEMPT_CLOSED"
            | "FOREMAN_PROVIDER_DISPATCH_EXECUTION_ENVIRONMENT_NOT_CURRENT",
        ) => adapter_error(
            AdapterErrorKind::ClaimRejected,
            "FOREMAN_PROVIDER_DISPATCH_REJECTED",
        ),
        _ => database_error(),
    }
}

fn classify_claim_database_message(message: Option<&str>) -> AdapterError {
    let code = match message {
        Some("FOREMAN_RETRY_BUDGET_EXHAUSTED") => "FOREMAN_RETRY_BUDGET_EXHAUSTED",
        Some("FOREMAN_MODEL_NOT_ALLOWED") => "FOREMAN_MODEL_NOT_ALLOWED",
        Some("FOREMAN_ATTEMPT_BINDING_MISMATCH") => "FOREMAN_ATTEMPT_BINDING_MISMATCH",
        Some("FOREMAN_ATTEMPT_SEQUENCE_MISMATCH") => "FOREMAN_ATTEMPT_SEQUENCE_MISMATCH",
        Some("FOREMAN_RETRY_PREDECESSOR_NOT_TERMINAL") => "FOREMAN_RETRY_PREDECESSOR_NOT_TERMINAL",
        Some("FOREMAN_GLOBAL_CAPACITY_EXHAUSTED") => "FOREMAN_GLOBAL_CAPACITY_EXHAUSTED",
        Some("FOREMAN_TASK_CAPACITY_EXHAUSTED") => "FOREMAN_TASK_CAPACITY_EXHAUSTED",
        Some("FOREMAN_ATTEMPT_STREAM_MISMATCH") => "FOREMAN_ATTEMPT_STREAM_MISMATCH",
        Some("FOREMAN_ATTEMPT_SUBSTITUTION") => "FOREMAN_ATTEMPT_SUBSTITUTION",
        Some("FOREMAN_PENDING_CLAIM_REQUIRED") => "FOREMAN_PENDING_CLAIM_REQUIRED",
        Some("FOREMAN_PENDING_CLAIM_SUBSTITUTION") => "FOREMAN_PENDING_CLAIM_SUBSTITUTION",
        Some("FOREMAN_PENDING_CLOSURE_REQUIRED") => "FOREMAN_PENDING_CLOSURE_REQUIRED",
        Some("FOREMAN_EXECUTION_ENVIRONMENT_REF_REJECTED") => {
            "FOREMAN_EXECUTION_ENVIRONMENT_REF_REJECTED"
        }
        Some("FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED") => "FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED",
        Some("FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH") => {
            "FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH"
        }
        Some("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH") => {
            "FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH"
        }
        Some("FOREMAN_TASK_LEDGER_EVENT_MISMATCH") => "FOREMAN_TASK_LEDGER_EVENT_MISMATCH",
        Some("FOREMAN_CHILD_EVENT_SUBSTITUTION") => "FOREMAN_CHILD_EVENT_SUBSTITUTION",
        Some("FOREMAN_CHILD_EVENT_REPLAY_MISMATCH") => "FOREMAN_CHILD_EVENT_REPLAY_MISMATCH",
        _ => return database_error(),
    };
    adapter_error(AdapterErrorKind::ClaimRejected, code)
}

fn artifact_database_error(error: &postgres::Error) -> AdapterError {
    classify_artifact_database_message(error.as_db_error().map(postgres::error::DbError::message))
}

fn attempt_closure_database_error(error: &postgres::Error) -> AdapterError {
    let Some(database) = error.as_db_error() else {
        return database_error();
    };
    match database.message() {
        "FOREMAN_ATTEMPT_CLOSURE_SUBSTITUTION"
        | "FOREMAN_ATTEMPT_CLOSURE_BINDING_MISMATCH"
        | "FOREMAN_ATTEMPT_CLOSURE_BLOCKER_REJECTED"
        | "FOREMAN_ATTEMPT_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE"
        | "FOREMAN_RETAINED_CLOSURE_SUBSTITUTION"
        | "FOREMAN_RETAINED_CLOSURE_BINDING_MISMATCH"
        | "FOREMAN_RETAINED_CLOSURE_BLOCKER_REJECTED"
        | "FOREMAN_RETAINED_CLOSURE_PROOF_REJECTED"
        | "FOREMAN_RETAINED_CLOSURE_PROVIDER_STILL_POSSIBLY_ACTIVE"
        | "FOREMAN_PENDING_CLOSURE_SUBSTITUTION"
        | "FOREMAN_PENDING_CLOSURE_REJECTED" => adapter_error(
            AdapterErrorKind::ClaimRejected,
            "FOREMAN_ATTEMPT_CLOSURE_REJECTED",
        ),
        _ => database_error(),
    }
}

fn closed_attempt_blocker(code: &str) -> bool {
    matches!(
        code,
        "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"
            | "LATTICE_MANAGED_HEARTBEAT_TIMEOUT_WHILE_IN_PROGRESS"
            | "LATTICE_MANAGED_PRESTART_CONFIGURATION_REJECTED"
            | "LATTICE_MANAGED_DEADLINE_EXCEEDED"
            | "LATTICE_MANAGED_MODEL_UNAVAILABLE"
            | "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED"
            | "LATTICE_MANAGED_REVIEW_MODEL_PROBE_TIMEOUT_NO_PROVIDER_EFFECT"
            | "LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED"
            | "LATTICE_MANAGED_VERIFICATION_FAILED"
            | "LATTICE_MANAGED_REVIEW_RESULT_REJECTED"
            | "LATTICE_MANAGED_TOKEN_BUDGET_EXHAUSTED"
            | "LATTICE_MANAGED_MODEL_CALL_BUDGET_EXHAUSTED"
            | "LATTICE_MANAGED_MODEL_USAGE_RECONCILIATION_REQUIRED"
            | "LATTICE_MANAGED_REPOSITORY_LINEAGE_MISMATCH"
    )
}

fn retained_worker_blocker(code: &str) -> bool {
    matches!(
        code,
        "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"
            | "LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED"
            | "LATTICE_MANAGED_BRIDGE_HEARTBEAT_TIMEOUT_RECONCILIATION_REQUIRED"
            | "LATTICE_MANAGED_THREAD_START_RPC_INVALID_PARAMS"
            | "LATTICE_MANAGED_THREAD_START_RPC_REJECTED"
            | "LATTICE_MANAGED_TURN_START_RPC_INVALID_PARAMS"
            | "LATTICE_MANAGED_TURN_START_RPC_REJECTED"
    )
}

fn classify_artifact_database_message(message: Option<&str>) -> AdapterError {
    let quota_code = match message {
        Some("FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED") => {
            Some("FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED")
        }
        Some("FOREMAN_ARTIFACT_TASK_QUOTA_EXHAUSTED") => {
            Some("FOREMAN_ARTIFACT_TASK_QUOTA_EXHAUSTED")
        }
        _ => None,
    };
    if let Some(code) = quota_code {
        return adapter_error(AdapterErrorKind::QuotaRejected, code);
    }
    let rejected_code = match message {
        Some("FOREMAN_ARTIFACT_SUBSTITUTION") => "FOREMAN_ARTIFACT_SUBSTITUTION",
        Some("FOREMAN_ARTIFACT_ATTEMPT_MISMATCH") => "FOREMAN_ARTIFACT_ATTEMPT_MISMATCH",
        Some("FOREMAN_ARTIFACT_STAGE_SUBSTITUTION") => "FOREMAN_ARTIFACT_STAGE_SUBSTITUTION",
        Some("FOREMAN_ARTIFACT_STAGE_LEDGER_HEAD_MISMATCH") => {
            "FOREMAN_ARTIFACT_STAGE_LEDGER_HEAD_MISMATCH"
        }
        Some("FOREMAN_ARTIFACT_STAGE_REQUIRED") => "FOREMAN_ARTIFACT_STAGE_REQUIRED",
        Some("FOREMAN_ARTIFACT_CONTENT_DIGEST_MISMATCH") => {
            "FOREMAN_ARTIFACT_CONTENT_DIGEST_MISMATCH"
        }
        Some("FOREMAN_ARTIFACT_DESCRIPTOR_DIGEST_MISMATCH") => {
            "FOREMAN_ARTIFACT_DESCRIPTOR_DIGEST_MISMATCH"
        }
        Some("FOREMAN_ARTIFACT_MEDIA_TYPE_REJECTED") => "FOREMAN_ARTIFACT_MEDIA_TYPE_REJECTED",
        Some("FOREMAN_ARTIFACT_SECRET_REJECTED") => "FOREMAN_ARTIFACT_SECRET_REJECTED",
        Some(
            "FOREMAN_TASK_LEDGER_EVENT_MISMATCH"
            | "FOREMAN_CHILD_EVENT_SUBSTITUTION"
            | "FOREMAN_CHILD_EVENT_REPLAY_MISMATCH",
        ) => "FOREMAN_ARTIFACT_FINALIZE_REJECTED",
        _ => return database_error(),
    };
    adapter_error(AdapterErrorKind::ClaimRejected, rejected_code)
}

const fn corrupt_error() -> AdapterError {
    adapter_error(
        AdapterErrorKind::CorruptReplay,
        "FOREMAN_ADAPTER_CORRUPT_REPLAY",
    )
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterDatabaseStage, AdapterErrorKind, ReplayRecord, ReplayRecordState,
        classify_artifact_database_message, classify_claim_database_message,
        classify_provider_dispatch_database_message, closed_attempt_blocker,
        provider_dispatch_receipt_digest, replay_digest, sanitized_sqlstate,
    };
    use lattice_contracts::ContentDigest;

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
    }

    #[test]
    fn database_diagnostics_are_limited_to_fixed_stage_and_sqlstate_allowlists() {
        assert_eq!(
            AdapterDatabaseStage::TaskPromotion.as_str(),
            "RECORD_TASK_PROMOTION"
        );
        assert_eq!(
            AdapterDatabaseStage::PreparationObservation.as_str(),
            "RECORD_PREPARATION_OBSERVATION"
        );
        for state in [
            "22P02", "42501", "42601", "42804", "42883", "42P01", "P0001", "XX000",
        ] {
            assert_eq!(sanitized_sqlstate(Some(state)), state);
        }
        for untrusted in [
            None,
            Some("password=do-not-echo"),
            Some("08006"),
            Some("P0001: injected detail"),
        ] {
            assert_eq!(sanitized_sqlstate(untrusted), "OTHER");
        }
    }

    #[test]
    fn task_replay_digest_binds_the_exact_ledger_link() {
        let task_ref = digest('1');
        let record = ReplayRecord {
            record_kind: "WORKER_ATTEMPT".to_owned(),
            record_state: ReplayRecordState::Retained,
            attempt_number: Some(1),
            record_ordinal: 1,
            record_digest: digest('2'),
            ledger_stream_id: digest('3'),
            ledger_event_sequence: 7,
            ledger_event_digest: digest('4'),
            recorded_at: "2026-08-27T12:00:00Z".to_owned(),
        };
        let exact = replay_digest(&task_ref, std::slice::from_ref(&record)).expect("exact replay");

        let mut changed_stream = record.clone();
        changed_stream.ledger_stream_id = digest('5');
        assert_ne!(
            exact,
            replay_digest(&task_ref, &[changed_stream]).expect("changed stream")
        );
        let mut changed_sequence = record;
        changed_sequence.ledger_event_sequence += 1;
        assert_ne!(
            exact,
            replay_digest(&task_ref, &[changed_sequence]).expect("changed sequence")
        );
    }

    #[test]
    fn provider_dispatch_receipt_binds_database_claim_time() {
        let dispatch = digest('7');
        let exact = provider_dispatch_receipt_digest(&dispatch, "2026-08-27T12:00:00.000001Z")
            .expect("exact receipt");
        assert_ne!(
            exact,
            provider_dispatch_receipt_digest(&dispatch, "2026-08-27T12:00:00.000002Z")
                .expect("changed time"),
        );
        assert_ne!(
            exact,
            provider_dispatch_receipt_digest(&digest('8'), "2026-08-27T12:00:00.000001Z")
                .expect("changed dispatch"),
        );
    }

    #[test]
    fn claim_database_messages_map_only_through_the_closed_allowlist() {
        for code in [
            "FOREMAN_RETRY_BUDGET_EXHAUSTED",
            "FOREMAN_MODEL_NOT_ALLOWED",
            "FOREMAN_ATTEMPT_BINDING_MISMATCH",
            "FOREMAN_ATTEMPT_SEQUENCE_MISMATCH",
            "FOREMAN_RETRY_PREDECESSOR_NOT_TERMINAL",
            "FOREMAN_GLOBAL_CAPACITY_EXHAUSTED",
            "FOREMAN_TASK_CAPACITY_EXHAUSTED",
            "FOREMAN_ATTEMPT_STREAM_MISMATCH",
            "FOREMAN_ATTEMPT_SUBSTITUTION",
            "FOREMAN_PENDING_CLAIM_REQUIRED",
            "FOREMAN_PENDING_CLAIM_SUBSTITUTION",
            "FOREMAN_PENDING_CLOSURE_REQUIRED",
            "FOREMAN_EXECUTION_ENVIRONMENT_REF_REJECTED",
            "FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED",
            "FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH",
            "FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH",
            "FOREMAN_TASK_LEDGER_EVENT_MISMATCH",
            "FOREMAN_CHILD_EVENT_SUBSTITUTION",
            "FOREMAN_CHILD_EVENT_REPLAY_MISMATCH",
        ] {
            let error = classify_claim_database_message(Some(code));
            assert_eq!(error.kind(), AdapterErrorKind::ClaimRejected);
            assert_eq!(error.code(), code);
        }

        for untrusted in [
            None,
            Some("password=do-not-echo"),
            Some("FOREMAN_GLOBAL_CAPACITY_EXHAUSTED: injected detail"),
        ] {
            let error = classify_claim_database_message(untrusted);
            assert_eq!(error.kind(), AdapterErrorKind::Database);
            assert_eq!(error.code(), "FOREMAN_ADAPTER_DATABASE_FAILED");
        }
    }

    #[test]
    fn provider_dispatch_messages_map_only_through_the_closed_allowlist() {
        for code in [
            "FOREMAN_PROVIDER_DISPATCH_INPUT_REJECTED",
            "FOREMAN_PROVIDER_DISPATCH_ATTEMPT_MISMATCH",
            "FOREMAN_PROVIDER_DISPATCH_ANCHOR_MISMATCH",
            "FOREMAN_PROVIDER_DISPATCH_AUTHORITY_NOT_CURRENT",
            "FOREMAN_PROVIDER_DISPATCH_WRITER_FENCE_STALE",
            "FOREMAN_PROVIDER_DISPATCH_FOREMAN_FENCE_STALE",
            "FOREMAN_PROVIDER_DISPATCH_SUBSTITUTION",
            "FOREMAN_PROVIDER_DISPATCH_ATTEMPT_CLOSED",
            "FOREMAN_PROVIDER_DISPATCH_EXECUTION_ENVIRONMENT_NOT_CURRENT",
        ] {
            let error = classify_provider_dispatch_database_message(Some(code));
            assert_eq!(error.kind(), AdapterErrorKind::ClaimRejected);
            assert_eq!(error.code(), "FOREMAN_PROVIDER_DISPATCH_REJECTED");
        }

        for untrusted in [
            None,
            Some("password=do-not-echo"),
            Some("FOREMAN_PROVIDER_DISPATCH_AUTHORITY_NOT_CURRENT: detail"),
        ] {
            let error = classify_provider_dispatch_database_message(untrusted);
            assert_eq!(error.kind(), AdapterErrorKind::Database);
            assert_eq!(error.code(), "FOREMAN_ADAPTER_DATABASE_FAILED");
        }
    }

    #[test]
    fn artifact_quota_messages_map_only_through_the_closed_allowlist() {
        for code in [
            "FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED",
            "FOREMAN_ARTIFACT_TASK_QUOTA_EXHAUSTED",
        ] {
            let error = classify_artifact_database_message(Some(code));
            assert_eq!(error.kind(), AdapterErrorKind::QuotaRejected);
            assert_eq!(error.code(), code);
        }
        for untrusted in [
            None,
            Some("token=do-not-echo"),
            Some("FOREMAN_ARTIFACT_TASK_QUOTA_EXHAUSTED: detail"),
        ] {
            let error = classify_artifact_database_message(untrusted);
            assert_eq!(error.kind(), AdapterErrorKind::Database);
            assert_eq!(error.code(), "FOREMAN_ADAPTER_DATABASE_FAILED");
        }

        for code in [
            "FOREMAN_ARTIFACT_SUBSTITUTION",
            "FOREMAN_ARTIFACT_ATTEMPT_MISMATCH",
            "FOREMAN_ARTIFACT_STAGE_SUBSTITUTION",
            "FOREMAN_ARTIFACT_STAGE_LEDGER_HEAD_MISMATCH",
            "FOREMAN_ARTIFACT_STAGE_REQUIRED",
            "FOREMAN_ARTIFACT_CONTENT_DIGEST_MISMATCH",
            "FOREMAN_ARTIFACT_DESCRIPTOR_DIGEST_MISMATCH",
            "FOREMAN_ARTIFACT_MEDIA_TYPE_REJECTED",
            "FOREMAN_ARTIFACT_SECRET_REJECTED",
        ] {
            let error = classify_artifact_database_message(Some(code));
            assert_eq!(error.kind(), AdapterErrorKind::ClaimRejected);
            assert_eq!(error.code(), code);
        }
        for code in [
            "FOREMAN_TASK_LEDGER_EVENT_MISMATCH",
            "FOREMAN_CHILD_EVENT_SUBSTITUTION",
            "FOREMAN_CHILD_EVENT_REPLAY_MISMATCH",
        ] {
            let error = classify_artifact_database_message(Some(code));
            assert_eq!(error.kind(), AdapterErrorKind::ClaimRejected);
            assert_eq!(error.code(), "FOREMAN_ARTIFACT_FINALIZE_REJECTED");
        }
    }

    #[test]
    fn attempt_closure_excludes_ambiguous_provider_failures() {
        for closed in [
            "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT",
            "LATTICE_MANAGED_VERIFICATION_FAILED",
            "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED",
            "LATTICE_MANAGED_REVIEW_MODEL_PROBE_TIMEOUT_NO_PROVIDER_EFFECT",
        ] {
            assert!(
                closed_attempt_blocker(closed),
                "missing closed code {closed}"
            );
        }
        for ambiguous in [
            "LATTICE_MANAGED_DISPATCH_RECONCILIATION_REQUIRED",
            "LATTICE_MANAGED_EXACT_START_EVIDENCE_LOST_AFTER_DISPATCH",
            "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL",
            "LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED",
            "LATTICE_MANAGED_REVIEW_DISPATCH_RECONCILIATION_REQUIRED",
            "LATTICE_MANAGED_REVIEW_CLEANUP_AMBIGUOUS",
        ] {
            assert!(!closed_attempt_blocker(ambiguous));
        }
    }
}
