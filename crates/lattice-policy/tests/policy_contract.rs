use lattice_contracts::{
    CONTRACT_VERSION, ContentDigest, GitRefIdentity, ProjectAuthorityReceipt, ProjectClass,
    ProjectId, ProjectLifecycle, ProjectSnapshotId, RuntimeKind, TaskId,
};
use lattice_policy::{
    AgentActionGate, AgentRole, Boundary, DecisionKind, DecisionStage, DecisionSubject,
    DeploymentIntent, ExecutionGate, ManagedExecutionBindingFact, NetworkIntent,
    POLICY_CONTRACT_VERSION, PolicyAction, PolicyInputFailure, PolicyReason, ProjectAuthorityFact,
    RuntimeAdmission, SubjectBinding, TaskContext, evaluate,
    evaluate_managed_execution_gate_with_evidence,
};
use lattice_task_domain::{
    AcceptanceCriterion, ApprovalRequirement, ApprovalRequirements, Capability, CapabilityRequest,
    DeploymentPolicy, EvidenceType, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    ScopeOperation, TaskBudget, TaskScope, TaskSpec, TaskSpecInput, TaskState,
};

#[test]
fn invalid_boundary_input_is_a_typed_default_deny() {
    let decision = evaluate(DecisionSubject::Invalid(PolicyInputFailure::UnknownAction));

    assert!(!decision.allowed());
    assert_eq!(decision.reason(), PolicyReason::UnknownAction);
    assert_eq!(decision.reason().code(), "UNKNOWN_ACTION");
    assert_eq!(
        decision.evidence().contract_version(),
        POLICY_CONTRACT_VERSION
    );
    assert_eq!(decision.evidence().subject(), DecisionKind::Invalid);
    assert_eq!(decision.evidence().checked_through(), DecisionStage::Input);
}

#[test]
fn missing_task_spec_cannot_inherit_matching_undefined_fields() {
    let spec = task_spec();
    let mut gate = read_gate(&spec);
    gate.context.task_spec = None;

    let decision = evaluate(DecisionSubject::AgentAction(gate));

    assert!(!decision.allowed());
    assert_eq!(decision.reason(), PolicyReason::InvalidDecisionSubject);
    assert_eq!(decision.evidence().checked_through(), DecisionStage::Input);
}

#[test]
fn complete_exact_read_subject_is_allowed() {
    let spec = task_spec();

    let decision = evaluate(DecisionSubject::AgentAction(read_gate(&spec)));

    assert!(decision.allowed());
    assert_eq!(decision.reason(), PolicyReason::AgentActionAllowed);
    assert_eq!(
        decision.evidence().checked_through(),
        DecisionStage::Complete
    );
}

#[test]
fn stale_registry_receipt_head_is_denied_before_action_authority() {
    let spec = task_spec();
    let mut gate = read_gate(&spec);
    let stale_fact = gate.context.project.as_mut().expect("project");
    stale_fact.current_head =
        authority_receipt(&spec, ProjectLifecycle::Active, RuntimeKind::Fake, 2, '9').head();

    let decision = evaluate(DecisionSubject::AgentAction(gate));

    assert!(!decision.allowed());
    assert_eq!(decision.reason(), PolicyReason::ProjectAuthorityStale);
}

#[test]
fn genuine_current_head_rejects_receipt_security_field_substitution() {
    let spec = task_spec();
    let genuine = authority_receipt(&spec, ProjectLifecycle::Active, RuntimeKind::Fake, 1, '8');
    let substitutions = [
        authority_receipt_with_security_fields(
            &spec,
            ProjectLifecycle::Suspended,
            RuntimeKind::Fake,
            ProjectClass::UserProject,
            "refs/heads/main",
            '7',
        ),
        authority_receipt_with_security_fields(
            &spec,
            ProjectLifecycle::Active,
            RuntimeKind::Live,
            ProjectClass::UserProject,
            "refs/heads/main",
            '7',
        ),
        authority_receipt_with_security_fields(
            &spec,
            ProjectLifecycle::Active,
            RuntimeKind::Fake,
            ProjectClass::LatticeSystem,
            "refs/heads/main",
            '7',
        ),
        authority_receipt_with_security_fields(
            &spec,
            ProjectLifecycle::Active,
            RuntimeKind::Fake,
            ProjectClass::UserProject,
            "refs/heads/other",
            '7',
        ),
        authority_receipt_with_security_fields(
            &spec,
            ProjectLifecycle::Active,
            RuntimeKind::Fake,
            ProjectClass::UserProject,
            "refs/heads/main",
            '6',
        ),
    ];

    for receipt in substitutions {
        let mut gate = read_gate(&spec);
        gate.context.project = Some(ProjectAuthorityFact {
            binding: binding(&spec),
            receipt,
            current_head: genuine.head(),
        });

        let decision = evaluate(DecisionSubject::AgentAction(gate));

        assert!(!decision.allowed());
        assert_eq!(decision.reason(), PolicyReason::ProjectAuthorityStale);
    }
}

