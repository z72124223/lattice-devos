//! Pure Writer Lease 1.0 semantics and a deterministic non-durable fake.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, ContentDigest, DaemonEpoch, FencingToken, HolderProcessId,
    ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind, TaskId,
    WRITER_LEASE_PRODUCER_ID, WRITER_LEASE_PRODUCER_VERSION, WriterLeaseAuthorityHead,
    WriterLeaseAuthorityReceipt, WriterLeaseIdentity, WriterLeaseRevision, WriterLeaseStatus,
};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

const SNAPSHOT_VERSION: &str = "1.0";
const MAX_SIGNED_BIGINT: u64 = i64::MAX as u64;
/// Maximum accepted canonical snapshot payload at the persistence boundary.
pub const MAX_CANONICAL_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CANONICAL_NESTING_DEPTH: usize = 128;

/// One stable Writer Lease denial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseDenial {
    /// The supplied complete expected authority head is not current.
    StaleHead,
    /// Another active or suspect writer already reserves the project.
    WriterAlreadyHeld,
    /// The requested transition requires a current lease.
    LeaseVacant,
    /// The requested transition is illegal for the current lease state.
    InvalidState,
    /// Runtime admission forbids this transition.
    AdmissionDenied,
    /// Fake/live owner identity changed within one lease.
    RuntimeMismatch,
    /// A heartbeat arrived at or after expiry, or did not advance time.
    HeartbeatRejected,
    /// The lease has not reached expiry and cannot become suspect.
    NotExpired,
    /// Recovery evidence does not bind the exact suspect lease.
    RecoveryEvidenceMismatch,
    /// A signed `BIGINT` fence or revision would overflow.
    CounterExhausted,
}

impl LeaseDenial {
    /// Returns the stable receipt-facing denial code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StaleHead => "STALE_HEAD",
            Self::WriterAlreadyHeld => "WRITER_ALREADY_HELD",
            Self::LeaseVacant => "LEASE_VACANT",
            Self::InvalidState => "INVALID_STATE",
            Self::AdmissionDenied => "ADMISSION_DENIED",
            Self::RuntimeMismatch => "RUNTIME_MISMATCH",
            Self::HeartbeatRejected => "HEARTBEAT_REJECTED",
            Self::NotExpired => "NOT_EXPIRED",
            Self::RecoveryEvidenceMismatch => "RECOVERY_EVIDENCE_MISMATCH",
            Self::CounterExhausted => "COUNTER_EXHAUSTED",
        }
    }
}

/// Writer Lease construction, planning, or verification failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterLeaseError {
    /// A command identifier violates the exact bounded ASCII contract.
    InvalidCommandId,
    /// A timestamp is not exact canonical UTC RFC 3339.
    InvalidTimestamp,
    /// Typed recovery evidence contains malformed identity text.
    InvalidRecoveryEvidence,
    /// An expiry does not follow its observation.
    InvalidExpiry,
    /// A required evidence digest is the all-zero sentinel.
    ZeroEvidenceDigest,
    /// A command names another aggregate/project.
    ProjectMismatch,
    /// A command identifier was reused with changed canonical content.
    CommandIdReuse,
    /// A fake owner was asked to issue live authority.
    FakeRuntimeRequired,
    /// A plan no longer applies to the aggregate it was planned against.
    PlanPreconditionChanged,
    /// Restore attempted to overwrite an already-retained project aggregate.
    RestoreWouldOverwrite,
    /// A verified snapshot disagrees with an independently retained checkpoint.
    CheckpointMismatch,
    /// Shared-contract construction failed.
    Contract,
    /// Canonical hashing failed.
    Canonical,
    /// An untrusted aggregate snapshot failed replay verification.
    CorruptSnapshot,
}

impl WriterLeaseError {
    /// Returns a stable machine-readable error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidCommandId => "WRITER_LEASE_INVALID_COMMAND_ID",
            Self::InvalidTimestamp => "WRITER_LEASE_INVALID_TIMESTAMP",
            Self::InvalidRecoveryEvidence => "WRITER_LEASE_INVALID_RECOVERY_EVIDENCE",
            Self::InvalidExpiry => "WRITER_LEASE_INVALID_EXPIRY",
            Self::ZeroEvidenceDigest => "WRITER_LEASE_ZERO_EVIDENCE_DIGEST",
            Self::ProjectMismatch => "WRITER_LEASE_PROJECT_MISMATCH",
            Self::CommandIdReuse => "WRITER_LEASE_COMMAND_ID_REUSE",
            Self::FakeRuntimeRequired => "WRITER_LEASE_FAKE_RUNTIME_REQUIRED",
            Self::PlanPreconditionChanged => "WRITER_LEASE_PLAN_PRECONDITION_CHANGED",
            Self::RestoreWouldOverwrite => "WRITER_LEASE_RESTORE_WOULD_OVERWRITE",
            Self::CheckpointMismatch => "WRITER_LEASE_CHECKPOINT_MISMATCH",
            Self::Contract => "WRITER_LEASE_CONTRACT",
            Self::Canonical => "WRITER_LEASE_CANONICAL",
            Self::CorruptSnapshot => "WRITER_LEASE_CORRUPT_SNAPSHOT",
        }
    }
}

impl fmt::Display for WriterLeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for WriterLeaseError {}

/// Stable persistence-boundary failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterLeaseRepositoryErrorKind {
    /// Pure Writer Lease validation or planning rejected the request.
    Domain,
    /// The durable owner could not be reached or admitted.
    Unavailable,
    /// Bounded serialization retries were exhausted.
    SerializationExhausted,
    /// `PostgreSQL` may have committed but the result could not be observed.
    CommitOutcomeUnknown,
    /// Durable history, checkpoint, catalog, or projection is corrupt.
    Corrupt,
    /// The independently loaded current authority does not match.
    AuthorityMismatch,
}

/// Closed error returned by a durable Writer Lease repository.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WriterLeaseRepositoryError {
    kind: WriterLeaseRepositoryErrorKind,
    domain: Option<WriterLeaseError>,
}

impl WriterLeaseRepositoryError {
    /// Constructs one non-domain repository failure.
    #[must_use]
    pub const fn new(kind: WriterLeaseRepositoryErrorKind) -> Self {
        Self { kind, domain: None }
    }

    /// Wraps an exact pure-domain failure without changing its meaning.
    #[must_use]
    pub const fn from_domain(error: WriterLeaseError) -> Self {
        Self {
            kind: WriterLeaseRepositoryErrorKind::Domain,
            domain: Some(error),
        }
    }

    /// Returns the stable failure class.
    #[must_use]
    pub const fn kind(self) -> WriterLeaseRepositoryErrorKind {
        self.kind
    }

    /// Returns the underlying pure-domain failure when present.
    #[must_use]
    pub const fn domain(self) -> Option<WriterLeaseError> {
        self.domain
    }

    /// Returns a stable non-secret machine code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            WriterLeaseRepositoryErrorKind::Domain => "WRITER_LEASE_REPOSITORY_DOMAIN",
            WriterLeaseRepositoryErrorKind::Unavailable => "WRITER_LEASE_REPOSITORY_UNAVAILABLE",
            WriterLeaseRepositoryErrorKind::SerializationExhausted => {
                "WRITER_LEASE_REPOSITORY_SERIALIZATION_EXHAUSTED"
            }
            WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown => {
                "WRITER_LEASE_REPOSITORY_COMMIT_OUTCOME_UNKNOWN"
            }
            WriterLeaseRepositoryErrorKind::Corrupt => "WRITER_LEASE_REPOSITORY_CORRUPT",
            WriterLeaseRepositoryErrorKind::AuthorityMismatch => {
                "WRITER_LEASE_REPOSITORY_AUTHORITY_MISMATCH"
            }
        }
    }
}

impl fmt::Display for WriterLeaseRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for WriterLeaseRepositoryError {}

/// Exact injected owner observation. No clock, process, or runtime is read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseObservation {
    pub runtime: RuntimeKind,
    pub admission: RuntimeAdmissionMode,
    pub observed_at: String,
    pub time_observation_digest: ContentDigest,
    pub admission_observation_digest: ContentDigest,
}

/// Complete requested holder identity before the owner allocates a fence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquireClaim {
    pub project_id: ProjectId,
    pub project_snapshot_id: ProjectSnapshotId,
    pub task_id: TaskId,
    pub task_revision: String,
    pub task_spec_digest: ContentDigest,
    pub attempt_id: AttemptId,
    pub lease_id: String,
    pub lease_holder_id: String,
    pub worktree_id: String,
    pub holder_process_id: HolderProcessId,
    pub holder_process_start_identity: ContentDigest,
    pub daemon_instance_id: String,
    pub daemon_epoch: DaemonEpoch,
}

/// Acquire a vacant project writer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcquireCommand {
    pub command_id: String,
    pub expected_head: Option<WriterLeaseAuthorityHead>,
    pub claim: AcquireClaim,
    pub observation: LeaseObservation,
    pub expires_at: String,
}

/// Advance the heartbeat and expiry of the exact active lease.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeartbeatCommand {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
    pub observation: LeaseObservation,
    pub expires_at: String,
}

/// Mark an expired active lease suspect without revoking it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkSuspectCommand {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
    pub observation: LeaseObservation,
}

/// Release the exact active or suspect holder voluntarily.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseCommand {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
    pub observation: LeaseObservation,
}

/// Transfer one exact active or suspect logical lease to a replacement OS
/// process after the predecessor process is proven dead.
///
/// This is a process-supervision handoff, not a new writer acquisition: the
/// project, task, attempt, lease, worktree, daemon leadership, and fencing
/// token remain unchanged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessHandoffCommand {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
    pub successor_holder_process_id: HolderProcessId,
    pub successor_holder_process_start_identity: ContentDigest,
    pub successor_daemon_instance_id: String,
    pub successor_daemon_epoch: DaemonEpoch,
    pub observation: LeaseObservation,
    pub expires_at: String,
    pub evidence: RecoveryEvidence,
}

/// Typed recovery evidence for a suspect holder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryEvidence {
    /// The exact PID and process-start identity are proven dead.
    ProcessDeath {
        holder_process_id: HolderProcessId,
        holder_process_start_identity: ContentDigest,
        holder_daemon_instance_id: String,
        evidence_digest: ContentDigest,
    },
    /// The holder's daemon leadership was replaced by a strictly newer epoch.
    LeadershipReplaced {
        replaced_daemon_instance_id: String,
        replaced_epoch: DaemonEpoch,
        replacement_daemon_instance_id: String,
        replacement_epoch: DaemonEpoch,
        evidence_digest: ContentDigest,
    },
}

impl RecoveryEvidence {
    fn evidence_digest(&self) -> &ContentDigest {
        match self {
            Self::ProcessDeath {
                evidence_digest, ..
            }
            | Self::LeadershipReplaced {
                evidence_digest, ..
            } => evidence_digest,
        }
    }
}

/// Revoke one exact suspect lease using typed recovery evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevokeCommand {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
    pub observation: LeaseObservation,
    pub evidence: RecoveryEvidence,
}

/// Closed Writer Lease command set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WriterLeaseCommand {
    Acquire(AcquireCommand),
    Heartbeat(HeartbeatCommand),
    MarkSuspect(MarkSuspectCommand),
    ProcessHandoff(ProcessHandoffCommand),
    Release(ReleaseCommand),
    Revoke(RevokeCommand),
}

impl WriterLeaseCommand {
    /// Returns the exact idempotency command identifier.
    #[must_use]
    pub fn command_id(&self) -> &str {
        match self {
            Self::Acquire(command) => &command.command_id,
            Self::Heartbeat(command) => &command.command_id,
            Self::MarkSuspect(command) => &command.command_id,
            Self::ProcessHandoff(command) => &command.command_id,
            Self::Release(command) => &command.command_id,
            Self::Revoke(command) => &command.command_id,
        }
    }

    /// Returns the exact project aggregate identity.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        match self {
            Self::Acquire(command) => &command.claim.project_id,
            Self::Heartbeat(command) => &command.project_id,
            Self::MarkSuspect(command) => &command.project_id,
            Self::ProcessHandoff(command) => &command.project_id,
            Self::Release(command) => &command.project_id,
            Self::Revoke(command) => &command.project_id,
        }
    }

    /// Returns the owner observation bound to this command.
    #[must_use]
    pub const fn observation(&self) -> &LeaseObservation {
        match self {
            Self::Acquire(command) => &command.observation,
            Self::Heartbeat(command) => &command.observation,
            Self::MarkSuspect(command) => &command.observation,
            Self::ProcessHandoff(command) => &command.observation,
            Self::Release(command) => &command.observation,
            Self::Revoke(command) => &command.observation,
        }
    }

    /// Returns the complete expected current head when required.
    #[must_use]
    pub const fn expected_head(&self) -> Option<&WriterLeaseAuthorityHead> {
        match self {
            Self::Acquire(command) => command.expected_head.as_ref(),
            Self::Heartbeat(command) => Some(&command.expected_head),
            Self::MarkSuspect(command) => Some(&command.expected_head),
            Self::ProcessHandoff(command) => Some(&command.expected_head),
            Self::Release(command) => Some(&command.expected_head),
            Self::Revoke(command) => Some(&command.expected_head),
        }
    }

    /// Exports the exact canonical pure-command bytes retained in one command
    /// receipt.
    ///
    /// # Errors
    ///
    /// Returns a canonicalization failure without changing the command.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WriterLeaseError> {
        canonicalize(&command_value(self))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(|_| WriterLeaseError::Canonical)
    }

    /// Reconstructs the caller-only repository intent bytes from one persisted
    /// live command. Database observation, expiry, daemon allocation, and a
    /// newly allocated fence remain excluded.
    ///
    /// # Errors
    ///
    /// Rejects fake commands and canonicalization failures.
    pub fn repository_intent_canonical_bytes(&self) -> Result<Vec<u8>, WriterLeaseError> {
        if !self.observation().runtime.is_live() {
            return Err(WriterLeaseError::FakeRuntimeRequired);
        }
        repository_command_from_live_command(self).canonical_bytes()
    }
}

/// Caller-owned acquire intent for a durable live repository.
///
/// `PostgreSQL` time, runtime admission, daemon identity/epoch, expiry, and the
/// fencing token are deliberately absent. The repository observes or allocates
/// them inside the same transaction that commits the pure owner plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseAcquireRequest {
    pub command_id: String,
    pub expected_head: Option<WriterLeaseAuthorityHead>,
    pub project_id: ProjectId,
    pub project_snapshot_id: ProjectSnapshotId,
    pub task_id: TaskId,
    pub task_revision: String,
    pub task_spec_digest: ContentDigest,
    pub attempt_id: AttemptId,
    pub lease_id: String,
    pub lease_holder_id: String,
    pub worktree_id: String,
    pub holder_process_id: HolderProcessId,
    pub holder_process_start_identity: ContentDigest,
}

/// Caller-owned heartbeat intent; time and expiry remain repository-owned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseHeartbeatRequest {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
}

/// Caller-owned request to mark one exact expired lease suspect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseMarkSuspectRequest {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
}

/// Caller-owned exact release intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseReleaseRequest {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
}

