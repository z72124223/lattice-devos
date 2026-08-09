use lattice_passive_supervisor_contract::{
    CheckpointState, DependencyBlocker, DependencyReadiness, DependencySnapshot,
    UpstreamCheckpoint, dependency_readiness,
};

const TASK038_THREAD: &str = "019fe6e2-f8b9-7bb1-9d31-1e8992207d99";
const TASK039_THREAD: &str = "019fe759-d4d6-7950-bf10-fbda7487fa9b";

fn checkpoint(
    source_thread_id: &'static str,
    state: CheckpointState,
    branch: Option<&'static str>,
    commit: Option<&'static str>,
    contract_version: Option<&'static str>,
    verification_digest: Option<&'static str>,
) -> UpstreamCheckpoint<'static> {
    UpstreamCheckpoint {
        source_thread_id,
        state,
        branch,
        commit,
        contract_version,
        verification_digest,
    }
}

#[test]
fn in_progress_dependencies_return_exact_blockers() {
    let readiness = dependency_readiness(&DependencySnapshot {
        task_control: checkpoint(
            TASK038_THREAD,
            CheckpointState::InProgress,
            Some("feature/task-038-chatgpt-mcp"),
            None,
            None,
            None,
        ),
        worker_observation: checkpoint(
            TASK039_THREAD,
            CheckpointState::InProgress,
            None,
            None,
            None,
            None,
        ),
    });

    assert_eq!(
        readiness,
        DependencyReadiness::Blocked(vec![
            DependencyBlocker::TaskControlCheckpointMissing,
            DependencyBlocker::WorkerObservationCheckpointMissing,
        ])
    );
    assert_eq!(
        DependencyBlocker::TaskControlCheckpointMissing.code(),
        "SUPERVISOR_TASK_CONTROL_CHECKPOINT_MISSING"
    );
    assert_eq!(
        DependencyBlocker::WorkerObservationCheckpointMissing.code(),
        "SUPERVISOR_WORKER_OBSERVATION_CHECKPOINT_MISSING"
    );
}

#[test]
fn exact_stable_checkpoints_are_ready() {
    let readiness = dependency_readiness(&DependencySnapshot {
        task_control: checkpoint(
            TASK038_THREAD,
            CheckpointState::Stable,
            Some("feature/task-038-stable"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            Some("lattice-task-control/1.0"),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        ),
        worker_observation: checkpoint(
            TASK039_THREAD,
            CheckpointState::Stable,
            Some("feature/task-039-stable"),
            Some("89abcdef0123456789abcdef0123456789abcdef"),
            Some("lattice-worker-observation/1.0"),
            Some("89abcdef0123456789abcdef0123456789abcdef0123456789abcdef01234567"),
        ),
    });

    assert_eq!(readiness, DependencyReadiness::Ready);
}

#[test]
fn malformed_stable_evidence_is_not_ready() {
    let readiness = dependency_readiness(&DependencySnapshot {
        task_control: checkpoint(
            TASK038_THREAD,
            CheckpointState::Stable,
            Some("feature/task-038-stable"),
            Some("not-a-commit"),
            Some("lattice-task-control/1.0"),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        ),
        worker_observation: checkpoint(
            TASK039_THREAD,
            CheckpointState::Stable,
            Some("feature/task-039-stable"),
            Some("89abcdef0123456789abcdef0123456789abcdef"),
            Some("lattice-worker-observation/1.0"),
            Some("89abcdef0123456789abcdef0123456789abcdef0123456789abcdef01234567"),
        ),
    });

    assert_eq!(
        readiness,
        DependencyReadiness::Blocked(vec![DependencyBlocker::TaskControlCheckpointInvalid])
    );
}

#[test]
fn another_valid_thread_cannot_substitute_for_the_owning_window() {
    let readiness = dependency_readiness(&DependencySnapshot {
        task_control: checkpoint(
            "11111111-1111-1111-1111-111111111111",
            CheckpointState::Stable,
            Some("feature/task-038-stable"),
            Some("0123456789abcdef0123456789abcdef01234567"),
            Some("lattice-task-control/1.0"),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"),
        ),
        worker_observation: checkpoint(
            TASK039_THREAD,
            CheckpointState::Stable,
            Some("feature/task-039-stable"),
            Some("89abcdef0123456789abcdef0123456789abcdef"),
            Some("lattice-worker-observation/1.0"),
            Some("89abcdef0123456789abcdef0123456789abcdef0123456789abcdef01234567"),
        ),
    });

    assert_eq!(
        readiness,
        DependencyReadiness::Blocked(vec![DependencyBlocker::TaskControlCheckpointInvalid])
    );
}

#[test]
fn in_progress_checkpoint_still_requires_the_exact_owner_window() {
    let readiness = dependency_readiness(&DependencySnapshot {
        task_control: checkpoint(
            "11111111-1111-1111-1111-111111111111",
            CheckpointState::InProgress,
            Some("feature/task-038-chatgpt-mcp"),
            None,
            None,
            None,
        ),
        worker_observation: checkpoint(
            TASK039_THREAD,
            CheckpointState::InProgress,
            None,
            None,
            None,
            None,
        ),
    });

    assert_eq!(
        readiness,
        DependencyReadiness::Blocked(vec![
            DependencyBlocker::TaskControlCheckpointInvalid,
            DependencyBlocker::WorkerObservationCheckpointMissing,
        ])
    );
}
