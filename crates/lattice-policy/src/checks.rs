use lattice_contracts::RuntimeKind;
use lattice_task_domain::{
    ApprovalRequirement, DeploymentPolicy, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    TaskSpec,
};

use crate::decimal::{DecimalLimit, checked_sum_within, is_zero};
use crate::matrix::ProviderRequirement;
use crate::{
    ApprovalAuthority, ApprovalFact, ApprovalKind, ApprovalLane, ApprovalOrigin, ApprovalStatus,
    ApprovalSubject, Boundary, DecisionKind, DecisionStage, DeploymentIntent, ExternalCostFact,
    ExternalCostSubject, NetworkIntent, PolicyAction, PolicyDecision, PolicyReason,
    ProjectAuthorityFact, ProjectLifecycle, ProviderCapabilityFact, ResourceObservationSubject,
    ResourceUsageFact, RuntimeAdmission, SubjectBinding, TaskContext, WriterLeaseFact,
    WriterLeaseStatus, WriterLeaseSubject,
};

pub(crate) struct BaseContext<'a> {
    pub spec: &'a TaskSpec,
    pub project: &'a ProjectAuthorityFact,
    pub runtime: RuntimeAdmission,
}

pub(crate) fn base_context<'a>(
    context: &'a TaskContext<'a>,
    kind: DecisionKind,
) -> Result<BaseContext<'a>, PolicyDecision> {
    let Some(spec) = context.task_spec else {
        return Err(PolicyDecision::deny(
            kind,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        ));
    };
    let Some(project) = &context.project else {
        return Err(PolicyDecision::deny(
            kind,
            PolicyReason::ProjectNotRegistered,
            DecisionStage::Project,
        ));
    };
    if let Some(reason) = project_reason(spec, project) {
        return Err(PolicyDecision::deny(kind, reason, DecisionStage::Project));
    }
    let Boundary::Known(runtime) = context.runtime_admission else {
        return Err(PolicyDecision::deny(
            kind,
            PolicyReason::UnknownRuntimeAdmission,
            DecisionStage::Runtime,
        ));
    };
    Ok(BaseContext {
        spec,
        project,
        runtime,
    })
}

pub(crate) fn known_state(
    context: &TaskContext<'_>,
    kind: DecisionKind,
) -> Result<lattice_task_domain::TaskState, PolicyDecision> {
    let Boundary::Known(state) = context.state else {
        return Err(PolicyDecision::deny(
            kind,
            PolicyReason::UnknownState,
            DecisionStage::State,
        ));
    };
    Ok(state)
}

pub(crate) fn binding_reason(spec: &TaskSpec, binding: &SubjectBinding) -> Option<PolicyReason> {
    let fields = spec.fields();
    if binding.project_id().as_str() != fields.project_id {
        return Some(PolicyReason::ProjectIdMismatch);
    }
    if binding.project_snapshot_id() != &fields.project_snapshot_id {
        return Some(PolicyReason::ProjectSnapshotMismatch);
    }
    if binding.task_id() != &fields.task_id {
        return Some(PolicyReason::TaskIdMismatch);
    }
    if binding.task_revision() != fields.revision {
        return Some(PolicyReason::TaskRevisionMismatch);
    }
    if binding.task_spec_digest().as_str() != spec.spec_hash().to_hex() {
        return Some(PolicyReason::TaskSpecHashMismatch);
    }
    None
}

