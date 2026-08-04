//! Read-only characterization of the safe Policy V1 matrix.
//!
//! Nothing in this module is an active V2 authority. In particular, the
//! historical protected actions remain recognizable only so regression tests
//! can prove that they are denied and absent from [`crate::PolicyAction`].

use crate::{AgentRole, SubjectBinding};

pub use lattice_task_domain::TaskState as State;

/// Hard V1 worker ceiling retained as a conservative V2 bound.
pub const GLOBAL_WORKER_CAP: usize = 4;

/// Frozen V1-only writer target used solely by characterization tests.
///
/// This caller-owned shape is intentionally quarantined from active V2 Policy
/// authority. Policy 2.5 uses fixed-producer Writer Lease receipts instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseSubject {
    pub lease_holder_id: String,
    pub worktree_id: String,
    pub daemon_instance_id: String,
    pub daemon_epoch: u64,
    pub fencing_token: u64,
}

/// Frozen V1-only lease fact retained as vulnerability characterization.
///
/// Its booleans and counters are not accepted by active V2 evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriterLeaseFact {
    pub binding: SubjectBinding,
    pub active: bool,
    pub current: bool,
    pub holder_role: AgentRole,
    pub subject: WriterLeaseSubject,
    pub current_daemon_epoch: u64,
    pub current_fencing_token: u64,
    pub active_implementers: u64,
}

macro_rules! legacy_wire_enum {
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
            /// Parses an exact historical wire value.
            #[must_use]
            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($wire => Some(Self::$variant),)+
                    _ => None,
                }
            }

            /// Returns the exact historical wire value.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }
        }
    };
}

legacy_wire_enum! {
    /// Frozen V1 role set. `Graphify` is characterization, not a V2 role.
    pub enum Role {
        LatticePm => "LATTICE_PM",
        Planner => "PLANNER",
        CodeMapper => "CODE_MAPPER",
        Graphify => "GRAPHIFY",
        Implementer => "IMPLEMENTER",
        CorrectnessReviewer => "CORRECTNESS_REVIEWER",
        SecurityReviewer => "SECURITY_REVIEWER",
        ArchitectureReviewer => "ARCHITECTURE_REVIEWER",
        Integrator => "INTEGRATOR",
    }
}

impl Role {
    /// Complete frozen V1 role set.
    pub const ALL: [Self; 9] = [
        Self::LatticePm,
        Self::Planner,
        Self::CodeMapper,
        Self::Graphify,
        Self::Implementer,
        Self::CorrectnessReviewer,
        Self::SecurityReviewer,
        Self::ArchitectureReviewer,
        Self::Integrator,
    ];
}

legacy_wire_enum! {
    /// Frozen V1 action set.
    ///
    /// The ten historical protected variants are never active V2 actions.
    pub enum Action {
        ReadRepository => "READ_REPOSITORY",
        SubmitPlan => "SUBMIT_PLAN",
        PlanTask => "PLAN_TASK",
        MapCode => "MAP_CODE",
        WriteProductCode => "WRITE_PRODUCT_CODE",
        RunTests => "RUN_TESTS",
        ReviewCorrectness => "REVIEW_CORRECTNESS",
        ReviewSecurity => "REVIEW_SECURITY",
        ReviewArchitecture => "REVIEW_ARCHITECTURE",
        PrepareWorktree => "PREPARE_WORKTREE",
        IntegrateGit => "INTEGRATE_GIT",
        StopRuntime => "STOP_RUNTIME",
        ResolveMergeConflict => "RESOLVE_MERGE_CONFLICT",
        CallRealModel => "CALL_REAL_MODEL",
        NetworkAccess => "NETWORK_ACCESS",
        DeployProduction => "DEPLOY_PRODUCTION",
        PurchaseService => "PURCHASE_SERVICE",
        ManageCredentials => "MANAGE_CREDENTIALS",
        PublicPublish => "PUBLIC_PUBLISH",
        PermanentDelete => "PERMANENT_DELETE",
        AccessPlaymate => "ACCESS_PLAYMATE",
        DisableSecurity => "DISABLE_SECURITY",
    }
}

impl Action {
    /// Complete frozen V1 action set.
    pub const ALL: [Self; 22] = [
        Self::ReadRepository,
        Self::SubmitPlan,
        Self::PlanTask,
        Self::MapCode,
        Self::WriteProductCode,
        Self::RunTests,
        Self::ReviewCorrectness,
        Self::ReviewSecurity,
        Self::ReviewArchitecture,
        Self::PrepareWorktree,
        Self::IntegrateGit,
        Self::StopRuntime,
        Self::ResolveMergeConflict,
        Self::CallRealModel,
        Self::NetworkAccess,
        Self::DeployProduction,
        Self::PurchaseService,
        Self::ManageCredentials,
        Self::PublicPublish,
        Self::PermanentDelete,
        Self::AccessPlaymate,
        Self::DisableSecurity,
    ];

