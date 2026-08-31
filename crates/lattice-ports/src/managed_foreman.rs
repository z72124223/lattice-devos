//! Narrow injected boundaries for one durable managed-worker attempt.
//!
//! Task Ledger remains the semantic owner of binding, attempt, observation,
//! and verification records. These ports transport only those verified values
//! plus digest-only provider observations; they own no lifecycle state or I/O.

use std::error::Error;
use std::fmt;

use lattice_artifact_store::{ManagedEvidenceKind, VerifiedManagedEvidence};
use lattice_contracts::ContentDigest;
use lattice_foreman_state::{
    AttemptPacketIdentity, ModelSelection, StartObservation, TurnStartedStatus, WorkerTerminal,
};
use lattice_task_ledger::{
    VerificationOutcome, VerifiedTaskExecutionBinding, VerifiedTaskVerificationRecord,
    VerifiedWorkerAttemptRecord, VerifiedWorkerObservationRecord, WorkerObservationInput,
    WorkerObservationKind,
};

/// Result returned by every managed-foreman injected boundary.
pub type ManagedPortResult<T> = Result<T, ManagedPortError>;

/// Whether a failed effect is known, ambiguous, or requires reconciliation
/// against retained provider/durable identity before any retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedPortErrorKind {
    Known,
    Ambiguous,
    ReconcileRequired,
}

/// Bounded, secret-free error shared by the managed repository, worker, and
/// independent verifier boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedPortError {
    kind: ManagedPortErrorKind,
    code: &'static str,
}

impl ManagedPortError {
    #[must_use]
    pub const fn new(kind: ManagedPortErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> ManagedPortErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ManagedPortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "managed port {:?}: {}", self.kind, self.code)
    }
}

impl Error for ManagedPortError {}

fn known_contract(code: &'static str) -> ManagedPortError {
    ManagedPortError::new(ManagedPortErrorKind::Known, code)
}

fn nonzero(digest: &ContentDigest) -> bool {
    !digest.as_str().bytes().all(|byte| byte == b'0')
}

/// One exact provider lifecycle observation paired with the Task-Ledger-owned
/// append input. Constructors prevent the two representations from diverging.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedWorkerObservation {
    kind: WorkerObservationKind,
    thread_id: String,
    turn_id: Option<String>,
    app_server_generation: u64,
    app_server_identity_digest: ContentDigest,
    evidence_digest: ContentDigest,
    ledger_input: WorkerObservationInput,
}

impl ManagedWorkerObservation {
    fn new(
        attempt_number: u64,
        kind: WorkerObservationKind,
        thread_id: impl Into<String>,
        turn_id: Option<impl Into<String>>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        let thread_id = thread_id.into();
        let turn_id = turn_id.map(Into::into);
        let ledger_input = WorkerObservationInput::new(
            attempt_number,
            kind,
            Some(thread_id.clone()),
            turn_id.clone(),
            app_server_generation,
            app_server_identity_digest.clone(),
            evidence_digest.clone(),
        )
        .map_err(|_| known_contract("LATTICE_MANAGED_OBSERVATION_REJECTED"))?;
        Ok(Self {
            kind,
            thread_id,
            turn_id,
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
            ledger_input,
        })
    }

    /// Records a known-success `thread/start` response. It is accepted, not
    /// executing, and therefore carries no turn identifier.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn thread_accepted(
        attempt_number: u64,
        thread_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        Self::new(
            attempt_number,
            WorkerObservationKind::ThreadAccepted,
            thread_id,
            None::<String>,
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    /// Records a known-success `turn/start` response. It remains starting
    /// until a separate exact `turn/started` notification is observed.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn turn_accepted(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        Self::new(
            attempt_number,
            WorkerObservationKind::TurnAccepted,
            thread_id,
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    /// Records only an exact matching in-progress `turn/started` notification.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn exact_started(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        provider_observed_at: impl Into<String>,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        let thread_id = thread_id.into();
        let turn_id = turn_id.into();
        let ledger_input = WorkerObservationInput::exact_started(
            attempt_number,
            thread_id.clone(),
            turn_id.clone(),
            app_server_generation,
            app_server_identity_digest.clone(),
            provider_observed_at,
            evidence_digest.clone(),
        )
        .map_err(|_| known_contract("LATTICE_MANAGED_OBSERVATION_REJECTED"))?;
        Ok(Self {
            kind: WorkerObservationKind::TurnStarted,
            thread_id,
            turn_id: Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
            ledger_input,
        })
    }

