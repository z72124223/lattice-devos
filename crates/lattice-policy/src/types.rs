pub use lattice_contracts::{
    ApprovalAuthority, ApprovalAuthorityHead, ApprovalAuthorityReceipt, ApprovalIdentity,
    ApprovalKind, ApprovalLane, ApprovalOrigin, ApprovalRevision, ApprovalStatus, ApprovalSubject,
    AttemptId, DaemonEpoch, ExternalCostSubject, FencingToken, GitRefIdentity,
    GuardianRuntimeSubject, HolderProcessId, MemoryCandidateSubject, MemoryKind, MergeSubject,
    MergeTarget, ProjectClass, ProjectLifecycle, ProtectedChangeClass, ProtectedChangeSubject,
    ProtectedReleaseSubject, ReleaseSubject, ResourceCounters, ResourceRequest,
    RuntimeAdmissionMode, SubjectBinding, TaskLedgerResourceHead, TaskLedgerResourceReceipt,
    UpgradeDelta, WriterLeaseAuthorityHead, WriterLeaseAuthorityReceipt, WriterLeaseStatus,
};
use lattice_contracts::{
    ContentDigest, ProjectAuthorityHead, ProjectAuthorityReceipt, RuntimeKind,
};
use lattice_task_domain::{Capability, TaskSpec, TaskState};

/// A closed boundary value whose unknown representation cannot be mistaken for
/// a known enum variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Boundary<T> {
    /// A parsed known value.
    Known(T),
    /// An unknown or unsupported value.
    Unknown,
}

impl<T> From<T> for Boundary<T> {
    fn from(value: T) -> Self {
        Self::Known(value)
    }
}

/// A boundary failure represented as data so evaluation cannot be skipped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyInputFailure {
    UnknownRole,
    UnknownAction,
    UnknownState,
    UnknownRuntimeAdmission,
    UnknownCapability,
    UnknownAuthority,
    MalformedSubject,
}

macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident => $wire:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Parses a stable wire value without inventing a fallback.
            #[must_use]
            pub fn parse(value: &str) -> Boundary<Self> {
                match value {
                    $($wire => Boundary::Known(Self::$variant),)+
                    _ => Boundary::Unknown,
                }
            }

            /// Returns the stable wire value.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }
        }
    };
}

wire_enum! {
    /// Closed V2 authority roles; providers are not roles.
    pub enum AgentRole {
        LatticePm => "LATTICE_PM",
        Planner => "PLANNER",
        CodeMapper => "CODE_MAPPER",
        Researcher => "RESEARCHER",
        Implementer => "IMPLEMENTER",
        CorrectnessReviewer => "CORRECTNESS_REVIEWER",
        SecurityReviewer => "SECURITY_REVIEWER",
        ArchitectureReviewer => "ARCHITECTURE_REVIEWER",
        MemoryReviewer => "MEMORY_REVIEWER",
        Integrator => "INTEGRATOR",
        UpgradeGuardian => "UPGRADE_GUARDIAN",
    }
}

impl AgentRole {
    /// Complete V2 role set.
    pub const ALL: [Self; 11] = [
        Self::LatticePm,
        Self::Planner,
        Self::CodeMapper,
        Self::Researcher,
        Self::Implementer,
        Self::CorrectnessReviewer,
        Self::SecurityReviewer,
        Self::ArchitectureReviewer,
        Self::MemoryReviewer,
        Self::Integrator,
        Self::UpgradeGuardian,
    ];
}

wire_enum! {
    /// Closed V2 policy actions.
    pub enum PolicyAction {
        SubmitPlan => "SUBMIT_PLAN",
        ReadRepository => "READ_REPOSITORY",
        PlanTask => "PLAN_TASK",
        MapCode => "MAP_CODE",
        Research => "RESEARCH",
        PrepareWorktree => "PREPARE_WORKTREE",
        WriteProductCode => "WRITE_PRODUCT_CODE",
        RunTests => "RUN_TESTS",
        RunCodex => "RUN_CODEX",
        RunGraphify => "RUN_GRAPHIFY",
        RunHermes => "RUN_HERMES",
        ReviewCorrectness => "REVIEW_CORRECTNESS",
        ReviewSecurity => "REVIEW_SECURITY",
        ReviewArchitecture => "REVIEW_ARCHITECTURE",
        ReadMemory => "READ_MEMORY",
        ProposeMemory => "PROPOSE_MEMORY",
        PromoteMemory => "PROMOTE_MEMORY",
        ProposeUpgrade => "PROPOSE_UPGRADE",
        IntegrateGit => "INTEGRATE_GIT",
        StopRuntime => "STOP_RUNTIME",
        ReconcileRuntime => "RECONCILE_RUNTIME",
        ReleaseWriter => "RELEASE_WRITER",
        GuardianShadow => "GUARDIAN_SHADOW",
        GuardianHealth => "GUARDIAN_HEALTH",
        ActivateUpgrade => "ACTIVATE_UPGRADE",
        RollbackUpgrade => "ROLLBACK_UPGRADE",
        RequestProtectedChange => "REQUEST_PROTECTED_CHANGE",
    }
}

