use lattice_orchestrator::{
    AutonomyLifecycleEvent as E, AutonomyLifecycleState as S, LifecycleTransition as T, transition,
};

#[test]
fn foreground_yield_and_reconciliation_require_new_evidence() {
    assert_eq!(
        transition(S::Claimed, E::ForegroundTaskObserved),
        T::Advanced(S::YieldToForeground)
    );
    assert_eq!(
        transition(S::YieldToForeground, E::ClaimVerified),
        T::Rejected
    );
    assert_eq!(
        transition(S::YieldToForeground, E::NewEvidenceSnapshot),
        T::Advanced(S::Idle)
    );
    assert_eq!(
        transition(S::OrphanSuspected, E::ClaimVerified),
        T::Rejected
    );
    assert_eq!(
        transition(S::OrphanSuspected, E::NewEvidenceSnapshot),
        T::Advanced(S::ReconcileRequired)
    );
}

#[test]
fn heartbeat_and_interrupt_fail_closed() {
    assert_eq!(
        transition(S::Claimed, E::HeartbeatDue),
        T::Advanced(S::HeartbeatDue)
    );
    assert_eq!(
        transition(S::HeartbeatDue, E::HeartbeatVerified),
        T::Advanced(S::Claimed)
    );
    assert_eq!(
        transition(S::HeartbeatDue, E::HolderDeathVerified),
        T::Advanced(S::OrphanSuspected)
    );
    assert_eq!(transition(S::HeartbeatDue, E::ClaimVerified), T::Rejected);
    assert_eq!(
        transition(S::Claimed, E::InterruptRequested),
        T::Advanced(S::Interrupting)
    );
    assert_eq!(
        transition(S::Interrupting, E::InterruptOutcomeUnknown),
        T::Advanced(S::ReconcileRequired)
    );
    assert_eq!(
        transition(S::ReconcileRequired, E::ClaimVerified),
        T::Rejected
    );
}
