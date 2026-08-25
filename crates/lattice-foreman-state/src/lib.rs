//! Secret-free foreman snapshot validation, replay projection, and watchdog logic.

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const SNAPSHOT_SCHEMA: &str = "lattice.foreman-snapshot/1.0";
const EPISTEMIC_SCHEMA: &str = "lattice.foreman-epistemic/1.0";
const DEPENDENCY_BLOCKER_PREFIX: &str = "dependency:v1:";
const MAX_REFERENCE_BYTES: usize = 256;
const MAX_CHECKPOINT_ID_BYTES: usize = 64;

/// The only durable foreman identity admitted to the product coordination
/// stream. Git evidence remains observed per checkpoint, but this identity is
/// fixed by the server and cannot be supplied by an MCP caller.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SoleForemanBinding;

impl SoleForemanBinding {
    pub const WORKER: &'static str = "sole-foreman-v1";
    pub const THREAD: &'static str = "lattice-devos-sole-foreman-v1";
    pub const TASK: &'static str = "TASK-FOREMAN-COORDINATION";

    /// Constructs one server-owned Git observation for the fixed identity.
    ///
    /// # Errors
    ///
    /// Rejects malformed Git evidence.
    pub fn observe_git(
        branch: impl Into<String>,
        worktree: impl Into<String>,
        head: impl Into<String>,
    ) -> Result<ForemanServerObservation, SnapshotError> {
        ForemanServerObservation::new(
            Self::WORKER,
            Self::THREAD,
            Self::TASK,
            branch,
            worktree,
            head,
        )
    }

    /// Verifies that a retained or proposed snapshot belongs to the sole
    /// product foreman rather than an arbitrary generic worker identity.
    #[must_use]
    pub fn matches(snapshot: &ForemanSnapshot) -> bool {
        snapshot.worker() == Self::WORKER
            && snapshot.thread() == Self::THREAD
            && snapshot.task() == Self::TASK
    }
}

/// Closed worker coordination state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForemanState {
    Active,
    Blocked,
    Completed,
}

/// The explicit confidence of a non-authoritative epistemic record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confidence {
    Unknown,
    Low,
    Medium,
    High,
}

/// A closed reason that forces an epistemic record to be checked again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshTrigger {
    Expiry,
    NewEvidence,
    Counterevidence,
    DependencyChange,
}

/// Bounded references for provisional facts and hypotheses. The text of a
/// hypothesis is deliberately absent: its pointer cannot become task truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpistemicReferences {
    observed_facts: Vec<String>,
    hypotheses: Vec<String>,
    confidence: Confidence,
    unknowns: Vec<String>,
    evidence: Vec<String>,
    counterevidence: Vec<String>,
    checked_at: String,
    expires_at: String,
    refresh_trigger: RefreshTrigger,
    decision: String,
    probe: String,
    falsifier: String,
}