impl PolicyAction {
    /// Complete V2 action set.
    pub const ALL: [Self; 27] = [
        Self::SubmitPlan,
        Self::ReadRepository,
        Self::PlanTask,
        Self::MapCode,
        Self::Research,
        Self::PrepareWorktree,
        Self::WriteProductCode,
        Self::RunTests,
        Self::RunCodex,
        Self::RunGraphify,
        Self::RunHermes,
        Self::ReviewCorrectness,
        Self::ReviewSecurity,
        Self::ReviewArchitecture,
        Self::ReadMemory,
        Self::ProposeMemory,
        Self::PromoteMemory,
        Self::ProposeUpgrade,
        Self::IntegrateGit,
        Self::StopRuntime,
        Self::ReconcileRuntime,
        Self::ReleaseWriter,
        Self::GuardianShadow,
        Self::GuardianHealth,
        Self::ActivateUpgrade,
        Self::RollbackUpgrade,
        Self::RequestProtectedChange,
    ];
}

wire_enum! {
    /// Durable runtime-admission modes evaluated by Policy.
    pub enum RuntimeAdmission {
        Active => "ACTIVE",
        Draining => "DRAINING",
        Canary => "CANARY",
        Stopped => "STOPPED",
        ReconciliationRequired => "RECONCILIATION_REQUIRED",
    }
}

impl From<RuntimeAdmission> for RuntimeAdmissionMode {
    fn from(value: RuntimeAdmission) -> Self {
        match value {
            RuntimeAdmission::Active => Self::Active,
            RuntimeAdmission::Draining => Self::Draining,
            RuntimeAdmission::Canary => Self::Canary,
            RuntimeAdmission::Stopped => Self::Stopped,
            RuntimeAdmission::ReconciliationRequired => Self::ReconciliationRequired,
        }
    }
}

impl RuntimeAdmission {
    /// Complete admission set.
    pub const ALL: [Self; 5] = [
        Self::Active,
        Self::Draining,
        Self::Canary,
        Self::Stopped,
        Self::ReconciliationRequired,
    ];
}

/// External provider identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderKind {
    Codex,
    Graphify,
    Hermes,
}

wire_enum! {
    /// Producer of exact merge-readiness evidence.
    pub enum MergeAnalysisProducer {
        WorkspaceGit => "WORKSPACE_GIT",
    }
}

wire_enum! {
    /// Semantic owner of a recovery-authority fact.
    pub enum RecoveryOwner {
        RuntimeSupervisor => "RUNTIME_SUPERVISOR",
        UpgradeGuardian => "UPGRADE_GUARDIAN",
    }
}

/// Requested network effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkIntent {
    None,
    Loopback,
    External {
        target_digest: ContentDigest,
        allowlist_digest: Option<ContentDigest>,
    },
}

/// Requested deployment effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeploymentIntent {
    None,
    PrepareArtifact,
    Deploy,
}

/// Workspace-Git-owned analysis bound to the exact reviewed merge subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergeReadinessFact {
    pub binding: SubjectBinding,
    pub subject: MergeSubject,
    pub producer: Boundary<MergeAnalysisProducer>,
    pub producer_id: String,
    pub producer_version: String,
    pub target_ref_identity: GitRefIdentity,
    pub analysis_digest: ContentDigest,
    pub scope_evidence_digest: ContentDigest,
    pub scope_verified: bool,
    pub conflict_free: bool,
    pub fresh: bool,
}

/// Guarded self-upgrade stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpgradeStage {
    Propose,
    Test,
    Shadow,
    Activate,
    HealthCanary,
    Rollback,
}