    /// Records one digest-only meaningful execution observation.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn meaningful_progress(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        Self::new(
            attempt_number,
            WorkerObservationKind::MeaningfulProgress,
            thread_id,
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    /// Records one exact heartbeat without accepting free-form progress text.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn heartbeat(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        Self::new(
            attempt_number,
            WorkerObservationKind::Heartbeat,
            thread_id,
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    /// Records one closed watchdog stall classification after exact provider
    /// reconciliation. The reason text remains inside the provider evidence
    /// digest; the Task Ledger stores only the closed observation kind.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn stall_classified(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        Self::new(
            attempt_number,
            WorkerObservationKind::StallClassified,
            thread_id,
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    /// Records a completed exact read/resume/reconciliation observation.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn reconciled(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        Self::new(
            attempt_number,
            WorkerObservationKind::Reconciled,
            thread_id,
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    /// Records a known-accepted interrupt request for one exact active turn.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn interrupt_requested(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        Self::new(
            attempt_number,
            WorkerObservationKind::InterruptRequested,
            thread_id,
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    /// Records one exact provider terminal. This is still not task success.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn terminal(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        terminal: WorkerTerminal,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        let kind = match terminal {
            WorkerTerminal::Completed => WorkerObservationKind::TerminalCompleted,
            WorkerTerminal::Interrupted => WorkerObservationKind::TerminalInterrupted,
            WorkerTerminal::Failed => WorkerObservationKind::TerminalFailed,
        };
        Self::new(
            attempt_number,
            kind,
            thread_id,
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    /// Records the unique recovered failure for a provider turn that was
    /// accepted but never produced an exact durable `turn/started` event.
    /// This is a terminal failure, not proof that execution began.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for an invalid attempt, provider
    /// identity, generation, or evidence digest.
    pub fn prestart_terminal_failed(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        Self::new(
            attempt_number,
            WorkerObservationKind::PrestartTerminalFailed,
            thread_id,
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            evidence_digest,
        )
    }

    #[must_use]
    pub const fn kind(&self) -> WorkerObservationKind {
        self.kind
    }

    #[must_use]
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    #[must_use]
    pub fn turn_id(&self) -> Option<&str> {
        self.turn_id.as_deref()
    }

    #[must_use]
    pub const fn app_server_generation(&self) -> u64 {
        self.app_server_generation
    }

    #[must_use]
    pub const fn app_server_identity_digest(&self) -> &ContentDigest {
        &self.app_server_identity_digest
    }

    /// Returns the exact connector-owned provider time only for
    /// `TURN_STARTED`.
    #[must_use]
    pub fn provider_observed_at(&self) -> Option<&str> {
        self.ledger_input.provider_observed_at()
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    #[must_use]
    pub const fn ledger_input(&self) -> &WorkerObservationInput {
        &self.ledger_input
    }

    /// Returns the corresponding pure exact-start state transition, if any.
    #[must_use]
    pub fn start_observation(&self) -> Option<StartObservation> {
        match self.kind {
            WorkerObservationKind::ThreadAccepted => Some(StartObservation::ThreadStartAccepted {
                thread_id: self.thread_id.clone(),
            }),
            WorkerObservationKind::TurnAccepted => Some(StartObservation::TurnStartAccepted {
                thread_id: self.thread_id.clone(),
                turn_id: self.turn_id.clone()?,
            }),
            WorkerObservationKind::TurnStarted => Some(StartObservation::TurnStarted {
                thread_id: self.thread_id.clone(),
                turn_id: self.turn_id.clone()?,
                status: TurnStartedStatus::InProgress,
                observed_at: self.ledger_input.provider_observed_at()?.to_owned(),
            }),
            _ => None,
        }
    }

    #[must_use]
    pub const fn terminal_kind(&self) -> Option<WorkerTerminal> {
        match self.kind {
            WorkerObservationKind::PrestartTerminalFailed
            | WorkerObservationKind::TerminalFailed => Some(WorkerTerminal::Failed),
            WorkerObservationKind::TerminalCompleted => Some(WorkerTerminal::Completed),
            WorkerObservationKind::TerminalInterrupted => Some(WorkerTerminal::Interrupted),
            _ => None,
        }
    }
}

/// Digest-only request to an independently composed verifier. It cannot carry
/// an objective, shell string, command arguments, worktree path, or environment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedVerificationRequest {
    profile_identity: ContentDigest,
    command_identity: ContentDigest,
    base_commit_digest: ContentDigest,
    result_commit_digest: ContentDigest,
    tree_digest: ContentDigest,
    diff_digest: ContentDigest,
    worker_evidence_digest: ContentDigest,
    evidence_artifact_digest: ContentDigest,
}

