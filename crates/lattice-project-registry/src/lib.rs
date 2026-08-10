//! Pure Project Registry domain and deterministic fake owner boundary.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use lattice_cjson::{
    CanonicalError, CanonicalValue, HashDomain, canonical_sha256, canonicalize, normalize_nfc,
};
use lattice_contracts::{
    CONTRACT_VERSION, ContentDigest, ContractError, GitRefIdentity, PROJECT_AUTHORITY_PRODUCER_ID,
    PROJECT_AUTHORITY_PRODUCER_VERSION, ProjectAuthorityHead, ProjectAuthorityReceipt,
    ProjectClass, ProjectId, ProjectLifecycle, ProjectSnapshotId, RuntimeKind,
};

/// Maximum current project projections retained by Registry 1.2.
pub const MAX_REGISTRY_PROJECTS: usize = 4_096;
/// Maximum first-seen terminal command records retained by Registry 1.2.
pub const MAX_REGISTRY_COMMANDS: usize = 65_536;
/// Maximum canonical logical-retained-state bytes retained by Registry 1.2.
pub const MAX_REGISTRY_RETAINED_BYTES: u64 = 67_108_864;
/// Maximum UTF-8 bytes in one already-NFC canonical root.
pub const MAX_CANONICAL_ROOT_BYTES: usize = 131_072;

/// Failure at the pure Registry contract boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    /// A command identifier is empty, non-canonical, or oversized.
    InvalidCommandId,
    /// A canonical-root observation is empty, non-canonical, or NUL-bearing.
    InvalidCanonicalRoot,
    /// A caller supplied raw text that is not already Unicode NFC.
    NonCanonicalText {
        /// Stable field name whose raw representation was rejected.
        field: &'static str,
    },
    /// A previously used command ID was replayed with another request digest.
    CommandIdReuse,
    /// An untrusted Registry snapshot failed internal replay verification.
    CorruptSnapshot,
    /// A verified snapshot disagrees with an independently retained checkpoint.
    CheckpointMismatch,
    /// A first-seen command would exceed a versioned Registry limit.
    CapacityExceeded,
    /// The non-wrapping global command ordinal cannot advance.
    CommandOrdinalOverflow,
    /// A shared contract value could not be constructed.
    Contract(ContractError),
    /// Canonical receipt/request bytes could not be produced.
    Canonical(CanonicalError),
}

impl RegistryError {
    /// Returns a stable machine-facing failure code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidCommandId => "REGISTRY_INVALID_COMMAND_ID",
            Self::InvalidCanonicalRoot => "REGISTRY_INVALID_CANONICAL_ROOT",
            Self::NonCanonicalText { .. } => "REGISTRY_NON_CANONICAL_TEXT",
            Self::CommandIdReuse => "REGISTRY_COMMAND_ID_REUSE",
            Self::CorruptSnapshot => "REGISTRY_CORRUPT_SNAPSHOT",
            Self::CheckpointMismatch => "REGISTRY_CHECKPOINT_MISMATCH",
            Self::CapacityExceeded => "REGISTRY_CAPACITY_EXCEEDED",
            Self::CommandOrdinalOverflow => "REGISTRY_COMMAND_ORDINAL_OVERFLOW",
            Self::Contract(_) => "REGISTRY_CONTRACT_INVALID",
            Self::Canonical(_) => "REGISTRY_CANONICAL_ENCODING_FAILED",
        }
    }
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCommandId => formatter.write_str("invalid Registry command_id"),
            Self::InvalidCanonicalRoot => {
                formatter.write_str("canonical_root must be exact, non-empty, and NUL-free")
            }
            Self::NonCanonicalText { field } => {
                write!(formatter, "{field} must already be Unicode NFC")
            }
            Self::CommandIdReuse => {
                formatter.write_str("command_id was already used for another request")
            }
            Self::CorruptSnapshot => {
                formatter.write_str("untrusted Registry snapshot failed replay verification")
            }
            Self::CheckpointMismatch => formatter.write_str("Registry checkpoint mismatch"),
            Self::CapacityExceeded => formatter.write_str("Registry retained-state limit exceeded"),
            Self::CommandOrdinalOverflow => {
                formatter.write_str("Registry command ordinal cannot advance")
            }
            Self::Contract(error) => write!(formatter, "shared contract rejected value: {error}"),
            Self::Canonical(error) => {
                write!(formatter, "Registry canonical encoding failed: {error}")
            }
        }
    }
}

impl Error for RegistryError {}

impl From<ContractError> for RegistryError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

impl From<CanonicalError> for RegistryError {
    fn from(value: CanonicalError) -> Self {
        Self::Canonical(value)
    }
}

/// Stable idempotency identity for one Registry command.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CommandId(String);

impl CommandId {
    /// Validates a command identifier without hidden normalization.
    ///
    /// # Errors
    ///
    /// Rejects empty, whitespace-padded, NUL-bearing, or oversized values.
    pub fn new(value: impl Into<String>) -> Result<Self, RegistryError> {
        let value = value.into();
        if value.is_empty() || value.trim() != value || value.contains('\0') || value.len() > 128 {
            Err(RegistryError::InvalidCommandId)
        } else if normalize_nfc(&value) != value {
            Err(RegistryError::NonCanonicalText {
                field: "command_id",
            })
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the exact command identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Immutable output of a future filesystem/repository inspection port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryObservation {
    canonical_root: String,
    canonical_root_identity_digest: ContentDigest,
    repository_identity_digest: ContentDigest,
    file_identity_digest: ContentDigest,
    primary_branch: GitRefIdentity,
    digest: ContentDigest,
}

impl RepositoryObservation {
    /// Constructs and hashes one already-inspected repository observation.
    ///
    /// # Errors
    ///
    /// Rejects an empty, whitespace-padded, or NUL-bearing canonical root and
    /// any canonical-encoding failure.
    pub fn new(
        canonical_root: impl Into<String>,
        canonical_root_identity_digest: ContentDigest,
        repository_identity_digest: ContentDigest,
        file_identity_digest: ContentDigest,
        primary_branch: GitRefIdentity,
    ) -> Result<Self, RegistryError> {
        let canonical_root = canonical_root.into();
        if canonical_root.is_empty()
            || canonical_root.trim() != canonical_root
            || canonical_root.contains('\0')
            || canonical_root.len() > MAX_CANONICAL_ROOT_BYTES
        {
            return Err(RegistryError::InvalidCanonicalRoot);
        }
        if normalize_nfc(&canonical_root) != canonical_root {
            return Err(RegistryError::NonCanonicalText {
                field: "canonical_root",
            });
        }
        if normalize_nfc(primary_branch.reference()) != primary_branch.reference() {
            return Err(RegistryError::NonCanonicalText {
                field: "primary_branch",
            });
        }
        let digest = registry_digest(
            "lattice.project-registry.repository-observation",
            &observation_value(
                &canonical_root,
                &canonical_root_identity_digest,
                &repository_identity_digest,
                &file_identity_digest,
                &primary_branch,
            ),
        )?;
        Ok(Self {
            canonical_root,
            canonical_root_identity_digest,
            repository_identity_digest,
            file_identity_digest,
            primary_branch,
            digest,
        })
    }

    /// Returns the canonical display root supplied by the inspector.
    #[must_use]
    pub fn canonical_root(&self) -> &str {
        &self.canonical_root
    }

    /// Returns the physical canonical-root comparison identity.
    #[must_use]
    pub const fn canonical_root_identity_digest(&self) -> &ContentDigest {
        &self.canonical_root_identity_digest
    }

    /// Returns the repository identity digest.
    #[must_use]
    pub const fn repository_identity_digest(&self) -> &ContentDigest {
        &self.repository_identity_digest
    }

    /// Returns the filesystem/file identity digest.
    #[must_use]
    pub const fn file_identity_digest(&self) -> &ContentDigest {
        &self.file_identity_digest
    }

    /// Returns the physical primary local-ref identity.
    #[must_use]
    pub const fn primary_branch(&self) -> &GitRefIdentity {
        &self.primary_branch
    }

    /// Returns the content digest of the complete observation.
    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

/// Identity dimension that collided with an existing registration.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum IdentityDimension {
    /// The canonical Project ID already exists.
    ProjectId,
    /// The physical canonical-root identity already exists.
    CanonicalRoot,
    /// The repository identity already exists.
    Repository,
    /// The filesystem/file identity already exists.
    File,
}

impl IdentityDimension {
    /// Returns the stable persistence-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectId => "PROJECT_ID",
            Self::CanonicalRoot => "CANONICAL_ROOT",
            Self::Repository => "REPOSITORY",
            Self::File => "FILE",
        }
    }
}

/// Exact identity dimension that differs from the accepted Registry record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityDrift {
    /// Canonical root text or physical root identity changed.
    CanonicalRoot,
    /// Repository identity changed.
    Repository,
    /// Filesystem/file identity changed.
    File,
    /// Canonical primary-ref text changed.
    PrimaryRefName,
    /// Physical primary-ref storage identity changed.
    PrimaryRefStorage,
}

impl IdentityDrift {
    /// Returns the stable persistence-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalRoot => "CANONICAL_ROOT",
            Self::Repository => "REPOSITORY",
            Self::File => "FILE",
            Self::PrimaryRefName => "PRIMARY_REF_NAME",
            Self::PrimaryRefStorage => "PRIMARY_REF_STORAGE",
        }
    }
}

/// Exact reconciliation operation accepted by Registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconciliationDecision {
    /// Accept a root-only move while retaining repository/file/ref identity.
    AcceptMove,
    /// Accept a repository, file, or primary-ref identity change.
    AcceptIdentityChange,
    /// Reactivate an exact suspended identity.
    Reactivate,
}

impl ReconciliationDecision {
    /// Returns the stable persistence-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AcceptMove => "ACCEPT_MOVE",
            Self::AcceptIdentityChange => "ACCEPT_IDENTITY_CHANGE",
            Self::Reactivate => "REACTIVATE",
        }
    }
}

/// Stable terminal Registry denial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryDenial {
    /// Another registration already owns one immutable identity dimension.
    DuplicateIdentity {
        /// Exact dimension that collided.
        dimension: IdentityDimension,
        /// Existing owner of the identity.
        existing_project_id: ProjectId,
    },
    /// The requested project is not registered.
    UnknownProject,
    /// The supplied expected head is no longer current.
    StaleHead,
    /// The current lifecycle does not permit the requested command.
    LifecycleBlocked {
        /// Exact current lifecycle.
        lifecycle: ProjectLifecycle,
    },
    /// Reconciliation selected the wrong operation for the observed drift.
    ReconciliationDecisionMismatch {
        /// Operation required by the current Registry state.
        expected: ReconciliationDecision,
        /// Operation supplied by the caller.
        found: ReconciliationDecision,
    },
    /// Reconciliation attempted to replace the pending observed identity.
    PendingObservationMismatch,
    /// The non-wrapping Registry revision cannot advance.
    RevisionOverflow,
}

/// Terminal command outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryCommandOutcome {
    /// The command was applied or an exact observation was resolved.
    Applied,
    /// The command was safely denied without applying its requested mutation.
    Denied(RegistryDenial),
    /// The requested resolution was denied and Registry advanced to a
    /// non-active defensive authority so stale ACTIVE evidence cannot survive.
    Blocked(RegistryDenial),
}

