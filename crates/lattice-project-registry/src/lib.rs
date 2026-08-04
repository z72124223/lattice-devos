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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    const fn as_str(self) -> &'static str {
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
    const fn as_str(self) -> &'static str {
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
    const fn as_str(self) -> &'static str {
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

    fn command_id(&self) -> &CommandId {
        match self {
            Self::Register { command_id, .. }
            | Self::Observe { command_id, .. }
            | Self::Suspend { command_id, .. }
            | Self::Reconcile { command_id, .. } => command_id,
        }
    }

    fn request_digest(&self) -> Result<ContentDigest, RegistryError> {
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
enum UntrustedRegistryRows {
    Vacant,
}

/// Exports one complete verified Registry through the untrusted persistence
/// boundary.
#[must_use]
pub fn export_untrusted_registry_snapshot(
    state: &VerifiedRegistryState,
) -> UntrustedRegistrySnapshot {
    UntrustedRegistrySnapshot {
        claimed_checkpoint: state.checkpoint.clone(),
        rows: UntrustedRegistryRows::Vacant,
    }
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
    match &snapshot.rows {
        UntrustedRegistryRows::Vacant => {
            let verified = VerifiedRegistryState::vacant(snapshot.claimed_checkpoint.runtime())?;
            if verified.checkpoint() != &snapshot.claimed_checkpoint {
                return Err(RegistryError::CorruptSnapshot);
            }
            Ok(verified)
        }
    }
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

#[derive(Clone, Debug)]
struct ProjectRecord {
    project_class: ProjectClass,
    observation: RepositoryObservation,
    pending_observation: Option<RepositoryObservation>,
    drift: Vec<IdentityDrift>,
    authority: ProjectAuthorityReceipt,
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
#[derive(Debug, Default)]
pub struct FakeProjectRegistry {
    projects: BTreeMap<ProjectId, ProjectRecord>,
    commands: BTreeMap<CommandId, RegistryCommandReceipt>,
}

impl FakeProjectRegistry {
    /// Creates an empty fake Registry. It is not durable truth.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            projects: BTreeMap::new(),
            commands: BTreeMap::new(),
        }
    }

    /// Executes one idempotent command.
    ///
    /// # Errors
    ///
    /// Returns an error for command-ID substitution or invalid canonical
    /// receipt/request construction. Domain denials are terminal receipts.
    pub fn execute(
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

    /// Returns the latest fake authority receipt for one registered project.
    #[must_use]
    pub fn latest(&self, project_id: &ProjectId) -> Option<&ProjectAuthorityReceipt> {
        self.projects
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
        let record = self.projects.get(project_id)?;
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
            ProjectRecord {
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
        text_entry("runtime", "FAKE"),
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
        RuntimeKind::Fake,
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