impl ManagedVerificationRequest {
    /// Builds a verifier request from closed identities and an owner-verified
    /// artifact descriptor.
    ///
    /// # Errors
    ///
    /// Returns a known contract error when any required identity or digest is
    /// the zero digest.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        profile_identity: ContentDigest,
        command_identity: ContentDigest,
        base_commit_digest: ContentDigest,
        result_commit_digest: ContentDigest,
        tree_digest: ContentDigest,
        diff_digest: ContentDigest,
        worker_evidence_digest: ContentDigest,
        evidence_artifact: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<Self> {
        if [
            &profile_identity,
            &command_identity,
            &base_commit_digest,
            &result_commit_digest,
            &tree_digest,
            &diff_digest,
            &worker_evidence_digest,
            evidence_artifact.descriptor_digest(),
        ]
        .into_iter()
        .any(|digest| !nonzero(digest))
        {
            return Err(known_contract(
                "LATTICE_MANAGED_VERIFICATION_BINDING_REJECTED",
            ));
        }
        Ok(Self {
            profile_identity,
            command_identity,
            base_commit_digest,
            result_commit_digest,
            tree_digest,
            diff_digest,
            worker_evidence_digest,
            evidence_artifact_digest: evidence_artifact.descriptor_digest().clone(),
        })
    }

    #[must_use]
    pub const fn profile_identity(&self) -> &ContentDigest {
        &self.profile_identity
    }
    #[must_use]
    pub const fn command_identity(&self) -> &ContentDigest {
        &self.command_identity
    }
    #[must_use]
    pub const fn base_commit_digest(&self) -> &ContentDigest {
        &self.base_commit_digest
    }
    #[must_use]
    pub const fn result_commit_digest(&self) -> &ContentDigest {
        &self.result_commit_digest
    }
    #[must_use]
    pub const fn tree_digest(&self) -> &ContentDigest {
        &self.tree_digest
    }
    #[must_use]
    pub const fn diff_digest(&self) -> &ContentDigest {
        &self.diff_digest
    }
    #[must_use]
    pub const fn worker_evidence_digest(&self) -> &ContentDigest {
        &self.worker_evidence_digest
    }
    #[must_use]
    pub const fn evidence_artifact_digest(&self) -> &ContentDigest {
        &self.evidence_artifact_digest
    }
}

/// Independent preparation result. The exact evidence object is already
/// content- and descriptor-verified but is not yet claimed durable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedVerificationPreparation {
    evidence: VerifiedManagedEvidence,
    supplemental_evidence: Vec<VerifiedManagedEvidence>,
    request: ManagedVerificationRequest,
    mechanical_outcome: VerificationOutcome,
}

impl ManagedVerificationPreparation {
    /// Binds owner-verified evidence to the exact task, attempt, and request.
    ///
    /// # Errors
    ///
    /// Returns a known contract error for task, attempt, evidence-kind, or
    /// descriptor substitution.
    pub fn new(
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        evidence: VerifiedManagedEvidence,
        request: ManagedVerificationRequest,
    ) -> ManagedPortResult<Self> {
        if evidence.task_ref() != binding.task_ref()
            || u64::from(evidence.attempt()) != attempt.attempt_number()
            || evidence.kind() != ManagedEvidenceKind::GitSnapshot
            || request.evidence_artifact_digest() != evidence.descriptor_digest()
        {
            return Err(known_contract(
                "LATTICE_MANAGED_VERIFICATION_PREPARATION_REJECTED",
            ));
        }
        Ok(Self {
            evidence,
            supplemental_evidence: Vec::new(),
            request,
            // Compatibility constructors represent the historical successful
            // mechanical preparation. Concrete adapters must override this
            // when any closed Git/check gate fails.
            mechanical_outcome: VerificationOutcome::Passed,
        })
    }

    /// Binds the closed mechanical Git/check outcome produced before any
    /// semantic reviewer model call.
    #[must_use]
    pub fn with_mechanical_outcome(mut self, outcome: VerificationOutcome) -> Self {
        self.mechanical_outcome = outcome;
        self
    }

    /// Adds bounded, owner-typed evidence produced by the independent
    /// verifier. Phase 4 permits only review and resource observations here;
    /// the primary exact Git snapshot remains singular.
    ///
    /// # Errors
    ///
    /// Returns a known contract error when the evidence exceeds the closed
    /// count, belongs to another task or attempt, uses a disallowed kind, or
    /// repeats a descriptor digest.
    pub fn with_supplemental_evidence(
        mut self,
        evidence: Vec<VerifiedManagedEvidence>,
    ) -> ManagedPortResult<Self> {
        if evidence.len() > 4
            || evidence.iter().any(|value| {
                value.task_ref() != self.evidence.task_ref()
                    || value.attempt() != self.evidence.attempt()
                    || !matches!(
                        value.kind(),
                        ManagedEvidenceKind::ReviewResult
                            | ManagedEvidenceKind::ResourceObservation
                    )
            })
        {
            return Err(known_contract(
                "LATTICE_MANAGED_VERIFICATION_PREPARATION_REJECTED",
            ));
        }
        if evidence.iter().enumerate().any(|(index, value)| {
            evidence[..index]
                .iter()
                .any(|prior| prior.descriptor_digest() == value.descriptor_digest())
        }) {
            return Err(known_contract(
                "LATTICE_MANAGED_VERIFICATION_PREPARATION_REJECTED",
            ));
        }
        self.supplemental_evidence = evidence;
        Ok(self)
    }

