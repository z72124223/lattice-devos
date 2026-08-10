//! Abstract I/O ports for LATTICE orchestration.

use std::error::Error;
use std::fmt;

use lattice_contracts::{
    CodeSnapshotEvidence, CodexDeliveryEvidence, CodexDeliveryRequest, CodexEvidence,
    CodexRunRequest, Component, DeliveryOutcomeEvidence, DeliveryOutcomeRequest, DeliveryReceipt,
    DeliveryRunRequest, DeliveryStage, DeliveryStatusRequest, DurableIntentEvidence,
    FixedTestEvidence, GatewayActorId, GatewayCommandId, GatewayPeerContext, GatewayReply,
    GatewayRequest, GitCommitEvidence, GraphMemoryPersistenceEvidence, GraphMemoryReceipt,
    GraphMemoryRunRequest, GraphifyBuildRequest, GraphifyEvidence, GraphifyRawEvidence,
    HermesEvidence, HermesReflectionCandidate, HermesReflectionReceipt, HermesResearchRequest,
    MemoryRetrievalPlan, NormalizedGraphAnalysis, PreparedWorkspaceEvidence, ProjectId, RequestId,
    StorePhysicalHead, StoreScope, StoreTransactionReceipt, StoreTransactionRequest,
    WorkspaceChangeEvidence,
};
use lattice_task_domain::{
    ReflectionCandidateKind, ReflectionFailureKind, ReflectionState, TaskState,
};

/// Result type returned by every LATTICE port.
pub type PortResult<T> = Result<T, PortError>;

/// Result returned by the inbound Rust-core gateway service.
///
/// This boundary is not an external adapter port, so its errors deliberately
/// do not carry a [`Component`] that could falsely attribute a core routing or
/// reply-binding failure to `OpenClaw` or another adapter.
pub type GatewayServiceResult<T> = Result<T, GatewayServiceError>;

/// Result returned by the typed physical control-store boundary.
pub type ControlStoreResult<T> = Result<T, ControlStoreError>;

/// Result returned by each typed delivery effect port.
pub type DeliveryPortResult<T> = Result<T, DeliveryPortError>;

/// Result returned by each exact graph-memory effect port.
pub type GraphMemoryPortResult<T> = Result<T, GraphMemoryPortError>;

/// Result returned by the authoritative Task lifecycle repository boundary.
pub type TaskLifecycleResult<T> = Result<T, TaskLifecycleError>;

/// Result returned by the independent Reflection journal boundary.
pub type TaskReflectionResult<T> = Result<T, TaskReflectionError>;

/// Maximum number of typed Reflection history entries returned in one read.
pub const MAX_TASK_REFLECTION_HISTORY_EVENTS: usize = 64;

/// Result returned by the single bounded task execution port.
pub type ControlledTaskExecutionResult<T> = Result<T, ControlledTaskExecutionError>;

/// Whether a failed controlled execution is known not to have completed or
/// requires reconciliation before any retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlledTaskExecutionErrorKind {
    Known,
    Ambiguous,
}

/// Secret-free controlled execution failure returned to the orchestrator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlledTaskExecutionError {
    kind: ControlledTaskExecutionErrorKind,
    code: &'static str,
}

impl ControlledTaskExecutionError {
    #[must_use]
    pub const fn new(kind: ControlledTaskExecutionErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> ControlledTaskExecutionErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ControlledTaskExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for ControlledTaskExecutionError {}

/// Existing-orchestrator effect boundary for one server-owned bounded task.
/// The current Writer Lease head is supplied to the adapter but never exposed
/// through MCP or accepted from the GPT caller.
pub trait ControlledTaskExecutionPort {
    /// Executes the one server-owned task under the exact current writer head.
    ///
    /// # Errors
    ///
    /// Returns a bounded known or ambiguous execution failure.
    fn execute(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        writer_authority: &lattice_contracts::WriterLeaseAuthorityHead,
        writer_guard: &mut dyn WriterAuthorityGuardPort,
    ) -> ControlledTaskExecutionResult<lattice_contracts::ContentDigest>;
}

/// Currentness assertion supplied by Orchestrator from the same injected
/// Writer Lease repository that allocated the fence. Execution adapters may
/// request checks but cannot acquire, release, or replace writer authority.
pub trait WriterAuthorityGuardPort {
    /// Proves the exact authority is still current at one mutation boundary.
    ///
    /// # Errors
    ///
    /// Returns a bounded known mismatch or ambiguous owner failure.
    fn assert_current(
        &mut self,
        expected: &lattice_contracts::WriterLeaseAuthorityHead,
    ) -> ControlledTaskExecutionResult<()>;
}

/// Closed durable Task lifecycle repository failure classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskLifecycleErrorKind {
    Rejected,
    Unavailable,
    Ambiguous,
    Corrupt,
}

/// Bounded Task lifecycle failure without database or task-source contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLifecycleError {
    kind: TaskLifecycleErrorKind,
    code: &'static str,
}