impl EpistemicReferences {
    /// # Errors
    ///
    /// Rejects non-pointer content, malformed timestamps, and an expiry that
    /// does not follow its check time.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        observed_facts: Vec<String>,
        hypotheses: Vec<String>,
        confidence: Confidence,
        unknowns: Vec<String>,
        evidence: Vec<String>,
        counterevidence: Vec<String>,
        checked_at: impl Into<String>,
        expires_at: impl Into<String>,
        refresh_trigger: RefreshTrigger,
        decision: impl Into<String>,
        probe: impl Into<String>,
        falsifier: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let checked_at = timestamp(checked_at.into())?;
        let expires_at = timestamp(expires_at.into())?;
        if expires_at <= checked_at {
            return Err(SnapshotError::MalformedReference);
        }
        Ok(Self {
            observed_facts: pointer_list(observed_facts, "fact")?,
            hypotheses: pointer_list(hypotheses, "hypothesis")?,
            confidence,
            unknowns: pointer_list(unknowns, "unknown")?,
            evidence: pointer_list(evidence, "evidence")?,
            counterevidence: pointer_list(counterevidence, "counterevidence")?,
            checked_at,
            expires_at,
            refresh_trigger,
            decision: digest_pointer(decision.into(), "decision")?,
            probe: digest_pointer(probe.into(), "probe")?,
            falsifier: digest_pointer(falsifier.into(), "falsifier")?,
        })
    }

    /// Versioned schema for non-authoritative epistemic pointers only.
    #[must_use]
    pub const fn schema(&self) -> &'static str {
        EPISTEMIC_SCHEMA
    }

    #[must_use]
    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }

    /// Observed-fact digest pointers; callers must resolve and assess them
    /// independently rather than treating them as lifecycle state.
    #[must_use]
    pub fn observed_facts(&self) -> &[String] {
        &self.observed_facts
    }

    /// Hypothesis digest pointers; they remain provisional by contract.
    #[must_use]
    pub fn hypotheses(&self) -> &[String] {
        &self.hypotheses
    }

    /// Unknowns that must remain explicit in any later decision.
    #[must_use]
    pub fn unknowns(&self) -> &[String] {
        &self.unknowns
    }

    /// Supporting evidence digest pointers.
    #[must_use]
    pub fn evidence(&self) -> &[String] {
        &self.evidence
    }

    /// Counterevidence digest pointers.
    #[must_use]
    pub fn counterevidence(&self) -> &[String] {
        &self.counterevidence
    }

    /// Time at which the epistemic record was checked.
    #[must_use]
    pub fn checked_at(&self) -> &str {
        &self.checked_at
    }

    /// Time at which the record must be refreshed before reuse.
    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }

    /// Closed condition that requires reassessment.
    #[must_use]
    pub const fn refresh_trigger(&self) -> RefreshTrigger {
        self.refresh_trigger
    }

    /// Digest pointer to the decision under examination.
    #[must_use]
    pub fn decision(&self) -> &str {
        &self.decision
    }

    /// Digest pointer to the probe that can reduce the uncertainty.
    #[must_use]
    pub fn probe(&self) -> &str {
        &self.probe
    }

    /// Digest pointer to the record that can falsify the hypothesis.
    #[must_use]
    pub fn falsifier(&self) -> &str {
        &self.falsifier
    }
}

impl Confidence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::Low => "LOW",
            Self::Medium => "MEDIUM",
            Self::High => "HIGH",
        }
    }

    /// Parses the closed persistence spelling.
    ///
    /// # Errors
    ///
    /// Rejects every unknown or case-substituted value.
    pub fn from_persisted(value: &str) -> Result<Self, SnapshotError> {
        match value {
            "UNKNOWN" => Ok(Self::Unknown),
            "LOW" => Ok(Self::Low),
            "MEDIUM" => Ok(Self::Medium),
            "HIGH" => Ok(Self::High),
            _ => Err(SnapshotError::MalformedReference),
        }
    }
}

impl RefreshTrigger {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Expiry => "EXPIRY",
            Self::NewEvidence => "NEW_EVIDENCE",
            Self::Counterevidence => "COUNTEREVIDENCE",
            Self::DependencyChange => "DEPENDENCY_CHANGE",
        }
    }

    /// Parses the closed persistence spelling.
    ///
    /// # Errors
    ///
    /// Rejects every unknown or case-substituted value.
    pub fn from_persisted(value: &str) -> Result<Self, SnapshotError> {
        match value {
            "EXPIRY" => Ok(Self::Expiry),
            "NEW_EVIDENCE" => Ok(Self::NewEvidence),
            "COUNTEREVIDENCE" => Ok(Self::Counterevidence),
            "DEPENDENCY_CHANGE" => Ok(Self::DependencyChange),
            _ => Err(SnapshotError::MalformedReference),
        }
    }
}

/// Stable rejection and replay failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    MalformedReference,
    ForbiddenContent,
    MissingBlocker,
    UnexpectedBlocker,
    GenerationRollback,
    DuplicateWorkerIdentity,
}

/// One closed dependency identity stored inside the existing bounded blocker
/// scalar. Branch and next action are redundant inputs at the wire boundary so
/// substitution is rejected, but only their canonical derivation is retained.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyBinding {
    parent_task_id: String,
    dependency_task_id: String,
    dependency_worktree_id: String,
    dependency_branch: String,
    base_sha: String,
    blocker_ref: String,
    evidence_ref: String,
}