    #[must_use]
    pub const fn evidence(&self) -> &VerifiedManagedEvidence {
        &self.evidence
    }

    #[must_use]
    pub fn supplemental_evidence(&self) -> &[VerifiedManagedEvidence] {
        &self.supplemental_evidence
    }

    #[must_use]
    pub const fn request(&self) -> &ManagedVerificationRequest {
        &self.request
    }

    #[must_use]
    pub const fn mechanical_outcome(&self) -> VerificationOutcome {
        self.mechanical_outcome
    }
}

/// Durable repository receipt for one exact owner-typed managed evidence
/// object. The storage receipt is opaque and cannot substitute either Artifact
/// Store digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedArtifactReceipt {
    task_ref: ContentDigest,
    attempt: u8,
    content_digest: ContentDigest,
    descriptor_digest: ContentDigest,
    storage_receipt_digest: ContentDigest,
}

impl ManagedArtifactReceipt {
    /// Creates an opaque durable receipt for one exact verified artifact.
    ///
    /// # Errors
    ///
    /// Returns a known contract error when the storage receipt is the zero
    /// digest.
    pub fn new(
        evidence: &VerifiedManagedEvidence,
        storage_receipt_digest: ContentDigest,
    ) -> ManagedPortResult<Self> {
        if !nonzero(&storage_receipt_digest) {
            return Err(known_contract("LATTICE_MANAGED_ARTIFACT_RECEIPT_REJECTED"));
        }
        Ok(Self {
            task_ref: evidence.task_ref().clone(),
            attempt: evidence.attempt(),
            content_digest: evidence.content_digest().clone(),
            descriptor_digest: evidence.descriptor_digest().clone(),
            storage_receipt_digest,
        })
    }

    #[must_use]
    pub fn matches(&self, evidence: &VerifiedManagedEvidence) -> bool {
        self.task_ref == *evidence.task_ref()
            && self.attempt == evidence.attempt()
            && self.content_digest == *evidence.content_digest()
            && self.descriptor_digest == *evidence.descriptor_digest()
    }

    #[must_use]
    pub const fn descriptor_digest(&self) -> &ContentDigest {
        &self.descriptor_digest
    }

    #[must_use]
    pub const fn storage_receipt_digest(&self) -> &ContentDigest {
        &self.storage_receipt_digest
    }
}

/// Independent verifier result, still awaiting durable Task Ledger append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedVerificationEvidence {
    request: ManagedVerificationRequest,
    outcome: VerificationOutcome,
    result_digest: ContentDigest,
    review_digest: Option<ContentDigest>,
}