fn project_reason(spec: &TaskSpec, project: &ProjectAuthorityFact) -> Option<PolicyReason> {
    if let Some(reason) = binding_reason(spec, &project.binding) {
        return Some(reason);
    }
    let receipt = &project.receipt;
    if receipt.project_id() != project.binding.project_id() {
        return Some(PolicyReason::ProjectIdMismatch);
    }
    if receipt.project_snapshot_id() != project.binding.project_snapshot_id() {
        return Some(PolicyReason::ProjectSnapshotMismatch);
    }
    let current = &project.current_head;
    if current != &receipt.head() {
        return Some(PolicyReason::ProjectAuthorityStale);
    }
    let expected_runtime = match spec.fields().runtime_profile {
        RuntimeProfile::Fake => RuntimeKind::Fake,
        RuntimeProfile::Codex => RuntimeKind::Live,
    };
    if receipt.runtime() != expected_runtime {
        return Some(PolicyReason::RuntimeKindMismatch);
    }
    match receipt.lifecycle() {
        ProjectLifecycle::Active => {}
        ProjectLifecycle::Suspended => return Some(PolicyReason::ProjectInactive),
        ProjectLifecycle::ReconciliationRequired => {
            return Some(PolicyReason::ProjectDrifted);
        }
    }
    if canonical_git_branch(receipt.primary_branch().reference()).is_none() {
        return Some(PolicyReason::InvalidDecisionSubject);
    }
    None
}

pub(crate) fn provider_reason(
    spec: &TaskSpec,
    requirement: ProviderRequirement,
    fact: Option<&ProviderCapabilityFact>,
) -> Option<PolicyReason> {
    let Some(fact) = fact else {
        return Some(PolicyReason::CapabilityEvidenceMissing);
    };
    if !fact.available {
        return Some(PolicyReason::CapabilityEvidenceMissing);
    }
    if fact.contract_version != 1 {
        return Some(PolicyReason::CapabilityContractMismatch);
    }
    if !fact.fresh {
        return Some(PolicyReason::CapabilityEvidenceStale);
    }
    if binding_reason(spec, &fact.binding).is_some()
        || fact.provider != requirement.provider
        || fact.capability != requirement.capability
        || fact.provider_id.trim().is_empty()
        || fact.provider_version.trim().is_empty()
        || fact.expected_executable_digest != fact.observed_executable_digest
        || fact.expected_schema_digest != fact.observed_schema_digest
        || !fact.identity_verified
    {
        return Some(PolicyReason::CapabilityIdentityMismatch);
    }
    if !fact.boundary_verified {
        return Some(PolicyReason::ProviderBoundaryDenied);
    }
    let expected_runtime = match spec.fields().runtime_profile {
        RuntimeProfile::Fake => RuntimeKind::Fake,
        RuntimeProfile::Codex => RuntimeKind::Live,
    };
    if fact.runtime != expected_runtime {
        return Some(PolicyReason::RuntimeKindMismatch);
    }
    None
}

pub(crate) fn effect_reason(
    spec: &TaskSpec,
    network: &NetworkIntent,
    deployment: DeploymentIntent,
    resources: Option<&ResourceUsageFact>,
    resources_required: bool,
) -> Result<bool, PolicyReason> {
    match (spec.fields().network_policy, network) {
        (NetworkPolicy::Deny, NetworkIntent::None)
        | (
            NetworkPolicy::LoopbackOnly | NetworkPolicy::Allowlisted,
            NetworkIntent::None | NetworkIntent::Loopback,
        ) => {}
        (NetworkPolicy::Deny, _) => return Err(PolicyReason::NetworkDenied),
        (NetworkPolicy::LoopbackOnly, NetworkIntent::External { .. }) => {
            return Err(PolicyReason::NetworkTargetMismatch);
        }
        (NetworkPolicy::Allowlisted, NetworkIntent::External { .. }) => {
            return Err(PolicyReason::NetworkAllowlistUnbound);
        }
    }

    match (spec.fields().deployment_policy, deployment) {
        (DeploymentPolicy::Deny, DeploymentIntent::None)
        | (
            DeploymentPolicy::PrepareOnly | DeploymentPolicy::Authorized,
            DeploymentIntent::None | DeploymentIntent::PrepareArtifact,
        ) => {}
        (DeploymentPolicy::Deny, _) => return Err(PolicyReason::DeploymentDenied),
        (DeploymentPolicy::PrepareOnly, DeploymentIntent::Deploy) => {
            return Err(PolicyReason::DeploymentPrepareOnly);
        }
        (DeploymentPolicy::Authorized, DeploymentIntent::Deploy) => {
            return Err(PolicyReason::ProtectedSurfaceRequired);
        }
    }

    if !resources_required && resources.is_none() {
        return Ok(false);
    }
    let Some(resources) = resources else {
        return Err(PolicyReason::ResourceEvidenceInvalid);
    };
    let Some(requested) = resources.receipt.request().requested_external_cost() else {
        return Err(PolicyReason::ExternalCostUnknown);
    };
    is_zero(requested)
        .map(|zero| !zero)
        .map_err(|()| PolicyReason::ResourceEvidenceInvalid)
}