/// Caller-owned exact process-handoff intent.
///
/// Database time, admission, expiry, and the retained daemon identity are
/// repository-owned. The caller supplies only the replacement process
/// identity and typed death evidence for the exact predecessor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseProcessHandoffRequest {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
    pub successor_holder_process_id: HolderProcessId,
    pub successor_holder_process_start_identity: ContentDigest,
    pub evidence: RecoveryEvidence,
}

/// Caller-owned evidence-bound revoke intent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseRevokeRequest {
    pub command_id: String,
    pub project_id: ProjectId,
    pub expected_head: WriterLeaseAuthorityHead,
    pub evidence: RecoveryEvidence,
}

/// Closed high-level command set accepted by a live durable repository.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WriterLeaseRepositoryCommand {
    Acquire(WriterLeaseAcquireRequest),
    Heartbeat(WriterLeaseHeartbeatRequest),
    MarkSuspect(WriterLeaseMarkSuspectRequest),
    ProcessHandoff(WriterLeaseProcessHandoffRequest),
    Release(WriterLeaseReleaseRequest),
    Revoke(WriterLeaseRevokeRequest),
}

impl WriterLeaseRepositoryCommand {
    /// Returns the exact caller-owned idempotency command identifier.
    #[must_use]
    pub fn command_id(&self) -> &str {
        match self {
            Self::Acquire(request) => &request.command_id,
            Self::Heartbeat(request) => &request.command_id,
            Self::MarkSuspect(request) => &request.command_id,
            Self::ProcessHandoff(request) => &request.command_id,
            Self::Release(request) => &request.command_id,
            Self::Revoke(request) => &request.command_id,
        }
    }

    /// Returns the exact project aggregate addressed by this command.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        match self {
            Self::Acquire(request) => &request.project_id,
            Self::Heartbeat(request) => &request.project_id,
            Self::MarkSuspect(request) => &request.project_id,
            Self::ProcessHandoff(request) => &request.project_id,
            Self::Release(request) => &request.project_id,
            Self::Revoke(request) => &request.project_id,
        }
    }

    /// Exports the byte-exact canonical caller intent used for durable
    /// repository idempotency.
    ///
    /// Database time, admission, daemon observation, expiry, and a newly
    /// allocated fencing token are deliberately absent. An adapter can retain
    /// these bytes beside the pure command receipt and, on retry, re-run the
    /// pure planner with the originally persisted live command.
    ///
    /// # Errors
    ///
    /// Returns a canonicalization failure without changing the request.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WriterLeaseError> {
        canonicalize(&repository_command_value(self))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(|_| WriterLeaseError::Canonical)
    }
}

/// Applied transition kind retained in replay evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionKind {
    Acquire,
    Heartbeat,
    MarkSuspect,
    ProcessHandoff,
    Release,
    Revoke,
}

impl TransitionKind {
    /// Returns the stable physical persistence code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Acquire => "ACQUIRE",
            Self::Heartbeat => "HEARTBEAT",
            Self::MarkSuspect => "MARK_SUSPECT",
            Self::ProcessHandoff => "PROCESS_HANDOFF",
            Self::Release => "RELEASE",
            Self::Revoke => "REVOKE",
        }
    }
}

/// Applied or denied terminal command outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandOutcome {
    Applied,
    Denied(LeaseDenial),
}

/// Immutable applied transition record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseTransitionRecord {
    pub ordinal: u64,
    pub command_id: String,
    pub kind: TransitionKind,
    pub request_digest: ContentDigest,
    pub before: Option<WriterLeaseAuthorityHead>,
    pub after: Option<WriterLeaseAuthorityReceipt>,
    pub transition_digest: ContentDigest,
}

impl WriterLeaseTransitionRecord {
    /// Exports the exact canonical physical transition bytes.
    ///
    /// # Errors
    ///
    /// Returns a canonicalization failure without changing the record.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WriterLeaseError> {
        canonicalize(&transition_record_value(self))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(|_| WriterLeaseError::Canonical)
    }
}

/// Immutable terminal command receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseCommandReceipt {
    pub ordinal: u64,
    pub previous_receipt_digest: Option<ContentDigest>,
    pub request: WriterLeaseCommand,
    pub request_digest: ContentDigest,
    pub before: Option<WriterLeaseAuthorityHead>,
    pub after: Option<WriterLeaseAuthorityHead>,
    pub outcome: CommandOutcome,
    pub transition_digest: Option<ContentDigest>,
    pub receipt_digest: ContentDigest,
}

impl WriterLeaseCommandReceipt {
    /// Exports the exact canonical physical receipt bytes.
    ///
    /// # Errors
    ///
    /// Returns a canonicalization failure without changing the receipt.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WriterLeaseError> {
        canonicalize(&command_receipt_value(self))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(|_| WriterLeaseError::Canonical)
    }
}

/// Complete raw persistence payload. No nested field is trusted until replayed.
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedWriterLeaseSnapshot {
    /// Raw canonical-value payload supplied by a future persistence adapter.
    pub payload: CanonicalValue,
}

/// Independently retained commitment to one complete verified aggregate.
///
/// A persistence adapter must source this value from its trusted current-row
/// boundary rather than deriving it from the untrusted snapshot being checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseCheckpoint {
    project_id: ProjectId,
    command_high_water: u64,
    command_tail_digest: Option<ContentDigest>,
    snapshot_digest: ContentDigest,
}

impl WriterLeaseCheckpoint {
    /// Constructs one independently stored checkpoint from persistence-row
    /// columns.
    ///
    /// # Errors
    ///
    /// Rejects out-of-range command high-water values, a missing/non-missing
    /// tail mismatch, or an all-zero commitment digest.
    pub fn new(
        project_id: ProjectId,
        command_high_water: u64,
        command_tail_digest: Option<ContentDigest>,
        snapshot_digest: ContentDigest,
    ) -> Result<Self, WriterLeaseError> {
        if command_high_water > MAX_SIGNED_BIGINT
            || (command_high_water == 0) != command_tail_digest.is_none()
            || command_tail_digest.as_ref().is_some_and(is_zero_digest)
            || is_zero_digest(&snapshot_digest)
        {
            return Err(WriterLeaseError::CheckpointMismatch);
        }
        Ok(Self {
            project_id,
            command_high_water,
            command_tail_digest,
            snapshot_digest,
        })
    }

    /// Returns the checkpointed project.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the number of terminal command receipts committed by the owner.
    #[must_use]
    pub const fn command_high_water(&self) -> u64 {
        self.command_high_water
    }

    /// Returns the digest-chain tail, or `None` for an empty aggregate.
    #[must_use]
    pub const fn command_tail_digest(&self) -> Option<&ContentDigest> {
        self.command_tail_digest.as_ref()
    }

    /// Returns the digest of the complete raw snapshot projection.
    #[must_use]
    pub const fn snapshot_digest(&self) -> &ContentDigest {
        &self.snapshot_digest
    }
}

impl fmt::Debug for UntrustedWriterLeaseSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedWriterLeaseSnapshot")
            .field("raw_fields", &"[ELIDED]")
            .finish_non_exhaustive()
    }
}

impl UntrustedWriterLeaseSnapshot {
    /// Parses one byte-exact `lattice-cjson-1` snapshot payload.
    ///
    /// The parser preserves duplicate object entries, accepts no JSON number,
    /// and re-canonicalizes before running the pure snapshot verifier. Thus
    /// whitespace, alternate escaping, key reordering, duplicate keys,
    /// trailing bytes, and malformed UTF-8 all fail closed.
    ///
    /// # Errors
    ///
    /// Returns [`WriterLeaseError::CorruptSnapshot`] for any non-canonical or
    /// semantically invalid payload.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, WriterLeaseError> {
        if bytes.is_empty() || bytes.len() > MAX_CANONICAL_SNAPSHOT_BYTES {
            return Err(WriterLeaseError::CorruptSnapshot);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| WriterLeaseError::CorruptSnapshot)?;
        let payload = CanonicalJsonParser::new(text)
            .parse()
            .map_err(|()| WriterLeaseError::CorruptSnapshot)?;
        let canonical = canonicalize(&payload).map_err(|_| WriterLeaseError::CorruptSnapshot)?;
        if canonical.as_slice() != bytes {
            return Err(WriterLeaseError::CorruptSnapshot);
        }
        let snapshot = Self { payload };
        verify_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    /// Returns the exact canonical byte representation.
    ///
    /// # Errors
    ///
    /// Returns a canonicalization failure for duplicate normalized keys or an
    /// invalid value tree, and rejects output beyond the durable boundary.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, WriterLeaseError> {
        let bytes = canonicalize(&self.payload)
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(|_| WriterLeaseError::Canonical)?;
        if bytes.len() > MAX_CANONICAL_SNAPSHOT_BYTES {
            return Err(WriterLeaseError::CorruptSnapshot);
        }
        Ok(bytes)
    }
}

struct CanonicalJsonParser<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> CanonicalJsonParser<'a> {
    const fn new(input: &'a str) -> Self {
        Self { input, position: 0 }
    }

    fn parse(mut self) -> Result<CanonicalValue, ()> {
        let value = self.value(0)?;
        if self.position != self.input.len() {
            return Err(());
        }
        Ok(value)
    }

    fn value(&mut self, depth: usize) -> Result<CanonicalValue, ()> {
        if depth > MAX_CANONICAL_NESTING_DEPTH {
            return Err(());
        }
        match self.peek() {
            Some(b'n') => {
                self.literal("null")?;
                Ok(CanonicalValue::Null)
            }
            Some(b't') => {
                self.literal("true")?;
                Ok(CanonicalValue::Bool(true))
            }
            Some(b'f') => {
                self.literal("false")?;
                Ok(CanonicalValue::Bool(false))
            }
            Some(b'\"') => self.string().map(CanonicalValue::String),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            _ => Err(()),
        }
    }

    fn array(&mut self, depth: usize) -> Result<CanonicalValue, ()> {
        self.byte(b'[')?;
        let mut values = Vec::new();
        if self.take(b']') {
            return Ok(CanonicalValue::Array(values));
        }
        loop {
            values.push(self.value(depth + 1)?);
            if self.take(b']') {
                break;
            }
            self.byte(b',')?;
        }
        Ok(CanonicalValue::Array(values))
    }

    fn object(&mut self, depth: usize) -> Result<CanonicalValue, ()> {
        self.byte(b'{')?;
        let mut entries = Vec::new();
        if self.take(b'}') {
            return Ok(CanonicalValue::Object(entries));
        }
        loop {
            let key = self.string()?;
            self.byte(b':')?;
            entries.push((key, self.value(depth + 1)?));
            if self.take(b'}') {
                break;
            }
            self.byte(b',')?;
        }
        Ok(CanonicalValue::Object(entries))
    }

    fn string(&mut self) -> Result<String, ()> {
        self.byte(b'\"')?;
        let mut output = String::new();
        loop {
            let byte = self.peek().ok_or(())?;
            match byte {
                b'\"' => {
                    self.position += 1;
                    return Ok(output);
                }
                b'\\' => {
                    self.position += 1;
                    self.escape(&mut output)?;
                }
                0x00..=0x1f => return Err(()),
                _ => {
                    let character = self.input[self.position..].chars().next().ok_or(())?;
                    output.push(character);
                    self.position += character.len_utf8();
                }
            }
        }
    }

    fn escape(&mut self, output: &mut String) -> Result<(), ()> {
        let escaped = self.peek().ok_or(())?;
        self.position += 1;
        match escaped {
            b'\"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{8}'),
            b'f' => output.push('\u{c}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => self.unicode_escape(output)?,
            _ => return Err(()),
        }
        Ok(())
    }

    fn unicode_escape(&mut self, output: &mut String) -> Result<(), ()> {
        let first = self.hex_quad()?;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            self.byte(b'\\')?;
            self.byte(b'u')?;
            let second = self.hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(());
            }
            0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) {
            return Err(());
        } else {
            u32::from(first)
        };
        output.push(char::from_u32(scalar).ok_or(())?);
        Ok(())
    }

    fn hex_quad(&mut self) -> Result<u16, ()> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let digit = match self.peek().ok_or(())? {
                b'0'..=b'9' => u16::from(self.input.as_bytes()[self.position] - b'0'),
                b'a'..=b'f' => u16::from(self.input.as_bytes()[self.position] - b'a' + 10),
                b'A'..=b'F' => u16::from(self.input.as_bytes()[self.position] - b'A' + 10),
                _ => return Err(()),
            };
            self.position += 1;
            value = value
                .checked_mul(16)
                .and_then(|v| v.checked_add(digit))
                .ok_or(())?;
        }
        Ok(value)
    }

    fn literal(&mut self, value: &str) -> Result<(), ()> {
        if self.input[self.position..].starts_with(value) {
            self.position += value.len();
            Ok(())
        } else {
            Err(())
        }
    }

    fn byte(&mut self, expected: u8) -> Result<(), ()> {
        if self.take(expected) { Ok(()) } else { Err(()) }
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.position).copied()
    }
}

/// A fully replay-verified Writer Lease aggregate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedWriterLeaseAggregate {
    project_id: ProjectId,
    fencing_high_water: u64,
    revision: u64,
    current_receipt: Option<WriterLeaseAuthorityReceipt>,
    transitions: Vec<WriterLeaseTransitionRecord>,
    command_receipts: Vec<WriterLeaseCommandReceipt>,
}

impl VerifiedWriterLeaseAggregate {
    /// Creates one empty project aggregate.
    #[must_use]
    pub fn vacant(project_id: ProjectId) -> Self {
        Self {
            project_id,
            fencing_high_water: 0,
            revision: 0,
            current_receipt: None,
            transitions: Vec::new(),
            command_receipts: Vec::new(),
        }
    }

    /// Returns the aggregate project.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the last allocated fencing token, or zero before first acquire.
    #[must_use]
    pub const fn fencing_high_water(&self) -> u64 {
        self.fencing_high_water
    }

    /// Returns the last applied transition revision.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns the current active or suspect authority receipt.
    #[must_use]
    pub const fn current_receipt(&self) -> Option<&WriterLeaseAuthorityReceipt> {
        self.current_receipt.as_ref()
    }

    /// Returns the current structural authority head.
    #[must_use]
    pub fn current_head(&self) -> Option<WriterLeaseAuthorityHead> {
        self.current_receipt
            .as_ref()
            .map(WriterLeaseAuthorityReceipt::head)
    }

    /// Returns immutable transition records.
    #[must_use]
    pub fn transitions(&self) -> &[WriterLeaseTransitionRecord] {
        &self.transitions
    }

    /// Returns immutable terminal command receipts.
    #[must_use]
    pub fn command_receipts(&self) -> &[WriterLeaseCommandReceipt] {
        &self.command_receipts
    }

