use lattice_contracts::{ContentDigest, ProjectId, ProjectSnapshotId, TaskId};
use lattice_policy::v1_compat::{
    Action, GLOBAL_WORKER_CAP, Reason, Role, STATES, State, WriterLeaseFact, WriterLeaseSubject,
    characterize_agent_action, characterize_worker_admission,
};
use lattice_policy::{AgentRole, Boundary, PolicyAction, SubjectBinding};

#[test]
fn frozen_v1_sets_have_exact_counts_and_wire_round_trips() {
    assert_eq!(Role::ALL.len(), 9);
    assert_eq!(Action::ALL.len(), 22);
    assert_eq!(STATES.len(), 14);

    for role in Role::ALL {
        assert_eq!(Role::parse(role.as_str()), Some(role));
    }
    for action in Action::ALL {
        assert_eq!(Action::parse(action.as_str()), Some(action));
    }
    for state in STATES {
        assert_eq!(State::parse(state.as_str()).expect("known V1 state"), state);
    }
}

#[test]
fn protected_role_and_state_reason_precedence_is_frozen() {
    let protected = characterize_agent_action(
        Role::Planner,
        Action::CallRealModel,
        State::Completed,
        None,
        None,
        "",
    );
    assert_eq!(protected.reason(), Reason::Phase1ProtectedAction);

    let role_denied = characterize_agent_action(
        Role::Planner,
        Action::WriteProductCode,
        State::Draft,
        None,
        None,
        "",
    );
    assert_eq!(role_denied.reason(), Reason::RoleActionDenied);

    let state_denied = characterize_agent_action(
        Role::Planner,
        Action::PlanTask,
        State::Executing,
        None,
        None,
        "",
    );
    assert_eq!(state_denied.reason(), Reason::ActionStateDenied);
}

#[test]
fn complete_v1_role_action_matrix_is_preserved_as_characterization() {
    let subject = subject();
    let writer = writer(&subject);
    let expected = [
        (
            Role::LatticePm,
            [
                Action::ReadRepository,
                Action::SubmitPlan,
                Action::StopRuntime,
            ]
            .as_slice(),
        ),
        (
            Role::Planner,
            [Action::ReadRepository, Action::PlanTask].as_slice(),
        ),
        (
            Role::CodeMapper,
            [Action::ReadRepository, Action::MapCode].as_slice(),
        ),
        (
            Role::Graphify,
            [Action::ReadRepository, Action::MapCode].as_slice(),
        ),
        (
            Role::Implementer,
            [
                Action::ReadRepository,
                Action::WriteProductCode,
                Action::RunTests,
            ]
            .as_slice(),
        ),
        (
            Role::CorrectnessReviewer,
            [Action::ReadRepository, Action::ReviewCorrectness].as_slice(),
        ),
        (
            Role::SecurityReviewer,
            [Action::ReadRepository, Action::ReviewSecurity].as_slice(),
        ),
        (
            Role::ArchitectureReviewer,
            [Action::ReadRepository, Action::ReviewArchitecture].as_slice(),
        ),
        (
            Role::Integrator,
            [
                Action::ReadRepository,
                Action::PrepareWorktree,
                Action::IntegrateGit,
            ]
            .as_slice(),
        ),
    ];

    for (role, allowed_actions) in expected {
        for action in Action::ALL
            .into_iter()
            .filter(|action| !action.is_protected())
        {
            let decision = characterize_agent_action(
                role,
                action,
                admitted_state(action),
                Some(&subject),
                Some(&writer),
                "implementer-1",
            );
            assert_eq!(
                decision.allowed(),
                allowed_actions.contains(&action),
                "{} {} returned {}",
                role.as_str(),
                action.as_str(),
                decision.reason().code(),
            );
        }
    }
}

