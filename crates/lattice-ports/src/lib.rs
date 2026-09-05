//! Abstract I/O ports for LATTICE orchestration.

mod managed_foreman;

pub use lattice_artifact_store::{
    ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence,
};
pub use lattice_task_ledger::{
    VerificationOutcome, VerifiedTaskExecutionBinding, VerifiedTaskVerificationRecord,
    VerifiedWorkerAttemptRecord, VerifiedWorkerObservationRecord, WorkerObservationInput,
    WorkerObservationKind,
};
pub use managed_foreman::*;

use std::error::Error;
use std::fmt;

use lattice_contracts::{
    CodeSnapshotEvidence, CodexDeliveryEvidence, CodexDeliveryRequest, CodexEvidence,
    CodexRunRequest, Component, ContentDigest, DeliveryOutcomeEvidence, DeliveryOutcomeRequest,
    DeliveryReceipt, DeliveryRunRequest, DeliveryStage, DeliveryStatusRequest,
    DurableIntentEvidence, FixedTestEvidence, GatewayActorId, GatewayCommandId, GatewayPeerContext,
    GatewayReply, GatewayRequest, GitCommitEvidence, GraphMemoryPersistenceEvidence,
    GraphMemoryReceipt, GraphMemoryRunRequest, GraphifyBuildRequest, GraphifyEvidence,
    GraphifyRawEvidence, HermesEvidence, HermesReflectionCandidate, HermesReflectionReceipt,
    HermesResearchRequest, MemoryRetrievalPlan, NormalizedGraphAnalysis, PreparedWorkspaceEvidence,
    ProjectId, RequestId, StorePhysicalHead, StoreScope, StoreTransactionReceipt,
    StoreTransactionRequest, WorkspaceChangeEvidence, WriterLeaseAuthorityHead,
};
use lattice_foreman_state::{DependencyContinuation, ForemanCheckpointIntent, ForemanSnapshot};
use lattice_task_domain::TaskState;

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

/// Result returned by the typed foreman coordination persistence boundary.
pub type ForemanCoordinationResult<T> = Result<T, ForemanCoordinationError>;

/// Closed failure classes for foreman append/replay persistence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForemanCoordinationErrorKind {
    Malformed,
    Unauthorized,
    StaleWriter,
    Conflict,
    Corrupt,
    Unavailable,
    OutcomeUnknown,
}

/// Bounded component-free foreman persistence error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanCoordinationError {
    kind: ForemanCoordinationErrorKind,
    code: &'static str,
}

impl ForemanCoordinationError {
    #[must_use]
    pub const fn new(kind: ForemanCoordinationErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> ForemanCoordinationErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ForemanCoordinationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Foreman coordination {:?}: {}",
            self.kind, self.code
        )
    }
}

impl Error for ForemanCoordinationError {}

/// Durable evidence for one new append or exact retry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanAppendReceipt {
    event_digest: ContentDigest,
    ledger_digest: ContentDigest,
    checkpoint_digest: ContentDigest,
    generation: u64,
    exact_retry: bool,
}

/// Exact durable append replay plus the Writer-owned authority receipt digest
/// needed only to reconcile a possibly unknown release.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanCheckpointReplay {
    receipt: ForemanAppendReceipt,
    authority_receipt_digest: ContentDigest,
}

/// Replay-verified durable Runtime projection for the sole foreman.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanRuntimeStatus {
    ledger_digest: ContentDigest,
    checkpoint_digest: ContentDigest,
    latest_generation: u64,
    active_count: usize,
    blocked_count: usize,
    completed_count: usize,
    next_action: &'static str,
    dependency: Option<DependencyContinuation>,
}

