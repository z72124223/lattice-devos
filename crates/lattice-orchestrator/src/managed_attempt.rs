//! Pure coordinator for one bounded durable managed-worker attempt.

use std::error::Error;
use std::fmt;

use lattice_contracts::{ContentDigest, SubjectBinding, WriterLeaseAuthorityHead};
use lattice_foreman_state::{
    AttemptPacketIdentity, AttemptWatchdogObservation, MeaningfulProgress, RestartDecision,
    RetryDecision, StallClassification, StallReason, StartGateDecision, StartObservation,
    WorkerAttemptError, WorkerAttemptPhase, WorkerAttemptState, WorkerBudget, WorkerTerminal,
    classify_attempt_stall, decide_repair_retry, restart_reconciliation_decision,
};
use lattice_ports::{
    ManagedArtifactReceipt, ManagedAttemptClaimDisposition, ManagedCodexWorkerPort,
    ManagedEvidenceKind, ManagedForemanRepositoryPort, ManagedModelAvailability, ManagedPortError,
    ManagedPortErrorKind, ManagedPortResult, ManagedPrestartClosureDisposition,
    ManagedPrestartNoEffectProof, ManagedProviderEffectGuardPort, ManagedReviewDispatchDisposition,
    ManagedReviewEvidenceSink, ManagedTerminalCandidate, ManagedVerificationPort,
    ManagedVerificationPreparation, ManagedVerificationRequest, ManagedWorkerDispatchState,
    ManagedWorkerExecutionEvent, ManagedWorkerObservation, ManagedWorkerPrestartRecovery,
    ManagedWorkerReconciliation, ManagedWorkerThreadDispatchDisposition,
    ManagedWorkerTurnDispatchDisposition, TaskLifecycleAutonomyEvidence, TaskLifecycleError,
    TaskLifecycleEvidence, TaskLifecyclePort, VerificationOutcome, VerifiedManagedEvidence,
    VerifiedTaskExecutionBinding, VerifiedTaskVerificationRecord, VerifiedWorkerAttemptRecord,
    VerifiedWorkerObservationRecord, WorkerObservationKind,
};
use lattice_task_domain::TaskState;
use lattice_writer_lease::{
    CommandOutcome as WriterLeaseCommandOutcome, WriterLeaseAcquireRequest,
    WriterLeaseReleaseRequest, WriterLeaseRepository, WriterLeaseRepositoryCommand,
    WriterLeaseRepositoryError,
};
use serde_json::Value;

use super::ControlledTaskRequest;

/// Fully bound inputs for one new managed-worker attempt. The execution
/// authority is opaque here and must be revalidated by the injected repository
/// before the atomic claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedAttemptRequest {
    binding: VerifiedTaskExecutionBinding,
    packet: AttemptPacketIdentity,
    authority_digest: ContentDigest,
    execution_preflight: Option<VerifiedManagedEvidence>,
    predispatch_baseline: Option<VerifiedManagedEvidence>,
}

const WSL2_ZERO_MODEL_PREFLIGHT_SCHEMA: &str = "lattice.wsl2-zero-model-preflight/1.0";

impl ManagedAttemptRequest {
    /// Constructs one digest-only coordination request.
    ///
    /// # Errors
    ///
    /// Rejects a cross-task packet or the all-zero authority sentinel.
    pub fn new(
        binding: VerifiedTaskExecutionBinding,
        packet: AttemptPacketIdentity,
        authority_digest: ContentDigest,
    ) -> Result<Self, ManagedAttemptOrchestratorError> {
        if packet.task_ref() != binding.task_ref().as_str()
            || authority_digest.as_str().bytes().all(|byte| byte == b'0')
        {
            return Err(ManagedAttemptOrchestratorError::BindingMismatch);
        }
        Ok(Self {
            binding,
            packet,
            authority_digest,
            execution_preflight: None,
            predispatch_baseline: None,
        })
    }

    /// Binds one exact, zero-provider-effect WSL2 technical preflight receipt
    /// to the immutable packet. The receipt is persisted after the atomic
    /// attempt claim and before the first provider dispatch claim.
    ///
    /// # Errors
    ///
    /// Rejects native-Windows packets, task/attempt/environment/worktree/HEAD
    /// substitutions, non-Linux paths, and any reported provider effect.
    pub fn with_execution_preflight(
        mut self,
        preflight: VerifiedManagedEvidence,
    ) -> Result<Self, ManagedAttemptOrchestratorError> {
        if self.packet.is_native_windows_execution_environment()
            || !execution_preflight_matches(&self, &preflight)
        {
            return Err(ManagedAttemptOrchestratorError::ExecutionPreflightMismatch);
        }
        self.execution_preflight = Some(preflight);
        Ok(self)
    }

    /// Binds one owner-verified managed-worktree baseline to this exact
    /// attempt. The Artifact Store object is persisted after the atomic claim
    /// and before the first provider thread RPC.
    ///
    /// # Errors
    ///
    /// Rejects task/attempt/project/schema or packet worktree substitutions.
    pub fn with_predispatch_baseline(
        mut self,
        baseline: VerifiedManagedEvidence,
    ) -> Result<Self, ManagedAttemptOrchestratorError> {
        let expected_worktree = self.packet.worktree_ref().strip_prefix("worktree:sha256:");
        if baseline.kind() != ManagedEvidenceKind::GitSnapshot
            || baseline.payload_schema() != "lattice.managed-worktree-baseline/1.0"
            || baseline.task_ref() != self.binding.task_ref()
            || u64::from(baseline.attempt()) != u64::from(self.packet.attempt())
            || expected_worktree != Some(baseline.content_digest().as_str())
        {
            return Err(ManagedAttemptOrchestratorError::BindingMismatch);
        }
        self.predispatch_baseline = Some(baseline);
        Ok(self)
    }

    #[must_use]
    pub const fn binding(&self) -> &VerifiedTaskExecutionBinding {
        &self.binding
    }

    #[must_use]
    pub const fn packet(&self) -> &AttemptPacketIdentity {
        &self.packet
    }

    #[must_use]
    pub const fn authority_digest(&self) -> &ContentDigest {
        &self.authority_digest
    }

    #[must_use]
    pub const fn execution_preflight(&self) -> Option<&VerifiedManagedEvidence> {
        self.execution_preflight.as_ref()
    }

    #[must_use]
    pub const fn predispatch_baseline(&self) -> Option<&VerifiedManagedEvidence> {
        self.predispatch_baseline.as_ref()
    }
}

fn execution_preflight_matches(
    request: &ManagedAttemptRequest,
    preflight: &VerifiedManagedEvidence,
) -> bool {
    if preflight.kind() != ManagedEvidenceKind::WorkerLifecycle
        || preflight.media_type() != "application/json"
        || preflight.payload_schema() != WSL2_ZERO_MODEL_PREFLIGHT_SCHEMA
        || preflight.task_ref() != request.binding.task_ref()
        || u64::from(preflight.attempt()) != u64::from(request.packet.attempt())
    {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<Value>(preflight.bytes()) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    let string = |key: &str| object.get(key).and_then(Value::as_str);
    let count = |key: &str| object.get(key).and_then(Value::as_u64);
    let Some(counters) = object.get("effect_counters").and_then(Value::as_object) else {
        return false;
    };
    let linux_cwd = string("linux_cwd").unwrap_or_default();
    string("schema") == Some(WSL2_ZERO_MODEL_PREFLIGHT_SCHEMA)
        && string("status") == Some("PASS")
        && string("task_ref") == Some(request.binding.task_ref().as_str())
        && count("attempt") == Some(u64::from(request.packet.attempt()))
        && string("execution_environment_ref") == Some(request.packet.execution_environment_ref())
        && string("repository_head") == Some(request.packet.base_commit())
        && string("worktree_ref") == Some(request.packet.worktree_ref())
        && linux_cwd.starts_with("/home/")
        && !linux_cwd.contains('\\')
        && count("provider_effect_count") == Some(0)
        && counters.get("thread_start").and_then(Value::as_u64) == Some(0)
        && counters.get("turn_start").and_then(Value::as_u64) == Some(0)
        && counters
            .get("provider_effect_count")
            .and_then(Value::as_u64)
            == Some(0)
}

/// The sole successful next-state recommendation emitted by this coordinator.
/// It contains no merge, push, deployment, release, or completion operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedAttemptTarget {
    AwaitingMergeApproval,
}

impl ManagedAttemptTarget {
    #[must_use]
    pub const fn task_state(self) -> TaskState {
        match self {
            Self::AwaitingMergeApproval => TaskState::AwaitingMergeApproval,
        }
    }
}

/// Verified terminal projection returned only after independent verification
/// has passed and its Task Ledger child record has been durably reloaded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedAttemptOutcome {
    attempt: VerifiedWorkerAttemptRecord,
    terminal: VerifiedWorkerObservationRecord,
    verification: VerifiedTaskVerificationRecord,
    target: ManagedAttemptTarget,
}

impl ManagedAttemptOutcome {
    #[must_use]
    pub const fn attempt(&self) -> &VerifiedWorkerAttemptRecord {
        &self.attempt
    }

    #[must_use]
    pub const fn terminal(&self) -> &VerifiedWorkerObservationRecord {
        &self.terminal
    }

    #[must_use]
    pub const fn verification(&self) -> &VerifiedTaskVerificationRecord {
        &self.verification
    }

    #[must_use]
    pub const fn target(&self) -> ManagedAttemptTarget {
        self.target
    }
}

/// Fail-closed managed-attempt coordination errors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedAttemptOrchestratorError {
    BindingMismatch,
    ClaimMismatch,
    ExecutionPreflightRequired,
    ExecutionPreflightMismatch,
    PredispatchBaselineRequired,
    DispatchReconciliationRequired,
    TurnDispatchReconciliationRequired,
    ReviewDispatchReconciliationRequired,
    ObservationMismatch,
    ExactStartNotConfirmed,
    MissingVerificationCandidate,
    ModelUnavailable { code: &'static str },
    ProviderEffectGuard(ManagedPortError),
    WorkerTerminal(WorkerTerminal),
    VerificationFailed(Box<VerifiedTaskVerificationRecord>),
    Domain(WorkerAttemptError),
    Repository(ManagedPortError),
    Worker(ManagedPortError),
    Verification(ManagedPortError),
}

impl fmt::Display for ManagedAttemptOrchestratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BindingMismatch => formatter.write_str("managed attempt binding mismatch"),
            Self::ClaimMismatch => formatter.write_str("managed attempt claim mismatch"),
            Self::ExecutionPreflightRequired => {
                formatter.write_str("managed WSL2 execution preflight is required")
            }
            Self::ExecutionPreflightMismatch => {
                formatter.write_str("managed WSL2 execution preflight mismatch")
            }
            Self::PredispatchBaselineRequired => {
                formatter.write_str("managed pre-dispatch baseline is required")
            }
            Self::DispatchReconciliationRequired => {
                formatter.write_str("managed attempt replay requires provider reconciliation")
            }
            Self::TurnDispatchReconciliationRequired => {
                formatter.write_str("managed turn replay requires provider reconciliation")
            }
            Self::ReviewDispatchReconciliationRequired => {
                formatter.write_str("managed review replay requires provider reconciliation")
            }
            Self::ObservationMismatch => formatter.write_str("managed worker observation mismatch"),
            Self::ExactStartNotConfirmed => {
                formatter.write_str("exact in-progress turn/started not confirmed")
            }
            Self::MissingVerificationCandidate => {
                formatter.write_str("completed worker omitted verification candidate")
            }
            Self::ModelUnavailable { code } => {
                write!(formatter, "managed model unavailable: {code}")
            }
            Self::ProviderEffectGuard(error) => {
                write!(
                    formatter,
                    "managed provider effect writer rejected: {error}"
                )
            }
            Self::WorkerTerminal(terminal) => {
                write!(
                    formatter,
                    "worker reached non-completed terminal: {terminal:?}"
                )
            }
            Self::VerificationFailed(_) => formatter.write_str("independent verification failed"),
            Self::Domain(error) => write!(formatter, "managed worker state rejected: {error:?}"),
            Self::Repository(error) => write!(formatter, "managed repository rejected: {error}"),
            Self::Worker(error) => write!(formatter, "managed worker rejected: {error}"),
            Self::Verification(error) => {
                write!(formatter, "managed verification rejected: {error}")
            }
        }
    }
}

