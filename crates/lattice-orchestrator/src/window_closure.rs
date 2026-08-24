//! Pure execution-window closure eligibility.
//!
//! This module never persists a handoff and cannot archive a Codex window.
//! It only makes the fail-closed decision that a coordinator may invoke its
//! platform archive capability after durable handoff persistence succeeds.

use lattice_task_domain::TaskState;

/// The bounded kinds of window understood by the closure rule.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowKind {
    Execution,
    Planning,
    Coordination,
    Conversation,
}

/// The durable handoff fields required before an execution window may close.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffFieldStatus {
    Recorded,
    Missing,
}

/// The durable handoff fields required before an execution window may close.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DurableHandoffReceipt {
    pub scope: HandoffFieldStatus,
    pub terminal_status: HandoffFieldStatus,
    pub verified_work: HandoffFieldStatus,
    pub remaining_risks: HandoffFieldStatus,
    pub next_step: HandoffFieldStatus,
    /// The optional local commit or pull-request reference was recorded when one exists.
    pub related_change_when_present: HandoffFieldStatus,
}

impl DurableHandoffReceipt {
    /// Returns whether this receipt contains every required continuation field.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(
            self,
            Self {
                scope: HandoffFieldStatus::Recorded,
                terminal_status: HandoffFieldStatus::Recorded,
                verified_work: HandoffFieldStatus::Recorded,
                remaining_risks: HandoffFieldStatus::Recorded,
                next_step: HandoffFieldStatus::Recorded,
                related_change_when_present: HandoffFieldStatus::Recorded,
            }
        )
    }
}

/// Why a window remains open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeepWindowOpenReason {
    NotAnExecutionWindow,
    NotTerminal,
    MissingDurableHandoff,
}

/// A platform-neutral closure decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowClosureDecision {
    /// The coordinator may archive only after it has persisted the represented receipt.
    ReadyForCoordinatorArchive,
    KeepOpen {
        reason: KeepWindowOpenReason,
    },
}

/// Decides whether a window is eligible for coordinator-driven archival.
///
/// Only completed or genuinely blocked execution work with a complete durable
/// handoff is eligible. This deliberately excludes failed, cancelled,
/// in-progress, planning, coordination, and conversation windows.
#[must_use]
pub const fn classify_window_closure(
    kind: WindowKind,
    state: TaskState,
    handoff: Option<DurableHandoffReceipt>,
) -> WindowClosureDecision {
    if !matches!(kind, WindowKind::Execution) {
        return WindowClosureDecision::KeepOpen {
            reason: KeepWindowOpenReason::NotAnExecutionWindow,
        };
    }
    if !matches!(state, TaskState::Completed | TaskState::Blocked) {
        return WindowClosureDecision::KeepOpen {
            reason: KeepWindowOpenReason::NotTerminal,
        };
    }
    match handoff {
        Some(receipt) if receipt.is_complete() => WindowClosureDecision::ReadyForCoordinatorArchive,
        Some(_) | None => WindowClosureDecision::KeepOpen {
            reason: KeepWindowOpenReason::MissingDurableHandoff,
        },
    }
}