/// One typed Registry command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryCommand {
    /// Register one previously unknown project identity.
    Register {
        command_id: CommandId,
        project_id: ProjectId,
        project_class: ProjectClass,
        observation: RepositoryObservation,
    },
    /// Resolve one exact observation against a current Registry head.
    Observe {
        command_id: CommandId,
        project_id: ProjectId,
        expected_head: ProjectAuthorityHead,
        observation: RepositoryObservation,
    },
    /// Suspend one exact current active project.
    Suspend {
        command_id: CommandId,
        project_id: ProjectId,
        expected_head: ProjectAuthorityHead,
        evidence_digest: ContentDigest,
    },
    /// Reconcile one exact suspended or drifted project head.
    Reconcile {
        command_id: CommandId,
        project_id: ProjectId,
        expected_head: ProjectAuthorityHead,
        observation: RepositoryObservation,
        decision: ReconciliationDecision,
        evidence_digest: ContentDigest,
    },
}

impl RegistryCommand {
    /// Constructs a registration command.
    #[must_use]
    pub const fn register(
        command_id: CommandId,
        project_id: ProjectId,
        project_class: ProjectClass,
        observation: RepositoryObservation,
    ) -> Self {
        Self::Register {
            command_id,
            project_id,
            project_class,
            observation,
        }
    }

    /// Constructs an exact resolve/observation command.
    #[must_use]
    pub const fn observe(
        command_id: CommandId,
        project_id: ProjectId,
        expected_head: ProjectAuthorityHead,
        observation: RepositoryObservation,
    ) -> Self {
        Self::Observe {
            command_id,
            project_id,
            expected_head,
            observation,
        }
    }

    /// Constructs an exact suspension command.
    #[must_use]
    pub const fn suspend(
        command_id: CommandId,
        project_id: ProjectId,
        expected_head: ProjectAuthorityHead,
        evidence_digest: ContentDigest,
    ) -> Self {
        Self::Suspend {
            command_id,
            project_id,
            expected_head,
            evidence_digest,
        }
    }

    /// Constructs an exact reconciliation command.
    #[must_use]
    pub const fn reconcile(
        command_id: CommandId,
        project_id: ProjectId,
        expected_head: ProjectAuthorityHead,
        observation: RepositoryObservation,
        decision: ReconciliationDecision,
        evidence_digest: ContentDigest,
    ) -> Self {
        Self::Reconcile {
            command_id,
            project_id,
            expected_head,
            observation,
            decision,
            evidence_digest,
        }
    }

    /// Returns the idempotency identity of this command.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        match self {
            Self::Register { command_id, .. }
            | Self::Observe { command_id, .. }
            | Self::Suspend { command_id, .. }
            | Self::Reconcile { command_id, .. } => command_id,
        }
    }

    /// Returns the exact semantic request commitment.
    ///
    /// # Errors
    ///
    /// Returns a canonical hashing failure if the fixed request cannot be
    /// represented by the Registry hash contract.
    pub fn request_digest(&self) -> Result<ContentDigest, RegistryError> {
        let value = match self {
            Self::Register {
                project_id,
                project_class,
                observation,
                ..
            } => CanonicalValue::Object(vec![
                text_entry("action", "REGISTER"),
                text_entry("project_id", project_id.as_str()),
                text_entry("project_class", project_class.as_str()),
                text_entry("observation_digest", observation.digest().as_str()),
            ]),
            Self::Observe {
                project_id,
                expected_head,
                observation,
                ..
            } => CanonicalValue::Object(vec![
                text_entry("action", "OBSERVE"),
                text_entry("project_id", project_id.as_str()),
                (
                    "expected_head".to_owned(),
                    authority_head_value(expected_head),
                ),
                text_entry("observation_digest", observation.digest().as_str()),
            ]),
            Self::Suspend {
                project_id,
                expected_head,
                evidence_digest,
                ..
            } => CanonicalValue::Object(vec![
                text_entry("action", "SUSPEND"),
                text_entry("project_id", project_id.as_str()),
                (
                    "expected_head".to_owned(),
                    authority_head_value(expected_head),
                ),
                text_entry("evidence_digest", evidence_digest.as_str()),
            ]),
            Self::Reconcile {
                project_id,
                expected_head,
                observation,
                decision,
                evidence_digest,
                ..
            } => CanonicalValue::Object(vec![
                text_entry("action", "RECONCILE"),
                text_entry("project_id", project_id.as_str()),
                (
                    "expected_head".to_owned(),
                    authority_head_value(expected_head),
                ),
                text_entry("observation_digest", observation.digest().as_str()),
                text_entry("decision", decision.as_str()),
                text_entry("evidence_digest", evidence_digest.as_str()),
            ]),
        };
        registry_digest("lattice.project-registry.command-request", &value)
    }
}

/// Immutable idempotent terminal receipt for one Registry command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryCommandReceipt {
    command_id: CommandId,
    request_digest: ContentDigest,
    before: Option<ProjectAuthorityHead>,
    after: Option<ProjectAuthorityHead>,
    outcome: RegistryCommandOutcome,
    drift: Vec<IdentityDrift>,
    authority: Option<ProjectAuthorityReceipt>,
    result_digest: ContentDigest,
}

impl RegistryCommandReceipt {
    /// Constructs one retained semantic receipt without claiming replay
    /// verification.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn from_retained(
        command_id: CommandId,
        request_digest: ContentDigest,
        before: Option<ProjectAuthorityHead>,
        after: Option<ProjectAuthorityHead>,
        outcome: RegistryCommandOutcome,
        drift: Vec<IdentityDrift>,
        authority: Option<ProjectAuthorityReceipt>,
        result_digest: ContentDigest,
    ) -> Self {
        Self {
            command_id,
            request_digest,
            before,
            after,
            outcome,
            drift,
            authority,
            result_digest,
        }
    }

    /// Returns the exact command ID.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    /// Returns the canonical semantic request digest.
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    /// Returns the authority head before execution, when one existed.
    #[must_use]
    pub const fn before(&self) -> Option<&ProjectAuthorityHead> {
        self.before.as_ref()
    }

    /// Returns the authority head after execution, when one exists.
    #[must_use]
    pub const fn after(&self) -> Option<&ProjectAuthorityHead> {
        self.after.as_ref()
    }

    /// Returns the typed terminal outcome.
    #[must_use]
    pub fn outcome(&self) -> RegistryCommandOutcome {
        self.outcome.clone()
    }

    /// Returns the stable ordered drift dimensions observed by this command.
    #[must_use]
    pub fn drift(&self) -> &[IdentityDrift] {
        &self.drift
    }

    /// Returns the resulting authority receipt, when the project exists.
    #[must_use]
    pub const fn authority(&self) -> Option<&ProjectAuthorityReceipt> {
        self.authority.as_ref()
    }

    /// Returns the digest of the complete terminal result.
    #[must_use]
    pub const fn result_digest(&self) -> &ContentDigest {
        &self.result_digest
    }
}

/// Reservation lifecycle represented in the global Registry projection.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RegistryReservationStatus {
    /// The identity belongs to the accepted current observation.
    Accepted,
    /// The identity belongs to an accepted pending reconciliation observation.
    Pending,
}

impl RegistryReservationStatus {
    /// Returns the stable persistence-facing value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "ACCEPTED",
            Self::Pending => "PENDING",
        }
    }
}

/// One normalized accepted or pending physical-identity reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryIdentityReservation {
    dimension: IdentityDimension,
    identity_digest: ContentDigest,
    status: RegistryReservationStatus,
    project_id: ProjectId,
}

impl Ord for RegistryIdentityReservation {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.dimension
            .cmp(&other.dimension)
            .then_with(|| {
                self.identity_digest
                    .as_str()
                    .cmp(other.identity_digest.as_str())
            })
            .then_with(|| self.status.cmp(&other.status))
            .then_with(|| self.project_id.cmp(&other.project_id))
    }
}

impl PartialOrd for RegistryIdentityReservation {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl RegistryIdentityReservation {
    /// Constructs one retained reservation row without claiming verification.
    #[must_use]
    pub const fn from_retained(
        dimension: IdentityDimension,
        identity_digest: ContentDigest,
        status: RegistryReservationStatus,
        project_id: ProjectId,
    ) -> Self {
        Self {
            dimension,
            identity_digest,
            status,
            project_id,
        }
    }

    /// Returns the physical identity dimension.
    #[must_use]
    pub const fn dimension(&self) -> IdentityDimension {
        self.dimension
    }

    /// Returns the physical identity digest.
    #[must_use]
    pub const fn identity_digest(&self) -> &ContentDigest {
        &self.identity_digest
    }

    /// Returns whether the identity is accepted or pending.
    #[must_use]
    pub const fn status(&self) -> RegistryReservationStatus {
        self.status
    }

    /// Returns the project that owns the reservation.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
}

/// Complete first-seen command record retained by a verified Registry state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryCommandRecord {
    ordinal: u64,
    command: RegistryCommand,
    receipt: RegistryCommandReceipt,
    base_checkpoint: RegistryCheckpoint,
    result_checkpoint: RegistryCheckpoint,
    record_set_digest: ContentDigest,
}

impl RegistryCommandRecord {
    /// Constructs one retained command row without claiming verification.
    #[must_use]
    pub const fn from_retained(
        ordinal: u64,
        command: RegistryCommand,
        receipt: RegistryCommandReceipt,
        base_checkpoint: RegistryCheckpoint,
        result_checkpoint: RegistryCheckpoint,
        record_set_digest: ContentDigest,
    ) -> Self {
        Self {
            ordinal,
            command,
            receipt,
            base_checkpoint,
            result_checkpoint,
            record_set_digest,
        }
    }

    /// Returns the strict positive first-seen global ordinal.
    #[must_use]
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }

    /// Returns the complete original typed command.
    #[must_use]
    pub const fn command(&self) -> &RegistryCommand {
        &self.command
    }

    /// Returns the complete semantic terminal receipt.
    #[must_use]
    pub const fn receipt(&self) -> &RegistryCommandReceipt {
        &self.receipt
    }

    /// Returns the checkpoint against which this command was planned.
    #[must_use]
    pub const fn base_checkpoint(&self) -> &RegistryCheckpoint {
        &self.base_checkpoint
    }

    /// Returns the checkpoint produced by this first-seen command.
    #[must_use]
    pub const fn result_checkpoint(&self) -> &RegistryCheckpoint {
        &self.result_checkpoint
    }

    /// Returns the Registry-owned record-set commitment.
    #[must_use]
    pub const fn record_set_digest(&self) -> &ContentDigest {
        &self.record_set_digest
    }
}

/// Complete immutable commitment to one verified global Registry snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryCheckpoint {
    runtime: RuntimeKind,
    command_ordinal: u64,
    observation_count: u64,
    project_count: u64,
    command_count: u64,
    reservation_count: u64,
    retained_bytes: u64,
    checkpoint_digest: ContentDigest,
}