impl Error for ManagedAttemptOrchestratorError {}

/// Runs one new attempt using only injected boundaries.
///
/// Exact effect order is effect-free model-availability preflight -> current
/// authorization -> atomic durable claim -> thread/turn RPC acceptance -> exact in-progress
/// `turn/started` -> execution observation -> exact terminal -> independent
/// verification -> durable verification record. The first failure suppresses
/// every later call.
///
/// # Errors
///
/// Fails closed on any authority, identity, lifecycle, provider, repository,
/// or verification mismatch. A verification failure is returned with its
/// retained verified record and never becomes merge approval.
pub fn run_managed_attempt<R, W, V, G>(
    request: &ManagedAttemptRequest,
    repository: &mut R,
    worker: &mut W,
    verifier: &mut V,
    provider_guard: &mut G,
) -> Result<ManagedAttemptOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
    V: ManagedVerificationPort,
    G: ManagedProviderEffectGuardPort,
{
    let starting = prepare_managed_attempt(request, repository, worker, provider_guard)?;
    let executing = confirm_managed_exact_start(starting, repository, worker)?;
    finish_managed_attempt(executing, repository, worker, verifier, provider_guard)
}

/// Type-state token proving thread/turn start acceptance while the parent Task
/// remains `PREPARING`. Fields are private so a caller cannot manufacture the
/// token or skip the exact-start gate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedStartingAttempt {
    request: ManagedAttemptRequest,
    attempt: VerifiedWorkerAttemptRecord,
    state: WorkerAttemptState,
}

/// Closed result of recovering only the pre-exact-start crash windows. A
/// `Starting` token still requires the ordinary exact-start gate; a failed
/// start can only feed retry/block handling and can never enter verification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedPrestartRestartOutcome {
    NoProviderEffect(ManagedPrestartNoEffectProof),
    Starting(Box<ManagedStartingAttempt>),
    FailedStart {
        terminal: Box<VerifiedWorkerObservationRecord>,
    },
    ReconciliationRequired,
}

impl ManagedStartingAttempt {
    #[must_use]
    pub const fn attempt(&self) -> &VerifiedWorkerAttemptRecord {
        &self.attempt
    }

    #[must_use]
    pub const fn state(&self) -> &WorkerAttemptState {
        &self.state
    }
}

/// Type-state token proving the exact matching in-progress `turn/started` was
/// durably recorded. Only this token may reach execution observation/terminal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedExecutingAttempt {
    request: ManagedAttemptRequest,
    attempt: VerifiedWorkerAttemptRecord,
    state: WorkerAttemptState,
}

impl ManagedExecutingAttempt {
    #[must_use]
    pub const fn attempt(&self) -> &VerifiedWorkerAttemptRecord {
        &self.attempt
    }

    #[must_use]
    pub const fn state(&self) -> &WorkerAttemptState {
        &self.state
    }
}

/// Type-state token proving the exact worker terminal was durably appended.
/// It carries no verification or review claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTerminalAttempt {
    request: ManagedAttemptRequest,
    attempt: VerifiedWorkerAttemptRecord,
    terminal: VerifiedWorkerObservationRecord,
}

impl ManagedTerminalAttempt {
    #[must_use]
    pub const fn attempt(&self) -> &VerifiedWorkerAttemptRecord {
        &self.attempt
    }

    #[must_use]
    pub const fn terminal(&self) -> &VerifiedWorkerObservationRecord {
        &self.terminal
    }
}

/// Reconstitutes the terminal type-state token from an exact durable claim and
/// completed terminal projection after a fresh process restart. No provider,
/// Git, reviewer, or persistence effect occurs here.
///
/// # Errors
///
/// Rejects any task, attempt, packet, writer-fence, or terminal substitution.
pub fn replay_managed_terminal(
    request: ManagedAttemptRequest,
    attempt: VerifiedWorkerAttemptRecord,
    terminal: VerifiedWorkerObservationRecord,
) -> Result<ManagedTerminalAttempt, ManagedAttemptOrchestratorError> {
    ensure_claim(&request, &attempt)?;
    if terminal.task_ref() != request.binding.task_ref()
        || terminal.successor_stream_id() != request.binding.successor_stream_id()
        || terminal.binding_digest() != request.binding.binding_digest()
        || terminal.attempt_id() != attempt.attempt_id()
        || terminal.attempt_number() != attempt.attempt_number()
        || terminal.kind() != WorkerObservationKind::TerminalCompleted
        || terminal.turn_id().is_none()
    {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    Ok(ManagedTerminalAttempt {
        request,
        attempt,
        terminal,
    })
}

/// Type-state token proving the mechanical Git/check evidence was prepared,
/// durably recorded, and passed. Only this token may start a semantic reviewer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedReviewReadyAttempt {
    terminal: ManagedTerminalAttempt,
    preparation: ManagedVerificationPreparation,
}

impl ManagedReviewReadyAttempt {
    #[must_use]
    pub const fn binding(&self) -> &VerifiedTaskExecutionBinding {
        &self.terminal.request.binding
    }

    #[must_use]
    pub const fn attempt(&self) -> &VerifiedWorkerAttemptRecord {
        &self.terminal.attempt
    }

    #[must_use]
    pub const fn terminal(&self) -> &VerifiedWorkerObservationRecord {
        &self.terminal.terminal
    }

    #[must_use]
    pub const fn verification_request(&self) -> &ManagedVerificationRequest {
        self.preparation.request()
    }
}

/// Type-state token proving the exact semantic-review provider effect was
/// durably claimed after mechanical preparation. The disposition is retained
/// so composition must choose either a fresh reviewer or an explicit replay
/// reconciler before any review effect is reachable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedClaimedReviewAttempt {
    reviewing: ManagedReviewReadyAttempt,
    disposition: ManagedReviewDispatchDisposition,
}

impl ManagedClaimedReviewAttempt {
    #[must_use]
    pub const fn disposition(&self) -> ManagedReviewDispatchDisposition {
        self.disposition
    }

    #[must_use]
    pub const fn attempt(&self) -> &VerifiedWorkerAttemptRecord {
        self.reviewing.attempt()
    }

    #[must_use]
    pub const fn binding(&self) -> &VerifiedTaskExecutionBinding {
        &self.reviewing.terminal.request.binding
    }

    #[must_use]
    pub const fn terminal(&self) -> &VerifiedWorkerObservationRecord {
        self.reviewing.terminal()
    }

    #[must_use]
    pub const fn verification_request(&self) -> &ManagedVerificationRequest {
        self.reviewing.preparation.request()
    }
}

/// Full existing Task-lifecycle/Writer binding paired with one exact managed
/// attempt. The two bindings must commit the same Task Spec and attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedWorkflowRequest {
    control: ControlledTaskRequest,
    attempt: ManagedAttemptRequest,
}

impl ManagedWorkflowRequest {
    /// Joins the existing lifecycle request to the exact managed-attempt
    /// binding.
    ///
    /// # Errors
    ///
    /// Rejects cross-spec bindings or an empty attempt identity.
    pub fn new(
        control: ControlledTaskRequest,
        attempt: ManagedAttemptRequest,
    ) -> Result<Self, ManagedWorkflowError> {
        if control.binding.task_spec_digest() != attempt.binding.task_spec_digest()
            || control.attempt_id.as_str().is_empty()
        {
            return Err(ManagedWorkflowError::BindingMismatch);
        }
        Ok(Self { control, attempt })
    }

    #[must_use]
    pub const fn subject_binding(&self) -> &SubjectBinding {
        &self.control.binding
    }

    #[must_use]
    pub const fn attempt_request(&self) -> &ManagedAttemptRequest {
        &self.attempt
    }
}

/// High-level managed workflow failure. Any failure after Writer acquisition
/// deliberately retains the exact lease/fence for retry or reconciliation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedWorkflowError {
    BindingMismatch,
    StateMismatch,
    LeaseMismatch,
    ExecutionApprovalRequired,
    ReconciliationRequired,
    Lifecycle(TaskLifecycleError),
    Lease(WriterLeaseRepositoryError),
    Attempt(Box<ManagedAttemptOrchestratorError>),
}

impl fmt::Display for ManagedWorkflowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BindingMismatch => formatter.write_str("managed workflow binding mismatch"),
            Self::StateMismatch => formatter.write_str("managed workflow state mismatch"),
            Self::LeaseMismatch => formatter.write_str("managed workflow lease mismatch"),
            Self::ExecutionApprovalRequired => {
                formatter.write_str("managed workflow awaits execution approval")
            }
            Self::ReconciliationRequired => {
                formatter.write_str("managed workflow requires reconciliation")
            }
            Self::Lifecycle(error) => write!(formatter, "managed lifecycle rejected: {error}"),
            Self::Lease(error) => write!(formatter, "managed writer lease rejected: {error}"),
            Self::Attempt(error) => write!(formatter, "managed attempt rejected: {error}"),
        }
    }
}

impl Error for ManagedWorkflowError {}

fn workflow_attempt_error(error: ManagedAttemptOrchestratorError) -> ManagedWorkflowError {
    ManagedWorkflowError::Attempt(Box::new(error))
}

/// Successful high-level projection. The Task Ledger is exactly
/// `AWAITING_MERGE_APPROVAL`, the Writer Lease is released, and no merge or
/// completion effect has occurred.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedWorkflowOutcome {
    attempt: ManagedAttemptOutcome,
    lifecycle: TaskLifecycleEvidence,
}

impl ManagedWorkflowOutcome {
    #[must_use]
    pub const fn attempt(&self) -> &ManagedAttemptOutcome {
        &self.attempt
    }

    #[must_use]
    pub const fn lifecycle(&self) -> &TaskLifecycleEvidence {
        &self.lifecycle
    }
}

/// Runs the complete pure Phase-4 local-execution order while leaving merge,
/// push, deploy, release, and completion structurally unreachable.
///
/// The parent Task remains `PREPARING` through claim and both accepted start
/// RPCs. Only after the exact `turn/started` observation is durably appended is
/// `PREPARING -> EXECUTING` requested. Verification pass advances only through
/// `VERIFYING`, `REVIEWING`, and `AWAITING_MERGE_APPROVAL`, then releases the
/// exact Writer Lease. Every failure after acquisition retains the lease/fence.
///
/// # Errors
///
/// Fails closed on lifecycle, authority, lease, attempt, provider, artifact,
/// or verification mismatch. Any error after writer acquisition retains the
/// exact lease and fence for reconciliation or bounded repair.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn run_managed_workflow<T, L, R, W, V>(
    request: &ManagedWorkflowRequest,
    lifecycle: &mut T,
    writer_lease: &mut L,
    repository: &mut R,
    worker: &mut W,
    verifier: &mut V,
) -> Result<ManagedWorkflowOutcome, ManagedWorkflowError>
where
    T: TaskLifecyclePort,
    L: WriterLeaseRepository,
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
    V: ManagedVerificationPort,
{
    run_managed_workflow_with_verified_hook(
        request,
        lifecycle,
        writer_lease,
        repository,
        worker,
        verifier,
        |_, _, _| Ok(()),
    )
}

