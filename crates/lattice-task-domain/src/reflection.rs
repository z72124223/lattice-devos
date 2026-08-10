use crate::TaskDomainError;

/// Independent lifecycle of Reflection work and terminal Reflection evidence.
///
/// This graph describes legal semantic edges only. A repository port may
/// impose a narrower authority policy. In the first slice, only the
/// post-completion queue lane may claim, retry, degrade, or append candidates;
/// direct core-failure and fixed-output-rejection records are terminal,
/// read-only evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReflectionState {
    /// Durable work exists and is eligible for a bounded claim.
    Pending,
    /// The latest bounded Reflection attempt failed.
    Failed,
    /// A later bounded retry is authorized but has not been claimed.
    RetryPending,
    /// Core completion is usable without a successful Reflection result.
    Degraded,
}

impl ReflectionState {
    /// Complete stable first-slice state set.
    pub const ALL: [Self; 4] = [
        Self::Pending,
        Self::Failed,
        Self::RetryPending,
        Self::Degraded,
    ];

    /// Parses one exact wire state.
    ///
    /// # Errors
    ///
    /// Rejects every value outside the closed first-slice state set.
    pub fn parse(value: &str) -> Result<Self, TaskDomainError> {
        match value {
            "REFLECTION_PENDING" => Ok(Self::Pending),
            "REFLECTION_FAILED" => Ok(Self::Failed),
            "RETRY_PENDING" => Ok(Self::RetryPending),
            "DEGRADED" => Ok(Self::Degraded),
            _ => Err(TaskDomainError::UnknownReflectionState {
                state: value.to_owned(),
            }),
        }
    }

    /// Returns the exact wire state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "REFLECTION_PENDING",
            Self::Failed => "REFLECTION_FAILED",
            Self::RetryPending => "RETRY_PENDING",
            Self::Degraded => "DEGRADED",
        }
    }
}

/// Closed durable failure-evidence categories.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReflectionFailureKind {
    /// The controlled core task failed before completion.
    TaskFailure,
    /// A bounded output was rejected by a fixed verifier.
    OutputRejected,
    /// A bounded Hermes Reflection attempt failed after core completion.
    HermesFailure,
}

impl ReflectionFailureKind {
    /// Returns the exact wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TaskFailure => "TASK_FAILURE",
            Self::OutputRejected => "OUTPUT_REJECTED",
            Self::HermesFailure => "HERMES_FAILURE",
        }
    }
}

/// Closed non-authoritative values Hermes may append.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ReflectionCandidateKind {
    /// A bounded observation over authorized history.
    Observation,
    /// A bounded inference over authorized history.
    Inference,
    /// A non-authoritative root-cause candidate.
    RootCauseCandidate,
    /// A non-authoritative improvement candidate.
    ImprovementCandidate,
}

impl ReflectionCandidateKind {
    /// Returns the exact wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observation => "OBSERVATION",
            Self::Inference => "INFERENCE",
            Self::RootCauseCandidate => "ROOT_CAUSE_CANDIDATE",
            Self::ImprovementCandidate => "IMPROVEMENT_CANDIDATE",
        }
    }
}

/// Returns whether the independent Reflection graph contains an edge.
#[must_use]
pub const fn is_reflection_transition_allowed(from: ReflectionState, to: ReflectionState) -> bool {
    use ReflectionState::{Degraded, Failed, Pending, RetryPending};

    match from {
        Pending => matches!(to, Failed | Degraded),
        Failed => matches!(to, RetryPending | Degraded),
        RetryPending => matches!(to, Pending),
        Degraded => matches!(to, RetryPending | Failed),
    }
}

/// Applies one legal independent Reflection transition.
///
/// # Errors
///
/// Rejects every edge outside the closed first-slice graph.
pub fn reflection_transition(
    from: ReflectionState,
    to: ReflectionState,
) -> Result<ReflectionState, TaskDomainError> {
    if is_reflection_transition_allowed(from, to) {
        Ok(to)
    } else {
        Err(TaskDomainError::InvalidReflectionTransition { from, to })
    }
}
