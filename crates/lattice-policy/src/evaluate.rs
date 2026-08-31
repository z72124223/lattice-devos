use lattice_contracts::ContentDigest;
use lattice_task_domain::{ApprovalRequirement, Capability, DeploymentPolicy, TaskSpec};

use crate::checks::{
    approval_reason, base_context, binding_reason, canonical_git_branch, effect_reason,
    external_cost_subject, known_state, provider_reason, r3_checks_required,
    requested_cost_is_zero, resource_binding_reason, resource_reason, risk_approval_requirement,
    stricter_requirement, valid_git_object_id, writer_reason,
};
use crate::matrix::{
    consumes_resources, is_protected_action, is_recovery_action, provider_requirement,
    required_capabilities, requires_execution_approval, requires_writer, role_allows_action,
    runtime_allows_action, state_allows_action, targets_writer, upgrade_role,
    upgrade_runtime_allowed, upgrade_state_allowed,
};
use crate::{
    AgentActionGate, AgentRole, ApprovalKind, ApprovalSubject, Boundary, DecisionKind,
    DecisionStage, DecisionSubject, DeploymentIntent, ExecutionGate, ExecutionGateDecisionEvidence,
    GuardianSagaOutcome, ManagedExecutionBindingFact, MemoryKind, MemoryPromotionGate, MergeGate,
    MergeTarget, NetworkIntent, NormalRecoveryResolution, PolicyDecision, PolicyInputFailure,
    PolicyReason, ProjectClass, ProtectedChangeClass, ProtectedChangeGate, ProtectedReleaseSubject,
    RecoveryGate, RecoveryOwner, RecoverySubject, RuntimeAdmission, UpgradeGate, UpgradeStage,
    WorkerAdmissionGate,
};

/// Evaluates one closed policy subject.
#[must_use]
pub fn evaluate(subject: DecisionSubject<'_>) -> PolicyDecision {
    match subject {
        DecisionSubject::Invalid(failure) => evaluate_invalid(failure),
        DecisionSubject::ExecutionGate(gate) => evaluate_execution(&gate),
        DecisionSubject::AgentAction(gate) => evaluate_agent_action(&gate),
        DecisionSubject::WorkerAdmission(gate) => evaluate_worker_admission(&gate),
        DecisionSubject::MergeGate(gate) => evaluate_merge(&gate),
        DecisionSubject::MemoryPromotion(gate) => evaluate_memory(&gate),
        DecisionSubject::UpgradeStage(gate) => evaluate_upgrade(&gate),
        DecisionSubject::Recovery(gate) => evaluate_recovery(&gate),
        DecisionSubject::ProtectedChange(gate) => evaluate_protected_change(&gate),
    }
}

/// Evaluates the existing execution subject and captures the exact owned input
/// facts in one opaque result. Downstream approval code can inspect this
/// evidence but cannot construct or substitute it.
#[must_use]
pub fn evaluate_execution_gate_with_evidence(
    gate: ExecutionGate<'_>,
) -> ExecutionGateDecisionEvidence {
    let task_spec_digest = gate
        .context
        .task_spec
        .and_then(|task_spec| ContentDigest::from_sha256(task_spec.spec_hash().to_hex()).ok());
    let (project_binding, project_receipt, current_project_head) = gate
        .context
        .project
        .as_ref()
        .map_or((None, None, None), |project| {
            (
                Some(project.binding.clone()),
                Some(project.receipt.clone()),
                Some(project.current_head.clone()),
            )
        });
    let state = gate.context.state;
    let runtime_admission = gate.context.runtime_admission;
    let decision = evaluate(DecisionSubject::ExecutionGate(gate));
    ExecutionGateDecisionEvidence::new(
        decision,
        task_spec_digest,
        project_binding,
        project_receipt,
        current_project_head,
        None,
        state,
        runtime_admission,
    )
}

/// Evaluates a managed execution gate and seals the exact immutable
/// Task-Ledger execution binding into the returned opaque evidence.
///
/// A binding whose Task Spec does not equal the actual gate subject (or which
/// contains a zero digest) is denied before the ordinary execution decision.
#[must_use]
pub fn evaluate_managed_execution_gate_with_evidence(
    gate: ExecutionGate<'_>,
    binding: ManagedExecutionBindingFact,
) -> ExecutionGateDecisionEvidence {
    let task_spec_digest = gate
        .context
        .task_spec
        .and_then(|task_spec| ContentDigest::from_sha256(task_spec.spec_hash().to_hex()).ok());
    let (project_binding, project_receipt, current_project_head) = gate
        .context
        .project
        .as_ref()
        .map_or((None, None, None), |project| {
            (
                Some(project.binding.clone()),
                Some(project.receipt.clone()),
                Some(project.current_head.clone()),
            )
        });
    let state = gate.context.state;
    let runtime_admission = gate.context.runtime_admission;
    let binding_is_exact = task_spec_digest.as_ref() == Some(&binding.task_spec_digest)
        && [
            &binding.task_ref,
            &binding.successor_stream_id,
            &binding.task_spec_digest,
            &binding.approval_subject_digest,
            &binding.budget_digest,
        ]
        .into_iter()
        .all(|digest| !digest.as_str().bytes().all(|byte| byte == b'0'));
    let decision = if binding_is_exact {
        evaluate(DecisionSubject::ExecutionGate(gate))
    } else {
        PolicyDecision::deny(
            DecisionKind::ExecutionGate,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        )
    };
    ExecutionGateDecisionEvidence::new(
        decision,
        task_spec_digest,
        project_binding,
        project_receipt,
        current_project_head,
        Some(binding),
        state,
        runtime_admission,
    )
}

fn evaluate_invalid(failure: PolicyInputFailure) -> PolicyDecision {
    PolicyDecision::deny(
        DecisionKind::Invalid,
        invalid_reason(failure),
        DecisionStage::Input,
    )
}