/// Runs the complete workflow with one fail-closed hook after the passing
/// verification record is durably reloaded and before lifecycle advance or
/// Writer Lease release. Production uses this boundary to create or exactly
/// replay the task-owned protected Git ref.
///
/// # Errors
///
/// Propagates a hook failure as a repository attempt error while the Task
/// remains `REVIEWING` and the exact Writer Lease stays retained.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn run_managed_workflow_with_verified_hook<T, L, R, W, V, F>(
    request: &ManagedWorkflowRequest,
    lifecycle: &mut T,
    writer_lease: &mut L,
    repository: &mut R,
    worker: &mut W,
    verifier: &mut V,
    verified_hook: F,
) -> Result<ManagedWorkflowOutcome, ManagedWorkflowError>
where
    T: TaskLifecyclePort,
    L: WriterLeaseRepository,
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
    V: ManagedVerificationPort,
    F: FnOnce(&ManagedAttemptOutcome, &mut R, &mut L) -> Result<(), ManagedPortError>,
{
    run_managed_workflow_with_review_configuration_and_verified_hook(
        request,
        lifecycle,
        writer_lease,
        repository,
        worker,
        verifier,
        |_| Ok(()),
        |claimed, _, _| {
            if claimed.disposition() != ManagedReviewDispatchDisposition::Claimed {
                return Err(ManagedPortError::new(
                    ManagedPortErrorKind::ReconcileRequired,
                    "LATTICE_MANAGED_REVIEW_DISPATCH_RECONCILIATION_REQUIRED",
                ));
            }
            Ok(())
        },
        verified_hook,
    )
}

/// Runs the complete workflow while exposing one post-claim configuration
/// boundary. The callback receives the immutable claimed-review type-state and
/// must configure only the matching fresh or replay reviewer before any
/// provider review effect can occur.
///
/// # Errors
///
/// Propagates a post-claim configuration failure without invoking the reviewer;
/// the durable review claim remains the sole restart/reconciliation authority.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn run_managed_workflow_with_review_configuration_and_verified_hook<T, L, R, W, V, P, C, F>(
    request: &ManagedWorkflowRequest,
    lifecycle: &mut T,
    writer_lease: &mut L,
    repository: &mut R,
    worker: &mut W,
    verifier: &mut V,
    validate_writer: P,
    configure_review: C,
    verified_hook: F,
) -> Result<ManagedWorkflowOutcome, ManagedWorkflowError>
where
    T: TaskLifecyclePort,
    L: WriterLeaseRepository,
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
    V: ManagedVerificationPort,
    P: FnOnce(&WriterLeaseAuthorityHead) -> Result<(), ManagedPortError>,
    C: FnOnce(&ManagedClaimedReviewAttempt, &mut R, &mut V) -> Result<(), ManagedPortError>,
    F: FnOnce(&ManagedAttemptOutcome, &mut R, &mut L) -> Result<(), ManagedPortError>,
{
    let binding = &request.control.binding;
    let admission = lifecycle
        .admit(binding, &request.control.client_request_id)
        .map_err(ManagedWorkflowError::Lifecycle)?;
    if admission.binding() != binding {
        return Err(ManagedWorkflowError::StateMismatch);
    }
    let (existing, existing_state) = match admission.into_existing() {
        None => (None, TaskState::Draft),
        Some(evidence) => {
            if !matches!(
                evidence.state(),
                TaskState::Draft | TaskState::AwaitingExecutionApproval
            ) || evidence.result_digest().is_some()
            {
                return Err(ManagedWorkflowError::ReconciliationRequired);
            }
            (Some(evidence.clone()), evidence.state())
        }
    };

    let autonomy = if let Some(existing) = existing {
        existing
    } else {
        lifecycle
            .record_autonomy_receipt(binding, None)
            .map_err(ManagedWorkflowError::Lifecycle)?
    };
    match autonomy.autonomy_evidence() {
        TaskLifecycleAutonomyEvidence::RequiredComplete(receipt)
        | TaskLifecycleAutonomyEvidence::HistoricalOptional(Some(receipt)) => {
            // The intake receipt records whether the objective itself granted
            // autonomy. It is deliberately not the later, task/spec/budget-
            // bound execution authority, which is verified below.
            let _ = receipt.disposition();
        }
        TaskLifecycleAutonomyEvidence::HistoricalOptional(None) => {}
        TaskLifecycleAutonomyEvidence::Unadmitted => {
            return Err(ManagedWorkflowError::StateMismatch);
        }
    }
    ensure_managed_task_awaiting_execution_approval(lifecycle, binding, existing_state)?;
    // `AWAITING_EXECUTION_APPROVAL` is a real durable gate, not a transient
    // label inferred from the intake receipt. If the exact later
    // task/spec/budget-bound Policy authority cannot be revalidated, fail here
    // without acquiring a Writer lease, claiming a worker, or opening a Codex
    // thread/turn.
    repository
        .assert_execution_authority_current(
            &request.attempt.binding,
            &request.attempt.authority_digest,
        )
        .map_err(|error| {
            workflow_attempt_error(ManagedAttemptOrchestratorError::Repository(error))
        })?;

    if writer_lease
        .current_authority(binding.project_id())
        .map_err(ManagedWorkflowError::Lease)?
        .is_some()
    {
        return Err(ManagedWorkflowError::ReconciliationRequired);
    }
    let acquired = writer_lease
        .execute(WriterLeaseRepositoryCommand::Acquire(
            WriterLeaseAcquireRequest {
                command_id: managed_writer_command_id(
                    binding,
                    request.attempt.packet.attempt(),
                    "acquire",
                ),
                expected_head: None,
                project_id: binding.project_id().clone(),
                project_snapshot_id: binding.project_snapshot_id().clone(),
                task_id: binding.task_id().clone(),
                task_revision: binding.task_revision().to_owned(),
                task_spec_digest: binding.task_spec_digest().clone(),
                attempt_id: request.control.attempt_id.clone(),
                lease_id: request.control.lease_id.clone(),
                lease_holder_id: request.control.lease_holder_id.clone(),
                worktree_id: request.control.worktree_id.clone(),
                holder_process_id: request.control.holder_process_id,
                holder_process_start_identity: request
                    .control
                    .holder_process_start_identity
                    .clone(),
            },
        ))
        .map_err(ManagedWorkflowError::Lease)?;
    let authority = acquired
        .after
        .filter(|_| acquired.outcome == WriterLeaseCommandOutcome::Applied)
        .ok_or(ManagedWorkflowError::LeaseMismatch)?;
    ensure_workflow_writer(&authority, binding, request)?;
    validate_writer(&authority).map_err(|failure| {
        workflow_attempt_error(ManagedAttemptOrchestratorError::ProviderEffectGuard(
            failure,
        ))
    })?;
    let current = writer_lease
        .current_authority(binding.project_id())
        .map_err(ManagedWorkflowError::Lease)?
        .ok_or(ManagedWorkflowError::LeaseMismatch)?;
    if current.independent_head() != &authority {
        return Err(ManagedWorkflowError::LeaseMismatch);
    }
    writer_lease
        .assert_current(&authority)
        .map_err(ManagedWorkflowError::Lease)?;

    workflow_transition(
        lifecycle,
        binding,
        TaskState::AwaitingExecutionApproval,
        TaskState::Preparing,
        None,
    )?;

    let mut provider_guard =
        |guard_binding: &VerifiedTaskExecutionBinding,
         guard_attempt: &VerifiedWorkerAttemptRecord| {
            if guard_binding != &request.attempt.binding
                || guard_attempt.writer_fence() != authority.identity().fencing_token().get()
            {
                return Err(ManagedPortError::new(
                    ManagedPortErrorKind::ReconcileRequired,
                    "LATTICE_MANAGED_PROVIDER_WRITER_FENCE_MISMATCH",
                ));
            }
            writer_lease.assert_current(&authority).map_err(|_| {
                ManagedPortError::new(
                    ManagedPortErrorKind::ReconcileRequired,
                    "LATTICE_MANAGED_PROVIDER_WRITER_NOT_CURRENT",
                )
            })
        };
    let starting =
        prepare_managed_attempt(&request.attempt, repository, worker, &mut provider_guard)
            .map_err(workflow_attempt_error)?;
    let executing = confirm_managed_exact_start(starting, repository, worker)
        .map_err(workflow_attempt_error)?;
    writer_lease
        .assert_current(&authority)
        .map_err(ManagedWorkflowError::Lease)?;
    workflow_transition(
        lifecycle,
        binding,
        TaskState::Preparing,
        TaskState::Executing,
        Some(&authority),
    )?;

    let terminal =
        finish_managed_execution(executing, repository, worker).map_err(workflow_attempt_error)?;
    writer_lease
        .assert_current(&authority)
        .map_err(ManagedWorkflowError::Lease)?;
    workflow_transition(
        lifecycle,
        binding,
        TaskState::Executing,
        TaskState::Verifying,
        Some(&authority),
    )?;
    let review_ready =
        prepare_managed_review(terminal, repository, verifier).map_err(workflow_attempt_error)?;
    workflow_transition(
        lifecycle,
        binding,
        TaskState::Verifying,
        TaskState::Reviewing,
        Some(&authority),
    )?;
    let claimed = claim_managed_review(review_ready, repository).map_err(workflow_attempt_error)?;
    configure_review(&claimed, repository, verifier).map_err(|failure| {
        workflow_attempt_error(ManagedAttemptOrchestratorError::Verification(failure))
    })?;
    let attempt = match claimed.disposition() {
        ManagedReviewDispatchDisposition::Claimed => {
            let mut review_provider_guard =
                |guard_binding: &VerifiedTaskExecutionBinding,
                 guard_attempt: &VerifiedWorkerAttemptRecord| {
                    if guard_binding != &request.attempt.binding
                        || guard_attempt.writer_fence()
                            != authority.identity().fencing_token().get()
                    {
                        return Err(ManagedPortError::new(
                            ManagedPortErrorKind::ReconcileRequired,
                            "LATTICE_MANAGED_PROVIDER_WRITER_FENCE_MISMATCH",
                        ));
                    }
                    writer_lease.assert_current(&authority).map_err(|_| {
                        ManagedPortError::new(
                            ManagedPortErrorKind::ReconcileRequired,
                            "LATTICE_MANAGED_PROVIDER_WRITER_NOT_CURRENT",
                        )
                    })
                };
            finish_claimed_managed_review(claimed, repository, verifier, &mut review_provider_guard)
        }
        ManagedReviewDispatchDisposition::ExactReplay => {
            let mut review_provider_guard =
                |guard_binding: &VerifiedTaskExecutionBinding,
                 guard_attempt: &VerifiedWorkerAttemptRecord| {
                    if guard_binding != &request.attempt.binding
                        || guard_attempt.writer_fence()
                            != authority.identity().fencing_token().get()
                    {
                        return Err(ManagedPortError::new(
                            ManagedPortErrorKind::ReconcileRequired,
                            "LATTICE_MANAGED_PROVIDER_WRITER_FENCE_MISMATCH",
                        ));
                    }
                    writer_lease.assert_current(&authority).map_err(|_| {
                        ManagedPortError::new(
                            ManagedPortErrorKind::ReconcileRequired,
                            "LATTICE_MANAGED_PROVIDER_WRITER_NOT_CURRENT",
                        )
                    })
                };
            finish_replayed_managed_review_with_provider_guard(
                claimed,
                repository,
                verifier,
                &mut review_provider_guard,
            )
        }
    }
    .map_err(workflow_attempt_error)?;
    verified_hook(&attempt, repository, writer_lease).map_err(|failure| {
        workflow_attempt_error(ManagedAttemptOrchestratorError::Repository(failure))
    })?;
    let final_lifecycle = workflow_transition(
        lifecycle,
        binding,
        TaskState::Reviewing,
        TaskState::AwaitingMergeApproval,
        Some(&authority),
    )?;

    writer_lease
        .assert_current(&authority)
        .map_err(ManagedWorkflowError::Lease)?;
    let release = writer_lease
        .execute(WriterLeaseRepositoryCommand::Release(
            WriterLeaseReleaseRequest {
                command_id: managed_writer_command_id(
                    binding,
                    request.attempt.packet.attempt(),
                    "release",
                ),
                project_id: binding.project_id().clone(),
                expected_head: authority,
            },
        ))
        .map_err(ManagedWorkflowError::Lease)?;
    if release.outcome != WriterLeaseCommandOutcome::Applied || release.after.is_some() {
        return Err(ManagedWorkflowError::LeaseMismatch);
    }
    if writer_lease
        .current_authority(binding.project_id())
        .map_err(ManagedWorkflowError::Lease)?
        .is_some()
    {
        return Err(ManagedWorkflowError::LeaseMismatch);
    }
    ensure_workflow_evidence(&final_lifecycle, binding, TaskState::AwaitingMergeApproval)?;
    Ok(ManagedWorkflowOutcome {
        attempt,
        lifecycle: final_lifecycle,
    })
}

