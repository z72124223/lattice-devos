//! Workspace Git 2.0 consumes formal owner authority before Git effects.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, normalize_nfc};
use lattice_contracts::{
    ContentDigest, FencingToken, ProjectAuthorityHead, ProjectAuthorityReceipt, ProjectId,
    ProjectLifecycle, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind,
    WriterLeaseAuthorityHead, WriterLeaseAuthorityReceipt, WriterLeaseStatus,
};
use lattice_project_registry::RepositoryObservation;

/// Stable fail-closed errors returned by Workspace Git 2.0.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceError {
    /// The supplied Registry receipt is not the independently queried head.
    ProjectHeadMismatch,
    /// The accepted Registry observation does not match the current receipt.
    ProjectObservationMismatch,
    /// Registry lifecycle is not active.
    ProjectNotActive,
    /// The supplied Writer Lease receipt is not the independently queried head.
    LeaseHeadMismatch,
    /// Writer Lease is suspect or otherwise cannot authorize mutation.
    LeaseNotActive,
    /// Runtime admission is not active.
    RuntimeNotActive,
    /// Registry and Writer Lease project/snapshot/runtime bindings disagree.
    AuthorityBindingMismatch,
    /// The configured task-workspace root is absent, linked, or ambiguous.
    WorkspaceRootUnsafe,
    /// The Registry-owned repository root is absent, linked, or not the Git top level.
    RepositoryIdentityMismatch,
    /// A commit identifier is not an exact lowercase Git object hash.
    InvalidCommitId,
    /// The exact requested base commit could not be resolved.
    BaseCommitMismatch,
    /// The registered source checkout contains product changes.
    RepositoryDirty,
    /// Local/worktree Git configuration could execute external behavior.
    UnsafeGitConfiguration,
    /// A task worktree target, branch, or defense-in-depth guard already exists.
    WorktreeTargetExists,
    /// A Git child could not be started or returned malformed/oversized output.
    GitExecutionFailed,
    /// Git reported a terminal command failure before a verified result existed.
    GitCommandFailed {
        /// Bounded child exit code.
        exit_code: i32,
        /// Exact argument array rendered lossily for diagnostics only.
        args: Vec<String>,
        /// Bounded standard error.
        stderr: String,
    },
    /// Git changed state but the resulting worktree identity could not be proven.
    ReconciliationRequired,
    /// A process-local worktree handle no longer matches current authority or Git identity.
    WorktreeIdentityMismatch,
    /// A LATTICE-owned worktree has tracked or untracked changes and is retained.
    WorktreeDirty,
    /// Git returned malformed or unsafe repository-relative change evidence.
    InvalidChangeEvidence,
    /// Canonical evidence hashing failed.
    EvidenceHashFailed,
}

impl WorkspaceError {
    /// Returns a stable machine-readable reason code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ProjectHeadMismatch => "WORKSPACE_PROJECT_HEAD_MISMATCH",
            Self::ProjectObservationMismatch => "WORKSPACE_PROJECT_OBSERVATION_MISMATCH",
            Self::ProjectNotActive => "WORKSPACE_PROJECT_NOT_ACTIVE",
            Self::LeaseHeadMismatch => "WORKSPACE_LEASE_HEAD_MISMATCH",
            Self::LeaseNotActive => "WORKSPACE_LEASE_NOT_ACTIVE",
            Self::RuntimeNotActive => "WORKSPACE_RUNTIME_NOT_ACTIVE",
            Self::AuthorityBindingMismatch => "WORKSPACE_AUTHORITY_BINDING_MISMATCH",
            Self::WorkspaceRootUnsafe => "WORKSPACE_ROOT_UNSAFE",
            Self::RepositoryIdentityMismatch => "WORKSPACE_REPOSITORY_IDENTITY_MISMATCH",
            Self::InvalidCommitId => "WORKSPACE_INVALID_COMMIT_ID",
            Self::BaseCommitMismatch => "WORKSPACE_BASE_COMMIT_MISMATCH",
            Self::RepositoryDirty => "WORKSPACE_REPOSITORY_DIRTY",
            Self::UnsafeGitConfiguration => "WORKSPACE_GIT_CONFIGURATION_UNSAFE",
            Self::WorktreeTargetExists => "WORKSPACE_TARGET_EXISTS",
            Self::GitExecutionFailed => "WORKSPACE_GIT_EXECUTION_FAILED",
            Self::GitCommandFailed { .. } => "WORKSPACE_GIT_COMMAND_FAILED",
            Self::ReconciliationRequired => "WORKSPACE_RECONCILIATION_REQUIRED",
            Self::WorktreeIdentityMismatch => "WORKSPACE_WORKTREE_IDENTITY_MISMATCH",
            Self::WorktreeDirty => "WORKSPACE_WORKTREE_DIRTY",
            Self::InvalidChangeEvidence => "WORKSPACE_INVALID_CHANGE_EVIDENCE",
            Self::EvidenceHashFailed => "WORKSPACE_EVIDENCE_HASH_FAILED",
        }
    }
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for WorkspaceError {}

/// Exact Registry and Writer Lease capability required before Git effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceAuthority {
    repository_root: String,
    project_receipt: ProjectAuthorityReceipt,
    lease_receipt: WriterLeaseAuthorityReceipt,
}

impl WorkspaceAuthority {
    /// Binds owner-produced receipts to independently queried current heads.
    ///
    /// # Errors
    ///
    /// Every stale, substituted, non-active, or cross-scope value fails
    /// closed. This function issues no lease, fencing token, or Git truth.
    pub fn bind(
        observation: &RepositoryObservation,
        project_receipt: ProjectAuthorityReceipt,
        project_current_head: &ProjectAuthorityHead,
        lease_receipt: WriterLeaseAuthorityReceipt,
        lease_current_head: &WriterLeaseAuthorityHead,
    ) -> Result<Self, WorkspaceError> {
        if project_receipt.head() != *project_current_head {
            return Err(WorkspaceError::ProjectHeadMismatch);
        }
        if observation.digest() != project_receipt.observation_digest()
            || observation.primary_branch() != project_receipt.primary_branch()
        {
            return Err(WorkspaceError::ProjectObservationMismatch);
        }
        if project_receipt.lifecycle() != ProjectLifecycle::Active {
            return Err(WorkspaceError::ProjectNotActive);
        }
        if lease_receipt.head() != *lease_current_head {
            return Err(WorkspaceError::LeaseHeadMismatch);
        }
        if lease_receipt.status() != WriterLeaseStatus::Active {
            return Err(WorkspaceError::LeaseNotActive);
        }
        if lease_receipt.runtime_admission() != RuntimeAdmissionMode::Active {
            return Err(WorkspaceError::RuntimeNotActive);
        }
        let lease_identity = lease_receipt.identity();
        if lease_receipt.runtime() != project_receipt.runtime()
            || lease_identity.project_id() != project_receipt.project_id()
            || lease_identity.project_snapshot_id() != project_receipt.project_snapshot_id()
        {
            return Err(WorkspaceError::AuthorityBindingMismatch);
        }
        Ok(Self {
            repository_root: observation.canonical_root().to_owned(),
            project_receipt,
            lease_receipt,
        })
    }

