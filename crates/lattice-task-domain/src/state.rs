use crate::TaskDomainError;

/// Stable task lifecycle states retained from the V1 characterization.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TaskState {
    /// Draft specification.
    Draft,
    /// Execution evidence is not yet satisfied.
    AwaitingExecutionApproval,
    /// Workspace and authority preparation.
    Preparing,
    /// Product-code execution.
    Executing,
    /// Focused and full verification.
    Verifying,
    /// Independent review.
    Reviewing,
    /// Merge evidence is not yet satisfied.
    AwaitingMergeApproval,
    /// Authorized integration.
    Merging,
    /// Terminal success.
    Completed,
    /// Terminal approval rejection.
    Rejected,
    /// Terminal external or policy blocker.
    Blocked,
    /// Terminal execution failure.
    Failed,
    /// Stop and reconciliation in progress.
    Stopping,
    /// Terminal cancellation.
    Cancelled,
}

impl TaskState {
    /// Complete stable state set.
    pub const ALL: [Self; 14] = [
        Self::Draft,
        Self::AwaitingExecutionApproval,
        Self::Preparing,
        Self::Executing,
        Self::Verifying,
        Self::Reviewing,
        Self::AwaitingMergeApproval,
        Self::Merging,
        Self::Completed,
        Self::Rejected,
        Self::Blocked,
        Self::Failed,
        Self::Stopping,
        Self::Cancelled,
    ];

    /// Parses a stable wire state.
    ///
    /// # Errors
    ///
    /// Returns `UNKNOWN_TASK_STATE` for every unknown value.
    pub fn parse(value: &str) -> Result<Self, TaskDomainError> {
        match value {
            "DRAFT" => Ok(Self::Draft),
            "AWAITING_EXECUTION_APPROVAL" => Ok(Self::AwaitingExecutionApproval),
            "PREPARING" => Ok(Self::Preparing),
            "EXECUTING" => Ok(Self::Executing),
            "VERIFYING" => Ok(Self::Verifying),
            "REVIEWING" => Ok(Self::Reviewing),
            "AWAITING_MERGE_APPROVAL" => Ok(Self::AwaitingMergeApproval),
            "MERGING" => Ok(Self::Merging),
            "COMPLETED" => Ok(Self::Completed),
            "REJECTED" => Ok(Self::Rejected),
            "BLOCKED" => Ok(Self::Blocked),
            "FAILED" => Ok(Self::Failed),
            "STOPPING" => Ok(Self::Stopping),
            "CANCELLED" => Ok(Self::Cancelled),
            _ => Err(TaskDomainError::UnknownTaskState {
                state: value.to_owned(),
            }),
        }
    }

    /// Returns the stable wire state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::AwaitingExecutionApproval => "AWAITING_EXECUTION_APPROVAL",
            Self::Preparing => "PREPARING",
            Self::Executing => "EXECUTING",
            Self::Verifying => "VERIFYING",
            Self::Reviewing => "REVIEWING",
            Self::AwaitingMergeApproval => "AWAITING_MERGE_APPROVAL",
            Self::Merging => "MERGING",
            Self::Completed => "COMPLETED",
            Self::Rejected => "REJECTED",
            Self::Blocked => "BLOCKED",
            Self::Failed => "FAILED",
            Self::Stopping => "STOPPING",
            Self::Cancelled => "CANCELLED",
        }
    }
}

/// Returns whether the frozen transition graph contains an edge.
#[must_use]
pub fn is_transition_allowed(from: TaskState, to: TaskState) -> bool {
    use TaskState::{
        AwaitingExecutionApproval, AwaitingMergeApproval, Blocked, Cancelled, Completed, Draft,
        Executing, Failed, Merging, Preparing, Rejected, Reviewing, Stopping, Verifying,
    };

    match from {
        Draft => matches!(to, AwaitingExecutionApproval | Cancelled),
        AwaitingExecutionApproval => matches!(to, Preparing | Rejected | Cancelled),
        Preparing => matches!(to, Executing | Blocked | Failed | Stopping),
        Executing => matches!(to, Verifying | Blocked | Failed | Stopping),
        Verifying => matches!(to, Reviewing | Blocked | Failed | Stopping),
        Reviewing => matches!(to, AwaitingMergeApproval | Blocked | Failed | Stopping),
        AwaitingMergeApproval => matches!(to, Merging | Rejected | Cancelled),
        Merging => matches!(to, Completed | Blocked | Failed | Stopping),
        Stopping => matches!(to, Cancelled | Failed),
        Completed | Rejected | Blocked | Failed | Cancelled => false,
    }
}

/// Applies one known legal transition.
///
/// # Errors
///
/// Returns `INVALID_STATE_TRANSITION` when no edge exists.
pub fn transition(from: TaskState, to: TaskState) -> Result<TaskState, TaskDomainError> {
    if is_transition_allowed(from, to) {
        Ok(to)
    } else {
        Err(TaskDomainError::InvalidStateTransition { from, to })
    }
}

/// Read-only V1 string compatibility for the frozen state graph.
pub mod v1_compat {
    use super::{TaskDomainError, TaskState};

    /// Returns false for unknown states and otherwise checks the V1 matrix.
    #[must_use]
    pub fn is_transition_allowed(from: &str, to: &str) -> bool {
        let Ok(from) = TaskState::parse(from) else {
            return false;
        };
        let Ok(to) = TaskState::parse(to) else {
            return false;
        };
        super::is_transition_allowed(from, to)
    }

    /// Parses and applies one V1 transition.
    ///
    /// # Errors
    ///
    /// Preserves `UNKNOWN_TASK_STATE` and `INVALID_STATE_TRANSITION`.
    pub fn transition(from: &str, to: &str) -> Result<TaskState, TaskDomainError> {
        super::transition(TaskState::parse(from)?, TaskState::parse(to)?)
    }
}