    /// Looks up one immutable historical authority receipt by its exact owner
    /// digest, including after the aggregate has been released.
    ///
    /// Only transition-produced authority receipts are eligible. A duplicate
    /// digest is impossible in a valid replay and therefore fails closed.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero sentinel or duplicate replay evidence.
    pub fn historical_authority_receipt(
        &self,
        receipt_digest: &ContentDigest,
    ) -> Result<Option<WriterLeaseAuthorityReceipt>, WriterLeaseError> {
        if is_zero_digest(receipt_digest) {
            return Err(WriterLeaseError::ZeroEvidenceDigest);
        }
        let matches = self
            .transitions
            .iter()
            .filter_map(|transition| transition.after.as_ref())
            .filter(|receipt| receipt.receipt_digest() == receipt_digest)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [receipt] => Ok(Some((*receipt).clone())),
            _ => Err(WriterLeaseError::CorruptSnapshot),
        }
    }

    /// Returns a commitment suitable for an independently trusted restore
    /// precondition.
    ///
    /// # Errors
    ///
    /// Returns a canonical hashing failure if the complete snapshot cannot be
    /// committed.
    pub fn checkpoint(&self) -> Result<WriterLeaseCheckpoint, WriterLeaseError> {
        WriterLeaseCheckpoint::new(
            self.project_id.clone(),
            u64::try_from(self.command_receipts.len())
                .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
            self.command_receipts
                .last()
                .map(|receipt| receipt.receipt_digest.clone()),
            snapshot_digest(self)?,
        )
    }

    /// Exports a complete untrusted persistence shape.
    #[must_use]
    pub fn export_untrusted(&self) -> UntrustedWriterLeaseSnapshot {
        UntrustedWriterLeaseSnapshot {
            payload: aggregate_value(self),
        }
    }

    /// Exports the exact canonical bytes persisted by a durable repository.
    ///
    /// # Errors
    ///
    /// Returns a canonicalization or bounded-size failure without changing the
    /// aggregate.
    pub fn export_canonical_bytes(&self) -> Result<Vec<u8>, WriterLeaseError> {
        self.export_untrusted().canonical_bytes()
    }
}

/// One current receipt paired with its independently loaded owner head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseCurrentAuthority {
    receipt: WriterLeaseAuthorityReceipt,
    independent_head: WriterLeaseAuthorityHead,
}

impl WriterLeaseCurrentAuthority {
    /// Constructs a current authority only when both independent shapes agree.
    ///
    /// # Errors
    ///
    /// Returns an authority-mismatch failure for a historical or substituted
    /// head.
    pub fn new(
        receipt: WriterLeaseAuthorityReceipt,
        independent_head: WriterLeaseAuthorityHead,
    ) -> Result<Self, WriterLeaseRepositoryError> {
        if receipt.head() != independent_head {
            return Err(WriterLeaseRepositoryError::new(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        Ok(Self {
            receipt,
            independent_head,
        })
    }

    /// Returns the exact current owner receipt.
    #[must_use]
    pub const fn receipt(&self) -> &WriterLeaseAuthorityReceipt {
        &self.receipt
    }

    /// Returns the independently loaded current head.
    #[must_use]
    pub const fn independent_head(&self) -> &WriterLeaseAuthorityHead {
        &self.independent_head
    }
}

/// Replay-verified state summary for one existing Writer Lease project.
///
/// This value deliberately distinguishes an existing released aggregate, whose
/// current authority is `None` but whose high-water marks remain durable, from
/// a repository lookup that found no project history at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseProjectEvidence {
    project_id: ProjectId,
    current_authority: Option<WriterLeaseCurrentAuthority>,
    fencing_high_water: u64,
    transition_high_water: u64,
    command_high_water: u64,
}

impl WriterLeaseProjectEvidence {
    /// Builds one evidence value from an already replay-verified aggregate.
    ///
    /// A persistence adapter must first verify its independent snapshot,
    /// checkpoint, current projection, and physical command/transition rows.
    /// This constructor performs no I/O and grants no writer authority.
    ///
    /// # Errors
    ///
    /// Returns a closed repository failure if the verified aggregate cannot
    /// produce a coherent checkpoint or current receipt/head pair.
    pub fn from_verified_aggregate(
        aggregate: &VerifiedWriterLeaseAggregate,
    ) -> Result<Self, WriterLeaseRepositoryError> {
        let current_authority = aggregate
            .current_receipt()
            .cloned()
            .map(|receipt| {
                let head = aggregate.current_head().ok_or_else(|| {
                    WriterLeaseRepositoryError::new(
                        WriterLeaseRepositoryErrorKind::AuthorityMismatch,
                    )
                })?;
                WriterLeaseCurrentAuthority::new(receipt, head)
            })
            .transpose()?;
        let checkpoint = aggregate
            .checkpoint()
            .map_err(WriterLeaseRepositoryError::from_domain)?;
        Ok(Self {
            project_id: aggregate.project_id().clone(),
            current_authority,
            fencing_high_water: aggregate.fencing_high_water(),
            transition_high_water: aggregate.revision(),
            command_high_water: checkpoint.command_high_water(),
        })
    }

    /// Returns the exact project whose history was replayed.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// Returns the current active/suspect authority, or `None` after release.
    #[must_use]
    pub const fn current_authority(&self) -> Option<&WriterLeaseCurrentAuthority> {
        self.current_authority.as_ref()
    }

    /// Returns the last fencing token ever allocated for this project.
    #[must_use]
    pub const fn fencing_high_water(&self) -> u64 {
        self.fencing_high_water
    }

    /// Returns the last applied transition revision.
    #[must_use]
    pub const fn transition_high_water(&self) -> u64 {
        self.transition_high_water
    }

    /// Returns the number of immutable terminal command receipts.
    #[must_use]
    pub const fn command_high_water(&self) -> u64 {
        self.command_high_water
    }
}

/// Domain-owned durable Writer Lease repository boundary.
///
/// Implementations must create `PostgreSQL` observations inside their own
/// transaction and then invoke the public pure plan/apply/verify functions.
/// Callers never supply database time, admission, daemon epoch, expiry, or a
/// fencing token through this contract.
pub trait WriterLeaseRepository {
    /// Executes one high-level typed command and returns its immutable terminal
    /// receipt.
    ///
    /// # Errors
    ///
    /// Returns a closed domain, availability, serialization, ambiguity,
    /// corruption, or current-authority failure.
    fn execute(
        &mut self,
        command: WriterLeaseRepositoryCommand,
    ) -> Result<WriterLeaseCommandReceipt, WriterLeaseRepositoryError>;

    /// Loads the current authority from replay-verified durable state.
    ///
    /// # Errors
    ///
    /// Returns a closed availability, corruption, or current-authority failure.
    fn current_authority(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<Option<WriterLeaseCurrentAuthority>, WriterLeaseRepositoryError>;

    /// Rejects a historical/substituted authority at the durable owner.
    ///
    /// # Errors
    ///
    /// Returns an authority mismatch or a closed durable-owner failure.
    fn assert_current(
        &mut self,
        expected: &WriterLeaseAuthorityHead,
    ) -> Result<(), WriterLeaseRepositoryError>;
}

/// One pure command plan. Applying it rechecks the complete aggregate digest.
#[derive(Clone, Debug)]
pub struct WriterLeasePlan {
    base_snapshot_digest: ContentDigest,
    next: VerifiedWriterLeaseAggregate,
    receipt: WriterLeaseCommandReceipt,
    exact_retry: bool,
}

impl WriterLeasePlan {
    /// Returns the terminal receipt produced by this plan.
    #[must_use]
    pub const fn receipt(&self) -> &WriterLeaseCommandReceipt {
        &self.receipt
    }

    /// Returns true when this is an exact retry of an existing terminal result.
    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.exact_retry
    }
}

/// Plans one command without I/O or mutation.
///
/// # Errors
///
/// Rejects malformed input, project mismatch, changed command-ID reuse, hash
/// failures, and shared-contract construction failures.
pub fn plan_command(
    current: &VerifiedWriterLeaseAggregate,
    command: &WriterLeaseCommand,
) -> Result<WriterLeasePlan, WriterLeaseError> {
    if current.project_id != *command.project_id() {
        return Err(WriterLeaseError::ProjectMismatch);
    }
    let request_digest = digest(
        "lattice-writer-lease-command-request",
        command_value(command),
    )?;
    let base_snapshot_digest = snapshot_digest(current)?;

    if let Some(existing) = current
        .command_receipts
        .iter()
        .find(|receipt| receipt.request.command_id() == command.command_id())
    {
        if existing.request_digest != request_digest {
            return Err(WriterLeaseError::CommandIdReuse);
        }
        return Ok(WriterLeasePlan {
            base_snapshot_digest,
            next: current.clone(),
            receipt: existing.clone(),
            exact_retry: true,
        });
    }
    validate_command(current, command)?;

    let ordinal = u64::try_from(current.command_receipts.len())
        .ok()
        .and_then(|value| value.checked_add(1))
        .filter(|value| *value <= MAX_SIGNED_BIGINT)
        .ok_or(WriterLeaseError::CorruptSnapshot)?;
    let before = current.current_head();

    let mut next = current.clone();
    let (outcome, transition) = transition_for(&mut next, command, &request_digest, ordinal)?;
    let after = next.current_head();
    let transition_digest = transition
        .as_ref()
        .map(|record| record.transition_digest.clone());
    if let Some(record) = transition {
        next.transitions.push(record);
    }
    let previous_receipt_digest = current
        .command_receipts
        .last()
        .map(|receipt| receipt.receipt_digest.clone());
    let receipt_digest = command_receipt_digest(&CommandReceiptDigestSubject {
        ordinal,
        previous_receipt_digest: previous_receipt_digest.as_ref(),
        command,
        request_digest: &request_digest,
        before: before.as_ref(),
        after: after.as_ref(),
        outcome,
        transition_digest: transition_digest.as_ref(),
    })?;
    let receipt = WriterLeaseCommandReceipt {
        ordinal,
        previous_receipt_digest,
        request: command.clone(),
        request_digest,
        before,
        after,
        outcome,
        transition_digest,
        receipt_digest,
    };
    next.command_receipts.push(receipt.clone());

    Ok(WriterLeasePlan {
        base_snapshot_digest,
        next,
        receipt,
        exact_retry: false,
    })
}

/// Applies a pure plan only to the exact aggregate it was planned against.
///
/// # Errors
///
/// Rejects a plan after any aggregate field or history changed.
pub fn apply_plan(
    current: &VerifiedWriterLeaseAggregate,
    plan: WriterLeasePlan,
) -> Result<VerifiedWriterLeaseAggregate, WriterLeaseError> {
    if snapshot_digest(current)? != plan.base_snapshot_digest {
        return Err(WriterLeaseError::PlanPreconditionChanged);
    }
    Ok(plan.next)
}

/// Replays and verifies every field of an untrusted snapshot.
///
/// # Errors
///
/// Rejects unknown versions, malformed ordering, truncation, hash
/// substitution, counter rollback, orphan transitions, or claimed-state drift.
pub fn verify_snapshot(
    snapshot: &UntrustedWriterLeaseSnapshot,
) -> Result<VerifiedWriterLeaseAggregate, WriterLeaseError> {
    let decoded = decode_snapshot(&snapshot.payload)?;
    if decoded.revision > MAX_SIGNED_BIGINT
        || decoded.fencing_high_water > MAX_SIGNED_BIGINT
        || decoded.command_high_water > MAX_SIGNED_BIGINT
    {
        return Err(WriterLeaseError::CorruptSnapshot);
    }
    let command_high_water = u64::try_from(decoded.command_receipts.len())
        .map_err(|_| WriterLeaseError::CorruptSnapshot)?;
    let command_tail_digest = decoded
        .command_receipts
        .last()
        .map(|receipt| receipt.receipt_digest.clone());
    if decoded.command_high_water != command_high_water
        || decoded.command_tail_digest != command_tail_digest
    {
        return Err(WriterLeaseError::CorruptSnapshot);
    }

    let mut replayed = VerifiedWriterLeaseAggregate::vacant(decoded.project_id.clone());
    for (index, expected_receipt) in decoded.command_receipts.iter().enumerate() {
        let expected_ordinal =
            u64::try_from(index + 1).map_err(|_| WriterLeaseError::CorruptSnapshot)?;
        if expected_receipt.ordinal != expected_ordinal {
            return Err(WriterLeaseError::CorruptSnapshot);
        }
        let plan = plan_command(&replayed, &expected_receipt.request)
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?;
        if plan.receipt != *expected_receipt || plan.exact_retry {
            return Err(WriterLeaseError::CorruptSnapshot);
        }
        replayed = apply_plan(&replayed, plan).map_err(|_| WriterLeaseError::CorruptSnapshot)?;
    }

    if replayed.fencing_high_water != decoded.fencing_high_water
        || replayed.revision != decoded.revision
        || replayed.current_receipt != decoded.current_receipt
        || replayed.transitions != decoded.transitions
        || replayed.command_receipts != decoded.command_receipts
        || canonicalize(&replayed.export_untrusted().payload)
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?
            != canonicalize(&snapshot.payload).map_err(|_| WriterLeaseError::CorruptSnapshot)?
    {
        return Err(WriterLeaseError::CorruptSnapshot);
    }
    Ok(replayed)
}

/// Verifies one raw snapshot against an independently retained current
/// checkpoint.
///
/// # Errors
///
/// Returns [`WriterLeaseError::CheckpointMismatch`] when the snapshot is
/// internally valid but is an older or substituted complete history.
pub fn verify_snapshot_against_checkpoint(
    snapshot: &UntrustedWriterLeaseSnapshot,
    expected: &WriterLeaseCheckpoint,
) -> Result<VerifiedWriterLeaseAggregate, WriterLeaseError> {
    let verified = verify_snapshot(snapshot)?;
    if verified.checkpoint()? != *expected {
        return Err(WriterLeaseError::CheckpointMismatch);
    }
    Ok(verified)
}

struct DecodedSnapshot {
    project_id: ProjectId,
    fencing_high_water: u64,
    revision: u64,
    command_high_water: u64,
    command_tail_digest: Option<ContentDigest>,
    current_receipt: Option<WriterLeaseAuthorityReceipt>,
    transitions: Vec<WriterLeaseTransitionRecord>,
    command_receipts: Vec<WriterLeaseCommandReceipt>,
}

struct RawObject<'a> {
    fields: BTreeMap<&'a str, &'a CanonicalValue>,
}

impl<'a> RawObject<'a> {
    fn new(value: &'a CanonicalValue, expected: &[&str]) -> Result<Self, WriterLeaseError> {
        let CanonicalValue::Object(fields) = value else {
            return Err(WriterLeaseError::CorruptSnapshot);
        };
        if fields.len() != expected.len() {
            return Err(WriterLeaseError::CorruptSnapshot);
        }
        let mut parsed = BTreeMap::new();
        for (name, value) in fields {
            if !expected.contains(&name.as_str()) || parsed.insert(name.as_str(), value).is_some() {
                return Err(WriterLeaseError::CorruptSnapshot);
            }
        }
        if expected.iter().any(|name| !parsed.contains_key(name)) {
            return Err(WriterLeaseError::CorruptSnapshot);
        }
        Ok(Self { fields: parsed })
    }

    fn versioned(value: &'a CanonicalValue, expected: &[&str]) -> Result<Self, WriterLeaseError> {
        let object = Self::new(value, expected)?;
        if object.text("schema_version")? != SNAPSHOT_VERSION {
            return Err(WriterLeaseError::CorruptSnapshot);
        }
        Ok(object)
    }