    /// Returns the fake/live runtime carried by both owner receipts.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.project_receipt.runtime()
    }

    /// Returns the registered project identity.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        self.project_receipt.project_id()
    }

    /// Returns the immutable Registry snapshot identity.
    #[must_use]
    pub const fn project_snapshot_id(&self) -> &ProjectSnapshotId {
        self.project_receipt.project_snapshot_id()
    }

    /// Returns the Registry-owned canonical repository root.
    #[must_use]
    pub fn repository_root(&self) -> &str {
        &self.repository_root
    }

    /// Returns the current formal fencing token.
    #[must_use]
    pub const fn fencing_token(&self) -> FencingToken {
        self.lease_receipt.identity().fencing_token()
    }

    /// Returns the lease-bound LATTICE worktree identity.
    #[must_use]
    pub fn worktree_id(&self) -> &str {
        self.lease_receipt.identity().worktree_id()
    }

    fn task_id(&self) -> &str {
        self.lease_receipt.identity().task_id().as_str()
    }

    fn lease_id(&self) -> &str {
        self.lease_receipt.identity().lease_id()
    }
}

/// Validated exact SHA-1 or SHA-256 Git commit object identifier.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CommitId(String);

impl CommitId {
    /// Validates a lowercase 40- or 64-hex Git object identifier.
    ///
    /// # Errors
    ///
    /// Returns [`WorkspaceError::InvalidCommitId`] for ambiguous revision text.
    pub fn new(value: impl Into<String>) -> Result<Self, WorkspaceError> {
        let value = value.into();
        if matches!(value.len(), 40 | 64)
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err(WorkspaceError::InvalidCommitId)
        }
    }

    /// Returns the exact lowercase object ID.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Closed normalized Git change operation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ChangeOperation {
    /// A new path appears.
    Create,
    /// Existing content or metadata changed.
    Modify,
    /// A tracked path was removed.
    Delete,
    /// One tracked path moved to another path.
    Rename,
    /// Git file kind changed.
    TypeChange,
    /// Git reports an unresolved merge entry.
    Unmerged,
    /// Git emitted an unrecognized operation.
    Unknown,
}

impl ChangeOperation {
    /// Returns the stable language-neutral value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "CREATE",
            Self::Modify => "MODIFY",
            Self::Delete => "DELETE",
            Self::Rename => "RENAME",
            Self::TypeChange => "TYPE_CHANGE",
            Self::Unmerged => "UNMERGED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// Where one observed change exists relative to the base commit.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ChangeState {
    /// Change is present in `HEAD` after the requested base.
    Committed,
    /// Change is present in the index after `HEAD`.
    Staged,
    /// Change is present in the worktree after the index.
    Unstaged,
    /// Path is untracked and not ignored.
    Untracked,
    /// Path is ignored but remains observable scope evidence.
    Ignored,
}

impl ChangeState {
    /// Returns the stable language-neutral value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "COMMITTED",
            Self::Staged => "STAGED",
            Self::Unstaged => "UNSTAGED",
            Self::Untracked => "UNTRACKED",
            Self::Ignored => "IGNORED",
        }
    }
}

/// Physical/path kind observed on one side of a Git change.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PathKind {
    /// Ordinary tracked or untracked file.
    Regular,
    /// Filesystem symbolic link.
    Symlink,
    /// Gitlink/submodule entry.
    GitLink,
    /// Windows junction or other reparse-point traversal.
    ReparsePoint,
    /// Directory appeared as change evidence.
    Directory,
    /// The path is absent on this side of the change.
    Missing,
    /// Kind could not be proven.
    Unknown,
}

impl PathKind {
    /// Returns the stable language-neutral value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Regular => "REGULAR",
            Self::Symlink => "SYMLINK",
            Self::GitLink => "GIT_LINK",
            Self::ReparsePoint => "REPARSE_POINT",
            Self::Directory => "DIRECTORY",
            Self::Missing => "MISSING",
            Self::Unknown => "UNKNOWN",
        }
    }
}

/// One validated repository-relative path and physical kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathEvidence {
    path: String,
    kind: PathKind,
}

impl PathEvidence {
    /// Constructs validated repository-relative path evidence.
    ///
    /// # Errors
    ///
    /// Absolute, non-normalized, `.git`, traversal, drive-qualified, or
    /// backslash-bearing paths fail closed.
    pub fn new(path: impl Into<String>, kind: PathKind) -> Result<Self, WorkspaceError> {
        let path = normalized_repository_path(&path.into())?;
        Ok(Self { path, kind })
    }

    /// Returns the normalized slash-separated repository-relative path.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the observed physical/path kind.
    #[must_use]
    pub const fn kind(&self) -> PathKind {
        self.kind
    }
}

/// One normalized changed-path record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeRecord {
    operation: ChangeOperation,
    source: Option<PathEvidence>,
    destination: Option<PathEvidence>,
    states: Vec<ChangeState>,
    git_status: String,
}

impl ChangeRecord {
    /// Constructs one syntactically normalized change record for read-only
    /// downstream classifiers.
    ///
    /// This constructor does not make caller-supplied evidence authoritative.
    ///
    /// # Errors
    ///
    /// Invalid operation sides, empty states, or malformed status text fail
    /// closed.
    pub fn new(
        operation: ChangeOperation,
        source: Option<PathEvidence>,
        destination: Option<PathEvidence>,
        mut states: Vec<ChangeState>,
        git_status: impl Into<String>,
    ) -> Result<Self, WorkspaceError> {
        let valid_sides = match operation {
            ChangeOperation::Create => source.is_none() && destination.is_some(),
            ChangeOperation::Delete => source.is_some() && destination.is_none(),
            ChangeOperation::Rename => {
                matches!((&source, &destination), (Some(left), Some(right)) if left.path() != right.path())
            }
            ChangeOperation::Modify | ChangeOperation::TypeChange | ChangeOperation::Unmerged => {
                matches!((&source, &destination), (Some(left), Some(right)) if left.path() == right.path())
            }
            ChangeOperation::Unknown => source.is_some() || destination.is_some(),
        };
        if !valid_sides
            || source
                .as_ref()
                .is_some_and(|evidence| evidence.kind() == PathKind::Missing)
            || destination
                .as_ref()
                .is_some_and(|evidence| evidence.kind() == PathKind::Missing)
            || states.is_empty()
        {
            return Err(WorkspaceError::InvalidChangeEvidence);
        }
        states.sort();
        states.dedup();
        let git_status = git_status.into();
        if git_status.is_empty()
            || git_status.len() > 32
            || git_status.trim() != git_status
            || git_status.contains(['\0', '\r', '\n'])
            || normalize_nfc(&git_status) != git_status
        {
            return Err(WorkspaceError::InvalidChangeEvidence);
        }
        Ok(Self {
            operation,
            source,
            destination,
            states,
            git_status,
        })
    }

    /// Returns the normalized operation.
    #[must_use]
    pub const fn operation(&self) -> ChangeOperation {
        self.operation
    }

    /// Returns the rename/delete/source path when present.
    #[must_use]
    pub fn source_path(&self) -> Option<&str> {
        self.source.as_ref().map(PathEvidence::path)
    }

    /// Returns the create/modify/rename destination path when present.
    #[must_use]
    pub fn destination_path(&self) -> Option<&str> {
        self.destination.as_ref().map(PathEvidence::path)
    }

    /// Returns the source physical kind when present.
    #[must_use]
    pub fn source_kind(&self) -> Option<PathKind> {
        self.source.as_ref().map(PathEvidence::kind)
    }

    /// Returns the destination physical kind when present.
    #[must_use]
    pub fn destination_kind(&self) -> Option<PathKind> {
        self.destination.as_ref().map(PathEvidence::kind)
    }