#[allow(clippy::too_many_lines)]
fn evaluate_agent_action(gate: &AgentActionGate<'_>) -> PolicyDecision {
    const KIND: DecisionKind = DecisionKind::AgentAction;
    if gate.context.task_spec.is_none() {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        );
    }
    if matches!(gate.role, Boundary::Unknown) {
        return deny(KIND, PolicyReason::UnknownRole, DecisionStage::Input);
    }
    if matches!(gate.action, Boundary::Unknown) {
        return deny(KIND, PolicyReason::UnknownAction, DecisionStage::Input);
    }
    if matches!(gate.context.state, Boundary::Unknown) {
        return deny(KIND, PolicyReason::UnknownState, DecisionStage::Input);
    }
    if matches!(gate.context.runtime_admission, Boundary::Unknown) {
        return deny(
            KIND,
            PolicyReason::UnknownRuntimeAdmission,
            DecisionStage::Input,
        );
    }
    let base = match base_context(&gate.context, KIND) {
        Ok(base) => base,
        Err(decision) => return decision,
    };
    if let Some(resources) = gate.resources.as_ref()
        && let Some(reason) = resource_binding_reason(base.spec, resources)
    {
        return deny(KIND, reason, DecisionStage::Project);
    }
    if !runtime_allows_action(base.runtime, gate.action) {
        return deny(
            KIND,
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }

    let Boundary::Known(role) = gate.role else {
        return deny(KIND, PolicyReason::UnknownRole, DecisionStage::RoleAction);
    };
    let Boundary::Known(action) = gate.action else {
        return deny(KIND, PolicyReason::UnknownAction, DecisionStage::RoleAction);
    };
    if !role_allows_action(role, action) {
        return deny(
            KIND,
            PolicyReason::RoleActionDenied,
            DecisionStage::RoleAction,
        );
    }

    let state = match known_state(&gate.context, KIND) {
        Ok(state) => state,
        Err(decision) => return decision,
    };
    if !state_allows_action(state, action) {
        return deny(KIND, PolicyReason::ActionStateDenied, DecisionStage::State);
    }
    if is_protected_action(action) {
        return deny(
            KIND,
            PolicyReason::ProtectedSurfaceRequired,
            DecisionStage::Protected,
        );
    }

    for capability in required_capabilities(action) {
        if !requested_capability(base.spec, *capability) {
            return deny(
                KIND,
                PolicyReason::CapabilityNotRequested,
                DecisionStage::RequestedCapability,
            );
        }
    }
    if let Some(requirement) = provider_requirement(action)
        && let Some(reason) =
            provider_reason(base.spec, requirement, gate.provider_capability.as_ref())
    {
        return deny(KIND, reason, DecisionStage::ProviderCapability);
    }

    let requires_resources = consumes_resources(action);
    if is_recovery_action(action) && !recovery_effect_is_bounded(gate) {
        return deny(
            KIND,
            PolicyReason::ProtectedSurfaceRequired,
            DecisionStage::EffectEnvelope,
        );
    }
    let cost_requires_user = match effect_reason(
        base.spec,
        &gate.network,
        gate.deployment,
        gate.resources.as_ref(),
        requires_resources,
    ) {
        Ok(requires_user) => requires_user,
        Err(reason) => return deny(KIND, reason, DecisionStage::EffectEnvelope),
    };
    if matches!(gate.network, NetworkIntent::Loopback) && provider_requirement(action).is_none() {
        return deny(
            KIND,
            PolicyReason::NetworkTargetMismatch,
            DecisionStage::EffectEnvelope,
        );
    }
    if gate.deployment == DeploymentIntent::PrepareArtifact {
        return deny(
            KIND,
            PolicyReason::ProtectedSurfaceRequired,
            DecisionStage::EffectEnvelope,
        );
    }
    let exact_external_cost = if cost_requires_user {
        let Some(resources) = gate.resources.as_ref() else {
            return deny(
                KIND,
                PolicyReason::ResourceEvidenceInvalid,
                DecisionStage::EffectEnvelope,
            );
        };
        match external_cost_subject(base.spec, resources, gate.external_cost.as_ref()) {
            Ok(subject) => subject,
            Err(reason) => return deny(KIND, reason, DecisionStage::EffectEnvelope),
        }
    } else {
        None
    };
    if let Some(cost) = exact_external_cost.as_ref() {
        let (Some(requirement), Some(provider)) = (
            provider_requirement(action),
            gate.provider_capability.as_ref(),
        ) else {
            return deny(
                KIND,
                PolicyReason::ExternalCostUnknown,
                DecisionStage::EffectEnvelope,
            );
        };
        if provider.provider != requirement.provider || cost.provider_id() != provider.provider_id {
            return deny(
                KIND,
                PolicyReason::ExternalCostUnknown,
                DecisionStage::EffectEnvelope,
            );
        }
    }
    if cost_requires_user && gate.approval.is_none() {
        return deny(
            KIND,
            PolicyReason::ExternalCostProtected,
            DecisionStage::EffectEnvelope,
        );
    }

    if requires_execution_approval(action) || cost_requires_user {
        let mut requirement = risk_approval_requirement(base.spec, false);
        if cost_requires_user {
            requirement = stricter_requirement(requirement, ApprovalRequirement::ResponsibleUser);
        }
        let Some(task_spec_hash) = task_spec_digest(base.spec) else {
            return deny(
                KIND,
                PolicyReason::InternalPolicyError,
                DecisionStage::Approval,
            );
        };
        let subject = ApprovalSubject::Execution {
            task_spec_hash,
            external_cost: exact_external_cost,
        };
        if let Some(reason) = approval_reason(
            base.spec,
            gate.approval.as_ref(),
            ApprovalKind::Execution,
            &subject,
            requirement,
            r3_checks_required(base.spec),
        ) {
            return deny(KIND, reason, DecisionStage::Approval);
        }
    }

    if (requires_writer(action) || targets_writer(action))
        && let Some(reason) = writer_reason(
            base.spec,
            action,
            base.runtime,
            (requires_writer(action) || targets_writer(action)).then_some(gate.actor_id.as_str()),
            gate.writer_subject.as_ref(),
            gate.writer.as_ref(),
        )
    {
        return deny(KIND, reason, DecisionStage::Writer);
    }
    if (requires_resources || gate.resources.is_some() || gate.resource_subject.is_some())
        && !is_recovery_action(action)
    {
        let (Some(resource_subject), Some(resources)) =
            (gate.resource_subject.as_ref(), gate.resources.as_ref())
        else {
            return deny(
                KIND,
                PolicyReason::ResourceEvidenceInvalid,
                DecisionStage::Resources,
            );
        };
        if let Some(reason) = resource_reason(base.spec, resource_subject, resources) {
            return deny(KIND, reason, DecisionStage::Resources);
        }
    }

    PolicyDecision::allow(KIND, PolicyReason::AgentActionAllowed)
}