    /// Historical protected variants, retained only as deny regressions.
    pub const PROTECTED: [Self; 10] = [
        Self::ResolveMergeConflict,
        Self::CallRealModel,
        Self::NetworkAccess,
        Self::DeployProduction,
        Self::PurchaseService,
        Self::ManageCredentials,
        Self::PublicPublish,
        Self::PermanentDelete,
        Self::AccessPlaymate,
        Self::DisableSecurity,
    ];

    /// Returns whether V1 classified this action as protected.
    #[must_use]
    pub const fn is_protected(self) -> bool {
        matches!(
            self,
            Self::ResolveMergeConflict
                | Self::CallRealModel
                | Self::NetworkAccess
                | Self::DeployProduction
                | Self::PurchaseService
                | Self::ManageCredentials
                | Self::PublicPublish
                | Self::PermanentDelete
                | Self::AccessPlaymate
                | Self::DisableSecurity
        )
    }
}

/// Complete frozen V1 state set, owned by Task Domain V2.
pub const STATES: [State; 14] = State::ALL;

/// Stable reason from the read-only V1 characterization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reason {
    /// Historical protected action, always denied before matrix checks.
    Phase1ProtectedAction,
    /// The role/action pair is absent from the V1 matrix.
    RoleActionDenied,
    /// The action is not admitted in the supplied state.
    ActionStateDenied,
    /// A write caller omitted the immutable expected subject.
    WriterSubjectRequired,
    /// A write caller omitted its lease fact.
    WriterLeaseRequired,
    /// The lease is inactive, stale, or bound to a stale daemon epoch.
    WriterLeaseNotCurrent,
    /// The lease does not match the exact expected subject or holder.
    WriterLeaseSubjectMismatch,
    /// The fencing token is zero or differs from the current token.
    FencingTokenMismatch,
    /// The writer fact does not prove exactly one active Implementer.
    MultipleImplementers,
    /// The Task Spec worker limit is absent or outside one through four.
    WorkerEnvelopeDenied,
    /// Checked worker accounting exceeds the effective limit.
    AgentLimitExceeded,
    /// The frozen action matrix and all applicable writer checks passed.
    AgentActionAllowed,
    /// Checked worker admission passed.
    WorkerAdmissionAllowed,
}

impl Reason {
    /// Returns the stable characterization reason code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Phase1ProtectedAction => "PHASE1_PROTECTED_ACTION",
            Self::RoleActionDenied => "ROLE_ACTION_DENIED",
            Self::ActionStateDenied => "ACTION_STATE_DENIED",
            Self::WriterSubjectRequired => "WRITER_SUBJECT_REQUIRED",
            Self::WriterLeaseRequired => "WRITER_LEASE_REQUIRED",
            Self::WriterLeaseNotCurrent => "WRITER_LEASE_NOT_CURRENT",
            Self::WriterLeaseSubjectMismatch => "WRITER_LEASE_SUBJECT_MISMATCH",
            Self::FencingTokenMismatch => "FENCING_TOKEN_MISMATCH",
            Self::MultipleImplementers => "MULTIPLE_IMPLEMENTERS",
            Self::WorkerEnvelopeDenied => "WORKER_ENVELOPE_DENIED",
            Self::AgentLimitExceeded => "AGENT_LIMIT_EXCEEDED",
            Self::AgentActionAllowed => "AGENT_ACTION_ALLOWED",
            Self::WorkerAdmissionAllowed => "WORKER_ADMISSION_ALLOWED",
        }
    }
}

/// Immutable result from a V1 characterization function.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Decision {
    allowed: bool,
    reason: Reason,
}

impl Decision {
    /// Returns whether the characterization allows the request.
    #[must_use]
    pub const fn allowed(self) -> bool {
        self.allowed
    }

    /// Returns the stable characterization reason.
    #[must_use]
    pub const fn reason(self) -> Reason {
        self.reason
    }

    const fn allow(reason: Reason) -> Self {
        Self {
            allowed: true,
            reason,
        }
    }

    const fn deny(reason: Reason) -> Self {
        Self {
            allowed: false,
            reason,
        }
    }
}

/// Characterizes one known V1 role/action/state request.
///
/// The decision order is intentionally frozen as protected, role/action,
/// state, then writer evidence. A product-code write additionally requires a
/// complete expected subject and the frozen characterization-only writer fact.
#[must_use]
pub fn characterize_agent_action(
    role: Role,
    action: Action,
    state: State,
    expected_subject: Option<&SubjectBinding>,
    writer: Option<&WriterLeaseFact>,
    actor_id: &str,
) -> Decision {
    if action.is_protected() {
        return Decision::deny(Reason::Phase1ProtectedAction);
    }
    if !role_allows_action(role, action) {
        return Decision::deny(Reason::RoleActionDenied);
    }
    if !state_allows_action(state, action) {
        return Decision::deny(Reason::ActionStateDenied);
    }
    if action == Action::WriteProductCode
        && let Some(reason) = writer_denial(expected_subject, writer, actor_id)
    {
        return Decision::deny(reason);
    }
    Decision::allow(Reason::AgentActionAllowed)
}