pub(crate) fn risk_approval_requirement(spec: &TaskSpec, merge: bool) -> ApprovalRequirement {
    let risk = match spec.fields().risk_class {
        RiskClass::R0 => ApprovalRequirement::NotRequired,
        RiskClass::R1 => ApprovalRequirement::Policy,
        RiskClass::R2 | RiskClass::R3 => ApprovalRequirement::ResponsibleUser,
    };
    let requested = if merge {
        spec.fields().approval_requirements.merge
    } else {
        spec.fields().approval_requirements.execution
    };
    stricter_requirement(risk, requested)
}

pub(crate) fn stricter_requirement(
    left: ApprovalRequirement,
    right: ApprovalRequirement,
) -> ApprovalRequirement {
    if requirement_rank(left) >= requirement_rank(right) {
        left
    } else {
        right
    }
}

pub(crate) fn approval_reason(
    spec: &TaskSpec,
    fact: Option<&ApprovalFact>,
    expected_kind: ApprovalKind,
    expected_subject: &ApprovalSubject,
    requirement: ApprovalRequirement,
    require_independent_checks: bool,
) -> Option<PolicyReason> {
    if matches!(
        requirement,
        ApprovalRequirement::NotRequired | ApprovalRequirement::Policy
    ) {
        return require_independent_checks.then_some(PolicyReason::ReviewAuthorityUnavailable);
    }
    let Some(fact) = fact else {
        return Some(if requirement == ApprovalRequirement::ProtectedGuardian {
            PolicyReason::GuardianApprovalRequired
        } else {
            PolicyReason::ApprovalMissing
        });
    };
    let receipt = &fact.receipt;
    let identity = receipt.identity();
    if identity.subject().kind() != expected_kind || expected_subject.kind() != expected_kind {
        return Some(PolicyReason::ApprovalKindMismatch);
    }
    if binding_reason(spec, identity.binding()).is_some() || identity.subject() != expected_subject
    {
        return Some(PolicyReason::ApprovalSubjectMismatch);
    }
    let expected_runtime = match spec.fields().runtime_profile {
        RuntimeProfile::Fake => RuntimeKind::Fake,
        RuntimeProfile::Codex => RuntimeKind::Live,
    };
    if receipt.runtime() != expected_runtime {
        return Some(PolicyReason::RuntimeKindMismatch);
    }
    let (expected_authority, expected_origin, expected_lane, expected_status) = match requirement {
        ApprovalRequirement::ResponsibleUser => (
            ApprovalAuthority::ResponsibleUser,
            ApprovalOrigin::OsAuthenticatedUser,
            ApprovalLane::Normal,
            ApprovalStatus::Available,
        ),
        ApprovalRequirement::ProtectedGuardian => (
            ApprovalAuthority::ProtectedGuardian,
            ApprovalOrigin::GuardianTrustRoot,
            ApprovalLane::Protected,
            ApprovalStatus::ProtectedPendingClaim,
        ),
        ApprovalRequirement::NotRequired | ApprovalRequirement::Policy => unreachable!(),
    };
    if identity.authority() != expected_authority || identity.lane() != expected_lane {
        return Some(PolicyReason::ApprovalAuthorityDenied);
    }
    if identity.origin() != expected_origin {
        return Some(if requirement == ApprovalRequirement::ProtectedGuardian {
            PolicyReason::GuardianRequired
        } else {
            PolicyReason::ApprovalAuthorityDenied
        });
    }
    if identity.requester_id() == identity.approver_id() {
        return Some(PolicyReason::SelfApprovalDenied);
    }
    if receipt.status() != expected_status {
        return Some(match receipt.status() {
            ApprovalStatus::ClaimedNormal => PolicyReason::ApprovalReplayed,
            ApprovalStatus::Available
            | ApprovalStatus::ProtectedPendingClaim
            | ApprovalStatus::Revoked => PolicyReason::ApprovalStale,
        });
    }
    let Some(current_head) = fact.current_head.as_ref() else {
        return Some(PolicyReason::ApprovalStale);
    };
    match current_head.status() {
        ApprovalStatus::ClaimedNormal => return Some(PolicyReason::ApprovalReplayed),
        ApprovalStatus::Revoked => return Some(PolicyReason::ApprovalStale),
        ApprovalStatus::Available | ApprovalStatus::ProtectedPendingClaim => {}
    }
    if current_head != &receipt.head() {
        return Some(PolicyReason::ApprovalStale);
    }
    if require_independent_checks {
        return Some(PolicyReason::ReviewAuthorityUnavailable);
    }
    None
}