impl RegistryCheckpoint {
    /// Reconstructs one independently retained checkpoint commitment.
    ///
    /// This constructor does not claim that the checkpoint is current or that
    /// its separately retained Registry rows are internally consistent.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn from_retained(
        runtime: RuntimeKind,
        command_ordinal: u64,
        observation_count: u64,
        project_count: u64,
        command_count: u64,
        reservation_count: u64,
        retained_bytes: u64,
        checkpoint_digest: ContentDigest,
    ) -> Self {
        Self {
            runtime,
            command_ordinal,
            observation_count,
            project_count,
            command_count,
            reservation_count,
            retained_bytes,
            checkpoint_digest,
        }
    }

    /// Returns the structural runtime marker bound by the checkpoint.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    /// Returns the global first-seen command high-water.
    #[must_use]
    pub const fn command_ordinal(&self) -> u64 {
        self.command_ordinal
    }

    /// Returns the number of retained immutable observations.
    #[must_use]
    pub const fn observation_count(&self) -> u64 {
        self.observation_count
    }

    /// Returns the number of current project projections.
    #[must_use]
    pub const fn project_count(&self) -> u64 {
        self.project_count
    }

    /// Returns the number of retained first-seen terminal commands.
    #[must_use]
    pub const fn command_count(&self) -> u64 {
        self.command_count
    }

    /// Returns the number of accepted and pending identity reservations.
    #[must_use]
    pub const fn reservation_count(&self) -> u64 {
        self.reservation_count
    }

    /// Returns the exact canonical logical-retained-state byte count.
    #[must_use]
    pub const fn retained_bytes(&self) -> u64 {
        self.retained_bytes
    }

    /// Returns the complete checkpoint digest.
    #[must_use]
    pub const fn checkpoint_digest(&self) -> &ContentDigest {
        &self.checkpoint_digest
    }
}

/// Verified immutable view of the global Project Registry retained state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedRegistryState {
    checkpoint: RegistryCheckpoint,
    observations: BTreeMap<String, RepositoryObservation>,
    projects: BTreeMap<ProjectId, RegistryProjectProjection>,
    commands: BTreeMap<u64, RegistryCommandRecord>,
    record_sets: BTreeMap<u64, RegistryRecordSet>,
    reservations: Vec<RegistryIdentityReservation>,
}

impl VerifiedRegistryState {
    /// Constructs a structural zero-command Registry for one explicit runtime.
    ///
    /// A `Live` marker remains structural only; it does not prove database
    /// durability or current authority.
    ///
    /// # Errors
    ///
    /// Returns an error when canonical logical-state or checkpoint hashing
    /// cannot be produced.
    pub fn vacant(runtime: RuntimeKind) -> Result<Self, RegistryError> {
        let logical_state = vacant_registry_logical_state(runtime);
        let retained_bytes =
            u64::try_from(canonicalize(&logical_state)?.as_slice().len()).unwrap_or(u64::MAX);
        let checkpoint_value =
            vacant_registry_checkpoint_value(runtime, retained_bytes, logical_state);
        let checkpoint_digest =
            registry_digest("lattice.project-registry.checkpoint", &checkpoint_value)?;
        Ok(Self {
            checkpoint: RegistryCheckpoint::from_retained(
                runtime,
                0,
                0,
                0,
                0,
                0,
                retained_bytes,
                checkpoint_digest,
            ),
            observations: BTreeMap::new(),
            projects: BTreeMap::new(),
            commands: BTreeMap::new(),
            record_sets: BTreeMap::new(),
            reservations: Vec::new(),
        })
    }

    /// Returns the complete verified checkpoint.
    #[must_use]
    pub const fn checkpoint(&self) -> &RegistryCheckpoint {
        &self.checkpoint
    }

    /// Returns true only for the structural zero-command Registry state.
    #[must_use]
    pub const fn is_vacant(&self) -> bool {
        self.checkpoint.command_ordinal == 0
            && self.checkpoint.observation_count == 0
            && self.checkpoint.project_count == 0
            && self.checkpoint.command_count == 0
            && self.checkpoint.reservation_count == 0
    }

    /// Returns the complete retained first-seen command history.
    #[must_use]
    pub const fn commands(&self) -> &BTreeMap<u64, RegistryCommandRecord> {
        &self.commands
    }

    /// Returns the complete current normalized identity reservations.
    #[must_use]
    pub fn reservations(&self) -> &[RegistryIdentityReservation] {
        &self.reservations
    }
}

/// Complete untrusted persistence snapshot for the global Project Registry.
///
/// The representation is intentionally opaque. No field is authoritative
/// until [`verify_untrusted_registry_snapshot`] succeeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedRegistrySnapshot {
    claimed_checkpoint: RegistryCheckpoint,
    rows: UntrustedRegistryRows,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct UntrustedRegistryRows {
    observations: Vec<RepositoryObservation>,
    projects: Vec<RegistryProjectRow>,
    commands: Vec<RegistryCommandRecord>,
    reservations: Vec<RegistryIdentityReservation>,
}

impl UntrustedRegistrySnapshot {
    /// Constructs one complete retained snapshot without claiming that any row
    /// or checkpoint is valid/current.
    #[must_use]
    pub const fn from_retained(
        claimed_checkpoint: RegistryCheckpoint,
        observations: Vec<RepositoryObservation>,
        projects: Vec<RegistryProjectRow>,
        commands: Vec<RegistryCommandRecord>,
        reservations: Vec<RegistryIdentityReservation>,
    ) -> Self {
        Self {
            claimed_checkpoint,
            rows: UntrustedRegistryRows {
                observations,
                projects,
                commands,
                reservations,
            },
        }
    }

    /// Returns the untrusted claimed checkpoint.
    #[must_use]
    pub const fn claimed_checkpoint(&self) -> &RegistryCheckpoint {
        &self.claimed_checkpoint
    }

    /// Returns the untrusted retained observations in supplied order.
    #[must_use]
    pub fn observations(&self) -> &[RepositoryObservation] {
        &self.rows.observations
    }

    /// Returns the untrusted retained project rows in supplied order.
    #[must_use]
    pub fn projects(&self) -> &[RegistryProjectRow] {
        &self.rows.projects
    }

    /// Returns the untrusted command rows in supplied order.
    #[must_use]
    pub fn commands(&self) -> &[RegistryCommandRecord] {
        &self.rows.commands
    }

    /// Returns the untrusted reservation rows in supplied order.
    #[must_use]
    pub fn reservations(&self) -> &[RegistryIdentityReservation] {
        &self.rows.reservations
    }
}

/// Exports one complete verified Registry through the untrusted persistence
/// boundary.
#[must_use]
pub fn export_untrusted_registry_snapshot(
    state: &VerifiedRegistryState,
) -> UntrustedRegistrySnapshot {
    UntrustedRegistrySnapshot::from_retained(
        state.checkpoint.clone(),
        state.observations.values().cloned().collect(),
        state
            .projects
            .iter()
            .map(|(project_id, projection)| {
                RegistryProjectRow::from_retained(project_id.clone(), projection.clone())
            })
            .collect(),
        state.commands.values().cloned().collect(),
        state.reservations.clone(),
    )
}

/// Reconstructs and verifies one internally self-consistent Registry snapshot.
///
/// This proves only snapshot self-consistency. It does not prove that the
/// snapshot is the independently retained current Registry state.
///
/// # Errors
///
/// Returns [`RegistryError::CorruptSnapshot`] when the claimed checkpoint does
/// not match the replayed snapshot, or propagates canonical construction
/// failures.
pub fn verify_untrusted_registry_snapshot(
    snapshot: &UntrustedRegistrySnapshot,
) -> Result<VerifiedRegistryState, RegistryError> {
    let mut verified = VerifiedRegistryState::vacant(snapshot.claimed_checkpoint.runtime())?;
    for (index, retained_record) in snapshot.rows.commands.iter().enumerate() {
        let expected_ordinal = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(RegistryError::CorruptSnapshot)?;
        if retained_record.ordinal != expected_ordinal {
            return Err(RegistryError::CorruptSnapshot);
        }
        let plan = plan_command(&verified, retained_record.command.clone())
            .map_err(|_| RegistryError::CorruptSnapshot)?;
        if plan.replay || plan.record != *retained_record {
            return Err(RegistryError::CorruptSnapshot);
        }
        verified = apply_command_plan(&verified, &plan)
            .map_err(|_| RegistryError::CorruptSnapshot)?
            .state;
    }

    let expected_observations = verified.observations.values().cloned().collect::<Vec<_>>();
    let expected_projects = verified
        .projects
        .iter()
        .map(|(project_id, projection)| {
            RegistryProjectRow::from_retained(project_id.clone(), projection.clone())
        })
        .collect::<Vec<_>>();
    let expected_commands = verified.commands.values().cloned().collect::<Vec<_>>();
    if verified.checkpoint != snapshot.claimed_checkpoint
        || expected_observations != snapshot.rows.observations
        || expected_projects != snapshot.rows.projects
        || expected_commands != snapshot.rows.commands
        || verified.reservations != snapshot.rows.reservations
    {
        return Err(RegistryError::CorruptSnapshot);
    }
    Ok(verified)
}

/// Verifies an untrusted Registry snapshot against an independently retained
/// complete checkpoint.
///
/// # Errors
///
/// Returns the underlying replay error or
/// [`RegistryError::CheckpointMismatch`] when the internally verified snapshot
/// differs from the independently retained checkpoint.
pub fn verify_untrusted_registry_snapshot_against_checkpoint(
    snapshot: &UntrustedRegistrySnapshot,
    retained_checkpoint: &RegistryCheckpoint,
) -> Result<VerifiedRegistryState, RegistryError> {
    let verified = verify_untrusted_registry_snapshot(snapshot)?;
    if verified.checkpoint() != retained_checkpoint {
        return Err(RegistryError::CheckpointMismatch);
    }
    Ok(verified)
}

/// Complete current projection for one registered project.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryProjectProjection {
    project_class: ProjectClass,
    observation: RepositoryObservation,
    pending_observation: Option<RepositoryObservation>,
    drift: Vec<IdentityDrift>,
    authority: ProjectAuthorityReceipt,
}

impl RegistryProjectProjection {
    /// Constructs one untrusted retained projection for snapshot verification.
    #[must_use]
    pub const fn from_retained(
        project_class: ProjectClass,
        observation: RepositoryObservation,
        pending_observation: Option<RepositoryObservation>,
        drift: Vec<IdentityDrift>,
        authority: ProjectAuthorityReceipt,
    ) -> Self {
        Self {
            project_class,
            observation,
            pending_observation,
            drift,
            authority,
        }
    }

    /// Returns the immutable registered project class.
    #[must_use]
    pub const fn project_class(&self) -> ProjectClass {
        self.project_class
    }

    /// Returns the accepted current observation.
    #[must_use]
    pub const fn observation(&self) -> &RepositoryObservation {
        &self.observation
    }

    /// Returns the pending reconciliation observation, when retained.
    #[must_use]
    pub const fn pending_observation(&self) -> Option<&RepositoryObservation> {
        self.pending_observation.as_ref()
    }

    /// Returns the canonical ordered drift dimensions.
    #[must_use]
    pub fn drift(&self) -> &[IdentityDrift] {
        &self.drift
    }

    /// Returns the current immutable project authority receipt.
    #[must_use]
    pub const fn authority(&self) -> &ProjectAuthorityReceipt {
        &self.authority
    }
}

/// One untrusted retained current-project row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryProjectRow {
    project_id: ProjectId,
    projection: RegistryProjectProjection,
}

impl RegistryProjectRow {
    /// Constructs one retained project row without claiming verification.
    #[must_use]
    pub const fn from_retained(
        project_id: ProjectId,
        projection: RegistryProjectProjection,
    ) -> Self {
        Self {
            project_id,
            projection,
        }
    }