#[test]
fn closed_registry_lifecycle_denies_suspended_and_drifted_projects() {
    for (lifecycle, reason) in [
        (ProjectLifecycle::Suspended, PolicyReason::ProjectInactive),
        (
            ProjectLifecycle::ReconciliationRequired,
            PolicyReason::ProjectDrifted,
        ),
    ] {
        let spec = task_spec();
        let mut gate = read_gate(&spec);
        gate.context.project = Some(project_fact(&spec, lifecycle, RuntimeKind::Fake));

        let decision = evaluate(DecisionSubject::AgentAction(gate));

        assert!(!decision.allowed());
        assert_eq!(decision.reason(), reason);
    }
}

#[test]
fn fake_registry_receipt_cannot_masquerade_as_live_task_authority() {
    let spec = task_spec_with_runtime(RuntimeProfile::Codex);
    let gate = read_gate(&spec);

    let decision = evaluate(DecisionSubject::AgentAction(gate));

    assert!(!decision.allowed());
    assert_eq!(decision.reason(), PolicyReason::RuntimeKindMismatch);
}

#[test]
fn managed_execution_evidence_seals_exact_binding_and_denies_spec_drift() {
    let spec = task_spec();
    let subject_binding = binding(&spec);
    let execution_binding = ManagedExecutionBindingFact {
        task_ref: ContentDigest::from_sha256("1".repeat(64)).expect("task ref"),
        successor_stream_id: ContentDigest::from_sha256("2".repeat(64)).expect("successor"),
        task_spec_digest: subject_binding.task_spec_digest().clone(),
        approval_subject_digest: ContentDigest::from_sha256("3".repeat(64))
            .expect("approval subject"),
        budget_digest: ContentDigest::from_sha256("4".repeat(64)).expect("budget"),
    };
    let evidence = evaluate_managed_execution_gate_with_evidence(
        ExecutionGate {
            context: TaskContext {
                task_spec: Some(&spec),
                project: Some(project_fact(
                    &spec,
                    ProjectLifecycle::Active,
                    RuntimeKind::Fake,
                )),
                state: Boundary::Known(TaskState::AwaitingExecutionApproval),
                runtime_admission: Boundary::Known(RuntimeAdmission::Active),
            },
            approval: None,
        },
        execution_binding.clone(),
    );
    assert!(evidence.decision().allowed());
    assert_eq!(
        evidence.managed_execution_binding(),
        Some(&execution_binding)
    );

    let mut drifted = execution_binding;
    drifted.task_spec_digest = ContentDigest::from_sha256("5".repeat(64)).expect("drift");
    let denied = evaluate_managed_execution_gate_with_evidence(
        ExecutionGate {
            context: TaskContext {
                task_spec: Some(&spec),
                project: Some(project_fact(
                    &spec,
                    ProjectLifecycle::Active,
                    RuntimeKind::Fake,
                )),
                state: Boundary::Known(TaskState::AwaitingExecutionApproval),
                runtime_admission: Boundary::Known(RuntimeAdmission::Active),
            },
            approval: None,
        },
        drifted,
    );
    assert!(!denied.decision().allowed());
    assert_eq!(
        denied.decision().reason(),
        PolicyReason::InvalidDecisionSubject
    );
}

fn read_gate(spec: &TaskSpec) -> AgentActionGate<'_> {
    AgentActionGate {
        context: TaskContext {
            task_spec: Some(spec),
            project: Some(project_fact(
                spec,
                ProjectLifecycle::Active,
                RuntimeKind::Fake,
            )),
            state: Boundary::Known(TaskState::Draft),
            runtime_admission: Boundary::Known(RuntimeAdmission::Active),
        },
        role: Boundary::Known(AgentRole::Planner),
        action: Boundary::Known(PolicyAction::ReadRepository),
        actor_id: "planner-1".to_owned(),
        approval: None,
        provider_capability: None,
        external_cost: None,
        writer_subject: None,
        writer: None,
        resource_subject: None,
        resources: None,
        network: NetworkIntent::None,
        deployment: DeploymentIntent::None,
    }
}

fn project_fact(
    spec: &TaskSpec,
    lifecycle: ProjectLifecycle,
    runtime: RuntimeKind,
) -> ProjectAuthorityFact {
    let receipt = authority_receipt(spec, lifecycle, runtime, 1, '8');
    ProjectAuthorityFact {
        binding: binding(spec),
        current_head: receipt.head(),
        receipt,
    }
}

