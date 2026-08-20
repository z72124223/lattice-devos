//! Fail-closed, side-effect-free workflow-learning contracts.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions
)]

use std::collections::{BTreeMap, BTreeSet};

const MAX_REFERENCE_BYTES: usize = 160;
const EPISTEMIC_SCHEMA: &str = "lattice.workflow-epistemic-record/1.0";
const ZERO_WORK_SCHEMA: &str = "lattice.zero-work-certificate/1.0";

/// Stable validation and state-machine failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LearningError {
    MalformedReference,
    ForbiddenContent,
    MissingEvidence,
    InvalidEpistemicWindow,
    InvalidProbe,
    MissingMachineGate,
    InsufficientReplayCases,
    StalePolicyFence,
    CostLimitExceeded,
    ProtectedAuthority,
    InvalidPolicyCandidate,
    InvalidCheckpoint,
}

/// Evidence classes stay explicit so projections and reports never impersonate truth.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EvidenceSource {
    TaskLedger,
    Ticket,
    Plans,
    Dashboard,
    GitWorktree,
    DependencyGraph,
    BacklogInventory,
    ToolInventory,
    ModelCatalog,
    Sidebar,
    UntrustedReport,
}

impl EvidenceSource {
    const STOP_CERTIFICATE_REQUIRED: [Self; 9] = [
        Self::TaskLedger,
        Self::Ticket,
        Self::Plans,
        Self::Dashboard,
        Self::GitWorktree,
        Self::DependencyGraph,
        Self::BacklogInventory,
        Self::ToolInventory,
        Self::ModelCatalog,
    ];

    const fn trusted_for_learning(self) -> bool {
        !matches!(self, Self::Sidebar | Self::UntrustedReport)
    }
}

/// A bounded pointer to evidence; payload bytes are deliberately absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidencePointer {
    source: EvidenceSource,
    digest: String,
    fresh: bool,
}

impl EvidencePointer {
    pub fn new(
        source: EvidenceSource,
        digest: impl Into<String>,
        fresh: bool,
    ) -> Result<Self, LearningError> {
        let digest = digest.into();
        let Some(hex) = digest.strip_prefix("sha256:") else {
            return Err(LearningError::MalformedReference);
        };
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(LearningError::MalformedReference);
        }
        Ok(Self {
            source,
            digest,
            fresh,
        })
    }

    #[must_use]
    pub const fn source(&self) -> EvidenceSource {
        self.source
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    #[must_use]
    pub const fn fresh(&self) -> bool {
        self.fresh
    }

    const fn trusted_and_fresh(&self) -> bool {
        self.fresh && self.source.trusted_for_learning()
    }
}

/// Closed observations describe what was seen, not why it happened.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationCode {
    ZeroWorkClaim,
    UnknownWindow,
    IdleWindow,
    StaleDashboard,
    ReadyBacklog,
    DependencyReleased,
    ToolPathFailure,
    RealDependencyBlock,
    ResourceConflict,
    DuplicateWork,
}

/// One idempotent, secret-free observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation {
    event_id: String,
    incident_id: String,
    code: ObservationCode,
    evidence: Vec<EvidencePointer>,
}

impl Observation {
    pub fn new(
        event_id: impl Into<String>,
        incident_id: impl Into<String>,
        code: ObservationCode,
        evidence: Vec<EvidencePointer>,
    ) -> Result<Self, LearningError> {
        let event_id = bounded_reference(event_id.into())?;
        let incident_id = bounded_reference(incident_id.into())?;
        if evidence.is_empty() {
            return Err(LearningError::MissingEvidence);
        }
        Ok(Self {
            event_id,
            incident_id,
            code,
            evidence,
        })
    }

    #[must_use]
    pub fn event_id(&self) -> &str {
        &self.event_id
    }

    #[must_use]
    pub fn incident_id(&self) -> &str {
        &self.incident_id
    }

    #[must_use]
    pub const fn code(&self) -> ObservationCode {
        self.code
    }

    #[must_use]
    pub fn evidence(&self) -> &[EvidencePointer] {
        &self.evidence
    }
}

/// Idempotent observation result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObserveOutcome {
    Recorded,
    DuplicateIgnored,
}

/// Closed, preliminary diagnosis evidence. It never becomes authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnosis {
    incident_id: String,
    code: ObservationCode,
}

impl Diagnosis {
    #[must_use]
    pub fn incident_id(&self) -> &str {
        &self.incident_id
    }

    #[must_use]
    pub const fn code(&self) -> ObservationCode {
        self.code
    }
}

/// In-memory characterization log. It is not durable truth.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LearningLog {
    observations: BTreeMap<String, Observation>,
}

impl LearningLog {
    pub fn observe(&mut self, observation: Observation) -> ObserveOutcome {
        if self.observations.contains_key(observation.event_id()) {
            return ObserveOutcome::DuplicateIgnored;
        }
        self.observations
            .insert(observation.event_id().to_owned(), observation);
        ObserveOutcome::Recorded
    }

