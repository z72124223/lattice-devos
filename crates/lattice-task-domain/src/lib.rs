//! Pure Task Spec V2, state, and dependency-graph semantics.

mod error;
mod graph;
mod spec;
mod state;
mod types;
mod validation;

pub use error::TaskDomainError;
pub use graph::validate_task_graph;
pub use spec::{TASK_SPEC_SCHEMA_ID, TASK_SPEC_SCHEMA_VERSION, TaskSpec};
pub use state::{TaskState, is_transition_allowed, transition, v1_compat};
pub use types::{
    AcceptanceCriterion, ApprovalRequirement, ApprovalRequirements, Capability, CapabilityRequest,
    DeploymentPolicy, EvidenceType, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    ScopeOperation, TaskBudget, TaskScope, TaskSpecInput,
};

/// Shared upper bound for canonical non-negative decimal strings.
///
/// Policy imports this contract directly so a constructed Task Spec cannot
/// exceed the exact parser bound used for checked resource arithmetic.
pub const MAX_CANONICAL_DECIMAL_BYTES: usize = 256;
/// Maximum digits before the decimal point in a canonical decimal.
pub const MAX_CANONICAL_DECIMAL_INTEGER_DIGITS: usize = 127;
/// Maximum digits after the decimal point in a canonical decimal.
pub const MAX_CANONICAL_DECIMAL_SCALE: usize = 128;