impl ManagedVerificationEvidence {
    /// Binds an independent verifier result to its exact closed request.
    ///
    /// # Errors
    ///
    /// Returns a known contract error when a required result or optional
    /// review digest is the zero digest.
    pub fn new(
        request: ManagedVerificationRequest,
        outcome: VerificationOutcome,
        result_digest: ContentDigest,
        review_digest: Option<ContentDigest>,
    ) -> ManagedPortResult<Self> {
        if !nonzero(&result_digest) || review_digest.as_ref().is_some_and(|value| !nonzero(value)) {
            return Err(known_contract(
                "LATTICE_MANAGED_VERIFICATION_EVIDENCE_REJECTED",
            ));
        }
        Ok(Self {
            request,
            outcome,
            result_digest,
            review_digest,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &ManagedVerificationRequest {
        &self.request
    }
    #[must_use]
    pub const fn outcome(&self) -> VerificationOutcome {
        self.outcome
    }
    #[must_use]
    pub const fn result_digest(&self) -> &ContentDigest {
        &self.result_digest
    }
    #[must_use]
    pub const fn review_digest(&self) -> Option<&ContentDigest> {
        self.review_digest.as_ref()
    }
}

/// Exact provider terminal. It carries no Artifact Store or verification
/// claim; those are prepared independently only after this observation is
/// durable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTerminalCandidate {
    observation: ManagedWorkerObservation,
    intermediate_observations: Vec<ManagedWorkerObservation>,
    resource_evidence: Vec<VerifiedManagedEvidence>,
}

/// One exact execution-stream item. The worker boundary yields only one item
/// at a time so the orchestrator can make it durable before it asks the
/// provider for another event. In particular, progress and resource usage are
/// never buffered until a terminal notification.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedWorkerExecutionEvent {
    /// A heartbeat, meaningful-progress, reconciliation, stall, or interrupt
    /// observation for the retained exact thread/turn.
    Observation(ManagedWorkerObservation),
    /// A cumulative resource sample paired with the exact progress event that
    /// caused it. Both values must be durably recorded before polling again.
    ResourceObservation {
        observation: ManagedWorkerObservation,
        evidence: Box<VerifiedManagedEvidence>,
    },
    /// A provider-subtree lifecycle receipt. The orchestrator must persist it
    /// before polling again, so a terminal candidate can never outrun the
    /// durable zero-descendant proof for its execution domain.
    LifecycleEvidence(VerifiedManagedEvidence),
    /// The exact provider terminal. This remains only a verification
    /// candidate; it does not imply task completion.
    Terminal(ManagedTerminalCandidate),
}

impl ManagedTerminalCandidate {
    /// Accepts only an exact terminal provider observation.
    ///
    /// # Errors
    ///
    /// Returns a known contract error when the supplied observation is not a
    /// completed, interrupted, or failed terminal.
    pub fn new(observation: ManagedWorkerObservation) -> ManagedPortResult<Self> {
        if observation.terminal_kind().is_none() {
            return Err(known_contract(
                "LATTICE_MANAGED_TERMINAL_CANDIDATE_REJECTED",
            ));
        }
        Ok(Self {
            observation,
            intermediate_observations: Vec::new(),
            resource_evidence: Vec::new(),
        })
    }

    /// Attaches bounded observations and owner-typed resource evidence
    /// collected while waiting for the exact terminal. The orchestrator still
    /// validates and persists every item before treating the terminal as
    /// durable.
    #[must_use]
    pub fn with_intermediate(
        mut self,
        observations: Vec<ManagedWorkerObservation>,
        resource_evidence: Vec<VerifiedManagedEvidence>,
    ) -> Self {
        self.intermediate_observations = observations;
        self.resource_evidence = resource_evidence;
        self
    }

    #[must_use]
    pub const fn observation(&self) -> &ManagedWorkerObservation {
        &self.observation
    }

    #[must_use]
    pub fn intermediate_observations(&self) -> &[ManagedWorkerObservation] {
        &self.intermediate_observations
    }

    #[must_use]
    pub fn resource_evidence(&self) -> &[VerifiedManagedEvidence] {
        &self.resource_evidence
    }
}

/// Closed provider availability result. No implicit fallback model exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedModelAvailability {
    Available,
    Unavailable { code: &'static str },
}

/// Result of reading/resuming/reconciling only retained provider identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedWorkerReconciliation {
    ExactActive(ManagedWorkerObservation),
    ExactTerminal(ManagedTerminalCandidate),
    Unresolved,
}

/// Restart-only outcome for a provider effect that may have crossed a durable
/// `WorkerThread` or `WorkerTurn` dispatch claim before the prior process died.
/// It deliberately has no exact-start/success variant: a marker-bound turn
/// without a durable exact `turn/started` must be closed as a failed start.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedWorkerPrestartRecovery {
    /// Bounded provider discovery completed and proved that no post-claim
    /// candidate exists. Only this outcome may be combined with fresh current
    /// authority, model preflight, and a newly `Claimed` dispatch disposition.
    ProvenNoProviderCandidate,
    /// One exact provider thread exists and is still proven to contain no
    /// turns. A separately claimed `WorkerTurn` effect may continue it.
    ExactEmptyThread { thread: ManagedWorkerObservation },
    /// Exact provider identities were recovered and the turn was brought to a
    /// terminal, but exact start was never durable. The terminal must be
    /// `Failed`; provider-native status remains inside the evidence digest.
    ExactFailedStart {
        thread: ManagedWorkerObservation,
        turn: Box<ManagedWorkerObservation>,
        terminal: Box<ManagedTerminalCandidate>,
    },
    /// Bounded discovery/read/resume could not prove a unique safe outcome.
    ReconciliationRequired,
}

/// Durable dispatch facts used to choose a restart-only provider operation.
///
/// These values are repository-owned truth, not an inference from provider
/// discovery. In particular, `NoWorkerThread` is the only state in which a
/// bounded no-candidate result can prove that no worker provider effect was
/// ever authorized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedWorkerDispatchState {
    NoWorkerThread,
    WorkerThreadClaimed,
    WorkerTurnClaimed,
}