impl TaskLifecycleError {
    #[must_use]
    pub const fn new(kind: TaskLifecycleErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> TaskLifecycleErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for TaskLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for TaskLifecycleError {}

/// Replay-derived authoritative Task lifecycle projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLifecycleEvidence {
    binding: lattice_contracts::SubjectBinding,
    admitted: bool,
    state: TaskState,
    ledger_head_digest: lattice_contracts::ContentDigest,
    core_head_digest: Option<lattice_contracts::ContentDigest>,
    result_digest: Option<lattice_contracts::ContentDigest>,
}

impl TaskLifecycleEvidence {
    #[must_use]
    pub const fn new(
        binding: lattice_contracts::SubjectBinding,
        admitted: bool,
        state: TaskState,
        ledger_head_digest: lattice_contracts::ContentDigest,
        result_digest: Option<lattice_contracts::ContentDigest>,
    ) -> Self {
        Self {
            binding,
            admitted,
            state,
            ledger_head_digest,
            core_head_digest: None,
            result_digest,
        }
    }

    /// Constructs a lifecycle projection whose current journal head may be
    /// newer than its last authoritative core event.
    #[must_use]
    pub const fn new_with_core_head(
        binding: lattice_contracts::SubjectBinding,
        admitted: bool,
        state: TaskState,
        ledger_head_digest: lattice_contracts::ContentDigest,
        core_head_digest: lattice_contracts::ContentDigest,
        result_digest: Option<lattice_contracts::ContentDigest>,
    ) -> Self {
        Self {
            binding,
            admitted,
            state,
            ledger_head_digest,
            core_head_digest: Some(core_head_digest),
            result_digest,
        }
    }

    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::SubjectBinding {
        &self.binding
    }

    #[must_use]
    pub const fn admitted(&self) -> bool {
        self.admitted
    }

    #[must_use]
    pub const fn state(&self) -> TaskState {
        self.state
    }

    #[must_use]
    pub const fn ledger_head_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.ledger_head_digest
    }

    /// Returns the verified head immediately after the last authoritative core
    /// Task event. Independent Reflection events cannot advance this value.
    #[must_use]
    pub fn core_head_digest(&self) -> &lattice_contracts::ContentDigest {
        self.core_head_digest
            .as_ref()
            .unwrap_or(&self.ledger_head_digest)
    }

    #[must_use]
    pub const fn result_digest(&self) -> Option<&lattice_contracts::ContentDigest> {
        self.result_digest.as_ref()
    }
}

/// PostgreSQL-backed authoritative lifecycle boundary used by the sole
/// orchestrator. Implementations may persist and replay but never decide
/// Task Domain transition legality.
pub trait TaskLifecyclePort {
    /// Idempotently admits one exact caller retry key and Task binding.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn admit(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        client_request_id: &str,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence>;

    /// Appends one Task Domain-approved state transition.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn transition(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        from: TaskState,
        to: TaskState,
        writer_authority: Option<&lattice_contracts::WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence>;

    /// Persists the exact governed execution result under the current writer.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn record_result(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        result_digest: &lattice_contracts::ContentDigest,
        writer_authority: &lattice_contracts::WriterLeaseAuthorityHead,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence>;

    /// Replays the authoritative lifecycle projection.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn load(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence>;
}

/// Closed durable Reflection repository failure classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskReflectionErrorKind {
    /// The typed command or state edge was rejected.
    Rejected,
    /// The fixed repository was unavailable.
    Unavailable,
    /// A mutation may have committed but its result is unknown.
    Ambiguous,
    /// Verified durable history violated the closed Reflection contract.
    Corrupt,
}

/// Secret-free Reflection repository failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskReflectionError {
    kind: TaskReflectionErrorKind,
    code: &'static str,
}

impl TaskReflectionError {
    /// Constructs one fixed Reflection failure.
    #[must_use]
    pub const fn new(kind: TaskReflectionErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    /// Returns the closed failure class.
    #[must_use]
    pub const fn kind(&self) -> TaskReflectionErrorKind {
        self.kind
    }

    /// Returns the fixed machine-facing code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for TaskReflectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for TaskReflectionError {}

/// Closed semantic kind of one immutable Reflection journal event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskReflectionEventKind {
    /// One exact work generation was durably queued.
    Pending,
    /// One exact work generation was claimed.
    Claimed,
    /// One fixed failure category was recorded.
    Failure(ReflectionFailureKind),
    /// A later retry generation was authorized.
    RetryPending,
    /// Core output remains usable without successful Reflection.
    Degraded,
    /// Hermes appended a non-authoritative digest-only candidate.
    Candidate(ReflectionCandidateKind),
}

/// Digest-only proof material exposed by one authorized history event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskReflectionEventReference {
    /// The event has no external evidence leaf.
    None,
    /// The event commits one bounded evidence digest.
    Evidence(lattice_contracts::ContentDigest),
    /// A non-authoritative candidate and the exact history window it used.
    Candidate {
        /// Digest of the bounded candidate value retained outside this port.
        candidate_digest: lattice_contracts::ContentDigest,
        /// Digest of the exact authorized history window.
        history_digest: lattice_contracts::ContentDigest,
        /// Exact bounded page that authorized the candidate.
        history_query: TaskReflectionHistoryQuery,
    },
}

/// Stable keyset query for one bounded Reflection-history page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskReflectionHistoryQuery {
    before_sequence: Option<u64>,
    limit: u16,
}

