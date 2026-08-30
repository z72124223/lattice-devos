use lattice_contracts::{
    ContentDigest, ProjectAuthorityHead, ProjectAuthorityReceipt, SubjectBinding,
};
use lattice_task_domain::TaskState;

use crate::{Boundary, ManagedExecutionBindingFact, POLICY_CONTRACT_VERSION, RuntimeAdmission};

/// Closed policy subject classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionKind {
    /// A boundary parser or caller reported an invalid subject.
    Invalid,
    /// Task execution approval.
    ExecutionGate,
    /// One agent action.
    AgentAction,
    /// A worker-set admission.
    WorkerAdmission,
    /// Git integration approval.
    MergeGate,
    /// Codebase Memory candidate promotion.
    MemoryPromotion,
    /// One guarded upgrade stage.
    UpgradeStage,
    /// One exact runtime reconciliation.
    RecoveryGate,
    /// One protected system change.
    ProtectedChange,
}

/// Fixed evaluation stages used as deterministic evidence.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DecisionStage {
    /// Boundary value validity.
    Input,
    /// Project/snapshot/task/spec binding.
    Project,
    /// Runtime admission.
    Runtime,
    /// Role and action compatibility.
    RoleAction,
    /// Task state.
    State,
    /// Protected-surface routing.
    Protected,
    /// Task-requested capability.
    RequestedCapability,
    /// Current provider capability.
    ProviderCapability,
    /// Network, deployment, and cost.
    EffectEnvelope,
    /// Risk and approval.
    Approval,
    /// Writer lease, epoch, and fencing.
    Writer,
    /// Checked resource budget.
    Resources,
    /// All applicable checks passed.
    Complete,
}

/// Stable allow/deny reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyReason {
    UnknownRole,
    UnknownAction,
    UnknownState,
    UnknownRuntimeAdmission,
    UnknownCapability,
    UnknownAuthority,
    InvalidDecisionSubject,
    ProjectNotRegistered,
    ProjectInactive,
    ProjectDrifted,
    ProjectAuthorityStale,
    ProjectIdMismatch,
    ProjectSnapshotMismatch,
    TaskIdMismatch,
    TaskRevisionMismatch,
    TaskSpecHashMismatch,
    RuntimeAdmissionDenied,
    RoleActionDenied,
    ActionStateDenied,
    ProtectedSurfaceRequired,
    CapabilityNotRequested,
    CapabilityEvidenceMissing,
    CapabilityContractMismatch,
    CapabilityEvidenceStale,
    CapabilityIdentityMismatch,
    ProviderBoundaryDenied,
    RuntimeKindMismatch,
    NetworkDenied,
    NetworkTargetMismatch,
    NetworkAllowlistUnbound,
    DeploymentDenied,
    DeploymentPrepareOnly,
    ExternalCostUnknown,
    ExternalCostProtected,
    RiskChecksInsufficient,
    ApprovalMissing,
    ApprovalKindMismatch,
    ApprovalAuthorityDenied,
    ApprovalSubjectMismatch,
    ApprovalIdentityUnverified,
    ApprovalStale,
    ApprovalReplayed,
    SelfApprovalDenied,
    ReviewAuthorityUnavailable,
    WriterLeaseRequired,
    WriterLeaseNotCurrent,
    WriterLeaseSubjectMismatch,
    FencingTokenMismatch,
    MultipleImplementers,
    ResourceEvidenceInvalid,
    AgentLimitExceeded,
    DurationBudgetExceeded,
    AttemptBudgetExceeded,
    ModelCallBudgetExceeded,
    ExternalCostBudgetExceeded,
    MergeReadinessRequired,
    MergeReadinessMismatch,
    MergeReadinessStale,
    MergeConflictRequiresImplementer,
    PrimaryBranchApprovalRequired,
    MemoryCannotAuthorize,
    MemoryCrossProject,
    MemoryProvenanceRequired,
    MemoryReviewRequired,
    PreferenceUserEvidenceRequired,
    UpgradeStageDenied,
    UpgradeDeltaProtected,
    UpgradeSchemaMigrationDenied,
    GuardianRequired,
    GuardianApprovalRequired,
    ResourceEvidenceStale,
    ResourceCurrencyMismatch,
    RecoveryAuthorityRequired,
    RecoveryAuthorityMismatch,
    InternalPolicyError,
    AgentActionAllowed,
    ExecutionGateAllowed,
    WorkerAdmissionAllowed,
    MergeGateAllowed,
    MemoryPromotionAllowed,
    UpgradeStageAllowed,
    RecoveryGateAllowed,
    ProtectedChangeAllowed,
}