    fn value(&self, name: &str) -> Result<&'a CanonicalValue, WriterLeaseError> {
        self.fields
            .get(name)
            .copied()
            .ok_or(WriterLeaseError::CorruptSnapshot)
    }

    fn text(&self, name: &str) -> Result<&'a str, WriterLeaseError> {
        raw_text(self.value(name)?)
    }

    fn integer(&self, name: &str) -> Result<u64, WriterLeaseError> {
        raw_u64(self.value(name)?)
    }
}

fn raw_text(value: &CanonicalValue) -> Result<&str, WriterLeaseError> {
    let CanonicalValue::String(value) = value else {
        return Err(WriterLeaseError::CorruptSnapshot);
    };
    Ok(value)
}

fn raw_u64(value: &CanonicalValue) -> Result<u64, WriterLeaseError> {
    let value = raw_text(value)?;
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(WriterLeaseError::CorruptSnapshot);
    }
    value.parse().map_err(|_| WriterLeaseError::CorruptSnapshot)
}

fn raw_array(value: &CanonicalValue) -> Result<&[CanonicalValue], WriterLeaseError> {
    let CanonicalValue::Array(values) = value else {
        return Err(WriterLeaseError::CorruptSnapshot);
    };
    Ok(values)
}

fn raw_discriminator<'a>(
    value: &'a CanonicalValue,
    name: &str,
) -> Result<&'a str, WriterLeaseError> {
    let CanonicalValue::Object(fields) = value else {
        return Err(WriterLeaseError::CorruptSnapshot);
    };
    let mut found = None;
    for (field_name, field_value) in fields {
        if field_name == name {
            if found.is_some() {
                return Err(WriterLeaseError::CorruptSnapshot);
            }
            found = Some(raw_text(field_value)?);
        }
    }
    found.ok_or(WriterLeaseError::CorruptSnapshot)
}

fn raw_digest(value: &CanonicalValue) -> Result<ContentDigest, WriterLeaseError> {
    ContentDigest::from_sha256(raw_text(value)?.to_owned())
        .map_err(|_| WriterLeaseError::CorruptSnapshot)
}

fn raw_project_id(value: &CanonicalValue) -> Result<ProjectId, WriterLeaseError> {
    ProjectId::new(raw_text(value)?.to_owned()).map_err(|_| WriterLeaseError::CorruptSnapshot)
}

fn raw_runtime(value: &CanonicalValue) -> Result<RuntimeKind, WriterLeaseError> {
    match raw_text(value)? {
        "FAKE" => Ok(RuntimeKind::Fake),
        "LIVE" => Ok(RuntimeKind::Live),
        _ => Err(WriterLeaseError::CorruptSnapshot),
    }
}

fn raw_admission(value: &CanonicalValue) -> Result<RuntimeAdmissionMode, WriterLeaseError> {
    match raw_text(value)? {
        "ACTIVE" => Ok(RuntimeAdmissionMode::Active),
        "DRAINING" => Ok(RuntimeAdmissionMode::Draining),
        "CANARY" => Ok(RuntimeAdmissionMode::Canary),
        "STOPPED" => Ok(RuntimeAdmissionMode::Stopped),
        "RECONCILIATION_REQUIRED" => Ok(RuntimeAdmissionMode::ReconciliationRequired),
        _ => Err(WriterLeaseError::CorruptSnapshot),
    }
}

fn raw_status(value: &CanonicalValue) -> Result<WriterLeaseStatus, WriterLeaseError> {
    match raw_text(value)? {
        "ACTIVE" => Ok(WriterLeaseStatus::Active),
        "SUSPECT" => Ok(WriterLeaseStatus::Suspect),
        _ => Err(WriterLeaseError::CorruptSnapshot),
    }
}

fn raw_transition_kind(value: &CanonicalValue) -> Result<TransitionKind, WriterLeaseError> {
    match raw_text(value)? {
        "ACQUIRE" => Ok(TransitionKind::Acquire),
        "HEARTBEAT" => Ok(TransitionKind::Heartbeat),
        "MARK_SUSPECT" => Ok(TransitionKind::MarkSuspect),
        "PROCESS_HANDOFF" => Ok(TransitionKind::ProcessHandoff),
        "RELEASE" => Ok(TransitionKind::Release),
        "REVOKE" => Ok(TransitionKind::Revoke),
        _ => Err(WriterLeaseError::CorruptSnapshot),
    }
}

fn raw_denial(value: &CanonicalValue) -> Result<LeaseDenial, WriterLeaseError> {
    match raw_text(value)? {
        "STALE_HEAD" => Ok(LeaseDenial::StaleHead),
        "WRITER_ALREADY_HELD" => Ok(LeaseDenial::WriterAlreadyHeld),
        "LEASE_VACANT" => Ok(LeaseDenial::LeaseVacant),
        "INVALID_STATE" => Ok(LeaseDenial::InvalidState),
        "ADMISSION_DENIED" => Ok(LeaseDenial::AdmissionDenied),
        "RUNTIME_MISMATCH" => Ok(LeaseDenial::RuntimeMismatch),
        "HEARTBEAT_REJECTED" => Ok(LeaseDenial::HeartbeatRejected),
        "NOT_EXPIRED" => Ok(LeaseDenial::NotExpired),
        "RECOVERY_EVIDENCE_MISMATCH" => Ok(LeaseDenial::RecoveryEvidenceMismatch),
        "COUNTER_EXHAUSTED" => Ok(LeaseDenial::CounterExhausted),
        _ => Err(WriterLeaseError::CorruptSnapshot),
    }
}

fn parse_identity(value: &CanonicalValue) -> Result<WriterLeaseIdentity, WriterLeaseError> {
    let object = RawObject::versioned(
        value,
        &[
            "schema_version",
            "project_id",
            "project_snapshot_id",
            "task_id",
            "task_revision",
            "task_spec_digest",
            "attempt_id",
            "lease_id",
            "lease_holder_id",
            "worktree_id",
            "holder_process_id",
            "holder_process_start_identity",
            "daemon_instance_id",
            "daemon_epoch",
            "fencing_token",
        ],
    )?;
    WriterLeaseIdentity::new(
        raw_project_id(object.value("project_id")?)?,
        ProjectSnapshotId::new(object.text("project_snapshot_id")?.to_owned())
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        TaskId::new(object.text("task_id")?.to_owned())
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        object.text("task_revision")?.to_owned(),
        raw_digest(object.value("task_spec_digest")?)?,
        AttemptId::new(object.text("attempt_id")?.to_owned())
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        object.text("lease_id")?.to_owned(),
        object.text("lease_holder_id")?.to_owned(),
        object.text("worktree_id")?.to_owned(),
        HolderProcessId::new(object.integer("holder_process_id")?)
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        raw_digest(object.value("holder_process_start_identity")?)?,
        object.text("daemon_instance_id")?.to_owned(),
        DaemonEpoch::new(object.integer("daemon_epoch")?)
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        FencingToken::new(object.integer("fencing_token")?)
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
    )
    .map_err(|_| WriterLeaseError::CorruptSnapshot)
}

#[allow(clippy::too_many_lines)]
fn parse_authority_receipt(
    value: &CanonicalValue,
) -> Result<WriterLeaseAuthorityReceipt, WriterLeaseError> {
    let object = RawObject::versioned(
        value,
        &[
            "schema_version",
            "contract_version",
            "producer_id",
            "producer_version",
            "runtime",
            "identity",
            "status",
            "revision",
            "admission",
            "acquired_at",
            "heartbeat_at",
            "expires_at",
            "time_observation_digest",
            "admission_observation_digest",
            "transition_digest",
            "receipt_digest",
        ],
    )?;
    if object.integer("contract_version")? != u64::from(CONTRACT_VERSION) {
        return Err(WriterLeaseError::CorruptSnapshot);
    }
    for name in ["acquired_at", "heartbeat_at", "expires_at"] {
        parse_canonical_utc(object.text(name)?).map_err(|_| WriterLeaseError::CorruptSnapshot)?;
    }
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        object.text("producer_id")?.to_owned(),
        object.text("producer_version")?.to_owned(),
        raw_runtime(object.value("runtime")?)?,
        parse_identity(object.value("identity")?)?,
        raw_status(object.value("status")?)?,
        WriterLeaseRevision::new(object.integer("revision")?)
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        raw_admission(object.value("admission")?)?,
        object.text("acquired_at")?.to_owned(),
        object.text("heartbeat_at")?.to_owned(),
        object.text("expires_at")?.to_owned(),
        raw_digest(object.value("time_observation_digest")?)?,
        raw_digest(object.value("admission_observation_digest")?)?,
        raw_digest(object.value("transition_digest")?)?,
        raw_digest(object.value("receipt_digest")?)?,
    )
    .map_err(|_| WriterLeaseError::CorruptSnapshot)
}

fn parse_optional_receipt(
    value: &CanonicalValue,
) -> Result<Option<WriterLeaseAuthorityReceipt>, WriterLeaseError> {
    if matches!(value, CanonicalValue::Null) {
        Ok(None)
    } else {
        parse_authority_receipt(value).map(Some)
    }
}

fn parse_optional_head(
    value: &CanonicalValue,
) -> Result<Option<WriterLeaseAuthorityHead>, WriterLeaseError> {
    parse_optional_receipt(value).map(|receipt| receipt.map(|receipt| receipt.head()))
}

fn parse_required_head(
    value: &CanonicalValue,
) -> Result<WriterLeaseAuthorityHead, WriterLeaseError> {
    parse_optional_head(value)?.ok_or(WriterLeaseError::CorruptSnapshot)
}

fn parse_claim(value: &CanonicalValue) -> Result<AcquireClaim, WriterLeaseError> {
    let object = RawObject::versioned(
        value,
        &[
            "schema_version",
            "project_id",
            "project_snapshot_id",
            "task_id",
            "task_revision",
            "task_spec_digest",
            "attempt_id",
            "lease_id",
            "lease_holder_id",
            "worktree_id",
            "holder_process_id",
            "holder_process_start_identity",
            "daemon_instance_id",
            "daemon_epoch",
        ],
    )?;
    Ok(AcquireClaim {
        project_id: raw_project_id(object.value("project_id")?)?,
        project_snapshot_id: ProjectSnapshotId::new(object.text("project_snapshot_id")?.to_owned())
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        task_id: TaskId::new(object.text("task_id")?.to_owned())
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        task_revision: object.text("task_revision")?.to_owned(),
        task_spec_digest: raw_digest(object.value("task_spec_digest")?)?,
        attempt_id: AttemptId::new(object.text("attempt_id")?.to_owned())
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        lease_id: object.text("lease_id")?.to_owned(),
        lease_holder_id: object.text("lease_holder_id")?.to_owned(),
        worktree_id: object.text("worktree_id")?.to_owned(),
        holder_process_id: HolderProcessId::new(object.integer("holder_process_id")?)
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
        holder_process_start_identity: raw_digest(object.value("holder_process_start_identity")?)?,
        daemon_instance_id: object.text("daemon_instance_id")?.to_owned(),
        daemon_epoch: DaemonEpoch::new(object.integer("daemon_epoch")?)
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
    })
}

fn parse_observation(value: &CanonicalValue) -> Result<LeaseObservation, WriterLeaseError> {
    let object = RawObject::versioned(
        value,
        &[
            "schema_version",
            "runtime",
            "admission",
            "observed_at",
            "time_observation_digest",
            "admission_observation_digest",
        ],
    )?;
    let observed_at = object.text("observed_at")?.to_owned();
    parse_canonical_utc(&observed_at).map_err(|_| WriterLeaseError::CorruptSnapshot)?;
    Ok(LeaseObservation {
        runtime: raw_runtime(object.value("runtime")?)?,
        admission: raw_admission(object.value("admission")?)?,
        observed_at,
        time_observation_digest: raw_digest(object.value("time_observation_digest")?)?,
        admission_observation_digest: raw_digest(object.value("admission_observation_digest")?)?,
    })
}

fn parse_recovery(value: &CanonicalValue) -> Result<RecoveryEvidence, WriterLeaseError> {
    match raw_discriminator(value, "kind")? {
        "PROCESS_DEATH" => {
            let object = RawObject::versioned(
                value,
                &[
                    "schema_version",
                    "kind",
                    "holder_process_id",
                    "holder_process_start_identity",
                    "holder_daemon_instance_id",
                    "evidence_digest",
                ],
            )?;
            Ok(RecoveryEvidence::ProcessDeath {
                holder_process_id: HolderProcessId::new(object.integer("holder_process_id")?)
                    .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
                holder_process_start_identity: raw_digest(
                    object.value("holder_process_start_identity")?,
                )?,
                holder_daemon_instance_id: object.text("holder_daemon_instance_id")?.to_owned(),
                evidence_digest: raw_digest(object.value("evidence_digest")?)?,
            })
        }
        "LEADERSHIP_REPLACED" => {
            let object = RawObject::versioned(
                value,
                &[
                    "schema_version",
                    "kind",
                    "replaced_daemon_instance_id",
                    "replaced_epoch",
                    "replacement_daemon_instance_id",
                    "replacement_epoch",
                    "evidence_digest",
                ],
            )?;
            Ok(RecoveryEvidence::LeadershipReplaced {
                replaced_daemon_instance_id: object.text("replaced_daemon_instance_id")?.to_owned(),
                replaced_epoch: DaemonEpoch::new(object.integer("replaced_epoch")?)
                    .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
                replacement_daemon_instance_id: object
                    .text("replacement_daemon_instance_id")?
                    .to_owned(),
                replacement_epoch: DaemonEpoch::new(object.integer("replacement_epoch")?)
                    .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
                evidence_digest: raw_digest(object.value("evidence_digest")?)?,
            })
        }
        _ => Err(WriterLeaseError::CorruptSnapshot),
    }
}