pub(crate) fn r3_checks_required(spec: &TaskSpec) -> bool {
    spec.fields().risk_class == RiskClass::R3
        || (spec
            .fields()
            .required_checks
            .contains(&RequiredCheck::Security)
            && spec
                .fields()
                .required_checks
                .contains(&RequiredCheck::Architecture))
}

pub(crate) fn writer_reason(
    spec: &TaskSpec,
    action: PolicyAction,
    current_runtime: RuntimeAdmission,
    actor_id: Option<&str>,
    expected_subject: Option<&WriterLeaseSubject>,
    fact: Option<&WriterLeaseFact>,
) -> Option<PolicyReason> {
    let Some(expected_subject) = expected_subject else {
        return Some(PolicyReason::WriterLeaseRequired);
    };
    let Some(fact) = fact else {
        return Some(PolicyReason::WriterLeaseRequired);
    };
    let Some(current_head) = fact.current_head.as_ref() else {
        return Some(PolicyReason::WriterLeaseNotCurrent);
    };
    let receipt = &fact.receipt;
    if receipt.head() != *current_head {
        return Some(PolicyReason::WriterLeaseNotCurrent);
    }
    if action == PolicyAction::ReleaseWriter {
        if !matches!(
            current_runtime,
            RuntimeAdmission::Active | RuntimeAdmission::Draining
        ) {
            return Some(PolicyReason::RuntimeAdmissionDenied);
        }
        if !matches!(
            receipt.status(),
            WriterLeaseStatus::Active | WriterLeaseStatus::Suspect
        ) {
            return Some(PolicyReason::WriterLeaseNotCurrent);
        }
    } else {
        if current_runtime != RuntimeAdmission::Active {
            return Some(PolicyReason::RuntimeAdmissionDenied);
        }
        if receipt.status() != WriterLeaseStatus::Active {
            return Some(PolicyReason::WriterLeaseNotCurrent);
        }
    }

    let fields = spec.fields();
    let identity = receipt.identity();
    let expected_runtime = match fields.runtime_profile {
        RuntimeProfile::Fake => RuntimeKind::Fake,
        RuntimeProfile::Codex => RuntimeKind::Live,
    };
    if identity.project_id().as_str() != fields.project_id
        || identity.project_snapshot_id() != &fields.project_snapshot_id
        || identity.task_id() != &fields.task_id
        || identity.task_revision() != fields.revision
        || identity.task_spec_digest().as_str() != spec.spec_hash().to_hex()
        || identity.lease_holder_id() != expected_subject.lease_holder_id
        || identity.lease_id() != expected_subject.lease_id
        || identity.attempt_id() != &expected_subject.attempt_id
        || identity.worktree_id() != expected_subject.worktree_id
        || identity.holder_process_id() != expected_subject.holder_process_id
        || identity.holder_process_start_identity()
            != &expected_subject.holder_process_start_identity
        || identity.daemon_instance_id() != expected_subject.daemon_instance_id
        || identity.daemon_epoch() != expected_subject.daemon_epoch
        || receipt.runtime() != expected_subject.runtime
        || receipt.runtime() != expected_runtime
        || expected_subject.lease_holder_id.trim().is_empty()
        || expected_subject.lease_id.trim().is_empty()
        || expected_subject.worktree_id.trim().is_empty()
        || expected_subject.daemon_instance_id.trim().is_empty()
        || actor_id.is_some_and(|actor| {
            actor.trim().is_empty() || actor != expected_subject.lease_holder_id
        })
    {
        return Some(PolicyReason::WriterLeaseSubjectMismatch);
    }
    if identity.fencing_token() != expected_subject.fencing_token {
        return Some(PolicyReason::FencingTokenMismatch);
    }
    None
}