/// Typed proof that a retained pre-exact-start attempt currently has no live
/// worker turn. It deliberately does not grant authority for a new provider
/// effect; continuation must separately revalidate execution authority,
/// model availability, the Writer fence, and any required baseline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedPrestartNoEffectProof {
    /// The attempt is still a durable reservation and no provider dispatch
    /// claim exists.
    PendingReservation,
    /// Bounded discovery found no provider candidate. The boolean retains
    /// whether a `WorkerThread` effect had nevertheless already been claimed.
    ProvenNoProviderCandidate { worker_thread_claimed: bool },
    /// One exact retained thread was durably observed empty. A prior
    /// `WorkerTurn` claim is retained because it requires a second exact-empty
    /// proof immediately before any continuation RPC.
    ExactEmptyThreadNoTurn {
        thread: Box<VerifiedWorkerObservationRecord>,
        worker_turn_claimed: bool,
    },
}

/// Closed outcome of durably cancelling a prestart attempt after typed proof
/// of no live provider turn. Exact replay is idempotent success only when the
/// repository proves the retained closure is identical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedPrestartClosureDisposition {
    Closed,
    ExactReplay,
}

/// Closed outcome of the PostgreSQL-owned attempt claim. Only `Claimed`
/// authorizes a new provider dispatch. `ExactReplay` proves that the durable
/// attempt already existed and therefore permits reconciliation only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedAttemptClaimDisposition {
    Claimed,
    ExactReplay,
}

/// Closed outcome of the one-shot semantic-review provider claim. A replay
/// may only enter an explicitly reconciliation-only reviewer adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedReviewDispatchDisposition {
    Claimed,
    ExactReplay,
}

/// Closed one-shot authorization for the initial worker thread provider RPC.
/// This is deliberately distinct from the durable attempt/capacity claim so
/// a pre-provider failure can be proven without treating an unobserved RPC as
/// inactive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedWorkerThreadDispatchDisposition {
    Claimed,
    ExactReplay,
}

/// Closed one-shot authorization for `turn/start` on an already durably
/// accepted worker thread. `ExactReplay` is not fresh turn authority; a
/// restart may continue it only after a typed exact-empty read and an adapter
/// that repeats the exact-empty proof immediately before the RPC.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedWorkerTurnDispatchDisposition {
    Claimed,
    ExactReplay,
}

/// One owner-verified attempt paired with its atomic claim disposition.
/// Keeping the disposition attached prevents a replayed Task Ledger append
/// from accidentally being treated as fresh provider-effect authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedAttemptClaim {
    attempt: VerifiedWorkerAttemptRecord,
    disposition: ManagedAttemptClaimDisposition,
}

impl ManagedAttemptClaim {
    #[must_use]
    pub const fn new(
        attempt: VerifiedWorkerAttemptRecord,
        disposition: ManagedAttemptClaimDisposition,
    ) -> Self {
        Self {
            attempt,
            disposition,
        }
    }

    #[must_use]
    pub const fn attempt(&self) -> &VerifiedWorkerAttemptRecord {
        &self.attempt
    }

    #[must_use]
    pub const fn disposition(&self) -> ManagedAttemptClaimDisposition {
        self.disposition
    }

    #[must_use]
    pub fn into_attempt(self) -> VerifiedWorkerAttemptRecord {
        self.attempt
    }
}

/// Durable managed-attempt repository over the existing Task Ledger and
/// authority paths. Implementations must make claim/capacity/fence admission
/// one atomic operation; this trait does not expose SQL or another state machine.
/// Every method uses the closed [`ManagedPortErrorKind`] classification;
/// ambiguous and reconciliation-required outcomes never imply success.
#[allow(clippy::missing_errors_doc)]
pub trait ManagedForemanRepositoryPort {
    fn assert_execution_authority_current(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        authority_digest: &ContentDigest,
    ) -> ManagedPortResult<()>;

    fn claim_attempt(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<ManagedAttemptClaim>;

    fn record_observation(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        observation: &ManagedWorkerObservation,
    ) -> ManagedPortResult<VerifiedWorkerObservationRecord>;

    /// Atomically claims the one initial worker thread provider effect after
    /// the attempt and optional pre-dispatch baseline are durable.
    fn claim_worker_thread_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
    ) -> ManagedPortResult<ManagedWorkerThreadDispatchDisposition>;