/// Makes the Orchestrator the single semantic owner of the normal durable
/// `DRAFT -> AWAITING_EXECUTION_APPROVAL` transition.
///
/// Runtime services may coordinate preparation and persistence, but must call
/// this contract instead of mutating the Task lifecycle directly.
///
/// # Errors
///
/// Propagates the lifecycle transition failure and rejects any state outside
/// the exact initial approval-gate pair.
pub fn ensure_managed_task_awaiting_execution_approval<T: TaskLifecyclePort>(
    lifecycle: &mut T,
    binding: &SubjectBinding,
    current_state: TaskState,
) -> Result<(), ManagedWorkflowError> {
    match current_state {
        TaskState::Draft => workflow_transition(
            lifecycle,
            binding,
            TaskState::Draft,
            TaskState::AwaitingExecutionApproval,
            None,
        )
        .map(|_| ()),
        TaskState::AwaitingExecutionApproval => Ok(()),
        _ => Err(ManagedWorkflowError::ReconciliationRequired),
    }
}

fn workflow_transition<T: TaskLifecyclePort>(
    lifecycle: &mut T,
    binding: &SubjectBinding,
    from: TaskState,
    to: TaskState,
    authority: Option<&WriterLeaseAuthorityHead>,
) -> Result<TaskLifecycleEvidence, ManagedWorkflowError> {
    let evidence = lifecycle
        .transition(binding, from, to, authority)
        .map_err(ManagedWorkflowError::Lifecycle)?;
    ensure_workflow_evidence(&evidence, binding, to)?;
    Ok(evidence)
}

fn ensure_workflow_evidence(
    evidence: &TaskLifecycleEvidence,
    binding: &SubjectBinding,
    state: TaskState,
) -> Result<(), ManagedWorkflowError> {
    if evidence.binding() != binding || evidence.state() != state || !evidence.admitted() {
        return Err(ManagedWorkflowError::StateMismatch);
    }
    Ok(())
}

fn ensure_workflow_writer(
    authority: &WriterLeaseAuthorityHead,
    binding: &SubjectBinding,
    request: &ManagedWorkflowRequest,
) -> Result<(), ManagedWorkflowError> {
    let identity = authority.identity();
    if identity.project_id() != binding.project_id()
        || identity.project_snapshot_id() != binding.project_snapshot_id()
        || identity.task_id() != binding.task_id()
        || identity.task_revision() != binding.task_revision()
        || identity.task_spec_digest() != binding.task_spec_digest()
        || identity.attempt_id() != &request.control.attempt_id
        || identity.fencing_token().get() != request.attempt.packet.writer_fence()
    {
        return Err(ManagedWorkflowError::LeaseMismatch);
    }
    Ok(())
}

fn managed_writer_command_id(binding: &SubjectBinding, attempt: u8, operation: &str) -> String {
    let suffix = &binding.task_spec_digest().as_str()[..20];
    format!("managed-{operation}-{suffix}-{attempt}")
}

/// Performs model preflight, current-authority validation, atomic claim, and
/// the two accepted start RPCs. It deliberately stops before exact start and
/// cannot mark the parent Task executing.
///
/// # Errors
///
/// Fails closed on unavailable model, stale authority, claim substitution,
/// provider start uncertainty, or durable observation mismatch.
pub fn prepare_managed_attempt<R, W, G>(
    request: &ManagedAttemptRequest,
    repository: &mut R,
    worker: &mut W,
    provider_guard: &mut G,
) -> Result<ManagedStartingAttempt, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
    G: ManagedProviderEffectGuardPort,
{
    if !request.packet.is_native_windows_execution_environment()
        && request.execution_preflight().is_none()
    {
        return Err(ManagedAttemptOrchestratorError::ExecutionPreflightRequired);
    }
    // Native Windows availability remains a no-effect preclaim probe. WSL2
    // must not spawn an unowned App Server before a durable process segment
    // exists, so its account/model read is deferred until the exact OPEN
    // marker below has been persisted.
    if request.packet.is_native_windows_execution_environment() {
        ensure_restart_model_available(worker, &request.packet)?;
    }

    repository
        .assert_execution_authority_current(&request.binding, &request.authority_digest)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;

    let claim = repository
        .claim_attempt(&request.binding, &request.packet)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    if claim.disposition() == ManagedAttemptClaimDisposition::ExactReplay {
        return Err(ManagedAttemptOrchestratorError::DispatchReconciliationRequired);
    }
    let attempt = claim.into_attempt();
    ensure_claim(request, &attempt)?;

    if let Some(preflight) = request.execution_preflight() {
        let receipt = repository
            .record_artifact(&request.binding, &attempt, preflight)
            .map_err(ManagedAttemptOrchestratorError::Repository)?;
        if !receipt.matches(preflight) {
            return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
        }
    }

    if let Some(baseline) = request.predispatch_baseline() {
        let receipt = repository
            .record_artifact(&request.binding, &attempt, baseline)
            .map_err(ManagedAttemptOrchestratorError::Repository)?;
        if !receipt.matches(baseline) {
            return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
        }
    }

    let thread_claim = repository
        .claim_worker_thread_dispatch(&request.binding, &attempt)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    if thread_claim == ManagedWorkerThreadDispatchDisposition::ExactReplay {
        return Err(ManagedAttemptOrchestratorError::DispatchReconciliationRequired);
    }

    let mut state = WorkerAttemptState::new(request.packet.clone())
        .map_err(ManagedAttemptOrchestratorError::Domain)?;
    state
        .begin_dispatch()
        .map_err(ManagedAttemptOrchestratorError::Domain)?;

    if request.execution_preflight().is_some() {
        let lifecycle = worker
            .prepare_provider_dispatch(&attempt, &request.packet)
            .map_err(ManagedAttemptOrchestratorError::Worker)?;
        persist_provider_lifecycle(
            repository,
            &request.binding,
            &attempt,
            &lifecycle,
            "lattice.wsl2-provider-subtree-marker/1.0",
        )?;
        ensure_restart_model_available(worker, &request.packet)?;
    }

    provider_guard
        .assert_provider_effect_writer_current(&request.binding, &attempt)
        .map_err(ManagedAttemptOrchestratorError::ProviderEffectGuard)?;
    let thread = worker
        .start_thread(&attempt, &request.packet)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    apply_expected_start(
        &mut state,
        &thread,
        WorkerObservationKind::ThreadAccepted,
        WorkerAttemptPhase::Accepted,
    )?;
    let thread_record = persist_observation(repository, &request.binding, &attempt, &thread)?;

    let turn_claim = repository
        .claim_worker_turn_dispatch(&request.binding, &attempt, &thread_record)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    if turn_claim == ManagedWorkerTurnDispatchDisposition::ExactReplay {
        return Err(ManagedAttemptOrchestratorError::TurnDispatchReconciliationRequired);
    }

    let thread_id = state
        .thread_id()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?
        .to_owned();
    provider_guard
        .assert_provider_effect_writer_current(&request.binding, &attempt)
        .map_err(ManagedAttemptOrchestratorError::ProviderEffectGuard)?;
    let turn = worker
        .start_turn(&attempt, &thread_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    apply_expected_start(
        &mut state,
        &turn,
        WorkerObservationKind::TurnAccepted,
        WorkerAttemptPhase::Starting,
    )?;
    persist_observation(repository, &request.binding, &attempt, &turn)?;

    Ok(ManagedStartingAttempt {
        request: request.clone(),
        attempt,
        state,
    })
}

/// Recovers only an already verified active attempt that has not durably
/// crossed the exact-start gate. The `WorkerThread` dispatch claim is the sole
/// authority for a fresh thread: `Claimed` may use `start_thread`, while
/// `ExactReplay` is forced through the worker's recovery-only boundary.
///
/// A recovered marker turn without durable exact-start evidence is always
/// persisted as a failed-start terminal. It never returns an executing token
/// or enters verification, even when the provider-native terminal was
/// `completed`.
///
/// # Errors
///
/// Rejects attempt/state substitution, provider identity drift, an unsafe
/// dispatch replay, or any durable append mismatch. Stale authority and model
/// availability gate only a new provider effect; exact recovery remains
/// available so an already live provider cannot be abandoned.
#[allow(clippy::too_many_lines)]
pub fn recover_managed_prestart_on_restart<R, W, G>(
    request: &ManagedAttemptRequest,
    attempt: &VerifiedWorkerAttemptRecord,
    retained_state: &WorkerAttemptState,
    repository: &mut R,
    worker: &mut W,
    provider_guard: &mut G,
) -> Result<ManagedPrestartRestartOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
    G: ManagedProviderEffectGuardPort,
{
    ensure_claim(request, attempt)?;
    ensure_attempt_state(attempt, retained_state)?;
    if !matches!(
        retained_state.phase(),
        WorkerAttemptPhase::Claimed
            | WorkerAttemptPhase::Dispatching
            | WorkerAttemptPhase::Accepted
            | WorkerAttemptPhase::Starting
    ) {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }

    let dispatch_state = repository
        .load_worker_dispatch_state(&request.binding, attempt)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    let recovery = if matches!(
        retained_state.phase(),
        WorkerAttemptPhase::Claimed | WorkerAttemptPhase::Dispatching
    ) {
        if dispatch_state == ManagedWorkerDispatchState::NoWorkerThread {
            return Ok(ManagedPrestartRestartOutcome::NoProviderEffect(
                ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
                    worker_thread_claimed: false,
                },
            ));
        }
        provider_guard
            .assert_provider_effect_writer_current(&request.binding, attempt)
            .map_err(ManagedAttemptOrchestratorError::ProviderEffectGuard)?;
        worker
            .recover_claimed_dispatch(attempt, &request.packet)
            .map_err(ManagedAttemptOrchestratorError::Worker)?
    } else {
        if dispatch_state == ManagedWorkerDispatchState::NoWorkerThread {
            return Ok(ManagedPrestartRestartOutcome::ReconciliationRequired);
        }
        let thread_id = retained_state
            .thread_id()
            .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?;
        provider_guard
            .assert_provider_effect_writer_current(&request.binding, attempt)
            .map_err(ManagedAttemptOrchestratorError::ProviderEffectGuard)?;
        worker
            .recover_prestart(attempt, thread_id, retained_state.turn_id())
            .map_err(ManagedAttemptOrchestratorError::Worker)?
    };

    match recovery {
        ManagedWorkerPrestartRecovery::ProvenNoProviderCandidate => {
            // Bounded provider discovery is not an exact absence proof after
            // any durable provider claim. The claimed thread may become
            // discoverable later, so retain the attempt for reconciliation.
            if dispatch_state != ManagedWorkerDispatchState::NoWorkerThread {
                return Ok(ManagedPrestartRestartOutcome::ReconciliationRequired);
            }
            Ok(ManagedPrestartRestartOutcome::NoProviderEffect(
                ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
                    worker_thread_claimed: false,
                },
            ))
        }
        ManagedWorkerPrestartRecovery::ReconciliationRequired => {
            Ok(ManagedPrestartRestartOutcome::ReconciliationRequired)
        }
        ManagedWorkerPrestartRecovery::ExactEmptyThread { thread } => {
            if dispatch_state == ManagedWorkerDispatchState::NoWorkerThread
                || thread.kind() != WorkerObservationKind::ThreadAccepted
                || thread.turn_id().is_some()
                || retained_state.turn_id().is_some()
                || retained_state
                    .thread_id()
                    .is_some_and(|retained| retained != thread.thread_id())
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            let thread_record =
                persist_observation(repository, &request.binding, attempt, &thread)?;
            Ok(ManagedPrestartRestartOutcome::NoProviderEffect(
                ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn {
                    thread: Box::new(thread_record),
                    worker_turn_claimed: dispatch_state
                        == ManagedWorkerDispatchState::WorkerTurnClaimed,
                },
            ))
        }
        ManagedWorkerPrestartRecovery::ExactFailedStart {
            thread,
            turn,
            terminal,
        } => {
            if thread.kind() != WorkerObservationKind::ThreadAccepted
                || thread.turn_id().is_some()
                || turn.kind() != WorkerObservationKind::TurnAccepted
                || turn.thread_id() != thread.thread_id()
                || terminal.observation().kind() != WorkerObservationKind::PrestartTerminalFailed
                || terminal.observation().terminal_kind() != Some(WorkerTerminal::Failed)
                || terminal.observation().thread_id() != thread.thread_id()
                || terminal.observation().turn_id() != turn.turn_id()
                || !terminal.intermediate_observations().is_empty()
                || !terminal.resource_evidence().is_empty()
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            if dispatch_state == ManagedWorkerDispatchState::NoWorkerThread
                || retained_state
                    .thread_id()
                    .is_some_and(|retained| retained != thread.thread_id())
                || retained_state
                    .turn_id()
                    .is_some_and(|retained| Some(retained) != turn.turn_id())
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            persist_observation(repository, &request.binding, attempt, &thread)?;
            persist_observation(repository, &request.binding, attempt, &turn)?;
            let terminal = persist_observation(
                repository,
                &request.binding,
                attempt,
                terminal.observation(),
            )?;
            Ok(ManagedPrestartRestartOutcome::FailedStart {
                terminal: Box::new(terminal),
            })
        }
    }
}