/// Quote-owner output supplied to pure Policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalCostFact {
    pub binding: SubjectBinding,
    pub subject: ExternalCostSubject,
    pub quote_verified: bool,
    pub fresh: bool,
}

/// Exact writer lease target requested by a product-code or recovery action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseSubject {
    pub lease_holder_id: String,
    pub lease_id: String,
    pub attempt_id: AttemptId,
    pub worktree_id: String,
    pub holder_process_id: HolderProcessId,
    pub holder_process_start_identity: ContentDigest,
    pub daemon_instance_id: String,
    pub daemon_epoch: DaemonEpoch,
    pub fencing_token: FencingToken,
    pub runtime: RuntimeKind,
}

/// Codebase Memory owner/reviewer output supplied to pure Policy.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryReviewFact {
    pub subject: MemoryCandidateSubject,
    pub provenance_digest: ContentDigest,
    pub schema_digest: ContentDigest,
    pub reviewer_id: String,
    pub immutable_provenance: bool,
    pub schema_valid: bool,
    pub review_accepted: bool,
    pub fresh: bool,
}

/// Owner-produced receipt for the exact approved and claimed activation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedActivationReceipt {
    pub subject: ProtectedReleaseSubject,
    pub approval_id: String,
    pub activation_claim_id: String,
}

/// Stage-specific rollback target for one failed activation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackSubject {
    pub rollback_id: String,
    pub failed_activation_id: String,
    pub saga_id: String,
    pub failed_activation: Box<ProtectedActivationReceipt>,
    pub current_release_id: String,
    pub current_manifest_digest: ContentDigest,
    pub current_slot_id: String,
    pub current_epoch: u64,
    pub target_release_id: String,
    pub target_manifest_digest: ContentDigest,
    pub target_slot_id: String,
    pub requested_epoch: u64,
    pub compatibility_evidence_digest: ContentDigest,
    pub failure_evidence_digest: ContentDigest,
    pub schema_compatible: bool,
    pub migration_digests: Vec<ContentDigest>,
}

/// Guardian-owned immutable candidate/saga evidence.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeEvidenceFact {
    pub binding: SubjectBinding,
    pub subject: ReleaseSubject,
    pub rollback: Option<RollbackSubject>,
    pub candidate_immutable: bool,
    pub inactive_slot_verified: bool,
    pub prior_slot_verified: bool,
    pub saga_bound: bool,
    pub epoch_bound: bool,
    pub fresh: bool,
}

/// Independent guardian runtime identity for shadow/activation/recovery.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianAuthorityFact {
    pub binding: SubjectBinding,
    pub subject: ReleaseSubject,
    pub rollback: Option<RollbackSubject>,
    pub origin: Boundary<ApprovalOrigin>,
    pub runtime: GuardianRuntimeSubject,
    pub identity_verified: bool,
    pub fresh: bool,
    pub reserved_system_stream: bool,
    pub user_project_access: bool,
}

/// Project Registry fact. Policy verifies sufficiency but does not produce it.
///
/// `current_head` must come from an independent current Registry-owner lookup.
/// Re-projecting `receipt.head()` does not establish currentness after a later
/// Registry transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectAuthorityFact {
    pub binding: SubjectBinding,
    pub receipt: ProjectAuthorityReceipt,
    pub current_head: ProjectAuthorityHead,
}

/// Common validated-subject context.
#[derive(Clone, Debug)]
pub struct TaskContext<'a> {
    pub task_spec: Option<&'a TaskSpec>,
    pub project: Option<ProjectAuthorityFact>,
    pub state: Boundary<TaskState>,
    pub runtime_admission: Boundary<RuntimeAdmission>,
}

/// Approval-Verifier-owned authority plus an independently obtained current
/// owner head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalFact {
    pub receipt: ApprovalAuthorityReceipt,
    pub current_head: Option<ApprovalAuthorityHead>,
}

/// Current exact provider-capability observation.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderCapabilityFact {
    pub binding: SubjectBinding,
    pub provider: ProviderKind,
    pub capability: Capability,
    pub contract_version: u16,
    pub runtime: RuntimeKind,
    pub provider_id: String,
    pub provider_version: String,
    pub expected_executable_digest: ContentDigest,
    pub observed_executable_digest: ContentDigest,
    pub expected_schema_digest: ContentDigest,
    pub observed_schema_digest: ContentDigest,
    pub available: bool,
    pub identity_verified: bool,
    pub boundary_verified: bool,
    pub fresh: bool,
}