impl DependencyBinding {
    /// Constructs one exact dependency binding.
    ///
    /// # Errors
    ///
    /// Rejects malformed identifiers, a substituted branch/base/next action,
    /// or a value that cannot fit the existing durable 256-byte scalar.
    pub fn new(
        parent_task_id: impl Into<String>,
        dependency_task_id: impl Into<String>,
        dependency_worktree_id: impl Into<String>,
        dependency_branch: impl Into<String>,
        base_sha: impl Into<String>,
        next_action: &str,
    ) -> Result<Self, SnapshotError> {
        let parent_task_id = dependency_task_identifier(parent_task_id.into())?;
        let dependency_task_id = dependency_task_identifier(dependency_task_id.into())?;
        if parent_task_id == dependency_task_id {
            return Err(SnapshotError::MalformedReference);
        }
        let dependency_worktree_id = dependency_worktree_identifier(dependency_worktree_id.into())?;
        let expected_branch = format!("lattice/{}", dependency_task_id.to_ascii_lowercase());
        if dependency_branch.into() != expected_branch || next_action != "COMPLETE_DEPENDENCY" {
            return Err(SnapshotError::MalformedReference);
        }
        let base_sha = base_sha.into();
        if !is_lower_hex(&base_sha, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        let blocker_ref = format!(
            "{DEPENDENCY_BLOCKER_PREFIX}{parent_task_id}:{dependency_task_id}:{dependency_worktree_id}:{base_sha}"
        );
        bounded_reference(blocker_ref.clone())?;
        let domain = HashDomain::new("lattice.foreman-dependency-binding", "1.0")
            .map_err(|_| SnapshotError::MalformedReference)?;
        let evidence_ref = format!(
            "evidence:sha256:{}",
            canonical_sha256(&domain, &CanonicalValue::String(blocker_ref.clone()))
                .map_err(|_| SnapshotError::MalformedReference)?
                .to_hex()
        );
        Ok(Self {
            parent_task_id,
            dependency_task_id,
            dependency_worktree_id,
            dependency_branch: expected_branch,
            base_sha,
            blocker_ref,
            evidence_ref,
        })
    }

    /// Parses only the versioned dependency namespace. Legacy blocker strings
    /// remain opaque and return `None`.
    ///
    /// # Errors
    ///
    /// Only the complete canonical v1 encoding is promoted. A colliding
    /// historical free-form string remains an opaque legacy blocker.
    pub fn from_blocker_ref(value: &str) -> Result<Option<Self>, SnapshotError> {
        if !value.starts_with(DEPENDENCY_BLOCKER_PREFIX) {
            return Ok(None);
        }
        let fields = value.split(':').collect::<Vec<_>>();
        if fields.len() != 6 || fields[0] != "dependency" || fields[1] != "v1" {
            return Ok(None);
        }
        let Ok(binding) = Self::new(
            fields[2],
            fields[3],
            fields[4],
            format!("lattice/{}", fields[3].to_ascii_lowercase()),
            fields[5],
            "COMPLETE_DEPENDENCY",
        ) else {
            return Ok(None);
        };
        if binding.as_blocker_ref() != value {
            return Ok(None);
        }
        Ok(Some(binding))
    }

    #[must_use]
    pub fn parent_task_id(&self) -> &str {
        &self.parent_task_id
    }

    #[must_use]
    pub fn dependency_task_id(&self) -> &str {
        &self.dependency_task_id
    }

    #[must_use]
    pub fn dependency_worktree_id(&self) -> &str {
        &self.dependency_worktree_id
    }

    #[must_use]
    pub fn dependency_branch(&self) -> &str {
        &self.dependency_branch
    }

    #[must_use]
    pub fn base_sha(&self) -> &str {
        &self.base_sha
    }

    #[must_use]
    pub const fn next_action(&self) -> &'static str {
        "COMPLETE_DEPENDENCY"
    }

    #[must_use]
    pub fn as_blocker_ref(&self) -> &str {
        &self.blocker_ref
    }

    #[must_use]
    pub fn evidence_ref(&self) -> &str {
        &self.evidence_ref
    }
}

/// Restart-restored phase for the most recent structured dependency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DependencyContinuationState {
    Blocked,
    Resumed,
}

/// Pure projection of one dependency relation from verified snapshot history.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyContinuation {
    binding: DependencyBinding,
    parent_branch: String,
    parent_worktree: String,
    state: DependencyContinuationState,
}

impl DependencyContinuation {
    #[must_use]
    pub const fn state(&self) -> DependencyContinuationState {
        self.state
    }

    #[must_use]
    pub fn parent_task_id(&self) -> &str {
        self.binding.parent_task_id()
    }

    #[must_use]
    pub fn dependency_task_id(&self) -> &str {
        self.binding.dependency_task_id()
    }

