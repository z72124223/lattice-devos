use std::error::Error;
use std::fmt;

use lattice_cjson::CanonicalError;

use crate::TaskState;

/// Pure Task Domain validation or transition failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskDomainError {
    /// A Task Spec schema version is not supported.
    UnsupportedTaskSpecVersion {
        /// Supplied version.
        found: String,
    },
    /// A field or typed enum is invalid.
    InvalidTaskSpec {
        /// Stable field name.
        field: &'static str,
        /// Concise stable diagnostic.
        reason: &'static str,
    },
    /// A task identifier does not match the V2 contract.
    InvalidTaskId {
        /// Rejected identifier.
        value: String,
    },
    /// A project identifier does not match the V2 contract.
    InvalidProjectId {
        /// Rejected identifier.
        value: String,
    },
    /// A Git object identifier is not lowercase-normalizable SHA-1/SHA-256 hex.
    InvalidGitObjectId {
        /// Rejected identifier.
        value: String,
    },
    /// A timestamp is not canonicalizable UTC RFC 3339.
    InvalidUtcTimestamp {
        /// Rejected timestamp.
        value: String,
    },
    /// An integer string is not canonical or outside its schema range.
    InvalidCanonicalInteger {
        /// Stable field name.
        field: &'static str,
        /// Rejected integer.
        value: String,
    },
    /// A decimal string is not canonical and non-negative.
    InvalidCanonicalDecimal {
        /// Stable field name.
        field: &'static str,
        /// Rejected decimal.
        value: String,
    },
    /// A repository-relative scope path is unsafe.
    InvalidScopePath {
        /// Stable field name.
        field: &'static str,
        /// Rejected path.
        path: String,
    },
    /// A set-like Task Spec field contains a duplicate after normalization.
    DuplicateTaskFieldValue {
        /// Stable field name.
        field: &'static str,
        /// Duplicate normalized value.
        value: String,
    },
    /// A textual task state is unknown.
    UnknownTaskState {
        /// Rejected state.
        state: String,
    },
    /// Two known states do not have a legal edge.
    InvalidStateTransition {
        /// Source state.
        from: TaskState,
        /// Target state.
        to: TaskState,
    },
    /// A graph edge references a task absent from the graph.
    UnknownTaskDependency {
        /// Owning task.
        task_id: String,
        /// Missing dependency.
        dependency: String,
    },
    /// A task dependency graph contains a cycle.
    TaskDependencyCycle {
        /// Stable closed cycle path; first equals last.
        cycle: Vec<String>,
    },
    /// The canonical-byte mechanism rejected the Task Spec subject.
    Canonical(CanonicalError),
}

impl TaskDomainError {
    /// Returns a stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedTaskSpecVersion { .. } => "UNSUPPORTED_TASK_SPEC_VERSION",
            Self::InvalidTaskSpec { .. } => "INVALID_TASK_SPEC",
            Self::InvalidTaskId { .. } => "INVALID_TASK_ID",
            Self::InvalidProjectId { .. } => "INVALID_PROJECT_ID",
            Self::InvalidGitObjectId { .. } => "INVALID_GIT_OBJECT_ID",
            Self::InvalidUtcTimestamp { .. } => "INVALID_UTC_TIMESTAMP",
            Self::InvalidCanonicalInteger { .. } => "INVALID_CANONICAL_INTEGER",
            Self::InvalidCanonicalDecimal { .. } => "INVALID_CANONICAL_DECIMAL",
            Self::InvalidScopePath { .. } => "INVALID_SCOPE_PATH",
            Self::DuplicateTaskFieldValue { .. } => "DUPLICATE_TASK_FIELD_VALUE",
            Self::UnknownTaskState { .. } => "UNKNOWN_TASK_STATE",
            Self::InvalidStateTransition { .. } => "INVALID_STATE_TRANSITION",
            Self::UnknownTaskDependency { .. } => "UNKNOWN_TASK_DEPENDENCY",
            Self::TaskDependencyCycle { .. } => "TASK_DEPENDENCY_CYCLE",
            Self::Canonical(error) => error.code(),
        }
    }

    /// Returns cycle evidence when this is a dependency-cycle error.
    #[must_use]
    pub fn cycle(&self) -> Option<&[String]> {
        match self {
            Self::TaskDependencyCycle { cycle } => Some(cycle),
            _ => None,
        }
    }
}

impl fmt::Display for TaskDomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTaskSpecVersion { found } => {
                write!(formatter, "unsupported Task Spec version: {found}")
            }
            Self::InvalidTaskSpec { field, reason } => {
                write!(formatter, "invalid Task Spec field {field}: {reason}")
            }
            Self::InvalidTaskId { value } => write!(formatter, "invalid task_id: {value}"),
            Self::InvalidProjectId { value } => {
                write!(formatter, "invalid project_id: {value}")
            }
            Self::InvalidGitObjectId { value } => {
                write!(formatter, "invalid base_commit_id: {value}")
            }
            Self::InvalidUtcTimestamp { value } => {
                write!(formatter, "invalid UTC RFC 3339 timestamp: {value}")
            }
            Self::InvalidCanonicalInteger { field, value } => {
                write!(formatter, "{field} is not a canonical integer: {value}")
            }
            Self::InvalidCanonicalDecimal { field, value } => {
                write!(formatter, "{field} is not a canonical decimal: {value}")
            }
            Self::InvalidScopePath { field, path } => {
                write!(formatter, "{field} is not a safe repository path: {path}")
            }
            Self::DuplicateTaskFieldValue { field, value } => {
                write!(formatter, "{field} contains duplicate value: {value}")
            }
            Self::UnknownTaskState { state } => write!(formatter, "unknown task state: {state}"),
            Self::InvalidStateTransition { from, to } => {
                write!(
                    formatter,
                    "task state cannot transition from {} to {}",
                    from.as_str(),
                    to.as_str()
                )
            }
            Self::UnknownTaskDependency {
                task_id,
                dependency,
            } => write!(
                formatter,
                "task {task_id} references unknown dependency {dependency}"
            ),
            Self::TaskDependencyCycle { cycle } => {
                write!(formatter, "task dependency cycle: {}", cycle.join(" -> "))
            }
            Self::Canonical(error) => error.fmt(formatter),
        }
    }
}

impl Error for TaskDomainError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Canonical(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CanonicalError> for TaskDomainError {
    fn from(value: CanonicalError) -> Self {
        Self::Canonical(value)
    }
}