#[allow(clippy::too_many_lines)]
fn parse_command(value: &CanonicalValue) -> Result<WriterLeaseCommand, WriterLeaseError> {
    let kind = raw_discriminator(value, "kind")?;
    let expected = match kind {
        "ACQUIRE" => &[
            "schema_version",
            "kind",
            "command_id",
            "project_id",
            "expected_head",
            "observation",
            "claim",
            "expires_at",
        ][..],
        "HEARTBEAT" => &[
            "schema_version",
            "kind",
            "command_id",
            "project_id",
            "expected_head",
            "observation",
            "expires_at",
        ][..],
        "MARK_SUSPECT" | "RELEASE" => &[
            "schema_version",
            "kind",
            "command_id",
            "project_id",
            "expected_head",
            "observation",
        ][..],
        "PROCESS_HANDOFF" => &[
            "schema_version",
            "kind",
            "command_id",
            "project_id",
            "expected_head",
            "successor_holder_process_id",
            "successor_holder_process_start_identity",
            "successor_daemon_instance_id",
            "successor_daemon_epoch",
            "observation",
            "expires_at",
            "evidence",
        ][..],
        "REVOKE" => &[
            "schema_version",
            "kind",
            "command_id",
            "project_id",
            "expected_head",
            "observation",
            "evidence",
        ][..],
        _ => return Err(WriterLeaseError::CorruptSnapshot),
    };
    let object = RawObject::versioned(value, expected)?;
    let command_id = object.text("command_id")?.to_owned();
    let project_id = raw_project_id(object.value("project_id")?)?;
    let observation = parse_observation(object.value("observation")?)?;
    match kind {
        "ACQUIRE" => {
            let claim = parse_claim(object.value("claim")?)?;
            if claim.project_id != project_id {
                return Err(WriterLeaseError::CorruptSnapshot);
            }
            Ok(WriterLeaseCommand::Acquire(AcquireCommand {
                command_id,
                expected_head: parse_optional_head(object.value("expected_head")?)?,
                claim,
                observation,
                expires_at: object.text("expires_at")?.to_owned(),
            }))
        }
        "HEARTBEAT" => Ok(WriterLeaseCommand::Heartbeat(HeartbeatCommand {
            command_id,
            project_id,
            expected_head: parse_required_head(object.value("expected_head")?)?,
            observation,
            expires_at: object.text("expires_at")?.to_owned(),
        })),
        "MARK_SUSPECT" => Ok(WriterLeaseCommand::MarkSuspect(MarkSuspectCommand {
            command_id,
            project_id,
            expected_head: parse_required_head(object.value("expected_head")?)?,
            observation,
        })),
        "PROCESS_HANDOFF" => Ok(WriterLeaseCommand::ProcessHandoff(ProcessHandoffCommand {
            command_id,
            project_id,
            expected_head: parse_required_head(object.value("expected_head")?)?,
            successor_holder_process_id: HolderProcessId::new(
                object.integer("successor_holder_process_id")?,
            )
            .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
            successor_holder_process_start_identity: raw_digest(
                object.value("successor_holder_process_start_identity")?,
            )?,
            successor_daemon_instance_id: object.text("successor_daemon_instance_id")?.to_owned(),
            successor_daemon_epoch: DaemonEpoch::new(object.integer("successor_daemon_epoch")?)
                .map_err(|_| WriterLeaseError::CorruptSnapshot)?,
            observation,
            expires_at: object.text("expires_at")?.to_owned(),
            evidence: parse_recovery(object.value("evidence")?)?,
        })),
        "RELEASE" => Ok(WriterLeaseCommand::Release(ReleaseCommand {
            command_id,
            project_id,
            expected_head: parse_required_head(object.value("expected_head")?)?,
            observation,
        })),
        "REVOKE" => Ok(WriterLeaseCommand::Revoke(RevokeCommand {
            command_id,
            project_id,
            expected_head: parse_required_head(object.value("expected_head")?)?,
            observation,
            evidence: parse_recovery(object.value("evidence")?)?,
        })),
        _ => Err(WriterLeaseError::CorruptSnapshot),
    }
}

fn parse_transition_record(
    value: &CanonicalValue,
) -> Result<WriterLeaseTransitionRecord, WriterLeaseError> {
    let object = RawObject::versioned(
        value,
        &[
            "schema_version",
            "ordinal",
            "command_id",
            "kind",
            "request_digest",
            "before",
            "after",
            "transition_digest",
        ],
    )?;
    Ok(WriterLeaseTransitionRecord {
        ordinal: object.integer("ordinal")?,
        command_id: object.text("command_id")?.to_owned(),
        kind: raw_transition_kind(object.value("kind")?)?,
        request_digest: raw_digest(object.value("request_digest")?)?,
        before: parse_optional_head(object.value("before")?)?,
        after: parse_optional_receipt(object.value("after")?)?,
        transition_digest: raw_digest(object.value("transition_digest")?)?,
    })
}

fn parse_optional_digest(
    value: &CanonicalValue,
) -> Result<Option<ContentDigest>, WriterLeaseError> {
    if matches!(value, CanonicalValue::Null) {
        Ok(None)
    } else {
        raw_digest(value).map(Some)
    }
}

fn parse_command_receipt(
    value: &CanonicalValue,
) -> Result<WriterLeaseCommandReceipt, WriterLeaseError> {
    let object = RawObject::versioned(
        value,
        &[
            "schema_version",
            "ordinal",
            "previous_receipt_digest",
            "request",
            "request_digest",
            "before",
            "after",
            "outcome",
            "denial_reason",
            "transition_digest",
            "receipt_digest",
        ],
    )?;
    let outcome = match object.text("outcome")? {
        "APPLIED" if matches!(object.value("denial_reason")?, CanonicalValue::Null) => {
            CommandOutcome::Applied
        }
        "DENIED" => CommandOutcome::Denied(raw_denial(object.value("denial_reason")?)?),
        _ => return Err(WriterLeaseError::CorruptSnapshot),
    };
    Ok(WriterLeaseCommandReceipt {
        ordinal: object.integer("ordinal")?,
        previous_receipt_digest: parse_optional_digest(object.value("previous_receipt_digest")?)?,
        request: parse_command(object.value("request")?)?,
        request_digest: raw_digest(object.value("request_digest")?)?,
        before: parse_optional_head(object.value("before")?)?,
        after: parse_optional_head(object.value("after")?)?,
        outcome,
        transition_digest: parse_optional_digest(object.value("transition_digest")?)?,
        receipt_digest: raw_digest(object.value("receipt_digest")?)?,
    })
}

#[allow(clippy::too_many_lines)]
fn decode_snapshot(value: &CanonicalValue) -> Result<DecodedSnapshot, WriterLeaseError> {
    let object = RawObject::versioned(
        value,
        &[
            "schema_version",
            "project_id",
            "fencing_high_water",
            "revision",
            "command_high_water",
            "command_tail_digest",
            "current_receipt",
            "transitions",
            "commands",
        ],
    )?;
    let transitions = raw_array(object.value("transitions")?)?
        .iter()
        .map(parse_transition_record)
        .collect::<Result<Vec<_>, _>>()?;
    let command_receipts = raw_array(object.value("commands")?)?
        .iter()
        .map(parse_command_receipt)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DecodedSnapshot {
        project_id: raw_project_id(object.value("project_id")?)?,
        fencing_high_water: object.integer("fencing_high_water")?,
        revision: object.integer("revision")?,
        command_high_water: object.integer("command_high_water")?,
        command_tail_digest: parse_optional_digest(object.value("command_tail_digest")?)?,
        current_receipt: parse_optional_receipt(object.value("current_receipt")?)?,
        transitions,
        command_receipts,
    })
}

/// Deterministic in-memory owner used only for characterization/composition.
#[derive(Clone, Debug, Default)]
pub struct FakeWriterLease {
    projects: BTreeMap<ProjectId, VerifiedWriterLeaseAggregate>,
}

impl FakeWriterLease {
    /// Creates an empty visibly non-durable fake.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of project aggregates retained in memory.
    #[must_use]
    pub fn project_count(&self) -> usize {
        self.projects.len()
    }

    /// Executes one fake-only command through the public planner and apply path.
    ///
    /// # Errors
    ///
    /// Returns the same validation/planning failures as [`plan_command`] and
    /// rejects a live runtime marker.
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute(
        &mut self,
        command: WriterLeaseCommand,
    ) -> Result<WriterLeaseCommandReceipt, WriterLeaseError> {
        if command.observation().runtime != RuntimeKind::Fake {
            return Err(WriterLeaseError::FakeRuntimeRequired);
        }
        let project_id = command.project_id().clone();
        let current = self
            .projects
            .get(&project_id)
            .cloned()
            .unwrap_or_else(|| VerifiedWriterLeaseAggregate::vacant(project_id.clone()));
        let plan = plan_command(&current, &command)?;
        let receipt = plan.receipt.clone();
        let next = apply_plan(&current, plan)?;
        self.projects.insert(project_id, next);
        Ok(receipt)
    }

    /// Returns the fake owner's current authority receipt.
    #[must_use]
    pub fn current_receipt(&self, project_id: &ProjectId) -> Option<WriterLeaseAuthorityReceipt> {
        self.projects
            .get(project_id)
            .and_then(VerifiedWriterLeaseAggregate::current_receipt)
            .cloned()
    }

    /// Returns the fake owner's independently queried current authority head.
    #[must_use]
    pub fn current_head(&self, project_id: &ProjectId) -> Option<WriterLeaseAuthorityHead> {
        self.projects
            .get(project_id)
            .and_then(VerifiedWriterLeaseAggregate::current_head)
    }

    /// Exports one complete untrusted project snapshot.
    #[must_use]
    pub fn export_snapshot(&self, project_id: &ProjectId) -> Option<UntrustedWriterLeaseSnapshot> {
        self.projects
            .get(project_id)
            .map(VerifiedWriterLeaseAggregate::export_untrusted)
    }

    /// Returns the independently retained checkpoint for one fake aggregate.
    ///
    /// # Errors
    ///
    /// Returns a canonical hashing failure if the complete aggregate cannot be
    /// committed.
    pub fn current_checkpoint(
        &self,
        project_id: &ProjectId,
    ) -> Result<Option<WriterLeaseCheckpoint>, WriterLeaseError> {
        self.projects
            .get(project_id)
            .map(VerifiedWriterLeaseAggregate::checkpoint)
            .transpose()
    }

    /// Verifies and restores one complete snapshot against a trusted
    /// checkpoint.
    ///
    /// # Errors
    ///
    /// Rejects corrupt, stale, substituted, or live snapshots without
    /// modifying existing fake state.
    pub fn restore_snapshot(
        &mut self,
        snapshot: &UntrustedWriterLeaseSnapshot,
        expected_checkpoint: &WriterLeaseCheckpoint,
    ) -> Result<(), WriterLeaseError> {
        let verified = verify_snapshot_against_checkpoint(snapshot, expected_checkpoint)?;
        if self.projects.contains_key(verified.project_id()) {
            return Err(WriterLeaseError::RestoreWouldOverwrite);
        }
        if !aggregate_is_fake(&verified) {
            return Err(WriterLeaseError::FakeRuntimeRequired);
        }
        self.projects
            .insert(verified.project_id().clone(), verified);
        Ok(())
    }
}

fn aggregate_is_fake(aggregate: &VerifiedWriterLeaseAggregate) -> bool {
    aggregate
        .current_receipt
        .as_ref()
        .is_none_or(|receipt| receipt.runtime() == RuntimeKind::Fake)
        && aggregate.transitions.iter().all(|transition| {
            transition
                .before
                .as_ref()
                .is_none_or(|head| head.runtime() == RuntimeKind::Fake)
                && transition
                    .after
                    .as_ref()
                    .is_none_or(|receipt| receipt.runtime() == RuntimeKind::Fake)
        })
        && aggregate.command_receipts.iter().all(|receipt| {
            receipt.request.observation().runtime == RuntimeKind::Fake
                && receipt
                    .request
                    .expected_head()
                    .is_none_or(|head| head.runtime() == RuntimeKind::Fake)
                && receipt
                    .before
                    .as_ref()
                    .is_none_or(|head| head.runtime() == RuntimeKind::Fake)
                && receipt
                    .after
                    .as_ref()
                    .is_none_or(|head| head.runtime() == RuntimeKind::Fake)
        })
}

fn validate_command(
    current: &VerifiedWriterLeaseAggregate,
    command: &WriterLeaseCommand,
) -> Result<(), WriterLeaseError> {
    if current.project_id != *command.project_id() {
        return Err(WriterLeaseError::ProjectMismatch);
    }
    if !valid_identifier(command.command_id()) {
        return Err(WriterLeaseError::InvalidCommandId);
    }
    validate_observation(command.observation())?;
    if is_zero_digest(&command.observation().time_observation_digest)
        || is_zero_digest(&command.observation().admission_observation_digest)
    {
        return Err(WriterLeaseError::ZeroEvidenceDigest);
    }
    match command {
        WriterLeaseCommand::Acquire(command) => {
            if is_zero_digest(&command.claim.task_spec_digest)
                || is_zero_digest(&command.claim.holder_process_start_identity)
            {
                return Err(WriterLeaseError::ZeroEvidenceDigest);
            }
            WriterLeaseIdentity::new(
                command.claim.project_id.clone(),
                command.claim.project_snapshot_id.clone(),
                command.claim.task_id.clone(),
                command.claim.task_revision.clone(),
                command.claim.task_spec_digest.clone(),
                command.claim.attempt_id.clone(),
                command.claim.lease_id.clone(),
                command.claim.lease_holder_id.clone(),
                command.claim.worktree_id.clone(),
                command.claim.holder_process_id,
                command.claim.holder_process_start_identity.clone(),
                command.claim.daemon_instance_id.clone(),
                command.claim.daemon_epoch,
                FencingToken::new(1).map_err(|_| WriterLeaseError::Contract)?,
            )
            .map_err(|_| WriterLeaseError::Contract)?;
            let observed = parse_canonical_utc(&command.observation.observed_at)?;
            let expires = parse_canonical_utc(&command.expires_at)?;
            if expires <= observed {
                return Err(WriterLeaseError::InvalidExpiry);
            }
        }
        WriterLeaseCommand::Heartbeat(command) => {
            let observed = parse_canonical_utc(&command.observation.observed_at)?;
            let expires = parse_canonical_utc(&command.expires_at)?;
            if expires <= observed {
                return Err(WriterLeaseError::InvalidExpiry);
            }
        }
        WriterLeaseCommand::ProcessHandoff(command) => {
            validate_process_handoff(command)?;
        }
        WriterLeaseCommand::MarkSuspect(_)
        | WriterLeaseCommand::Release(_)
        | WriterLeaseCommand::Revoke(_) => {}
    }
    if let WriterLeaseCommand::Revoke(command) = command
        && is_zero_digest(command.evidence.evidence_digest())
    {
        return Err(WriterLeaseError::ZeroEvidenceDigest);
    }
    if let WriterLeaseCommand::Revoke(command) = command {
        let valid_recovery_identifiers = match &command.evidence {
            RecoveryEvidence::ProcessDeath {
                holder_daemon_instance_id,
                ..
            } => valid_identifier(holder_daemon_instance_id),
            RecoveryEvidence::LeadershipReplaced {
                replaced_daemon_instance_id,
                replacement_daemon_instance_id,
                ..
            } => {
                valid_identifier(replaced_daemon_instance_id)
                    && valid_identifier(replacement_daemon_instance_id)
            }
        };
        if !valid_recovery_identifiers {
            return Err(WriterLeaseError::InvalidRecoveryEvidence);
        }
    }
    Ok(())
}

fn validate_process_handoff(command: &ProcessHandoffCommand) -> Result<(), WriterLeaseError> {
    let observed = parse_canonical_utc(&command.observation.observed_at)?;
    let expires = parse_canonical_utc(&command.expires_at)?;
    if expires <= observed {
        return Err(WriterLeaseError::InvalidExpiry);
    }
    if is_zero_digest(&command.successor_holder_process_start_identity)
        || is_zero_digest(command.evidence.evidence_digest())
    {
        return Err(WriterLeaseError::ZeroEvidenceDigest);
    }
    if !valid_identifier(&command.successor_daemon_instance_id)
        || !matches!(command.evidence, RecoveryEvidence::ProcessDeath { .. })
    {
        return Err(WriterLeaseError::InvalidRecoveryEvidence);
    }
    let RecoveryEvidence::ProcessDeath {
        holder_daemon_instance_id,
        ..
    } = &command.evidence
    else {
        unreachable!();
    };
    if !valid_identifier(holder_daemon_instance_id) {
        return Err(WriterLeaseError::InvalidRecoveryEvidence);
    }
    Ok(())
}