    #[must_use]
    pub fn diagnose(&self) -> Vec<Diagnosis> {
        self.observations
            .values()
            .filter(|observation| {
                observation
                    .evidence()
                    .iter()
                    .filter(|pointer| pointer.trusted_and_fresh())
                    .map(EvidencePointer::source)
                    .collect::<BTreeSet<_>>()
                    .len()
                    >= 2
            })
            .map(|observation| Diagnosis {
                incident_id: observation.incident_id().to_owned(),
                code: observation.code(),
            })
            .collect()
    }
}

/// Reasonable competing explanations. None is true merely because it is present.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Hypothesis {
    NoDispatchableWork,
    UnopenedBacklog,
    StaleProjection,
    RealDependencyBlock,
    ResourceConflict,
    DuplicateWork,
    ToolCoverageFailure,
}

/// Confidence is explicitly degradable and never substitutes for evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Confidence {
    Unknown,
    Low,
    Medium,
    High,
}

impl Confidence {
    const fn degrade(self) -> Self {
        match self {
            Self::High => Self::Medium,
            Self::Medium => Self::Low,
            Self::Low | Self::Unknown => Self::Unknown,
        }
    }
}

/// Current fallible decision attached to an epistemic record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpistemicDecision {
    Unknown,
    Wait,
    Refresh,
    Probe,
    Reconcile,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Scope {
    ForemanDispatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshTrigger {
    EvidenceExpired,
    Counterevidence,
    LowConfidence,
    DependencyChanged,
    ToolFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    ExhaustiveNoReadyCandidates,
    ReadyTicketFound,
    EvidenceContradiction,
    ToolFailure,
    DependencyChanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeRisk {
    ReadOnly,
    ReversibleLocal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopCondition {
    FirstContradiction,
    CostLimit,
    RiskLimit,
}

/// A reversible probe declares competing hypotheses and hard bounds first.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbePlan {
    first: Hypothesis,
    second: Hypothesis,
    max_cost_units: u32,
    risk: ProbeRisk,
    stop_condition: StopCondition,
}

impl ProbePlan {
    pub fn new(
        first: Hypothesis,
        second: Hypothesis,
        max_cost_units: u32,
        risk: ProbeRisk,
        stop_condition: StopCondition,
    ) -> Result<Self, LearningError> {
        if first == second || max_cost_units == 0 {
            return Err(LearningError::InvalidProbe);
        }
        Ok(Self {
            first,
            second,
            max_cost_units,
            risk,
            stop_condition,
        })
    }

    #[must_use]
    pub const fn hypotheses(&self) -> (Hypothesis, Hypothesis) {
        (self.first, self.second)
    }

    #[must_use]
    pub const fn max_cost_units(&self) -> u32 {
        self.max_cost_units
    }

    #[must_use]
    pub const fn risk(&self) -> ProbeRisk {
        self.risk
    }

    #[must_use]
    pub const fn stop_condition(&self) -> StopCondition {
        self.stop_condition
    }
}

/// Typed probe evidence may support or falsify one hypothesis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeResult {
    Supported {
        hypothesis: Hypothesis,
        actual_signal: Signal,
        evidence: EvidencePointer,
    },
    Falsified {
        hypothesis: Hypothesis,
        actual_signal: Signal,
        evidence: EvidencePointer,
    },
}

/// Required input makes every epistemic field visible at construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpistemicRecordInput {
    pub id: String,
    pub observation: Observation,
    pub hypotheses: Vec<Hypothesis>,
    pub confidence: Confidence,
    pub unknowns: Vec<String>,
    pub supporting_evidence: Vec<EvidencePointer>,
    pub counterevidence: Vec<EvidencePointer>,
    pub assumptions: Vec<String>,
    pub scope: Scope,
    pub checked_at: u64,
    pub expires_at: u64,
    pub refresh_triggers: Vec<RefreshTrigger>,
    pub decision: EpistemicDecision,
    pub expected_signal: Signal,
    pub actual_signal: Option<Signal>,
    pub falsifier: Signal,
    pub next_probe: ProbePlan,
}

/// Expiring knowledge record; it is never an eternal policy fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpistemicRecord {
    id: String,
    observation: Observation,
    hypotheses: Vec<Hypothesis>,
    confidence: Confidence,
    unknowns: Vec<String>,
    supporting_evidence: Vec<EvidencePointer>,
    counterevidence: Vec<EvidencePointer>,
    assumptions: Vec<String>,
    scope: Scope,
    checked_at: u64,
    expires_at: u64,
    refresh_triggers: Vec<RefreshTrigger>,
    decision: EpistemicDecision,
    expected_signal: Signal,
    actual_signal: Option<Signal>,
    falsifier: Signal,
    next_probe: ProbePlan,
}