/// Characterizes the retained global V1 worker ceiling with checked addition.
#[must_use]
pub fn characterize_worker_admission(
    active_workers: usize,
    requested_workers: usize,
    task_max_agents: Option<usize>,
) -> Decision {
    let Some(task_max_agents) = task_max_agents else {
        return Decision::deny(Reason::WorkerEnvelopeDenied);
    };
    if !(1..=GLOBAL_WORKER_CAP).contains(&task_max_agents) {
        return Decision::deny(Reason::WorkerEnvelopeDenied);
    }
    let Some(resulting_workers) = active_workers.checked_add(requested_workers) else {
        return Decision::deny(Reason::AgentLimitExceeded);
    };
    let limit = task_max_agents.min(GLOBAL_WORKER_CAP);
    if resulting_workers > limit {
        return Decision::deny(Reason::AgentLimitExceeded);
    }
    Decision::allow(Reason::WorkerAdmissionAllowed)
}

const fn role_allows_action(role: Role, action: Action) -> bool {
    match role {
        Role::LatticePm => matches!(
            action,
            Action::ReadRepository | Action::SubmitPlan | Action::StopRuntime
        ),
        Role::Planner => matches!(action, Action::ReadRepository | Action::PlanTask),
        Role::CodeMapper | Role::Graphify => {
            matches!(action, Action::ReadRepository | Action::MapCode)
        }
        Role::Implementer => matches!(
            action,
            Action::ReadRepository | Action::WriteProductCode | Action::RunTests
        ),
        Role::CorrectnessReviewer => {
            matches!(action, Action::ReadRepository | Action::ReviewCorrectness)
        }
        Role::SecurityReviewer => {
            matches!(action, Action::ReadRepository | Action::ReviewSecurity)
        }
        Role::ArchitectureReviewer => {
            matches!(action, Action::ReadRepository | Action::ReviewArchitecture)
        }
        Role::Integrator => matches!(
            action,
            Action::ReadRepository | Action::PrepareWorktree | Action::IntegrateGit
        ),
    }
}

const fn state_allows_action(state: State, action: Action) -> bool {
    match action {
        Action::ReadRepository => true,
        Action::SubmitPlan | Action::PlanTask => matches!(state, State::Draft),
        Action::MapCode => matches!(state, State::Draft | State::AwaitingExecutionApproval),
        Action::WriteProductCode => matches!(state, State::Executing),
        Action::RunTests => matches!(state, State::Executing | State::Verifying),
        Action::ReviewCorrectness | Action::ReviewSecurity | Action::ReviewArchitecture => {
            matches!(state, State::Reviewing)
        }
        Action::PrepareWorktree => matches!(state, State::Preparing),
        Action::IntegrateGit => matches!(state, State::Merging),
        Action::StopRuntime => matches!(
            state,
            State::Preparing
                | State::Executing
                | State::Verifying
                | State::Reviewing
                | State::Merging
                | State::Stopping
        ),
        Action::ResolveMergeConflict
        | Action::CallRealModel
        | Action::NetworkAccess
        | Action::DeployProduction
        | Action::PurchaseService
        | Action::ManageCredentials
        | Action::PublicPublish
        | Action::PermanentDelete
        | Action::AccessPlaymate
        | Action::DisableSecurity => false,
    }
}

fn writer_denial(
    expected_subject: Option<&SubjectBinding>,
    writer: Option<&WriterLeaseFact>,
    actor_id: &str,
) -> Option<Reason> {
    let Some(expected_subject) = expected_subject else {
        return Some(Reason::WriterSubjectRequired);
    };
    let Some(writer) = writer else {
        return Some(Reason::WriterLeaseRequired);
    };
    if !writer.active
        || !writer.current
        || writer.holder_role != AgentRole::Implementer
        || writer.subject.daemon_epoch == 0
        || writer.subject.daemon_epoch != writer.current_daemon_epoch
    {
        return Some(Reason::WriterLeaseNotCurrent);
    }
    if &writer.binding != expected_subject
        || actor_id.is_empty()
        || writer.subject.lease_holder_id != actor_id
        || writer.subject.worktree_id.is_empty()
        || writer.subject.daemon_instance_id.is_empty()
    {
        return Some(Reason::WriterLeaseSubjectMismatch);
    }
    if writer.subject.fencing_token == 0
        || writer.subject.fencing_token != writer.current_fencing_token
    {
        return Some(Reason::FencingTokenMismatch);
    }
    if writer.active_implementers != 1 {
        return Some(Reason::MultipleImplementers);
    }
    None
}