fn evaluate_execution(gate: &ExecutionGate<'_>) -> PolicyDecision {
    const KIND: DecisionKind = DecisionKind::ExecutionGate;
    if let Some(decision) = context_input_denial(&gate.context, KIND) {
        return decision;
    }
    let base = match base_context(&gate.context, KIND) {
        Ok(base) => base,
        Err(decision) => return decision,
    };
    if base.runtime != RuntimeAdmission::Active {
        return deny(
            KIND,
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }
    let state = match known_state(&gate.context, KIND) {
        Ok(state) => state,
        Err(decision) => return decision,
    };
    if state != lattice_task_domain::TaskState::AwaitingExecutionApproval {
        return deny(KIND, PolicyReason::ActionStateDenied, DecisionStage::State);
    }
    let Some(task_spec_hash) = task_spec_digest(base.spec) else {
        return deny(
            KIND,
            PolicyReason::InternalPolicyError,
            DecisionStage::Approval,
        );
    };
    let subject = ApprovalSubject::Execution {
        task_spec_hash,
        external_cost: None,
    };
    if let Some(reason) = approval_reason(
        base.spec,
        gate.approval.as_ref(),
        ApprovalKind::Execution,
        &subject,
        risk_approval_requirement(base.spec, false),
        r3_checks_required(base.spec),
    ) {
        return deny(KIND, reason, DecisionStage::Approval);
    }
    PolicyDecision::allow(KIND, PolicyReason::ExecutionGateAllowed)
}

fn evaluate_worker_admission(gate: &WorkerAdmissionGate<'_>) -> PolicyDecision {
    const KIND: DecisionKind = DecisionKind::WorkerAdmission;
    if let Some(decision) = context_input_denial(&gate.context, KIND) {
        return decision;
    }
    let base = match base_context(&gate.context, KIND) {
        Ok(base) => base,
        Err(decision) => return decision,
    };
    if let Some(reason) = resource_binding_reason(base.spec, &gate.resources) {
        return deny(KIND, reason, DecisionStage::Project);
    }
    if base.runtime != RuntimeAdmission::Active {
        return deny(
            KIND,
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }
    let state = match known_state(&gate.context, KIND) {
        Ok(state) => state,
        Err(decision) => return decision,
    };
    if !matches!(
        state,
        lattice_task_domain::TaskState::Draft | lattice_task_domain::TaskState::Preparing
    ) {
        return deny(KIND, PolicyReason::ActionStateDenied, DecisionStage::State);
    }
    match requested_cost_is_zero(&gate.resources) {
        Ok(true) => {}
        Ok(false) => {
            return deny(
                KIND,
                PolicyReason::ExternalCostProtected,
                DecisionStage::EffectEnvelope,
            );
        }
        Err(reason) => return deny(KIND, reason, DecisionStage::EffectEnvelope),
    }
    let Ok(worker_count) = u64::try_from(gate.workers.len()) else {
        return deny(
            KIND,
            PolicyReason::ResourceEvidenceInvalid,
            DecisionStage::Resources,
        );
    };
    let implementers = gate
        .workers
        .iter()
        .filter(|role| **role == AgentRole::Implementer)
        .count();
    let Ok(implementers) = u64::try_from(implementers) else {
        return deny(
            KIND,
            PolicyReason::ResourceEvidenceInvalid,
            DecisionStage::Resources,
        );
    };
    if gate.workers.is_empty()
        || gate.resources.receipt.request().requested_agents() != worker_count
        || gate.resources.receipt.request().requested_implementers() != implementers
    {
        return deny(
            KIND,
            PolicyReason::ResourceEvidenceInvalid,
            DecisionStage::Resources,
        );
    }
    if implementers > 1 {
        return deny(
            KIND,
            PolicyReason::MultipleImplementers,
            DecisionStage::Resources,
        );
    }
    if let Some(reason) = resource_reason(base.spec, &gate.resource_subject, &gate.resources) {
        return deny(KIND, reason, DecisionStage::Resources);
    }
    PolicyDecision::allow(KIND, PolicyReason::WorkerAdmissionAllowed)
}

#[allow(clippy::too_many_lines)]
fn evaluate_merge(gate: &MergeGate<'_>) -> PolicyDecision {
    const KIND: DecisionKind = DecisionKind::MergeGate;
    if let Some(decision) = role_context_input_denial(&gate.context, gate.role, KIND) {
        return decision;
    }
    let Some(target_reference) = gate.subject.target().reference() else {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        );
    };
    let Some(canonical_target) = canonical_git_branch(target_reference) else {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        );
    };
    if !valid_git_object_id(gate.subject.reviewed_commit())
        || !valid_git_object_id(gate.subject.target_head())
    {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        );
    }
    let base = match base_context(&gate.context, KIND) {
        Ok(base) => base,
        Err(decision) => return decision,
    };
    if let Some(reason) = resource_binding_reason(base.spec, &gate.resources) {
        return deny(KIND, reason, DecisionStage::Project);
    }
    let Some(readiness) = gate.readiness.as_ref() else {
        return deny(
            KIND,
            PolicyReason::MergeReadinessRequired,
            DecisionStage::Project,
        );
    };
    if let Some(reason) = binding_reason(base.spec, &readiness.binding) {
        return deny(KIND, reason, DecisionStage::Project);
    }
    if readiness.subject != gate.subject
        || readiness.producer != Boundary::Known(crate::MergeAnalysisProducer::WorkspaceGit)
        || readiness.producer_id.trim().is_empty()
        || readiness.producer_version.trim().is_empty()
        || readiness.target_ref_identity.reference() != target_reference
        || !readiness.scope_verified
    {
        return deny(
            KIND,
            PolicyReason::MergeReadinessMismatch,
            DecisionStage::Project,
        );
    }
    if !readiness.fresh {
        return deny(
            KIND,
            PolicyReason::MergeReadinessStale,
            DecisionStage::Project,
        );
    }
    let Some(canonical_primary) =
        canonical_git_branch(base.project.receipt.primary_branch().reference())
    else {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Project,
        );
    };
    let same_text_identity = canonical_target == canonical_primary;
    let same_storage_identity = readiness.target_ref_identity.storage_identity_digest()
        == base
            .project
            .receipt
            .primary_branch()
            .storage_identity_digest();
    if same_text_identity && !same_storage_identity {
        return deny(
            KIND,
            PolicyReason::MergeReadinessMismatch,
            DecisionStage::Project,
        );
    }
    let primary = same_storage_identity;
    let declared_primary = matches!(gate.subject.target(), MergeTarget::PrimaryBranch(_));
    if primary != declared_primary {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Project,
        );
    }
    if base.runtime != RuntimeAdmission::Active {
        return deny(
            KIND,
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }
    let Boundary::Known(role) = gate.role else {
        return deny(KIND, PolicyReason::UnknownRole, DecisionStage::RoleAction);
    };
    if role != AgentRole::Integrator {
        return deny(
            KIND,
            PolicyReason::RoleActionDenied,
            DecisionStage::RoleAction,
        );
    }
    let state = match known_state(&gate.context, KIND) {
        Ok(state) => state,
        Err(decision) => return decision,
    };
    if state != lattice_task_domain::TaskState::AwaitingMergeApproval {
        return deny(KIND, PolicyReason::ActionStateDenied, DecisionStage::State);
    }
    if !readiness.conflict_free {
        return deny(
            KIND,
            PolicyReason::MergeConflictRequiresImplementer,
            DecisionStage::Protected,
        );
    }
    if !requested_capability(base.spec, Capability::GitIntegrate) {
        return deny(
            KIND,
            PolicyReason::CapabilityNotRequested,
            DecisionStage::RequestedCapability,
        );
    }
    match requested_cost_is_zero(&gate.resources) {
        Ok(true) => {}
        Ok(false) => {
            return deny(
                KIND,
                PolicyReason::ExternalCostProtected,
                DecisionStage::EffectEnvelope,
            );
        }
        Err(reason) => return deny(KIND, reason, DecisionStage::EffectEnvelope),
    }

    let mut requirement = risk_approval_requirement(base.spec, true);
    if primary {
        requirement = stricter_requirement(requirement, ApprovalRequirement::ResponsibleUser);
        if gate.approval.is_none() {
            return deny(
                KIND,
                PolicyReason::PrimaryBranchApprovalRequired,
                DecisionStage::Approval,
            );
        }
    }
    if let Some(reason) = approval_reason(
        base.spec,
        gate.approval.as_ref(),
        ApprovalKind::Merge,
        &ApprovalSubject::Merge(gate.subject.clone()),
        requirement,
        r3_checks_required(base.spec),
    ) {
        return deny(KIND, reason, DecisionStage::Approval);
    }
    if let Some(reason) = resource_reason(base.spec, &gate.resource_subject, &gate.resources) {
        return deny(KIND, reason, DecisionStage::Resources);
    }
    PolicyDecision::allow(KIND, PolicyReason::MergeGateAllowed)
}