#[test]
fn complete_v1_action_state_matrix_is_preserved_as_characterization() {
    let subject = subject();
    let writer = writer(&subject);

    for action in Action::ALL
        .into_iter()
        .filter(|action| !action.is_protected())
    {
        for state in STATES {
            let decision = characterize_agent_action(
                admitted_role(action),
                action,
                state,
                Some(&subject),
                Some(&writer),
                "implementer-1",
            );
            assert_eq!(
                decision.allowed(),
                state_allows(action, state),
                "{} {} returned {}",
                action.as_str(),
                state.as_str(),
                decision.reason().code(),
            );
        }
    }
}

#[test]
fn every_historical_protected_action_is_always_denied() {
    for role in Role::ALL {
        for action in Action::PROTECTED {
            for state in STATES {
                let decision =
                    characterize_agent_action(role, action, state, None, None, "implementer-1");
                assert!(!decision.allowed());
                assert_eq!(decision.reason(), Reason::Phase1ProtectedAction);
            }
        }
    }
}

#[test]
fn legacy_project_specific_and_effect_actions_are_not_active_v2_actions() {
    for action in Action::PROTECTED {
        assert_eq!(
            PolicyAction::parse(action.as_str()),
            Boundary::Unknown,
            "{} must remain outside active PolicyAction",
            action.as_str(),
        );
    }
}

#[test]
fn missing_subject_and_unbound_or_stale_writer_evidence_deny() {
    let subject = subject();
    let valid_writer = writer(&subject);

    let missing_subject = characterize_agent_action(
        Role::Implementer,
        Action::WriteProductCode,
        State::Executing,
        None,
        Some(&valid_writer),
        "implementer-1",
    );
    assert_eq!(missing_subject.reason(), Reason::WriterSubjectRequired);

    let missing_writer = characterize_agent_action(
        Role::Implementer,
        Action::WriteProductCode,
        State::Executing,
        Some(&subject),
        None,
        "implementer-1",
    );
    assert_eq!(missing_writer.reason(), Reason::WriterLeaseRequired);

    let mut unbound_writer = valid_writer.clone();
    unbound_writer.binding = SubjectBinding::new(
        ProjectId::new("another-project").expect("project"),
        unbound_writer.binding.project_snapshot_id().clone(),
        unbound_writer.binding.task_id().clone(),
        unbound_writer.binding.task_revision(),
        unbound_writer.binding.task_spec_digest().clone(),
    )
    .expect("binding");
    let unbound = characterize_agent_action(
        Role::Implementer,
        Action::WriteProductCode,
        State::Executing,
        Some(&subject),
        Some(&unbound_writer),
        "implementer-1",
    );
    assert_eq!(unbound.reason(), Reason::WriterLeaseSubjectMismatch);

    let mut stale_writer = valid_writer.clone();
    stale_writer.current = false;
    let stale = characterize_agent_action(
        Role::Implementer,
        Action::WriteProductCode,
        State::Executing,
        Some(&subject),
        Some(&stale_writer),
        "implementer-1",
    );
    assert_eq!(stale.reason(), Reason::WriterLeaseNotCurrent);

    let mut stale_epoch = valid_writer.clone();
    stale_epoch.current_daemon_epoch += 1;
    let stale = characterize_agent_action(
        Role::Implementer,
        Action::WriteProductCode,
        State::Executing,
        Some(&subject),
        Some(&stale_epoch),
        "implementer-1",
    );
    assert_eq!(stale.reason(), Reason::WriterLeaseNotCurrent);

    let mut stale_fence = valid_writer;
    stale_fence.current_fencing_token += 1;
    let stale = characterize_agent_action(
        Role::Implementer,
        Action::WriteProductCode,
        State::Executing,
        Some(&subject),
        Some(&stale_fence),
        "implementer-1",
    );
    assert_eq!(stale.reason(), Reason::FencingTokenMismatch);
}