    /// Returns the retained project identity.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the retained current projection.
    #[must_use]
    pub const fn projection(&self) -> &RegistryProjectProjection {
        &self.projection
    }
}

/// Exact Registry-owned persistence delta for one first-seen command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryRecordSet {
    ordinal: u64,
    command: RegistryCommand,
    receipt: RegistryCommandReceipt,
    base_checkpoint: RegistryCheckpoint,
    result_checkpoint: RegistryCheckpoint,
    new_observation: Option<RepositoryObservation>,
    project_replacement: Option<(ProjectId, RegistryProjectProjection)>,
    reservation_deletes: Vec<RegistryIdentityReservation>,
    reservation_inserts: Vec<RegistryIdentityReservation>,
    record_set_digest: ContentDigest,
}

impl RegistryRecordSet {
    /// Returns the first-seen command ordinal.
    #[must_use]
    pub const fn ordinal(&self) -> u64 {
        self.ordinal
    }

    /// Returns the complete original typed command.
    #[must_use]
    pub const fn command(&self) -> &RegistryCommand {
        &self.command
    }

    /// Returns the complete semantic terminal receipt.
    #[must_use]
    pub const fn receipt(&self) -> &RegistryCommandReceipt {
        &self.receipt
    }

    /// Returns the verified base checkpoint.
    #[must_use]
    pub const fn base_checkpoint(&self) -> &RegistryCheckpoint {
        &self.base_checkpoint
    }

    /// Returns the planned result checkpoint.
    #[must_use]
    pub const fn result_checkpoint(&self) -> &RegistryCheckpoint {
        &self.result_checkpoint
    }

    /// Returns an immutable observation inserted by this command, when new.
    #[must_use]
    pub const fn new_observation(&self) -> Option<&RepositoryObservation> {
        self.new_observation.as_ref()
    }

    /// Returns the current project replacement, when the command changed it.
    #[must_use]
    pub fn project_replacement(&self) -> Option<(&ProjectId, &RegistryProjectProjection)> {
        self.project_replacement
            .as_ref()
            .map(|(project_id, projection)| (project_id, projection))
    }

    /// Returns the ordered reservations removed by this command.
    #[must_use]
    pub fn reservation_deletes(&self) -> &[RegistryIdentityReservation] {
        &self.reservation_deletes
    }

    /// Returns the ordered reservations inserted by this command.
    #[must_use]
    pub fn reservation_inserts(&self) -> &[RegistryIdentityReservation] {
        &self.reservation_inserts
    }

    /// Returns the acyclic Registry record-set commitment.
    #[must_use]
    pub const fn record_set_digest(&self) -> &ContentDigest {
        &self.record_set_digest
    }
}

/// Immutable result of planning one command against a verified base.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryCommandPlan {
    expected_current_checkpoint: RegistryCheckpoint,
    result_state: VerifiedRegistryState,
    record: RegistryCommandRecord,
    record_set: RegistryRecordSet,
    replay: bool,
}

impl RegistryCommandPlan {
    /// Returns the complete checkpoint that must still be current on apply.
    #[must_use]
    pub const fn base_checkpoint(&self) -> &RegistryCheckpoint {
        &self.expected_current_checkpoint
    }

    /// Returns the checkpoint that will be current after applying this plan.
    #[must_use]
    pub const fn result_checkpoint(&self) -> &RegistryCheckpoint {
        &self.result_state.checkpoint
    }

    /// Returns the terminal semantic receipt.
    #[must_use]
    pub const fn receipt(&self) -> &RegistryCommandReceipt {
        &self.record.receipt
    }

    /// Returns the Registry-owned persistence record set.
    #[must_use]
    pub const fn record_set(&self) -> &RegistryRecordSet {
        &self.record_set
    }

    /// Returns true when this plan is an exact historical command replay.
    #[must_use]
    pub const fn is_replay(&self) -> bool {
        self.replay
    }
}

/// Verified result of applying one exact Registry plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedRegistryCommand {
    state: VerifiedRegistryState,
    record: RegistryCommandRecord,
    record_set: RegistryRecordSet,
}

impl AppliedRegistryCommand {
    /// Returns the new verified state.
    #[must_use]
    pub const fn state(&self) -> &VerifiedRegistryState {
        &self.state
    }

    /// Returns the resulting current checkpoint.
    #[must_use]
    pub const fn checkpoint(&self) -> &RegistryCheckpoint {
        &self.state.checkpoint
    }

    /// Returns the semantic terminal receipt.
    #[must_use]
    pub const fn receipt(&self) -> &RegistryCommandReceipt {
        &self.record.receipt
    }

    /// Returns the Registry-owned record set.
    #[must_use]
    pub const fn record_set(&self) -> &RegistryRecordSet {
        &self.record_set
    }
}

/// Plans one command without mutating its verified base state.
///
/// # Errors
///
/// Returns a typed error for changed command reuse, ordinal/capacity overflow,
/// or canonical construction failure.
pub fn plan_command(
    base: &VerifiedRegistryState,
    command: RegistryCommand,
) -> Result<RegistryCommandPlan, RegistryError> {
    let request_digest = command.request_digest()?;
    if let Some(replay) = plan_exact_replay(base, &command, &request_digest)? {
        return Ok(replay);
    }
    plan_first_seen_command(base, command)
}

fn plan_exact_replay(
    base: &VerifiedRegistryState,
    command: &RegistryCommand,
    request_digest: &ContentDigest,
) -> Result<Option<RegistryCommandPlan>, RegistryError> {
    let Some(record) = base
        .commands
        .values()
        .find(|record| record.command.command_id() == command.command_id())
    else {
        return Ok(None);
    };
    if record.receipt.request_digest() != request_digest {
        return Err(RegistryError::CommandIdReuse);
    }
    let record_set = base
        .record_sets
        .get(&record.ordinal)
        .ok_or(RegistryError::CorruptSnapshot)?
        .clone();
    Ok(Some(RegistryCommandPlan {
        expected_current_checkpoint: base.checkpoint.clone(),
        result_state: base.clone(),
        record: record.clone(),
        record_set,
        replay: true,
    }))
}

fn plan_first_seen_command(
    base: &VerifiedRegistryState,
    command: RegistryCommand,
) -> Result<RegistryCommandPlan, RegistryError> {
    if base.commands.len() >= MAX_REGISTRY_COMMANDS {
        return Err(RegistryError::CapacityExceeded);
    }
    let ordinal = next_registry_ordinal(base.checkpoint.command_ordinal)?;
    let mut machine = RegistryMachine {
        runtime: base.checkpoint.runtime,
        projects: base.projects.clone(),
        commands: base
            .commands
            .values()
            .map(|record| (record.command.command_id().clone(), record.receipt.clone()))
            .collect(),
    };
    let receipt = machine.execute(command.clone())?;
    let delta = build_registry_delta(base, &command, &machine.projects);
    let result_checkpoint = build_planned_result_checkpoint(
        base,
        ordinal,
        &command,
        &receipt,
        &delta,
        &machine.projects,
    )?;
    assemble_first_seen_plan(
        base,
        ordinal,
        command,
        receipt,
        machine.projects,
        delta,
        result_checkpoint,
    )
}

struct RegistryDelta {
    observations: BTreeMap<String, RepositoryObservation>,
    new_observation: Option<RepositoryObservation>,
    project_replacement: Option<(ProjectId, RegistryProjectProjection)>,
    reservations: Vec<RegistryIdentityReservation>,
    reservation_deletes: Vec<RegistryIdentityReservation>,
    reservation_inserts: Vec<RegistryIdentityReservation>,
}

fn build_registry_delta(
    base: &VerifiedRegistryState,
    command: &RegistryCommand,
    projects: &BTreeMap<ProjectId, RegistryProjectProjection>,
) -> RegistryDelta {
    let mut observations = base.observations.clone();
    let command_observation = command_observation(command);
    let new_observation = command_observation.and_then(|observation| {
        let key = observation.digest().as_str().to_owned();
        if let std::collections::btree_map::Entry::Vacant(entry) = observations.entry(key) {
            entry.insert(observation.clone());
            return Some(observation.clone());
        }
        None
    });
    let reservations = registry_reservations(projects);
    let target_project_id = command_project_id(command);
    let project_replacement = match (
        base.projects.get(target_project_id),
        projects.get(target_project_id),
    ) {
        (before, Some(after)) if before != Some(after) => {
            Some((target_project_id.clone(), after.clone()))
        }
        _ => None,
    };
    let reservation_deletes = base
        .reservations
        .iter()
        .filter(|item| !reservations.contains(item))
        .cloned()
        .collect::<Vec<_>>();
    let reservation_inserts = reservations
        .iter()
        .filter(|item| !base.reservations.contains(item))
        .cloned()
        .collect::<Vec<_>>();
    RegistryDelta {
        observations,
        new_observation,
        project_replacement,
        reservations,
        reservation_deletes,
        reservation_inserts,
    }
}

fn build_planned_result_checkpoint(
    base: &VerifiedRegistryState,
    ordinal: u64,
    command: &RegistryCommand,
    receipt: &RegistryCommandReceipt,
    delta: &RegistryDelta,
    projects: &BTreeMap<ProjectId, RegistryProjectProjection>,
) -> Result<RegistryCheckpoint, RegistryError> {
    let mut command_cores = base
        .commands
        .values()
        .map(|record| (record.ordinal, &record.command, &record.receipt))
        .collect::<Vec<_>>();
    command_cores.push((ordinal, command, receipt));
    let logical_state = registry_logical_state_value(
        base.checkpoint.runtime,
        &delta.observations,
        projects,
        &command_cores,
        &delta.reservations,
    );
    let retained_bytes = u64::try_from(canonicalize(&logical_state)?.as_slice().len())
        .map_err(|_| RegistryError::CapacityExceeded)?;
    ensure_registry_limits(projects.len(), command_cores.len(), retained_bytes)?;
    build_registry_checkpoint(
        base.checkpoint.runtime,
        ordinal,
        delta.observations.len(),
        projects.len(),
        command_cores.len(),
        delta.reservations.len(),
        retained_bytes,
        logical_state,
    )
}

#[allow(clippy::too_many_arguments)]
fn assemble_first_seen_plan(
    base: &VerifiedRegistryState,
    ordinal: u64,
    command: RegistryCommand,
    receipt: RegistryCommandReceipt,
    projects: BTreeMap<ProjectId, RegistryProjectProjection>,
    delta: RegistryDelta,
    result_checkpoint: RegistryCheckpoint,
) -> Result<RegistryCommandPlan, RegistryError> {
    let record_set_value = registry_record_set_value(
        ordinal,
        &command,
        &receipt,
        &base.checkpoint,
        &result_checkpoint,
        delta.new_observation.as_ref(),
        delta.project_replacement.as_ref(),
        &delta.reservation_deletes,
        &delta.reservation_inserts,
    );
    let record_set_digest =
        registry_digest("lattice.project-registry.record-set", &record_set_value)?;
    let record_set = RegistryRecordSet {
        ordinal,
        command: command.clone(),
        receipt: receipt.clone(),
        base_checkpoint: base.checkpoint.clone(),
        result_checkpoint: result_checkpoint.clone(),
        new_observation: delta.new_observation,
        project_replacement: delta.project_replacement,
        reservation_deletes: delta.reservation_deletes,
        reservation_inserts: delta.reservation_inserts,
        record_set_digest: record_set_digest.clone(),
    };
    let record = RegistryCommandRecord {
        ordinal,
        command,
        receipt,
        base_checkpoint: base.checkpoint.clone(),
        result_checkpoint: result_checkpoint.clone(),
        record_set_digest,
    };
    let mut commands = base.commands.clone();
    commands.insert(ordinal, record.clone());
    let mut record_sets = base.record_sets.clone();
    record_sets.insert(ordinal, record_set.clone());
    let result_state = VerifiedRegistryState {
        checkpoint: result_checkpoint,
        observations: delta.observations,
        projects,
        commands,
        record_sets,
        reservations: delta.reservations,
    };
    Ok(RegistryCommandPlan {
        expected_current_checkpoint: base.checkpoint.clone(),
        result_state,
        record,
        record_set,
        replay: false,
    })
}

