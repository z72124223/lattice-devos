use std::collections::BTreeSet;

use lattice_cjson::{CanonicalValue, HashDomain, Sha256Digest, canonical_sha256, normalize_nfc};
use lattice_contracts::ProjectSnapshotId;

use crate::validation::{
    canonical_decimal, canonical_unsigned, canonical_utc_timestamp, normalize_base_ref,
    normalize_git_object_id, normalize_project_id, normalize_scope_path, normalize_text,
    normalize_text_list, validate_task_id,
};
use crate::{
    AcceptanceCriterion, CapabilityRequest, RequiredCheck, ScopeOperation, TaskBudget,
    TaskDomainError, TaskScope, TaskSpecInput,
};

/// Task Spec V2.1 schema identifier.
pub const TASK_SPEC_SCHEMA_ID: &str = "lattice.task-spec";
/// Task Spec V2.1 schema version.
pub const TASK_SPEC_SCHEMA_VERSION: &str = "2.1";

/// Validated immutable Task Spec V2.1 plus its domain-separated digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSpec {
    fields: TaskSpecInput,
    spec_hash: Sha256Digest,
}

impl TaskSpec {
    /// Validates, normalizes, owns, and hashes a Task Spec V2.1.
    ///
    /// # Errors
    ///
    /// Returns a typed fail-closed validation or canonicalization error.
    pub fn new(input: TaskSpecInput) -> Result<Self, TaskDomainError> {
        let fields = normalize_input(input)?;
        let subject = canonical_subject(&fields);
        let domain = HashDomain::new(TASK_SPEC_SCHEMA_ID, TASK_SPEC_SCHEMA_VERSION)?;
        let spec_hash = canonical_sha256(&domain, &subject)?;
        Ok(Self { fields, spec_hash })
    }

    /// Returns all normalized immutable fields.
    #[must_use]
    pub const fn fields(&self) -> &TaskSpecInput {
        &self.fields
    }

    /// Returns the normalized creation timestamp.
    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.fields.created_at
    }

    /// Returns the normalized lowercase Git object ID.
    #[must_use]
    pub fn base_commit_id(&self) -> &str {
        &self.fields.base_commit_id
    }

    /// Returns the normalized goal.
    #[must_use]
    pub fn goal(&self) -> &str {
        &self.fields.goal
    }

    /// Returns the immutable Task Spec digest.
    #[must_use]
    pub const fn spec_hash(&self) -> &Sha256Digest {
        &self.spec_hash
    }
}

fn normalize_input(mut input: TaskSpecInput) -> Result<TaskSpecInput, TaskDomainError> {
    if input.schema_version != TASK_SPEC_SCHEMA_VERSION {
        return Err(TaskDomainError::UnsupportedTaskSpecVersion {
            found: input.schema_version,
        });
    }
    validate_task_id(input.task_id.as_str())?;
    canonical_unsigned(&input.revision, "revision", true)?;
    input.created_at = canonical_utc_timestamp(&input.created_at)?;
    input.created_by = normalize_text(&input.created_by, "created_by")?;
    input.project_id = normalize_project_id(&input.project_id)?;
    input.project_snapshot_id = normalize_snapshot_id(&input.project_snapshot_id)?;
    input.base_ref = normalize_base_ref(&input.base_ref)?;
    input.base_commit_id = normalize_git_object_id(&input.base_commit_id)?;
    input.goal = normalize_text(&input.goal, "goal")?;
    input.non_goals = normalize_text_list(input.non_goals, "non_goals", 1, false)?;
    input.depends_on = normalize_dependencies(input.depends_on, input.task_id.as_str())?;
    input.scope = normalize_scope(input.scope)?;
    input.acceptance_criteria = normalize_acceptance(input.acceptance_criteria)?;
    input.verification_commands = normalize_text_list(
        input.verification_commands,
        "verification_commands",
        1,
        false,
    )?;
    input.required_checks = normalize_checks(input.required_checks)?;
    input.requested_capabilities = normalize_capabilities(input.requested_capabilities)?;
    normalize_budget(&input.budget)?;
    Ok(input)
}

fn normalize_snapshot_id(value: &ProjectSnapshotId) -> Result<ProjectSnapshotId, TaskDomainError> {
    let normalized = normalize_text(value.as_str(), "project_snapshot_id")?;
    ProjectSnapshotId::new(normalized).map_err(|_| TaskDomainError::InvalidTaskSpec {
        field: "project_snapshot_id",
        reason: "invalid snapshot identity",
    })
}

fn normalize_dependencies(
    mut values: Vec<lattice_contracts::TaskId>,
    current_task: &str,
) -> Result<Vec<lattice_contracts::TaskId>, TaskDomainError> {
    let mut seen = BTreeSet::new();
    for value in &values {
        validate_task_id(value.as_str())?;
        if value.as_str() == current_task {
            return Err(TaskDomainError::TaskDependencyCycle {
                cycle: vec![current_task.to_owned(), current_task.to_owned()],
            });
        }
        if !seen.insert(value.as_str().to_owned()) {
            return Err(TaskDomainError::DuplicateTaskFieldValue {
                field: "depends_on",
                value: value.as_str().to_owned(),
            });
        }
    }
    values.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    Ok(values)
}

