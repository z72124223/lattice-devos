use lattice_contracts::{ProjectSnapshotId, TaskId};

use crate::TaskDomainError;

macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident {
            $($variant:ident => $wire:literal),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Parses the stable wire value.
            ///
            /// # Errors
            ///
            /// Returns `INVALID_TASK_SPEC` for an unknown value.
            pub fn parse(value: &str) -> Result<Self, TaskDomainError> {
                match value {
                    $($wire => Ok(Self::$variant),)+
                    _ => Err(TaskDomainError::InvalidTaskSpec {
                        field: stringify!($name),
                        reason: "unknown enum value",
                    }),
                }
            }

            /// Returns the stable wire value.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire),+
                }
            }
        }
    };
}

string_enum! {
    /// Task risk class.
    pub enum RiskClass {
        R0 => "R0",
        R1 => "R1",
        R2 => "R2",
        R3 => "R3",
    }
}

string_enum! {
    /// Product-path mutation kind requested by a task.
    pub enum ScopeOperation {
        Create => "create",
        Modify => "modify",
        Delete => "delete",
        Rename => "rename",
        TypeChange => "typechange",
    }
}

string_enum! {
    /// Acceptance evidence class.
    pub enum EvidenceType {
        Test => "test",
        Command => "command",
        Artifact => "artifact",
        Manual => "manual",
    }
}

string_enum! {
    /// Required verification gate.
    pub enum RequiredCheck {
        Build => "build",
        Test => "test",
        Scope => "scope",
        Security => "security",
        Architecture => "architecture",
        Lint => "lint",
        Typecheck => "typecheck",
    }
}

string_enum! {
    /// Versioned capability requested by a task.
    pub enum Capability {
        ReadRepository => "READ_REPOSITORY",
        MapCode => "MAP_CODE",
        PlanTask => "PLAN_TASK",
        WriteProductCode => "WRITE_PRODUCT_CODE",
        RunTests => "RUN_TESTS",
        GitWorktree => "GIT_WORKTREE",
        GitIntegrate => "GIT_INTEGRATE",
        ReadReview => "READ_REVIEW",
        StopRuntime => "STOP_RUNTIME",
        UseCodex => "USE_CODEX",
        UseGraphify => "USE_GRAPHIFY",
        UseHermes => "USE_HERMES",
        ReadCodebaseMemory => "READ_CODEBASE_MEMORY",
        ProposeMemory => "PROPOSE_MEMORY",
        ProposeUpgrade => "PROPOSE_UPGRADE",
    }
}

string_enum! {
    /// Requested runtime profile; availability remains a Policy/capability fact.
    pub enum RuntimeProfile {
        Fake => "fake",
        Codex => "codex",
    }
}

string_enum! {
    /// Network envelope requested by a task.
    pub enum NetworkPolicy {
        Deny => "deny",
        LoopbackOnly => "loopback_only",
        Allowlisted => "allowlisted",
    }
}

string_enum! {
    /// Deployment envelope requested by a task.
    pub enum DeploymentPolicy {
        Deny => "deny",
        PrepareOnly => "prepare_only",
        Authorized => "authorized",
    }
}

string_enum! {
    /// Evidence class required before a gated transition.
    pub enum ApprovalRequirement {
        NotRequired => "not_required",
        Policy => "policy",
        ResponsibleUser => "responsible_user",
        ProtectedGuardian => "protected_guardian",
    }
}

/// One immutable acceptance criterion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptanceCriterion {
    /// Stable criterion identifier.
    pub id: String,
    /// Required behavior.
    pub description: String,
    /// Evidence class.
    pub evidence_type: EvidenceType,
    /// Exact expected result.
    pub expected_result: String,
}

/// One capability plus its positive contract version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityRequest {
    /// Capability identifier.
    pub capability: Capability,
    /// Canonical positive integer string.
    pub contract_version: String,
}

/// Bounded task budget represented without raw JSON numbers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskBudget {
    /// Canonical three-letter uppercase ASCII accounting currency.
    pub accounting_currency: String,
    /// Canonical positive integer, currently bounded to 1 through 4.
    pub max_agents: String,
    /// Canonical positive seconds.
    pub max_duration_seconds: String,
    /// Canonical positive attempt count.
    pub max_attempts: String,
    /// Canonical non-negative model-call count.
    pub max_model_calls: String,
    /// Canonical non-negative decimal cost.
    pub max_external_cost: String,
}

/// Repository-relative product scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskScope {
    /// Set-like allowed path patterns.
    pub allowed_paths: Vec<String>,
    /// Set-like forbidden path patterns.
    pub forbidden_paths: Vec<String>,
    /// Set-like product mutation operations.
    pub allowed_operations: Vec<ScopeOperation>,
}

/// Required evidence for task, merge, and protected release gates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApprovalRequirements {
    /// Execution transition requirement.
    pub execution: ApprovalRequirement,
    /// Merge transition requirement.
    pub merge: ApprovalRequirement,
    /// Protected release transition requirement.
    pub protected_release: ApprovalRequirement,
}

/// Caller-supplied Task Spec V2.1 fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSpecInput {
    /// Must be `2.1`.
    pub schema_version: String,
    /// Immutable task identity.
    pub task_id: TaskId,
    /// Canonical positive integer string.
    pub revision: String,
    /// UTC RFC 3339 creation time.
    pub created_at: String,
    /// Authenticated creator identity reference.
    pub created_by: String,
    /// Registered project identity.
    pub project_id: String,
    /// Immutable registered project snapshot identity.
    pub project_snapshot_id: ProjectSnapshotId,
    /// Safe Git base ref.
    pub base_ref: String,
    /// Git SHA-1 or SHA-256 object identity.
    pub base_commit_id: String,
    /// Desired outcome.
    pub goal: String,
    /// Explicit exclusions.
    pub non_goals: Vec<String>,
    /// Risk class.
    pub risk_class: RiskClass,
    /// Set-like task dependencies.
    pub depends_on: Vec<TaskId>,
    /// Product path and operation scope.
    pub scope: TaskScope,
    /// Acceptance criteria.
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    /// Ordered verification commands.
    pub verification_commands: Vec<String>,
    /// Set-like required checks.
    pub required_checks: Vec<RequiredCheck>,
    /// Set-like versioned capability requests.
    pub requested_capabilities: Vec<CapabilityRequest>,
    /// Bounded numeric-string budget.
    pub budget: TaskBudget,
    /// Requested runtime profile.
    pub runtime_profile: RuntimeProfile,
    /// Requested network envelope.
    pub network_policy: NetworkPolicy,
    /// Requested deployment envelope.
    pub deployment_policy: DeploymentPolicy,
    /// Separate approval requirements.
    pub approval_requirements: ApprovalRequirements,
}
