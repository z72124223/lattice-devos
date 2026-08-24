use lattice_orchestrator::{
    DurableHandoffReceipt, HandoffFieldStatus, KeepWindowOpenReason, WindowClosureDecision,
    WindowKind, classify_window_closure,
};
use lattice_task_domain::TaskState;

const COMPLETE_HANDOFF: DurableHandoffReceipt = DurableHandoffReceipt {
    scope: HandoffFieldStatus::Recorded,
    terminal_status: HandoffFieldStatus::Recorded,
    verified_work: HandoffFieldStatus::Recorded,
    remaining_risks: HandoffFieldStatus::Recorded,
    next_step: HandoffFieldStatus::Recorded,
    related_change_when_present: HandoffFieldStatus::Recorded,
};

#[test]
fn completed_and_genuinely_blocked_execution_windows_need_a_complete_handoff() {
    for state in [TaskState::Completed, TaskState::Blocked] {
        assert_eq!(
            classify_window_closure(WindowKind::Execution, state, Some(COMPLETE_HANDOFF)),
            WindowClosureDecision::ReadyForCoordinatorArchive
        );
    }
}

#[test]
fn missing_or_incomplete_handoff_keeps_terminal_execution_window_open() {
    assert_eq!(
        classify_window_closure(WindowKind::Execution, TaskState::Completed, None),
        WindowClosureDecision::KeepOpen {
            reason: KeepWindowOpenReason::MissingDurableHandoff
        }
    );
    let incomplete = DurableHandoffReceipt {
        next_step: HandoffFieldStatus::Missing,
        ..COMPLETE_HANDOFF
    };
    assert_eq!(
        classify_window_closure(WindowKind::Execution, TaskState::Blocked, Some(incomplete)),
        WindowClosureDecision::KeepOpen {
            reason: KeepWindowOpenReason::MissingDurableHandoff
        }
    );
}

#[test]
fn in_progress_failed_cancelled_and_non_execution_windows_are_not_archived() {
    for state in [
        TaskState::Draft,
        TaskState::Executing,
        TaskState::Failed,
        TaskState::Cancelled,
    ] {
        assert_eq!(
            classify_window_closure(WindowKind::Execution, state, Some(COMPLETE_HANDOFF)),
            WindowClosureDecision::KeepOpen {
                reason: KeepWindowOpenReason::NotTerminal
            }
        );
    }
    for kind in [
        WindowKind::Planning,
        WindowKind::Coordination,
        WindowKind::Conversation,
    ] {
        assert_eq!(
            classify_window_closure(kind, TaskState::Completed, Some(COMPLETE_HANDOFF)),
            WindowClosureDecision::KeepOpen {
                reason: KeepWindowOpenReason::NotAnExecutionWindow
            }
        );
    }
}