impl ForemanRuntimeStatus {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        ledger_digest: ContentDigest,
        checkpoint_digest: ContentDigest,
        latest_generation: u64,
        active_count: usize,
        blocked_count: usize,
        completed_count: usize,
        next_action: &'static str,
        dependency: Option<DependencyContinuation>,
    ) -> Self {
        Self {
            ledger_digest,
            checkpoint_digest,
            latest_generation,
            active_count,
            blocked_count,
            completed_count,
            next_action,
            dependency,
        }
    }

    #[must_use]
    pub const fn ledger_digest(&self) -> &ContentDigest {
        &self.ledger_digest
    }
    #[must_use]
    pub const fn checkpoint_digest(&self) -> &ContentDigest {
        &self.checkpoint_digest
    }
    #[must_use]
    pub const fn latest_generation(&self) -> u64 {
        self.latest_generation
    }
    #[must_use]
    pub const fn active_count(&self) -> usize {
        self.active_count
    }
    #[must_use]
    pub const fn blocked_count(&self) -> usize {
        self.blocked_count
    }
    #[must_use]
    pub const fn completed_count(&self) -> usize {
        self.completed_count
    }
    #[must_use]
    pub const fn dependency(&self) -> Option<&DependencyContinuation> {
        self.dependency.as_ref()
    }
    #[must_use]
    pub const fn next_action(&self) -> &'static str {
        self.next_action
    }
}

impl ForemanCheckpointReplay {
    #[must_use]
    pub const fn new(
        receipt: ForemanAppendReceipt,
        authority_receipt_digest: ContentDigest,
    ) -> Self {
        Self {
            receipt,
            authority_receipt_digest,
        }
    }

    #[must_use]
    pub const fn receipt(&self) -> &ForemanAppendReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn authority_receipt_digest(&self) -> &ContentDigest {
        &self.authority_receipt_digest
    }

    #[must_use]
    pub fn into_receipt(self) -> ForemanAppendReceipt {
        self.receipt
    }
}

impl ForemanAppendReceipt {
    /// # Errors
    ///
    /// Rejects zero generation.
    pub fn new(
        event_digest: ContentDigest,
        ledger_digest: ContentDigest,
        checkpoint_digest: ContentDigest,
        generation: u64,
        exact_retry: bool,
    ) -> ForemanCoordinationResult<Self> {
        if generation == 0 {
            return Err(ForemanCoordinationError::new(
                ForemanCoordinationErrorKind::Malformed,
                "FOREMAN_RECEIPT_GENERATION_INVALID",
            ));
        }
        Ok(Self {
            event_digest,
            ledger_digest,
            checkpoint_digest,
            generation,
            exact_retry,
        })
    }

    #[must_use]
    pub const fn event_digest(&self) -> &ContentDigest {
        &self.event_digest
    }

    /// The authoritative resulting Task Ledger stream-head digest exposed on
    /// the checkpoint wire and reproduced by Runtime Status after restart.
    #[must_use]
    pub const fn ledger_digest(&self) -> &ContentDigest {
        &self.ledger_digest
    }

    #[must_use]
    pub const fn checkpoint_digest(&self) -> &ContentDigest {
        &self.checkpoint_digest
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.exact_retry
    }
}

/// Narrow append/replay boundary for the sole durable Task Ledger truth.
pub trait ForemanCoordinationPort {
    /// Replays an exact caller intent before any new server observation or
    /// Writer effect. A changed payload under a retained ID is a conflict.
    ///
    /// # Errors
    ///
    /// Corrupt, unsupported, unavailable or changed replay fails closed.
    fn replay_checkpoint(
        &mut self,
        intent: &ForemanCheckpointIntent,
    ) -> ForemanCoordinationResult<Option<ForemanCheckpointReplay>>;

    /// Appends one already validated snapshot under exact Writer authority.
    ///
    /// # Errors
    ///
    /// Returns a closed failure for malformed metadata, stale/fake authority,
    /// conflict, corruption, unavailability, or unknown commit outcome.
    fn append_snapshot(
        &mut self,
        command_id: &str,
        correlation_id: &str,
        occurred_at: &str,
        snapshot: ForemanSnapshot,
        writer: &WriterLeaseAuthorityHead,
    ) -> ForemanCoordinationResult<ForemanAppendReceipt>;