impl PolicyReason {
    /// Returns the stable wire-facing reason code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnknownRole => "UNKNOWN_ROLE",
            Self::UnknownAction => "UNKNOWN_ACTION",
            Self::UnknownState => "UNKNOWN_STATE",
            Self::UnknownRuntimeAdmission => "UNKNOWN_RUNTIME_ADMISSION",
            Self::UnknownCapability => "UNKNOWN_CAPABILITY",
            Self::UnknownAuthority => "UNKNOWN_AUTHORITY",
            Self::InvalidDecisionSubject => "INVALID_DECISION_SUBJECT",
            Self::ProjectNotRegistered => "PROJECT_NOT_REGISTERED",
            Self::ProjectInactive => "PROJECT_INACTIVE",
            Self::ProjectDrifted => "PROJECT_DRIFTED",
            Self::ProjectAuthorityStale => "PROJECT_AUTHORITY_STALE",
            Self::ProjectIdMismatch => "PROJECT_ID_MISMATCH",
            Self::ProjectSnapshotMismatch => "PROJECT_SNAPSHOT_MISMATCH",
            Self::TaskIdMismatch => "TASK_ID_MISMATCH",
            Self::TaskRevisionMismatch => "TASK_REVISION_MISMATCH",
            Self::TaskSpecHashMismatch => "TASK_SPEC_HASH_MISMATCH",
            Self::RuntimeAdmissionDenied => "RUNTIME_ADMISSION_DENIED",
            Self::RoleActionDenied => "ROLE_ACTION_DENIED",
            Self::ActionStateDenied => "ACTION_STATE_DENIED",
            Self::ProtectedSurfaceRequired => "PROTECTED_SURFACE_REQUIRED",
            Self::CapabilityNotRequested => "CAPABILITY_NOT_REQUESTED",
            Self::CapabilityEvidenceMissing => "CAPABILITY_EVIDENCE_MISSING",
            Self::CapabilityContractMismatch => "CAPABILITY_CONTRACT_MISMATCH",
            Self::CapabilityEvidenceStale => "CAPABILITY_EVIDENCE_STALE",
            Self::CapabilityIdentityMismatch => "CAPABILITY_IDENTITY_MISMATCH",
            Self::ProviderBoundaryDenied => "PROVIDER_BOUNDARY_DENIED",
            Self::RuntimeKindMismatch => "RUNTIME_KIND_MISMATCH",
            Self::NetworkDenied => "NETWORK_DENIED",
            Self::NetworkTargetMismatch => "NETWORK_TARGET_MISMATCH",
            Self::NetworkAllowlistUnbound => "NETWORK_ALLOWLIST_UNBOUND",
            Self::DeploymentDenied => "DEPLOYMENT_DENIED",
            Self::DeploymentPrepareOnly => "DEPLOYMENT_PREPARE_ONLY",
            Self::ExternalCostUnknown => "EXTERNAL_COST_UNKNOWN",
            Self::ExternalCostProtected => "EXTERNAL_COST_PROTECTED",
            Self::RiskChecksInsufficient => "RISK_CHECKS_INSUFFICIENT",
            Self::ApprovalMissing => "APPROVAL_MISSING",
            Self::ApprovalKindMismatch => "APPROVAL_KIND_MISMATCH",
            Self::ApprovalAuthorityDenied => "APPROVAL_AUTHORITY_DENIED",
            Self::ApprovalSubjectMismatch => "APPROVAL_SUBJECT_MISMATCH",
            Self::ApprovalIdentityUnverified => "APPROVAL_IDENTITY_UNVERIFIED",
            Self::ApprovalStale => "APPROVAL_STALE",
            Self::ApprovalReplayed => "APPROVAL_REPLAYED",
            Self::SelfApprovalDenied => "SELF_APPROVAL_DENIED",
            Self::ReviewAuthorityUnavailable => "REVIEW_AUTHORITY_UNAVAILABLE",
            Self::WriterLeaseRequired => "WRITER_LEASE_REQUIRED",
            Self::WriterLeaseNotCurrent => "WRITER_LEASE_NOT_CURRENT",
            Self::WriterLeaseSubjectMismatch => "WRITER_LEASE_SUBJECT_MISMATCH",
            Self::FencingTokenMismatch => "FENCING_TOKEN_MISMATCH",
            Self::MultipleImplementers => "MULTIPLE_IMPLEMENTERS",
            Self::ResourceEvidenceInvalid => "RESOURCE_EVIDENCE_INVALID",
            Self::AgentLimitExceeded => "AGENT_LIMIT_EXCEEDED",
            Self::DurationBudgetExceeded => "DURATION_BUDGET_EXCEEDED",
            Self::AttemptBudgetExceeded => "ATTEMPT_BUDGET_EXCEEDED",
            Self::ModelCallBudgetExceeded => "MODEL_CALL_BUDGET_EXCEEDED",
            Self::ExternalCostBudgetExceeded => "EXTERNAL_COST_BUDGET_EXCEEDED",
            Self::MergeReadinessRequired => "MERGE_READINESS_REQUIRED",
            Self::MergeReadinessMismatch => "MERGE_READINESS_MISMATCH",
            Self::MergeReadinessStale => "MERGE_READINESS_STALE",
            Self::MergeConflictRequiresImplementer => "MERGE_CONFLICT_REQUIRES_IMPLEMENTER",
            Self::PrimaryBranchApprovalRequired => "PRIMARY_BRANCH_APPROVAL_REQUIRED",
            Self::MemoryCannotAuthorize => "MEMORY_CANNOT_AUTHORIZE",
            Self::MemoryCrossProject => "MEMORY_CROSS_PROJECT",
            Self::MemoryProvenanceRequired => "MEMORY_PROVENANCE_REQUIRED",
            Self::MemoryReviewRequired => "MEMORY_REVIEW_REQUIRED",
            Self::PreferenceUserEvidenceRequired => "PREFERENCE_USER_EVIDENCE_REQUIRED",
            Self::UpgradeStageDenied => "UPGRADE_STAGE_DENIED",
            Self::UpgradeDeltaProtected => "UPGRADE_DELTA_PROTECTED",
            Self::UpgradeSchemaMigrationDenied => "UPGRADE_SCHEMA_MIGRATION_DENIED",
            Self::GuardianRequired => "GUARDIAN_REQUIRED",
            Self::GuardianApprovalRequired => "GUARDIAN_APPROVAL_REQUIRED",
            Self::ResourceEvidenceStale => "RESOURCE_EVIDENCE_STALE",
            Self::ResourceCurrencyMismatch => "RESOURCE_CURRENCY_MISMATCH",
            Self::RecoveryAuthorityRequired => "RECOVERY_AUTHORITY_REQUIRED",
            Self::RecoveryAuthorityMismatch => "RECOVERY_AUTHORITY_MISMATCH",
            Self::InternalPolicyError => "INTERNAL_POLICY_ERROR",
            Self::AgentActionAllowed => "AGENT_ACTION_ALLOWED",
            Self::ExecutionGateAllowed => "EXECUTION_GATE_ALLOWED",
            Self::WorkerAdmissionAllowed => "WORKER_ADMISSION_ALLOWED",
            Self::MergeGateAllowed => "MERGE_GATE_ALLOWED",
            Self::MemoryPromotionAllowed => "MEMORY_PROMOTION_ALLOWED",
            Self::UpgradeStageAllowed => "UPGRADE_STAGE_ALLOWED",
            Self::RecoveryGateAllowed => "RECOVERY_GATE_ALLOWED",
            Self::ProtectedChangeAllowed => "PROTECTED_CHANGE_ALLOWED",
        }
    }
}

