//! Isolated executable contract checkpoint for GitHub Issue #2 Phase 1.
//!
//! The evaluator consumes already-authoritative evidence projections and
//! returns a passive classification plus the next observation to make. It has
//! no I/O, clock, persistence, dispatch, repair, or process-control surface.

/// Owning TASK-038 coordination window for this checkpoint.
pub const TASK_CONTROL_SOURCE_THREAD_ID: &str = "019fe6e2-f8b9-7bb1-9d31-1e8992207d99";
/// Owning Issue #6/TASK-039 coordination window for this checkpoint.
pub const WORKER_OBSERVATION_SOURCE_THREAD_ID: &str = "019fe759-d4d6-7950-bf10-fbda7487fa9b";

/// Terminal task truth projected from the authoritative task owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalOutcome {
    Completed,
    Rejected,
    Blocked,
    Failed,
    Cancelled,
}

/// Coarse task truth required by the passive Phase 1 evaluator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskLifecycle {
    Pending,
    Running,
    Terminal(TerminalOutcome),
}

/// Provider-neutral worker activity projected by the observation owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerActivity {
    Active,
    WaitingExternal,
    Verifying,
    Terminal,
}

/// Verification evidence projected by the verification owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationState {
    Pending,
    Passed,
    Failed,
}

/// Authoritative task evidence reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskEvidence<'a> {
    pub task_ref: &'a str,
    pub attempt_ref: &'a str,
    pub revision: u64,
    pub observed_at: u64,
    pub lifecycle: TaskLifecycle,
}

/// Evidence that the task-control owner assigned one worker session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchEvidence<'a> {
    pub task_ref: &'a str,
    pub attempt_ref: &'a str,
    pub worker_session_ref: &'a str,
    pub observed_at: u64,
}

/// Read-only worker/session evidence projected by Issue #6/TASK-039.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerEvidence<'a> {
    pub task_ref: &'a str,
    pub attempt_ref: &'a str,
    pub worker_session_ref: &'a str,
    pub observed_at: u64,
    pub activity: WorkerActivity,
}

/// Read-only verification evidence for the exact task attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerificationEvidence<'a> {
    pub task_ref: &'a str,
    pub attempt_ref: &'a str,
    pub observed_at: u64,
    pub state: VerificationState,
    pub evidence_ref: Option<&'a str>,
}

/// One immutable observation snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PassiveSnapshot<'a> {
    /// Explicit caller-supplied observation clock. The evaluator reads no time.
    pub observed_at: u64,
    /// Maximum age of worker activity before it is classified as stalled.
    pub stale_after: u64,
    pub task: Option<TaskEvidence<'a>>,
    pub dispatch: Option<DispatchEvidence<'a>>,
    pub worker: Option<WorkerEvidence<'a>>,
    pub verification: Option<VerificationEvidence<'a>>,
}

/// Passive evidence classification. `Stalled` is not a task transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceClassification {
    Sleeping,
    Pending,
    Running,
    Terminal,
    Stalled,
}

/// The only representable Phase 1 next actions; every variant is observational.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NextObservation {
    Sleep,
    ObserveDispatch,
    ObserveTask,
    ObserveWorkerSession,
    ReadVerification,
    RecommendDurableProjection,
    Wait,
}

impl NextObservation {
    /// Complete Phase 1 surface. Every variant is passive and non-mutating.
    pub const ALL: [Self; 7] = [
        Self::Sleep,
        Self::ObserveDispatch,
        Self::ObserveTask,
        Self::ObserveWorkerSession,
        Self::ReadVerification,
        Self::RecommendDurableProjection,
        Self::Wait,
    ];

    /// Returns the stable machine-facing recommendation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sleep => "SLEEP",
            Self::ObserveDispatch => "OBSERVE_DISPATCH",
            Self::ObserveTask => "OBSERVE_TASK",
            Self::ObserveWorkerSession => "OBSERVE_WORKER_SESSION",
            Self::ReadVerification => "READ_VERIFICATION",
            Self::RecommendDurableProjection => "RECOMMEND_DURABLE_PROJECTION",
            Self::Wait => "WAIT",
        }
    }
}