fn normalize_scope(mut scope: TaskScope) -> Result<TaskScope, TaskDomainError> {
    scope.allowed_paths = normalize_paths(scope.allowed_paths, "scope.allowed_paths", false)?;
    scope.forbidden_paths = normalize_paths(scope.forbidden_paths, "scope.forbidden_paths", true)?;
    if !scope.forbidden_paths.iter().any(|path| path == ".git/**") {
        return Err(TaskDomainError::InvalidTaskSpec {
            field: "scope.forbidden_paths",
            reason: "must explicitly forbid .git/**",
        });
    }
    scope.allowed_operations =
        normalize_operations(scope.allowed_operations, "scope.allowed_operations")?;
    Ok(scope)
}

fn normalize_paths(
    values: Vec<String>,
    field: &'static str,
    allow_git: bool,
) -> Result<Vec<String>, TaskDomainError> {
    if values.is_empty() {
        return Err(TaskDomainError::InvalidTaskSpec {
            field,
            reason: "must not be empty",
        });
    }
    let mut normalized = Vec::with_capacity(values.len());
    let mut seen = BTreeSet::new();
    for value in values {
        let value = normalize_scope_path(&value, field, allow_git)?;
        if !seen.insert(value.clone()) {
            return Err(TaskDomainError::DuplicateTaskFieldValue { field, value });
        }
        normalized.push(value);
    }
    normalized.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    Ok(normalized)
}

fn normalize_operations(
    mut values: Vec<ScopeOperation>,
    field: &'static str,
) -> Result<Vec<ScopeOperation>, TaskDomainError> {
    if values.is_empty() {
        return Err(TaskDomainError::InvalidTaskSpec {
            field,
            reason: "must not be empty",
        });
    }
    values.sort_by_key(|value| value.as_str());
    for pair in values.windows(2) {
        if pair[0] == pair[1] {
            return Err(TaskDomainError::DuplicateTaskFieldValue {
                field,
                value: pair[0].as_str().to_owned(),
            });
        }
    }
    Ok(values)
}

fn normalize_acceptance(
    mut values: Vec<AcceptanceCriterion>,
) -> Result<Vec<AcceptanceCriterion>, TaskDomainError> {
    if values.is_empty() {
        return Err(TaskDomainError::InvalidTaskSpec {
            field: "acceptance_criteria",
            reason: "must not be empty",
        });
    }
    let mut ids = BTreeSet::new();
    for value in &mut values {
        value.id = normalize_text(&value.id, "acceptance_criteria.id")?;
        value.description = normalize_text(&value.description, "acceptance_criteria.description")?;
        value.expected_result = normalize_text(
            &value.expected_result,
            "acceptance_criteria.expected_result",
        )?;
        if !ids.insert(value.id.clone()) {
            return Err(TaskDomainError::DuplicateTaskFieldValue {
                field: "acceptance_criteria.id",
                value: value.id.clone(),
            });
        }
    }
    Ok(values)
}

fn normalize_checks(mut values: Vec<RequiredCheck>) -> Result<Vec<RequiredCheck>, TaskDomainError> {
    if values.is_empty() {
        return Err(TaskDomainError::InvalidTaskSpec {
            field: "required_checks",
            reason: "must not be empty",
        });
    }
    values.sort_by_key(|value| value.as_str());
    for pair in values.windows(2) {
        if pair[0] == pair[1] {
            return Err(TaskDomainError::DuplicateTaskFieldValue {
                field: "required_checks",
                value: pair[0].as_str().to_owned(),
            });
        }
    }
    Ok(values)
}

fn normalize_capabilities(
    mut values: Vec<CapabilityRequest>,
) -> Result<Vec<CapabilityRequest>, TaskDomainError> {
    if values.is_empty() {
        return Err(TaskDomainError::InvalidTaskSpec {
            field: "requested_capabilities",
            reason: "must not be empty",
        });
    }
    let mut seen = BTreeSet::new();
    for value in &values {
        let parsed =
            canonical_unsigned(&value.contract_version, "capability.contract_version", true)?;
        if parsed > u64::from(u16::MAX) {
            return Err(TaskDomainError::InvalidCanonicalInteger {
                field: "capability.contract_version",
                value: value.contract_version.clone(),
            });
        }
        if parsed != 1 {
            return Err(TaskDomainError::InvalidTaskSpec {
                field: "capability.contract_version",
                reason: "unsupported capability contract version",
            });
        }
        if !seen.insert(value.capability) {
            return Err(TaskDomainError::DuplicateTaskFieldValue {
                field: "requested_capabilities",
                value: value.capability.as_str().to_owned(),
            });
        }
    }
    values.sort_by(|left, right| {
        left.capability
            .as_str()
            .cmp(right.capability.as_str())
            .then_with(|| left.contract_version.cmp(&right.contract_version))
    });
    Ok(values)
}