fn authority_receipt(
    spec: &TaskSpec,
    lifecycle: ProjectLifecycle,
    runtime: RuntimeKind,
    revision: u64,
    receipt_byte: char,
) -> ProjectAuthorityReceipt {
    authority_receipt_with_fields(
        spec,
        lifecycle,
        runtime,
        revision,
        ProjectClass::UserProject,
        "refs/heads/main",
        '7',
        receipt_byte,
    )
}

fn authority_receipt_with_security_fields(
    spec: &TaskSpec,
    lifecycle: ProjectLifecycle,
    runtime: RuntimeKind,
    project_class: ProjectClass,
    primary_branch: &str,
    observation_byte: char,
) -> ProjectAuthorityReceipt {
    authority_receipt_with_fields(
        spec,
        lifecycle,
        runtime,
        1,
        project_class,
        primary_branch,
        observation_byte,
        '8',
    )
}

#[allow(clippy::too_many_arguments)]
fn authority_receipt_with_fields(
    spec: &TaskSpec,
    lifecycle: ProjectLifecycle,
    runtime: RuntimeKind,
    revision: u64,
    project_class: ProjectClass,
    primary_branch: &str,
    observation_byte: char,
    receipt_byte: char,
) -> ProjectAuthorityReceipt {
    ProjectAuthorityReceipt::new(
        CONTRACT_VERSION,
        "lattice-project-registry",
        "1.0",
        runtime,
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        revision,
        lifecycle,
        project_class,
        GitRefIdentity::new(
            primary_branch,
            ContentDigest::from_sha256("0".repeat(64)).expect("Git ref identity"),
        )
        .expect("fully qualified local ref"),
        ContentDigest::from_sha256(observation_byte.to_string().repeat(64)).expect("observation"),
        ContentDigest::from_sha256(receipt_byte.to_string().repeat(64)).expect("receipt"),
    )
    .expect("authority receipt")
}

fn binding(spec: &TaskSpec) -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        spec.fields().task_id.clone(),
        spec.fields().revision.clone(),
        ContentDigest::from_sha256(spec.spec_hash().to_hex()).expect("Task Spec SHA-256"),
    )
    .expect("binding")
}

fn task_spec() -> TaskSpec {
    task_spec_with_runtime(RuntimeProfile::Fake)
}

fn task_spec_with_runtime(runtime_profile: RuntimeProfile) -> TaskSpec {
    TaskSpec::new(TaskSpecInput {
        schema_version: "2.1".to_owned(),
        task_id: TaskId::new("TASK-011-POLICY").expect("task ID"),
        revision: "1".to_owned(),
        created_at: "2026-07-29T08:00:00Z".to_owned(),
        created_by: "owner".to_owned(),
        project_id: "general-ai-platform".to_owned(),
        project_snapshot_id: ProjectSnapshotId::new("snapshot-policy-1").expect("snapshot"),
        base_ref: "feature/policy".to_owned(),
        base_commit_id: "a".repeat(40),
        goal: "Evaluate policy deterministically.".to_owned(),
        non_goals: vec!["Do not perform I/O.".to_owned()],
        risk_class: RiskClass::R0,
        depends_on: vec![],
        scope: TaskScope {
            allowed_paths: vec!["crates/lattice-policy/**".to_owned()],
            forbidden_paths: vec![".git/**".to_owned()],
            allowed_operations: vec![ScopeOperation::Modify],
        },
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-01".to_owned(),
            description: "Policy tests pass.".to_owned(),
            evidence_type: EvidenceType::Test,
            expected_result: "Exit zero.".to_owned(),
        }],
        verification_commands: vec!["cargo test -p lattice-policy".to_owned()],
        required_checks: vec![RequiredCheck::Test],
        requested_capabilities: vec![CapabilityRequest {
            capability: Capability::ReadRepository,
            contract_version: "1".to_owned(),
        }],
        budget: TaskBudget {
            accounting_currency: "USD".to_owned(),
            max_agents: "4".to_owned(),
            max_duration_seconds: "1800".to_owned(),
            max_attempts: "2".to_owned(),
            max_model_calls: "0".to_owned(),
            max_external_cost: "0".to_owned(),
        },
        runtime_profile,
        network_policy: NetworkPolicy::Deny,
        deployment_policy: DeploymentPolicy::Deny,
        approval_requirements: ApprovalRequirements {
            execution: ApprovalRequirement::NotRequired,
            merge: ApprovalRequirement::ResponsibleUser,
            protected_release: ApprovalRequirement::ProtectedGuardian,
        },
    })
    .expect("valid task spec")
}