#[allow(clippy::too_many_lines)]
fn evaluate_memory(gate: &MemoryPromotionGate<'_>) -> PolicyDecision {
    const KIND: DecisionKind = DecisionKind::MemoryPromotion;
    if let Some(decision) = role_context_input_denial(&gate.context, gate.role, KIND) {
        return decision;
    }
    let base = match base_context(&gate.context, KIND) {
        Ok(base) => base,
        Err(decision) => return decision,
    };
    if gate.subject.binding().project_id().as_str() != base.spec.fields().project_id
        || gate.subject.binding().project_snapshot_id() != &base.spec.fields().project_snapshot_id
    {
        return deny(
            KIND,
            PolicyReason::MemoryCrossProject,
            DecisionStage::Project,
        );
    }
    if binding_reason(base.spec, gate.subject.binding()).is_some() {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Project,
        );
    }
    if base.runtime != RuntimeAdmission::Active {
        return deny(
            KIND,
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }
    let Boundary::Known(role) = gate.role else {
        return deny(KIND, PolicyReason::UnknownRole, DecisionStage::RoleAction);
    };
    if role != AgentRole::MemoryReviewer {
        return deny(
            KIND,
            PolicyReason::RoleActionDenied,
            DecisionStage::RoleAction,
        );
    }
    let state = match known_state(&gate.context, KIND) {
        Ok(state) => state,
        Err(decision) => return decision,
    };
    if state != lattice_task_domain::TaskState::Reviewing {
        return deny(KIND, PolicyReason::ActionStateDenied, DecisionStage::State);
    }
    if gate.claims_authority {
        return deny(
            KIND,
            PolicyReason::MemoryCannotAuthorize,
            DecisionStage::Protected,
        );
    }
    if !requested_capability(base.spec, Capability::ProposeMemory) {
        return deny(
            KIND,
            PolicyReason::CapabilityNotRequested,
            DecisionStage::RequestedCapability,
        );
    }
    let Some(review) = gate.review.as_ref() else {
        return deny(
            KIND,
            PolicyReason::MemoryProvenanceRequired,
            DecisionStage::ProviderCapability,
        );
    };
    if review.subject != gate.subject
        || review.reviewer_id.trim().is_empty()
        || !review.immutable_provenance
        || !review.schema_valid
        || !review.fresh
    {
        return deny(
            KIND,
            PolicyReason::MemoryProvenanceRequired,
            DecisionStage::ProviderCapability,
        );
    }
    if !review.review_accepted {
        return deny(
            KIND,
            PolicyReason::MemoryReviewRequired,
            DecisionStage::Approval,
        );
    }
    let review_authority_required = r3_checks_required(base.spec);
    if gate.subject.kind() == MemoryKind::Preference
        && let Some(reason) = approval_reason(
            base.spec,
            gate.preference_user_approval.as_ref(),
            ApprovalKind::Preference,
            &ApprovalSubject::Preference(gate.subject.clone()),
            ApprovalRequirement::ResponsibleUser,
            review_authority_required,
        )
    {
        return deny(
            KIND,
            match reason {
                PolicyReason::ApprovalMissing => PolicyReason::PreferenceUserEvidenceRequired,
                other => other,
            },
            DecisionStage::Approval,
        );
    }
    if gate.subject.kind() != MemoryKind::Preference && review_authority_required {
        return deny(
            KIND,
            PolicyReason::ReviewAuthorityUnavailable,
            DecisionStage::Approval,
        );
    }
    PolicyDecision::allow(KIND, PolicyReason::MemoryPromotionAllowed)
}