    /// Loads only snapshots verified against the authoritative Ledger replay.
    ///
    /// # Errors
    ///
    /// Missing, partial, unknown-version, or corrupt persistence fails closed.
    fn load_snapshots(&mut self) -> ForemanCoordinationResult<Vec<ForemanSnapshot>>;

    /// Loads the complete verified Runtime projection and its Ledger digests.
    ///
    /// # Errors
    ///
    /// Corrupt, unsupported or unavailable replay fails closed.
    fn load_runtime_status(&mut self) -> ForemanCoordinationResult<ForemanRuntimeStatus>;
}

/// Result returned by the authoritative Task lifecycle repository boundary.
pub type TaskLifecycleResult<T> = Result<T, TaskLifecycleError>;

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

/// Replay-verified projection of one pre-specification general-task intake.
///
/// Ordinary intake remains structurally `DRAFT`. The only terminal carve-out
/// is a digest-bound `COMPLETED` projection from a dedicated verified result
/// adoption path; it still carries no Task Spec, currency, autonomy, approval,
/// Writer Lease, or execution field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskIntakeLifecycleEvidence {
    binding: lattice_contracts::TaskIntakeBinding,
    ledger_head_digest: lattice_contracts::ContentDigest,
    state: TaskState,
    result_digest: Option<lattice_contracts::ContentDigest>,
}

impl TaskIntakeLifecycleEvidence {
    /// Constructs the only successful intake lifecycle projection.
    ///
    /// # Errors
    ///
    /// Rejects the all-zero Ledger-head commitment sentinel.
    pub fn new(
        binding: lattice_contracts::TaskIntakeBinding,
        ledger_head_digest: lattice_contracts::ContentDigest,
    ) -> TaskLifecycleResult<Self> {
        if ledger_head_digest.as_str().bytes().all(|byte| byte == b'0') {
            return Err(TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "LATTICE_TASK_INTAKE_LEDGER_HEAD_REJECTED",
            ));
        }
        Ok(Self {
            binding,
            ledger_head_digest,
            state: TaskState::Draft,
            result_digest: None,
        })
    }

    /// Constructs the sole non-executable terminal intake projection.
    ///
    /// This constructor grants no mutation authority. Its caller must have
    /// independently verified the immutable external-result adoption receipt
    /// and committed its typed terminal Ledger event.
    ///
    /// # Errors
    ///
    /// Rejects all-zero head or result commitments.
    pub fn externally_adopted(
        binding: lattice_contracts::TaskIntakeBinding,
        ledger_head_digest: lattice_contracts::ContentDigest,
        result_digest: lattice_contracts::ContentDigest,
    ) -> TaskLifecycleResult<Self> {
        Self::verified_result(binding, ledger_head_digest, result_digest)
    }

    /// A replay-verified local or external descriptor was committed by its typed Ledger path.
    /// Builds a completed intake result from verified durable evidence.
    ///
    /// # Errors
    /// Rejects a zero ledger head or result digest.
    pub fn verified_result(
        binding: lattice_contracts::TaskIntakeBinding,
        ledger_head_digest: lattice_contracts::ContentDigest,
        result_digest: lattice_contracts::ContentDigest,
    ) -> TaskLifecycleResult<Self> {
        if ledger_head_digest.as_str().bytes().all(|byte| byte == b'0')
            || result_digest.as_str().bytes().all(|byte| byte == b'0')
        {
            return Err(TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "LATTICE_TASK_INTAKE_EXTERNAL_RESULT_REJECTED",
            ));
        }
        Ok(Self {
            binding,
            ledger_head_digest,
            state: TaskState::Completed,
            result_digest: Some(result_digest),
        })
    }

    /// Returns the exact non-executable intake binding.
    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::TaskIntakeBinding {
        &self.binding
    }

    /// Returns the replay-verified intake state.
    #[must_use]
    pub const fn state(&self) -> TaskState {
        self.state
    }

    /// Returns the digest-bound external terminal result, if adopted.
    #[must_use]
    pub fn result_digest(&self) -> Option<&lattice_contracts::ContentDigest> {
        self.result_digest.as_ref()
    }

    /// Returns the verified current Task Ledger head commitment.
    #[must_use]
    pub const fn ledger_head_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.ledger_head_digest
    }
}