#[test]
fn worker_admission_keeps_the_global_cap_of_four_and_checks_overflow() {
    assert_eq!(GLOBAL_WORKER_CAP, 4);
    let active = [Role::Planner, Role::CodeMapper];
    let requested = [Role::Implementer, Role::SecurityReviewer];

    let four = characterize_worker_admission(active.len(), requested.len(), Some(4));
    assert!(four.allowed());
    assert_eq!(four.reason(), Reason::WorkerAdmissionAllowed);

    let five = characterize_worker_admission(active.len(), 3, Some(4));
    assert!(!five.allowed());
    assert_eq!(five.reason(), Reason::AgentLimitExceeded);

    assert_eq!(
        characterize_worker_admission(0, 0, None).reason(),
        Reason::WorkerEnvelopeDenied,
    );
    assert_eq!(
        characterize_worker_admission(0, 0, Some(5)).reason(),
        Reason::WorkerEnvelopeDenied,
    );
    assert_eq!(
        characterize_worker_admission(usize::MAX, 1, Some(4)).reason(),
        Reason::AgentLimitExceeded,
    );
}

fn admitted_state(action: Action) -> State {
    match action {
        Action::ReadRepository | Action::SubmitPlan | Action::PlanTask | Action::MapCode => {
            State::Draft
        }
        Action::WriteProductCode | Action::RunTests | Action::StopRuntime => State::Executing,
        Action::ReviewCorrectness | Action::ReviewSecurity | Action::ReviewArchitecture => {
            State::Reviewing
        }
        Action::PrepareWorktree => State::Preparing,
        Action::IntegrateGit => State::Merging,
        Action::ResolveMergeConflict
        | Action::CallRealModel
        | Action::NetworkAccess
        | Action::DeployProduction
        | Action::PurchaseService
        | Action::ManageCredentials
        | Action::PublicPublish
        | Action::PermanentDelete
        | Action::AccessPlaymate
        | Action::DisableSecurity => State::Completed,
    }
}

fn admitted_role(action: Action) -> Role {
    match action {
        Action::ReadRepository
        | Action::SubmitPlan
        | Action::StopRuntime
        | Action::ResolveMergeConflict
        | Action::CallRealModel
        | Action::NetworkAccess
        | Action::DeployProduction
        | Action::PurchaseService
        | Action::ManageCredentials
        | Action::PublicPublish
        | Action::PermanentDelete
        | Action::AccessPlaymate
        | Action::DisableSecurity => Role::LatticePm,
        Action::PlanTask => Role::Planner,
        Action::MapCode => Role::CodeMapper,
        Action::WriteProductCode | Action::RunTests => Role::Implementer,
        Action::ReviewCorrectness => Role::CorrectnessReviewer,
        Action::ReviewSecurity => Role::SecurityReviewer,
        Action::ReviewArchitecture => Role::ArchitectureReviewer,
        Action::PrepareWorktree | Action::IntegrateGit => Role::Integrator,
    }
}

fn state_allows(action: Action, state: State) -> bool {
    match action {
        Action::ReadRepository => true,
        Action::SubmitPlan | Action::PlanTask => state == State::Draft,
        Action::MapCode => matches!(state, State::Draft | State::AwaitingExecutionApproval),
        Action::WriteProductCode => state == State::Executing,
        Action::RunTests => matches!(state, State::Executing | State::Verifying),
        Action::ReviewCorrectness | Action::ReviewSecurity | Action::ReviewArchitecture => {
            state == State::Reviewing
        }
        Action::PrepareWorktree => state == State::Preparing,
        Action::IntegrateGit => state == State::Merging,
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

fn subject() -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new("general-ai-platform").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-011").expect("task"),
        "1",
        ContentDigest::from_sha256("a".repeat(64)).expect("digest"),
    )
    .expect("binding")
}

fn writer(subject: &SubjectBinding) -> WriterLeaseFact {
    WriterLeaseFact {
        binding: subject.clone(),
        active: true,
        current: true,
        holder_role: AgentRole::Implementer,
        subject: WriterLeaseSubject {
            lease_holder_id: "implementer-1".to_owned(),
            worktree_id: "worktree-1".to_owned(),
            daemon_instance_id: "daemon-1".to_owned(),
            daemon_epoch: 7,
            fencing_token: 11,
        },
        current_daemon_epoch: 7,
        current_fencing_token: 11,
        active_implementers: 1,
    }
}