/// Deterministic passive evaluation result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PassiveEvaluation {
    pub classification: EvidenceClassification,
    pub task_truth: Option<TaskLifecycle>,
    pub next: NextObservation,
    pub reason_code: &'static str,
}

/// Stable fail-closed contract faults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceFault {
    InvalidConfiguration,
    InvalidEvidenceReference,
    MissingTaskTruth,
    CrossTaskEvidence,
    CrossAttemptEvidence,
    WorkerWithoutDispatch,
    WorkerSessionMismatch,
    ImpossibleObservationOrder,
    FutureObservation,
    VerificationBeforeTerminal,
    MissingVerificationReference,
    TerminalWorkerConflict,
}

impl EvidenceFault {
    /// Returns the stable machine-facing fault code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "SUPERVISOR_INVALID_CONFIGURATION",
            Self::InvalidEvidenceReference => "SUPERVISOR_INVALID_EVIDENCE_REFERENCE",
            Self::MissingTaskTruth => "SUPERVISOR_TASK_TRUTH_MISSING",
            Self::CrossTaskEvidence => "SUPERVISOR_CROSS_TASK_EVIDENCE",
            Self::CrossAttemptEvidence => "SUPERVISOR_CROSS_ATTEMPT_EVIDENCE",
            Self::WorkerWithoutDispatch => "SUPERVISOR_WORKER_WITHOUT_DISPATCH",
            Self::WorkerSessionMismatch => "SUPERVISOR_WORKER_SESSION_MISMATCH",
            Self::ImpossibleObservationOrder => "SUPERVISOR_IMPOSSIBLE_OBSERVATION_ORDER",
            Self::FutureObservation => "SUPERVISOR_FUTURE_OBSERVATION",
            Self::VerificationBeforeTerminal => "SUPERVISOR_VERIFICATION_BEFORE_TERMINAL",
            Self::MissingVerificationReference => "SUPERVISOR_VERIFICATION_REFERENCE_MISSING",
            Self::TerminalWorkerConflict => "SUPERVISOR_TERMINAL_WORKER_CONFLICT",
        }
    }
}

/// Lifecycle of one upstream contract checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointState {
    InProgress,
    Stable,
}

/// Exact evidence supplied by one upstream contract owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UpstreamCheckpoint<'a> {
    pub source_thread_id: &'a str,
    pub state: CheckpointState,
    pub branch: Option<&'a str>,
    pub commit: Option<&'a str>,
    pub contract_version: Option<&'a str>,
    pub verification_digest: Option<&'a str>,
}

/// The two direct dependencies required before production composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DependencySnapshot<'a> {
    pub task_control: UpstreamCheckpoint<'a>,
    pub worker_observation: UpstreamCheckpoint<'a>,
}

/// Stable machine-facing dependency blockers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyBlocker {
    TaskControlCheckpointMissing,
    TaskControlCheckpointInvalid,
    WorkerObservationCheckpointMissing,
    WorkerObservationCheckpointInvalid,
}

impl DependencyBlocker {
    /// Returns the stable machine-facing blocker code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::TaskControlCheckpointMissing => "SUPERVISOR_TASK_CONTROL_CHECKPOINT_MISSING",
            Self::TaskControlCheckpointInvalid => "SUPERVISOR_TASK_CONTROL_CHECKPOINT_INVALID",
            Self::WorkerObservationCheckpointMissing => {
                "SUPERVISOR_WORKER_OBSERVATION_CHECKPOINT_MISSING"
            }
            Self::WorkerObservationCheckpointInvalid => {
                "SUPERVISOR_WORKER_OBSERVATION_CHECKPOINT_INVALID"
            }
        }
    }
}

/// Production-integration readiness for the passive Supervisor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyReadiness {
    Ready,
    Blocked(Vec<DependencyBlocker>),
}