impl TaskReflectionHistoryQuery {
    /// Constructs an exclusive sequence cursor with a fixed bounded limit.
    ///
    /// # Errors
    ///
    /// Rejects a zero cursor, zero limit, or a limit above
    /// [`MAX_TASK_REFLECTION_HISTORY_EVENTS`].
    pub fn new(before_sequence: Option<u64>, limit: usize) -> TaskReflectionResult<Self> {
        if before_sequence == Some(0) {
            return Err(TaskReflectionError::new(
                TaskReflectionErrorKind::Rejected,
                "LATTICE_REFLECTION_HISTORY_CURSOR_REJECTED",
            ));
        }
        if limit == 0 || limit > MAX_TASK_REFLECTION_HISTORY_EVENTS {
            return Err(TaskReflectionError::new(
                TaskReflectionErrorKind::Rejected,
                "LATTICE_REFLECTION_HISTORY_LIMIT_REJECTED",
            ));
        }
        let limit = u16::try_from(limit).map_err(|_| {
            TaskReflectionError::new(
                TaskReflectionErrorKind::Rejected,
                "LATTICE_REFLECTION_HISTORY_LIMIT_REJECTED",
            )
        })?;
        Ok(Self {
            before_sequence,
            limit,
        })
    }

    /// Constructs the newest bounded page.
    ///
    /// # Errors
    ///
    /// Rejects zero or a limit above [`MAX_TASK_REFLECTION_HISTORY_EVENTS`].
    pub fn latest(limit: usize) -> TaskReflectionResult<Self> {
        Self::new(None, limit)
    }

    /// Returns the exclusive sequence cursor, or `None` for the newest page.
    #[must_use]
    pub const fn before_sequence(self) -> Option<u64> {
        self.before_sequence
    }

    /// Returns the validated page limit.
    #[must_use]
    pub const fn limit(self) -> usize {
        self.limit as usize
    }
}

/// One bounded typed projection of an immutable Reflection journal event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskReflectionHistoryEvent {
    sequence: u64,
    generation: u64,
    kind: TaskReflectionEventKind,
    reference: TaskReflectionEventReference,
    subject_digest: lattice_contracts::ContentDigest,
    event_digest: lattice_contracts::ContentDigest,
}

impl TaskReflectionHistoryEvent {
    /// Constructs one event projection after verified replay.
    #[must_use]
    pub const fn new(
        sequence: u64,
        generation: u64,
        kind: TaskReflectionEventKind,
        reference: TaskReflectionEventReference,
        subject_digest: lattice_contracts::ContentDigest,
        event_digest: lattice_contracts::ContentDigest,
    ) -> Self {
        Self {
            sequence,
            generation,
            kind,
            reference,
            subject_digest,
            event_digest,
        }
    }

    /// Returns the authoritative Task Ledger sequence.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the deterministic Reflection generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the closed semantic event kind.
    #[must_use]
    pub const fn kind(&self) -> TaskReflectionEventKind {
        self.kind
    }

    /// Returns only digest commitments, never raw evidence or candidate text.
    #[must_use]
    pub const fn reference(&self) -> &TaskReflectionEventReference {
        &self.reference
    }

    /// Returns the authoritative typed subject commitment.
    #[must_use]
    pub const fn subject_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.subject_digest
    }

    /// Returns the immutable Task Ledger event commitment.
    #[must_use]
    pub const fn event_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.event_digest
    }
}

/// Replay-derived independent Reflection projection for one exact Task.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskReflectionEvidence {
    binding: lattice_contracts::SubjectBinding,
    state: ReflectionState,
    generation: u64,
    core_head_digest: lattice_contracts::ContentDigest,
    journal_head_digest: lattice_contracts::ContentDigest,
    pending_admission_digest: Option<lattice_contracts::ContentDigest>,
    claim_digest: Option<lattice_contracts::ContentDigest>,
}

impl TaskReflectionEvidence {
    /// Constructs one fully replayed Reflection projection.
    #[must_use]
    pub const fn new(
        binding: lattice_contracts::SubjectBinding,
        state: ReflectionState,
        generation: u64,
        core_head_digest: lattice_contracts::ContentDigest,
        journal_head_digest: lattice_contracts::ContentDigest,
        pending_admission_digest: Option<lattice_contracts::ContentDigest>,
        claim_digest: Option<lattice_contracts::ContentDigest>,
    ) -> Self {
        Self {
            binding,
            state,
            generation,
            core_head_digest,
            journal_head_digest,
            pending_admission_digest,
            claim_digest,
        }
    }

    /// Returns the exact Task binding.
    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::SubjectBinding {
        &self.binding
    }

    /// Returns the independent Reflection state.
    #[must_use]
    pub const fn state(&self) -> ReflectionState {
        self.state
    }

    /// Returns the deterministic queue generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the immutable completed-core anchor.
    #[must_use]
    pub const fn core_head_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.core_head_digest
    }

    /// Returns the current complete append-only journal head.
    #[must_use]
    pub const fn journal_head_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.journal_head_digest
    }

    /// Returns the immutable admission for the current generation.
    #[must_use]
    pub const fn pending_admission_digest(&self) -> Option<&lattice_contracts::ContentDigest> {
        self.pending_admission_digest.as_ref()
    }

    /// Returns the current exact claim commitment, when claimed.
    #[must_use]
    pub const fn claim_digest(&self) -> Option<&lattice_contracts::ContentDigest> {
        self.claim_digest.as_ref()
    }
}