/// Closed result of one idempotent general-task intake admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskIntakeAdmission {
    evidence: TaskIntakeLifecycleEvidence,
    exact_replay: bool,
}

impl TaskIntakeAdmission {
    /// Wraps evidence for a newly durably created intake.
    #[must_use]
    pub const fn created(evidence: TaskIntakeLifecycleEvidence) -> Self {
        Self {
            evidence,
            exact_replay: false,
        }
    }

    /// Wraps evidence for an exact idempotent replay.
    #[must_use]
    pub const fn exact_replay(evidence: TaskIntakeLifecycleEvidence) -> Self {
        Self {
            evidence,
            exact_replay: true,
        }
    }

    /// Returns the complete verified intake evidence.
    #[must_use]
    pub const fn evidence(&self) -> &TaskIntakeLifecycleEvidence {
        &self.evidence
    }

    /// Returns whether no new durable intake was created.
    #[must_use]
    pub const fn is_exact_replay(&self) -> bool {
        self.exact_replay
    }

    /// Returns the exact non-executable intake binding.
    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::TaskIntakeBinding {
        self.evidence.binding()
    }

    /// Returns the replay-verified intake state.
    #[must_use]
    pub const fn state(&self) -> TaskState {
        self.evidence.state()
    }

    /// Returns the digest-bound external terminal result, if adopted.
    #[must_use]
    pub fn result_digest(&self) -> Option<&lattice_contracts::ContentDigest> {
        self.evidence.result_digest()
    }

    /// Consumes the admission wrapper and returns its verified evidence.
    #[must_use]
    pub fn into_evidence(self) -> TaskIntakeLifecycleEvidence {
        self.evidence
    }
}

/// Narrow durable boundary for create/status-only general-task intake.
///
/// Unlike [`TaskLifecyclePort`], this trait cannot transition state, record a
/// result or autonomy classification, or receive Writer authority.
pub trait TaskIntakeLifecyclePort {
    /// Idempotently admits one exact intake binding and client retry key.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn admit(
        &mut self,
        binding: &lattice_contracts::TaskIntakeBinding,
        client_request_id: &str,
    ) -> TaskLifecycleResult<TaskIntakeAdmission>;

    /// Replays one exact authoritative intake projection without mutation.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn load(
        &mut self,
        binding: &lattice_contracts::TaskIntakeBinding,
    ) -> TaskLifecycleResult<TaskIntakeLifecycleEvidence>;
}