    #[must_use]
    pub fn parent_branch(&self) -> &str {
        &self.parent_branch
    }

    #[must_use]
    pub fn parent_worktree(&self) -> &str {
        &self.parent_worktree
    }

    #[must_use]
    pub fn dependency_branch(&self) -> &str {
        self.binding.dependency_branch()
    }

    #[must_use]
    pub fn dependency_worktree_id(&self) -> &str {
        self.binding.dependency_worktree_id()
    }

    #[must_use]
    pub fn base_sha(&self) -> &str {
        self.binding.base_sha()
    }

    #[must_use]
    pub const fn next_action(&self) -> &'static str {
        match self.state {
            DependencyContinuationState::Blocked => "COMPLETE_DEPENDENCY",
            DependencyContinuationState::Resumed => "CONTINUE_PARENT",
        }
    }
}

/// Caller-owned, closed checkpoint intent. Server identity, Git and Writer
/// authority are deliberately absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanCheckpointIntent {
    checkpoint_id: String,
    generation: u64,
    occurred_at: String,
    state: ForemanState,
    blocker_ref: Option<String>,
    heartbeat_ref: String,
    evidence_ref: String,
}

impl ForemanCheckpointIntent {
    /// # Errors
    ///
    /// Rejects unsafe IDs, zero generation, non-canonical time, malformed
    /// lowercase digest pointers, and state/blocker mismatch.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        checkpoint_id: impl Into<String>,
        generation: u64,
        occurred_at: impl Into<String>,
        state: ForemanState,
        blocker_ref: Option<String>,
        heartbeat_ref: impl Into<String>,
        evidence_ref: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let checkpoint_id = checkpoint_identifier(checkpoint_id.into())?;
        if generation == 0 {
            return Err(SnapshotError::GenerationRollback);
        }
        let occurred_at = timestamp(occurred_at.into())?;
        let heartbeat_ref = lowercase_digest_pointer(heartbeat_ref.into(), "heartbeat")?;
        let evidence_ref = lowercase_digest_pointer(evidence_ref.into(), "evidence")?;
        let blocker_ref = blocker_ref.map(bounded_reference).transpose()?;
        if let Some(blocker) = blocker_ref.as_deref()
            && let Some(binding) = DependencyBinding::from_blocker_ref(blocker)?
            && binding.evidence_ref() != evidence_ref
        {
            return Err(SnapshotError::MalformedReference);
        }
        match (state, blocker_ref.is_some()) {
            (ForemanState::Blocked, false) => return Err(SnapshotError::MissingBlocker),
            (ForemanState::Active | ForemanState::Completed, true) => {
                return Err(SnapshotError::UnexpectedBlocker);
            }
            _ => {}
        }
        Ok(Self {
            checkpoint_id,
            generation,
            occurred_at,
            state,
            blocker_ref,
            heartbeat_ref,
            evidence_ref,
        })
    }

    #[must_use]
    pub fn checkpoint_id(&self) -> &str {
        &self.checkpoint_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn occurred_at(&self) -> &str {
        &self.occurred_at
    }

    #[must_use]
    pub const fn state(&self) -> ForemanState {
        self.state
    }

    #[must_use]
    pub fn blocker_ref(&self) -> Option<&str> {
        self.blocker_ref.as_deref()
    }

    #[must_use]
    pub fn heartbeat_ref(&self) -> &str {
        &self.heartbeat_ref
    }

    #[must_use]
    pub fn evidence_ref(&self) -> &str {
        &self.evidence_ref
    }

    /// Matches only caller-owned fields against one retained server snapshot.
    #[must_use]
    pub fn matches_snapshot(&self, snapshot: &ForemanSnapshot) -> bool {
        self.generation == snapshot.generation()
            && self.state == snapshot.state()
            && self.blocker_ref() == snapshot.blocker()
            && self.heartbeat_ref == snapshot.heartbeat()
            && self.evidence_ref == snapshot.evidence()
    }
}

/// Server-owned binding and Git observation made only after replay proves that
/// a checkpoint is new. Writer authority is attached later by Orchestrator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanServerObservation {
    worker: String,
    thread: String,
    task: String,
    branch: String,
    worktree: String,
    head: String,
}