/// Bounded authorized Reflection history without raw payloads or database authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskReflectionHistory {
    binding: lattice_contracts::SubjectBinding,
    core_head_digest: lattice_contracts::ContentDigest,
    journal_head_digest: lattice_contracts::ContentDigest,
    history_digest: lattice_contracts::ContentDigest,
    query: TaskReflectionHistoryQuery,
    next_before_sequence: Option<u64>,
    events: Vec<TaskReflectionHistoryEvent>,
}

impl TaskReflectionHistory {
    /// Constructs one bounded history after exact replay and hashing.
    #[must_use]
    pub const fn new(
        binding: lattice_contracts::SubjectBinding,
        core_head_digest: lattice_contracts::ContentDigest,
        journal_head_digest: lattice_contracts::ContentDigest,
        history_digest: lattice_contracts::ContentDigest,
        query: TaskReflectionHistoryQuery,
        next_before_sequence: Option<u64>,
        events: Vec<TaskReflectionHistoryEvent>,
    ) -> Self {
        Self {
            binding,
            core_head_digest,
            journal_head_digest,
            history_digest,
            query,
            next_before_sequence,
            events,
        }
    }

    /// Returns the exact Task binding.
    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::SubjectBinding {
        &self.binding
    }

    /// Returns the immutable completed-core anchor.
    #[must_use]
    pub const fn core_head_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.core_head_digest
    }

    /// Returns the current complete journal head.
    #[must_use]
    pub const fn journal_head_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.journal_head_digest
    }

    /// Returns the commitment to the exact returned history window.
    #[must_use]
    pub const fn history_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.history_digest
    }

    /// Returns the exact query bound into this page commitment.
    #[must_use]
    pub const fn query(&self) -> TaskReflectionHistoryQuery {
        self.query
    }

    /// Returns the exclusive cursor for the next older page.
    #[must_use]
    pub const fn next_before_sequence(&self) -> Option<u64> {
        self.next_before_sequence
    }

    /// Returns chronologically ordered typed events.
    #[must_use]
    pub fn events(&self) -> &[TaskReflectionHistoryEvent] {
        &self.events
    }
}

/// Known-Task Reflection queue and lifecycle mutation capability.
///
/// This first-slice port intentionally has no cross-Task listing or claim-next
/// operation. A caller must already hold the exact Task binding. Queue, claim,
/// retry, degraded, and candidate operations are confined to an independently
/// completed core Task. Direct core-failure and fixed-output-rejection records
/// are durable, bounded, read-only history and cannot acquire a claim through
/// this port.
pub trait TaskReflectionQueuePort {
    /// Idempotently appends one durable pending generation after core completion.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn ensure_pending(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
    ) -> TaskReflectionResult<TaskReflectionEvidence>;

    /// Claims the current pending generation by exact command identity.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn claim_pending(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        command_id: &str,
    ) -> TaskReflectionResult<TaskReflectionEvidence>;

    /// Appends one typed failure without changing the core Task projection.
    ///
    /// Core `Failed`/`Blocked`/`Cancelled` failures and fixed-output rejection
    /// are terminal evidence. A Hermes failure instead requires an exact claim
    /// in the post-completion queue lane.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn record_failure(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        command_id: &str,
        kind: ReflectionFailureKind,
        evidence_digest: &lattice_contracts::ContentDigest,
    ) -> TaskReflectionResult<TaskReflectionEvidence>;

    /// Authorizes a later retry generation.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn mark_retry_pending(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        command_id: &str,
    ) -> TaskReflectionResult<TaskReflectionEvidence>;

    /// Marks the Reflection projection degraded without changing core success.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn mark_degraded(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        command_id: &str,
        evidence_digest: &lattice_contracts::ContentDigest,
    ) -> TaskReflectionResult<TaskReflectionEvidence>;

    /// Replays one exact known-Task Reflection projection.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn load_reflection(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
    ) -> TaskReflectionResult<TaskReflectionEvidence>;
}

/// Hermes-only bounded read capability over one exact known Task.
pub trait HermesTaskReflectionHistoryPort {
    /// Reads at most the fixed maximum number of typed digest-only events.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn read_authorized_history(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        query: TaskReflectionHistoryQuery,
    ) -> TaskReflectionResult<TaskReflectionHistory>;
}

/// Hermes-only append capability for non-authoritative candidate digests.
pub trait HermesTaskReflectionCandidatePort {
    /// Appends a closed candidate kind bound to the current exact claim.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn append_candidate(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        command_id: &str,
        kind: ReflectionCandidateKind,
        history_query: TaskReflectionHistoryQuery,
        history_digest: &lattice_contracts::ContentDigest,
        candidate_digest: &lattice_contracts::ContentDigest,
    ) -> TaskReflectionResult<TaskReflectionEvidence>;
}

/// Stable fail-closed categories shared across port and inbound-service boundaries.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PortErrorKind {
    Unavailable,
    VersionMismatch,
    CapabilityMismatch,
    Malformed,
    Timeout,
    Cancelled,
    Ambiguous,
    Denied,
}

/// A typed port failure with a stable machine-facing code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortError {
    component: Component,
    kind: PortErrorKind,
    code: String,
}