impl EpistemicRecord {
    pub fn new(input: EpistemicRecordInput) -> Result<Self, LearningError> {
        let id = bounded_reference(input.id)?;
        if input.expires_at <= input.checked_at {
            return Err(LearningError::InvalidEpistemicWindow);
        }
        let hypotheses = input.hypotheses.iter().copied().collect::<BTreeSet<_>>();
        let (first, second) = input.next_probe.hypotheses();
        if hypotheses.len() < 2
            || !hypotheses.contains(&first)
            || !hypotheses.contains(&second)
            || input.supporting_evidence.is_empty()
            || input.refresh_triggers.is_empty()
        {
            return Err(LearningError::InvalidProbe);
        }
        if input
            .supporting_evidence
            .iter()
            .chain(&input.counterevidence)
            .any(|pointer| !pointer.trusted_and_fresh())
        {
            return Err(LearningError::MissingEvidence);
        }
        validate_references(&input.unknowns)?;
        validate_references(&input.assumptions)?;
        Ok(Self {
            id,
            observation: input.observation,
            hypotheses: input.hypotheses,
            confidence: input.confidence,
            unknowns: input.unknowns,
            supporting_evidence: input.supporting_evidence,
            counterevidence: input.counterevidence,
            assumptions: input.assumptions,
            scope: input.scope,
            checked_at: input.checked_at,
            expires_at: input.expires_at,
            refresh_triggers: input.refresh_triggers,
            decision: input.decision,
            expected_signal: input.expected_signal,
            actual_signal: input.actual_signal,
            falsifier: input.falsifier,
            next_probe: input.next_probe,
        })
    }

    #[must_use]
    pub const fn schema(&self) -> &'static str {
        EPISTEMIC_SCHEMA
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn observation(&self) -> &Observation {
        &self.observation
    }

    #[must_use]
    pub fn hypotheses(&self) -> &[Hypothesis] {
        &self.hypotheses
    }

    #[must_use]
    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }

    #[must_use]
    pub fn unknowns(&self) -> &[String] {
        &self.unknowns
    }

    #[must_use]
    pub fn supporting_evidence(&self) -> &[EvidencePointer] {
        &self.supporting_evidence
    }

    #[must_use]
    pub fn counterevidence(&self) -> &[EvidencePointer] {
        &self.counterevidence
    }

    #[must_use]
    pub fn assumptions(&self) -> &[String] {
        &self.assumptions
    }

    #[must_use]
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    #[must_use]
    pub const fn checked_at(&self) -> u64 {
        self.checked_at
    }

    #[must_use]
    pub const fn expires_at(&self) -> u64 {
        self.expires_at
    }

    #[must_use]
    pub fn refresh_triggers(&self) -> &[RefreshTrigger] {
        &self.refresh_triggers
    }

    #[must_use]
    pub const fn decision(&self) -> EpistemicDecision {
        self.decision
    }

    #[must_use]
    pub const fn expected_signal(&self) -> Signal {
        self.expected_signal
    }

    #[must_use]
    pub const fn actual_signal(&self) -> Option<Signal> {
        self.actual_signal
    }

    #[must_use]
    pub const fn falsifier(&self) -> Signal {
        self.falsifier
    }

    #[must_use]
    pub const fn next_probe(&self) -> &ProbePlan {
        &self.next_probe
    }

    #[must_use]
    pub const fn status_at(&self, now: u64) -> RecordStatus {
        if now > self.expires_at {
            RecordStatus::RefreshRequired(RefreshReason::Expired)
        } else if matches!(self.confidence, Confidence::Unknown | Confidence::Low) {
            RecordStatus::RefreshRequired(RefreshReason::LowConfidence)
        } else {
            RecordStatus::Fresh
        }
    }

    pub fn add_counterevidence(
        &mut self,
        evidence: EvidencePointer,
        checked_at: u64,
    ) -> Result<(), LearningError> {
        if !evidence.trusted_and_fresh() || checked_at > self.expires_at {
            return Err(LearningError::MissingEvidence);
        }
        self.counterevidence.push(evidence);
        self.checked_at = checked_at;
        self.confidence = self.confidence.degrade();
        self.decision = EpistemicDecision::Refresh;
        Ok(())
    }

    pub fn apply_probe(
        &mut self,
        result: ProbeResult,
        checked_at: u64,
    ) -> Result<(), LearningError> {
        if checked_at > self.expires_at {
            return Err(LearningError::InvalidEpistemicWindow);
        }
        let (hypothesis, actual_signal, evidence, falsified) = match result {
            ProbeResult::Supported {
                hypothesis,
                actual_signal,
                evidence,
            } => (hypothesis, actual_signal, evidence, false),
            ProbeResult::Falsified {
                hypothesis,
                actual_signal,
                evidence,
            } => (hypothesis, actual_signal, evidence, true),
        };
        if !self.hypotheses.contains(&hypothesis) || !evidence.trusted_and_fresh() {
            return Err(LearningError::InvalidProbe);
        }
        self.checked_at = checked_at;
        self.actual_signal = Some(actual_signal);
        if falsified {
            self.counterevidence.push(evidence);
            self.confidence = self.confidence.degrade();
            self.decision = EpistemicDecision::Reconcile;
        } else {
            self.supporting_evidence.push(evidence);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshReason {
    Expired,
    LowConfidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordStatus {
    Fresh,
    RefreshRequired(RefreshReason),
}

/// Domains are broad enough for product skepticism but remain closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuspicionDomain {
    ProductBehavior,
    IntegrationTestGap,
    Dependency,
    GitWorktreeTopology,
    TruthSourceContradiction,
    ResourceCost,
    SecurityAuthority,
    UserValue,
}

/// A suspicion remains a hypothesis until evidence changes its state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuspicionState {
    Hypothesis,
    Confirmed,
    Refuted,
}

/// Explicit priority inputs keep queue policy inspectable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuspicionInput {
    pub id: String,
    pub domain: SuspicionDomain,
    pub record: EpistemicRecord,
    pub user_value: u8,
    pub consequence_risk: u8,
    pub freshness: u8,
    pub probe_cost: u8,
    pub reversibility: u8,
    pub conflict: u8,
    pub governance_only: bool,
}