impl ForemanServerObservation {
    /// # Errors
    ///
    /// Rejects malformed fixed identity or Git evidence.
    pub fn new(
        worker: impl Into<String>,
        thread: impl Into<String>,
        task: impl Into<String>,
        branch: impl Into<String>,
        worktree: impl Into<String>,
        head: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        Ok(Self {
            worker: bounded_reference(worker.into())?,
            thread: bounded_reference(thread.into())?,
            task: bounded_reference(task.into())?,
            branch: bounded_reference(branch.into())?,
            worktree: bounded_reference(worktree.into())?,
            head,
        })
    }

    /// Binds caller intent to the newly acquired Writer authority.
    ///
    /// # Errors
    ///
    /// Rejects malformed authority evidence or any impossible snapshot shape.
    pub fn bind(
        self,
        intent: &ForemanCheckpointIntent,
        authority_ref: impl Into<String>,
    ) -> Result<ForemanSnapshot, SnapshotError> {
        ForemanSnapshot::new(
            self.worker,
            self.thread,
            self.task,
            self.branch,
            self.worktree,
            self.head,
            intent.state(),
            intent.blocker_ref().map(str::to_owned),
            intent.heartbeat_ref(),
            lowercase_digest_pointer(authority_ref.into(), "authority")?,
            intent.evidence_ref(),
            intent.generation(),
        )
    }
}

/// One versioned, bounded coordination record. It deliberately has no free-form
/// transcript, command, path, environment, or credential field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanSnapshot {
    worker: String,
    thread: String,
    task: String,
    branch: String,
    worktree: String,
    head: String,
    state: ForemanState,
    blocker: Option<String>,
    heartbeat: String,
    authority: String,
    evidence: String,
    generation: u64,
    epistemic: Option<EpistemicReferences>,
}

impl ForemanSnapshot {
    /// # Errors
    ///
    /// Returns a typed rejection for malformed, secret-bearing, or
    /// state-incompatible fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        worker: impl Into<String>,
        thread: impl Into<String>,
        task: impl Into<String>,
        branch: impl Into<String>,
        worktree: impl Into<String>,
        head: impl Into<String>,
        state: ForemanState,
        blocker: Option<String>,
        heartbeat: impl Into<String>,
        authority: impl Into<String>,
        evidence: impl Into<String>,
        generation: u64,
    ) -> Result<Self, SnapshotError> {
        let worker = bounded_reference(worker.into())?;
        let thread = bounded_reference(thread.into())?;
        let task = bounded_reference(task.into())?;
        let branch = bounded_reference(branch.into())?;
        let worktree = bounded_reference(worktree.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        let heartbeat = digest_pointer(heartbeat.into(), "heartbeat")?;
        let authority = digest_pointer(authority.into(), "authority")?;
        let evidence = digest_pointer(evidence.into(), "evidence")?;
        if generation == 0 {
            return Err(SnapshotError::GenerationRollback);
        }
        let blocker = blocker.map(bounded_reference).transpose()?;
        if let Some(blocker) = blocker.as_deref() {
            DependencyBinding::from_blocker_ref(blocker)?;
        }
        match (state, blocker.is_some()) {
            (ForemanState::Blocked, false) => return Err(SnapshotError::MissingBlocker),
            (ForemanState::Active | ForemanState::Completed, true) => {
                return Err(SnapshotError::UnexpectedBlocker);
            }
            _ => {}
        }
        Ok(Self {
            worker,
            thread,
            task,
            branch,
            worktree,
            head,
            state,
            blocker,
            heartbeat,
            authority,
            evidence,
            generation,
            epistemic: None,
        })
    }

    /// Attaches only expiring, non-authoritative pointers to this snapshot.
    ///
    /// # Errors
    ///
    /// Rejects an epistemic record whose lifetime has already expired.
    pub fn with_epistemic(mut self, epistemic: EpistemicReferences) -> Result<Self, SnapshotError> {
        if epistemic.expires_at <= epistemic.checked_at {
            return Err(SnapshotError::MalformedReference);
        }
        self.epistemic = Some(epistemic);
        Ok(self)
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        SNAPSHOT_SCHEMA
    }

    #[must_use]
    pub fn worker(&self) -> &str {
        &self.worker
    }

    #[must_use]
    pub fn thread(&self) -> &str {
        &self.thread
    }

    #[must_use]
    pub fn task(&self) -> &str {
        &self.task
    }

    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    #[must_use]
    pub fn worktree(&self) -> &str {
        &self.worktree
    }

    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }

    #[must_use]
    pub const fn state(&self) -> ForemanState {
        self.state
    }

    #[must_use]
    pub fn blocker(&self) -> Option<&str> {
        self.blocker.as_deref()
    }

    #[must_use]
    pub fn heartbeat(&self) -> &str {
        &self.heartbeat
    }

    /// Digest pointer to the authority receipt/head used for this report.
    #[must_use]
    pub fn authority(&self) -> &str {
        &self.authority
    }

    #[must_use]
    pub fn evidence(&self) -> &str {
        &self.evidence
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns provisional pointers only; callers must not use them as state.
    #[must_use]
    pub const fn epistemic(&self) -> Option<&EpistemicReferences> {
        self.epistemic.as_ref()
    }
}