/// Applies one immutable plan only while its complete base checkpoint remains
/// current.
///
/// # Errors
///
/// Returns [`RegistryError::CheckpointMismatch`] on stale/substituted state.
pub fn apply_command_plan(
    current: &VerifiedRegistryState,
    plan: &RegistryCommandPlan,
) -> Result<AppliedRegistryCommand, RegistryError> {
    if current.checkpoint != plan.expected_current_checkpoint {
        return Err(RegistryError::CheckpointMismatch);
    }
    Ok(AppliedRegistryCommand {
        state: plan.result_state.clone(),
        record: plan.record.clone(),
        record_set: plan.record_set.clone(),
    })
}

/// Exact Registry-owned subset a future Scope Check receipt must bind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistryScopeBinding {
    authority_head: ProjectAuthorityHead,
    observation_digest: ContentDigest,
    primary_branch: GitRefIdentity,
}

impl RegistryScopeBinding {
    /// Returns the exact active Registry head.
    #[must_use]
    pub const fn authority_head(&self) -> &ProjectAuthorityHead {
        &self.authority_head
    }

    /// Returns the complete active owner-observation digest.
    #[must_use]
    pub const fn observation_digest(&self) -> &ContentDigest {
        &self.observation_digest
    }

    /// Returns the canonical physical primary-ref identity.
    #[must_use]
    pub const fn primary_branch(&self) -> &GitRefIdentity {
        &self.primary_branch
    }
}

/// Deterministic in-memory owner used only for fake/local contract evidence.
///
/// The wrapper forces `RuntimeKind::Fake` and delegates every command to the
/// same pure plan/apply boundary consumed by durable adapters.
#[derive(Debug)]
pub struct FakeProjectRegistry {
    state: VerifiedRegistryState,
}

impl Default for FakeProjectRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeProjectRegistry {
    /// Creates an empty fake Registry. It is not durable truth.
    ///
    /// # Panics
    ///
    /// Panics only if the compile-time frozen vacant canonical subject can no
    /// longer be encoded, which indicates an incompatible crate build.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: VerifiedRegistryState::vacant(RuntimeKind::Fake)
                .expect("frozen vacant Fake Registry construction"),
        }
    }

    /// Executes one idempotent command through the shared pure planner.
    ///
    /// # Errors
    ///
    /// Returns a typed error for command substitution, stale plan state,
    /// retained-state limits, or canonical construction failure. Domain
    /// denials remain terminal semantic receipts.
    pub fn execute(
        &mut self,
        command: RegistryCommand,
    ) -> Result<RegistryCommandReceipt, RegistryError> {
        let plan = plan_command(&self.state, command)?;
        let applied = apply_command_plan(&self.state, &plan)?;
        let receipt = applied.record.receipt.clone();
        self.state = applied.state;
        Ok(receipt)
    }

    /// Returns the complete current verified Fake state.
    #[must_use]
    pub const fn verified_state(&self) -> &VerifiedRegistryState {
        &self.state
    }

    /// Returns the complete current global checkpoint.
    #[must_use]
    pub const fn checkpoint(&self) -> &RegistryCheckpoint {
        &self.state.checkpoint
    }

    /// Returns the latest fake authority receipt for one registered project.
    #[must_use]
    pub fn latest(&self, project_id: &ProjectId) -> Option<&ProjectAuthorityReceipt> {
        self.state
            .projects
            .get(project_id)
            .map(|record| &record.authority)
    }

    /// Performs an independent fake-owner lookup of the current authority
    /// head, including non-active heads.
    ///
    /// This lookup, not a historical receipt's structural `head()` projection,
    /// is the currentness input expected by Policy composition. The fake lookup
    /// is deterministic but is not durable or authenticated evidence.
    #[must_use]
    pub fn current_head(&self, project_id: &ProjectId) -> Option<ProjectAuthorityHead> {
        self.latest(project_id).map(ProjectAuthorityReceipt::head)
    }

    /// Returns the exact active Registry subset required by future Scope Check
    /// composition. Non-active and unknown projects return no binding.
    #[must_use]
    pub fn scope_binding(&self, project_id: &ProjectId) -> Option<RegistryScopeBinding> {
        let record = self.state.projects.get(project_id)?;
        if record.authority.lifecycle() != ProjectLifecycle::Active
            || record.pending_observation.is_some()
        {
            return None;
        }
        Some(RegistryScopeBinding {
            authority_head: record.authority.head(),
            observation_digest: record.observation.digest.clone(),
            primary_branch: record.observation.primary_branch.clone(),
        })
    }
}

#[derive(Debug)]
struct RegistryMachine {
    runtime: RuntimeKind,
    projects: BTreeMap<ProjectId, RegistryProjectProjection>,
    commands: BTreeMap<CommandId, RegistryCommandReceipt>,
}

impl RegistryMachine {
    /// Executes one semantic command against an isolated in-memory projection.
    ///
    /// # Errors
    ///
    /// Returns an error for command-ID substitution or invalid canonical
    /// receipt/request construction. Domain denials are terminal receipts.
    fn execute(
        &mut self,
        command: RegistryCommand,
    ) -> Result<RegistryCommandReceipt, RegistryError> {
        let request_digest = command.request_digest()?;
        if let Some(previous) = self.commands.get(command.command_id()) {
            return if previous.request_digest == request_digest {
                Ok(previous.clone())
            } else {
                Err(RegistryError::CommandIdReuse)
            };
        }

        let command_id = command.command_id().clone();
        let effect = match command {
            RegistryCommand::Register {
                project_id,
                project_class,
                observation,
                ..
            } => self.register(project_id, project_class, observation)?,
            RegistryCommand::Observe {
                project_id,
                expected_head,
                observation,
                ..
            } => self.observe(&project_id, &expected_head, observation)?,
            RegistryCommand::Suspend {
                project_id,
                expected_head,
                evidence_digest,
                ..
            } => self.suspend(&project_id, &expected_head, &evidence_digest)?,
            RegistryCommand::Reconcile {
                project_id,
                expected_head,
                observation,
                decision,
                evidence_digest,
                ..
            } => self.reconcile(
                &project_id,
                &expected_head,
                observation,
                decision,
                &evidence_digest,
            )?,
        };
        let result_digest = command_result_digest(
            &command_id,
            &request_digest,
            effect.before.as_ref(),
            effect.after.as_ref(),
            &effect.outcome,
            &effect.drift,
            effect.authority.as_ref(),
        )?;
        let receipt = RegistryCommandReceipt {
            command_id: command_id.clone(),
            request_digest,
            before: effect.before,
            after: effect.after,
            outcome: effect.outcome,
            drift: effect.drift,
            authority: effect.authority,
            result_digest,
        };
        self.commands.insert(command_id, receipt.clone());
        Ok(receipt)
    }

    fn register(
        &mut self,
        project_id: ProjectId,
        project_class: ProjectClass,
        observation: RepositoryObservation,
    ) -> Result<CommandEffect, RegistryError> {
        if let Some(existing) = self.projects.get(&project_id) {
            return Ok(denied_effect(
                None,
                RegistryDenial::DuplicateIdentity {
                    dimension: IdentityDimension::ProjectId,
                    existing_project_id: existing.authority.project_id().clone(),
                },
            ));
        }
        if let Some((dimension, existing_project_id)) = self.duplicate_identity(&observation, None)
        {
            return Ok(denied_effect(
                None,
                RegistryDenial::DuplicateIdentity {
                    dimension,
                    existing_project_id,
                },
            ));
        }

        let authority = issue_authority(
            self.runtime,
            &project_id,
            project_class,
            1,
            ProjectLifecycle::Active,
            &observation,
            AuthorityTransition {
                name: "REGISTER",
                previous_head: None,
                evidence_digest: None,
            },
        )?;
        let after = authority.head();
        self.projects.insert(
            project_id,
            RegistryProjectProjection {
                project_class,
                observation,
                pending_observation: None,
                drift: Vec::new(),
                authority: authority.clone(),
            },
        );
        Ok(applied_effect(None, after, authority, Vec::new()))
    }