/// Bounded deterministic evidence for one policy decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyEvidence {
    contract_version: u16,
    subject: DecisionKind,
    checked_through: DecisionStage,
}

impl PolicyEvidence {
    pub(crate) const fn new(subject: DecisionKind, checked_through: DecisionStage) -> Self {
        Self {
            contract_version: POLICY_CONTRACT_VERSION,
            subject,
            checked_through,
        }
    }

    /// Returns the policy contract version.
    #[must_use]
    pub const fn contract_version(self) -> u16 {
        self.contract_version
    }

    /// Returns the evaluated subject class.
    #[must_use]
    pub const fn subject(self) -> DecisionKind {
        self.subject
    }

    /// Returns the last completed decision stage.
    #[must_use]
    pub const fn checked_through(self) -> DecisionStage {
        self.checked_through
    }
}

/// A complete typed policy outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyDecision {
    allowed: bool,
    reason: PolicyReason,
    evidence: PolicyEvidence,
}

impl PolicyDecision {
    pub(crate) const fn deny(
        subject: DecisionKind,
        reason: PolicyReason,
        checked_through: DecisionStage,
    ) -> Self {
        Self {
            allowed: false,
            reason,
            evidence: PolicyEvidence::new(subject, checked_through),
        }
    }

    pub(crate) const fn allow(subject: DecisionKind, reason: PolicyReason) -> Self {
        Self {
            allowed: true,
            reason,
            evidence: PolicyEvidence::new(subject, DecisionStage::Complete),
        }
    }