#[allow(clippy::too_many_lines)]
fn evaluate_upgrade(gate: &UpgradeGate<'_>) -> PolicyDecision {
    const KIND: DecisionKind = DecisionKind::UpgradeStage;
    if let Some(decision) = role_context_input_denial(&gate.context, gate.role, KIND) {
        return decision;
    }
    if !valid_release_subject(&gate.subject) {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        );
    }
    match (gate.stage, gate.rollback.as_ref()) {
        (UpgradeStage::Rollback, Some(rollback))
            if valid_rollback_subject(&gate.subject, rollback) => {}
        (UpgradeStage::Rollback, _) | (_, Some(_)) => {
            return deny(KIND, PolicyReason::UpgradeStageDenied, DecisionStage::Input);
        }
        _ => {}
    }
    let base = match base_context(&gate.context, KIND) {
        Ok(base) => base,
        Err(decision) => return decision,
    };
    if base.project.receipt.project_class() != ProjectClass::LatticeSystem {
        return deny(
            KIND,
            PolicyReason::ProtectedSurfaceRequired,
            DecisionStage::Project,
        );
    }
    if !upgrade_runtime_allowed(gate.stage, base.runtime) {
        return deny(
            KIND,
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }
    let Boundary::Known(role) = gate.role else {
        return deny(KIND, PolicyReason::UnknownRole, DecisionStage::RoleAction);
    };
    let expected_role = upgrade_role(gate.stage);
    if role != expected_role {
        return deny(
            KIND,
            if expected_role == AgentRole::UpgradeGuardian {
                PolicyReason::GuardianRequired
            } else {
                PolicyReason::RoleActionDenied
            },
            DecisionStage::RoleAction,
        );
    }
    let state = match known_state(&gate.context, KIND) {
        Ok(state) => state,
        Err(decision) => return decision,
    };
    if !upgrade_state_allowed(gate.stage, state) {
        return deny(KIND, PolicyReason::UpgradeStageDenied, DecisionStage::State);
    }
    let delta = gate.subject.delta();
    if delta.schema_migration()
        || !gate.subject.migration_digests().is_empty()
        || gate.rollback.as_ref().is_some_and(|rollback| {
            !rollback.schema_compatible || !rollback.migration_digests.is_empty()
        })
    {
        return deny(
            KIND,
            PolicyReason::UpgradeSchemaMigrationDenied,
            DecisionStage::Protected,
        );
    }
    if gate.stage == UpgradeStage::Activate
        && (delta.policy()
            || delta.constitution()
            || delta.supervisor()
            || delta.credentials()
            || delta.public_exposure()
            || delta.destructive()
            || delta.capability_expansion())
    {
        return deny(
            KIND,
            PolicyReason::UpgradeDeltaProtected,
            DecisionStage::Protected,
        );
    }
    if !requested_capability(base.spec, Capability::ProposeUpgrade) {
        return deny(
            KIND,
            PolicyReason::CapabilityNotRequested,
            DecisionStage::RequestedCapability,
        );
    }

    if gate.stage != UpgradeStage::Propose {
        let Some(evidence) = gate.evidence.as_ref() else {
            return deny(
                KIND,
                PolicyReason::UpgradeStageDenied,
                DecisionStage::ProviderCapability,
            );
        };
        if binding_reason(base.spec, &evidence.binding).is_some()
            || evidence.subject != gate.subject
            || evidence.rollback != gate.rollback
            || !evidence.candidate_immutable
            || !evidence.inactive_slot_verified
            || !gate.subject.schema_compatible()
            || !evidence.fresh
        {
            return deny(
                KIND,
                PolicyReason::UpgradeStageDenied,
                DecisionStage::ProviderCapability,
            );
        }
        if matches!(
            gate.stage,
            UpgradeStage::Activate | UpgradeStage::HealthCanary | UpgradeStage::Rollback
        ) && (!evidence.saga_bound || !evidence.epoch_bound)
        {
            return deny(
                KIND,
                PolicyReason::UpgradeStageDenied,
                DecisionStage::ProviderCapability,
            );
        }
        if gate.stage == UpgradeStage::Rollback && !evidence.prior_slot_verified {
            return deny(
                KIND,
                PolicyReason::UpgradeStageDenied,
                DecisionStage::ProviderCapability,
            );
        }
    }

    if matches!(
        gate.stage,
        UpgradeStage::Shadow
            | UpgradeStage::Activate
            | UpgradeStage::HealthCanary
            | UpgradeStage::Rollback
    ) {
        let Some(guardian) = gate.guardian.as_ref() else {
            return deny(
                KIND,
                PolicyReason::GuardianRequired,
                DecisionStage::Approval,
            );
        };
        let expected_observed_epoch = gate
            .rollback
            .as_ref()
            .map_or(gate.subject.requested_epoch().get(), |rollback| {
                rollback.current_epoch
            });
        if binding_reason(base.spec, &guardian.binding).is_some()
            || guardian.subject != gate.subject
            || guardian.rollback != gate.rollback
            || guardian.origin != Boundary::Known(crate::ApprovalOrigin::GuardianTrustRoot)
            || guardian.runtime.guardian_id().trim().is_empty()
            || guardian.runtime.daemon_instance_id().trim().is_empty()
            || guardian.runtime.observed_epoch().get() != expected_observed_epoch
            || gate.rollback.as_ref().is_some_and(|rollback| {
                rollback.failed_activation.subject.guardian() != &guardian.runtime
            })
            || !guardian.identity_verified
            || !guardian.fresh
            || guardian.user_project_access
            || (matches!(
                gate.stage,
                UpgradeStage::HealthCanary | UpgradeStage::Rollback
            ) && !guardian.reserved_system_stream)
        {
            return deny(
                KIND,
                PolicyReason::GuardianRequired,
                DecisionStage::Approval,
            );
        }
    }

    match gate.stage {
        UpgradeStage::Test => {
            let Some(task_spec_hash) = task_spec_digest(base.spec) else {
                return deny(
                    KIND,
                    PolicyReason::InternalPolicyError,
                    DecisionStage::Approval,
                );
            };
            let subject = ApprovalSubject::Execution {
                task_spec_hash,
                external_cost: None,
            };
            if let Some(reason) = approval_reason(
                base.spec,
                gate.approval.as_ref(),
                ApprovalKind::Execution,
                &subject,
                risk_approval_requirement(base.spec, false),
                r3_checks_required(base.spec),
            ) {
                return deny(KIND, reason, DecisionStage::Approval);
            }
        }
        UpgradeStage::Activate => {
            let Some(guardian) = gate.guardian.as_ref() else {
                return deny(
                    KIND,
                    PolicyReason::GuardianRequired,
                    DecisionStage::Approval,
                );
            };
            let subject =
                ProtectedReleaseSubject::new(gate.subject.clone(), guardian.runtime.clone());
            if let Some(reason) = approval_reason(
                base.spec,
                gate.approval.as_ref(),
                ApprovalKind::ProtectedRelease,
                &ApprovalSubject::ProtectedRelease(Box::new(subject)),
                ApprovalRequirement::ProtectedGuardian,
                true,
            ) {
                return deny(KIND, reason, DecisionStage::Approval);
            }
        }
        UpgradeStage::Propose
        | UpgradeStage::Shadow
        | UpgradeStage::HealthCanary
        | UpgradeStage::Rollback => {}
    }

    PolicyDecision::allow(KIND, PolicyReason::UpgradeStageAllowed)
}