    fn observe(
        &mut self,
        project_id: &ProjectId,
        expected_head: &ProjectAuthorityHead,
        observation: RepositoryObservation,
    ) -> Result<CommandEffect, RegistryError> {
        let Some(current_record) = self.projects.get(project_id) else {
            return Ok(denied_effect(None, RegistryDenial::UnknownProject));
        };
        let current_head = current_record.authority.head();
        if expected_head != &current_head {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::StaleHead,
            ));
        }
        if current_record.authority.lifecycle() != ProjectLifecycle::Active {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::LifecycleBlocked {
                    lifecycle: current_record.authority.lifecycle(),
                },
            ));
        }
        if observation == current_record.observation {
            return Ok(applied_effect(
                Some(current_head.clone()),
                current_head,
                current_record.authority.clone(),
                Vec::new(),
            ));
        }
        if let Some((dimension, existing_project_id)) =
            self.duplicate_identity(&observation, Some(project_id))
        {
            let drift = identity_drift(&current_record.observation, &observation);
            let Some(next_revision) = current_record.authority.registry_revision().checked_add(1)
            else {
                return Ok(denied_effect(
                    Some(current_record.authority.clone()),
                    RegistryDenial::RevisionOverflow,
                ));
            };
            let authority = issue_authority(
                self.runtime,
                project_id,
                current_record.project_class,
                next_revision,
                ProjectLifecycle::Suspended,
                &observation,
                AuthorityTransition {
                    name: "OBSERVE_IDENTITY_CONFLICT",
                    previous_head: Some(&current_head),
                    evidence_digest: Some(observation.digest()),
                },
            )?;
            let after = authority.head();
            let record = self.projects.get_mut(project_id).expect("record exists");
            // The first project whose drift produced a pending identity owns
            // that reservation. A later collision is blocked but never gets a
            // second reservation for the same physical identity.
            record.pending_observation = None;
            record.drift.clone_from(&drift);
            record.authority = authority.clone();
            return Ok(blocked_effect(
                Some(current_head),
                after,
                authority,
                drift,
                RegistryDenial::DuplicateIdentity {
                    dimension,
                    existing_project_id,
                },
            ));
        }

        let drift = identity_drift(&current_record.observation, &observation);
        let Some(next_revision) = current_record.authority.registry_revision().checked_add(1)
        else {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::RevisionOverflow,
            ));
        };
        let authority = issue_authority(
            self.runtime,
            project_id,
            current_record.project_class,
            next_revision,
            ProjectLifecycle::ReconciliationRequired,
            &observation,
            AuthorityTransition {
                name: "OBSERVE_DRIFT",
                previous_head: Some(&current_head),
                evidence_digest: Some(observation.digest()),
            },
        )?;
        let after = authority.head();
        let record = self.projects.get_mut(project_id).expect("record exists");
        record.pending_observation = Some(observation);
        record.drift.clone_from(&drift);
        record.authority = authority.clone();
        Ok(applied_effect(Some(current_head), after, authority, drift))
    }

    fn suspend(
        &mut self,
        project_id: &ProjectId,
        expected_head: &ProjectAuthorityHead,
        evidence_digest: &ContentDigest,
    ) -> Result<CommandEffect, RegistryError> {
        let Some(current_record) = self.projects.get(project_id) else {
            return Ok(denied_effect(None, RegistryDenial::UnknownProject));
        };
        let current_head = current_record.authority.head();
        if expected_head != &current_head {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::StaleHead,
            ));
        }
        if current_record.authority.lifecycle() != ProjectLifecycle::Active {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::LifecycleBlocked {
                    lifecycle: current_record.authority.lifecycle(),
                },
            ));
        }
        let Some(next_revision) = current_record.authority.registry_revision().checked_add(1)
        else {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::RevisionOverflow,
            ));
        };
        let authority = issue_authority(
            self.runtime,
            project_id,
            current_record.project_class,
            next_revision,
            ProjectLifecycle::Suspended,
            &current_record.observation,
            AuthorityTransition {
                name: "SUSPEND",
                previous_head: Some(&current_head),
                evidence_digest: Some(evidence_digest),
            },
        )?;
        let after = authority.head();
        let record = self.projects.get_mut(project_id).expect("record exists");
        record.pending_observation = None;
        record.drift.clear();
        record.authority = authority.clone();
        Ok(applied_effect(
            Some(current_head),
            after,
            authority,
            Vec::new(),
        ))
    }

    fn reconcile(
        &mut self,
        project_id: &ProjectId,
        expected_head: &ProjectAuthorityHead,
        observation: RepositoryObservation,
        decision: ReconciliationDecision,
        evidence_digest: &ContentDigest,
    ) -> Result<CommandEffect, RegistryError> {
        let Some(current_record) = self.projects.get(project_id) else {
            return Ok(denied_effect(None, RegistryDenial::UnknownProject));
        };
        let current_head = current_record.authority.head();
        if expected_head != &current_head {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::StaleHead,
            ));
        }
        let expected_decision = match current_record.authority.lifecycle() {
            ProjectLifecycle::ReconciliationRequired => {
                let Some(pending) = current_record.pending_observation.as_ref() else {
                    return Ok(denied_effect(
                        Some(current_record.authority.clone()),
                        RegistryDenial::PendingObservationMismatch,
                    ));
                };
                if pending != &observation {
                    return Ok(denied_effect(
                        Some(current_record.authority.clone()),
                        RegistryDenial::PendingObservationMismatch,
                    ));
                }
                if current_record.drift.as_slice() == [IdentityDrift::CanonicalRoot] {
                    ReconciliationDecision::AcceptMove
                } else {
                    ReconciliationDecision::AcceptIdentityChange
                }
            }
            ProjectLifecycle::Suspended => {
                if observation != current_record.observation {
                    return Ok(denied_effect(
                        Some(current_record.authority.clone()),
                        RegistryDenial::PendingObservationMismatch,
                    ));
                }
                ReconciliationDecision::Reactivate
            }
            ProjectLifecycle::Active => {
                return Ok(denied_effect(
                    Some(current_record.authority.clone()),
                    RegistryDenial::LifecycleBlocked {
                        lifecycle: ProjectLifecycle::Active,
                    },
                ));
            }
        };
        if decision != expected_decision {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::ReconciliationDecisionMismatch {
                    expected: expected_decision,
                    found: decision,
                },
            ));
        }
        if let Some((dimension, existing_project_id)) =
            self.duplicate_identity(&observation, Some(project_id))
        {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::DuplicateIdentity {
                    dimension,
                    existing_project_id,
                },
            ));
        }
        let Some(next_revision) = current_record.authority.registry_revision().checked_add(1)
        else {
            return Ok(denied_effect(
                Some(current_record.authority.clone()),
                RegistryDenial::RevisionOverflow,
            ));
        };
        let authority = issue_authority(
            self.runtime,
            project_id,
            current_record.project_class,
            next_revision,
            ProjectLifecycle::Active,
            &observation,
            AuthorityTransition {
                name: decision.as_str(),
                previous_head: Some(&current_head),
                evidence_digest: Some(evidence_digest),
            },
        )?;
        let after = authority.head();
        let record = self.projects.get_mut(project_id).expect("record exists");
        record.observation = observation;
        record.pending_observation = None;
        record.drift.clear();
        record.authority = authority.clone();
        Ok(applied_effect(
            Some(current_head),
            after,
            authority,
            Vec::new(),
        ))
    }

    fn duplicate_identity(
        &self,
        candidate: &RepositoryObservation,
        excluded_project_id: Option<&ProjectId>,
    ) -> Option<(IdentityDimension, ProjectId)> {
        // Accepted identities always win over reservations when an already
        // conflicting pending observation names the same physical repository.
        for (project_id, record) in &self.projects {
            if excluded_project_id == Some(project_id) {
                continue;
            }
            if let Some(dimension) = identity_collision(candidate, &record.observation) {
                return Some((dimension, project_id.clone()));
            }
        }
        // A pending identity is reserved by the project whose authoritative
        // observation entered reconciliation. Another project cannot front-run
        // that explicit transition into ACTIVE.
        for (project_id, record) in &self.projects {
            if excluded_project_id == Some(project_id) {
                continue;
            }
            if let Some(pending) = record.pending_observation.as_ref()
                && let Some(dimension) = identity_collision(candidate, pending)
            {
                return Some((dimension, project_id.clone()));
            }
        }
        None
    }
}

struct CommandEffect {
    before: Option<ProjectAuthorityHead>,
    after: Option<ProjectAuthorityHead>,
    outcome: RegistryCommandOutcome,
    drift: Vec<IdentityDrift>,
    authority: Option<ProjectAuthorityReceipt>,
}

fn denied_effect(
    authority: Option<ProjectAuthorityReceipt>,
    denial: RegistryDenial,
) -> CommandEffect {
    let head = authority.as_ref().map(ProjectAuthorityReceipt::head);
    CommandEffect {
        before: head.clone(),
        after: head,
        outcome: RegistryCommandOutcome::Denied(denial),
        drift: Vec::new(),
        authority,
    }
}

fn applied_effect(
    before: Option<ProjectAuthorityHead>,
    after: ProjectAuthorityHead,
    authority: ProjectAuthorityReceipt,
    drift: Vec<IdentityDrift>,
) -> CommandEffect {
    CommandEffect {
        before,
        after: Some(after),
        outcome: RegistryCommandOutcome::Applied,
        drift,
        authority: Some(authority),
    }
}

fn blocked_effect(
    before: Option<ProjectAuthorityHead>,
    after: ProjectAuthorityHead,
    authority: ProjectAuthorityReceipt,
    drift: Vec<IdentityDrift>,
    denial: RegistryDenial,
) -> CommandEffect {
    CommandEffect {
        before,
        after: Some(after),
        outcome: RegistryCommandOutcome::Blocked(denial),
        drift,
        authority: Some(authority),
    }
}

fn command_project_id(command: &RegistryCommand) -> &ProjectId {
    match command {
        RegistryCommand::Register { project_id, .. }
        | RegistryCommand::Observe { project_id, .. }
        | RegistryCommand::Suspend { project_id, .. }
        | RegistryCommand::Reconcile { project_id, .. } => project_id,
    }
}

fn command_observation(command: &RegistryCommand) -> Option<&RepositoryObservation> {
    match command {
        RegistryCommand::Register { observation, .. }
        | RegistryCommand::Observe { observation, .. }
        | RegistryCommand::Reconcile { observation, .. } => Some(observation),
        RegistryCommand::Suspend { .. } => None,
    }
}

fn registry_reservations(
    projects: &BTreeMap<ProjectId, RegistryProjectProjection>,
) -> Vec<RegistryIdentityReservation> {
    let mut reservations = Vec::with_capacity(projects.len().saturating_mul(6));
    for (project_id, projection) in projects {
        extend_observation_reservations(
            &mut reservations,
            project_id,
            &projection.observation,
            RegistryReservationStatus::Accepted,
        );
        if let Some(pending) = projection.pending_observation.as_ref() {
            extend_observation_reservations(
                &mut reservations,
                project_id,
                pending,
                RegistryReservationStatus::Pending,
            );
        }
    }
    reservations.sort();
    reservations
}

fn extend_observation_reservations(
    reservations: &mut Vec<RegistryIdentityReservation>,
    project_id: &ProjectId,
    observation: &RepositoryObservation,
    status: RegistryReservationStatus,
) {
    reservations.extend([
        RegistryIdentityReservation {
            dimension: IdentityDimension::CanonicalRoot,
            identity_digest: observation.canonical_root_identity_digest.clone(),
            status,
            project_id: project_id.clone(),
        },
        RegistryIdentityReservation {
            dimension: IdentityDimension::Repository,
            identity_digest: observation.repository_identity_digest.clone(),
            status,
            project_id: project_id.clone(),
        },
        RegistryIdentityReservation {
            dimension: IdentityDimension::File,
            identity_digest: observation.file_identity_digest.clone(),
            status,
            project_id: project_id.clone(),
        },
    ]);
}

fn registry_logical_state_value(
    runtime: RuntimeKind,
    observations: &BTreeMap<String, RepositoryObservation>,
    projects: &BTreeMap<ProjectId, RegistryProjectProjection>,
    command_cores: &[(u64, &RegistryCommand, &RegistryCommandReceipt)],
    reservations: &[RegistryIdentityReservation],
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("schema_version", "1"),
        text_entry("runtime", registry_runtime_text(runtime)),
        (
            "observations".to_owned(),
            CanonicalValue::Array(
                observations
                    .values()
                    .map(registry_observation_value)
                    .collect(),
            ),
        ),
        (
            "projects".to_owned(),
            CanonicalValue::Array(
                projects
                    .iter()
                    .map(|(project_id, projection)| registry_project_value(project_id, projection))
                    .collect(),
            ),
        ),
        (
            "commands".to_owned(),
            CanonicalValue::Array(
                command_cores
                    .iter()
                    .map(|(ordinal, command, receipt)| {
                        registry_command_core_value(*ordinal, command, receipt)
                    })
                    .collect(),
            ),
        ),
        (
            "reservations".to_owned(),
            CanonicalValue::Array(
                reservations
                    .iter()
                    .map(registry_reservation_value)
                    .collect(),
            ),
        ),
    ])
}

