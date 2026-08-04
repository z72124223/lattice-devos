//! Pure Writer Lease 1.0 semantics and a deterministic non-durable fake.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
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
    const fn as_str(self) -> &'static str {
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
    Release(ReleaseCommand),
    Revoke(RevokeCommand),
}

impl WriterLeaseCommand {
    fn command_id(&self) -> &str {
        match self {
            Self::Acquire(command) => &command.command_id,
            Self::Heartbeat(command) => &command.command_id,
            Self::MarkSuspect(command) => &command.command_id,
            Self::Release(command) => &command.command_id,
            Self::Revoke(command) => &command.command_id,
        }
    }

    fn project_id(&self) -> &ProjectId {
        match self {
            Self::Acquire(command) => &command.claim.project_id,
            Self::Heartbeat(command) => &command.project_id,
            Self::MarkSuspect(command) => &command.project_id,
            Self::Release(command) => &command.project_id,
            Self::Revoke(command) => &command.project_id,
        }
    }

    fn observation(&self) -> &LeaseObservation {
        match self {
            Self::Acquire(command) => &command.observation,
            Self::Heartbeat(command) => &command.observation,
            Self::MarkSuspect(command) => &command.observation,
            Self::Release(command) => &command.observation,
            Self::Revoke(command) => &command.observation,
        }
    }

    fn expected_head(&self) -> Option<&WriterLeaseAuthorityHead> {
        match self {
            Self::Acquire(command) => command.expected_head.as_ref(),
            Self::Heartbeat(command) => Some(&command.expected_head),
            Self::MarkSuspect(command) => Some(&command.expected_head),
            Self::Release(command) => Some(&command.expected_head),
            Self::Revoke(command) => Some(&command.expected_head),
        }
    }
}

/// Applied transition kind retained in replay evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionKind {
    Acquire,
    Heartbeat,
    MarkSuspect,
    Release,
    Revoke,
}

impl TransitionKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Acquire => "ACQUIRE",
            Self::Heartbeat => "HEARTBEAT",
            Self::MarkSuspect => "MARK_SUSPECT",
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
        || replayed.export_untrusted() != *snapshot
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