/// Continues a typed restart proof into a new provider effect. Recovery and
/// continuation are intentionally separate so stale authority can still
/// reconcile or close retained work without ever authorizing a new thread or
/// turn. Every provider mutation is preceded by a fresh Writer/fence guard.
///
/// # Errors
///
/// Rejects substituted attempt state, stale execution/model/Writer authority,
/// changed baseline evidence, unsafe dispatch replay, or provider mismatch.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn continue_managed_prestart_on_restart<R, W, G>(
    request: &ManagedAttemptRequest,
    attempt: &VerifiedWorkerAttemptRecord,
    retained_state: &WorkerAttemptState,
    proof: &ManagedPrestartNoEffectProof,
    repository: &mut R,
    worker: &mut W,
    provider_guard: &mut G,
) -> Result<ManagedPrestartRestartOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
    G: ManagedProviderEffectGuardPort,
{
    ensure_claim(request, attempt)?;
    ensure_attempt_state(attempt, retained_state)?;
    let baseline = request
        .predispatch_baseline()
        .ok_or(ManagedAttemptOrchestratorError::PredispatchBaselineRequired)?;

    repository
        .assert_execution_authority_current(&request.binding, &request.authority_digest)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    ensure_restart_model_available(worker, &request.packet)?;
    let receipt = repository
        .record_artifact(&request.binding, attempt, baseline)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    if !receipt.matches(baseline) {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }

    let mut state = retained_state.clone();
    let (thread_record, thread_id) = match proof {
        ManagedPrestartNoEffectProof::PendingReservation => {
            return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
        }
        ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
            worker_thread_claimed: true,
        } => {
            return Ok(ManagedPrestartRestartOutcome::ReconciliationRequired);
        }
        ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
            worker_thread_claimed: false,
        } => {
            if !matches!(
                state.phase(),
                WorkerAttemptPhase::Claimed | WorkerAttemptPhase::Dispatching
            ) || state.thread_id().is_some()
                || state.turn_id().is_some()
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            if state.phase() == WorkerAttemptPhase::Claimed {
                state
                    .begin_dispatch()
                    .map_err(ManagedAttemptOrchestratorError::Domain)?;
            }
            if repository
                .claim_worker_thread_dispatch(&request.binding, attempt)
                .map_err(ManagedAttemptOrchestratorError::Repository)?
                != ManagedWorkerThreadDispatchDisposition::Claimed
            {
                return Ok(ManagedPrestartRestartOutcome::ReconciliationRequired);
            }
            if request.execution_preflight().is_some() {
                let lifecycle = worker
                    .prepare_provider_dispatch(attempt, &request.packet)
                    .map_err(ManagedAttemptOrchestratorError::Worker)?;
                persist_provider_lifecycle(
                    repository,
                    &request.binding,
                    attempt,
                    &lifecycle,
                    "lattice.wsl2-provider-subtree-marker/1.0",
                )?;
            }
            provider_guard
                .assert_provider_effect_writer_current(&request.binding, attempt)
                .map_err(ManagedAttemptOrchestratorError::ProviderEffectGuard)?;
            let thread = worker
                .start_thread(attempt, &request.packet)
                .map_err(ManagedAttemptOrchestratorError::Worker)?;
            apply_expected_start(
                &mut state,
                &thread,
                WorkerObservationKind::ThreadAccepted,
                WorkerAttemptPhase::Accepted,
            )?;
            let record = persist_observation(repository, &request.binding, attempt, &thread)?;
            let thread_id = thread.thread_id().to_owned();
            (record, thread_id)
        }
        ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn { thread, .. } => {
            if thread.task_ref() != request.binding.task_ref()
                || thread.binding_digest() != request.binding.binding_digest()
                || thread.attempt_id() != attempt.attempt_id()
                || thread.attempt_number() != attempt.attempt_number()
                || thread.kind() != WorkerObservationKind::ThreadAccepted
                || thread.turn_id().is_some()
                || state.turn_id().is_some()
                || state
                    .thread_id()
                    .is_some_and(|retained| retained != thread.thread_id())
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            if state.phase() == WorkerAttemptPhase::Claimed {
                state
                    .begin_dispatch()
                    .map_err(ManagedAttemptOrchestratorError::Domain)?;
            }
            if state.thread_id().is_none() {
                state
                    .apply_start(StartObservation::ThreadStartAccepted {
                        thread_id: thread.thread_id().to_owned(),
                    })
                    .map_err(ManagedAttemptOrchestratorError::Domain)?;
            }
            ((**thread).clone(), thread.thread_id().to_owned())
        }
    };

    let turn_disposition = repository
        .claim_worker_turn_dispatch(&request.binding, attempt, &thread_record)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    if let ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn {
        worker_turn_claimed,
        ..
    } = proof
    {
        if *worker_turn_claimed
            && turn_disposition != ManagedWorkerTurnDispatchDisposition::ExactReplay
        {
            return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
        }
    } else if turn_disposition == ManagedWorkerTurnDispatchDisposition::ExactReplay {
        return Ok(ManagedPrestartRestartOutcome::ReconciliationRequired);
    }

    provider_guard
        .assert_provider_effect_writer_current(&request.binding, attempt)
        .map_err(ManagedAttemptOrchestratorError::ProviderEffectGuard)?;
    let turn = worker
        .start_turn(attempt, &thread_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    apply_expected_start(
        &mut state,
        &turn,
        WorkerObservationKind::TurnAccepted,
        WorkerAttemptPhase::Starting,
    )?;
    persist_observation(repository, &request.binding, attempt, &turn)?;
    Ok(ManagedPrestartRestartOutcome::Starting(Box::new(
        ManagedStartingAttempt {
            request: request.clone(),
            attempt: attempt.clone(),
            state,
        },
    )))
}

/// Durably closes a pending or prestart attempt only through a repository
/// implementation that can verify the typed proof against provider claims.
///
/// # Errors
///
/// Rejects a mismatched binding, malformed exact-empty proof, invalid blocker,
/// or any repository failure to prove and persist the exact closure.
pub fn close_managed_prestart_without_provider_effect<R>(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    proof: &ManagedPrestartNoEffectProof,
    blocker_code: &'static str,
    repository: &mut R,
) -> Result<ManagedPrestartClosureDisposition, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
{
    if attempt.task_ref() != binding.task_ref()
        || attempt.binding_digest() != binding.binding_digest()
        || blocker_code.trim().is_empty()
    {
        return Err(ManagedAttemptOrchestratorError::BindingMismatch);
    }
    if matches!(
        proof,
        ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
            worker_thread_claimed: true
        }
    ) {
        return Err(ManagedAttemptOrchestratorError::DispatchReconciliationRequired);
    }
    if let ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn { thread, .. } = proof
        && (thread.task_ref() != binding.task_ref()
            || thread.binding_digest() != binding.binding_digest()
            || thread.attempt_id() != attempt.attempt_id()
            || thread.attempt_number() != attempt.attempt_number()
            || thread.kind() != WorkerObservationKind::ThreadAccepted
            || thread.turn_id().is_some())
    {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    repository
        .close_prestart_without_provider_effect(binding, attempt, proof, blocker_code)
        .map_err(ManagedAttemptOrchestratorError::Repository)
}

fn ensure_restart_model_available<W: ManagedCodexWorkerPort>(
    worker: &mut W,
    packet: &AttemptPacketIdentity,
) -> Result<(), ManagedAttemptOrchestratorError> {
    match worker
        .model_availability(packet.model_selection())
        .map_err(ManagedAttemptOrchestratorError::Worker)?
    {
        ManagedModelAvailability::Available => Ok(()),
        ManagedModelAvailability::Unavailable { code } => {
            Err(ManagedAttemptOrchestratorError::ModelUnavailable { code })
        }
    }
}