/// Replay-derived authoritative Task lifecycle projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyDisposition {
    Proceed,
    AskUser,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyReason {
    RoutineAuthorized,
    NewUserDecision,
    NewAuthority,
    HighRiskOrIrreversible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyModel {
    GovernedCodexWriter,
    NoModel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyVerification {
    FocusedChecks,
    BuildAndFocusedChecks,
    ReadOnlyEvidence,
}

/// Internal-only replay projection of one verified autonomy receipt event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutonomyReceiptProjection {
    receipt_digest: lattice_contracts::ContentDigest,
    authority_digest: lattice_contracts::ContentDigest,
    event_digest: lattice_contracts::ContentDigest,
    observed_state: TaskState,
    disposition: AutonomyDisposition,
    reason: AutonomyReason,
    model: Option<AutonomyModel>,
    verification: Option<AutonomyVerification>,
}

impl AutonomyReceiptProjection {
    /// Constructs one closed projection from Task-Ledger-verified values.
    ///
    /// # Errors
    ///
    /// Rejects non-Draft observations and impossible decision shapes.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        receipt_digest: lattice_contracts::ContentDigest,
        authority_digest: lattice_contracts::ContentDigest,
        event_digest: lattice_contracts::ContentDigest,
        observed_state: TaskState,
        disposition: AutonomyDisposition,
        reason: AutonomyReason,
        model: Option<AutonomyModel>,
        verification: Option<AutonomyVerification>,
    ) -> TaskLifecycleResult<Self> {
        let decision_shape_valid = matches!(
            (disposition, reason, model, verification),
            (
                AutonomyDisposition::Proceed,
                AutonomyReason::RoutineAuthorized,
                Some(_),
                Some(_)
            ) | (
                AutonomyDisposition::AskUser,
                AutonomyReason::NewUserDecision
                    | AutonomyReason::NewAuthority
                    | AutonomyReason::HighRiskOrIrreversible,
                None,
                None
            )
        );
        if observed_state != TaskState::Draft || !decision_shape_valid {
            return Err(TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "TASK_LIFECYCLE_AUTONOMY_PROJECTION_INVALID",
            ));
        }
        Ok(Self {
            receipt_digest,
            authority_digest,
            event_digest,
            observed_state,
            disposition,
            reason,
            model,
            verification,
        })
    }

    #[must_use]
    pub const fn receipt_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.receipt_digest
    }
    #[must_use]
    pub const fn authority_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.authority_digest
    }
    #[must_use]
    pub const fn event_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.event_digest
    }
    #[must_use]
    pub const fn observed_state(&self) -> TaskState {
        self.observed_state
    }
    #[must_use]
    pub const fn disposition(&self) -> AutonomyDisposition {
        self.disposition
    }
    #[must_use]
    pub const fn reason(&self) -> AutonomyReason {
        self.reason
    }
    #[must_use]
    pub const fn model(&self) -> Option<AutonomyModel> {
        self.model
    }
    #[must_use]
    pub const fn verification(&self) -> Option<AutonomyVerification> {
        self.verification
    }
}

/// Closed lifecycle admission/receipt relationship exposed through Ports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskLifecycleAutonomyEvidence {
    Unadmitted,
    HistoricalOptional(Option<AutonomyReceiptProjection>),
    RequiredComplete(AutonomyReceiptProjection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TaskLifecycleAdmissionState {
    PendingRequiredReceipt {
        binding: lattice_contracts::SubjectBinding,
        ledger_head_digest: lattice_contracts::ContentDigest,
    },
    Existing(TaskLifecycleEvidence),
}

/// Closed result of idempotent task admission. A fresh required profile may
/// exist only as reconciliation state until its sequence-2 receipt is durable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLifecycleAdmission {
    state: TaskLifecycleAdmissionState,
}

impl TaskLifecycleAdmission {
    #[must_use]
    pub const fn pending_required_receipt(
        binding: lattice_contracts::SubjectBinding,
        ledger_head_digest: lattice_contracts::ContentDigest,
    ) -> Self {
        Self {
            state: TaskLifecycleAdmissionState::PendingRequiredReceipt {
                binding,
                ledger_head_digest,
            },
        }
    }

    /// Wraps replay-derived evidence only after it represents an admitted task.
    ///
    /// # Errors
    ///
    /// Rejects unadmitted/not-applicable lifecycle evidence.
    pub fn existing(evidence: TaskLifecycleEvidence) -> TaskLifecycleResult<Self> {
        if !evidence.admitted() {
            return Err(TaskLifecycleError::new(
                TaskLifecycleErrorKind::Corrupt,
                "LATTICE_TASK_ADMISSION_STATE_REJECTED",
            ));
        }
        Ok(Self {
            state: TaskLifecycleAdmissionState::Existing(evidence),
        })
    }

    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::SubjectBinding {
        match &self.state {
            TaskLifecycleAdmissionState::PendingRequiredReceipt { binding, .. } => binding,
            TaskLifecycleAdmissionState::Existing(evidence) => evidence.binding(),
        }
    }

