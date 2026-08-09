use lattice_passive_supervisor_contract::{
    CheckpointState, DependencyBlocker, DependencyReadiness, DependencySnapshot,
    UpstreamCheckpoint, dependency_readiness,
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
        state: CheckpointState::Stable,
        branch: Some("feature/task-048-worker-observation-contract"),
        commit: Some("180a269c8af825a3989b2e26875cfa128389ba34"),
        contract_version: Some("lattice-contracts@1.12/worker-observation@1"),
        // SHA-256 of the canonical verification record documented in TASK-049.
        verification_digest: Some(
            "41dec097bc564de5cb6ed74db437876cf7b53c788129059b0a6bb5ec8b7e0d52",
        ),
    },
};

#[test]
fn stable_worker_checkpoint_removes_only_its_blocker() {
    assert_eq!(
        dependency_readiness(&CURRENT_DEPENDENCIES),
        DependencyReadiness::Blocked(vec![DependencyBlocker::TaskControlCheckpointMissing])
    );
}

#[test]
fn production_integration_requires_both_stable_upstream_contracts() {
    let readiness = dependency_readiness(&CURRENT_DEPENDENCIES);
    assert_eq!(
        readiness,
        DependencyReadiness::Ready,
        "READY=false BLOCKERS={readiness:?}"
    );
}