/// One reconstructed blocked record. Blocked coordination never permits archive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockedWorker {
    snapshot: ForemanSnapshot,
}

impl BlockedWorker {
    #[must_use]
    pub const fn archive_ready(&self) -> bool {
        false
    }
}

/// Fresh-reader projection over verified ordered snapshot events.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForemanProjection {
    active: Vec<ForemanSnapshot>,
    blocked: Vec<BlockedWorker>,
    completed: Vec<ForemanSnapshot>,
    latest_generation: u64,
    next_action: String,
    dependency: Option<DependencyContinuation>,
}

impl ForemanProjection {
    #[must_use]
    pub fn active(&self) -> &[ForemanSnapshot] {
        &self.active
    }

    #[must_use]
    pub fn blocked(&self) -> &[BlockedWorker] {
        &self.blocked
    }

    #[must_use]
    pub fn completed(&self) -> &[ForemanSnapshot] {
        &self.completed
    }

    #[must_use]
    pub const fn latest_generation(&self) -> u64 {
        self.latest_generation
    }

    #[must_use]
    pub const fn runtime_next_action(&self) -> &'static str {
        if !self.blocked.is_empty() {
            "RESOLVE_BLOCKERS"
        } else if !self.active.is_empty() {
            "CONTINUE"
        } else if !self.completed.is_empty() {
            "ALL_COMPLETED"
        } else {
            "NO_DURABLE_SNAPSHOT"
        }
    }

    #[must_use]
    pub fn next_action(&self) -> &str {
        &self.next_action
    }

    #[must_use]
    pub const fn dependency(&self) -> Option<&DependencyContinuation> {
        self.dependency.as_ref()
    }
}

/// Reconstructs the current worker projection from append order without I/O.
///
/// # Errors
///
/// Rejects duplicate worker ownership and non-monotonic generations.
pub fn reconstruct(
    snapshots: impl IntoIterator<Item = ForemanSnapshot>,
) -> Result<ForemanProjection, SnapshotError> {
    let mut by_worker = BTreeMap::<String, ForemanSnapshot>::new();
    let mut dependency = None::<(String, DependencyContinuation)>;
    for snapshot in snapshots {
        if let Some(previous) = by_worker.get(snapshot.worker()) {
            if previous.thread() != snapshot.thread() {
                return Err(SnapshotError::DuplicateWorkerIdentity);
            }
            if !is_exact_next_generation(Some(previous.generation()), snapshot.generation()) {
                return Err(SnapshotError::GenerationRollback);
            }
        } else if !is_exact_next_generation(None, snapshot.generation()) {
            return Err(SnapshotError::GenerationRollback);
        }
        if snapshot.state() == ForemanState::Blocked {
            if let Some(blocker) = snapshot.blocker()
                && let Some(binding) = DependencyBinding::from_blocker_ref(blocker)?
                && binding.evidence_ref() == snapshot.evidence()
            {
                if binding.base_sha() != snapshot.head() {
                    return Err(SnapshotError::MalformedReference);
                }
                if let Some((worker, current)) = dependency.as_ref()
                    && (worker != snapshot.worker()
                        || (current.state == DependencyContinuationState::Blocked
                            && current.binding != binding))
                {
                    return Err(SnapshotError::DuplicateWorkerIdentity);
                }
                dependency = Some((
                    snapshot.worker().to_owned(),
                    DependencyContinuation {
                        binding,
                        parent_branch: snapshot.branch().to_owned(),
                        parent_worktree: snapshot.worktree().to_owned(),
                        state: DependencyContinuationState::Blocked,
                    },
                ));
            }
        } else if let Some((worker, current)) = dependency.as_mut()
            && worker == snapshot.worker()
        {
            match (snapshot.state(), current.state) {
                (ForemanState::Active, DependencyContinuationState::Blocked) => {
                    current.state = DependencyContinuationState::Resumed;
                }
                (ForemanState::Completed, DependencyContinuationState::Blocked) => {
                    return Err(SnapshotError::MalformedReference);
                }
                _ => {}
            }
        }
        by_worker.insert(snapshot.worker().to_owned(), snapshot);
    }
    let mut active = Vec::new();
    let mut blocked = Vec::new();
    let mut completed = Vec::new();
    let mut latest_generation = 0;
    for snapshot in by_worker.into_values() {
        latest_generation = latest_generation.max(snapshot.generation());
        match snapshot.state() {
            ForemanState::Active => active.push(snapshot),
            ForemanState::Blocked => blocked.push(BlockedWorker { snapshot }),
            ForemanState::Completed => completed.push(snapshot),
        }
    }
    let next_action = if let Some(blocked_worker) = blocked.first() {
        format!(
            "unblock {}: {}",
            blocked_worker.snapshot.worker(),
            blocked_worker.snapshot.blocker().unwrap_or_default(),
        )
    } else if let Some(active_worker) = active.first() {
        format!("await {}", active_worker.worker())
    } else {
        "no active worker".to_owned()
    };
    Ok(ForemanProjection {
        active,
        blocked,
        completed,
        latest_generation,
        next_action,
        dependency: dependency.map(|(_, continuation)| continuation),
    })
}