#[allow(clippy::too_many_arguments)]
fn build_registry_checkpoint(
    runtime: RuntimeKind,
    command_ordinal: u64,
    observation_count: usize,
    project_count: usize,
    command_count: usize,
    reservation_count: usize,
    retained_bytes: u64,
    logical_state: CanonicalValue,
) -> Result<RegistryCheckpoint, RegistryError> {
    let observation_count =
        u64::try_from(observation_count).map_err(|_| RegistryError::CapacityExceeded)?;
    let project_count =
        u64::try_from(project_count).map_err(|_| RegistryError::CapacityExceeded)?;
    let command_count =
        u64::try_from(command_count).map_err(|_| RegistryError::CapacityExceeded)?;
    let reservation_count =
        u64::try_from(reservation_count).map_err(|_| RegistryError::CapacityExceeded)?;
    let value = CanonicalValue::Object(vec![
        text_entry("schema_version", "1"),
        text_entry("runtime", registry_runtime_text(runtime)),
        text_entry("command_ordinal", &command_ordinal.to_string()),
        text_entry("observation_count", &observation_count.to_string()),
        text_entry("project_count", &project_count.to_string()),
        text_entry("command_count", &command_count.to_string()),
        text_entry("reservation_count", &reservation_count.to_string()),
        text_entry("retained_bytes", &retained_bytes.to_string()),
        ("logical_state".to_owned(), logical_state),
    ]);
    let checkpoint_digest = registry_digest("lattice.project-registry.checkpoint", &value)?;
    Ok(RegistryCheckpoint::from_retained(
        runtime,
        command_ordinal,
        observation_count,
        project_count,
        command_count,
        reservation_count,
        retained_bytes,
        checkpoint_digest,
    ))
}

#[allow(clippy::too_many_arguments)]
fn registry_record_set_value(
    ordinal: u64,
    command: &RegistryCommand,
    receipt: &RegistryCommandReceipt,
    base_checkpoint: &RegistryCheckpoint,
    result_checkpoint: &RegistryCheckpoint,
    new_observation: Option<&RepositoryObservation>,
    project_replacement: Option<&(ProjectId, RegistryProjectProjection)>,
    reservation_deletes: &[RegistryIdentityReservation],
    reservation_inserts: &[RegistryIdentityReservation],
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "command".to_owned(),
            CanonicalValue::Object(vec![
                (
                    "core".to_owned(),
                    registry_command_core_value(ordinal, command, receipt),
                ),
                (
                    "base_checkpoint".to_owned(),
                    registry_checkpoint_projection_value(base_checkpoint),
                ),
                (
                    "result_checkpoint".to_owned(),
                    registry_checkpoint_projection_value(result_checkpoint),
                ),
            ]),
        ),
        (
            "new_observation".to_owned(),
            new_observation.map_or(CanonicalValue::Null, registry_observation_value),
        ),
        (
            "project_replacement".to_owned(),
            project_replacement.map_or(CanonicalValue::Null, |(project_id, projection)| {
                registry_project_value(project_id, projection)
            }),
        ),
        (
            "reservation_deletes".to_owned(),
            CanonicalValue::Array(
                reservation_deletes
                    .iter()
                    .map(registry_reservation_value)
                    .collect(),
            ),
        ),
        (
            "reservation_inserts".to_owned(),
            CanonicalValue::Array(
                reservation_inserts
                    .iter()
                    .map(registry_reservation_value)
                    .collect(),
            ),
        ),
    ])
}

fn registry_checkpoint_projection_value(checkpoint: &RegistryCheckpoint) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("runtime", registry_runtime_text(checkpoint.runtime)),
        text_entry("command_ordinal", &checkpoint.command_ordinal.to_string()),
        text_entry(
            "observation_count",
            &checkpoint.observation_count.to_string(),
        ),
        text_entry("project_count", &checkpoint.project_count.to_string()),
        text_entry("command_count", &checkpoint.command_count.to_string()),
        text_entry(
            "reservation_count",
            &checkpoint.reservation_count.to_string(),
        ),
        text_entry("retained_bytes", &checkpoint.retained_bytes.to_string()),
        text_entry("checkpoint_digest", checkpoint.checkpoint_digest.as_str()),
    ])
}

fn registry_observation_value(observation: &RepositoryObservation) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("digest", observation.digest.as_str()),
        text_entry("canonical_root", &observation.canonical_root),
        text_entry(
            "canonical_root_identity_digest",
            observation.canonical_root_identity_digest.as_str(),
        ),
        text_entry(
            "repository_identity_digest",
            observation.repository_identity_digest.as_str(),
        ),
        text_entry(
            "file_identity_digest",
            observation.file_identity_digest.as_str(),
        ),
        text_entry("primary_ref", observation.primary_branch.reference()),
        text_entry(
            "primary_ref_storage_identity_digest",
            observation
                .primary_branch
                .storage_identity_digest()
                .as_str(),
        ),
    ])
}

fn registry_project_value(
    project_id: &ProjectId,
    projection: &RegistryProjectProjection,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("project_id", project_id.as_str()),
        text_entry("project_class", projection.project_class.as_str()),
        text_entry(
            "accepted_observation_digest",
            projection.observation.digest.as_str(),
        ),
        (
            "pending_observation_digest".to_owned(),
            projection
                .pending_observation
                .as_ref()
                .map_or(CanonicalValue::Null, |observation| {
                    CanonicalValue::String(observation.digest.as_str().to_owned())
                }),
        ),
        (
            "drift".to_owned(),
            CanonicalValue::Array(
                projection
                    .drift
                    .iter()
                    .map(|item| CanonicalValue::String(item.as_str().to_owned()))
                    .collect(),
            ),
        ),
        (
            "authority".to_owned(),
            registry_authority_receipt_value(&projection.authority),
        ),
    ])
}

fn registry_command_core_value(
    ordinal: u64,
    command: &RegistryCommand,
    receipt: &RegistryCommandReceipt,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("ordinal", &ordinal.to_string()),
        ("request".to_owned(), registry_typed_command_value(command)),
        (
            "receipt".to_owned(),
            registry_command_receipt_value(receipt),
        ),
    ])
}

fn registry_typed_command_value(command: &RegistryCommand) -> CanonicalValue {
    let mut fields = vec![text_entry("command_id", command.command_id().as_str())];
    match command {
        RegistryCommand::Register {
            project_id,
            project_class,
            observation,
            ..
        } => fields.extend([
            text_entry("action", "REGISTER"),
            text_entry("project_id", project_id.as_str()),
            text_entry("project_class", project_class.as_str()),
            text_entry("observation_digest", observation.digest.as_str()),
        ]),
        RegistryCommand::Observe {
            project_id,
            expected_head,
            observation,
            ..
        } => fields.extend([
            text_entry("action", "OBSERVE"),
            text_entry("project_id", project_id.as_str()),
            (
                "expected_head".to_owned(),
                authority_head_value(expected_head),
            ),
            text_entry("observation_digest", observation.digest.as_str()),
        ]),
        RegistryCommand::Suspend {
            project_id,
            expected_head,
            evidence_digest,
            ..
        } => fields.extend([
            text_entry("action", "SUSPEND"),
            text_entry("project_id", project_id.as_str()),
            (
                "expected_head".to_owned(),
                authority_head_value(expected_head),
            ),
            text_entry("evidence_digest", evidence_digest.as_str()),
        ]),
        RegistryCommand::Reconcile {
            project_id,
            expected_head,
            observation,
            decision,
            evidence_digest,
            ..
        } => fields.extend([
            text_entry("action", "RECONCILE"),
            text_entry("project_id", project_id.as_str()),
            (
                "expected_head".to_owned(),
                authority_head_value(expected_head),
            ),
            text_entry("observation_digest", observation.digest.as_str()),
            text_entry("decision", decision.as_str()),
            text_entry("evidence_digest", evidence_digest.as_str()),
        ]),
    }
    CanonicalValue::Object(fields)
}

fn registry_command_receipt_value(receipt: &RegistryCommandReceipt) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("command_id", receipt.command_id.as_str()),
        text_entry("request_digest", receipt.request_digest.as_str()),
        optional_head_entry("before", receipt.before.as_ref()),
        optional_head_entry("after", receipt.after.as_ref()),
        ("outcome".to_owned(), outcome_value(&receipt.outcome)),
        (
            "drift".to_owned(),
            CanonicalValue::Array(
                receipt
                    .drift
                    .iter()
                    .map(|item| CanonicalValue::String(item.as_str().to_owned()))
                    .collect(),
            ),
        ),
        (
            "authority".to_owned(),
            receipt
                .authority
                .as_ref()
                .map_or(CanonicalValue::Null, registry_authority_receipt_value),
        ),
        text_entry("result_digest", receipt.result_digest.as_str()),
    ])
}

fn registry_authority_receipt_value(receipt: &ProjectAuthorityReceipt) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("contract_version", &CONTRACT_VERSION.to_string()),
        text_entry("producer_id", receipt.producer_id()),
        text_entry("producer_version", receipt.producer_version()),
        text_entry("runtime", registry_runtime_text(receipt.runtime())),
        text_entry("project_id", receipt.project_id().as_str()),
        text_entry(
            "project_snapshot_id",
            receipt.project_snapshot_id().as_str(),
        ),
        text_entry(
            "registry_revision",
            &receipt.registry_revision().to_string(),
        ),
        text_entry("lifecycle", receipt.lifecycle().as_str()),
        text_entry("project_class", receipt.project_class().as_str()),
        text_entry("primary_ref", receipt.primary_branch().reference()),
        text_entry(
            "primary_ref_storage_identity_digest",
            receipt.primary_branch().storage_identity_digest().as_str(),
        ),
        text_entry("observation_digest", receipt.observation_digest().as_str()),
        text_entry("receipt_digest", receipt.receipt_digest().as_str()),
    ])
}

fn registry_reservation_value(reservation: &RegistryIdentityReservation) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("dimension", reservation.dimension.as_str()),
        text_entry("identity_digest", reservation.identity_digest.as_str()),
        text_entry("status", reservation.status.as_str()),
        text_entry("project_id", reservation.project_id.as_str()),
    ])
}

fn identity_collision(
    candidate: &RepositoryObservation,
    existing: &RepositoryObservation,
) -> Option<IdentityDimension> {
    if candidate.canonical_root_identity_digest == existing.canonical_root_identity_digest {
        return Some(IdentityDimension::CanonicalRoot);
    }
    if candidate.repository_identity_digest == existing.repository_identity_digest {
        return Some(IdentityDimension::Repository);
    }
    if candidate.file_identity_digest == existing.file_identity_digest {
        return Some(IdentityDimension::File);
    }
    None
}

#[derive(Clone, Copy)]
struct AuthorityTransition<'a> {
    name: &'a str,
    previous_head: Option<&'a ProjectAuthorityHead>,
    evidence_digest: Option<&'a ContentDigest>,
}

