//! Pure lifecycle gate for a future evidence-bound autonomy runner.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyLifecycleState {
    Idle,
    Claimed,
    HeartbeatDue,
    YieldToForeground,
    Interrupting,
    OrphanSuspected,
    ReconcileRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutonomyLifecycleEvent {
    ClaimVerified,
    HeartbeatDue,
    HeartbeatVerified,
    ForegroundTaskObserved,
    InterruptRequested,
    InterruptOutcomeUnknown,
    HolderDeathVerified,
    NewEvidenceSnapshot,
    ReconciliationVerified,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleTransition {
    Advanced(AutonomyLifecycleState),
    Rejected,
}

#[must_use]
pub const fn transition(
    state: AutonomyLifecycleState,
    event: AutonomyLifecycleEvent,
) -> LifecycleTransition {
    let next = match (state, event) {
        (AutonomyLifecycleState::Idle, AutonomyLifecycleEvent::ClaimVerified)
        | (AutonomyLifecycleState::HeartbeatDue, AutonomyLifecycleEvent::HeartbeatVerified) => {
            AutonomyLifecycleState::Claimed
        }
        (AutonomyLifecycleState::Claimed, AutonomyLifecycleEvent::HeartbeatDue) => {
            AutonomyLifecycleState::HeartbeatDue
        }
        (
            AutonomyLifecycleState::Claimed | AutonomyLifecycleState::HeartbeatDue,
            AutonomyLifecycleEvent::ForegroundTaskObserved,
        ) => AutonomyLifecycleState::YieldToForeground,
        (AutonomyLifecycleState::Claimed, AutonomyLifecycleEvent::InterruptRequested) => {
            AutonomyLifecycleState::Interrupting
        }
        (AutonomyLifecycleState::HeartbeatDue, AutonomyLifecycleEvent::HolderDeathVerified) => {
            AutonomyLifecycleState::OrphanSuspected
        }
        (AutonomyLifecycleState::Interrupting, AutonomyLifecycleEvent::InterruptOutcomeUnknown)
        | (AutonomyLifecycleState::OrphanSuspected, AutonomyLifecycleEvent::NewEvidenceSnapshot) => {
            AutonomyLifecycleState::ReconcileRequired
        }
        (
            AutonomyLifecycleState::ReconcileRequired,
            AutonomyLifecycleEvent::ReconciliationVerified,
        )
        | (
            AutonomyLifecycleState::YieldToForeground,
            AutonomyLifecycleEvent::NewEvidenceSnapshot,
        ) => AutonomyLifecycleState::Idle,
        _ => return LifecycleTransition::Rejected,
    };
    LifecycleTransition::Advanced(next)
}