impl PortError {
    /// Constructs a typed port failure.
    #[must_use]
    pub fn new(component: Component, kind: PortErrorKind, code: impl Into<String>) -> Self {
        Self {
            component,
            kind,
            code: code.into(),
        }
    }

    /// Returns the component that observed the failure.
    #[must_use]
    pub const fn component(&self) -> Component {
        self.component
    }

    /// Returns the fail-closed failure category.
    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    /// Returns the stable machine-facing failure code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?} {:?}: {}",
            self.component, self.kind, self.code
        )
    }
}

impl Error for PortError {}

/// Whether a failed delivery call is known not to have completed or has an
/// outcome that must be reconciled before retry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DeliveryFailureCertainty {
    Known,
    Ambiguous,
}

/// Typed delivery failure with exact stage and effect certainty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryPortError {
    stage: DeliveryStage,
    kind: PortErrorKind,
    certainty: DeliveryFailureCertainty,
    code: String,
}

impl DeliveryPortError {
    /// Constructs one stage-specific delivery failure.
    #[must_use]
    pub fn new(
        stage: DeliveryStage,
        kind: PortErrorKind,
        certainty: DeliveryFailureCertainty,
        code: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            kind,
            certainty,
            code: code.into(),
        }
    }

    #[must_use]
    pub const fn stage(&self) -> DeliveryStage {
        self.stage
    }

    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn certainty(&self) -> DeliveryFailureCertainty {
        self.certainty
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for DeliveryPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Delivery {:?} {:?}/{:?}: {}",
            self.stage, self.kind, self.certainty, self.code
        )
    }
}

impl Error for DeliveryPortError {}

/// Ordered effect stages for the executable graph-memory node.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GraphMemoryStage {
    /// Exact tracked-only Git snapshot materialization.
    Snapshot,
    /// Pinned Graphify child execution and strict output parsing.
    Graphify,
    /// Atomic analysis/record persistence.
    Persistence,
    /// Exact-snapshot deterministic retrieval and audit persistence.
    Retrieval,
    /// Restart-safe terminal receipt readback.
    Receipt,
    /// Atomic Hermes structured-reflection persistence.
    ReflectionPersistence,
    /// Restart-safe Hermes structured-reflection readback.
    ReflectionReceipt,
}

/// Whether a failed graph-memory effect is known not to have completed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GraphMemoryFailureCertainty {
    /// The effect is proved not to have completed successfully.
    Known,
    /// The effect outcome cannot safely be inferred and requires reconciliation.
    Ambiguous,
}

/// Typed graph-memory failure with exact stage and outcome certainty.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphMemoryPortError {
    stage: GraphMemoryStage,
    kind: PortErrorKind,
    certainty: GraphMemoryFailureCertainty,
    code: String,
}

impl GraphMemoryPortError {
    /// Constructs one stage-specific graph-memory failure.
    #[must_use]
    pub fn new(
        stage: GraphMemoryStage,
        kind: PortErrorKind,
        certainty: GraphMemoryFailureCertainty,
        code: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            kind,
            certainty,
            code: code.into(),
        }
    }

    #[must_use]
    pub const fn stage(&self) -> GraphMemoryStage {
        self.stage
    }

    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn certainty(&self) -> GraphMemoryFailureCertainty {
        self.certainty
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for GraphMemoryPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "GraphMemory {:?} {:?}/{:?}: {}",
            self.stage, self.kind, self.certainty, self.code
        )
    }
}

impl Error for GraphMemoryPortError {}

/// A typed Rust-core gateway-service failure with no external component label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GatewayServiceError {
    kind: PortErrorKind,
    code: String,
}

impl GatewayServiceError {
    /// Constructs a typed gateway-service failure.
    #[must_use]
    pub fn new(kind: PortErrorKind, code: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.into(),
        }
    }

    /// Returns the fail-closed failure category.
    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    /// Returns the stable machine-facing failure code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for GatewayServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "GatewayService {:?}: {}", self.kind, self.code)
    }
}

impl Error for GatewayServiceError {}

/// Exact logical-session command scope used by durable `OpenClaw` idempotency.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OpenClawCommandScope {
    project: ProjectId,
    actor: GatewayActorId,
    session_epoch: u64,
    command: GatewayCommandId,
}

impl OpenClawCommandScope {
    /// Constructs one server-derived scope. All identifiers are already typed;
    /// the positive logical session epoch is checked again at this boundary.
    ///
    /// # Errors
    ///
    /// Rejects the zero epoch sentinel.
    pub fn new(
        project: ProjectId,
        actor: GatewayActorId,
        session_epoch: u64,
        command: GatewayCommandId,
    ) -> Result<Self, OpenClawIdempotencyError> {
        if session_epoch == 0 {
            return Err(OpenClawIdempotencyError::Malformed);
        }
        Ok(Self {
            project,
            actor,
            session_epoch,
            command,
        })
    }

    /// Returns the command's exact project.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project
    }

    /// Returns the server-derived actor identity.
    #[must_use]
    pub const fn actor_id(&self) -> &GatewayActorId {
        &self.actor
    }

    /// Returns the verified logical `OpenClaw` session epoch.
    #[must_use]
    pub const fn session_epoch(&self) -> u64 {
        self.session_epoch
    }

    /// Returns the idempotent semantic command identity.
    #[must_use]
    pub const fn command_id(&self) -> &GatewayCommandId {
        &self.command
    }
}