fn issue_authority(
    runtime: RuntimeKind,
    project_id: &ProjectId,
    project_class: ProjectClass,
    registry_revision: u64,
    lifecycle: ProjectLifecycle,
    observation: &RepositoryObservation,
    transition: AuthorityTransition<'_>,
) -> Result<ProjectAuthorityReceipt, RegistryError> {
    let snapshot_id = ProjectSnapshotId::new(format!(
        "{}:registry:{registry_revision}:{}",
        project_id.as_str(),
        observation.digest().as_str()
    ))?;
    let receipt_value = CanonicalValue::Object(vec![
        text_entry("contract_version", &CONTRACT_VERSION.to_string()),
        text_entry("producer_id", PROJECT_AUTHORITY_PRODUCER_ID),
        text_entry("producer_version", PROJECT_AUTHORITY_PRODUCER_VERSION),
        text_entry("runtime", registry_runtime_text(runtime)),
        text_entry("project_id", project_id.as_str()),
        text_entry("project_snapshot_id", snapshot_id.as_str()),
        text_entry("registry_revision", &registry_revision.to_string()),
        text_entry("lifecycle", lifecycle.as_str()),
        text_entry("project_class", project_class.as_str()),
        text_entry("transition", transition.name),
        optional_head_entry("previous_head", transition.previous_head),
        (
            "transition_evidence_digest".to_owned(),
            transition
                .evidence_digest
                .map_or(CanonicalValue::Null, |digest| {
                    CanonicalValue::String(digest.as_str().to_owned())
                }),
        ),
        text_entry("primary_ref", observation.primary_branch.reference()),
        text_entry(
            "primary_ref_storage_identity_digest",
            observation
                .primary_branch
                .storage_identity_digest()
                .as_str(),
        ),
        text_entry("observation_digest", observation.digest().as_str()),
    ]);
    let receipt_digest =
        registry_digest("lattice.project-registry.authority-receipt", &receipt_value)?;
    ProjectAuthorityReceipt::new(
        CONTRACT_VERSION,
        PROJECT_AUTHORITY_PRODUCER_ID,
        PROJECT_AUTHORITY_PRODUCER_VERSION,
        runtime,
        project_id.clone(),
        snapshot_id,
        registry_revision,
        lifecycle,
        project_class,
        observation.primary_branch.clone(),
        observation.digest.clone(),
        receipt_digest,
    )
    .map_err(RegistryError::from)
}

fn observation_value(
    canonical_root: &str,
    root_identity: &ContentDigest,
    repository_identity: &ContentDigest,
    file_identity: &ContentDigest,
    primary_branch: &GitRefIdentity,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("canonical_root", canonical_root),
        text_entry("canonical_root_identity_digest", root_identity.as_str()),
        text_entry("repository_identity_digest", repository_identity.as_str()),
        text_entry("file_identity_digest", file_identity.as_str()),
        text_entry("primary_ref", primary_branch.reference()),
        text_entry(
            "primary_ref_storage_identity_digest",
            primary_branch.storage_identity_digest().as_str(),
        ),
    ])
}

fn command_result_digest(
    command_id: &CommandId,
    request_digest: &ContentDigest,
    before: Option<&ProjectAuthorityHead>,
    after: Option<&ProjectAuthorityHead>,
    outcome: &RegistryCommandOutcome,
    drift: &[IdentityDrift],
    authority: Option<&ProjectAuthorityReceipt>,
) -> Result<ContentDigest, RegistryError> {
    registry_digest(
        "lattice.project-registry.command-result",
        &CanonicalValue::Object(vec![
            text_entry("command_id", command_id.as_str()),
            text_entry("request_digest", request_digest.as_str()),
            optional_head_entry("before", before),
            optional_head_entry("after", after),
            ("outcome".to_owned(), outcome_value(outcome)),
            (
                "drift".to_owned(),
                CanonicalValue::Array(
                    drift
                        .iter()
                        .map(|item| CanonicalValue::String(item.as_str().to_owned()))
                        .collect(),
                ),
            ),
            (
                "authority_receipt_digest".to_owned(),
                authority.map_or(CanonicalValue::Null, |receipt| {
                    CanonicalValue::String(receipt.receipt_digest().as_str().to_owned())
                }),
            ),
        ]),
    )
}

fn authority_head_value(head: &ProjectAuthorityHead) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("producer_id", head.producer_id()),
        text_entry("producer_version", head.producer_version()),
        text_entry(
            "runtime",
            if head.runtime().is_live() {
                "LIVE"
            } else {
                "FAKE"
            },
        ),
        text_entry("project_id", head.project_id().as_str()),
        text_entry("project_snapshot_id", head.project_snapshot_id().as_str()),
        text_entry("registry_revision", &head.registry_revision().to_string()),
        text_entry("lifecycle", head.lifecycle().as_str()),
        text_entry("project_class", head.project_class().as_str()),
        text_entry("primary_ref", head.primary_branch().reference()),
        text_entry(
            "primary_ref_storage_identity_digest",
            head.primary_branch().storage_identity_digest().as_str(),
        ),
        text_entry("observation_digest", head.observation_digest().as_str()),
        text_entry("receipt_digest", head.receipt_digest().as_str()),
    ])
}

fn optional_head_entry(
    name: &str,
    head: Option<&ProjectAuthorityHead>,
) -> (String, CanonicalValue) {
    (
        name.to_owned(),
        head.map_or(CanonicalValue::Null, authority_head_value),
    )
}

fn outcome_value(outcome: &RegistryCommandOutcome) -> CanonicalValue {
    match outcome {
        RegistryCommandOutcome::Applied => {
            CanonicalValue::Object(vec![text_entry("status", "APPLIED")])
        }
        RegistryCommandOutcome::Denied(denial) => denial_outcome_value("DENIED", denial),
        RegistryCommandOutcome::Blocked(denial) => denial_outcome_value("BLOCKED", denial),
    }
}

fn denial_outcome_value(status: &str, denial: &RegistryDenial) -> CanonicalValue {
    match denial {
        RegistryDenial::DuplicateIdentity {
            dimension,
            existing_project_id,
        } => CanonicalValue::Object(vec![
            text_entry("status", status),
            text_entry("reason", "DUPLICATE_IDENTITY"),
            text_entry("dimension", dimension.as_str()),
            text_entry("existing_project_id", existing_project_id.as_str()),
        ]),
        RegistryDenial::UnknownProject => terminal_outcome_value(status, "UNKNOWN_PROJECT"),
        RegistryDenial::StaleHead => terminal_outcome_value(status, "STALE_HEAD"),
        RegistryDenial::LifecycleBlocked { lifecycle } => CanonicalValue::Object(vec![
            text_entry("status", status),
            text_entry("reason", "LIFECYCLE_BLOCKED"),
            text_entry("lifecycle", lifecycle.as_str()),
        ]),
        RegistryDenial::ReconciliationDecisionMismatch { expected, found } => {
            CanonicalValue::Object(vec![
                text_entry("status", status),
                text_entry("reason", "RECONCILIATION_DECISION_MISMATCH"),
                text_entry("expected", expected.as_str()),
                text_entry("found", found.as_str()),
            ])
        }
        RegistryDenial::PendingObservationMismatch => {
            terminal_outcome_value(status, "PENDING_OBSERVATION_MISMATCH")
        }
        RegistryDenial::RevisionOverflow => terminal_outcome_value(status, "REVISION_OVERFLOW"),
    }
}

fn identity_drift(
    accepted: &RepositoryObservation,
    observed: &RepositoryObservation,
) -> Vec<IdentityDrift> {
    let mut drift = Vec::new();
    if accepted.canonical_root != observed.canonical_root
        || accepted.canonical_root_identity_digest != observed.canonical_root_identity_digest
    {
        drift.push(IdentityDrift::CanonicalRoot);
    }
    if accepted.repository_identity_digest != observed.repository_identity_digest {
        drift.push(IdentityDrift::Repository);
    }
    if accepted.file_identity_digest != observed.file_identity_digest {
        drift.push(IdentityDrift::File);
    }
    if accepted.primary_branch.reference() != observed.primary_branch.reference() {
        drift.push(IdentityDrift::PrimaryRefName);
    }
    if accepted.primary_branch.storage_identity_digest()
        != observed.primary_branch.storage_identity_digest()
    {
        drift.push(IdentityDrift::PrimaryRefStorage);
    }
    drift
}

fn terminal_outcome_value(status: &str, reason: &str) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("status", status),
        text_entry("reason", reason),
    ])
}

fn vacant_registry_logical_state(runtime: RuntimeKind) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("schema_version", "1"),
        text_entry("runtime", registry_runtime_text(runtime)),
        ("observations".to_owned(), CanonicalValue::Array(Vec::new())),
        ("projects".to_owned(), CanonicalValue::Array(Vec::new())),
        ("commands".to_owned(), CanonicalValue::Array(Vec::new())),
        ("reservations".to_owned(), CanonicalValue::Array(Vec::new())),
    ])
}

fn vacant_registry_checkpoint_value(
    runtime: RuntimeKind,
    retained_bytes: u64,
    logical_state: CanonicalValue,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        text_entry("schema_version", "1"),
        text_entry("runtime", registry_runtime_text(runtime)),
        text_entry("command_ordinal", "0"),
        text_entry("observation_count", "0"),
        text_entry("project_count", "0"),
        text_entry("command_count", "0"),
        text_entry("reservation_count", "0"),
        text_entry("retained_bytes", &retained_bytes.to_string()),
        ("logical_state".to_owned(), logical_state),
    ])
}

const fn registry_runtime_text(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Fake => "FAKE",
        RuntimeKind::Live => "LIVE",
    }
}

fn text_entry(name: &str, value: &str) -> (String, CanonicalValue) {
    (name.to_owned(), CanonicalValue::String(value.to_owned()))
}

fn registry_digest(
    schema_id: &str,
    value: &CanonicalValue,
) -> Result<ContentDigest, RegistryError> {
    let domain = HashDomain::new(schema_id, "1")?;
    let digest = canonical_sha256(&domain, value)?.to_hex();
    ContentDigest::from_sha256(digest).map_err(RegistryError::from)
}

fn ensure_registry_limits(
    project_count: usize,
    command_count: usize,
    retained_bytes: u64,
) -> Result<(), RegistryError> {
    if project_count > MAX_REGISTRY_PROJECTS
        || command_count > MAX_REGISTRY_COMMANDS
        || retained_bytes > MAX_REGISTRY_RETAINED_BYTES
    {
        Err(RegistryError::CapacityExceeded)
    } else {
        Ok(())
    }
}

fn next_registry_ordinal(current: u64) -> Result<u64, RegistryError> {
    current
        .checked_add(1)
        .filter(|value| i64::try_from(*value).is_ok())
        .ok_or(RegistryError::CommandOrdinalOverflow)
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn registry_limits_accept_exact_and_reject_each_plus_one() {
        assert_eq!(
            ensure_registry_limits(
                MAX_REGISTRY_PROJECTS,
                MAX_REGISTRY_COMMANDS,
                MAX_REGISTRY_RETAINED_BYTES,
            ),
            Ok(())
        );
        assert_eq!(
            ensure_registry_limits(
                MAX_REGISTRY_PROJECTS + 1,
                MAX_REGISTRY_COMMANDS,
                MAX_REGISTRY_RETAINED_BYTES,
            ),
            Err(RegistryError::CapacityExceeded)
        );
        assert_eq!(
            ensure_registry_limits(
                MAX_REGISTRY_PROJECTS,
                MAX_REGISTRY_COMMANDS + 1,
                MAX_REGISTRY_RETAINED_BYTES,
            ),
            Err(RegistryError::CapacityExceeded)
        );
        assert_eq!(
            ensure_registry_limits(
                MAX_REGISTRY_PROJECTS,
                MAX_REGISTRY_COMMANDS,
                MAX_REGISTRY_RETAINED_BYTES + 1,
            ),
            Err(RegistryError::CapacityExceeded)
        );
    }

    #[test]
    fn registry_ordinal_accepts_signed_bigint_max_and_rejects_advance() {
        assert_eq!(
            next_registry_ordinal((i64::MAX as u64) - 1),
            Ok(i64::MAX as u64)
        );
        assert_eq!(
            next_registry_ordinal(i64::MAX as u64),
            Err(RegistryError::CommandOrdinalOverflow)
        );
    }
}