    /// Returns whether every applicable gate passed.
    #[must_use]
    pub const fn allowed(self) -> bool {
        self.allowed
    }

    /// Returns the stable typed reason.
    #[must_use]
    pub const fn reason(self) -> PolicyReason {
        self.reason
    }

    /// Returns bounded deterministic decision evidence.
    #[must_use]
    pub const fn evidence(self) -> PolicyEvidence {
        self.evidence
    }
}

/// Opaque owned evidence captured from the exact `ExecutionGate` passed to
/// [`crate::evaluate`]. Only Policy can construct this value; downstream
/// verifiers can therefore bind an authority to the decision's actual input
/// facts rather than trusting caller-selected hashes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionGateDecisionEvidence {
    decision: PolicyDecision,
    task_spec_digest: Option<ContentDigest>,
    project_binding: Option<SubjectBinding>,
    project_receipt: Option<ProjectAuthorityReceipt>,
    current_project_head: Option<ProjectAuthorityHead>,
    managed_execution_binding: Option<ManagedExecutionBindingFact>,
    state: Boundary<TaskState>,
    runtime_admission: Boundary<RuntimeAdmission>,
}

impl ExecutionGateDecisionEvidence {
    pub(crate) const fn new(
        decision: PolicyDecision,
        task_spec_digest: Option<ContentDigest>,
        project_binding: Option<SubjectBinding>,
        project_receipt: Option<ProjectAuthorityReceipt>,
        current_project_head: Option<ProjectAuthorityHead>,
        managed_execution_binding: Option<ManagedExecutionBindingFact>,
        state: Boundary<TaskState>,
        runtime_admission: Boundary<RuntimeAdmission>,
    ) -> Self {
        Self {
            decision,
            task_spec_digest,
            project_binding,
            project_receipt,
            current_project_head,
            managed_execution_binding,
            state,
            runtime_admission,
        }
    }

    #[must_use]
    pub const fn decision(&self) -> PolicyDecision {
        self.decision
    }

    #[must_use]
    pub const fn task_spec_digest(&self) -> Option<&ContentDigest> {
        self.task_spec_digest.as_ref()
    }

    #[must_use]
    pub const fn project_binding(&self) -> Option<&SubjectBinding> {
        self.project_binding.as_ref()
    }

    #[must_use]
    pub const fn project_receipt(&self) -> Option<&ProjectAuthorityReceipt> {
        self.project_receipt.as_ref()
    }

    #[must_use]
    pub const fn current_project_head(&self) -> Option<&ProjectAuthorityHead> {
        self.current_project_head.as_ref()
    }

    /// Returns the exact Task-Ledger execution binding evaluated by the
    /// managed Policy lane. `None` identifies legacy unbound evidence and must
    /// fail closed at any execution-authority consumer.
    #[must_use]
    pub const fn managed_execution_binding(&self) -> Option<&ManagedExecutionBindingFact> {
        self.managed_execution_binding.as_ref()
    }

    /// The Phase-4 closed lane is issued only from this exact Task state.
    #[must_use]
    pub const fn is_awaiting_execution_approval(&self) -> bool {
        matches!(
            self.state,
            Boundary::Known(TaskState::AwaitingExecutionApproval)
        )
    }

    /// The Phase-4 closed lane is issued only while Runtime is exactly active.
    #[must_use]
    pub const fn is_runtime_active(&self) -> bool {
        matches!(
            self.runtime_admission,
            Boundary::Known(RuntimeAdmission::Active)
        )
    }
}