#[allow(clippy::too_many_lines)]
fn evaluate_recovery(gate: &RecoveryGate<'_>) -> PolicyDecision {
    const KIND: DecisionKind = DecisionKind::RecoveryGate;
    if let Some(decision) = role_context_input_denial(&gate.context, gate.role, KIND) {
        return decision;
    }
    if !valid_recovery_subject(&gate.subject) {
        return deny(
            KIND,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        );
    }
    let base = match base_context(&gate.context, KIND) {
        Ok(base) => base,
        Err(decision) => return decision,
    };
    let Some(authority) = gate.authority.as_ref() else {
        return deny(
            KIND,
            PolicyReason::RecoveryAuthorityRequired,
            DecisionStage::Project,
        );
    };
    if let Some(reason) = binding_reason(base.spec, &authority.binding) {
        return deny(KIND, reason, DecisionStage::Project);
    }
    if authority.subject != gate.subject {
        return deny(
            KIND,
            PolicyReason::RecoveryAuthorityMismatch,
            DecisionStage::Project,
        );
    }

    let (observed_admission, expected_role, expected_owner, expected_producer_id, valid_transition) =
        match &gate.subject {
            RecoverySubject::Normal(subject) => (
                subject.observed_admission,
                AgentRole::LatticePm,
                RecoveryOwner::RuntimeSupervisor,
                subject.runtime_supervisor_id.as_str(),
                valid_normal_recovery_transition(
                    subject.observed_admission,
                    subject.target_admission,
                ),
            ),
            RecoverySubject::GuardianRelease(subject) => {
                if base.project.receipt.project_class() != ProjectClass::LatticeSystem
                    || !authority.reserved_system_stream
                    || authority.user_project_access
                {
                    return deny(
                        KIND,
                        PolicyReason::ProtectedSurfaceRequired,
                        DecisionStage::Project,
                    );
                }
                (
                    subject.observed_admission,
                    AgentRole::UpgradeGuardian,
                    RecoveryOwner::UpgradeGuardian,
                    subject.guardian.guardian_id(),
                    valid_guardian_recovery_transition(
                        subject.observed_admission,
                        subject.target_admission,
                    ),
                )
            }
        };
    if base.runtime != observed_admission || !valid_transition {
        return deny(
            KIND,
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }
    let Boundary::Known(role) = gate.role else {
        return deny(KIND, PolicyReason::UnknownRole, DecisionStage::RoleAction);
    };
    if role != expected_role {
        return deny(
            KIND,
            if expected_role == AgentRole::UpgradeGuardian {
                PolicyReason::GuardianRequired
            } else {
                PolicyReason::RoleActionDenied
            },
            DecisionStage::RoleAction,
        );
    }
    let state = match known_state(&gate.context, KIND) {
        Ok(state) => state,
        Err(decision) => return decision,
    };
    if !matches!(
        state,
        lattice_task_domain::TaskState::Stopping
            | lattice_task_domain::TaskState::Blocked
            | lattice_task_domain::TaskState::Failed
    ) {
        return deny(KIND, PolicyReason::ActionStateDenied, DecisionStage::State);
    }
    if !requested_capability(base.spec, Capability::StopRuntime) {
        return deny(
            KIND,
            PolicyReason::CapabilityNotRequested,
            DecisionStage::RequestedCapability,
        );
    }
    if authority.owner != Boundary::Known(expected_owner)
        || authority.producer_id != expected_producer_id
        || !authority.identity_verified
        || !authority.fresh
    {
        return deny(
            KIND,
            PolicyReason::RecoveryAuthorityMismatch,
            DecisionStage::ProviderCapability,
        );
    }
    PolicyDecision::allow(KIND, PolicyReason::RecoveryGateAllowed)
}

#[allow(clippy::too_many_lines)]
fn evaluate_protected_change(gate: &ProtectedChangeGate<'_>) -> PolicyDecision {
    const KIND: DecisionKind = DecisionKind::ProtectedChange;
    if let Some(decision) = role_context_input_denial(&gate.context, gate.role, KIND) {
        return decision;
    }
    let base = match base_context(&gate.context, KIND) {
        Ok(base) => base,
        Err(decision) => return decision,
    };
    if gate.subject.class() == ProtectedChangeClass::CoreReleaseActivation
        && base.project.receipt.project_class() != ProjectClass::LatticeSystem
    {
        return deny(
            KIND,
            PolicyReason::ProtectedSurfaceRequired,
            DecisionStage::Project,
        );
    }
    // This subject authorizes an immutable protected-change intent only.
    // UpgradeStage separately enforces DRAINING/CANARY activation admission.
    let expected_runtime = RuntimeAdmission::Active;
    if base.runtime != expected_runtime {
        return deny(
            KIND,
            PolicyReason::RuntimeAdmissionDenied,
            DecisionStage::Runtime,
        );
    }
    let Boundary::Known(role) = gate.role else {
        return deny(KIND, PolicyReason::UnknownRole, DecisionStage::RoleAction);
    };
    let expected_role = match gate.subject.class() {
        ProtectedChangeClass::PrimaryBranchMerge => AgentRole::Integrator,
        ProtectedChangeClass::CoreReleaseActivation => AgentRole::UpgradeGuardian,
        _ => AgentRole::LatticePm,
    };
    if role != expected_role {
        return deny(
            KIND,
            if expected_role == AgentRole::UpgradeGuardian {
                PolicyReason::GuardianRequired
            } else {
                PolicyReason::RoleActionDenied
            },
            DecisionStage::RoleAction,
        );
    }
    let state = match known_state(&gate.context, KIND) {
        Ok(state) => state,
        Err(decision) => return decision,
    };
    let state_allowed = match gate.subject.class() {
        ProtectedChangeClass::PrimaryBranchMerge => {
            state == lattice_task_domain::TaskState::AwaitingMergeApproval
        }
        ProtectedChangeClass::CoreReleaseActivation => {
            state == lattice_task_domain::TaskState::Reviewing
        }
        _ => matches!(
            state,
            lattice_task_domain::TaskState::AwaitingExecutionApproval
                | lattice_task_domain::TaskState::Reviewing
        ),
    };
    if !state_allowed {
        return deny(KIND, PolicyReason::ActionStateDenied, DecisionStage::State);
    }

    match gate.subject.class() {
        ProtectedChangeClass::PrimaryBranchMerge => {
            if !requested_capability(base.spec, Capability::GitIntegrate) {
                return deny(
                    KIND,
                    PolicyReason::CapabilityNotRequested,
                    DecisionStage::RequestedCapability,
                );
            }
        }
        ProtectedChangeClass::CoreReleaseActivation => {
            if !requested_capability(base.spec, Capability::ProposeUpgrade) {
                return deny(
                    KIND,
                    PolicyReason::CapabilityNotRequested,
                    DecisionStage::RequestedCapability,
                );
            }
        }
        ProtectedChangeClass::ProductionDeployment
            if base.spec.fields().deployment_policy != DeploymentPolicy::Authorized =>
        {
            return deny(
                KIND,
                PolicyReason::DeploymentDenied,
                DecisionStage::EffectEnvelope,
            );
        }
        _ => {}
    }

    let (requirement, checks) = match gate.subject.class() {
        ProtectedChangeClass::CoreReleaseActivation => (
            stricter_requirement(
                base.spec.fields().approval_requirements.protected_release,
                ApprovalRequirement::ProtectedGuardian,
            ),
            true,
        ),
        ProtectedChangeClass::PrimaryBranchMerge => (
            stricter_requirement(
                risk_approval_requirement(base.spec, true),
                ApprovalRequirement::ResponsibleUser,
            ),
            r3_checks_required(base.spec),
        ),
        _ => (
            stricter_requirement(
                risk_approval_requirement(base.spec, false),
                ApprovalRequirement::ResponsibleUser,
            ),
            r3_checks_required(base.spec),
        ),
    };
    if gate.subject.class() == ProtectedChangeClass::PrimaryBranchMerge && gate.approval.is_none() {
        return deny(
            KIND,
            PolicyReason::PrimaryBranchApprovalRequired,
            DecisionStage::Approval,
        );
    }
    if let Some(reason) = approval_reason(
        base.spec,
        gate.approval.as_ref(),
        ApprovalKind::ProtectedChange,
        &ApprovalSubject::ProtectedChange(gate.subject.clone()),
        requirement,
        checks,
    ) {
        return deny(KIND, reason, DecisionStage::Approval);
    }
    PolicyDecision::allow(KIND, PolicyReason::ProtectedChangeAllowed)
}

fn requested_capability(spec: &TaskSpec, capability: Capability) -> bool {
    spec.fields()
        .requested_capabilities
        .iter()
        .any(|request| request.capability == capability && request.contract_version == "1")
}

fn task_spec_digest(spec: &TaskSpec) -> Option<ContentDigest> {
    ContentDigest::from_sha256(spec.spec_hash().to_hex()).ok()
}

fn recovery_effect_is_bounded(gate: &AgentActionGate<'_>) -> bool {
    gate.network == NetworkIntent::None
        && gate.deployment == DeploymentIntent::None
        && gate.resources.as_ref().is_none_or(|resources| {
            resources.receipt.request().requested_agents() == 0
                && resources.receipt.request().requested_implementers() == 0
                && resources.receipt.request().requested_attempts() == 0
                && resources.receipt.request().requested_model_calls() == 0
                && requested_cost_is_zero(resources) == Ok(true)
        })
}

fn valid_release_subject(subject: &crate::ReleaseSubject) -> bool {
    [
        subject.activation_id(),
        subject.saga_id(),
        subject.release_id(),
        subject.release_revision(),
        subject.source_release_id(),
        subject.source_slot_id(),
        subject.target_slot_id(),
    ]
    .iter()
    .all(|value| !value.trim().is_empty())
        && subject.source_slot_id() != subject.target_slot_id()
        && valid_git_object_id(subject.source_commit())
        && !subject.binary_digests().is_empty()
        && subject.requested_epoch().get() > 0
}

fn valid_rollback_subject(
    release: &crate::ReleaseSubject,
    rollback: &crate::RollbackSubject,
) -> bool {
    [
        rollback.rollback_id.as_str(),
        rollback.failed_activation_id.as_str(),
        rollback.saga_id.as_str(),
        rollback.current_release_id.as_str(),
        rollback.current_slot_id.as_str(),
        rollback.target_release_id.as_str(),
        rollback.target_slot_id.as_str(),
    ]
    .iter()
    .all(|value| !value.trim().is_empty())
        && rollback.failed_activation_id == release.activation_id()
        && rollback.saga_id == release.saga_id()
        && rollback.failed_activation.subject.release() == release
        && !rollback.failed_activation.approval_id.trim().is_empty()
        && !rollback
            .failed_activation
            .activation_claim_id
            .trim()
            .is_empty()
        && rollback.current_release_id == release.release_id()
        && &rollback.current_manifest_digest == release.manifest_digest()
        && rollback.current_slot_id == release.target_slot_id()
        && rollback.current_epoch == release.requested_epoch().get()
        && rollback.target_release_id == release.source_release_id()
        && &rollback.target_manifest_digest == release.source_manifest_digest()
        && rollback.target_slot_id == release.source_slot_id()
        && rollback.target_slot_id != rollback.current_slot_id
        && rollback.requested_epoch > rollback.current_epoch
}

fn valid_recovery_subject(subject: &RecoverySubject) -> bool {
    match subject {
        RecoverySubject::Normal(subject) => {
            !subject.runtime_supervisor_id.trim().is_empty()
                && !subject.daemon_instance_id.trim().is_empty()
                && !subject.effect_claim_id.trim().is_empty()
                && subject
                    .worktree_id
                    .as_ref()
                    .is_none_or(|value| !value.trim().is_empty())
                && subject.expected_daemon_epoch > 0
                && match &subject.resolution {
                    NormalRecoveryResolution::EffectOutcome { outcome, .. } => {
                        matches!(outcome, Boundary::Known(_))
                    }
                    NormalRecoveryResolution::HolderDeath {
                        holder_daemon_instance_id,
                        holder_process_id,
                        holder_process_start_identity,
                    } => {
                        holder_daemon_instance_id == &subject.daemon_instance_id
                            && *holder_process_id > 0
                            && !holder_process_start_identity.trim().is_empty()
                    }
                    NormalRecoveryResolution::ReplacedLeadership {
                        replaced_daemon_instance_id,
                        replaced_epoch,
                        active_daemon_instance_id,
                        active_epoch,
                    } => {
                        replaced_daemon_instance_id == &subject.daemon_instance_id
                            && *replaced_epoch == subject.expected_daemon_epoch
                            && !active_daemon_instance_id.trim().is_empty()
                            && active_daemon_instance_id != replaced_daemon_instance_id
                            && *active_epoch > *replaced_epoch
                    }
                }
        }
        RecoverySubject::GuardianRelease(subject) => {
            valid_release_subject(&subject.release)
                && !subject.guardian.guardian_id().trim().is_empty()
                && !subject.guardian.daemon_instance_id().trim().is_empty()
                && subject.guardian.observed_epoch() == subject.release.requested_epoch()
                && !subject.effect_claim_id.trim().is_empty()
                && subject.release.schema_compatible()
                && subject.release.migration_digests().is_empty()
                && !release_has_protected_activation_delta(&subject.release)
                && subject.resolution.activation.subject.release() == &subject.release
                && subject.resolution.activation.subject.guardian() == &subject.guardian
                && !subject.resolution.activation.approval_id.trim().is_empty()
                && !subject
                    .resolution
                    .activation
                    .activation_claim_id
                    .trim()
                    .is_empty()
                && !subject.resolution.active_release_id.trim().is_empty()
                && !subject.resolution.active_slot_id.trim().is_empty()
                && subject.resolution.active_epoch > 0
                && match subject.resolution.outcome {
                    Boundary::Known(GuardianSagaOutcome::ActivationFinalized) => {
                        subject.resolution.active_release_id == subject.release.release_id()
                            && subject.resolution.active_manifest_digest
                                == *subject.release.manifest_digest()
                            && subject.resolution.active_slot_id == subject.release.target_slot_id()
                            && subject.resolution.active_epoch
                                == subject.release.requested_epoch().get()
                    }
                    Boundary::Known(GuardianSagaOutcome::RollbackFinalized) => {
                        subject.resolution.active_release_id == subject.release.source_release_id()
                            && subject.resolution.active_manifest_digest
                                == *subject.release.source_manifest_digest()
                            && subject.resolution.active_slot_id == subject.release.source_slot_id()
                            && subject.resolution.active_epoch
                                > subject.release.requested_epoch().get()
                    }
                    Boundary::Unknown => false,
                }
        }
    }
}

const fn valid_normal_recovery_transition(
    observed: RuntimeAdmission,
    target: RuntimeAdmission,
) -> bool {
    matches!(
        (observed, target),
        (
            RuntimeAdmission::Draining | RuntimeAdmission::ReconciliationRequired,
            RuntimeAdmission::Stopped
        )
    )
}

const fn valid_guardian_recovery_transition(
    observed: RuntimeAdmission,
    target: RuntimeAdmission,
) -> bool {
    matches!(
        (observed, target),
        (
            RuntimeAdmission::Stopped | RuntimeAdmission::ReconciliationRequired,
            RuntimeAdmission::Active
        )
    )
}

const fn release_has_protected_activation_delta(release: &crate::ReleaseSubject) -> bool {
    let delta = release.delta();
    delta.schema_migration()
        || delta.policy()
        || delta.constitution()
        || delta.supervisor()
        || delta.credentials()
        || delta.public_exposure()
        || delta.destructive()
        || delta.capability_expansion()
}

fn context_input_denial(
    context: &crate::TaskContext<'_>,
    kind: DecisionKind,
) -> Option<PolicyDecision> {
    if context.task_spec.is_none() {
        return Some(deny(
            kind,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        ));
    }
    if matches!(context.state, Boundary::Unknown) {
        return Some(deny(kind, PolicyReason::UnknownState, DecisionStage::Input));
    }
    if matches!(context.runtime_admission, Boundary::Unknown) {
        return Some(deny(
            kind,
            PolicyReason::UnknownRuntimeAdmission,
            DecisionStage::Input,
        ));
    }
    None
}

fn role_context_input_denial<T>(
    context: &crate::TaskContext<'_>,
    role: Boundary<T>,
    kind: DecisionKind,
) -> Option<PolicyDecision>
where
    T: Copy,
{
    if context.task_spec.is_none() {
        return Some(deny(
            kind,
            PolicyReason::InvalidDecisionSubject,
            DecisionStage::Input,
        ));
    }
    if matches!(role, Boundary::Unknown) {
        return Some(deny(kind, PolicyReason::UnknownRole, DecisionStage::Input));
    }
    if matches!(context.state, Boundary::Unknown) {
        return Some(deny(kind, PolicyReason::UnknownState, DecisionStage::Input));
    }
    if matches!(context.runtime_admission, Boundary::Unknown) {
        return Some(deny(
            kind,
            PolicyReason::UnknownRuntimeAdmission,
            DecisionStage::Input,
        ));
    }
    None
}

const fn deny(kind: DecisionKind, reason: PolicyReason, stage: DecisionStage) -> PolicyDecision {
    PolicyDecision::deny(kind, reason, stage)
}

const fn invalid_reason(failure: PolicyInputFailure) -> PolicyReason {
    match failure {
        PolicyInputFailure::UnknownRole => PolicyReason::UnknownRole,
        PolicyInputFailure::UnknownAction => PolicyReason::UnknownAction,
        PolicyInputFailure::UnknownState => PolicyReason::UnknownState,
        PolicyInputFailure::UnknownRuntimeAdmission => PolicyReason::UnknownRuntimeAdmission,
        PolicyInputFailure::UnknownCapability => PolicyReason::UnknownCapability,
        PolicyInputFailure::UnknownAuthority => PolicyReason::UnknownAuthority,
        PolicyInputFailure::MalformedSubject => PolicyReason::InvalidDecisionSubject,
    }
}