/// One typed terminal command record suitable for durable receipt storage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenClawTerminalCommandRecord {
    scope: OpenClawCommandScope,
    request: GatewayRequest,
    reply: GatewayReply,
}

impl OpenClawTerminalCommandRecord {
    /// Constructs one exact terminal record after checking request, reply, and
    /// logical command-scope binding.
    ///
    /// # Errors
    ///
    /// Rejects command, project, request-digest, or reply binding drift.
    pub fn new(
        scope: OpenClawCommandScope,
        request: GatewayRequest,
        reply: GatewayReply,
    ) -> Result<Self, OpenClawIdempotencyError> {
        if request.project_id() != scope.project_id()
            || request.command_id() != scope.command_id()
            || reply.command_id() != request.command_id()
            || reply.correlation_id() != request.correlation_id()
            || reply.action() != request.action()
            || reply.request_digest() != request.request_digest()
        {
            return Err(OpenClawIdempotencyError::Malformed);
        }
        Ok(Self {
            scope,
            request,
            reply,
        })
    }

    /// Returns the exact project/actor/session/command scope.
    #[must_use]
    pub const fn scope(&self) -> &OpenClawCommandScope {
        &self.scope
    }

    /// Returns the exact mechanically verified gateway request.
    #[must_use]
    pub const fn request(&self) -> &GatewayRequest {
        &self.request
    }

    /// Returns the exact reconstructed gateway request digest.
    #[must_use]
    pub const fn request_digest(&self) -> &lattice_contracts::ContentDigest {
        self.request.request_digest()
    }

    /// Returns the validated terminal gateway reply.
    #[must_use]
    pub const fn reply(&self) -> &GatewayReply {
        &self.reply
    }
}

/// Persistence claim exposed by a terminal-idempotency provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyDurability {
    /// Process-local checkpoint storage; never restart-safe.
    ProcessMemory,
    /// Durable terminal command receipts reconciled across process starts.
    DurableTerminalReceipts,
}

/// Typed result of reconciling one command before dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyDecision {
    /// The exact request now owns a bounded pre-dispatch claim.
    Claimed,
    /// The exact request is already claimed but has no terminal reply yet.
    InFlight,
    /// The exact request already has a terminal reply.
    Exact(Box<GatewayReply>),
    /// The command scope exists under a different request digest.
    CommandSubstitution,
}

/// Closed idempotency-provider failure without backend details.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenClawIdempotencyError {
    /// The provider could not be reached.
    Unavailable,
    /// The provider's bounded capacity was exhausted.
    Capacity,
    /// The provider returned a malformed or contradictory record.
    Malformed,
}

/// Typed terminal-command idempotency port implemented by process-memory test
/// checkpoints or the `PostgreSQL` truth owner.
pub trait OpenClawIdempotencyStore: Send {
    /// States whether records survive a process restart.
    fn durability(&self) -> OpenClawIdempotencyDurability;

    /// Atomically reconciles or claims one command before dispatch.
    ///
    /// # Errors
    ///
    /// Returns a closed provider failure without backend detail.
    fn reconcile_and_claim(
        &mut self,
        scope: &OpenClawCommandScope,
        request: &GatewayRequest,
    ) -> Result<OpenClawIdempotencyDecision, OpenClawIdempotencyError>;

    /// Finalizes one existing claim with a validated terminal reply.
    ///
    /// # Errors
    ///
    /// Returns a closed provider failure without backend detail.
    fn finalize_terminal(
        &mut self,
        record: OpenClawTerminalCommandRecord,
    ) -> Result<(), OpenClawIdempotencyError>;
}

/// Stable fail-closed Store failure categories.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ControlStoreErrorKind {
    Malformed,
    UnsupportedVersion,
    CommandSubstitution,
    AuthorityMismatch,
    AdmissionDenied,
    RevisionOverflow,
    CapacityExceeded,
    Unavailable,
    SerializationExhausted,
    CommitOutcomeUnknown,
    CorruptState,
}

impl ControlStoreErrorKind {
    /// Complete closed Store error set; no variant represents success.
    pub const ALL: [Self; 11] = [
        Self::Malformed,
        Self::UnsupportedVersion,
        Self::CommandSubstitution,
        Self::AuthorityMismatch,
        Self::AdmissionDenied,
        Self::RevisionOverflow,
        Self::CapacityExceeded,
        Self::Unavailable,
        Self::SerializationExhausted,
        Self::CommitOutcomeUnknown,
        Self::CorruptState,
    ];
}

/// Typed Store failure with one stable bounded machine-facing code.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlStoreError {
    kind: ControlStoreErrorKind,
    code: &'static str,
}

impl ControlStoreError {
    /// Constructs one Store-specific failure.
    #[must_use]
    pub const fn new(kind: ControlStoreErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    /// Returns the fail-closed category.
    #[must_use]
    pub const fn kind(&self) -> ControlStoreErrorKind {
        self.kind
    }

    /// Returns the stable machine-facing code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ControlStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ControlStore {:?}: {}", self.kind, self.code)
    }
}

impl Error for ControlStoreError {}