    /// Returns stable ordered change states.
    #[must_use]
    pub fn states(&self) -> &[ChangeState] {
        &self.states
    }

    /// Returns a stable comparison key used by manifests and consumers.
    #[must_use]
    pub fn sort_key(&self) -> (&str, &str, &str) {
        (
            self.source_path().unwrap_or(""),
            self.destination_path().unwrap_or(""),
            self.operation.as_str(),
        )
    }

    /// Returns Git's bounded status token as diagnostic evidence.
    #[must_use]
    pub fn git_status(&self) -> &str {
        &self.git_status
    }
}

/// Stable normalized changed-path manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeManifest {
    base_commit: CommitId,
    head_commit: CommitId,
    changes: Vec<ChangeRecord>,
    digest: ContentDigest,
}

impl ChangeManifest {
    /// Constructs a stable manifest from already observed normalized records.
    ///
    /// This is a canonicalization boundary for downstream pure classifiers;
    /// it does not confer Git or mutation authority on caller-supplied records.
    ///
    /// # Errors
    ///
    /// Invalid commit identifiers, duplicate record keys, or hashing failure
    /// fails closed.
    pub fn from_normalized(
        base_commit: &str,
        head_commit: &str,
        mut changes: Vec<ChangeRecord>,
    ) -> Result<Self, WorkspaceError> {
        let base_commit = CommitId::new(base_commit)?;
        let head_commit = CommitId::new(head_commit)?;
        changes.sort_by(|left, right| left.sort_key().cmp(&right.sort_key()));
        if changes
            .windows(2)
            .any(|pair| pair[0].sort_key() == pair[1].sort_key())
        {
            return Err(WorkspaceError::InvalidChangeEvidence);
        }
        let digest = change_manifest_digest(&base_commit, &head_commit, &changes)?;
        Ok(Self {
            base_commit,
            head_commit,
            changes,
            digest,
        })
    }

    /// Returns the exact requested comparison base.
    #[must_use]
    pub const fn base_commit(&self) -> &CommitId {
        &self.base_commit
    }

    /// Returns the exact current worktree `HEAD`.
    #[must_use]
    pub const fn head_commit(&self) -> &CommitId {
        &self.head_commit
    }

    /// Returns the stable sorted complete changed-path set.
    #[must_use]
    pub fn changes(&self) -> &[ChangeRecord] {
        &self.changes
    }

    /// Returns the domain-separated manifest digest.
    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// Dry-run integration result. This is detection evidence, not merge authority.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum IntegrationOutcome {
    /// Git produced a merge result tree without reported conflicts.
    Clean,
    /// Git reported at least one conflicted path.
    Conflict,
}

impl IntegrationOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Conflict => "conflict",
        }
    }
}

/// Stable evidence from Git's checkout-free merge analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeReadiness {
    target_commit: CommitId,
    feature_commit: CommitId,
    result_tree: String,
    outcome: IntegrationOutcome,
    conflicts: Vec<String>,
    digest: ContentDigest,
}

impl MergeReadiness {
    /// Returns the exact integration target supplied to the analysis.
    #[must_use]
    pub const fn target_commit(&self) -> &CommitId {
        &self.target_commit
    }

    /// Returns the exact feature commit supplied to the analysis.
    #[must_use]
    pub const fn feature_commit(&self) -> &CommitId {
        &self.feature_commit
    }

    /// Returns the unreferenced result tree produced by Git.
    #[must_use]
    pub fn result_tree(&self) -> &str {
        &self.result_tree
    }

    /// Returns Git's normalized integration outcome.
    #[must_use]
    pub const fn outcome(&self) -> IntegrationOutcome {
        self.outcome
    }

    /// Returns stable sorted conflicted paths. Both commits remain unchanged.
    #[must_use]
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }

    /// Returns the domain-separated evidence digest.
    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// One argument-array Git invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitRequest {
    cwd: PathBuf,
    args: Vec<OsString>,
    empty_config: PathBuf,
}

impl GitRequest {
    /// Returns the explicit child working directory.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Returns the Git argument array. No shell string exists in this contract.
    #[must_use]
    pub fn args(&self) -> &[OsString] {
        &self.args
    }
}

/// Bounded Git child result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitOutput {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

impl GitOutput {
    /// Returns the process exit code.
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        self.exit_code
    }

    /// Returns bounded UTF-8 standard output.
    #[must_use]
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Returns bounded UTF-8 standard error.
    #[must_use]
    pub fn stderr(&self) -> &str {
        &self.stderr
    }
}

/// Injected Git executable boundary.
pub trait GitExecutor {
    /// Executes one explicit `git` argument array.
    ///
    /// # Errors
    ///
    /// Returns a fail-closed execution error for spawn, output, or encoding
    /// failure.
    fn execute(&self, request: &GitRequest) -> Result<GitOutput, WorkspaceError>;
}

/// Production standard-library Git child executor.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandGitExecutor;

const MAX_GIT_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

fn read_bounded<R: Read>(mut reader: R) -> io::Result<(Vec<u8>, bool)> {
    let mut kept = Vec::new();
    let mut overflow = false;
    let mut buffer = [0_u8; 8192];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = MAX_GIT_OUTPUT_BYTES.saturating_sub(kept.len());
        let copied = remaining.min(count);
        kept.extend_from_slice(&buffer[..copied]);
        overflow |= copied != count;
    }
    Ok((kept, overflow))
}

impl GitExecutor for CommandGitExecutor {
    fn execute(&self, request: &GitRequest) -> Result<GitOutput, WorkspaceError> {
        let mut command = Command::new("git");
        command
            .args(&request.args)
            .current_dir(&request.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_SYSTEM", &request.empty_config)
            .env("GIT_CONFIG_GLOBAL", &request.empty_config)
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat");
        for variable in [
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CEILING_DIRECTORIES",
            "GIT_COMMON_DIR",
            "GIT_CONFIG",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_PARAMETERS",
            "GIT_DIFF_OPTS",
            "GIT_DIR",
            "GIT_DISCOVERY_ACROSS_FILESYSTEM",
            "GIT_EXEC_PATH",
            "GIT_EXTERNAL_DIFF",
            "GIT_INDEX_FILE",
            "GIT_NAMESPACE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_OPTIONAL_LOCKS",
            "GIT_REPLACE_REF_BASE",
            "GIT_WORK_TREE",
        ] {
            command.env_remove(variable);
        }
        for index in 0..64 {
            command.env_remove(format!("GIT_CONFIG_KEY_{index}"));
            command.env_remove(format!("GIT_CONFIG_VALUE_{index}"));
        }
        let mut child = command
            .spawn()
            .map_err(|_| WorkspaceError::GitExecutionFailed)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(WorkspaceError::GitExecutionFailed)?;
        let stderr = child
            .stderr
            .take()
            .ok_or(WorkspaceError::GitExecutionFailed)?;
        let stdout_reader = thread::spawn(move || read_bounded(stdout));
        let stderr_reader = thread::spawn(move || read_bounded(stderr));
        let status = child
            .wait()
            .map_err(|_| WorkspaceError::GitExecutionFailed)?;
        let (stdout, stdout_overflow) = stdout_reader
            .join()
            .map_err(|_| WorkspaceError::GitExecutionFailed)?
            .map_err(|_| WorkspaceError::GitExecutionFailed)?;
        let (stderr, stderr_overflow) = stderr_reader
            .join()
            .map_err(|_| WorkspaceError::GitExecutionFailed)?
            .map_err(|_| WorkspaceError::GitExecutionFailed)?;
        if stdout_overflow || stderr_overflow {
            return Err(WorkspaceError::GitExecutionFailed);
        }
        Ok(GitOutput {
            exit_code: status.code().unwrap_or(1),
            stdout: String::from_utf8(stdout).map_err(|_| WorkspaceError::GitExecutionFailed)?,
            stderr: String::from_utf8(stderr).map_err(|_| WorkspaceError::GitExecutionFailed)?,
        })
    }
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
const fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn native_canonical_path(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

fn ensure_real_directory(path: &Path) -> Result<PathBuf, WorkspaceError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(WorkspaceError::WorkspaceRootUnsafe);
    }
    for ancestor in path.ancestors() {
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|_| WorkspaceError::WorkspaceRootUnsafe)?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(WorkspaceError::WorkspaceRootUnsafe);
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| WorkspaceError::WorkspaceRootUnsafe)?;
    if !metadata.is_dir() {
        return Err(WorkspaceError::WorkspaceRootUnsafe);
    }
    fs::canonicalize(path)
        .map(native_canonical_path)
        .map_err(|_| WorkspaceError::WorkspaceRootUnsafe)
}

fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace("\\\\?\\", "");
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

fn path_contained_by(parent: &Path, candidate: &Path) -> bool {
    let parent = path_key(parent);
    let candidate = path_key(candidate);
    candidate == parent
        || candidate
            .strip_prefix(&parent)
            .is_some_and(|suffix| suffix.starts_with(['/', '\\']))
}

fn authority_slug(authority: &WorkspaceAuthority) -> Result<String, WorkspaceError> {
    let domain = HashDomain::new("lattice.workspace-git.branch", "2.0")
        .map_err(|_| WorkspaceError::AuthorityBindingMismatch)?;
    let value = CanonicalValue::Object(vec![
        (
            "project_id".to_owned(),
            CanonicalValue::String(authority.project_id().as_str().to_owned()),
        ),
        (
            "task_id".to_owned(),
            CanonicalValue::String(authority.task_id().to_owned()),
        ),
        (
            "fencing_token".to_owned(),
            CanonicalValue::String(authority.fencing_token().get().to_string()),
        ),
    ]);
    let digest = canonical_sha256(&domain, &value)
        .map_err(|_| WorkspaceError::AuthorityBindingMismatch)?
        .to_hex();
    Ok(format!(
        "{}-fence-{}-{}",
        authority.project_id().as_str(),
        authority.fencing_token().get(),
        &digest[..16]
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedBinding {
    repository_root: String,
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    project_head: ProjectAuthorityHead,
    lease_id: String,
    worktree_id: String,
    fencing_token: FencingToken,
    lease_head: WriterLeaseAuthorityHead,
}

impl OwnedBinding {
    fn from_authority(authority: &WorkspaceAuthority) -> Self {
        Self {
            repository_root: authority.repository_root().to_owned(),
            project_id: authority.project_id().clone(),
            project_snapshot_id: authority.project_snapshot_id().clone(),
            project_head: authority.project_receipt.head(),
            lease_id: authority.lease_id().to_owned(),
            worktree_id: authority.worktree_id().to_owned(),
            fencing_token: authority.fencing_token(),
            lease_head: authority.lease_receipt.head(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct RawChangeKey {
    operation: ChangeOperation,
    source: Option<String>,
    destination: Option<String>,
}

#[derive(Clone, Debug)]
struct RawChange {
    key: RawChangeKey,
    git_status: String,
    states: BTreeSet<ChangeState>,
}

fn normalized_repository_path(value: &str) -> Result<String, WorkspaceError> {
    if value.is_empty()
        || value.contains(['\0', '\\'])
        || value.starts_with('/')
        || value.ends_with('/')
        || normalize_nfc(value) != value
    {
        return Err(WorkspaceError::InvalidChangeEvidence);
    }
    let segments: Vec<&str> = value.split('/').collect();
    if segments.iter().any(|segment| {
        segment.is_empty()
            || matches!(*segment, "." | "..")
            || segment.eq_ignore_ascii_case(".git")
            || segment.contains(':')
    }) {
        return Err(WorkspaceError::InvalidChangeEvidence);
    }
    Ok(value.to_owned())
}

fn parse_name_status(output: &str) -> Result<Vec<RawChange>, WorkspaceError> {
    if output.is_empty() {
        return Ok(Vec::new());
    }
    let mut tokens: Vec<&str> = output.split('\0').collect();
    if tokens.last() == Some(&"") {
        tokens.pop();
    }
    let mut changes = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        index += 1;
        let (status, first_path) = if let Some((status, path)) = token.split_once('\t') {
            (status, path)
        } else {
            let path = tokens
                .get(index)
                .copied()
                .ok_or(WorkspaceError::InvalidChangeEvidence)?;
            index += 1;
            (token, path)
        };
        let code = status
            .as_bytes()
            .first()
            .copied()
            .ok_or(WorkspaceError::InvalidChangeEvidence)?;
        let first_path = normalized_repository_path(first_path)?;
        let (operation, source, destination) = match code {
            b'R' => {
                let destination = tokens
                    .get(index)
                    .copied()
                    .ok_or(WorkspaceError::InvalidChangeEvidence)?;
                index += 1;
                (
                    ChangeOperation::Rename,
                    Some(first_path),
                    Some(normalized_repository_path(destination)?),
                )
            }
            b'A' | b'C' => (ChangeOperation::Create, None, Some(first_path)),
            b'M' => (
                ChangeOperation::Modify,
                Some(first_path.clone()),
                Some(first_path),
            ),
            b'D' => (ChangeOperation::Delete, Some(first_path), None),
            b'T' => (
                ChangeOperation::TypeChange,
                Some(first_path.clone()),
                Some(first_path),
            ),
            b'U' => (
                ChangeOperation::Unmerged,
                Some(first_path.clone()),
                Some(first_path),
            ),
            _ => (ChangeOperation::Unknown, None, Some(first_path)),
        };
        changes.push(RawChange {
            key: RawChangeKey {
                operation,
                source,
                destination,
            },
            git_status: status.to_owned(),
            states: BTreeSet::new(),
        });
    }
    Ok(changes)
}

fn parse_nul_paths(output: &str) -> Result<BTreeSet<String>, WorkspaceError> {
    output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(normalized_repository_path)
        .collect()
}

fn parse_tree_modes(output: &str) -> Result<BTreeMap<String, String>, WorkspaceError> {
    let mut modes = BTreeMap::new();
    for record in output.split('\0').filter(|record| !record.is_empty()) {
        let (metadata, path) = record
            .split_once('\t')
            .ok_or(WorkspaceError::InvalidChangeEvidence)?;
        let mode = metadata
            .split_whitespace()
            .next()
            .ok_or(WorkspaceError::InvalidChangeEvidence)?;
        modes.insert(normalized_repository_path(path)?, mode.to_owned());
    }
    Ok(modes)
}

fn parse_index_modes(output: &str) -> Result<BTreeMap<String, String>, WorkspaceError> {
    parse_tree_modes(output)
}

fn mode_kind(mode: Option<&String>) -> PathKind {
    match mode.map(String::as_str) {
        Some("120000") => PathKind::Symlink,
        Some("160000") => PathKind::GitLink,
        Some(value) if value.starts_with("100") => PathKind::Regular,
        Some(value) if value.starts_with("040") => PathKind::Directory,
        Some(_) => PathKind::Unknown,
        None => PathKind::Missing,
    }
}

fn current_path_kind(root: &Path, relative: &str) -> PathKind {
    let mut current = root.to_owned();
    let segments: Vec<&str> = relative.split('/').collect();
    for (index, segment) in segments.iter().enumerate() {
        current.push(segment);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return if index + 1 == segments.len() {
                    PathKind::Missing
                } else {
                    PathKind::Unknown
                };
            }
            Err(_) => return PathKind::Unknown,
        };
        if metadata.file_type().is_symlink() {
            return if index + 1 == segments.len() {
                PathKind::Symlink
            } else {
                PathKind::ReparsePoint
            };
        }
        if is_reparse_point(&metadata) {
            return PathKind::ReparsePoint;
        }
        if index + 1 == segments.len() {
            return if metadata.is_file() {
                PathKind::Regular
            } else if metadata.is_dir() {
                PathKind::Directory
            } else {
                PathKind::Unknown
            };
        }
        if !metadata.is_dir() {
            return PathKind::Unknown;
        }
    }
    PathKind::Unknown
}

fn change_value(change: &ChangeRecord) -> CanonicalValue {
    let path_value = |path: &Option<PathEvidence>| {
        path.as_ref().map_or(CanonicalValue::Null, |evidence| {
            CanonicalValue::Object(vec![
                (
                    "path".to_owned(),
                    CanonicalValue::String(evidence.path.clone()),
                ),
                (
                    "kind".to_owned(),
                    CanonicalValue::String(evidence.kind.as_str().to_owned()),
                ),
            ])
        })
    };
    CanonicalValue::Object(vec![
        (
            "operation".to_owned(),
            CanonicalValue::String(change.operation.as_str().to_owned()),
        ),
        ("source".to_owned(), path_value(&change.source)),
        ("destination".to_owned(), path_value(&change.destination)),
        (
            "states".to_owned(),
            CanonicalValue::Array(
                change
                    .states
                    .iter()
                    .map(|state| CanonicalValue::String(state.as_str().to_owned()))
                    .collect(),
            ),
        ),
        (
            "git_status".to_owned(),
            CanonicalValue::String(change.git_status.clone()),
        ),
    ])
}

fn change_manifest_digest(
    base_commit: &CommitId,
    head_commit: &CommitId,
    changes: &[ChangeRecord],
) -> Result<ContentDigest, WorkspaceError> {
    let domain = HashDomain::new("lattice.workspace-git.change-manifest", "2.0")
        .map_err(|_| WorkspaceError::EvidenceHashFailed)?;
    let value = CanonicalValue::Object(vec![
        (
            "base_commit".to_owned(),
            CanonicalValue::String(base_commit.as_str().to_owned()),
        ),
        (
            "head_commit".to_owned(),
            CanonicalValue::String(head_commit.as_str().to_owned()),
        ),
        (
            "changes".to_owned(),
            CanonicalValue::Array(changes.iter().map(change_value).collect()),
        ),
    ]);
    ContentDigest::from_sha256(
        canonical_sha256(&domain, &value)
            .map_err(|_| WorkspaceError::EvidenceHashFailed)?
            .to_hex(),
    )
    .map_err(|_| WorkspaceError::EvidenceHashFailed)
}

fn merge_readiness_digest(
    target_commit: &CommitId,
    feature_commit: &CommitId,
    result_tree: &str,
    outcome: IntegrationOutcome,
    conflicts: &[String],
) -> Result<ContentDigest, WorkspaceError> {
    let domain = HashDomain::new("lattice.workspace-git.merge-readiness", "2.0")
        .map_err(|_| WorkspaceError::EvidenceHashFailed)?;
    let value = CanonicalValue::Object(vec![
        (
            "target_commit".to_owned(),
            CanonicalValue::String(target_commit.as_str().to_owned()),
        ),
        (
            "feature_commit".to_owned(),
            CanonicalValue::String(feature_commit.as_str().to_owned()),
        ),
        (
            "result_tree".to_owned(),
            CanonicalValue::String(result_tree.to_owned()),
        ),
        (
            "outcome".to_owned(),
            CanonicalValue::String(outcome.as_str().to_owned()),
        ),
        (
            "conflicts".to_owned(),
            CanonicalValue::Array(
                conflicts
                    .iter()
                    .cloned()
                    .map(CanonicalValue::String)
                    .collect(),
            ),
        ),
    ]);
    ContentDigest::from_sha256(
        canonical_sha256(&domain, &value)
            .map_err(|_| WorkspaceError::EvidenceHashFailed)?
            .to_hex(),
    )
    .map_err(|_| WorkspaceError::EvidenceHashFailed)
}

fn parse_merge_tree_output(output: &str) -> Result<(String, Vec<String>), WorkspaceError> {
    let separator = output
        .find(['\n', '\0'])
        .ok_or(WorkspaceError::InvalidChangeEvidence)?;
    let result_tree = output[..separator].trim_end_matches('\r');
    CommitId::new(result_tree).map_err(|_| WorkspaceError::InvalidChangeEvidence)?;
    let remaining = &output[separator + 1..];
    let mut conflicts = Vec::new();
    for path in remaining.split('\0') {
        if path.is_empty() {
            continue;
        }
        conflicts.push(normalized_repository_path(path)?);
    }
    conflicts.sort();
    conflicts.dedup();
    Ok((result_tree.to_owned(), conflicts))
}

/// Process-local handle for one LATTICE-created worktree.
///
/// The held guard file is defense in depth only and grants no authority.
#[derive(Debug)]
pub struct OwnedWorktree {
    path: PathBuf,
    branch_ref: String,
    base_commit: CommitId,
    binding: OwnedBinding,
    guard_path: PathBuf,
    guard_file: Option<File>,
    removed: bool,
}

impl OwnedWorktree {
    /// Returns the exact task-worktree path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the fully qualified local task branch.
    #[must_use]
    pub fn branch_ref(&self) -> &str {
        &self.branch_ref
    }

    /// Returns the exact base commit.
    #[must_use]
    pub const fn base_commit(&self) -> &CommitId {
        &self.base_commit
    }
}

/// Workspace Git 2.0 service over one injected argument-array executor.
#[derive(Debug)]
pub struct GitWorkspace<E> {
    workspace_root: PathBuf,
    guards_root: PathBuf,
    hooks_root: PathBuf,
    empty_config: PathBuf,
    executor: E,
}

impl<E: GitExecutor> GitWorkspace<E> {
    /// Constructs one boundary rooted in an existing LATTICE-owned directory.
    ///
    /// # Errors
    ///
    /// Linked, reparse-point, relative, missing, or ambiguous roots fail
    /// closed before any child process.
    pub fn new(workspace_root: impl AsRef<Path>, executor: E) -> Result<Self, WorkspaceError> {
        let workspace_root = ensure_real_directory(workspace_root.as_ref())?;
        let guards_root = workspace_root.join(".lattice-guards");
        let hooks_root = workspace_root.join(".lattice-hooks-empty");
        fs::create_dir(&guards_root)
            .or_else(|error| {
                (error.kind() == io::ErrorKind::AlreadyExists)
                    .then_some(())
                    .ok_or(error)
            })
            .map_err(|_| WorkspaceError::WorkspaceRootUnsafe)?;
        fs::create_dir(&hooks_root)
            .or_else(|error| {
                (error.kind() == io::ErrorKind::AlreadyExists)
                    .then_some(())
                    .ok_or(error)
            })
            .map_err(|_| WorkspaceError::WorkspaceRootUnsafe)?;
        ensure_real_directory(&guards_root)?;
        ensure_real_directory(&hooks_root)?;
        if fs::read_dir(&hooks_root)
            .map_err(|_| WorkspaceError::WorkspaceRootUnsafe)?
            .next()
            .is_some()
        {
            return Err(WorkspaceError::WorkspaceRootUnsafe);
        }
        let empty_config = workspace_root.join(".lattice-empty-gitconfig");
        let mut config = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&empty_config)
            .or_else(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    OpenOptions::new().read(true).open(&empty_config)
                } else {
                    Err(error)
                }
            })
            .map_err(|_| WorkspaceError::WorkspaceRootUnsafe)?;
        config
            .flush()
            .map_err(|_| WorkspaceError::WorkspaceRootUnsafe)?;
        let metadata =
            fs::symlink_metadata(&empty_config).map_err(|_| WorkspaceError::WorkspaceRootUnsafe)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(WorkspaceError::WorkspaceRootUnsafe);
        }
        Ok(Self {
            workspace_root,
            guards_root,
            hooks_root,
            empty_config,
            executor,
        })
    }

    fn git(
        &self,
        cwd: &Path,
        args: &[&OsStr],
        allowed_exit_codes: &[i32],
    ) -> Result<GitOutput, WorkspaceError> {
        let mut protected = vec![
            OsString::from("--no-pager"),
            OsString::from("-c"),
            OsString::from(format!("core.hooksPath={}", self.hooks_root.display())),
            OsString::from("-c"),
            OsString::from("core.fsmonitor=false"),
            OsString::from("-c"),
            OsString::from("diff.external="),
        ];
        protected.extend(args.iter().map(|argument| (*argument).to_os_string()));
        let request = GitRequest {
            cwd: cwd.to_owned(),
            args: protected,
            empty_config: self.empty_config.clone(),
        };
        let output = self.executor.execute(&request)?;
        if allowed_exit_codes.contains(&output.exit_code()) {
            Ok(output)
        } else {
            Err(WorkspaceError::GitCommandFailed {
                exit_code: output.exit_code(),
                args: request
                    .args()
                    .iter()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect(),
                stderr: output.stderr().to_owned(),
            })
        }
    }

    fn repository_root(&self, authority: &WorkspaceAuthority) -> Result<PathBuf, WorkspaceError> {
        let raw = Path::new(authority.repository_root());
        let root =
            ensure_real_directory(raw).map_err(|_| WorkspaceError::RepositoryIdentityMismatch)?;
        if path_contained_by(&root, &self.workspace_root)
            || path_contained_by(&self.workspace_root, &root)
        {
            return Err(WorkspaceError::RepositoryIdentityMismatch);
        }
        let top_level = self.git(
            &root,
            &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
            &[0],
        )?;
        let observed = fs::canonicalize(top_level.stdout().trim())
            .map(native_canonical_path)
            .map_err(|_| WorkspaceError::RepositoryIdentityMismatch)?;
        if path_key(&observed) != path_key(&root) {
            return Err(WorkspaceError::RepositoryIdentityMismatch);
        }
        Ok(root)
    }

    fn reject_unsafe_configuration(&self, repository: &Path) -> Result<(), WorkspaceError> {
        let unsafe_pattern = concat!(
            "^(include\\.|includeif\\.|filter\\..*\\.(clean|smudge|process|required)$|",
            "merge\\..*\\.driver$|diff\\..*\\.(command|textconv)$|",
            "core\\.(hookspath|fsmonitor|attributesfile)$|extensions\\.worktreeconfig$)"
        );
        let output = self.git(
            repository,
            &[
                OsStr::new("config"),
                OsStr::new("--local"),
                OsStr::new("--null"),
                OsStr::new("--name-only"),
                OsStr::new("--get-regexp"),
                OsStr::new(unsafe_pattern),
            ],
            &[0, 1],
        )?;
        if output.exit_code() == 0 && !output.stdout().is_empty() {
            return Err(WorkspaceError::UnsafeGitConfiguration);
        }
        Ok(())
    }

    fn verify_owned_worktree(
        &self,
        authority: &WorkspaceAuthority,
        owned: &OwnedWorktree,
    ) -> Result<PathBuf, WorkspaceError> {
        let expected = OwnedBinding::from_authority(authority);
        if expected != owned.binding || owned.removed || owned.guard_file.is_none() {
            return Err(WorkspaceError::WorktreeIdentityMismatch);
        }
        let repository = self.repository_root(authority)?;
        self.reject_unsafe_configuration(&repository)?;
        let target = ensure_real_directory(&owned.path)
            .map_err(|_| WorkspaceError::WorktreeIdentityMismatch)?;
        if !path_contained_by(&self.workspace_root, &target) {
            return Err(WorkspaceError::WorktreeIdentityMismatch);
        }
        let guard_metadata = fs::symlink_metadata(&owned.guard_path)
            .map_err(|_| WorkspaceError::WorktreeIdentityMismatch)?;
        if !guard_metadata.is_file()
            || guard_metadata.file_type().is_symlink()
            || is_reparse_point(&guard_metadata)
            || fs::read(&owned.guard_path).map_err(|_| WorkspaceError::WorktreeIdentityMismatch)?
                != b"LATTICE_WORKSPACE_GUARD_V2_NON_AUTHORITATIVE\n"
        {
            return Err(WorkspaceError::WorktreeIdentityMismatch);
        }
        let top_level = self.git(
            &target,
            &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
            &[0],
        )?;
        let branch = self.git(
            &target,
            &[
                OsStr::new("symbolic-ref"),
                OsStr::new("--quiet"),
                OsStr::new("HEAD"),
            ],
            &[0],
        )?;
        let target_common = self.git(
            &target,
            &[OsStr::new("rev-parse"), OsStr::new("--git-common-dir")],
            &[0],
        )?;
        let repository_common = self.git(
            &repository,
            &[OsStr::new("rev-parse"), OsStr::new("--git-common-dir")],
            &[0],
        )?;
        let resolve_common = |cwd: &Path, value: &str| {
            let candidate = Path::new(value.trim());
            let candidate = if candidate.is_absolute() {
                candidate.to_owned()
            } else {
                cwd.join(candidate)
            };
            fs::canonicalize(candidate).map(native_canonical_path)
        };
        let target_common = resolve_common(&target, target_common.stdout())
            .map_err(|_| WorkspaceError::WorktreeIdentityMismatch)?;
        let repository_common = resolve_common(&repository, repository_common.stdout())
            .map_err(|_| WorkspaceError::WorktreeIdentityMismatch)?;
        let base_ancestor = self.git(
            &target,
            &[
                OsStr::new("merge-base"),
                OsStr::new("--is-ancestor"),
                OsStr::new(owned.base_commit.as_str()),
                OsStr::new("HEAD"),
            ],
            &[0, 1],
        )?;
        let top_level = fs::canonicalize(top_level.stdout().trim())
            .map(native_canonical_path)
            .map_err(|_| WorkspaceError::WorktreeIdentityMismatch)?;
        if path_key(&top_level) != path_key(&target)
            || branch.stdout().trim() != owned.branch_ref
            || path_key(&target_common) != path_key(&repository_common)
            || base_ancestor.exit_code() != 0
        {
            return Err(WorkspaceError::WorktreeIdentityMismatch);
        }
        Ok(target)
    }

    /// Analyzes two exact commits without changing a checkout or reference.
    ///
    /// The resulting tree object and conflict list are detection evidence only;
    /// this method does not authorize or perform a merge.
    ///
    /// # Errors
    ///
    /// Stale repository identity, unsafe Git configuration, unknown commits,
    /// malformed merge output, or evidence hashing failure fails closed.
    pub fn analyze_integration(
        &self,
        authority: &WorkspaceAuthority,
        target_commit: &str,
        feature_commit: &str,
    ) -> Result<MergeReadiness, WorkspaceError> {
        let target_commit = CommitId::new(target_commit)?;
        let feature_commit = CommitId::new(feature_commit)?;
        let repository = self.repository_root(authority)?;
        self.reject_unsafe_configuration(&repository)?;
        for commit in [&target_commit, &feature_commit] {
            let verified = self.git(
                &repository,
                &[
                    OsStr::new("rev-parse"),
                    OsStr::new("--verify"),
                    OsStr::new(&format!("{}^{{commit}}", commit.as_str())),
                ],
                &[0],
            )?;
            if verified.stdout().trim() != commit.as_str() {
                return Err(WorkspaceError::BaseCommitMismatch);
            }
        }
        let output = self.git(
            &repository,
            &[
                OsStr::new("merge-tree"),
                OsStr::new("--write-tree"),
                OsStr::new("--name-only"),
                OsStr::new("-z"),
                OsStr::new("--no-messages"),
                OsStr::new(target_commit.as_str()),
                OsStr::new(feature_commit.as_str()),
            ],
            &[0, 1],
        )?;
        let (result_tree, conflicts) = parse_merge_tree_output(output.stdout())?;
        let outcome = match output.exit_code() {
            0 if conflicts.is_empty() => IntegrationOutcome::Clean,
            1 if !conflicts.is_empty() => IntegrationOutcome::Conflict,
            _ => return Err(WorkspaceError::InvalidChangeEvidence),
        };
        let digest = merge_readiness_digest(
            &target_commit,
            &feature_commit,
            &result_tree,
            outcome,
            &conflicts,
        )?;
        Ok(MergeReadiness {
            target_commit,
            feature_commit,
            result_tree,
            outcome,
            conflicts,
            digest,
        })
    }

    /// Removes one exact LATTICE-owned clean worktree without forcing or
    /// deleting its branch.
    ///
    /// # Errors
    ///
    /// Stale identity, any tracked or untracked change, or an ambiguous
    /// post-effect state fails closed. Dirty worktrees are retained intact.
    pub fn remove_owned_worktree(
        &self,
        authority: &WorkspaceAuthority,
        owned: &mut OwnedWorktree,
    ) -> Result<(), WorkspaceError> {
        let target = self.verify_owned_worktree(authority, owned)?;
        let status = self.git(
            &target,
            &[
                OsStr::new("status"),
                OsStr::new("--porcelain=v1"),
                OsStr::new("-z"),
                OsStr::new("--untracked-files=all"),
            ],
            &[0],
        )?;
        if !status.stdout().is_empty() {
            return Err(WorkspaceError::WorktreeDirty);
        }
        let repository = self.repository_root(authority)?;
        self.git(
            &repository,
            &[
                OsStr::new("worktree"),
                OsStr::new("remove"),
                target.as_os_str(),
            ],
            &[0],
        )
        .map_err(|_| WorkspaceError::ReconciliationRequired)?;
        if fs::symlink_metadata(&target).is_ok() {
            return Err(WorkspaceError::ReconciliationRequired);
        }
        drop(owned.guard_file.take());
        fs::remove_file(&owned.guard_path).map_err(|_| WorkspaceError::ReconciliationRequired)?;
        if fs::symlink_metadata(&owned.guard_path).is_ok() {
            return Err(WorkspaceError::ReconciliationRequired);
        }
        owned.removed = true;
        Ok(())
    }

    /// Produces one stable complete changed-path manifest for the exact owned
    /// worktree and current authority.
    ///
    /// # Errors
    ///
    /// Stale authority, linked worktrees, malformed Git output, path escape,
    /// or canonical hashing failure fails closed without mutation.
    // Kept as one linear evidence choreography so reviewers can audit the
    // complete Git observation set and precedence without hidden mutation.
    #[allow(clippy::too_many_lines)]
    pub fn changed_manifest(
        &self,
        authority: &WorkspaceAuthority,
        owned: &OwnedWorktree,
    ) -> Result<ChangeManifest, WorkspaceError> {
        let target = self.verify_owned_worktree(authority, owned)?;
        let head = self.git(
            &target,
            &[OsStr::new("rev-parse"), OsStr::new("HEAD")],
            &[0],
        )?;
        let head_commit = CommitId::new(head.stdout().trim())?;
        let total = self.git(
            &target,
            &[
                OsStr::new("diff"),
                OsStr::new("--name-status"),
                OsStr::new("-z"),
                OsStr::new("-M"),
                OsStr::new(owned.base_commit.as_str()),
                OsStr::new("--"),
            ],
            &[0],
        )?;
        let committed = self.git(
            &target,
            &[
                OsStr::new("diff"),
                OsStr::new("--name-only"),
                OsStr::new("-z"),
                OsStr::new(owned.base_commit.as_str()),
                OsStr::new("HEAD"),
                OsStr::new("--"),
            ],
            &[0],
        )?;
        let staged = self.git(
            &target,
            &[
                OsStr::new("diff"),
                OsStr::new("--cached"),
                OsStr::new("--name-only"),
                OsStr::new("-z"),
                OsStr::new("HEAD"),
                OsStr::new("--"),
            ],
            &[0],
        )?;
        let unstaged = self.git(
            &target,
            &[
                OsStr::new("diff"),
                OsStr::new("--name-only"),
                OsStr::new("-z"),
                OsStr::new("--"),
            ],
            &[0],
        )?;
        let unmerged = self.git(
            &target,
            &[
                OsStr::new("diff"),
                OsStr::new("--name-only"),
                OsStr::new("--diff-filter=U"),
                OsStr::new("-z"),
                OsStr::new("--"),
            ],
            &[0],
        )?;
        let untracked = self.git(
            &target,
            &[
                OsStr::new("ls-files"),
                OsStr::new("--others"),
                OsStr::new("--exclude-standard"),
                OsStr::new("-z"),
            ],
            &[0],
        )?;
        let ignored = self.git(
            &target,
            &[
                OsStr::new("ls-files"),
                OsStr::new("--others"),
                OsStr::new("--ignored"),
                OsStr::new("--exclude-standard"),
                OsStr::new("-z"),
            ],
            &[0],
        )?;
        let base_modes = self.git(
            &target,
            &[
                OsStr::new("ls-tree"),
                OsStr::new("-r"),
                OsStr::new("-z"),
                OsStr::new(owned.base_commit.as_str()),
                OsStr::new("--"),
            ],
            &[0],
        )?;
        let index_modes = self.git(
            &target,
            &[OsStr::new("ls-files"), OsStr::new("-s"), OsStr::new("-z")],
            &[0],
        )?;

        let committed = parse_nul_paths(committed.stdout())?;
        let staged = parse_nul_paths(staged.stdout())?;
        let unstaged = parse_nul_paths(unstaged.stdout())?;
        let unmerged = parse_nul_paths(unmerged.stdout())?;
        let untracked = parse_nul_paths(untracked.stdout())?;
        let ignored = parse_nul_paths(ignored.stdout())?;
        let base_modes = parse_tree_modes(base_modes.stdout())?;
        let index_modes = parse_index_modes(index_modes.stdout())?;
        let mut raw = BTreeMap::new();
        for change in parse_name_status(total.stdout())? {
            raw.insert(change.key.clone(), change);
        }
        for path in &unmerged {
            raw.retain(|key, _| {
                key.source.as_ref() != Some(path) && key.destination.as_ref() != Some(path)
            });
            let key = RawChangeKey {
                operation: ChangeOperation::Unmerged,
                source: Some(path.clone()),
                destination: Some(path.clone()),
            };
            raw.insert(
                key.clone(),
                RawChange {
                    key,
                    git_status: "U".to_owned(),
                    states: BTreeSet::new(),
                },
            );
        }
        for change in raw.values_mut() {
            let paths = change
                .key
                .source
                .iter()
                .chain(change.key.destination.iter());
            for path in paths {
                if committed.contains(path) {
                    change.states.insert(ChangeState::Committed);
                }
                if staged.contains(path) {
                    change.states.insert(ChangeState::Staged);
                }
                if unstaged.contains(path) {
                    change.states.insert(ChangeState::Unstaged);
                }
            }
            if change.key.operation == ChangeOperation::Unmerged && change.states.is_empty() {
                change.states.insert(ChangeState::Unstaged);
            }
        }
        for (paths, state, status) in [
            (untracked, ChangeState::Untracked, "??"),
            (ignored, ChangeState::Ignored, "!!"),
        ] {
            for path in paths {
                let key = RawChangeKey {
                    operation: ChangeOperation::Create,
                    source: None,
                    destination: Some(path),
                };
                raw.entry(key.clone()).or_insert_with(|| RawChange {
                    key,
                    git_status: status.to_owned(),
                    states: BTreeSet::from([state]),
                });
            }
        }
        let mut changes = Vec::with_capacity(raw.len());
        for change in raw.into_values() {
            let source = change
                .key
                .source
                .map(|path| PathEvidence::new(path.clone(), mode_kind(base_modes.get(&path))))
                .transpose()?;
            let destination = change
                .key
                .destination
                .map(|path| {
                    let current = current_path_kind(&target, &path);
                    let index_kind = mode_kind(index_modes.get(&path));
                    let kind = match index_kind {
                        PathKind::Symlink | PathKind::GitLink => index_kind,
                        _ if current == PathKind::Missing => index_kind,
                        _ => current,
                    };
                    PathEvidence::new(path, kind)
                })
                .transpose()?;
            changes.push(ChangeRecord::new(
                change.key.operation,
                source,
                destination,
                change.states.into_iter().collect(),
                change.git_status,
            )?);
        }
        ChangeManifest::from_normalized(owned.base_commit.as_str(), head_commit.as_str(), changes)
    }

    /// Creates one exact-base task worktree after all authority and safety
    /// preflights.
    ///
    /// # Errors
    ///
    /// Unknown, dirty, linked, configured, existing, or post-effect ambiguous
    /// state fails closed. Post-effect failure is reconciliation-required and
    /// is not automatically reset or cleaned.
    // Kept linear to make the before/after effect boundary and all
    // reconciliation-required exits visible in one audit surface.
    #[allow(clippy::too_many_lines)]
    pub fn create_worktree(
        &self,
        authority: &WorkspaceAuthority,
        base_commit: &str,
    ) -> Result<OwnedWorktree, WorkspaceError> {
        let base_commit = CommitId::new(base_commit)?;
        let repository = self.repository_root(authority)?;
        self.reject_unsafe_configuration(&repository)?;
        let verified = self.git(
            &repository,
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new(&format!("{}^{{commit}}", base_commit.as_str())),
            ],
            &[0],
        )?;
        if verified.stdout().trim() != base_commit.as_str() {
            return Err(WorkspaceError::BaseCommitMismatch);
        }
        let status = self.git(
            &repository,
            &[
                OsStr::new("status"),
                OsStr::new("--porcelain=v1"),
                OsStr::new("-z"),
                OsStr::new("--untracked-files=all"),
            ],
            &[0],
        )?;
        if !status.stdout().is_empty() {
            return Err(WorkspaceError::RepositoryDirty);
        }
        let slug = authority_slug(authority)?;
        let target = self.workspace_root.join(&slug);
        let guard_path = self.guards_root.join(format!("{slug}.guard"));
        if target.exists() || guard_path.exists() {
            return Err(WorkspaceError::WorktreeTargetExists);
        }
        let branch_ref = format!("refs/heads/lattice/{slug}");
        let branch_short = branch_ref
            .strip_prefix("refs/heads/")
            .ok_or(WorkspaceError::WorktreeTargetExists)?;
        let branch_status = self.git(
            &repository,
            &[
                OsStr::new("show-ref"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new(&branch_ref),
            ],
            &[0, 1],
        )?;
        if branch_status.exit_code() == 0 {
            return Err(WorkspaceError::WorktreeTargetExists);
        }
        let mut guard_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&guard_path)
            .map_err(|_| WorkspaceError::WorktreeTargetExists)?;
        guard_file
            .write_all(b"LATTICE_WORKSPACE_GUARD_V2_NON_AUTHORITATIVE\n")
            .and_then(|()| guard_file.sync_data())
            .map_err(|_| WorkspaceError::WorktreeTargetExists)?;

        self.git(
            &repository,
            &[
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("-b"),
                OsStr::new(branch_short),
                target.as_os_str(),
                OsStr::new(base_commit.as_str()),
            ],
            &[0],
        )
        .map_err(|_| WorkspaceError::ReconciliationRequired)?;

        let actual_head = self
            .git(
                &target,
                &[OsStr::new("rev-parse"), OsStr::new("HEAD")],
                &[0],
            )
            .map_err(|_| WorkspaceError::ReconciliationRequired)?;
        let actual_branch = self
            .git(
                &target,
                &[
                    OsStr::new("symbolic-ref"),
                    OsStr::new("--quiet"),
                    OsStr::new("HEAD"),
                ],
                &[0],
            )
            .map_err(|_| WorkspaceError::ReconciliationRequired)?;
        let created_status = self
            .git(
                &target,
                &[
                    OsStr::new("status"),
                    OsStr::new("--porcelain=v1"),
                    OsStr::new("-z"),
                    OsStr::new("--untracked-files=all"),
                ],
                &[0],
            )
            .map_err(|_| WorkspaceError::ReconciliationRequired)?;
        let target_real = fs::canonicalize(&target)
            .map(native_canonical_path)
            .map_err(|_| WorkspaceError::ReconciliationRequired)?;
        if actual_head.stdout().trim() != base_commit.as_str()
            || actual_branch.stdout().trim() != branch_ref
            || !created_status.stdout().is_empty()
            || !path_contained_by(&self.workspace_root, &target_real)
        {
            return Err(WorkspaceError::ReconciliationRequired);
        }
        Ok(OwnedWorktree {
            path: target_real,
            branch_ref,
            base_commit,
            binding: OwnedBinding::from_authority(authority),
            guard_path,
            guard_file: Some(guard_file),
            removed: false,
        })
    }
}
