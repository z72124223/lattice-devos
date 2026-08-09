use lattice_passive_supervisor_contract::{
    CheckpointState, DependencyReadiness, DependencySnapshot, UpstreamCheckpoint,
    dependency_readiness,
};

/// This fixture records live coordination truth observed on 2026-08-10.
/// Replace an entry only with the owning window's exact stable evidence.
const CURRENT_DEPENDENCIES: DependencySnapshot<'static> = DependencySnapshot {
    task_control: UpstreamCheckpoint {
        source_thread_id: "019fe6e2-f8b9-7bb1-9d31-1e8992207d99",
        state: CheckpointState::InProgress,
        branch: Some("feature/task-038-chatgpt-mcp"),
        commit: None,
        contract_version: None,
        verification_digest: None,
    },
    worker_observation: UpstreamCheckpoint {
        source_thread_id: "019fe759-d4d6-7950-bf10-fbda7487fa9b",
        state: CheckpointState::InProgress,
        branch: Some("feature/task-048-worker-observation-contract"),
        commit: None,
        contract_version: None,
        verification_digest: None,
    },
};

#[test]
fn production_integration_requires_both_stable_upstream_contracts() {
    let readiness = dependency_readiness(&CURRENT_DEPENDENCIES);
    assert_eq!(
        readiness,
        DependencyReadiness::Ready,
        "READY=false BLOCKERS={readiness:?}"
    );
}