/// Inbound service implemented by the Rust core for the normal `OpenClaw` gateway.
pub trait GatewayService {
    /// Handles one mechanically verified request under server-derived peer context.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when the request cannot be safely accepted or
    /// observed.
    fn handle(
        &mut self,
        peer: GatewayPeerContext,
        request: GatewayRequest,
    ) -> GatewayServiceResult<GatewayReply>;
}

/// Typed physical control-store boundary implemented by fake or live adapters.
pub trait ControlStore {
    /// Applies one complete domain-committed physical transaction.
    ///
    /// # Errors
    ///
    /// Returns a Store-specific failure for malformed/substituted authority,
    /// capacity, transport, serialization, corruption, or unknown outcomes.
    /// A terminal receipt may classify proven physical durability, but it
    /// never defines domain legality, freshness, or effect delivery.
    fn transact(
        &mut self,
        request: StoreTransactionRequest,
    ) -> ControlStoreResult<StoreTransactionReceipt>;

    /// Returns the independently retained physical head for one exact scope.
    ///
    /// # Errors
    ///
    /// Returns a Store-specific failure when the head cannot be safely
    /// observed. The result is not a domain-owner current head.
    fn current_head(&mut self, scope: &StoreScope) -> ControlStoreResult<StorePhysicalHead>;
}

/// Durable delivery-ledger boundary. Implementations own persistence and
/// canonical receipt reconstruction; callers never receive a database client.
pub trait DeliveryLedgerPort {
    /// Commits intent before any workspace or provider effect.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when durability cannot be proved.
    fn record_intent(
        &mut self,
        request: &DeliveryRunRequest,
    ) -> DeliveryPortResult<DurableIntentEvidence>;

    /// Records one completed, failed, or reconciliation-required outcome.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when terminal persistence is rejected or
    /// unknown.
    fn record_outcome(
        &mut self,
        request: &DeliveryOutcomeRequest,
    ) -> DeliveryPortResult<DeliveryOutcomeEvidence>;

    /// Reconstructs the terminal receipt from durable state.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing, incomplete, corrupt, or ambiguous
    /// durable state.
    fn load_receipt(
        &mut self,
        request: &DeliveryStatusRequest,
    ) -> DeliveryPortResult<DeliveryReceipt>;
}

/// Sole production Codex writer boundary for typed delivery. The legacy
/// generic [`CodexPort`] remains source compatible for earlier consumers but
/// is frozen outside the production delivery composition.
pub trait DeliveryCodexPort {
    /// Runs one request bound to durable intent and a prepared workspace.
    ///
    /// # Errors
    ///
    /// Returns a typed known or ambiguous Codex-stage failure.
    fn run_delivery(
        &mut self,
        request: CodexDeliveryRequest,
    ) -> DeliveryPortResult<CodexDeliveryEvidence>;

    /// Interrupts the delivery associated with one request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when interruption or final outcome is unknown.
    fn interrupt_delivery(&mut self, request_id: &RequestId) -> DeliveryPortResult<()>;
}

/// Bounded workspace and Git lane. It exposes no command text or caller path.
pub trait WorkspaceGitPort {
    /// Prepares the fixed delivery workspace after durable intent.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for unsafe or ambiguous preparation.
    fn prepare(
        &mut self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
    ) -> DeliveryPortResult<PreparedWorkspaceEvidence>;

    /// Inspects the fixed changed-path scope after Codex completes.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when scope does not match or cannot be proved.
    fn inspect_changes(
        &mut self,
        request: &DeliveryRunRequest,
        intent: &DurableIntentEvidence,
        workspace: &PreparedWorkspaceEvidence,
        codex: &CodexDeliveryEvidence,
    ) -> DeliveryPortResult<WorkspaceChangeEvidence>;

    /// Creates one local commit after passing scope and fixed-test evidence.
    ///
    /// # Errors
    ///
    /// Returns a typed known or ambiguous Git-stage failure.
    fn commit(
        &mut self,
        request: &DeliveryRunRequest,
        workspace: &PreparedWorkspaceEvidence,
        changes: &WorkspaceChangeEvidence,
        test: &FixedTestEvidence,
    ) -> DeliveryPortResult<GitCommitEvidence>;
}

/// Sole fixed verification profile used by the bounded delivery node.
pub trait TestRunnerPort {
    /// Runs the profile-selected fixed test; no command text is accepted.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for a failed or unobservable test.
    fn run_fixed(
        &mut self,
        request: &DeliveryRunRequest,
        workspace: &PreparedWorkspaceEvidence,
        changes: &WorkspaceChangeEvidence,
    ) -> DeliveryPortResult<FixedTestEvidence>;
}

/// Frozen generic product-code writer boundary retained for pre-delivery
/// consumers. Production delivery uses [`DeliveryCodexPort`] and must not wire
/// both interfaces as separate runtime writers.
pub trait CodexPort {
    /// Runs one approved implementation request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when capability, version, permission, runtime,
    /// completion, or outcome evidence is unsafe or unknown.
    fn run(&mut self, request: CodexRunRequest) -> PortResult<CodexEvidence>;

    /// Interrupts the run associated with a request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when interruption or final outcome is unknown.
    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()>;
}