/// One expiring, ranked, still-refutable queue item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Suspicion {
    id: String,
    domain: SuspicionDomain,
    record: EpistemicRecord,
    user_value: u8,
    consequence_risk: u8,
    freshness: u8,
    probe_cost: u8,
    reversibility: u8,
    conflict: u8,
    governance_only: bool,
    state: SuspicionState,
}

impl Suspicion {
    pub fn new(input: SuspicionInput) -> Result<Self, LearningError> {
        let scores = [
            input.user_value,
            input.consequence_risk,
            input.freshness,
            input.probe_cost,
            input.reversibility,
            input.conflict,
        ];
        if scores.iter().any(|score| *score > 100) || input.probe_cost == 0 {
            return Err(LearningError::InvalidProbe);
        }
        Ok(Self {
            id: bounded_reference(input.id)?,
            domain: input.domain,
            record: input.record,
            user_value: input.user_value,
            consequence_risk: input.consequence_risk,
            freshness: input.freshness,
            probe_cost: input.probe_cost,
            reversibility: input.reversibility,
            conflict: input.conflict,
            governance_only: input.governance_only,
            state: SuspicionState::Hypothesis,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn domain(&self) -> SuspicionDomain {
        self.domain
    }

    #[must_use]
    pub const fn record(&self) -> &EpistemicRecord {
        &self.record
    }

    #[must_use]
    pub const fn state(&self) -> SuspicionState {
        self.state
    }

    #[must_use]
    pub const fn governance_only(&self) -> bool {
        self.governance_only
    }

    fn priority(&self) -> i32 {
        let base = i32::from(self.user_value) * 4
            + i32::from(self.consequence_risk) * 3
            + i32::from(self.freshness) * 2
            + i32::from(self.reversibility) * 2;
        let burden = i32::from(self.probe_cost) * 2 + i32::from(self.conflict) * 3;
        base - burden - if self.governance_only { 250 } else { 0 }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuspicionQueueOutcome {
    Queued,
    DuplicateFreshRejected,
}

/// Pure queue; it performs no probe or side effect itself.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SuspicionQueue {
    entries: BTreeMap<String, Suspicion>,
}

impl SuspicionQueue {
    pub fn enqueue(&mut self, suspicion: Suspicion, now: u64) -> SuspicionQueueOutcome {
        if self
            .entries
            .get(suspicion.id())
            .is_some_and(|existing| existing.record().status_at(now) == RecordStatus::Fresh)
        {
            return SuspicionQueueOutcome::DuplicateFreshRejected;
        }
        self.entries.insert(suspicion.id().to_owned(), suspicion);
        SuspicionQueueOutcome::Queued
    }

    #[must_use]
    pub fn next_probe(&self, now: u64) -> Option<&Suspicion> {
        let eligible = self
            .entries
            .values()
            .filter(|suspicion| {
                suspicion.state() == SuspicionState::Hypothesis
                    && suspicion.record().status_at(now) == RecordStatus::Fresh
            })
            .collect::<Vec<_>>();
        let product_work_available = eligible
            .iter()
            .any(|suspicion| !suspicion.governance_only());
        eligible
            .into_iter()
            .filter(|suspicion| !product_work_available || !suspicion.governance_only())
            .max_by_key(|suspicion| suspicion.priority())
    }

    pub fn refute(&mut self, id: &str) -> Result<(), LearningError> {
        let suspicion = self
            .entries
            .get_mut(id)
            .ok_or(LearningError::MalformedReference)?;
        suspicion.state = SuspicionState::Refuted;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopClaim {
    ZeroWork,
    AllWaiting,
    Healthy,
    EndRound,
    Completion,
}

/// The only supported repair is Git's registered, reversible worktree move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopologyAction {
    GitWorktreeMove,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopologyDecision {
    Healthy,
    RepairRequired(TopologyAction),
    RepairVerified,
}

/// Bounded evidence collected before making a topology judgment.
///
/// The booleans mirror independent machine checks in an auditable receipt.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeTopologyInput {
    pub worktree: String,
    pub branch: String,
    pub head: String,
    pub resolved_path: EvidencePointer,
    pub approved_root: EvidencePointer,
    pub direct_child_of_approved_root: bool,
    pub nested_ancestor: Option<String>,
    pub reparse_escape: bool,
    pub child_clean: bool,
    pub parent_clean_before: bool,
    pub parent_clean_after: Option<bool>,
}

/// Epistemic topology record; it does not mutate Git or the filesystem.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorktreeTopology {
    worktree: String,
    branch: String,
    head: String,
    resolved_path: EvidencePointer,
    approved_root: EvidencePointer,
    direct_child_of_approved_root: bool,
    nested_ancestor: Option<String>,
    reparse_escape: bool,
    child_clean: bool,
    parent_clean_before: bool,
    parent_clean_after: Option<bool>,
}

impl WorktreeTopology {
    pub fn new(input: WorktreeTopologyInput) -> Result<Self, LearningError> {
        if input.head.len() != 40
            || !input.head.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !input.resolved_path.trusted_and_fresh()
            || !input.approved_root.trusted_and_fresh()
        {
            return Err(LearningError::InvalidProbe);
        }
        Ok(Self {
            worktree: bounded_reference(input.worktree)?,
            branch: bounded_reference(input.branch)?,
            head: input.head,
            resolved_path: input.resolved_path,
            approved_root: input.approved_root,
            direct_child_of_approved_root: input.direct_child_of_approved_root,
            nested_ancestor: input.nested_ancestor.map(bounded_reference).transpose()?,
            reparse_escape: input.reparse_escape,
            child_clean: input.child_clean,
            parent_clean_before: input.parent_clean_before,
            parent_clean_after: input.parent_clean_after,
        })
    }

    #[must_use]
    pub fn analyze(&self) -> TopologyDecision {
        if !self.direct_child_of_approved_root
            || self.nested_ancestor.is_some()
            || self.reparse_escape
            || !self.child_clean
            || !self.parent_clean_before
            || self.parent_clean_after == Some(false)
        {
            TopologyDecision::RepairRequired(TopologyAction::GitWorktreeMove)
        } else {
            TopologyDecision::Healthy
        }
    }

    pub fn verify_repair(
        &self,
        receipt: &WorktreeMoveReceipt,
    ) -> Result<TopologyDecision, LearningError> {
        if self.analyze() != TopologyDecision::RepairRequired(TopologyAction::GitWorktreeMove)
            || receipt.operation != TopologyAction::GitWorktreeMove
            || !receipt.source_absent
            || !receipt.target_registered
            || !receipt.branch_head_exact
            || !receipt.child_clean
            || !receipt.parent_clean_after
            || receipt.used_manual_file_move
            || receipt.used_delete
            || receipt.used_reset_or_clean
        {
            return Err(LearningError::InvalidProbe);
        }
        Ok(TopologyDecision::RepairVerified)
    }

    #[must_use]
    pub fn worktree(&self) -> &str {
        &self.worktree
    }

    #[must_use]
    pub fn branch(&self) -> &str {
        &self.branch
    }

    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }

    #[must_use]
    pub const fn resolved_path(&self) -> &EvidencePointer {
        &self.resolved_path
    }

    #[must_use]
    pub const fn approved_root(&self) -> &EvidencePointer {
        &self.approved_root
    }
}

/// Machine-verifiable outcome of the external Git-native repair executor.
/// Each flag is a separately falsifiable postcondition or prohibited action.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorktreeMoveReceipt {
    pub operation: TopologyAction,
    pub source_absent: bool,
    pub target_registered: bool,
    pub branch_head_exact: bool,
    pub child_clean: bool,
    pub parent_clean_after: bool,
    pub used_manual_file_move: bool,
    pub used_delete: bool,
    pub used_reset_or_clean: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowState {
    Unknown,
    Idle,
    Active,
    Blocked,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Window {
    id: String,
    state: WindowState,
}

impl Window {
    #[must_use]
    pub fn new(id: impl Into<String>, state: WindowState) -> Self {
        Self {
            id: id.into(),
            state,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn state(&self) -> WindowState {
        self.state
    }
}

/// GPT-5.6-only bounded work tiers. This is not model execution authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelTier {
    Luna,
    Terra,
    Sol,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateDisposition {
    Ready,
    BlockedDependency { dependency: String },
    ResourceConflict { resource: String },
    DuplicateWork { existing_task: String },
    Protected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkCandidate {
    task: String,
    disposition: CandidateDisposition,
    model: Option<ModelTier>,
    cost_units: u32,
}

impl WorkCandidate {
    pub fn ready(
        task: impl Into<String>,
        model: ModelTier,
        cost_units: u32,
    ) -> Result<Self, LearningError> {
        if cost_units == 0 {
            return Err(LearningError::CostLimitExceeded);
        }
        Ok(Self {
            task: bounded_reference(task.into())?,
            disposition: CandidateDisposition::Ready,
            model: Some(model),
            cost_units,
        })
    }

    pub fn blocked_dependency(
        task: impl Into<String>,
        dependency: impl Into<String>,
    ) -> Result<Self, LearningError> {
        Ok(Self {
            task: bounded_reference(task.into())?,
            disposition: CandidateDisposition::BlockedDependency {
                dependency: bounded_reference(dependency.into())?,
            },
            model: None,
            cost_units: 0,
        })
    }

    pub fn resource_conflict(
        task: impl Into<String>,
        resource: impl Into<String>,
    ) -> Result<Self, LearningError> {
        Ok(Self {
            task: bounded_reference(task.into())?,
            disposition: CandidateDisposition::ResourceConflict {
                resource: bounded_reference(resource.into())?,
            },
            model: None,
            cost_units: 0,
        })
    }

    pub fn duplicate(
        task: impl Into<String>,
        existing_task: impl Into<String>,
    ) -> Result<Self, LearningError> {
        Ok(Self {
            task: bounded_reference(task.into())?,
            disposition: CandidateDisposition::DuplicateWork {
                existing_task: bounded_reference(existing_task.into())?,
            },
            model: None,
            cost_units: 0,
        })
    }

    #[must_use]
    pub fn task(&self) -> &str {
        &self.task
    }

    #[must_use]
    pub const fn disposition(&self) -> &CandidateDisposition {
        &self.disposition
    }

    #[must_use]
    pub const fn model(&self) -> Option<ModelTier> {
        self.model
    }

    #[must_use]
    pub const fn cost_units(&self) -> u32 {
        self.cost_units
    }

    #[must_use]
    pub const fn dispatchable(&self) -> bool {
        matches!(self.disposition, CandidateDisposition::Ready)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Counterexample {
    UnknownWindow { worker: String },
    IdleWindow { worker: String },
    ActiveWindow { worker: String },
    EvidenceContradiction,
    MissingToolCoverage,
    MissingOrStaleEvidence,
    ReadyCandidate { task: String },
    WorktreeTopology,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Reconcile,
    Dispatch(String),
    RepairWorktreeTopology(TopologyAction),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ZeroWorkCertificate {
    policy_version: u64,
    fence: u64,
    candidates: Vec<WorkCandidate>,
    evidence: Vec<EvidencePointer>,
}

impl ZeroWorkCertificate {
    #[must_use]
    pub const fn schema(&self) -> &'static str {
        ZERO_WORK_SCHEMA
    }

    #[must_use]
    pub const fn policy_version(&self) -> u64 {
        self.policy_version
    }

    #[must_use]
    pub const fn fence(&self) -> u64 {
        self.fence
    }

    #[must_use]
    pub fn candidates(&self) -> &[WorkCandidate] {
        &self.candidates
    }

    #[must_use]
    pub fn evidence(&self) -> &[EvidencePointer] {
        &self.evidence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChallengeOutcome {
    Continue {
        counterexamples: Vec<Counterexample>,
        actions: Vec<Action>,
    },
    Certified(ZeroWorkCertificate),
}

/// Complete pre-stop inventory presented to the pure challenger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StopAudit {
    claim: StopClaim,
    windows: Vec<Window>,
    candidates: Vec<WorkCandidate>,
    evidence: Vec<EvidencePointer>,
    contradiction: bool,
    tool_gap: bool,
    topology: Option<TopologyDecision>,
}

impl StopAudit {
    #[must_use]
    pub const fn new(
        claim: StopClaim,
        windows: Vec<Window>,
        candidates: Vec<WorkCandidate>,
        evidence: Vec<EvidencePointer>,
        contradiction: bool,
        tool_gap: bool,
    ) -> Self {
        Self {
            claim,
            windows,
            candidates,
            evidence,
            contradiction,
            tool_gap,
            topology: None,
        }
    }

    #[must_use]
    pub fn with_topology(mut self, topology: TopologyDecision) -> Self {
        self.topology = Some(topology);
        self
    }

    #[must_use]
    pub const fn claim(&self) -> StopClaim {
        self.claim
    }

    #[must_use]
    pub fn challenge(
        &self,
        policy_version: u64,
        fence: u64,
        hard_cost_limit: u32,
    ) -> ChallengeOutcome {
        let mut counterexamples = Vec::new();
        for window in &self.windows {
            match window.state() {
                WindowState::Unknown => counterexamples.push(Counterexample::UnknownWindow {
                    worker: window.id().to_owned(),
                }),
                WindowState::Idle => counterexamples.push(Counterexample::IdleWindow {
                    worker: window.id().to_owned(),
                }),
                WindowState::Active => counterexamples.push(Counterexample::ActiveWindow {
                    worker: window.id().to_owned(),
                }),
                WindowState::Blocked | WindowState::Complete => {}
            }
        }
        if self.contradiction {
            counterexamples.push(Counterexample::EvidenceContradiction);
        }
        if self.tool_gap {
            counterexamples.push(Counterexample::MissingToolCoverage);
        }
        if !has_complete_fresh_stop_evidence(&self.evidence) {
            counterexamples.push(Counterexample::MissingOrStaleEvidence);
        }
        for candidate in self
            .candidates
            .iter()
            .filter(|candidate| candidate.dispatchable())
        {
            counterexamples.push(Counterexample::ReadyCandidate {
                task: candidate.task().to_owned(),
            });
        }
        if matches!(self.topology, Some(TopologyDecision::RepairRequired(_))) {
            counterexamples.push(Counterexample::WorktreeTopology);
        }
        if counterexamples.is_empty() {
            return ChallengeOutcome::Certified(ZeroWorkCertificate {
                policy_version,
                fence,
                candidates: self.candidates.clone(),
                evidence: self.evidence.clone(),
            });
        }
        let mut actions = vec![Action::Reconcile];
        if let Some(TopologyDecision::RepairRequired(action)) = self.topology {
            actions.push(Action::RepairWorktreeTopology(action));
        }
        actions.extend(
            self.candidates
                .iter()
                .filter(|candidate| {
                    candidate.dispatchable() && candidate.cost_units() <= hard_cost_limit
                })
                .map(|candidate| Action::Dispatch(candidate.task().to_owned())),
        );
        ChallengeOutcome::Continue {
            counterexamples,
            actions,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyRule {
    ChallengeBeforeStop,
    UnknownRequiresReconciliation,
    IdleRequiresBacklogAudit,
    ValidateWorktreeTopology,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedAuthority {
    Payment,
    Credentials,
    PublicNetwork,
    ForcePush,
    DefaultMerge,
    Deploy,
    Release,
    Archive,
    DisableSecurity,
    WriterLease,
}

impl ProtectedAuthority {
    pub const ALL: [Self; 10] = [
        Self::Payment,
        Self::Credentials,
        Self::PublicNetwork,
        Self::ForcePush,
        Self::DefaultMerge,
        Self::Deploy,
        Self::Release,
        Self::Archive,
        Self::DisableSecurity,
        Self::WriterLease,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProposedChange {
    Rule(PolicyRule),
    AuthorityExpansion(ProtectedAuthority),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyScope {
    PreStopChallenge,
    ContinuousSuspicion,
    WorktreeTopology,
}

/// Complete proposal metadata prevents a rule delta from losing its rationale.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyCandidateInput {
    pub version: u64,
    pub base_version: u64,
    pub fence: u64,
    pub changes: Vec<ProposedChange>,
    pub max_cost_units: u32,
    pub replay_cases: Vec<String>,
    pub reason: String,
    pub expected_impact: String,
    pub scope: PolicyScope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyCandidate {
    version: u64,
    base_version: u64,
    fence: u64,
    changes: Vec<ProposedChange>,
    max_cost_units: u32,
    replay_cases: Vec<String>,
    reason: String,
    expected_impact: String,
    scope: PolicyScope,
}

impl PolicyCandidate {
    pub fn new(input: PolicyCandidateInput) -> Result<Self, LearningError> {
        if input.version <= input.base_version || input.fence == 0 || input.changes.is_empty() {
            return Err(LearningError::StalePolicyFence);
        }
        if input.max_cost_units == 0 {
            return Err(LearningError::CostLimitExceeded);
        }
        if input
            .changes
            .iter()
            .any(|change| matches!(change, ProposedChange::AuthorityExpansion(_)))
        {
            return Err(LearningError::ProtectedAuthority);
        }
        let topology_rule = input.changes.iter().any(|change| {
            matches!(
                change,
                ProposedChange::Rule(PolicyRule::ValidateWorktreeTopology)
            )
        });
        if topology_rule && input.scope != PolicyScope::WorktreeTopology {
            return Err(LearningError::InvalidPolicyCandidate);
        }
        validate_references(&input.replay_cases)?;
        Ok(Self {
            version: input.version,
            base_version: input.base_version,
            fence: input.fence,
            changes: input.changes,
            max_cost_units: input.max_cost_units,
            replay_cases: input.replay_cases,
            reason: bounded_reference(input.reason)?,
            expected_impact: bounded_reference(input.expected_impact)?,
            scope: input.scope,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u64 {
        self.version
    }

    #[must_use]
    pub const fn base_version(&self) -> u64 {
        self.base_version
    }

    #[must_use]
    pub const fn fence(&self) -> u64 {
        self.fence
    }

    #[must_use]
    pub fn changes(&self) -> &[ProposedChange] {
        &self.changes
    }

    #[must_use]
    pub const fn max_cost_units(&self) -> u32 {
        self.max_cost_units
    }

    #[must_use]
    pub fn replay_cases(&self) -> &[String] {
        &self.replay_cases
    }

    #[must_use]
    pub fn reason(&self) -> &str {
        &self.reason
    }

    #[must_use]
    pub fn expected_impact(&self) -> &str {
        &self.expected_impact
    }

    #[must_use]
    pub const fn scope(&self) -> PolicyScope {
        self.scope
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Gate {
    MultiCaseReplay,
    CounterexampleReplay,
    SafetyAndDependencyRegression,
    CostRegression,
    Rollback,
    Privacy,
}

impl Gate {
    pub const ALL: [Self; 6] = [
        Self::MultiCaseReplay,
        Self::CounterexampleReplay,
        Self::SafetyAndDependencyRegression,
        Self::CostRegression,
        Self::Rollback,
        Self::Privacy,
    ];
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateReceipt {
    gate: Gate,
    evidence: EvidencePointer,
    passed: bool,
}

impl GateReceipt {
    pub fn pass(gate: Gate, evidence: EvidencePointer) -> Result<Self, LearningError> {
        if !evidence.trusted_and_fresh() {
            return Err(LearningError::MissingEvidence);
        }
        Ok(Self {
            gate,
            evidence,
            passed: true,
        })
    }

    #[must_use]
    pub const fn gate(&self) -> Gate {
        self.gate
    }

    #[must_use]
    pub const fn evidence(&self) -> &EvidencePointer {
        &self.evidence
    }

    #[must_use]
    pub const fn passed(&self) -> bool {
        self.passed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyCheckpoint {
    schema: &'static str,
    active_version: u64,
    previous_version: Option<u64>,
    fence: u64,
    hard_cost_limit: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyController {
    active_version: u64,
    previous_version: Option<u64>,
    fence: u64,
    hard_cost_limit: u32,
}

impl PolicyController {
    #[must_use]
    pub const fn new(active_version: u64, fence: u64, hard_cost_limit: u32) -> Self {
        Self {
            active_version,
            previous_version: None,
            fence,
            hard_cost_limit,
        }
    }

    pub fn promote(
        &mut self,
        candidate: &PolicyCandidate,
        receipts: &[GateReceipt],
    ) -> Result<(), LearningError> {
        if candidate.base_version() != self.active_version
            || candidate.version() != self.active_version.saturating_add(1)
            || candidate.fence() != self.fence
        {
            return Err(LearningError::StalePolicyFence);
        }
        if candidate.max_cost_units() > self.hard_cost_limit {
            return Err(LearningError::CostLimitExceeded);
        }
        if candidate
            .replay_cases()
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            < 2
        {
            return Err(LearningError::InsufficientReplayCases);
        }
        if candidate.changes().iter().any(|change| {
            matches!(
                change,
                ProposedChange::Rule(PolicyRule::ValidateWorktreeTopology)
            )
        }) && (!candidate
            .replay_cases()
            .iter()
            .any(|case| case == "TASK-083")
            || !candidate
                .replay_cases()
                .iter()
                .any(|case| case != "TASK-083"))
        {
            return Err(LearningError::InsufficientReplayCases);
        }
        let passed = receipts
            .iter()
            .filter(|receipt| receipt.passed() && receipt.evidence().trusted_and_fresh())
            .map(GateReceipt::gate)
            .collect::<BTreeSet<_>>();
        if !Gate::ALL.iter().all(|gate| passed.contains(gate)) {
            return Err(LearningError::MissingMachineGate);
        }
        self.previous_version = Some(self.active_version);
        self.active_version = candidate.version();
        self.fence = self
            .fence
            .checked_add(1)
            .ok_or(LearningError::StalePolicyFence)?;
        Ok(())
    }

    #[must_use]
    pub const fn active_version(&self) -> u64 {
        self.active_version
    }

    #[must_use]
    pub const fn previous_version(&self) -> Option<u64> {
        self.previous_version
    }

    #[must_use]
    pub const fn checkpoint(&self) -> PolicyCheckpoint {
        PolicyCheckpoint {
            schema: "lattice.workflow-policy-checkpoint/1.0",
            active_version: self.active_version,
            previous_version: self.previous_version,
            fence: self.fence,
            hard_cost_limit: self.hard_cost_limit,
        }
    }

    pub fn recover(checkpoint: PolicyCheckpoint) -> Result<Self, LearningError> {
        if checkpoint.schema != "lattice.workflow-policy-checkpoint/1.0"
            || checkpoint.active_version == 0
            || checkpoint.fence == 0
            || checkpoint.hard_cost_limit == 0
            || checkpoint.previous_version == Some(checkpoint.active_version)
        {
            return Err(LearningError::InvalidCheckpoint);
        }
        Ok(Self {
            active_version: checkpoint.active_version,
            previous_version: checkpoint.previous_version,
            fence: checkpoint.fence,
            hard_cost_limit: checkpoint.hard_cost_limit,
        })
    }

    pub fn monitor(&mut self, metrics: Metrics) -> MonitorOutcome {
        let degraded = metrics.misdispatches > 0
            || metrics.missed_dispatches > 1
            || metrics.duplicate_work > 0
            || metrics.false_positives > 0
            || metrics.cost_units > self.hard_cost_limit;
        if degraded && let Some(previous) = self.previous_version.take() {
            let from = self.active_version;
            self.active_version = previous;
            self.fence = self.fence.saturating_add(1);
            return MonitorOutcome::RolledBack { from, to: previous };
        }
        MonitorOutcome::Stable
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Metrics {
    pub misdispatches: u32,
    pub missed_dispatches: u32,
    pub duplicate_work: u32,
    pub false_positives: u32,
    pub cost_units: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MonitorOutcome {
    Stable,
    RolledBack { from: u64, to: u64 },
}

fn has_complete_fresh_stop_evidence(evidence: &[EvidencePointer]) -> bool {
    let fresh_sources = evidence
        .iter()
        .filter(|pointer| pointer.trusted_and_fresh())
        .map(EvidencePointer::source)
        .collect::<BTreeSet<_>>();
    EvidenceSource::STOP_CERTIFICATE_REQUIRED
        .iter()
        .all(|source| fresh_sources.contains(source))
}

fn validate_references(values: &[String]) -> Result<(), LearningError> {
    for value in values {
        bounded_reference(value.clone())?;
    }
    Ok(())
}

fn bounded_reference(value: String) -> Result<String, LearningError> {
    let bytes = value.as_bytes();
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains("../")
        || (bytes.len() > 2 && bytes[1] == b':' && bytes[2] == b'/')
    {
        return Err(LearningError::ForbiddenContent);
    }
    if value.is_empty()
        || value.len() > MAX_REFERENCE_BYTES
        || !value.is_ascii()
        || value.contains(char::is_whitespace)
    {
        return Err(LearningError::MalformedReference);
    }
    let lowercase = value.to_ascii_lowercase();
    if lowercase.starts_with("sk-live-")
        || lowercase.starts_with("sk-proj-")
        || lowercase.starts_with("bearer:")
        || lowercase.starts_with("password=")
        || lowercase.contains("full-chat")
    {
        return Err(LearningError::ForbiddenContent);
    }
    Ok(value)
}