fn validate_observation(observation: &LeaseObservation) -> Result<(), WriterLeaseError> {
    parse_canonical_utc(&observation.observed_at).map(|_| ())
}

#[allow(clippy::too_many_lines)]
fn transition_for(
    next: &mut VerifiedWriterLeaseAggregate,
    command: &WriterLeaseCommand,
    request_digest: &ContentDigest,
    ordinal: u64,
) -> Result<(CommandOutcome, Option<WriterLeaseTransitionRecord>), WriterLeaseError> {
    let before = next.current_head();
    if matches!(command, WriterLeaseCommand::Acquire(_)) {
        if before.is_some() {
            return Ok((CommandOutcome::Denied(LeaseDenial::WriterAlreadyHeld), None));
        }
        if command.expected_head().is_some() {
            return Ok((CommandOutcome::Denied(LeaseDenial::StaleHead), None));
        }
    } else if command.expected_head() != before.as_ref() {
        return Ok((CommandOutcome::Denied(LeaseDenial::StaleHead), None));
    }
    if let Some(current) = next.current_receipt.as_ref()
        && current.runtime() != command.observation().runtime
    {
        return Ok((CommandOutcome::Denied(LeaseDenial::RuntimeMismatch), None));
    }

    match command {
        WriterLeaseCommand::Acquire(command) => {
            if command.observation.admission != RuntimeAdmissionMode::Active {
                return Ok((CommandOutcome::Denied(LeaseDenial::AdmissionDenied), None));
            }
            let Some(fence_value) = next.fencing_high_water.checked_add(1) else {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            };
            let Some(revision_value) = next.revision.checked_add(1) else {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            };
            if fence_value > MAX_SIGNED_BIGINT || revision_value > MAX_SIGNED_BIGINT {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            }
            let fence = FencingToken::new(fence_value).map_err(|_| WriterLeaseError::Contract)?;
            let revision =
                WriterLeaseRevision::new(revision_value).map_err(|_| WriterLeaseError::Contract)?;
            let identity = WriterLeaseIdentity::new(
                command.claim.project_id.clone(),
                command.claim.project_snapshot_id.clone(),
                command.claim.task_id.clone(),
                command.claim.task_revision.clone(),
                command.claim.task_spec_digest.clone(),
                command.claim.attempt_id.clone(),
                command.claim.lease_id.clone(),
                command.claim.lease_holder_id.clone(),
                command.claim.worktree_id.clone(),
                command.claim.holder_process_id,
                command.claim.holder_process_start_identity.clone(),
                command.claim.daemon_instance_id.clone(),
                command.claim.daemon_epoch,
                fence,
            )
            .map_err(|_| WriterLeaseError::Contract)?;
            let transition_digest = transition_digest(
                ordinal,
                TransitionKind::Acquire,
                &WriterLeaseCommand::Acquire(command.clone()),
                request_digest,
                before.as_ref(),
                Some(AuthoritySemantic {
                    identity: &identity,
                    status: WriterLeaseStatus::Active,
                    revision,
                    acquired_at: &command.observation.observed_at,
                    heartbeat_at: &command.observation.observed_at,
                    expires_at: &command.expires_at,
                }),
            )?;
            let authority = authority_receipt(
                command.observation.runtime,
                identity,
                WriterLeaseStatus::Active,
                revision,
                &command.observation,
                &command.observation.observed_at,
                &command.observation.observed_at,
                &command.expires_at,
                transition_digest.clone(),
            )?;
            next.fencing_high_water = fence_value;
            next.revision = revision_value;
            next.current_receipt = Some(authority.clone());
            Ok((
                CommandOutcome::Applied,
                Some(WriterLeaseTransitionRecord {
                    ordinal,
                    command_id: command.command_id.clone(),
                    kind: TransitionKind::Acquire,
                    request_digest: request_digest.clone(),
                    before,
                    after: Some(authority),
                    transition_digest,
                }),
            ))
        }
        WriterLeaseCommand::Heartbeat(command) => {
            if command.observation.admission != RuntimeAdmissionMode::Active {
                return Ok((CommandOutcome::Denied(LeaseDenial::AdmissionDenied), None));
            }
            let current = next
                .current_receipt
                .as_ref()
                .ok_or(WriterLeaseError::CorruptSnapshot)?;
            if current.status() != WriterLeaseStatus::Active {
                return Ok((CommandOutcome::Denied(LeaseDenial::InvalidState), None));
            }
            let observed = parse_canonical_utc(&command.observation.observed_at)?;
            let old_heartbeat = parse_canonical_utc(current.heartbeat_at())?;
            let old_expiry = parse_canonical_utc(current.expires_at())?;
            let new_expiry = parse_canonical_utc(&command.expires_at)?;
            if observed <= old_heartbeat || observed >= old_expiry || new_expiry <= old_expiry {
                return Ok((CommandOutcome::Denied(LeaseDenial::HeartbeatRejected), None));
            }
            let Some(revision_value) = next.revision.checked_add(1) else {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            };
            if revision_value > MAX_SIGNED_BIGINT {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            }
            let revision =
                WriterLeaseRevision::new(revision_value).map_err(|_| WriterLeaseError::Contract)?;
            let identity = current.identity().clone();
            let acquired_at = current.acquired_at().to_owned();
            let transition_digest = transition_digest(
                ordinal,
                TransitionKind::Heartbeat,
                &WriterLeaseCommand::Heartbeat(command.clone()),
                request_digest,
                before.as_ref(),
                Some(AuthoritySemantic {
                    identity: &identity,
                    status: WriterLeaseStatus::Active,
                    revision,
                    acquired_at: &acquired_at,
                    heartbeat_at: &command.observation.observed_at,
                    expires_at: &command.expires_at,
                }),
            )?;
            let authority = authority_receipt(
                command.observation.runtime,
                identity,
                WriterLeaseStatus::Active,
                revision,
                &command.observation,
                &acquired_at,
                &command.observation.observed_at,
                &command.expires_at,
                transition_digest.clone(),
            )?;
            next.revision = revision_value;
            next.current_receipt = Some(authority.clone());
            Ok((
                CommandOutcome::Applied,
                Some(WriterLeaseTransitionRecord {
                    ordinal,
                    command_id: command.command_id.clone(),
                    kind: TransitionKind::Heartbeat,
                    request_digest: request_digest.clone(),
                    before,
                    after: Some(authority),
                    transition_digest,
                }),
            ))
        }
        WriterLeaseCommand::MarkSuspect(command) => {
            if !matches!(
                command.observation.admission,
                RuntimeAdmissionMode::Active | RuntimeAdmissionMode::Draining
            ) {
                return Ok((CommandOutcome::Denied(LeaseDenial::AdmissionDenied), None));
            }
            let current = next
                .current_receipt
                .as_ref()
                .ok_or(WriterLeaseError::CorruptSnapshot)?;
            if current.status() != WriterLeaseStatus::Active {
                return Ok((CommandOutcome::Denied(LeaseDenial::InvalidState), None));
            }
            if parse_canonical_utc(&command.observation.observed_at)?
                < parse_canonical_utc(current.expires_at())?
            {
                return Ok((CommandOutcome::Denied(LeaseDenial::NotExpired), None));
            }
            let Some(revision_value) = next.revision.checked_add(1) else {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            };
            if revision_value > MAX_SIGNED_BIGINT {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            }
            let revision =
                WriterLeaseRevision::new(revision_value).map_err(|_| WriterLeaseError::Contract)?;
            let identity = current.identity().clone();
            let acquired_at = current.acquired_at().to_owned();
            let heartbeat_at = current.heartbeat_at().to_owned();
            let expires_at = current.expires_at().to_owned();
            let transition_digest = transition_digest(
                ordinal,
                TransitionKind::MarkSuspect,
                &WriterLeaseCommand::MarkSuspect(command.clone()),
                request_digest,
                before.as_ref(),
                Some(AuthoritySemantic {
                    identity: &identity,
                    status: WriterLeaseStatus::Suspect,
                    revision,
                    acquired_at: &acquired_at,
                    heartbeat_at: &heartbeat_at,
                    expires_at: &expires_at,
                }),
            )?;
            let authority = authority_receipt(
                command.observation.runtime,
                identity,
                WriterLeaseStatus::Suspect,
                revision,
                &command.observation,
                &acquired_at,
                &heartbeat_at,
                &expires_at,
                transition_digest.clone(),
            )?;
            next.revision = revision_value;
            next.current_receipt = Some(authority.clone());
            Ok((
                CommandOutcome::Applied,
                Some(WriterLeaseTransitionRecord {
                    ordinal,
                    command_id: command.command_id.clone(),
                    kind: TransitionKind::MarkSuspect,
                    request_digest: request_digest.clone(),
                    before,
                    after: Some(authority),
                    transition_digest,
                }),
            ))
        }
        WriterLeaseCommand::ProcessHandoff(command) => {
            if command.observation.admission != RuntimeAdmissionMode::Active {
                return Ok((CommandOutcome::Denied(LeaseDenial::AdmissionDenied), None));
            }
            let current = next
                .current_receipt
                .as_ref()
                .ok_or(WriterLeaseError::CorruptSnapshot)?;
            let current_identity = current.identity();
            if command.successor_daemon_instance_id != current_identity.daemon_instance_id()
                || command.successor_daemon_epoch != current_identity.daemon_epoch()
                || !recovery_matches(current_identity, &command.evidence)
                || (command.successor_holder_process_id == current_identity.holder_process_id()
                    && command.successor_holder_process_start_identity
                        == *current_identity.holder_process_start_identity())
            {
                return Ok((
                    CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch),
                    None,
                ));
            }
            let observed = parse_canonical_utc(&command.observation.observed_at)?;
            let old_heartbeat = parse_canonical_utc(current.heartbeat_at())?;
            let old_expiry = parse_canonical_utc(current.expires_at())?;
            let new_expiry = parse_canonical_utc(&command.expires_at)?;
            let valid_time = match current.status() {
                WriterLeaseStatus::Active => observed > old_heartbeat && observed < old_expiry,
                WriterLeaseStatus::Suspect => observed >= old_expiry,
            };
            if !valid_time || new_expiry <= old_expiry {
                return Ok((CommandOutcome::Denied(LeaseDenial::InvalidState), None));
            }
            let Some(revision_value) = next.revision.checked_add(1) else {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            };
            if revision_value > MAX_SIGNED_BIGINT {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            }
            let revision =
                WriterLeaseRevision::new(revision_value).map_err(|_| WriterLeaseError::Contract)?;
            let identity = WriterLeaseIdentity::new(
                current_identity.project_id().clone(),
                current_identity.project_snapshot_id().clone(),
                current_identity.task_id().clone(),
                current_identity.task_revision().to_owned(),
                current_identity.task_spec_digest().clone(),
                current_identity.attempt_id().clone(),
                current_identity.lease_id().to_owned(),
                current_identity.lease_holder_id().to_owned(),
                current_identity.worktree_id().to_owned(),
                command.successor_holder_process_id,
                command.successor_holder_process_start_identity.clone(),
                command.successor_daemon_instance_id.clone(),
                command.successor_daemon_epoch,
                current_identity.fencing_token(),
            )
            .map_err(|_| WriterLeaseError::Contract)?;
            let acquired_at = current.acquired_at().to_owned();
            let transition_digest = transition_digest(
                ordinal,
                TransitionKind::ProcessHandoff,
                &WriterLeaseCommand::ProcessHandoff(command.clone()),
                request_digest,
                before.as_ref(),
                Some(AuthoritySemantic {
                    identity: &identity,
                    status: WriterLeaseStatus::Active,
                    revision,
                    acquired_at: &acquired_at,
                    heartbeat_at: &command.observation.observed_at,
                    expires_at: &command.expires_at,
                }),
            )?;
            let authority = authority_receipt(
                command.observation.runtime,
                identity,
                WriterLeaseStatus::Active,
                revision,
                &command.observation,
                &acquired_at,
                &command.observation.observed_at,
                &command.expires_at,
                transition_digest.clone(),
            )?;
            next.revision = revision_value;
            next.current_receipt = Some(authority.clone());
            Ok((
                CommandOutcome::Applied,
                Some(WriterLeaseTransitionRecord {
                    ordinal,
                    command_id: command.command_id.clone(),
                    kind: TransitionKind::ProcessHandoff,
                    request_digest: request_digest.clone(),
                    before,
                    after: Some(authority),
                    transition_digest,
                }),
            ))
        }
        WriterLeaseCommand::Release(command) => {
            if !matches!(
                command.observation.admission,
                RuntimeAdmissionMode::Active | RuntimeAdmissionMode::Draining
            ) {
                return Ok((CommandOutcome::Denied(LeaseDenial::AdmissionDenied), None));
            }
            if next.current_receipt.is_none() {
                return Ok((CommandOutcome::Denied(LeaseDenial::LeaseVacant), None));
            }
            let Some(revision_value) = next.revision.checked_add(1) else {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            };
            if revision_value > MAX_SIGNED_BIGINT {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            }
            let transition_digest = transition_digest(
                ordinal,
                TransitionKind::Release,
                &WriterLeaseCommand::Release(command.clone()),
                request_digest,
                before.as_ref(),
                None,
            )?;
            next.revision = revision_value;
            next.current_receipt = None;
            Ok((
                CommandOutcome::Applied,
                Some(WriterLeaseTransitionRecord {
                    ordinal,
                    command_id: command.command_id.clone(),
                    kind: TransitionKind::Release,
                    request_digest: request_digest.clone(),
                    before,
                    after: None,
                    transition_digest,
                }),
            ))
        }
        WriterLeaseCommand::Revoke(command) => {
            if !matches!(
                command.observation.admission,
                RuntimeAdmissionMode::Draining | RuntimeAdmissionMode::ReconciliationRequired
            ) {
                return Ok((CommandOutcome::Denied(LeaseDenial::AdmissionDenied), None));
            }
            let current = next
                .current_receipt
                .as_ref()
                .ok_or(WriterLeaseError::CorruptSnapshot)?;
            if current.status() != WriterLeaseStatus::Suspect {
                return Ok((CommandOutcome::Denied(LeaseDenial::InvalidState), None));
            }
            if !recovery_matches(current.identity(), &command.evidence) {
                return Ok((
                    CommandOutcome::Denied(LeaseDenial::RecoveryEvidenceMismatch),
                    None,
                ));
            }
            let Some(revision_value) = next.revision.checked_add(1) else {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            };
            if revision_value > MAX_SIGNED_BIGINT {
                return Ok((CommandOutcome::Denied(LeaseDenial::CounterExhausted), None));
            }
            let transition_digest = transition_digest(
                ordinal,
                TransitionKind::Revoke,
                &WriterLeaseCommand::Revoke(command.clone()),
                request_digest,
                before.as_ref(),
                None,
            )?;
            next.revision = revision_value;
            next.current_receipt = None;
            Ok((
                CommandOutcome::Applied,
                Some(WriterLeaseTransitionRecord {
                    ordinal,
                    command_id: command.command_id.clone(),
                    kind: TransitionKind::Revoke,
                    request_digest: request_digest.clone(),
                    before,
                    after: None,
                    transition_digest,
                }),
            ))
        }
    }
}