pub(crate) fn external_cost_subject(
    spec: &TaskSpec,
    resources: &ResourceUsageFact,
    fact: Option<&ExternalCostFact>,
) -> Result<Option<ExternalCostSubject>, PolicyReason> {
    let Some(requested) = resources.receipt.request().requested_external_cost() else {
        return Err(PolicyReason::ExternalCostUnknown);
    };
    let zero = is_zero(requested).map_err(|()| PolicyReason::ResourceEvidenceInvalid)?;
    if zero {
        return Ok(None);
    }
    let Some(fact) = fact else {
        return Err(PolicyReason::ExternalCostUnknown);
    };
    if binding_reason(spec, &fact.binding).is_some()
        || fact.subject.amount() != requested
        || resources.receipt.accounting_currency() != spec.fields().budget.accounting_currency
        || fact.subject.currency() != resources.receipt.accounting_currency()
        || fact.subject.currency().trim().is_empty()
        || fact.subject.provider_id().trim().is_empty()
        || !fact.quote_verified
        || !fact.fresh
    {
        return Err(PolicyReason::ExternalCostUnknown);
    }
    Ok(Some(fact.subject.clone()))
}

pub(crate) fn requested_cost_is_zero(resources: &ResourceUsageFact) -> Result<bool, PolicyReason> {
    let Some(requested) = resources.receipt.request().requested_external_cost() else {
        return Err(PolicyReason::ExternalCostUnknown);
    };
    is_zero(requested).map_err(|()| PolicyReason::ResourceEvidenceInvalid)
}

pub(crate) fn valid_git_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Pure equivalent of the relevant `git check-ref-format --branch` rules.
pub(crate) fn valid_git_ref(value: &str) -> bool {
    if value.is_empty()
        || value.trim() != value
        || value == "@"
        || value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("//")
        || value.contains("@{")
        || value.contains('\\')
        || value.bytes().any(|byte| {
            byte.is_ascii_control()
                || matches!(byte, b' ' | b'~' | b'^' | b':' | b'?' | b'*' | b'[')
        })
    {
        return false;
    }
    value.split('/').all(|component| {
        !component.is_empty()
            && !component.starts_with('.')
            && !component
                .get(component.len().saturating_sub(5)..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".lock"))
    })
}

/// Accepts only a fully qualified local-branch reference and returns its
/// Registry comparison identity. Revision DWIM and other namespaces deny.
pub(crate) fn canonical_git_branch(value: &str) -> Option<&str> {
    if !valid_git_ref(value) {
        return None;
    }
    let canonical = value.strip_prefix("refs/heads/")?;
    if canonical.is_empty()
        || looks_like_git_pseudoref(canonical)
        || ["refs/", "heads/", "tags/", "remotes/"]
            .iter()
            .any(|prefix| canonical.starts_with(prefix))
        || !valid_git_ref(canonical)
    {
        return None;
    }
    Some(canonical)
}