/// Returns whether `candidate` is the only allowed generation after `previous`.
/// An empty identity starts at one, and overflow never wraps to a valid value.
#[must_use]
pub const fn is_exact_next_generation(previous: Option<u64>, candidate: u64) -> bool {
    match previous {
        None => candidate == 1,
        Some(previous) => match previous.checked_add(1) {
            Some(expected) => candidate == expected,
            None => false,
        },
    }
}

/// Read-only dashboard metadata; it is never a durable authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DashboardIndex {
    generated_at: String,
    branch: String,
    head: String,
    outcome: String,
}

impl DashboardIndex {
    /// # Errors
    ///
    /// Rejects malformed bounded dashboard index values.
    pub fn new(
        generated_at: impl Into<String>,
        branch: impl Into<String>,
        head: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Result<Self, SnapshotError> {
        let generated_at = bounded_reference(generated_at.into())?;
        let branch = bounded_reference(branch.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        let outcome = bounded_reference(outcome.into())?;
        Ok(Self {
            generated_at,
            branch,
            head,
            outcome,
        })
    }
}

/// Independently collected current worktree facts, injected by a later adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiveWorktree {
    worker: String,
    branch: String,
    head: String,
    heartbeat_fresh: bool,
}

impl LiveWorktree {
    /// # Errors
    ///
    /// Rejects malformed bounded live worktree values.
    pub fn new(
        worker: impl Into<String>,
        branch: impl Into<String>,
        head: impl Into<String>,
        heartbeat_fresh: bool,
    ) -> Result<Self, SnapshotError> {
        let worker = bounded_reference(worker.into())?;
        let branch = bounded_reference(branch.into())?;
        let head = head.into();
        if !is_hex(&head, 40) {
            return Err(SnapshotError::MalformedReference);
        }
        Ok(Self {
            worker,
            branch,
            head,
            heartbeat_fresh,
        })
    }
}

/// Fail-closed watchdog results.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WatchdogFinding {
    AllWorkersMissedHeartbeat,
    OldHead { worker: String },
    DashboardDrift,
}

/// Compares untrusted dashboard metadata with injected live observations.
///
/// # Errors
///
/// Rejects a snapshot with no exact independently supplied live worker.
pub fn watchdog(
    snapshots: &[ForemanSnapshot],
    dashboard: &DashboardIndex,
    live: &[LiveWorktree],
) -> Result<Vec<WatchdogFinding>, SnapshotError> {
    let mut findings = Vec::new();
    if !live.is_empty() && live.iter().all(|item| !item.heartbeat_fresh) {
        findings.push(WatchdogFinding::AllWorkersMissedHeartbeat);
    }
    for snapshot in snapshots {
        let item = live
            .iter()
            .find(|candidate| candidate.worker == snapshot.worker());
        let Some(item) = item else {
            return Err(SnapshotError::DuplicateWorkerIdentity);
        };
        if item.branch != snapshot.branch() || item.head != snapshot.head() {
            findings.push(WatchdogFinding::OldHead {
                worker: snapshot.worker().to_owned(),
            });
        }
        if (dashboard.branch != item.branch
            || dashboard.head != item.head
            || dashboard.outcome != snapshot.state().as_str())
            && !findings.contains(&WatchdogFinding::DashboardDrift)
        {
            findings.push(WatchdogFinding::DashboardDrift);
        }
    }
    Ok(findings)
}