fn recovery_matches(identity: &WriterLeaseIdentity, evidence: &RecoveryEvidence) -> bool {
    match evidence {
        RecoveryEvidence::ProcessDeath {
            holder_process_id,
            holder_process_start_identity,
            holder_daemon_instance_id,
            ..
        } => {
            identity.holder_process_id() == *holder_process_id
                && identity.holder_process_start_identity() == holder_process_start_identity
                && identity.daemon_instance_id() == holder_daemon_instance_id
        }
        RecoveryEvidence::LeadershipReplaced {
            replaced_daemon_instance_id,
            replaced_epoch,
            replacement_daemon_instance_id,
            replacement_epoch,
            ..
        } => {
            identity.daemon_instance_id() == replaced_daemon_instance_id
                && identity.daemon_epoch() == *replaced_epoch
                && !replacement_daemon_instance_id.trim().is_empty()
                && replacement_daemon_instance_id != replaced_daemon_instance_id
                && replacement_epoch.get() > replaced_epoch.get()
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn authority_receipt(
    runtime: RuntimeKind,
    identity: WriterLeaseIdentity,
    status: WriterLeaseStatus,
    revision: WriterLeaseRevision,
    observation: &LeaseObservation,
    acquired_at: &str,
    heartbeat_at: &str,
    expires_at: &str,
    transition_digest: ContentDigest,
) -> Result<WriterLeaseAuthorityReceipt, WriterLeaseError> {
    let receipt_digest = digest(
        "lattice-writer-lease-authority-receipt",
        authority_value(
            runtime,
            &identity,
            status,
            revision,
            observation.admission,
            acquired_at,
            heartbeat_at,
            expires_at,
            &observation.time_observation_digest,
            &observation.admission_observation_digest,
            &transition_digest,
        ),
    )?;
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        runtime,
        identity,
        status,
        revision,
        observation.admission,
        acquired_at,
        heartbeat_at,
        expires_at,
        observation.time_observation_digest.clone(),
        observation.admission_observation_digest.clone(),
        transition_digest,
        receipt_digest,
    )
    .map_err(|_| WriterLeaseError::Contract)
}

struct AuthoritySemantic<'a> {
    identity: &'a WriterLeaseIdentity,
    status: WriterLeaseStatus,
    revision: WriterLeaseRevision,
    acquired_at: &'a str,
    heartbeat_at: &'a str,
    expires_at: &'a str,
}

fn transition_digest(
    ordinal: u64,
    kind: TransitionKind,
    command: &WriterLeaseCommand,
    request_digest: &ContentDigest,
    before: Option<&WriterLeaseAuthorityHead>,
    after: Option<AuthoritySemantic<'_>>,
) -> Result<ContentDigest, WriterLeaseError> {
    let after_value = after.map_or(CanonicalValue::Null, |after| {
        CanonicalValue::Object(vec![
            ("identity".to_owned(), identity_value(after.identity)),
            ("status".to_owned(), string(after.status.as_str())),
            (
                "revision".to_owned(),
                string(after.revision.get().to_string()),
            ),
            ("acquired_at".to_owned(), string(after.acquired_at)),
            ("heartbeat_at".to_owned(), string(after.heartbeat_at)),
            ("expires_at".to_owned(), string(after.expires_at)),
        ])
    });
    digest(
        "lattice-writer-lease-transition",
        CanonicalValue::Object(vec![
            ("ordinal".to_owned(), string(ordinal.to_string())),
            ("kind".to_owned(), string(kind.as_str())),
            ("request".to_owned(), command_value(command)),
            ("request_digest".to_owned(), string(request_digest.as_str())),
            ("before".to_owned(), optional_head_value(before)),
            ("after".to_owned(), after_value),
        ]),
    )
}

struct CommandReceiptDigestSubject<'a> {
    ordinal: u64,
    previous_receipt_digest: Option<&'a ContentDigest>,
    command: &'a WriterLeaseCommand,
    request_digest: &'a ContentDigest,
    before: Option<&'a WriterLeaseAuthorityHead>,
    after: Option<&'a WriterLeaseAuthorityHead>,
    outcome: CommandOutcome,
    transition_digest: Option<&'a ContentDigest>,
}

fn command_receipt_digest(
    subject: &CommandReceiptDigestSubject<'_>,
) -> Result<ContentDigest, WriterLeaseError> {
    let outcome_value = match subject.outcome {
        CommandOutcome::Applied => "APPLIED",
        CommandOutcome::Denied(reason) => reason.as_str(),
    };
    digest(
        "lattice-writer-lease-command-receipt",
        CanonicalValue::Object(vec![
            ("ordinal".to_owned(), string(subject.ordinal.to_string())),
            (
                "previous_receipt_digest".to_owned(),
                subject
                    .previous_receipt_digest
                    .map_or(CanonicalValue::Null, |value| string(value.as_str())),
            ),
            ("request".to_owned(), command_value(subject.command)),
            (
                "request_digest".to_owned(),
                string(subject.request_digest.as_str()),
            ),
            ("before".to_owned(), optional_head_value(subject.before)),
            ("after".to_owned(), optional_head_value(subject.after)),
            ("outcome".to_owned(), string(outcome_value)),
            (
                "transition_digest".to_owned(),
                subject
                    .transition_digest
                    .map_or(CanonicalValue::Null, |value| string(value.as_str())),
            ),
        ]),
    )
}

fn snapshot_digest(
    aggregate: &VerifiedWriterLeaseAggregate,
) -> Result<ContentDigest, WriterLeaseError> {
    digest("lattice-writer-lease-snapshot", aggregate_value(aggregate))
}

fn command_value(command: &WriterLeaseCommand) -> CanonicalValue {
    let common = |kind: &str,
                  command_id: &str,
                  project_id: &ProjectId,
                  expected: Option<&WriterLeaseAuthorityHead>,
                  observation: &LeaseObservation,
                  extras: Vec<(String, CanonicalValue)>| {
        let mut fields = vec![
            ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
            ("kind".to_owned(), string(kind)),
            ("command_id".to_owned(), string(command_id)),
            ("project_id".to_owned(), string(project_id.as_str())),
            ("expected_head".to_owned(), optional_head_value(expected)),
            ("observation".to_owned(), observation_value(observation)),
        ];
        fields.extend(extras);
        CanonicalValue::Object(fields)
    };
    match command {
        WriterLeaseCommand::Acquire(command) => common(
            "ACQUIRE",
            &command.command_id,
            &command.claim.project_id,
            command.expected_head.as_ref(),
            &command.observation,
            vec![
                ("claim".to_owned(), claim_value(&command.claim)),
                ("expires_at".to_owned(), string(&command.expires_at)),
            ],
        ),
        WriterLeaseCommand::Heartbeat(command) => common(
            "HEARTBEAT",
            &command.command_id,
            &command.project_id,
            Some(&command.expected_head),
            &command.observation,
            vec![("expires_at".to_owned(), string(&command.expires_at))],
        ),
        WriterLeaseCommand::MarkSuspect(command) => common(
            "MARK_SUSPECT",
            &command.command_id,
            &command.project_id,
            Some(&command.expected_head),
            &command.observation,
            Vec::new(),
        ),
        WriterLeaseCommand::ProcessHandoff(command) => common(
            "PROCESS_HANDOFF",
            &command.command_id,
            &command.project_id,
            Some(&command.expected_head),
            &command.observation,
            vec![
                (
                    "successor_holder_process_id".to_owned(),
                    string(command.successor_holder_process_id.get().to_string()),
                ),
                (
                    "successor_holder_process_start_identity".to_owned(),
                    string(command.successor_holder_process_start_identity.as_str()),
                ),
                (
                    "successor_daemon_instance_id".to_owned(),
                    string(&command.successor_daemon_instance_id),
                ),
                (
                    "successor_daemon_epoch".to_owned(),
                    string(command.successor_daemon_epoch.get().to_string()),
                ),
                ("expires_at".to_owned(), string(&command.expires_at)),
                ("evidence".to_owned(), recovery_value(&command.evidence)),
            ],
        ),
        WriterLeaseCommand::Release(command) => common(
            "RELEASE",
            &command.command_id,
            &command.project_id,
            Some(&command.expected_head),
            &command.observation,
            Vec::new(),
        ),
        WriterLeaseCommand::Revoke(command) => common(
            "REVOKE",
            &command.command_id,
            &command.project_id,
            Some(&command.expected_head),
            &command.observation,
            vec![("evidence".to_owned(), recovery_value(&command.evidence))],
        ),
    }
}

fn repository_command_value(command: &WriterLeaseRepositoryCommand) -> CanonicalValue {
    let common = |kind: &str,
                  command_id: &str,
                  project_id: &ProjectId,
                  expected: Option<&WriterLeaseAuthorityHead>,
                  extras: Vec<(String, CanonicalValue)>| {
        let mut fields = vec![
            ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
            ("kind".to_owned(), string(kind)),
            ("command_id".to_owned(), string(command_id)),
            ("project_id".to_owned(), string(project_id.as_str())),
            ("expected_head".to_owned(), optional_head_value(expected)),
        ];
        fields.extend(extras);
        CanonicalValue::Object(fields)
    };
    match command {
        WriterLeaseRepositoryCommand::Acquire(request) => common(
            "ACQUIRE",
            &request.command_id,
            &request.project_id,
            request.expected_head.as_ref(),
            vec![(
                "claim".to_owned(),
                CanonicalValue::Object(vec![
                    ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
                    ("project_id".to_owned(), string(request.project_id.as_str())),
                    (
                        "project_snapshot_id".to_owned(),
                        string(request.project_snapshot_id.as_str()),
                    ),
                    ("task_id".to_owned(), string(request.task_id.as_str())),
                    ("task_revision".to_owned(), string(&request.task_revision)),
                    (
                        "task_spec_digest".to_owned(),
                        string(request.task_spec_digest.as_str()),
                    ),
                    ("attempt_id".to_owned(), string(request.attempt_id.as_str())),
                    ("lease_id".to_owned(), string(&request.lease_id)),
                    (
                        "lease_holder_id".to_owned(),
                        string(&request.lease_holder_id),
                    ),
                    ("worktree_id".to_owned(), string(&request.worktree_id)),
                    (
                        "holder_process_id".to_owned(),
                        string(request.holder_process_id.get().to_string()),
                    ),
                    (
                        "holder_process_start_identity".to_owned(),
                        string(request.holder_process_start_identity.as_str()),
                    ),
                ]),
            )],
        ),
        WriterLeaseRepositoryCommand::Heartbeat(request) => common(
            "HEARTBEAT",
            &request.command_id,
            &request.project_id,
            Some(&request.expected_head),
            Vec::new(),
        ),
        WriterLeaseRepositoryCommand::MarkSuspect(request) => common(
            "MARK_SUSPECT",
            &request.command_id,
            &request.project_id,
            Some(&request.expected_head),
            Vec::new(),
        ),
        WriterLeaseRepositoryCommand::ProcessHandoff(request) => common(
            "PROCESS_HANDOFF",
            &request.command_id,
            &request.project_id,
            Some(&request.expected_head),
            vec![
                (
                    "successor_holder_process_id".to_owned(),
                    string(request.successor_holder_process_id.get().to_string()),
                ),
                (
                    "successor_holder_process_start_identity".to_owned(),
                    string(request.successor_holder_process_start_identity.as_str()),
                ),
                ("evidence".to_owned(), recovery_value(&request.evidence)),
            ],
        ),
        WriterLeaseRepositoryCommand::Release(request) => common(
            "RELEASE",
            &request.command_id,
            &request.project_id,
            Some(&request.expected_head),
            Vec::new(),
        ),
        WriterLeaseRepositoryCommand::Revoke(request) => common(
            "REVOKE",
            &request.command_id,
            &request.project_id,
            Some(&request.expected_head),
            vec![("evidence".to_owned(), recovery_value(&request.evidence))],
        ),
    }
}

fn repository_command_from_live_command(
    command: &WriterLeaseCommand,
) -> WriterLeaseRepositoryCommand {
    match command {
        WriterLeaseCommand::Acquire(command) => {
            WriterLeaseRepositoryCommand::Acquire(WriterLeaseAcquireRequest {
                command_id: command.command_id.clone(),
                expected_head: command.expected_head.clone(),
                project_id: command.claim.project_id.clone(),
                project_snapshot_id: command.claim.project_snapshot_id.clone(),
                task_id: command.claim.task_id.clone(),
                task_revision: command.claim.task_revision.clone(),
                task_spec_digest: command.claim.task_spec_digest.clone(),
                attempt_id: command.claim.attempt_id.clone(),
                lease_id: command.claim.lease_id.clone(),
                lease_holder_id: command.claim.lease_holder_id.clone(),
                worktree_id: command.claim.worktree_id.clone(),
                holder_process_id: command.claim.holder_process_id,
                holder_process_start_identity: command.claim.holder_process_start_identity.clone(),
            })
        }
        WriterLeaseCommand::Heartbeat(command) => {
            WriterLeaseRepositoryCommand::Heartbeat(WriterLeaseHeartbeatRequest {
                command_id: command.command_id.clone(),
                project_id: command.project_id.clone(),
                expected_head: command.expected_head.clone(),
            })
        }
        WriterLeaseCommand::MarkSuspect(command) => {
            WriterLeaseRepositoryCommand::MarkSuspect(WriterLeaseMarkSuspectRequest {
                command_id: command.command_id.clone(),
                project_id: command.project_id.clone(),
                expected_head: command.expected_head.clone(),
            })
        }
        WriterLeaseCommand::ProcessHandoff(command) => {
            WriterLeaseRepositoryCommand::ProcessHandoff(WriterLeaseProcessHandoffRequest {
                command_id: command.command_id.clone(),
                project_id: command.project_id.clone(),
                expected_head: command.expected_head.clone(),
                successor_holder_process_id: command.successor_holder_process_id,
                successor_holder_process_start_identity: command
                    .successor_holder_process_start_identity
                    .clone(),
                evidence: command.evidence.clone(),
            })
        }
        WriterLeaseCommand::Release(command) => {
            WriterLeaseRepositoryCommand::Release(WriterLeaseReleaseRequest {
                command_id: command.command_id.clone(),
                project_id: command.project_id.clone(),
                expected_head: command.expected_head.clone(),
            })
        }
        WriterLeaseCommand::Revoke(command) => {
            WriterLeaseRepositoryCommand::Revoke(WriterLeaseRevokeRequest {
                command_id: command.command_id.clone(),
                project_id: command.project_id.clone(),
                expected_head: command.expected_head.clone(),
                evidence: command.evidence.clone(),
            })
        }
    }
}