fn looks_like_git_pseudoref(value: &str) -> bool {
    !value.contains('/')
        && matches!(
            value,
            "HEAD"
                | "AUTO_MERGE"
                | "BISECT_EXPECTED_REV"
                | "BISECT_HEAD"
                | "BISECT_START"
                | "CHERRY_PICK_HEAD"
                | "FETCH_HEAD"
                | "MERGE_HEAD"
                | "ORIG_HEAD"
                | "REBASE_HEAD"
                | "REVERT_HEAD"
        )
}

pub(crate) fn resource_binding_reason(
    spec: &TaskSpec,
    resources: &ResourceUsageFact,
) -> Option<PolicyReason> {
    if let Some(reason) = binding_reason(spec, &resources.binding) {
        return Some(reason);
    }
    let identity = resources.receipt.stream_head().identity();
    let fields = spec.fields();
    if identity.project_id().as_str() != fields.project_id {
        return Some(PolicyReason::ProjectIdMismatch);
    }
    if identity.project_snapshot_id() != &fields.project_snapshot_id {
        return Some(PolicyReason::ProjectSnapshotMismatch);
    }
    if identity.task_id() != &fields.task_id {
        return Some(PolicyReason::TaskIdMismatch);
    }
    if identity.task_revision() != fields.revision {
        return Some(PolicyReason::TaskRevisionMismatch);
    }
    if identity.task_spec_digest().as_str() != spec.spec_hash().to_hex() {
        return Some(PolicyReason::TaskSpecHashMismatch);
    }
    None
}

pub(crate) fn resource_reason(
    spec: &TaskSpec,
    expected: &ResourceObservationSubject,
    resources: &ResourceUsageFact,
) -> Option<PolicyReason> {
    let receipt = &resources.receipt;
    let stream_head = receipt.stream_head();
    if expected.observation_revision == 0
        || expected.effect_claim_id.trim().is_empty()
        || expected.stream_id != *stream_head.stream_id()
        || expected.stream_head_digest != *stream_head.head_digest()
        || receipt.observation_revision() != expected.observation_revision
        || receipt.effect_claim_id() != expected.effect_claim_id
        || receipt.effect_subject_digest() != &expected.effect_subject_digest
        || receipt.request() != &expected.request
    {
        return Some(PolicyReason::ResourceEvidenceInvalid);
    }
    if resources.current_head.as_ref() != Some(&receipt.head()) {
        return Some(PolicyReason::ResourceEvidenceStale);
    }
    let expected_runtime = match spec.fields().runtime_profile {
        RuntimeProfile::Fake => RuntimeKind::Fake,
        RuntimeProfile::Codex => RuntimeKind::Live,
    };
    if receipt.runtime() != expected_runtime {
        return Some(PolicyReason::RuntimeKindMismatch);
    }
    if receipt.accounting_currency() != spec.fields().budget.accounting_currency {
        return Some(PolicyReason::ResourceCurrencyMismatch);
    }
    let counters = receipt.counters();
    let request = receipt.request();
    let budget = &spec.fields().budget;
    let Ok(max_agents) = budget.max_agents.parse::<u64>() else {
        return Some(PolicyReason::InternalPolicyError);
    };
    let Ok(max_duration) = budget.max_duration_seconds.parse::<u64>() else {
        return Some(PolicyReason::InternalPolicyError);
    };
    let Ok(max_attempts) = budget.max_attempts.parse::<u64>() else {
        return Some(PolicyReason::InternalPolicyError);
    };
    let Ok(max_model_calls) = budget.max_model_calls.parse::<u64>() else {
        return Some(PolicyReason::InternalPolicyError);
    };
    let Some(agents) = counters
        .active_agents()
        .checked_add(request.requested_agents())
    else {
        return Some(PolicyReason::ResourceEvidenceInvalid);
    };
    if agents > max_agents.min(4) {
        return Some(PolicyReason::AgentLimitExceeded);
    }
    let Some(implementers) = counters
        .active_implementers()
        .checked_add(request.requested_implementers())
    else {
        return Some(PolicyReason::ResourceEvidenceInvalid);
    };
    if implementers > 1 {
        return Some(PolicyReason::MultipleImplementers);
    }
    let Some(duration) = counters
        .elapsed_seconds()
        .checked_add(request.requested_duration_seconds())
    else {
        return Some(PolicyReason::ResourceEvidenceInvalid);
    };
    if duration > max_duration {
        return Some(PolicyReason::DurationBudgetExceeded);
    }
    let Some(attempts) = counters
        .attempt_number()
        .checked_add(request.requested_attempts())
    else {
        return Some(PolicyReason::ResourceEvidenceInvalid);
    };
    if attempts > max_attempts {
        return Some(PolicyReason::AttemptBudgetExceeded);
    }
    let Some(model_calls) = counters
        .used_model_calls()
        .checked_add(request.requested_model_calls())
    else {
        return Some(PolicyReason::ResourceEvidenceInvalid);
    };
    if model_calls > max_model_calls {
        return Some(PolicyReason::ModelCallBudgetExceeded);
    }
    let Some(requested_cost) = request.requested_external_cost() else {
        return Some(PolicyReason::ExternalCostUnknown);
    };
    match checked_sum_within(
        counters.used_external_cost(),
        requested_cost,
        &budget.max_external_cost,
    ) {
        DecimalLimit::Within => None,
        DecimalLimit::Exceeded => Some(PolicyReason::ExternalCostBudgetExceeded),
        DecimalLimit::Invalid => Some(PolicyReason::ResourceEvidenceInvalid),
    }
}