/// Exact tracked-only snapshot boundary for the production graph-memory node.
///
/// The implementation owns its process-configured repository and staging
/// location; callers can select neither a path nor a Git command.
pub trait CodeSnapshotPort {
    /// Materializes one immutable tracked snapshot for the exact request commit.
    ///
    /// # Errors
    ///
    /// Returns a typed known or ambiguous failure for missing/substituted Git
    /// objects, unsafe tracked paths, secret/untracked leakage, or teardown.
    fn materialize_snapshot(
        &mut self,
        request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<CodeSnapshotEvidence>;
}

/// Pinned Graphify analysis boundary for production graph-memory work.
///
/// This interface is separate from the frozen generic [`GraphifyPort`] and
/// accepts no command, environment, path, credential, or backend selection.
pub trait GraphifyAnalysisPort {
    /// Runs the fixed headless code-only analysis over one exact snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed known or ambiguous failure for identity/capability
    /// mismatch, timeout, malformed/partial output, or unsafe provenance.
    fn analyze(
        &mut self,
        request: &GraphMemoryRunRequest,
        snapshot: &CodeSnapshotEvidence,
    ) -> GraphMemoryPortResult<GraphifyRawEvidence>;
}

/// Sole Codebase Memory repository boundary.
///
/// Implementations persist only typed analysis/plan values through fixed
/// repository operations and never expose SQL, credentials, or a database
/// client. Effect ordering remains owned by the orchestrator.
pub trait CodebaseMemoryPort {
    /// Atomically writes one complete normalized analysis and candidate set.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when exact persistence is rejected or unknown.
    fn persist_analysis(
        &mut self,
        analysis: &NormalizedGraphAnalysis,
    ) -> GraphMemoryPortResult<GraphMemoryPersistenceEvidence>;

    /// Executes and audits one precomputed deterministic retrieval plan.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing/substituted analysis, invalidation,
    /// corrupt ordering, unavailable storage, or ambiguous commit outcome.
    fn retrieve(
        &mut self,
        persistence: &GraphMemoryPersistenceEvidence,
        plan: MemoryRetrievalPlan,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt>;

    /// Replays one exact terminal receipt without invoking earlier effects.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing, incomplete, cross-bound, corrupt,
    /// or unavailable repository state. This receipt alone is not live
    /// database-identity or restart-replay proof.
    fn load_receipt(
        &mut self,
        request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<GraphMemoryReceipt>;
}

/// Sole durable repository boundary for structured Hermes reflection content.
///
/// The implementation is owned by the same `PostgreSQL` Codebase Memory adapter
/// as graph receipts. Hermes itself never receives this port or a database
/// client, and fresh status uses only [`Self::load_reflection`].
pub trait HermesReflectionMemoryPort {
    /// Atomically persists one exact-graph-bound inference candidate.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing/substituted graph truth, invalid
    /// structured content, unavailable storage, or ambiguous commit outcome.
    fn persist_reflection(
        &mut self,
        reflection: &HermesReflectionCandidate,
    ) -> GraphMemoryPortResult<HermesReflectionReceipt>;

    /// Loads one exact structured reflection without invoking Hermes.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for missing, incomplete, cross-bound, corrupt,
    /// or unavailable durable state.
    fn load_reflection(
        &mut self,
        request: &GraphMemoryRunRequest,
    ) -> GraphMemoryPortResult<HermesReflectionReceipt>;
}

/// Read-only derived-knowledge boundary implemented by the `Graphify` adapter.
pub trait GraphifyPort {
    /// Builds a code graph for an immutable project snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for unsafe source/output boundaries, unavailable
    /// capabilities, malformed output, timeout, or ambiguous completion.
    fn build_code_graph(&mut self, request: GraphifyBuildRequest) -> PortResult<GraphifyEvidence>;
}

/// Untrusted research-candidate boundary implemented by the `Hermes` adapter.
pub trait HermesPort {
    /// Runs one bounded research request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for unavailable capabilities, malformed or
    /// provenance-free output, timeout, cancellation, or ambiguity.
    fn research(&mut self, request: HermesResearchRequest) -> PortResult<HermesEvidence>;

    /// Interrupts the run associated with a request.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when interruption or final outcome is unknown.
    fn interrupt(&mut self, request_id: &RequestId) -> PortResult<()>;
}

#[cfg(test)]
mod reflection_contract_tests {
    use super::{MAX_TASK_REFLECTION_HISTORY_EVENTS, TaskReflectionHistoryQuery};

    #[test]
    fn history_query_is_bounded_and_cursor_exact() {
        let latest = TaskReflectionHistoryQuery::latest(1).expect("latest page");
        assert_eq!(latest.before_sequence(), None);
        assert_eq!(latest.limit(), 1);

        let older = TaskReflectionHistoryQuery::new(Some(9), 7).expect("older page");
        assert_eq!(older.before_sequence(), Some(9));
        assert_eq!(older.limit(), 7);

        for rejected in [
            TaskReflectionHistoryQuery::latest(0),
            TaskReflectionHistoryQuery::latest(MAX_TASK_REFLECTION_HISTORY_EVENTS + 1),
        ] {
            assert_eq!(
                rejected.expect_err("invalid page").code(),
                "LATTICE_REFLECTION_HISTORY_LIMIT_REJECTED"
            );
        }
        assert_eq!(
            TaskReflectionHistoryQuery::new(Some(0), 1)
                .expect_err("zero cursor")
                .code(),
            "LATTICE_REFLECTION_HISTORY_CURSOR_REJECTED"
        );
    }
}