fn claim_value(claim: &AcquireClaim) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
        ("project_id".to_owned(), string(claim.project_id.as_str())),
        (
            "project_snapshot_id".to_owned(),
            string(claim.project_snapshot_id.as_str()),
        ),
        ("task_id".to_owned(), string(claim.task_id.as_str())),
        ("task_revision".to_owned(), string(&claim.task_revision)),
        (
            "task_spec_digest".to_owned(),
            string(claim.task_spec_digest.as_str()),
        ),
        ("attempt_id".to_owned(), string(claim.attempt_id.as_str())),
        ("lease_id".to_owned(), string(&claim.lease_id)),
        ("lease_holder_id".to_owned(), string(&claim.lease_holder_id)),
        ("worktree_id".to_owned(), string(&claim.worktree_id)),
        (
            "holder_process_id".to_owned(),
            string(claim.holder_process_id.get().to_string()),
        ),
        (
            "holder_process_start_identity".to_owned(),
            string(claim.holder_process_start_identity.as_str()),
        ),
        (
            "daemon_instance_id".to_owned(),
            string(&claim.daemon_instance_id),
        ),
        (
            "daemon_epoch".to_owned(),
            string(claim.daemon_epoch.get().to_string()),
        ),
    ])
}

fn observation_value(observation: &LeaseObservation) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
        (
            "runtime".to_owned(),
            string(if observation.runtime.is_live() {
                "LIVE"
            } else {
                "FAKE"
            }),
        ),
        (
            "admission".to_owned(),
            string(observation.admission.as_str()),
        ),
        ("observed_at".to_owned(), string(&observation.observed_at)),
        (
            "time_observation_digest".to_owned(),
            string(observation.time_observation_digest.as_str()),
        ),
        (
            "admission_observation_digest".to_owned(),
            string(observation.admission_observation_digest.as_str()),
        ),
    ])
}

fn recovery_value(evidence: &RecoveryEvidence) -> CanonicalValue {
    match evidence {
        RecoveryEvidence::ProcessDeath {
            holder_process_id,
            holder_process_start_identity,
            holder_daemon_instance_id,
            evidence_digest,
        } => CanonicalValue::Object(vec![
            ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
            ("kind".to_owned(), string("PROCESS_DEATH")),
            (
                "holder_process_id".to_owned(),
                string(holder_process_id.get().to_string()),
            ),
            (
                "holder_process_start_identity".to_owned(),
                string(holder_process_start_identity.as_str()),
            ),
            (
                "holder_daemon_instance_id".to_owned(),
                string(holder_daemon_instance_id),
            ),
            (
                "evidence_digest".to_owned(),
                string(evidence_digest.as_str()),
            ),
        ]),
        RecoveryEvidence::LeadershipReplaced {
            replaced_daemon_instance_id,
            replaced_epoch,
            replacement_daemon_instance_id,
            replacement_epoch,
            evidence_digest,
        } => CanonicalValue::Object(vec![
            ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
            ("kind".to_owned(), string("LEADERSHIP_REPLACED")),
            (
                "replaced_daemon_instance_id".to_owned(),
                string(replaced_daemon_instance_id),
            ),
            (
                "replaced_epoch".to_owned(),
                string(replaced_epoch.get().to_string()),
            ),
            (
                "replacement_daemon_instance_id".to_owned(),
                string(replacement_daemon_instance_id),
            ),
            (
                "replacement_epoch".to_owned(),
                string(replacement_epoch.get().to_string()),
            ),
            (
                "evidence_digest".to_owned(),
                string(evidence_digest.as_str()),
            ),
        ]),
    }
}

fn identity_value(identity: &WriterLeaseIdentity) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
        (
            "project_id".to_owned(),
            string(identity.project_id().as_str()),
        ),
        (
            "project_snapshot_id".to_owned(),
            string(identity.project_snapshot_id().as_str()),
        ),
        ("task_id".to_owned(), string(identity.task_id().as_str())),
        ("task_revision".to_owned(), string(identity.task_revision())),
        (
            "task_spec_digest".to_owned(),
            string(identity.task_spec_digest().as_str()),
        ),
        (
            "attempt_id".to_owned(),
            string(identity.attempt_id().as_str()),
        ),
        ("lease_id".to_owned(), string(identity.lease_id())),
        (
            "lease_holder_id".to_owned(),
            string(identity.lease_holder_id()),
        ),
        ("worktree_id".to_owned(), string(identity.worktree_id())),
        (
            "holder_process_id".to_owned(),
            string(identity.holder_process_id().get().to_string()),
        ),
        (
            "holder_process_start_identity".to_owned(),
            string(identity.holder_process_start_identity().as_str()),
        ),
        (
            "daemon_instance_id".to_owned(),
            string(identity.daemon_instance_id()),
        ),
        (
            "daemon_epoch".to_owned(),
            string(identity.daemon_epoch().get().to_string()),
        ),
        (
            "fencing_token".to_owned(),
            string(identity.fencing_token().get().to_string()),
        ),
    ])
}

#[allow(clippy::too_many_arguments)]
fn authority_value(
    runtime: RuntimeKind,
    identity: &WriterLeaseIdentity,
    status: WriterLeaseStatus,
    revision: WriterLeaseRevision,
    admission: RuntimeAdmissionMode,
    acquired_at: &str,
    heartbeat_at: &str,
    expires_at: &str,
    time_observation_digest: &ContentDigest,
    admission_observation_digest: &ContentDigest,
    transition_digest: &ContentDigest,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
        (
            "contract_version".to_owned(),
            string(CONTRACT_VERSION.to_string()),
        ),
        ("producer_id".to_owned(), string(WRITER_LEASE_PRODUCER_ID)),
        (
            "producer_version".to_owned(),
            string(WRITER_LEASE_PRODUCER_VERSION),
        ),
        (
            "runtime".to_owned(),
            string(if runtime.is_live() { "LIVE" } else { "FAKE" }),
        ),
        ("identity".to_owned(), identity_value(identity)),
        ("status".to_owned(), string(status.as_str())),
        ("revision".to_owned(), string(revision.get().to_string())),
        ("admission".to_owned(), string(admission.as_str())),
        ("acquired_at".to_owned(), string(acquired_at)),
        ("heartbeat_at".to_owned(), string(heartbeat_at)),
        ("expires_at".to_owned(), string(expires_at)),
        (
            "time_observation_digest".to_owned(),
            string(time_observation_digest.as_str()),
        ),
        (
            "admission_observation_digest".to_owned(),
            string(admission_observation_digest.as_str()),
        ),
        (
            "transition_digest".to_owned(),
            string(transition_digest.as_str()),
        ),
    ])
}

fn head_value(head: &WriterLeaseAuthorityHead) -> CanonicalValue {
    let receipt_fields = authority_value(
        head.runtime(),
        head.identity(),
        head.status(),
        head.revision(),
        head.runtime_admission(),
        head.acquired_at(),
        head.heartbeat_at(),
        head.expires_at(),
        head.time_observation_digest(),
        head.admission_observation_digest(),
        head.transition_digest(),
    );
    let CanonicalValue::Object(mut fields) = receipt_fields else {
        unreachable!("authority_value always returns an object");
    };
    fields.push((
        "receipt_digest".to_owned(),
        string(head.receipt_digest().as_str()),
    ));
    CanonicalValue::Object(fields)
}

fn optional_head_value(head: Option<&WriterLeaseAuthorityHead>) -> CanonicalValue {
    head.map_or(CanonicalValue::Null, head_value)
}

fn aggregate_value(aggregate: &VerifiedWriterLeaseAggregate) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
        (
            "project_id".to_owned(),
            string(aggregate.project_id.as_str()),
        ),
        (
            "fencing_high_water".to_owned(),
            string(aggregate.fencing_high_water.to_string()),
        ),
        (
            "revision".to_owned(),
            string(aggregate.revision.to_string()),
        ),
        (
            "command_high_water".to_owned(),
            string(aggregate.command_receipts.len().to_string()),
        ),
        (
            "command_tail_digest".to_owned(),
            aggregate
                .command_receipts
                .last()
                .map_or(CanonicalValue::Null, |receipt| {
                    string(receipt.receipt_digest.as_str())
                }),
        ),
        (
            "current_receipt".to_owned(),
            aggregate
                .current_receipt
                .as_ref()
                .map_or(CanonicalValue::Null, |receipt| head_value(&receipt.head())),
        ),
        (
            "transitions".to_owned(),
            CanonicalValue::Array(
                aggregate
                    .transitions
                    .iter()
                    .map(transition_record_value)
                    .collect(),
            ),
        ),
        (
            "commands".to_owned(),
            CanonicalValue::Array(
                aggregate
                    .command_receipts
                    .iter()
                    .map(command_receipt_value)
                    .collect(),
            ),
        ),
    ])
}

fn transition_record_value(record: &WriterLeaseTransitionRecord) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
        ("ordinal".to_owned(), string(record.ordinal.to_string())),
        ("command_id".to_owned(), string(&record.command_id)),
        ("kind".to_owned(), string(record.kind.as_str())),
        (
            "request_digest".to_owned(),
            string(record.request_digest.as_str()),
        ),
        (
            "before".to_owned(),
            optional_head_value(record.before.as_ref()),
        ),
        (
            "after".to_owned(),
            record
                .after
                .as_ref()
                .map_or(CanonicalValue::Null, |receipt| head_value(&receipt.head())),
        ),
        (
            "transition_digest".to_owned(),
            string(record.transition_digest.as_str()),
        ),
    ])
}

fn command_receipt_value(receipt: &WriterLeaseCommandReceipt) -> CanonicalValue {
    let (outcome, denial_reason) = match receipt.outcome {
        CommandOutcome::Applied => ("APPLIED", CanonicalValue::Null),
        CommandOutcome::Denied(reason) => ("DENIED", string(reason.as_str())),
    };
    CanonicalValue::Object(vec![
        ("schema_version".to_owned(), string(SNAPSHOT_VERSION)),
        ("ordinal".to_owned(), string(receipt.ordinal.to_string())),
        (
            "previous_receipt_digest".to_owned(),
            receipt
                .previous_receipt_digest
                .as_ref()
                .map_or(CanonicalValue::Null, |value| string(value.as_str())),
        ),
        ("request".to_owned(), command_value(&receipt.request)),
        (
            "request_digest".to_owned(),
            string(receipt.request_digest.as_str()),
        ),
        (
            "before".to_owned(),
            optional_head_value(receipt.before.as_ref()),
        ),
        (
            "after".to_owned(),
            optional_head_value(receipt.after.as_ref()),
        ),
        ("outcome".to_owned(), string(outcome)),
        ("denial_reason".to_owned(), denial_reason),
        (
            "transition_digest".to_owned(),
            receipt
                .transition_digest
                .as_ref()
                .map_or(CanonicalValue::Null, |value| string(value.as_str())),
        ),
        (
            "receipt_digest".to_owned(),
            string(receipt.receipt_digest.as_str()),
        ),
    ])
}

#[allow(clippy::needless_pass_by_value)]
fn digest(schema_id: &str, value: CanonicalValue) -> Result<ContentDigest, WriterLeaseError> {
    let domain =
        HashDomain::new(schema_id, SNAPSHOT_VERSION).map_err(|_| WriterLeaseError::Canonical)?;
    let digest = canonical_sha256(&domain, &value).map_err(|_| WriterLeaseError::Canonical)?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| WriterLeaseError::Contract)
}

fn parse_canonical_utc(value: &str) -> Result<OffsetDateTime, WriterLeaseError> {
    let parsed =
        OffsetDateTime::parse(value, &Rfc3339).map_err(|_| WriterLeaseError::InvalidTimestamp)?;
    if parsed.offset() != UtcOffset::UTC
        || parsed
            .format(&Rfc3339)
            .map_err(|_| WriterLeaseError::InvalidTimestamp)?
            != value
    {
        return Err(WriterLeaseError::InvalidTimestamp);
    }
    Ok(parsed)
}

fn valid_identifier(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn is_zero_digest(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}

fn string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_plan_is_terminal_denial_without_partial_state() {
        let project = ProjectId::new("project-overflow").expect("project");
        let mut aggregate = VerifiedWriterLeaseAggregate::vacant(project.clone());
        aggregate.fencing_high_water = MAX_SIGNED_BIGINT;
        let command = crate::test_support::acquire_command(&project, "overflow", 1);
        let plan = plan_command(&aggregate, &command).expect("plan");
        assert_eq!(
            plan.receipt.outcome,
            CommandOutcome::Denied(LeaseDenial::CounterExhausted)
        );
        let next = apply_plan(&aggregate, plan).expect("apply");
        assert_eq!(next.fencing_high_water, MAX_SIGNED_BIGINT);
        assert_eq!(next.revision, 0);
        assert!(next.current_receipt.is_none());
    }

    #[test]
    fn transition_digest_binds_the_command_ordinal() {
        let project = ProjectId::new("project-ordinal").expect("project");
        let command = crate::test_support::acquire_command(&project, "ordinal", 1);
        let request_digest = crate::test_support::digest('a');
        let first = transition_digest(
            1,
            TransitionKind::Acquire,
            &command,
            &request_digest,
            None,
            None,
        )
        .expect("first digest");
        let second = transition_digest(
            2,
            TransitionKind::Acquire,
            &command,
            &request_digest,
            None,
            None,
        )
        .expect("second digest");
        assert_ne!(first, second);
    }
}

/// Test-fixture constructors shared by integration and Policy composition
/// tests. They produce no authority beyond explicit `RuntimeKind::Fake`.
#[allow(clippy::missing_panics_doc, clippy::wildcard_imports)]
pub mod test_support {
    use super::*;

    /// Returns a deterministic non-zero digest.
    #[must_use]
    pub fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("fixture digest")
    }

    /// Returns a complete deterministic acquire command.
    #[must_use]
    pub fn acquire_command(
        project_id: &ProjectId,
        command_id: &str,
        daemon_epoch: u64,
    ) -> WriterLeaseCommand {
        WriterLeaseCommand::Acquire(AcquireCommand {
            command_id: command_id.to_owned(),
            expected_head: None,
            claim: AcquireClaim {
                project_id: project_id.clone(),
                project_snapshot_id: ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
                task_id: TaskId::new("task-1").expect("task"),
                task_revision: "1".to_owned(),
                task_spec_digest: digest('1'),
                attempt_id: AttemptId::new("attempt-1").expect("attempt"),
                lease_id: format!("lease-{command_id}"),
                lease_holder_id: "implementer-1".to_owned(),
                worktree_id: "worktree-1".to_owned(),
                holder_process_id: HolderProcessId::new(42).expect("pid"),
                holder_process_start_identity: digest('2'),
                daemon_instance_id: "daemon-1".to_owned(),
                daemon_epoch: DaemonEpoch::new(daemon_epoch).expect("epoch"),
            },
            observation: observation(RuntimeAdmissionMode::Active, "2026-07-29T00:00:00Z"),
            expires_at: "2026-07-29T00:10:00Z".to_owned(),
        })
    }

    /// Returns an explicit fake observation.
    #[must_use]
    pub fn observation(admission: RuntimeAdmissionMode, observed_at: &str) -> LeaseObservation {
        LeaseObservation {
            runtime: RuntimeKind::Fake,
            admission,
            observed_at: observed_at.to_owned(),
            time_observation_digest: digest('3'),
            admission_observation_digest: digest('4'),
        }
    }
}