    /// Atomically claims the sole `turn/start` effect for the exact retained
    /// worker thread. The thread observation must already be durable.
    fn claim_worker_turn_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        thread: &VerifiedWorkerObservationRecord,
    ) -> ManagedPortResult<ManagedWorkerTurnDispatchDisposition>;

    /// Loads the durable worker provider-claim head for restart recovery.
    /// Implementations must not infer this state from provider discovery.
    fn load_worker_dispatch_state(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
    ) -> ManagedPortResult<ManagedWorkerDispatchState> {
        Err(ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_WORKER_DISPATCH_STATE_REQUIRED",
        ))
    }

    /// Atomically records a terminal prestart cancellation that may release
    /// capacity only after the supplied typed no-effect proof is verified
    /// against durable dispatch claims and provider observations.
    fn close_prestart_without_provider_effect(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        _proof: &ManagedPrestartNoEffectProof,
        _blocker_code: &'static str,
    ) -> ManagedPortResult<ManagedPrestartClosureDisposition> {
        Err(ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_PRESTART_CLOSURE_REPOSITORY_REQUIRED",
        ))
    }

    /// Persists the exact verified Artifact Store object before independent
    /// verification can consume its descriptor.
    fn record_artifact(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        evidence: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedArtifactReceipt>;

    /// Atomically claims the single semantic-review provider call for this
    /// exact attempt and closed verification subject. Implementations must
    /// return `ExactReplay` for an identical retained claim and reject every
    /// substitution.
    fn claim_review_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
        request: &ManagedVerificationRequest,
    ) -> ManagedPortResult<ManagedReviewDispatchDisposition>;

    /// Atomically claims the sole reviewer `turn/start` effect after the
    /// reviewer thread lifecycle artifact is durable.
    fn claim_review_turn_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        request: &ManagedVerificationRequest,
        thread_lifecycle: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedReviewDispatchDisposition>;

    fn record_verification(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        evidence: &ManagedVerificationEvidence,
    ) -> ManagedPortResult<VerifiedTaskVerificationRecord>;
}

/// Reasserts the exact current Writer/fence immediately before one provider
/// RPC that can create a new external effect. The durable implementation is
/// injected by composition so orchestration cannot rely on an earlier lease
/// observation.
#[allow(clippy::missing_errors_doc)]
pub trait ManagedProviderEffectGuardPort {
    fn assert_provider_effect_writer_current(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
    ) -> ManagedPortResult<()>;
}

impl<F> ManagedProviderEffectGuardPort for F
where
    F: FnMut(&VerifiedTaskExecutionBinding, &VerifiedWorkerAttemptRecord) -> ManagedPortResult<()>,
{
    fn assert_provider_effect_writer_current(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
    ) -> ManagedPortResult<()> {
        self(binding, attempt)
    }
}

/// Exact Codex App Server lifecycle boundary. RPC acceptance and exact
/// `turn/started` are separate calls, as are read/resume/reconcile/interrupt.
/// Every method uses the closed [`ManagedPortErrorKind`] classification;
/// ambiguous and reconciliation-required outcomes never imply success.
#[allow(clippy::missing_errors_doc)]
pub trait ManagedCodexWorkerPort {
    fn model_availability(
        &mut self,
        selection: &ModelSelection,
    ) -> ManagedPortResult<ManagedModelAvailability>;