    #[must_use]
    pub const fn ledger_head_digest(&self) -> &lattice_contracts::ContentDigest {
        match &self.state {
            TaskLifecycleAdmissionState::PendingRequiredReceipt {
                ledger_head_digest, ..
            } => ledger_head_digest,
            TaskLifecycleAdmissionState::Existing(evidence) => evidence.ledger_head_digest(),
        }
    }

    #[must_use]
    pub const fn existing_evidence(&self) -> Option<&TaskLifecycleEvidence> {
        match &self.state {
            TaskLifecycleAdmissionState::Existing(evidence) => Some(evidence),
            TaskLifecycleAdmissionState::PendingRequiredReceipt { .. } => None,
        }
    }

    #[must_use]
    pub fn into_existing(self) -> Option<TaskLifecycleEvidence> {
        match self.state {
            TaskLifecycleAdmissionState::Existing(evidence) => Some(evidence),
            TaskLifecycleAdmissionState::PendingRequiredReceipt { .. } => None,
        }
    }
}

/// Replay-derived authoritative Task lifecycle projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskLifecycleEvidence {
    binding: lattice_contracts::SubjectBinding,
    autonomy_evidence: TaskLifecycleAutonomyEvidence,
    state: TaskState,
    ledger_head_digest: lattice_contracts::ContentDigest,
    result_digest: Option<lattice_contracts::ContentDigest>,
}

impl TaskLifecycleEvidence {
    #[must_use]
    pub const fn new(
        binding: lattice_contracts::SubjectBinding,
        autonomy_evidence: TaskLifecycleAutonomyEvidence,
        state: TaskState,
        ledger_head_digest: lattice_contracts::ContentDigest,
        result_digest: Option<lattice_contracts::ContentDigest>,
    ) -> Self {
        Self {
            binding,
            autonomy_evidence,
            state,
            ledger_head_digest,
            result_digest,
        }
    }

    #[must_use]
    pub const fn binding(&self) -> &lattice_contracts::SubjectBinding {
        &self.binding
    }

    #[must_use]
    pub const fn admitted(&self) -> bool {
        !matches!(
            self.autonomy_evidence,
            TaskLifecycleAutonomyEvidence::Unadmitted
        )
    }

    #[must_use]
    pub const fn state(&self) -> TaskState {
        self.state
    }

    #[must_use]
    pub const fn ledger_head_digest(&self) -> &lattice_contracts::ContentDigest {
        &self.ledger_head_digest
    }

    #[must_use]
    pub const fn result_digest(&self) -> Option<&lattice_contracts::ContentDigest> {
        self.result_digest.as_ref()
    }

    #[must_use]
    pub const fn autonomy_receipt(&self) -> Option<&AutonomyReceiptProjection> {
        match &self.autonomy_evidence {
            TaskLifecycleAutonomyEvidence::HistoricalOptional(receipt) => receipt.as_ref(),
            TaskLifecycleAutonomyEvidence::RequiredComplete(receipt) => Some(receipt),
            TaskLifecycleAutonomyEvidence::Unadmitted => None,
        }
    }

    #[must_use]
    pub const fn autonomy_evidence(&self) -> &TaskLifecycleAutonomyEvidence {
        &self.autonomy_evidence
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
    ) -> TaskLifecycleResult<TaskLifecycleAdmission>;

