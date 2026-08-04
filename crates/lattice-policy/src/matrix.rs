use lattice_task_domain::{Capability, TaskState};

use crate::{AgentRole, Boundary, PolicyAction, ProviderKind, RuntimeAdmission, UpgradeStage};

#[derive(Clone, Copy)]
pub(crate) struct ProviderRequirement {
    pub provider: ProviderKind,
    pub capability: Capability,
}

pub(crate) const fn runtime_allows_action(
    runtime: RuntimeAdmission,
    action: Boundary<PolicyAction>,
) -> bool {
    let Boundary::Known(action) = action else {
        return true;
    };
    match runtime {
        RuntimeAdmission::Active => true,
        RuntimeAdmission::Draining => matches!(
            action,
            PolicyAction::StopRuntime
                | PolicyAction::ReconcileRuntime
                | PolicyAction::ReleaseWriter
        ),
        RuntimeAdmission::Canary => matches!(
            action,
            PolicyAction::GuardianHealth | PolicyAction::RollbackUpgrade
        ),
        RuntimeAdmission::Stopped | RuntimeAdmission::ReconciliationRequired => matches!(
            action,
            PolicyAction::StopRuntime
                | PolicyAction::ReconcileRuntime
                | PolicyAction::RollbackUpgrade
        ),
    }
}

pub(crate) const fn role_allows_action(role: AgentRole, action: PolicyAction) -> bool {
    match role {
        AgentRole::LatticePm => matches!(
            action,
            PolicyAction::SubmitPlan
                | PolicyAction::StopRuntime
                | PolicyAction::ProposeUpgrade
                | PolicyAction::RequestProtectedChange
        ),
        AgentRole::Planner => matches!(
            action,
            PolicyAction::ReadRepository | PolicyAction::PlanTask | PolicyAction::ReadMemory
        ),
        AgentRole::CodeMapper => matches!(
            action,
            PolicyAction::ReadRepository
                | PolicyAction::MapCode
                | PolicyAction::RunGraphify
                | PolicyAction::ReadMemory
        ),
        AgentRole::Researcher => matches!(
            action,
            PolicyAction::ReadRepository
                | PolicyAction::Research
                | PolicyAction::RunHermes
                | PolicyAction::ProposeMemory
        ),
        AgentRole::Implementer => matches!(
            action,
            PolicyAction::ReadRepository
                | PolicyAction::PrepareWorktree
                | PolicyAction::WriteProductCode
                | PolicyAction::RunTests
                | PolicyAction::RunCodex
                | PolicyAction::ReadMemory
                | PolicyAction::ReleaseWriter
        ),
        AgentRole::CorrectnessReviewer => matches!(
            action,
            PolicyAction::ReadRepository | PolicyAction::ReviewCorrectness
        ),
        AgentRole::SecurityReviewer => matches!(
            action,
            PolicyAction::ReadRepository | PolicyAction::ReviewSecurity
        ),
        AgentRole::ArchitectureReviewer => matches!(
            action,
            PolicyAction::ReadRepository | PolicyAction::ReviewArchitecture
        ),
        AgentRole::MemoryReviewer => matches!(
            action,
            PolicyAction::ReadMemory | PolicyAction::PromoteMemory
        ),
        AgentRole::Integrator => matches!(
            action,
            PolicyAction::ReadRepository
                | PolicyAction::PrepareWorktree
                | PolicyAction::IntegrateGit
        ),
        AgentRole::UpgradeGuardian => matches!(
            action,
            PolicyAction::ReconcileRuntime
                | PolicyAction::GuardianShadow
                | PolicyAction::GuardianHealth
                | PolicyAction::ActivateUpgrade
                | PolicyAction::RollbackUpgrade
        ),
    }
}