const fn requirement_rank(requirement: ApprovalRequirement) -> u8 {
    match requirement {
        ApprovalRequirement::NotRequired => 0,
        ApprovalRequirement::Policy => 1,
        ApprovalRequirement::ResponsibleUser => 2,
        ApprovalRequirement::ProtectedGuardian => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::{canonical_git_branch, valid_git_ref};

    #[test]
    fn git_reference_validation_matches_fail_closed_policy_subset() {
        for valid in ["main", "feature/policy-v2", "release/2026.07"] {
            assert!(valid_git_ref(valid), "expected valid ref: {valid}");
        }
        for invalid in [
            "",
            "@",
            "-main",
            "/main",
            "main/",
            "main.",
            "bad..ref",
            "bad//ref",
            "bad@{ref",
            "bad\\ref",
            "bad ref",
            "bad~ref",
            "bad^ref",
            "bad:ref",
            "bad?ref",
            "bad*ref",
            "bad[ref",
            ".hidden/ref",
            "path/.hidden",
            "foo.lock/bar",
            "foo/Bar.LOCK",
        ] {
            assert!(!valid_git_ref(invalid), "expected invalid ref: {invalid}");
        }
    }

    #[test]
    fn canonical_branch_rejects_revision_pseudorefs_and_ambiguous_namespaces() {
        assert_eq!(canonical_git_branch("refs/heads/main"), Some("main"));
        assert_eq!(canonical_git_branch("refs/heads/WIP"), Some("WIP"));
        assert_eq!(
            canonical_git_branch("refs/heads/RELEASE_2026"),
            Some("RELEASE_2026")
        );
        for invalid in [
            "HEAD",
            "refs/heads/HEAD",
            "FETCH_HEAD",
            "AUTO_MERGE",
            "BISECT_EXPECTED_REV",
            "BISECT_START",
            "integration",
            "origin/main",
            "v1",
            "heads/main",
            "tags/main",
            "remotes/origin/main",
        ] {
            assert_eq!(
                canonical_git_branch(invalid),
                None,
                "expected non-branch revision syntax to deny: {invalid}"
            );
        }
    }
}