/// Writer-Lease-owned authority receipt plus an independently obtained current
/// owner head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseFact {
    pub receipt: WriterLeaseAuthorityReceipt,
    pub current_head: Option<WriterLeaseAuthorityHead>,
}

/// Task-Ledger-owned resource observation plus independently obtained current
/// owner head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceUsageFact {
    pub binding: SubjectBinding,
    pub receipt: TaskLedgerResourceReceipt,
    pub current_head: Option<TaskLedgerResourceHead>,
}

/// Exact Task-Ledger observation and effect claim requested by one decision.
///
/// The gate owns this expected subject independently of the supplied fact so a
/// valid low-usage observation cannot be replayed for another same-task effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceObservationSubject {
    pub stream_id: ContentDigest,
    pub stream_head_digest: ContentDigest,
    pub observation_revision: u64,
    pub effect_claim_id: String,
    pub effect_subject_digest: ContentDigest,
    pub request: ResourceRequest,
}

/// Generic role/action decision.
#[derive(Clone, Debug)]
pub struct AgentActionGate<'a> {
    pub context: TaskContext<'a>,
    pub role: Boundary<AgentRole>,
    pub action: Boundary<PolicyAction>,
    pub actor_id: String,
    pub approval: Option<ApprovalFact>,
    pub provider_capability: Option<ProviderCapabilityFact>,
    pub external_cost: Option<ExternalCostFact>,
    pub writer_subject: Option<WriterLeaseSubject>,
    pub writer: Option<WriterLeaseFact>,
    pub resource_subject: Option<ResourceObservationSubject>,
    pub resources: Option<ResourceUsageFact>,
    pub network: NetworkIntent,
    pub deployment: DeploymentIntent,
}

/// Execution-transition approval decision.
#[derive(Clone, Debug)]
pub struct ExecutionGate<'a> {
    pub context: TaskContext<'a>,
    pub approval: Option<ApprovalFact>,
}

/// Exact immutable Task-Ledger execution binding supplied to the managed
/// execution Policy lane. Policy captures these values in opaque evidence so
/// downstream code cannot attach an allowed decision to another task,
/// successor stream, approval subject, or budget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedExecutionBindingFact {
    pub task_ref: ContentDigest,
    pub successor_stream_id: ContentDigest,
    pub task_spec_digest: ContentDigest,
    pub approval_subject_digest: ContentDigest,
    pub budget_digest: ContentDigest,
}

/// Worker-set admission decision.
#[derive(Clone, Debug)]
pub struct WorkerAdmissionGate<'a> {
    pub context: TaskContext<'a>,
    pub workers: Vec<AgentRole>,
    pub resource_subject: ResourceObservationSubject,
    pub resources: ResourceUsageFact,
}

/// Git integration approval decision.
#[derive(Clone, Debug)]
pub struct MergeGate<'a> {
    pub context: TaskContext<'a>,
    pub role: Boundary<AgentRole>,
    pub subject: MergeSubject,
    pub readiness: Option<MergeReadinessFact>,
    pub approval: Option<ApprovalFact>,
    pub resource_subject: ResourceObservationSubject,
    pub resources: ResourceUsageFact,
}

/// Memory candidate promotion decision.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct MemoryPromotionGate<'a> {
    pub context: TaskContext<'a>,
    pub role: Boundary<AgentRole>,
    pub subject: MemoryCandidateSubject,
    pub review: Option<MemoryReviewFact>,
    pub claims_authority: bool,
    pub preference_user_approval: Option<ApprovalFact>,
}

/// Guarded self-upgrade stage decision.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
pub struct UpgradeGate<'a> {
    pub context: TaskContext<'a>,
    pub role: Boundary<AgentRole>,
    pub stage: UpgradeStage,
    pub subject: ReleaseSubject,
    pub rollback: Option<RollbackSubject>,
    pub evidence: Option<UpgradeEvidenceFact>,
    pub guardian: Option<GuardianAuthorityFact>,
    pub approval: Option<ApprovalFact>,
}