pub(crate) const fn state_allows_action(state: TaskState, action: PolicyAction) -> bool {
    match action {
        PolicyAction::ReadRepository | PolicyAction::ReadMemory => true,
        PolicyAction::SubmitPlan | PolicyAction::PlanTask | PolicyAction::ProposeUpgrade => {
            matches!(state, TaskState::Draft)
        }
        PolicyAction::MapCode
        | PolicyAction::Research
        | PolicyAction::RunGraphify
        | PolicyAction::RunHermes
        | PolicyAction::ProposeMemory => matches!(
            state,
            TaskState::Draft | TaskState::AwaitingExecutionApproval
        ),
        PolicyAction::PrepareWorktree => matches!(state, TaskState::Preparing),
        PolicyAction::WriteProductCode | PolicyAction::RunCodex => {
            matches!(state, TaskState::Executing)
        }
        PolicyAction::RunTests => matches!(state, TaskState::Executing | TaskState::Verifying),
        PolicyAction::ReviewCorrectness
        | PolicyAction::ReviewSecurity
        | PolicyAction::ReviewArchitecture
        | PolicyAction::PromoteMemory
        | PolicyAction::GuardianShadow => matches!(state, TaskState::Reviewing),
        PolicyAction::IntegrateGit => matches!(state, TaskState::Merging),
        PolicyAction::StopRuntime => matches!(
            state,
            TaskState::Preparing
                | TaskState::Executing
                | TaskState::Verifying
                | TaskState::Reviewing
                | TaskState::Merging
                | TaskState::Stopping
                | TaskState::Blocked
                | TaskState::Failed
        ),
        PolicyAction::ReconcileRuntime
        | PolicyAction::ReleaseWriter
        | PolicyAction::RollbackUpgrade => {
            matches!(
                state,
                TaskState::Stopping | TaskState::Blocked | TaskState::Failed
            )
        }
        PolicyAction::GuardianHealth | PolicyAction::ActivateUpgrade => {
            matches!(state, TaskState::Merging)
        }
        PolicyAction::RequestProtectedChange => false,
    }
}

pub(crate) const fn is_protected_action(action: PolicyAction) -> bool {
    matches!(
        action,
        PolicyAction::PromoteMemory
            | PolicyAction::ReconcileRuntime
            | PolicyAction::ProposeUpgrade
            | PolicyAction::IntegrateGit
            | PolicyAction::GuardianShadow
            | PolicyAction::GuardianHealth
            | PolicyAction::RequestProtectedChange
            | PolicyAction::ActivateUpgrade
            | PolicyAction::RollbackUpgrade
    )
}

pub(crate) fn required_capabilities(action: PolicyAction) -> &'static [Capability] {
    match action {
        PolicyAction::ReadRepository => &[Capability::ReadRepository],
        PolicyAction::SubmitPlan | PolicyAction::PlanTask => &[Capability::PlanTask],
        PolicyAction::MapCode => &[Capability::MapCode],
        PolicyAction::Research | PolicyAction::ProposeMemory | PolicyAction::PromoteMemory => {
            &[Capability::ProposeMemory]
        }
        PolicyAction::PrepareWorktree => &[Capability::GitWorktree],
        PolicyAction::WriteProductCode => &[Capability::WriteProductCode],
        PolicyAction::RunTests => &[Capability::RunTests],
        PolicyAction::RunCodex => &[Capability::WriteProductCode, Capability::UseCodex],
        PolicyAction::RunGraphify => &[Capability::MapCode, Capability::UseGraphify],
        PolicyAction::RunHermes => &[Capability::ProposeMemory, Capability::UseHermes],
        PolicyAction::ReviewCorrectness
        | PolicyAction::ReviewSecurity
        | PolicyAction::ReviewArchitecture => &[Capability::ReadReview],
        PolicyAction::ReadMemory => &[Capability::ReadCodebaseMemory],
        PolicyAction::ProposeUpgrade
        | PolicyAction::GuardianShadow
        | PolicyAction::GuardianHealth
        | PolicyAction::ActivateUpgrade
        | PolicyAction::RollbackUpgrade => &[Capability::ProposeUpgrade],
        PolicyAction::IntegrateGit => &[Capability::GitIntegrate],
        PolicyAction::StopRuntime
        | PolicyAction::ReconcileRuntime
        | PolicyAction::ReleaseWriter => &[Capability::StopRuntime],
        PolicyAction::RequestProtectedChange => &[],
    }
}