impl ForemanState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Blocked => "BLOCKED",
            Self::Completed => "COMPLETED",
        }
    }

    /// Parses the closed persistence spelling.
    ///
    /// # Errors
    ///
    /// Rejects every unknown or case-substituted value.
    pub fn from_persisted(value: &str) -> Result<Self, SnapshotError> {
        match value {
            "ACTIVE" => Ok(Self::Active),
            "BLOCKED" => Ok(Self::Blocked),
            "COMPLETED" => Ok(Self::Completed),
            _ => Err(SnapshotError::MalformedReference),
        }
    }
}

fn bounded_reference(value: String) -> Result<String, SnapshotError> {
    if value.is_empty()
        || value.len() > MAX_REFERENCE_BYTES
        || !value.is_ascii()
        || value.contains(char::is_whitespace)
        || looks_secret_like(&value)
    {
        return Err(if looks_secret_like(&value) {
            SnapshotError::ForbiddenContent
        } else {
            SnapshotError::MalformedReference
        });
    }
    Ok(value)
}

fn digest_pointer(value: String, prefix: &str) -> Result<String, SnapshotError> {
    let expected_prefix = format!("{prefix}:sha256:");
    if !value.starts_with(&expected_prefix) || !is_hex(&value[expected_prefix.len()..], 64) {
        return Err(if looks_secret_like(&value) {
            SnapshotError::ForbiddenContent
        } else {
            SnapshotError::MalformedReference
        });
    }
    Ok(value)
}

fn lowercase_digest_pointer(value: String, prefix: &str) -> Result<String, SnapshotError> {
    let expected_prefix = format!("{prefix}:sha256:");
    let digest = value
        .strip_prefix(&expected_prefix)
        .ok_or(SnapshotError::MalformedReference)?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn checkpoint_identifier(value: String) -> Result<String, SnapshotError> {
    let mut bytes = value.bytes();
    if value.len() > MAX_CHECKPOINT_ID_BYTES
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        || !bytes
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn dependency_task_identifier(value: String) -> Result<String, SnapshotError> {
    let suffix = value
        .strip_prefix("TASK-")
        .ok_or(SnapshotError::MalformedReference)?;
    let mut bytes = suffix.bytes();
    if value.len() > 64
        || suffix.len() < 3
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        || !bytes.all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn dependency_worktree_identifier(value: String) -> Result<String, SnapshotError> {
    let mut bytes = value.bytes();
    if !(3..=64).contains(&value.len())
        || !bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        || !bytes.all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn pointer_list(values: Vec<String>, prefix: &str) -> Result<Vec<String>, SnapshotError> {
    if values.len() > 64 {
        return Err(SnapshotError::MalformedReference);
    }
    values
        .into_iter()
        .map(|value| digest_pointer(value, prefix))
        .collect()
}

fn timestamp(value: String) -> Result<String, SnapshotError> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
        || !bytes
            .iter()
            .enumerate()
            .filter(|(index, _)| !matches!(index, 4 | 7 | 10 | 13 | 16 | 19))
            .all(|(_, byte)| byte.is_ascii_digit())
    {
        return Err(SnapshotError::MalformedReference);
    }
    let parsed =
        OffsetDateTime::parse(&value, &Rfc3339).map_err(|_| SnapshotError::MalformedReference)?;
    if parsed
        .format(&Rfc3339)
        .map_err(|_| SnapshotError::MalformedReference)?
        != value
    {
        return Err(SnapshotError::MalformedReference);
    }
    Ok(value)
}

fn is_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn looks_secret_like(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    lowercase.starts_with("sk-")
        || lowercase.starts_with("bearer ")
        || lowercase.contains("password")
        || lowercase.contains("full chat")
        || lowercase.contains("begin private")
}