    /// Records the exactly-once autonomy receipt before the first writable task
    /// effect. `PROCEED` requires the supplied current writer authority;
    /// `ASK_USER` requires `None` and imports no ambient writer authority.
    ///
    /// # Errors
    ///
    /// Returns a typed rejection, availability, ambiguity, or corruption error.
    fn record_autonomy_receipt(
        &mut self,
        binding: &lattice_contracts::SubjectBinding,
        writer_authority: Option<&lattice_contracts::WriterLeaseAuthorityHead>,
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
mod tests {
    use super::*;
    use lattice_contracts::{
        ContentDigest, ProjectId, ProjectSnapshotId, SubjectBinding, TaskId, TaskIntakeBinding,
    };

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
    }

    #[test]
    fn task_lifecycle_autonomy_evidence_is_closed() {
        let binding = SubjectBinding::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
            TaskId::new("TASK-050").expect("task"),
            "1",
            digest('a'),
        )
        .expect("binding");
        let receipt = AutonomyReceiptProjection::new(
            digest('b'),
            digest('c'),
            digest('d'),
            TaskState::Draft,
            AutonomyDisposition::Proceed,
            AutonomyReason::RoutineAuthorized,
            Some(AutonomyModel::GovernedCodexWriter),
            Some(AutonomyVerification::FocusedChecks),
        )
        .expect("closed receipt projection");
        let evidence = TaskLifecycleEvidence::new(
            binding,
            TaskLifecycleAutonomyEvidence::RequiredComplete(receipt.clone()),
            TaskState::Draft,
            digest('e'),
            None,
        );
        assert!(evidence.admitted());
        assert_eq!(evidence.autonomy_receipt(), Some(&receipt));
        assert_eq!(
            evidence.autonomy_evidence(),
            &TaskLifecycleAutonomyEvidence::RequiredComplete(receipt)
        );
        assert!(TaskLifecycleAdmission::existing(evidence).is_ok());
        let unadmitted = TaskLifecycleEvidence::new(
            SubjectBinding::new(
                ProjectId::new("project-1").expect("project"),
                ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
                TaskId::new("TASK-050").expect("task"),
                "1",
                digest('a'),
            )
            .expect("binding"),
            TaskLifecycleAutonomyEvidence::Unadmitted,
            TaskState::Draft,
            digest('e'),
            None,
        );
        assert_eq!(
            TaskLifecycleAdmission::existing(unadmitted)
                .expect_err("unadmitted evidence cannot become existing admission")
                .code(),
            "LATTICE_TASK_ADMISSION_STATE_REJECTED"
        );
        assert!(
            AutonomyReceiptProjection::new(
                digest('b'),
                digest('c'),
                digest('d'),
                TaskState::Draft,
                AutonomyDisposition::AskUser,
                AutonomyReason::NewUserDecision,
                Some(AutonomyModel::GovernedCodexWriter),
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn task_intake_evidence_is_draft_only_and_has_no_execution_projection() {
        let binding = TaskIntakeBinding::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
            TaskId::new("TASK-INTAKE-1").expect("task"),
            "1",
            digest('a'),
        )
        .expect("intake binding");
        let evidence = TaskIntakeLifecycleEvidence::new(binding.clone(), digest('b'))
            .expect("intake evidence");
        assert_eq!(evidence.binding(), &binding);
        assert_eq!(evidence.state(), TaskState::Draft);
        assert_eq!(evidence.result_digest(), None);
        assert_eq!(evidence.ledger_head_digest(), &digest('b'));

        let created = TaskIntakeAdmission::created(evidence.clone());
        assert!(!created.is_exact_replay());
        assert_eq!(created.evidence(), &evidence);
        let replay = TaskIntakeAdmission::exact_replay(evidence.clone());
        assert!(replay.is_exact_replay());
        assert_eq!(replay.into_evidence(), evidence);

        assert_eq!(
            TaskIntakeLifecycleEvidence::new(binding, digest('0'))
                .expect_err("zero Ledger head cannot become intake evidence")
                .code(),
            "LATTICE_TASK_INTAKE_LEDGER_HEAD_REJECTED"
        );
    }

    #[test]
    fn foreman_append_receipt_keeps_event_and_resulting_ledger_head_distinct() {
        let receipt = ForemanAppendReceipt::new(digest('a'), digest('b'), digest('c'), 1, false)
            .expect("receipt");
        assert_eq!(receipt.event_digest(), &digest('a'));
        assert_eq!(receipt.ledger_digest(), &digest('b'));
        assert_eq!(receipt.checkpoint_digest(), &digest('c'));
        assert!(!receipt.is_exact_retry());
    }
}