pub(crate) const fn provider_requirement(action: PolicyAction) -> Option<ProviderRequirement> {
    match action {
        PolicyAction::RunCodex => Some(ProviderRequirement {
            provider: ProviderKind::Codex,
            capability: Capability::UseCodex,
        }),
        PolicyAction::RunGraphify => Some(ProviderRequirement {
            provider: ProviderKind::Graphify,
            capability: Capability::UseGraphify,
        }),
        PolicyAction::RunHermes => Some(ProviderRequirement {
            provider: ProviderKind::Hermes,
            capability: Capability::UseHermes,
        }),
        _ => None,
    }
}

pub(crate) const fn consumes_resources(action: PolicyAction) -> bool {
    !matches!(
        action,
        PolicyAction::ReadRepository
            | PolicyAction::ReadMemory
            | PolicyAction::StopRuntime
            | PolicyAction::ReconcileRuntime
            | PolicyAction::ReleaseWriter
            | PolicyAction::RollbackUpgrade
    )
}

pub(crate) const fn is_recovery_action(action: PolicyAction) -> bool {
    matches!(
        action,
        PolicyAction::StopRuntime
            | PolicyAction::ReconcileRuntime
            | PolicyAction::ReleaseWriter
            | PolicyAction::RollbackUpgrade
    )
}

pub(crate) const fn requires_execution_approval(action: PolicyAction) -> bool {
    matches!(
        action,
        PolicyAction::WriteProductCode | PolicyAction::RunTests | PolicyAction::RunCodex
    )
}

pub(crate) const fn requires_writer(action: PolicyAction) -> bool {
    matches!(
        action,
        PolicyAction::WriteProductCode | PolicyAction::RunTests | PolicyAction::RunCodex
    )
}

pub(crate) const fn targets_writer(action: PolicyAction) -> bool {
    matches!(action, PolicyAction::ReleaseWriter)
}

pub(crate) const fn upgrade_role(stage: UpgradeStage) -> AgentRole {
    match stage {
        UpgradeStage::Propose => AgentRole::LatticePm,
        UpgradeStage::Test => AgentRole::Implementer,
        UpgradeStage::Shadow
        | UpgradeStage::Activate
        | UpgradeStage::HealthCanary
        | UpgradeStage::Rollback => AgentRole::UpgradeGuardian,
    }
}

pub(crate) const fn upgrade_state_allowed(stage: UpgradeStage, task_state: TaskState) -> bool {
    match stage {
        UpgradeStage::Propose => matches!(task_state, TaskState::Draft),
        UpgradeStage::Test => matches!(task_state, TaskState::Executing | TaskState::Verifying),
        UpgradeStage::Shadow => matches!(task_state, TaskState::Reviewing),
        UpgradeStage::Activate | UpgradeStage::HealthCanary => {
            matches!(task_state, TaskState::Merging)
        }
        UpgradeStage::Rollback => {
            matches!(
                task_state,
                TaskState::Stopping | TaskState::Blocked | TaskState::Failed
            )
        }
    }
}

pub(crate) const fn upgrade_runtime_allowed(
    stage: UpgradeStage,
    runtime: RuntimeAdmission,
) -> bool {
    match stage {
        UpgradeStage::Propose | UpgradeStage::Test | UpgradeStage::Shadow => {
            matches!(runtime, RuntimeAdmission::Active)
        }
        UpgradeStage::Activate => matches!(runtime, RuntimeAdmission::Draining),
        UpgradeStage::HealthCanary => matches!(runtime, RuntimeAdmission::Canary),
        UpgradeStage::Rollback => matches!(
            runtime,
            RuntimeAdmission::Canary
                | RuntimeAdmission::Stopped
                | RuntimeAdmission::ReconciliationRequired
        ),
    }
}