/// Consumes an accepted-start token and returns an executing token only after
/// the exact matching in-progress notification is durably recorded.
///
/// # Errors
///
/// Fails closed unless the exact retained thread/turn reports an in-progress
/// `turn/started` observation that is durably recorded without substitution.
pub fn confirm_managed_exact_start<R, W>(
    mut starting: ManagedStartingAttempt,
    repository: &mut R,
    worker: &mut W,
) -> Result<ManagedExecutingAttempt, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
{
    let request = &starting.request;
    let attempt = &starting.attempt;
    let thread_id = starting
        .state
        .thread_id()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?
        .to_owned();
    let turn_id = starting
        .state
        .turn_id()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?
        .to_owned();
    let exact_started = worker
        .wait_exact_started(attempt, &thread_id, &turn_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    if exact_started.kind() != WorkerObservationKind::TurnStarted
        || exact_started.thread_id() != thread_id
        || exact_started.turn_id() != Some(turn_id.as_str())
    {
        return Err(ManagedAttemptOrchestratorError::ExactStartNotConfirmed);
    }
    let start = exact_started
        .start_observation()
        .ok_or(ManagedAttemptOrchestratorError::ExactStartNotConfirmed)?;
    if starting
        .state
        .apply_start(start)
        .map_err(ManagedAttemptOrchestratorError::Domain)?
        != StartGateDecision::Applied(WorkerAttemptPhase::Executing)
        || !starting.state.is_real_running()
    {
        return Err(ManagedAttemptOrchestratorError::ExactStartNotConfirmed);
    }
    persist_observation(repository, &request.binding, attempt, &exact_started)?;

    Ok(ManagedExecutingAttempt {
        request: starting.request,
        attempt: starting.attempt,
        state: starting.state,
    })
}

/// Consumes only an exact-start-proved token and durably records the exact
/// worker terminal. It deliberately performs no Git, verification, or review
/// effect so the caller can persist `EXECUTING -> VERIFYING` first.
///
/// # Errors
///
/// Fails closed on provider, terminal, or durable observation mismatch.
pub fn finish_managed_execution<R, W>(
    mut executing: ManagedExecutingAttempt,
    repository: &mut R,
    worker: &mut W,
) -> Result<ManagedTerminalAttempt, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
{
    let request = &executing.request;
    let attempt = &executing.attempt;
    let thread_id = executing
        .state
        .thread_id()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?
        .to_owned();
    let turn_id = executing
        .state
        .turn_id()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?
        .to_owned();
    let terminal_candidate = loop {
        if let Some(candidate) = persist_next_execution_event(
            &request.binding,
            attempt,
            &thread_id,
            &turn_id,
            repository,
            worker,
        )? {
            break candidate;
        }
    };
    let terminal_observation = terminal_candidate.observation();
    let terminal_kind = terminal_observation
        .terminal_kind()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?;
    executing
        .state
        .record_terminal(
            terminal_observation.thread_id(),
            terminal_observation
                .turn_id()
                .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?,
            terminal_kind,
            &format!(
                "evidence:sha256:{}",
                terminal_observation.evidence_digest().as_str()
            ),
        )
        .map_err(ManagedAttemptOrchestratorError::Domain)?;
    let terminal =
        persist_observation(repository, &request.binding, attempt, terminal_observation)?;
    if terminal_kind != WorkerTerminal::Completed {
        return Err(ManagedAttemptOrchestratorError::WorkerTerminal(
            terminal_kind,
        ));
    }

    Ok(ManagedTerminalAttempt {
        request: executing.request,
        attempt: executing.attempt,
        terminal,
    })
}

/// Prepares and durably stores the mechanical Git/check evidence. A failed
/// mechanical gate is recorded as a failed verification while the Task stays
/// `VERIFYING`; only a passing preparation yields a review-ready token.
///
/// # Errors
///
/// Fails closed on artifact mismatch or mechanical verification failure.
pub fn prepare_managed_review<R, V>(
    terminal: ManagedTerminalAttempt,
    repository: &mut R,
    verifier: &mut V,
) -> Result<ManagedReviewReadyAttempt, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    V: ManagedVerificationPort,
{
    let request = &terminal.request;
    let attempt = &terminal.attempt;
    let terminal_record = &terminal.terminal;

    let preparation = match verifier.prepare(&request.binding, attempt, terminal_record) {
        Ok(preparation) => preparation,
        Err(failure) => {
            let failure_evidence = verifier
                .preparation_failure_evidence(&request.binding, attempt, terminal_record, &failure)
                .map_err(ManagedAttemptOrchestratorError::Verification)?;
            if let Some(evidence) = failure_evidence {
                if evidence.task_ref() != request.binding.task_ref()
                    || u64::from(evidence.attempt()) != attempt.attempt_number()
                    || evidence.kind() != ManagedEvidenceKind::VerificationResult
                {
                    return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
                }
                let receipt = repository
                    .record_artifact(&request.binding, attempt, &evidence)
                    .map_err(ManagedAttemptOrchestratorError::Repository)?;
                if !receipt.matches(&evidence) {
                    return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
                }
            }
            return Err(ManagedAttemptOrchestratorError::Verification(failure));
        }
    };
    let artifact_receipt = repository
        .record_artifact(&request.binding, attempt, preparation.evidence())
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    if !artifact_receipt.matches(preparation.evidence()) {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    for evidence in preparation.supplemental_evidence() {
        let receipt = repository
            .record_artifact(&request.binding, attempt, evidence)
            .map_err(ManagedAttemptOrchestratorError::Repository)?;
        if !receipt.matches(evidence) {
            return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
        }
    }

    if preparation.mechanical_outcome() == VerificationOutcome::Failed {
        let verification_evidence = verifier
            .verify(
                &request.binding,
                attempt,
                terminal_record,
                preparation.request(),
            )
            .map_err(ManagedAttemptOrchestratorError::Verification)?;
        if verification_evidence.outcome() != VerificationOutcome::Failed
            || verification_evidence.request() != preparation.request()
        {
            return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
        }
        let verification = repository
            .record_verification(&request.binding, attempt, &verification_evidence)
            .map_err(ManagedAttemptOrchestratorError::Repository)?;
        ensure_verification(
            &request.binding,
            attempt,
            &verification_evidence,
            &verification,
        )?;
        return Err(ManagedAttemptOrchestratorError::VerificationFailed(
            Box::new(verification),
        ));
    }

    Ok(ManagedReviewReadyAttempt {
        terminal,
        preparation,
    })
}

struct RepositoryReviewEvidenceSink<'repository, 'guard, R> {
    repository: &'repository mut R,
    binding: VerifiedTaskExecutionBinding,
    attempt: VerifiedWorkerAttemptRecord,
    request: ManagedVerificationRequest,
    provider_guard: Option<&'guard mut dyn ManagedProviderEffectGuardPort>,
}

impl<R> ManagedReviewEvidenceSink for RepositoryReviewEvidenceSink<'_, '_, R>
where
    R: ManagedForemanRepositoryPort,
{
    fn record(
        &mut self,
        evidence: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedArtifactReceipt> {
        if evidence.task_ref() != self.binding.task_ref()
            || u64::from(evidence.attempt()) != self.attempt.attempt_number()
            || !matches!(
                evidence.kind(),
                ManagedEvidenceKind::WorkerLifecycle
                    | ManagedEvidenceKind::ReviewResult
                    | ManagedEvidenceKind::ResourceObservation
            )
        {
            return Err(ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "LATTICE_MANAGED_REVIEW_EVIDENCE_REJECTED",
            ));
        }
        let receipt = self
            .repository
            .record_artifact(&self.binding, &self.attempt, evidence)?;
        if !receipt.matches(evidence) {
            return Err(ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "LATTICE_MANAGED_REVIEW_EVIDENCE_REJECTED",
            ));
        }
        Ok(receipt)
    }

    fn authorize_provider_dispatch(
        &mut self,
        open_lifecycle: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<()> {
        let payload: serde_json::Value =
            serde_json::from_slice(open_lifecycle.bytes()).map_err(|_| {
                ManagedPortError::new(
                    ManagedPortErrorKind::Known,
                    "LATTICE_MANAGED_REVIEW_PROVIDER_DISPATCH_REJECTED",
                )
            })?;
        if open_lifecycle.task_ref() != self.binding.task_ref()
            || u64::from(open_lifecycle.attempt()) != self.attempt.attempt_number()
            || open_lifecycle.kind() != ManagedEvidenceKind::WorkerLifecycle
            || open_lifecycle.payload_schema() != "lattice.wsl2-provider-subtree-marker/1.0"
            || payload.get("status").and_then(serde_json::Value::as_str) != Some("OPEN")
            || payload.get("role").and_then(serde_json::Value::as_str) != Some("REVIEWER")
            || payload
                .get("model_call_identity")
                .and_then(serde_json::Value::as_str)
                .is_none()
        {
            return Err(ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "LATTICE_MANAGED_REVIEW_PROVIDER_DISPATCH_REJECTED",
            ));
        }
        self.provider_guard
            .as_deref_mut()
            .ok_or_else(|| {
                ManagedPortError::new(
                    ManagedPortErrorKind::ReconcileRequired,
                    "LATTICE_MANAGED_REVIEW_PROVIDER_WRITER_GUARD_REQUIRED",
                )
            })?
            .assert_provider_effect_writer_current(&self.binding, &self.attempt)
    }

    fn authorize_turn_start(
        &mut self,
        thread_lifecycle: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedReviewDispatchDisposition> {
        if thread_lifecycle.task_ref() != self.binding.task_ref()
            || u64::from(thread_lifecycle.attempt()) != self.attempt.attempt_number()
            || thread_lifecycle.kind() != ManagedEvidenceKind::WorkerLifecycle
            || thread_lifecycle.payload_schema() != "lattice.managed-review-lifecycle/1.0"
        {
            return Err(ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "LATTICE_MANAGED_REVIEW_TURN_DISPATCH_REJECTED",
            ));
        }
        let disposition = self.repository.claim_review_turn_dispatch(
            &self.binding,
            &self.attempt,
            &self.request,
            thread_lifecycle,
        )?;
        if disposition == ManagedReviewDispatchDisposition::Claimed {
            self.provider_guard
                .as_deref_mut()
                .ok_or_else(|| {
                    ManagedPortError::new(
                        ManagedPortErrorKind::ReconcileRequired,
                        "LATTICE_MANAGED_REVIEW_TURN_WRITER_GUARD_REQUIRED",
                    )
                })?
                .assert_provider_effect_writer_current(&self.binding, &self.attempt)?;
        }
        Ok(disposition)
    }
}

/// Starts the independent reviewer from a review-ready token, durably sinks
/// every bounded reviewer lifecycle artifact, then records the final combined
/// verification result.
///
/// # Errors
///
/// Fails closed on reviewer lifecycle, artifact receipt, or verification
/// mismatch. Any reviewer finding or malformed result is a failed verification.
pub fn finish_managed_review<R, V, G>(
    reviewing: ManagedReviewReadyAttempt,
    repository: &mut R,
    verifier: &mut V,
    provider_guard: &mut G,
) -> Result<ManagedAttemptOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    V: ManagedVerificationPort,
    G: ManagedProviderEffectGuardPort,
{
    let claimed = claim_managed_review(reviewing, repository)?;
    finish_claimed_managed_review(claimed, repository, verifier, provider_guard)
}

/// Re-enters a previously claimed semantic review through a verifier that was
/// explicitly configured for retained/discovery-only reconciliation. This is
/// the sole path that may consume an exact replay of the durable review claim.
///
/// # Errors
///
/// Fails closed under the same conditions as [`finish_managed_review`].
pub fn finish_managed_review_on_restart<R, V>(
    reviewing: ManagedReviewReadyAttempt,
    repository: &mut R,
    verifier: &mut V,
) -> Result<ManagedAttemptOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    V: ManagedVerificationPort,
{
    let claimed = claim_managed_review(reviewing, repository)?;
    finish_replayed_managed_review(claimed, repository, verifier)
}

/// Atomically claims the one `REVIEW_THREAD` provider effect after mechanical
/// evidence is durable. No reviewer call is made by this stage.
///
/// # Errors
///
/// Fails closed on a missing, substituted, or unavailable durable claim.
pub fn claim_managed_review<R>(
    reviewing: ManagedReviewReadyAttempt,
    repository: &mut R,
) -> Result<ManagedClaimedReviewAttempt, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
{
    let request = &reviewing.terminal.request;
    let attempt = &reviewing.terminal.attempt;
    let terminal = &reviewing.terminal.terminal;
    let disposition = repository
        .claim_review_dispatch(
            &request.binding,
            attempt,
            terminal,
            reviewing.preparation.request(),
        )
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    Ok(ManagedClaimedReviewAttempt {
        reviewing,
        disposition,
    })
}

/// Executes a newly claimed review. Exact replay is never fresh-provider
/// authority and is rejected before the verifier can run.
///
/// # Errors
///
/// Fails closed on a replay disposition or reviewer/verification mismatch.
pub fn finish_claimed_managed_review<R, V, G>(
    claimed: ManagedClaimedReviewAttempt,
    repository: &mut R,
    verifier: &mut V,
    provider_guard: &mut G,
) -> Result<ManagedAttemptOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    V: ManagedVerificationPort,
    G: ManagedProviderEffectGuardPort,
{
    if claimed.disposition != ManagedReviewDispatchDisposition::Claimed {
        return Err(ManagedAttemptOrchestratorError::ReviewDispatchReconciliationRequired);
    }
    finish_managed_review_inner(claimed, repository, verifier, Some(provider_guard))
}

/// Executes only an exact replay through a composition-configured
/// discovery/retained reviewer. A newly claimed dispatch cannot enter this
/// restart-only path.
///
/// # Errors
///
/// Fails closed on a fresh disposition or reviewer/verification mismatch.
pub fn finish_replayed_managed_review<R, V>(
    claimed: ManagedClaimedReviewAttempt,
    repository: &mut R,
    verifier: &mut V,
) -> Result<ManagedAttemptOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    V: ManagedVerificationPort,
{
    if claimed.disposition != ManagedReviewDispatchDisposition::ExactReplay {
        return Err(ManagedAttemptOrchestratorError::ReviewDispatchReconciliationRequired);
    }
    finish_managed_review_inner(claimed, repository, verifier, None)
}

/// Re-enters one exact durable review claim while retaining the current
/// provider-effect writer authority. This authorizes only continuation of the
/// already claimed review; it does not claim another review/model call.
///
/// # Errors
///
/// Fails closed unless the review dispatch is an exact replay and the injected
/// writer guard remains current before every reviewer provider boundary.
pub fn finish_replayed_managed_review_with_provider_guard<R, V, G>(
    claimed: ManagedClaimedReviewAttempt,
    repository: &mut R,
    verifier: &mut V,
    provider_guard: &mut G,
) -> Result<ManagedAttemptOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    V: ManagedVerificationPort,
    G: ManagedProviderEffectGuardPort,
{
    if claimed.disposition != ManagedReviewDispatchDisposition::ExactReplay {
        return Err(ManagedAttemptOrchestratorError::ReviewDispatchReconciliationRequired);
    }
    finish_managed_review_inner(claimed, repository, verifier, Some(provider_guard))
}

fn finish_managed_review_inner<R, V>(
    claimed: ManagedClaimedReviewAttempt,
    repository: &mut R,
    verifier: &mut V,
    mut provider_guard: Option<&mut dyn ManagedProviderEffectGuardPort>,
) -> Result<ManagedAttemptOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    V: ManagedVerificationPort,
{
    let reviewing = claimed.reviewing;
    let request = &reviewing.terminal.request;
    let attempt = &reviewing.terminal.attempt;
    let terminal = &reviewing.terminal.terminal;
    let verification_request = reviewing.preparation.request();
    if let Some(guard) = provider_guard.as_deref_mut() {
        guard
            .assert_provider_effect_writer_current(&request.binding, attempt)
            .map_err(ManagedAttemptOrchestratorError::ProviderEffectGuard)?;
    }
    {
        let mut sink = RepositoryReviewEvidenceSink {
            repository,
            binding: request.binding.clone(),
            attempt: attempt.clone(),
            request: verification_request.clone(),
            provider_guard,
        };
        verifier
            .review(
                &request.binding,
                attempt,
                terminal,
                verification_request,
                &mut sink,
            )
            .map_err(ManagedAttemptOrchestratorError::Verification)?;
    }
    let verification_evidence = verifier
        .verify(&request.binding, attempt, terminal, verification_request)
        .map_err(ManagedAttemptOrchestratorError::Verification)?;
    if verification_evidence.request() != verification_request {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    let verification = repository
        .record_verification(&request.binding, attempt, &verification_evidence)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    ensure_verification(
        &request.binding,
        attempt,
        &verification_evidence,
        &verification,
    )?;
    if verification.outcome() == VerificationOutcome::Failed {
        return Err(ManagedAttemptOrchestratorError::VerificationFailed(
            Box::new(verification),
        ));
    }

    Ok(ManagedAttemptOutcome {
        attempt: reviewing.terminal.attempt,
        terminal: reviewing.terminal.terminal,
        verification,
        target: ManagedAttemptTarget::AwaitingMergeApproval,
    })
}

/// Compatibility wrapper for callers that do not own Task lifecycle state.
/// Production workflow composition uses the three staged functions above so
/// `PostgreSQL` transitions bracket the mechanical and reviewer effects.
///
/// # Errors
///
/// Returns a managed-attempt error when exact worker completion, mechanical
/// verification, reviewer evidence, or final verification cannot be proven.
pub fn finish_managed_attempt<R, W, V, G>(
    executing: ManagedExecutingAttempt,
    repository: &mut R,
    worker: &mut W,
    verifier: &mut V,
    provider_guard: &mut G,
) -> Result<ManagedAttemptOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
    V: ManagedVerificationPort,
    G: ManagedProviderEffectGuardPort,
{
    let terminal = finish_managed_execution(executing, repository, worker)?;
    let reviewing = prepare_managed_review(terminal, repository, verifier)?;
    finish_managed_review(reviewing, repository, verifier, provider_guard)
}

/// Result of reconciling one retained attempt after a fresh process start.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedRestartOutcome {
    /// A claimed attempt has no provider-effect intent and can be handed to a
    /// retained-claim dispatch path without creating a new claim.
    DispatchRetainedClaim,
    /// Dispatch intent exists but no provider identity was retained; opening a
    /// replacement thread would be unsafe.
    BlockUncertainDispatch,
    /// The exact retained turn was read, resumed, and reconciled as active.
    ExactActive,
    /// An exact retained terminal was found and durably recorded.
    ExactTerminal {
        terminal: WorkerTerminal,
        evidence: Box<VerifiedWorkerObservationRecord>,
    },
    /// The already terminal local projection remains terminal; no provider call
    /// or duplicate observation is needed.
    PreserveTerminal,
    /// Retained IDs could not be reconciled; no new thread or turn was started.
    ReconciliationRequired,
}

/// Reconciles retained provider identity without invoking either start RPC.
///
/// For a retained thread/turn the fixed order is read thread -> read exact
/// turn -> resume exact turn -> reconcile exact turn. Any exact terminal
/// short-circuits the remaining steps. An unresolved step fails closed.
///
/// # Errors
///
/// Rejects cross-attempt/provider observations or durable append failures.
pub fn reconcile_managed_attempt_on_restart<R, W>(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    state: &WorkerAttemptState,
    repository: &mut R,
    worker: &mut W,
) -> Result<ManagedRestartOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
{
    ensure_attempt_state(attempt, state)?;
    match restart_reconciliation_decision(state).map_err(ManagedAttemptOrchestratorError::Domain)? {
        RestartDecision::DispatchUnsentAttempt => {
            return Ok(ManagedRestartOutcome::DispatchRetainedClaim);
        }
        RestartDecision::BlockUncertainDispatch => {
            return Ok(ManagedRestartOutcome::BlockUncertainDispatch);
        }
        RestartDecision::PreserveTerminal => {
            return Ok(ManagedRestartOutcome::PreserveTerminal);
        }
        RestartDecision::ReadExactThread { .. } | RestartDecision::ReadResumeExactTurn { .. } => {}
    }

    let thread_id = state
        .thread_id()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?;
    let read_thread = worker
        .read_exact_thread(attempt, thread_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    let active = match persist_reconciliation(
        binding,
        attempt,
        thread_id,
        state.turn_id(),
        read_thread,
        repository,
    )? {
        ReconciliationStep::Active(observation) => observation,
        ReconciliationStep::Terminal(outcome) => return Ok(outcome),
        ReconciliationStep::Unresolved => {
            return Ok(ManagedRestartOutcome::ReconciliationRequired);
        }
    };
    let turn_id = state
        .turn_id()
        .or_else(|| active.turn_id())
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?
        .to_owned();

    let read_turn = worker
        .read_exact_turn(attempt, thread_id, &turn_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    match persist_reconciliation(
        binding,
        attempt,
        thread_id,
        Some(&turn_id),
        read_turn,
        repository,
    )? {
        ReconciliationStep::Active(_) => {}
        ReconciliationStep::Terminal(outcome) => return Ok(outcome),
        ReconciliationStep::Unresolved => {
            return Ok(ManagedRestartOutcome::ReconciliationRequired);
        }
    }

    let resumed = worker
        .resume_exact_turn(attempt, thread_id, &turn_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    match persist_reconciliation(
        binding,
        attempt,
        thread_id,
        Some(&turn_id),
        resumed,
        repository,
    )? {
        ReconciliationStep::Active(_) => {}
        ReconciliationStep::Terminal(outcome) => return Ok(outcome),
        ReconciliationStep::Unresolved => {
            return Ok(ManagedRestartOutcome::ReconciliationRequired);
        }
    }

    let reconciled = worker
        .reconcile_exact_turn(attempt, thread_id, &turn_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    match persist_reconciliation(
        binding,
        attempt,
        thread_id,
        Some(&turn_id),
        reconciled,
        repository,
    )? {
        ReconciliationStep::Active(_) => {}
        ReconciliationStep::Terminal(outcome) => return Ok(outcome),
        ReconciliationStep::Unresolved => {
            return Ok(ManagedRestartOutcome::ReconciliationRequired);
        }
    }
    Ok(ManagedRestartOutcome::ExactActive)
}

enum ReconciliationStep {
    Active(ManagedWorkerObservation),
    Terminal(ManagedRestartOutcome),
    Unresolved,
}

fn persist_reconciliation<R: ManagedForemanRepositoryPort>(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    expected_thread: &str,
    expected_turn: Option<&str>,
    result: ManagedWorkerReconciliation,
    repository: &mut R,
) -> Result<ReconciliationStep, ManagedAttemptOrchestratorError> {
    match result {
        ManagedWorkerReconciliation::ExactActive(observation) => {
            if observation.kind() != WorkerObservationKind::Reconciled
                || observation.thread_id() != expected_thread
                || expected_turn.is_some_and(|turn| observation.turn_id() != Some(turn))
                || observation.turn_id().is_none()
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            persist_observation(repository, binding, attempt, &observation)?;
            Ok(ReconciliationStep::Active(observation))
        }
        ManagedWorkerReconciliation::ExactTerminal(candidate) => {
            let observation = candidate.observation();
            if observation.thread_id() != expected_thread
                || expected_turn.is_some_and(|turn| observation.turn_id() != Some(turn))
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            for progress in candidate.intermediate_observations() {
                if progress.kind() != WorkerObservationKind::MeaningfulProgress
                    || progress.thread_id() != expected_thread
                    || expected_turn.is_some_and(|turn| progress.turn_id() != Some(turn))
                {
                    return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
                }
                persist_observation(repository, binding, attempt, progress)?;
            }
            for resource in candidate.resource_evidence() {
                if resource.task_ref() != binding.task_ref()
                    || u64::from(resource.attempt()) != attempt.attempt_number()
                    || resource.kind() != ManagedEvidenceKind::ResourceObservation
                {
                    return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
                }
                let receipt = repository
                    .record_artifact(binding, attempt, resource)
                    .map_err(ManagedAttemptOrchestratorError::Repository)?;
                if !receipt.matches(resource) {
                    return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
                }
            }
            let terminal = observation
                .terminal_kind()
                .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?;
            let evidence = persist_observation(repository, binding, attempt, observation)?;
            Ok(ReconciliationStep::Terminal(
                ManagedRestartOutcome::ExactTerminal {
                    terminal,
                    evidence: Box::new(evidence),
                },
            ))
        }
        ManagedWorkerReconciliation::Unresolved => Ok(ReconciliationStep::Unresolved),
    }
}

/// Closed result of one watchdog-driven recovery pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedStallOutcome {
    Healthy,
    RecoveredExactActive {
        reason: StallReason,
    },
    TerminalRecovered {
        reason: StallReason,
        terminal: WorkerTerminal,
    },
    Retry {
        reason: StallReason,
        decision: RetryDecision,
    },
}

/// Applies the foreman-state watchdog classifier, reconciles first, and only
/// then interrupts the exact retained turn when reconciliation is exhausted.
/// Retry is returned only after the exact interrupt terminal is durably stored.
///
/// # Errors
///
/// Rejects substituted state/budget/observation identity or any uncertain
/// provider/repository effect. It never starts a replacement attempt.
#[allow(clippy::too_many_arguments)]
pub fn handle_managed_attempt_stall<R, W>(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    state: &mut WorkerAttemptState,
    budget: &WorkerBudget,
    last_progress: &MeaningfulProgress,
    watchdog: &AttemptWatchdogObservation,
    repairable: bool,
    repository: &mut R,
    worker: &mut W,
) -> Result<ManagedStallOutcome, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
{
    ensure_attempt_state(attempt, state)?;
    let classification = classify_attempt_stall(state, budget, last_progress, watchdog)
        .map_err(ManagedAttemptOrchestratorError::Domain)?;
    let reason = match classification {
        StallClassification::Healthy => return Ok(ManagedStallOutcome::Healthy),
        StallClassification::ReconcileFirst(reason) | StallClassification::Stalled(reason) => {
            reason
        }
    };
    let thread_id = state
        .thread_id()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?
        .to_owned();
    let turn_id = state
        .turn_id()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?
        .to_owned();

    if state.phase() == WorkerAttemptPhase::Executing {
        state
            .begin_reconciliation()
            .map_err(ManagedAttemptOrchestratorError::Domain)?;
    }
    let reconciled = worker
        .reconcile_exact_turn(attempt, &thread_id, &turn_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    match reconciled {
        ManagedWorkerReconciliation::ExactActive(observation) => {
            if observation.kind() != WorkerObservationKind::Reconciled
                || observation.thread_id() != thread_id
                || observation.turn_id() != Some(turn_id.as_str())
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            persist_observation(repository, binding, attempt, &observation)?;
            return Ok(ManagedStallOutcome::RecoveredExactActive { reason });
        }
        ManagedWorkerReconciliation::ExactTerminal(candidate) => {
            return finish_stall_terminal(
                binding, attempt, state, budget, reason, repairable, &candidate, repository,
            );
        }
        ManagedWorkerReconciliation::Unresolved => {}
    }

    state
        .begin_interrupt()
        .map_err(ManagedAttemptOrchestratorError::Domain)?;
    let interrupt = worker
        .interrupt_exact_turn(attempt, &thread_id, &turn_id)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    if interrupt.kind() != WorkerObservationKind::InterruptRequested
        || interrupt.thread_id() != thread_id
        || interrupt.turn_id() != Some(turn_id.as_str())
    {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    persist_observation(repository, binding, attempt, &interrupt)?;

    let terminal = loop {
        if let Some(candidate) = persist_next_execution_event(
            binding, attempt, &thread_id, &turn_id, repository, worker,
        )? {
            break candidate;
        }
    };
    finish_stall_terminal(
        binding, attempt, state, budget, reason, repairable, &terminal, repository,
    )
}

/// Polls and durably records exactly one execution event. `None` means the
/// event was non-terminal and the caller may poll again only after this
/// function returned success.
fn persist_next_execution_event<R, W>(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    expected_thread: &str,
    expected_turn: &str,
    repository: &mut R,
    worker: &mut W,
) -> Result<Option<ManagedTerminalCandidate>, ManagedAttemptOrchestratorError>
where
    R: ManagedForemanRepositoryPort,
    W: ManagedCodexWorkerPort,
{
    let event = worker
        .next_execution_event(attempt, expected_thread, expected_turn)
        .map_err(ManagedAttemptOrchestratorError::Worker)?;
    match event {
        ManagedWorkerExecutionEvent::Observation(observation) => {
            if !matches!(
                observation.kind(),
                WorkerObservationKind::MeaningfulProgress
                    | WorkerObservationKind::Heartbeat
                    | WorkerObservationKind::StallClassified
                    | WorkerObservationKind::InterruptRequested
                    | WorkerObservationKind::Reconciled
            ) || observation.thread_id() != expected_thread
                || observation.turn_id() != Some(expected_turn)
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            persist_observation(repository, binding, attempt, &observation)?;
            Ok(None)
        }
        ManagedWorkerExecutionEvent::ResourceObservation {
            observation,
            evidence,
        } => {
            if observation.kind() != WorkerObservationKind::MeaningfulProgress
                || observation.thread_id() != expected_thread
                || observation.turn_id() != Some(expected_turn)
                || evidence.task_ref() != binding.task_ref()
                || u64::from(evidence.attempt()) != attempt.attempt_number()
                || evidence.kind() != ManagedEvidenceKind::ResourceObservation
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            persist_observation(repository, binding, attempt, &observation)?;
            let receipt = repository
                .record_artifact(binding, attempt, &evidence)
                .map_err(ManagedAttemptOrchestratorError::Repository)?;
            if !receipt.matches(&evidence) {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            Ok(None)
        }
        ManagedWorkerExecutionEvent::LifecycleEvidence(evidence) => {
            persist_provider_lifecycle(
                repository,
                binding,
                attempt,
                &evidence,
                "lattice.wsl2-provider-subtree-receipt/1.0",
            )?;
            Ok(None)
        }
        ManagedWorkerExecutionEvent::Terminal(candidate) => {
            if !candidate.intermediate_observations().is_empty()
                || !candidate.resource_evidence().is_empty()
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            let observation = candidate.observation();
            if observation.thread_id() != expected_thread
                || observation.turn_id() != Some(expected_turn)
                || observation.terminal_kind().is_none()
            {
                return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
            }
            Ok(Some(candidate))
        }
    }
}

fn persist_provider_lifecycle<R: ManagedForemanRepositoryPort>(
    repository: &mut R,
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    evidence: &VerifiedManagedEvidence,
    expected_schema: &str,
) -> Result<(), ManagedAttemptOrchestratorError> {
    if evidence.task_ref() != binding.task_ref()
        || u64::from(evidence.attempt()) != attempt.attempt_number()
        || evidence.kind() != ManagedEvidenceKind::WorkerLifecycle
        || evidence.media_type() != "application/json"
        || evidence.payload_schema() != expected_schema
    {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    let receipt = repository
        .record_artifact(binding, attempt, evidence)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    if !receipt.matches(evidence) {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_stall_terminal<R: ManagedForemanRepositoryPort>(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    state: &mut WorkerAttemptState,
    budget: &WorkerBudget,
    reason: StallReason,
    repairable: bool,
    candidate: &ManagedTerminalCandidate,
    repository: &mut R,
) -> Result<ManagedStallOutcome, ManagedAttemptOrchestratorError> {
    let observation = candidate.observation();
    let terminal = observation
        .terminal_kind()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?;
    state
        .record_terminal(
            observation.thread_id(),
            observation
                .turn_id()
                .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?,
            terminal,
            &format!("evidence:sha256:{}", observation.evidence_digest().as_str()),
        )
        .map_err(ManagedAttemptOrchestratorError::Domain)?;
    persist_observation(repository, binding, attempt, observation)?;
    if terminal == WorkerTerminal::Completed {
        return Ok(ManagedStallOutcome::TerminalRecovered { reason, terminal });
    }
    let decision = decide_repair_retry(state, budget, repairable)
        .map_err(ManagedAttemptOrchestratorError::Domain)?;
    Ok(ManagedStallOutcome::Retry { reason, decision })
}

fn ensure_attempt_state(
    attempt: &VerifiedWorkerAttemptRecord,
    state: &WorkerAttemptState,
) -> Result<(), ManagedAttemptOrchestratorError> {
    if attempt.attempt_number() != u64::from(state.packet().attempt())
        || digest_pointer_payload(state.packet().digest(), "attempt-packet:sha256:")
            != Some(attempt.packet_digest().as_str())
    {
        return Err(ManagedAttemptOrchestratorError::ClaimMismatch);
    }
    Ok(())
}

fn ensure_claim(
    request: &ManagedAttemptRequest,
    attempt: &VerifiedWorkerAttemptRecord,
) -> Result<(), ManagedAttemptOrchestratorError> {
    let packet_digest = digest_pointer_payload(request.packet.digest(), "attempt-packet:sha256:");
    let model_reason_digest = digest_pointer_payload(
        request.packet.model_selection().digest(),
        "model-selection:sha256:",
    );
    if attempt.task_ref() != request.binding.task_ref()
        || attempt.successor_stream_id() != request.binding.successor_stream_id()
        || attempt.task_spec_digest() != request.binding.task_spec_digest()
        || attempt.binding_digest() != request.binding.binding_digest()
        || attempt.budget_digest() != request.binding.budget_digest()
        || attempt.attempt_number() != u64::from(request.packet.attempt())
        || attempt.model() != request.packet.model_selection().model()
        || attempt.reasoning() != request.packet.model_selection().reasoning()
        || attempt.model_reason() != request.packet.model_selection().reason()
        || attempt.writer_fence() != request.packet.writer_fence()
        || attempt.approval_receipt_digest() != &request.authority_digest
        || packet_digest != Some(attempt.packet_digest().as_str())
        || model_reason_digest != Some(attempt.model_reason_digest().as_str())
    {
        return Err(ManagedAttemptOrchestratorError::ClaimMismatch);
    }
    Ok(())
}

fn digest_pointer_payload<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let digest = value.strip_prefix(prefix)?;
    (digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(digest)
}

fn apply_expected_start(
    state: &mut WorkerAttemptState,
    observation: &ManagedWorkerObservation,
    expected_kind: WorkerObservationKind,
    expected_phase: WorkerAttemptPhase,
) -> Result<(), ManagedAttemptOrchestratorError> {
    if observation.kind() != expected_kind {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    let start = observation
        .start_observation()
        .ok_or(ManagedAttemptOrchestratorError::ObservationMismatch)?;
    if state
        .apply_start(start)
        .map_err(ManagedAttemptOrchestratorError::Domain)?
        != StartGateDecision::Applied(expected_phase)
    {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    Ok(())
}

fn persist_observation<R: ManagedForemanRepositoryPort>(
    repository: &mut R,
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    observation: &ManagedWorkerObservation,
) -> Result<VerifiedWorkerObservationRecord, ManagedAttemptOrchestratorError> {
    let record = repository
        .record_observation(binding, attempt, observation)
        .map_err(ManagedAttemptOrchestratorError::Repository)?;
    if record.task_ref() != binding.task_ref()
        || record.successor_stream_id() != binding.successor_stream_id()
        || record.binding_digest() != binding.binding_digest()
        || record.attempt_id() != attempt.attempt_id()
        || record.attempt_number() != attempt.attempt_number()
        || record.kind() != observation.kind()
        || record.thread_id() != observation.thread_id()
        || record.turn_id() != observation.turn_id()
        || record.app_server_generation() != observation.app_server_generation()
        || record.app_server_identity_digest() != observation.app_server_identity_digest()
        || record.evidence_digest() != observation.evidence_digest()
    {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    Ok(record)
}

fn ensure_verification(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    evidence: &lattice_ports::ManagedVerificationEvidence,
    record: &VerifiedTaskVerificationRecord,
) -> Result<(), ManagedAttemptOrchestratorError> {
    let request = evidence.request();
    if record.task_ref() != binding.task_ref()
        || record.successor_stream_id() != binding.successor_stream_id()
        || record.task_spec_digest() != binding.task_spec_digest()
        || record.binding_digest() != binding.binding_digest()
        || record.attempt_id() != attempt.attempt_id()
        || record.attempt_number() != attempt.attempt_number()
        || record.outcome() != evidence.outcome()
        || record.verification_profile_digest() != request.profile_identity()
        || record.base_commit_digest() != request.base_commit_digest()
        || record.result_commit_digest() != request.result_commit_digest()
        || record.tree_digest() != request.tree_digest()
        || record.diff_digest() != request.diff_digest()
        || record.result_digest() != evidence.result_digest()
        || record.evidence_artifact_digest() != request.evidence_artifact_digest()
        || record.review_digest() != evidence.review_digest()
    {
        return Err(ManagedAttemptOrchestratorError::ObservationMismatch);
    }
    Ok(())
}