wire_enum! {
    /// Durable provider outcome established before an effect leaves unknown state.
    pub enum ResolvedEffectOutcome {
        Succeeded => "SUCCEEDED",
        Failed => "FAILED",
        Cancelled => "CANCELLED",
        NotApplied => "NOT_APPLIED",
    }
}

/// Typed normal-runtime resolution established by its future owner module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NormalRecoveryResolution {
    /// A provider status/query contract resolved one exact effect claim.
    EffectOutcome {
        outcome: Boundary<ResolvedEffectOutcome>,
        provider_status_digest: ContentDigest,
    },
    /// The writer/effect holder process identity was proven dead.
    HolderDeath {
        holder_daemon_instance_id: String,
        holder_process_id: u32,
        holder_process_start_identity: String,
    },
    /// A strictly newer daemon leadership epoch replaced the old holder.
    ReplacedLeadership {
        replaced_daemon_instance_id: String,
        replaced_epoch: u64,
        active_daemon_instance_id: String,
        active_epoch: u64,
    },
}

wire_enum! {
    /// Durable guardian saga outcome established before runtime activation.
    pub enum GuardianSagaOutcome {
        ActivationFinalized => "ACTIVATION_FINALIZED",
        RollbackFinalized => "ROLLBACK_FINALIZED",
    }
}

/// Exact reconciled durable guardian saga/database/boot state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianSagaResolution {
    pub outcome: Boundary<GuardianSagaOutcome>,
    pub activation: Box<ProtectedActivationReceipt>,
    pub durable_saga_state_digest: ContentDigest,
    pub database_state_digest: ContentDigest,
    pub boot_state_digest: ContentDigest,
    pub active_release_id: String,
    pub active_manifest_digest: ContentDigest,
    pub active_slot_id: String,
    pub active_epoch: u64,
}

/// Exact normal-runtime recovery target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalRecoverySubject {
    pub runtime_supervisor_id: String,
    pub daemon_instance_id: String,
    pub effect_claim_id: String,
    pub worktree_id: Option<String>,
    pub expected_daemon_epoch: u64,
    pub resolution: NormalRecoveryResolution,
    pub resolution_evidence_digest: ContentDigest,
    pub observed_admission: RuntimeAdmission,
    pub target_admission: RuntimeAdmission,
}

/// Exact guardian release-saga reconciliation target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardianRecoverySubject {
    pub release: ReleaseSubject,
    pub guardian: GuardianRuntimeSubject,
    pub effect_claim_id: String,
    pub resolution: GuardianSagaResolution,
    pub resolution_evidence_digest: ContentDigest,
    pub observed_admission: RuntimeAdmission,
    pub target_admission: RuntimeAdmission,
}

/// Closed recovery lanes; normal and guardian authority cannot cross-label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoverySubject {
    Normal(NormalRecoverySubject),
    GuardianRelease(Box<GuardianRecoverySubject>),
}

/// Runtime-supervisor or guardian output supplied to pure Policy.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryAuthorityFact {
    pub binding: SubjectBinding,
    pub subject: RecoverySubject,
    pub owner: Boundary<RecoveryOwner>,
    pub producer_id: String,
    pub identity_verified: bool,
    pub fresh: bool,
    pub reserved_system_stream: bool,
    pub user_project_access: bool,
}

/// Dedicated exact runtime-reconciliation decision.
#[derive(Clone, Debug)]
pub struct RecoveryGate<'a> {
    pub context: TaskContext<'a>,
    pub role: Boundary<AgentRole>,
    pub subject: RecoverySubject,
    pub authority: Option<RecoveryAuthorityFact>,
}

/// Protected system change decision.
#[derive(Clone, Debug)]
pub struct ProtectedChangeGate<'a> {
    pub context: TaskContext<'a>,
    pub role: Boundary<AgentRole>,
    pub subject: ProtectedChangeSubject,
    pub approval: Option<ApprovalFact>,
}

/// Closed set of policy decision subjects.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum DecisionSubject<'a> {
    Invalid(PolicyInputFailure),
    ExecutionGate(ExecutionGate<'a>),
    AgentAction(AgentActionGate<'a>),
    WorkerAdmission(WorkerAdmissionGate<'a>),
    MergeGate(MergeGate<'a>),
    MemoryPromotion(MemoryPromotionGate<'a>),
    UpgradeStage(UpgradeGate<'a>),
    Recovery(RecoveryGate<'a>),
    ProtectedChange(ProtectedChangeGate<'a>),
}