    /// Prepares the exact provider execution domain without invoking a model
    /// or creating a provider thread. The returned OPEN lifecycle marker must
    /// be durable before [`Self::start_thread`] authorizes the provider RPC.
    fn prepare_provider_dispatch(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<VerifiedManagedEvidence>;

    fn start_thread(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<ManagedWorkerObservation>;

    fn start_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
    ) -> ManagedPortResult<ManagedWorkerObservation>;

    fn wait_exact_started(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerObservation>;

    /// Reconciles an already durable `WorkerThread` dispatch claim after a
    /// process restart. Implementations may only perform bounded discovery,
    /// exact read/resume, and exact prestart terminalization. They must never
    /// open a new provider thread or start a turn.
    fn recover_claimed_dispatch(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<ManagedWorkerPrestartRecovery>;

    /// Reconciles a durable `ThreadAccepted`/optional `TurnAccepted` identity
    /// before exact start. No execution window exists in this lifecycle phase.
    /// Implementations must never create a replacement thread or duplicate
    /// turn.
    fn recover_prestart(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: Option<&str>,
    ) -> ManagedPortResult<ManagedWorkerPrestartRecovery>;

    /// Polls exactly one bounded event for the retained active turn. Callers
    /// must persist the returned item before polling again.
    fn next_execution_event(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerExecutionEvent>;

    fn read_exact_thread(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation>;

    fn read_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation>;

    fn resume_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation>;

    fn reconcile_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerReconciliation>;

    fn interrupt_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> ManagedPortResult<ManagedWorkerObservation>;

    /// Performs a read-only exact retained-thread reconciliation solely to
    /// recover terminal cumulative usage after a process crash. It must never
    /// start replacement work. `None` means the provider could not supply a
    /// credible terminal cumulative sample and callers must fail closed.
    fn reconcile_terminal_usage(
        &mut self,
        _attempt: &VerifiedWorkerAttemptRecord,
        _thread_id: &str,
        _turn_id: &str,
    ) -> ManagedPortResult<Option<VerifiedManagedEvidence>> {
        Err(ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_MODEL_USAGE_RECONCILIATION_REQUIRED",
        ))
    }
}

/// Independent verification boundary. The complete request surface is closed
/// and digest-only; adapters resolve command/path details from trusted policy.
/// Every method uses the closed [`ManagedPortErrorKind`] classification;
/// ambiguous and reconciliation-required outcomes never imply success.
#[allow(clippy::missing_errors_doc)]
pub trait ManagedVerificationPort {
    /// Independently prepares an exact Git/evidence snapshot after the worker
    /// terminal is already durable. It resolves trusted command/path details
    /// internally and exposes only owner-typed evidence plus digest identities.
    fn prepare(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
    ) -> ManagedPortResult<ManagedVerificationPreparation>;

    /// Returns one bounded, owner-typed observation captured before a failed
    /// mechanical preparation returned. This is intentionally separate from
    /// [`ManagedVerificationPreparation`]: an incomplete Git snapshot must
    /// never be promoted into the primary verification artifact merely so a
    /// transport receipt can become durable.
    ///
    /// The default is empty because most verifier failures have no separately
    /// verified transport artifact. Concrete adapters may return only a typed
    /// verification-result artifact bound to the exact task and attempt
    /// supplied by the caller.
    fn preparation_failure_evidence(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        _terminal: &VerifiedWorkerObservationRecord,
        _failure: &ManagedPortError,
    ) -> ManagedPortResult<Option<VerifiedManagedEvidence>> {
        Ok(None)
    }

    /// Runs the independent semantic review only after the caller has made
    /// the mechanical evidence durable and transitioned the authoritative
    /// Task state to `REVIEWING`. The sink is owned by orchestration and must
    /// durably record each bounded lifecycle/result/resource artifact before
    /// the verifier reads the next provider event.
    fn review(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        _terminal: &VerifiedWorkerObservationRecord,
        _request: &ManagedVerificationRequest,
        _sink: &mut dyn ManagedReviewEvidenceSink,
    ) -> ManagedPortResult<()> {
        Ok(())
    }

    fn verify(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
        request: &ManagedVerificationRequest,
    ) -> ManagedPortResult<ManagedVerificationEvidence>;
}

/// Orchestrator-owned durable sink for one bounded semantic-review artifact.
/// The verifier sees only this narrow callback and never a database or
/// repository implementation.
pub trait ManagedReviewEvidenceSink {
    /// Persists one exact bounded reviewer evidence object.
    ///
    /// # Errors
    ///
    /// Returns a port error when persistence, replay verification, quota, or
    /// descriptor matching cannot be proven.
    fn record(
        &mut self,
        evidence: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedArtifactReceipt>;

    /// Authorizes account/model reads and the reviewer thread only after the
    /// exact WSL2 provider-subtree OPEN marker is durable. Native reviewer
    /// transports never call this boundary.
    ///
    /// # Errors
    ///
    /// The default implementation returns a reconciliation-required error.
    /// Concrete sinks may also return validation or provider-guard errors when
    /// lifecycle evidence or durable authorization cannot be proven.
    fn authorize_provider_dispatch(
        &mut self,
        _open_lifecycle: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<()> {
        Err(ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_REVIEW_PROVIDER_DISPATCH_RECONCILIATION_REQUIRED",
        ))
    }

    /// Claims the one reviewer `turn/start` effect only after the exact
    /// accepted/reconciled thread lifecycle object has been made durable.
    /// Review transports must wait for `Claimed` before issuing `turn/start`;
    /// `ExactReplay` is reconciliation-only and never an authorization.
    ///
    /// # Errors
    ///
    /// Returns a reconciliation-required error unless the concrete durable
    /// sink proves the exact reviewer-turn dispatch claim.
    fn authorize_turn_start(
        &mut self,
        _thread_lifecycle: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedReviewDispatchDisposition> {
        Err(ManagedPortError::new(
            ManagedPortErrorKind::ReconcileRequired,
            "LATTICE_MANAGED_REVIEW_TURN_DISPATCH_RECONCILIATION_REQUIRED",
        ))
    }
}

impl<F> ManagedReviewEvidenceSink for F
where
    F: FnMut(&VerifiedManagedEvidence) -> ManagedPortResult<ManagedArtifactReceipt>,
{
    fn record(
        &mut self,
        evidence: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedArtifactReceipt> {
        self(evidence)
    }
}