/// Evaluates the exact upstream checkpoint gate in deterministic owner order.
#[must_use]
pub fn dependency_readiness(snapshot: &DependencySnapshot<'_>) -> DependencyReadiness {
    let mut blockers = Vec::with_capacity(2);
    match checkpoint_valid(&snapshot.task_control, TASK_CONTROL_SOURCE_THREAD_ID) {
        CheckpointValidity::Missing => {
            blockers.push(DependencyBlocker::TaskControlCheckpointMissing);
        }
        CheckpointValidity::Invalid => {
            blockers.push(DependencyBlocker::TaskControlCheckpointInvalid);
        }
        CheckpointValidity::Ready => {}
    }
    match checkpoint_valid(
        &snapshot.worker_observation,
        WORKER_OBSERVATION_SOURCE_THREAD_ID,
    ) {
        CheckpointValidity::Missing => {
            blockers.push(DependencyBlocker::WorkerObservationCheckpointMissing);
        }
        CheckpointValidity::Invalid => {
            blockers.push(DependencyBlocker::WorkerObservationCheckpointInvalid);
        }
        CheckpointValidity::Ready => {}
    }
    if blockers.is_empty() {
        DependencyReadiness::Ready
    } else {
        DependencyReadiness::Blocked(blockers)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckpointValidity {
    Missing,
    Invalid,
    Ready,
}

fn checkpoint_valid(
    checkpoint: &UpstreamCheckpoint<'_>,
    expected_source_thread_id: &str,
) -> CheckpointValidity {
    if checkpoint.source_thread_id != expected_source_thread_id
        || !valid_thread_id(checkpoint.source_thread_id)
    {
        return CheckpointValidity::Invalid;
    }
    if checkpoint.state == CheckpointState::InProgress {
        return CheckpointValidity::Missing;
    }
    let (Some(branch), Some(commit), Some(contract_version), Some(verification_digest)) = (
        checkpoint.branch,
        checkpoint.commit,
        checkpoint.contract_version,
        checkpoint.verification_digest,
    ) else {
        return CheckpointValidity::Invalid;
    };
    if !valid_ref(branch)
        || !valid_commit(commit)
        || !valid_ref(contract_version)
        || !valid_digest(verification_digest)
    {
        return CheckpointValidity::Invalid;
    }
    CheckpointValidity::Ready
}

/// Evaluates one immutable evidence snapshot without performing an effect.
///
/// # Errors
///
/// Returns a stable fault for malformed, cross-bound, future-dated, or
/// contradictory evidence.
pub fn evaluate(snapshot: &PassiveSnapshot<'_>) -> Result<PassiveEvaluation, EvidenceFault> {
    if snapshot.stale_after == 0 {
        return Err(EvidenceFault::InvalidConfiguration);
    }

    let Some(task) = snapshot.task else {
        if snapshot.dispatch.is_some()
            || snapshot.worker.is_some()
            || snapshot.verification.is_some()
        {
            return Err(EvidenceFault::MissingTaskTruth);
        }
        return Ok(PassiveEvaluation {
            classification: EvidenceClassification::Sleeping,
            task_truth: None,
            next: NextObservation::Sleep,
            reason_code: "SUPERVISOR_NO_TASK",
        });
    };

    validate_task(snapshot, &task)?;
    validate_bindings(snapshot, &task)?;

    match task.lifecycle {
        TaskLifecycle::Pending => evaluate_pending(snapshot, task),
        TaskLifecycle::Running => evaluate_running(snapshot, task.lifecycle),
        TaskLifecycle::Terminal(_) => evaluate_terminal(snapshot, task),
    }
}

fn evaluate_pending(
    snapshot: &PassiveSnapshot<'_>,
    task: TaskEvidence<'_>,
) -> Result<PassiveEvaluation, EvidenceFault> {
    if let Some(verification) = snapshot.verification
        && verification.state != VerificationState::Pending
    {
        return Err(EvidenceFault::VerificationBeforeTerminal);
    }
    if let Some(worker) = snapshot.worker
        && worker.activity == WorkerActivity::Terminal
    {
        return Ok(PassiveEvaluation {
            classification: EvidenceClassification::Stalled,
            task_truth: Some(task.lifecycle),
            next: NextObservation::ObserveTask,
            reason_code: "SUPERVISOR_TASK_TERMINAL_NOT_OBSERVED",
        });
    }
    let (next, reason_code) = match (snapshot.dispatch, snapshot.worker) {
        (None, None) => (
            NextObservation::ObserveDispatch,
            "SUPERVISOR_PENDING_DISPATCH_NOT_OBSERVED",
        ),
        (Some(_), None) => (
            NextObservation::ObserveWorkerSession,
            "SUPERVISOR_PENDING_DISPATCH_OBSERVED",
        ),
        (Some(_), Some(_)) => (
            NextObservation::ObserveTask,
            "SUPERVISOR_PENDING_ACTIVITY_OBSERVED",
        ),
        (None, Some(_)) => return Err(EvidenceFault::WorkerWithoutDispatch),
    };
    let latest_activity = snapshot
        .worker
        .map_or(task.observed_at, |worker| worker.observed_at)
        .max(
            snapshot
                .dispatch
                .map_or(task.observed_at, |dispatch| dispatch.observed_at),
        );
    if snapshot.observed_at - latest_activity > snapshot.stale_after {
        return Ok(PassiveEvaluation {
            classification: EvidenceClassification::Stalled,
            task_truth: Some(task.lifecycle),
            next,
            reason_code: "SUPERVISOR_PENDING_EVIDENCE_STALE",
        });
    }
    Ok(PassiveEvaluation {
        classification: EvidenceClassification::Pending,
        task_truth: Some(task.lifecycle),
        next,
        reason_code,
    })
}

fn evaluate_running(
    snapshot: &PassiveSnapshot<'_>,
    task_truth: TaskLifecycle,
) -> Result<PassiveEvaluation, EvidenceFault> {
    if let Some(verification) = snapshot.verification
        && verification.state != VerificationState::Pending
    {
        return Ok(PassiveEvaluation {
            classification: EvidenceClassification::Stalled,
            task_truth: Some(task_truth),
            next: NextObservation::ObserveTask,
            reason_code: "SUPERVISOR_TASK_TERMINAL_NOT_OBSERVED",
        });
    }
    let Some(_dispatch) = snapshot.dispatch else {
        if snapshot.worker.is_some() {
            return Err(EvidenceFault::WorkerWithoutDispatch);
        }
        return Ok(PassiveEvaluation {
            classification: EvidenceClassification::Stalled,
            task_truth: Some(task_truth),
            next: NextObservation::ObserveDispatch,
            reason_code: "SUPERVISOR_RUNNING_DISPATCH_MISSING",
        });
    };
    let Some(worker) = snapshot.worker else {
        return Ok(PassiveEvaluation {
            classification: EvidenceClassification::Stalled,
            task_truth: Some(task_truth),
            next: NextObservation::ObserveWorkerSession,
            reason_code: "SUPERVISOR_RUNNING_WORKER_MISSING",
        });
    };
    if worker.activity == WorkerActivity::Terminal {
        return Ok(PassiveEvaluation {
            classification: EvidenceClassification::Stalled,
            task_truth: Some(task_truth),
            next: NextObservation::ObserveTask,
            reason_code: "SUPERVISOR_TASK_TERMINAL_NOT_OBSERVED",
        });
    }
    if snapshot.observed_at - worker.observed_at > snapshot.stale_after {
        return Ok(PassiveEvaluation {
            classification: EvidenceClassification::Stalled,
            task_truth: Some(task_truth),
            next: NextObservation::ObserveWorkerSession,
            reason_code: "SUPERVISOR_WORKER_ACTIVITY_STALE",
        });
    }
    Ok(PassiveEvaluation {
        classification: EvidenceClassification::Running,
        task_truth: Some(task_truth),
        next: NextObservation::Wait,
        reason_code: "SUPERVISOR_RUNNING_ACTIVITY_FRESH",
    })
}

fn evaluate_terminal(
    snapshot: &PassiveSnapshot<'_>,
    task: TaskEvidence<'_>,
) -> Result<PassiveEvaluation, EvidenceFault> {
    if let Some(worker) = snapshot.worker
        && worker.activity != WorkerActivity::Terminal
        && worker.observed_at >= task.observed_at
    {
        return Err(EvidenceFault::TerminalWorkerConflict);
    }
    let (next, reason_code) = match snapshot.verification {
        None => (
            NextObservation::ReadVerification,
            "SUPERVISOR_TERMINAL_VERIFICATION_MISSING",
        ),
        Some(VerificationEvidence {
            state: VerificationState::Pending,
            ..
        }) => (
            NextObservation::ReadVerification,
            "SUPERVISOR_TERMINAL_VERIFICATION_PENDING",
        ),
        Some(verification) => {
            if !verification.evidence_ref.is_some_and(valid_ref) {
                return Err(EvidenceFault::MissingVerificationReference);
            }
            (
                NextObservation::RecommendDurableProjection,
                "SUPERVISOR_TERMINAL_VERIFICATION_OBSERVED",
            )
        }
    };
    Ok(PassiveEvaluation {
        classification: EvidenceClassification::Terminal,
        task_truth: Some(task.lifecycle),
        next,
        reason_code,
    })
}

fn validate_task(
    snapshot: &PassiveSnapshot<'_>,
    task: &TaskEvidence<'_>,
) -> Result<(), EvidenceFault> {
    if !valid_ref(task.task_ref) || !valid_ref(task.attempt_ref) || task.revision == 0 {
        return Err(EvidenceFault::InvalidEvidenceReference);
    }
    if task.observed_at > snapshot.observed_at {
        return Err(EvidenceFault::FutureObservation);
    }
    Ok(())
}

fn validate_bindings(
    snapshot: &PassiveSnapshot<'_>,
    task: &TaskEvidence<'_>,
) -> Result<(), EvidenceFault> {
    if let Some(dispatch) = snapshot.dispatch {
        validate_bound_ref(
            task,
            dispatch.task_ref,
            dispatch.attempt_ref,
            dispatch.observed_at,
            snapshot.observed_at,
        )?;
        if !valid_ref(dispatch.worker_session_ref) {
            return Err(EvidenceFault::InvalidEvidenceReference);
        }
    }
    if let Some(worker) = snapshot.worker {
        validate_bound_ref(
            task,
            worker.task_ref,
            worker.attempt_ref,
            worker.observed_at,
            snapshot.observed_at,
        )?;
        if !valid_ref(worker.worker_session_ref) {
            return Err(EvidenceFault::InvalidEvidenceReference);
        }
        let Some(dispatch) = snapshot.dispatch else {
            return Err(EvidenceFault::WorkerWithoutDispatch);
        };
        if worker.worker_session_ref != dispatch.worker_session_ref {
            return Err(EvidenceFault::WorkerSessionMismatch);
        }
        if worker.observed_at < dispatch.observed_at {
            return Err(EvidenceFault::ImpossibleObservationOrder);
        }
    }
    if let Some(verification) = snapshot.verification {
        validate_bound_ref(
            task,
            verification.task_ref,
            verification.attempt_ref,
            verification.observed_at,
            snapshot.observed_at,
        )?;
        if let Some(reference) = verification.evidence_ref
            && !valid_ref(reference)
        {
            return Err(EvidenceFault::InvalidEvidenceReference);
        }
    }
    Ok(())
}

fn validate_bound_ref(
    task: &TaskEvidence<'_>,
    task_ref: &str,
    attempt_ref: &str,
    observed_at: u64,
    snapshot_observed_at: u64,
) -> Result<(), EvidenceFault> {
    if !valid_ref(task_ref) || !valid_ref(attempt_ref) {
        return Err(EvidenceFault::InvalidEvidenceReference);
    }
    if task.task_ref != task_ref {
        return Err(EvidenceFault::CrossTaskEvidence);
    }
    if task.attempt_ref != attempt_ref {
        return Err(EvidenceFault::CrossAttemptEvidence);
    }
    if observed_at > snapshot_observed_at {
        return Err(EvidenceFault::FutureObservation);
    }
    Ok(())
}

fn valid_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.is_ascii()
        && !value.starts_with(char::is_whitespace)
        && !value.ends_with(char::is_whitespace)
        && value.bytes().all(|byte| !byte.is_ascii_control())
}

fn valid_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_thread_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
        })
}