fn normalize_budget(value: &TaskBudget) -> Result<(), TaskDomainError> {
    if value.accounting_currency.len() != 3
        || !value
            .accounting_currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
    {
        return Err(TaskDomainError::InvalidTaskSpec {
            field: "budget.accounting_currency",
            reason: "must be exactly three uppercase ASCII letters",
        });
    }
    let agents = canonical_unsigned(&value.max_agents, "budget.max_agents", true)?;
    if agents > 4 {
        return Err(TaskDomainError::InvalidCanonicalInteger {
            field: "budget.max_agents",
            value: value.max_agents.clone(),
        });
    }
    canonical_unsigned(
        &value.max_duration_seconds,
        "budget.max_duration_seconds",
        true,
    )?;
    canonical_unsigned(&value.max_attempts, "budget.max_attempts", true)?;
    canonical_unsigned(&value.max_model_calls, "budget.max_model_calls", false)?;
    canonical_decimal(&value.max_external_cost, "budget.max_external_cost")
}

fn canonical_subject(fields: &TaskSpecInput) -> CanonicalValue {
    object(vec![
        ("schema_version", text(&fields.schema_version)),
        ("task_id", text(fields.task_id.as_str())),
        ("revision", text(&fields.revision)),
        ("created_at", text(&fields.created_at)),
        ("created_by", text(&fields.created_by)),
        ("project_id", text(&fields.project_id)),
        (
            "project_snapshot_id",
            text(fields.project_snapshot_id.as_str()),
        ),
        ("base_ref", text(&fields.base_ref)),
        ("base_commit_id", text(&fields.base_commit_id)),
        ("goal", text(&fields.goal)),
        ("non_goals", text_array(&fields.non_goals)),
        ("risk_class", text(fields.risk_class.as_str())),
        (
            "depends_on",
            CanonicalValue::Array(
                fields
                    .depends_on
                    .iter()
                    .map(|value| text(value.as_str()))
                    .collect(),
            ),
        ),
        ("scope", scope_subject(&fields.scope)),
        (
            "acceptance_criteria",
            acceptance_subject(&fields.acceptance_criteria),
        ),
        (
            "verification_commands",
            text_array(&fields.verification_commands),
        ),
        (
            "required_checks",
            CanonicalValue::Array(
                fields
                    .required_checks
                    .iter()
                    .map(|value| text(value.as_str()))
                    .collect(),
            ),
        ),
        (
            "requested_capabilities",
            capability_subject(&fields.requested_capabilities),
        ),
        ("budget", budget_subject(&fields.budget)),
        ("runtime_profile", text(fields.runtime_profile.as_str())),
        ("network_policy", text(fields.network_policy.as_str())),
        ("deployment_policy", text(fields.deployment_policy.as_str())),
        (
            "approval_requirements",
            object(vec![
                (
                    "execution",
                    text(fields.approval_requirements.execution.as_str()),
                ),
                ("merge", text(fields.approval_requirements.merge.as_str())),
                (
                    "protected_release",
                    text(fields.approval_requirements.protected_release.as_str()),
                ),
            ]),
        ),
    ])
}

fn scope_subject(scope: &TaskScope) -> CanonicalValue {
    object(vec![
        ("allowed_paths", text_array(&scope.allowed_paths)),
        ("forbidden_paths", text_array(&scope.forbidden_paths)),
        (
            "allowed_operations",
            CanonicalValue::Array(
                scope
                    .allowed_operations
                    .iter()
                    .map(|value| text(value.as_str()))
                    .collect(),
            ),
        ),
    ])
}

fn acceptance_subject(values: &[AcceptanceCriterion]) -> CanonicalValue {
    CanonicalValue::Array(
        values
            .iter()
            .map(|value| {
                object(vec![
                    ("id", text(&value.id)),
                    ("description", text(&value.description)),
                    ("evidence_type", text(value.evidence_type.as_str())),
                    ("expected_result", text(&value.expected_result)),
                ])
            })
            .collect(),
    )
}

fn capability_subject(values: &[CapabilityRequest]) -> CanonicalValue {
    CanonicalValue::Array(
        values
            .iter()
            .map(|value| {
                object(vec![
                    ("id", text(value.capability.as_str())),
                    ("contract_version", text(&value.contract_version)),
                ])
            })
            .collect(),
    )
}

fn budget_subject(value: &TaskBudget) -> CanonicalValue {
    object(vec![
        ("accounting_currency", text(&value.accounting_currency)),
        ("max_agents", text(&value.max_agents)),
        ("max_duration_seconds", text(&value.max_duration_seconds)),
        ("max_attempts", text(&value.max_attempts)),
        ("max_model_calls", text(&value.max_model_calls)),
        ("max_external_cost", text(&value.max_external_cost)),
    ])
}

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::String(normalize_nfc(value))
}

fn text_array(values: &[String]) -> CanonicalValue {
    CanonicalValue::Array(values.iter().map(|value| text(value)).collect())
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}
