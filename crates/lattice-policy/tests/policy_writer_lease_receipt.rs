use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, ContentDigest, DaemonEpoch, FencingToken, GitRefIdentity,
    HolderProcessId, PROJECT_AUTHORITY_PRODUCER_ID, PROJECT_AUTHORITY_PRODUCER_VERSION,
    ProjectAuthorityReceipt, ProjectClass, ProjectId, ProjectLifecycle, ProjectSnapshotId,
    RuntimeAdmissionMode, RuntimeKind, TaskId,
};
use lattice_policy::{
    AgentActionGate, AgentRole, Boundary, DecisionStage, DecisionSubject, DeploymentIntent,
    NetworkIntent, PolicyAction, PolicyReason, ProjectAuthorityFact, RuntimeAdmission,
    SubjectBinding, TaskContext, WriterLeaseFact, WriterLeaseSubject, evaluate,
};
use lattice_task_domain::{
    AcceptanceCriterion, ApprovalRequirement, ApprovalRequirements, Capability, CapabilityRequest,
    DeploymentPolicy, EvidenceType, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    ScopeOperation, TaskBudget, TaskScope, TaskSpec, TaskSpecInput, TaskState,
};
use lattice_writer_lease::test_support::observation;
use lattice_writer_lease::{
    AcquireClaim, AcquireCommand, FakeWriterLease, HeartbeatCommand, MarkSuspectCommand,
    WriterLeaseCommand,
};

#[test]
fn policy_writer_fact_composes_with_the_fake_owners_independent_current_head() {
    let spec = task_spec();
    let project = ProjectId::new("policy-writer-project").expect("project");
    let claim = acquire_claim(&spec, project.clone());
    let expected_subject = writer_subject(&claim, FencingToken::new(1).expect("first fence"));
    let mut owner = FakeWriterLease::new();
    owner
        .execute(WriterLeaseCommand::Acquire(AcquireCommand {
            command_id: "acquire-policy-writer".to_owned(),
            expected_head: None,
            claim,
            observation: observation(RuntimeAdmissionMode::Active, "2026-07-29T00:00:00Z"),
            expires_at: "2026-07-29T00:10:00Z".to_owned(),
        }))
        .expect("owner acquire");

    assert_active_release_and_head_rotation(&spec, &expected_subject, &mut owner, &project);
    assert_suspect_release_matrix(&spec, &expected_subject, &mut owner, &project);
}

fn assert_active_release_and_head_rotation(
    spec: &TaskSpec,
    expected_subject: &WriterLeaseSubject,
    owner: &mut FakeWriterLease,
    project: &ProjectId,
) {
    let initial_fact = WriterLeaseFact {
        receipt: owner.current_receipt(project).expect("owner receipt"),
        current_head: owner.current_head(project),
    };
    assert!(
        evaluate(DecisionSubject::AgentAction(release_gate(
            spec,
            expected_subject.to_owned(),
            initial_fact.clone(),
            "implementer-1",
            RuntimeAdmission::Draining,
        )))
        .allowed(),
        "an ACTIVE owner receipt remains releasable after current admission enters DRAINING"
    );

    let historical = initial_fact.receipt;
    owner
        .execute(WriterLeaseCommand::Heartbeat(HeartbeatCommand {
            command_id: "heartbeat-policy-writer".to_owned(),
            project_id: project.clone(),
            expected_head: owner.current_head(project).expect("old current head"),
            observation: observation(RuntimeAdmissionMode::Active, "2026-07-29T00:05:00Z"),
            expires_at: "2026-07-29T00:15:00Z".to_owned(),
        }))
        .expect("owner heartbeat");

    let stale = evaluate(DecisionSubject::AgentAction(release_gate(
        spec,
        expected_subject.to_owned(),
        WriterLeaseFact {
            receipt: historical,
            current_head: owner.current_head(project),
        },
        "implementer-1",
        RuntimeAdmission::Draining,
    )));
    assert!(!stale.allowed());
    assert_eq!(stale.reason(), PolicyReason::WriterLeaseNotCurrent);
    assert_eq!(stale.evidence().checked_through(), DecisionStage::Writer);

    let heartbeat_fact = current_fact(owner, project);
    assert!(
        evaluate(DecisionSubject::AgentAction(release_gate(
            spec,
            expected_subject.to_owned(),
            heartbeat_fact,
            "implementer-1",
            RuntimeAdmission::Draining,
        )))
        .allowed(),
        "the new owner receipt and independently queried current head must pass"
    );
}

fn assert_suspect_release_matrix(
    spec: &TaskSpec,
    expected_subject: &WriterLeaseSubject,
    owner: &mut FakeWriterLease,
    project: &ProjectId,
) {
    owner
        .execute(WriterLeaseCommand::MarkSuspect(MarkSuspectCommand {
            command_id: "suspect-policy-writer".to_owned(),
            project_id: project.clone(),
            expected_head: owner.current_head(project).expect("heartbeat head"),
            observation: observation(RuntimeAdmissionMode::Draining, "2026-07-29T00:15:00Z"),
        }))
        .expect("owner marks expired lease suspect");
    let suspect_fact = current_fact(owner, project);

    for runtime in [RuntimeAdmission::Active, RuntimeAdmission::Draining] {
        let decision = evaluate(DecisionSubject::AgentAction(release_gate(
            spec,
            expected_subject.to_owned(),
            suspect_fact.clone(),
            "implementer-1",
            runtime,
        )));
        assert!(
            decision.allowed(),
            "exact current SUSPECT holder must remain releasable in {runtime:?}: {:?}",
            decision.reason()
        );
    }

    let non_holder = evaluate(DecisionSubject::AgentAction(release_gate(
        spec,
        expected_subject.to_owned(),
        suspect_fact.clone(),
        "implementer-2",
        RuntimeAdmission::Draining,
    )));
    assert!(!non_holder.allowed());
    assert_eq!(
        non_holder.reason(),
        PolicyReason::WriterLeaseSubjectMismatch
    );

    let mut substituted_subject = expected_subject.clone();
    "substituted-lease".clone_into(&mut substituted_subject.lease_id);
    let substituted = evaluate(DecisionSubject::AgentAction(release_gate(
        spec,
        substituted_subject,
        suspect_fact.clone(),
        "implementer-1",
        RuntimeAdmission::Draining,
    )));
    assert!(!substituted.allowed());
    assert_eq!(
        substituted.reason(),
        PolicyReason::WriterLeaseSubjectMismatch
    );

    for runtime in [
        RuntimeAdmission::Canary,
        RuntimeAdmission::Stopped,
        RuntimeAdmission::ReconciliationRequired,
    ] {
        let decision = evaluate(DecisionSubject::AgentAction(release_gate(
            spec,
            expected_subject.to_owned(),
            suspect_fact.clone(),
            "implementer-1",
            runtime,
        )));
        assert!(!decision.allowed(), "{runtime:?} admitted writer release");
        assert_eq!(decision.reason(), PolicyReason::RuntimeAdmissionDenied);
        assert_eq!(
            decision.evidence().checked_through(),
            DecisionStage::Runtime
        );
    }
}

fn digest(character: char) -> ContentDigest {
    ContentDigest::from_sha256(character.to_string().repeat(64)).expect("fixture digest")
}

fn task_spec() -> TaskSpec {
    TaskSpec::new(TaskSpecInput {
        schema_version: "2.1".to_owned(),
        task_id: TaskId::new("TASK-014-POLICY-WRITER").expect("task"),
        revision: "1".to_owned(),
        created_at: "2026-07-29T00:00:00Z".to_owned(),
        created_by: "policy-owner".to_owned(),
        project_id: "policy-writer-project".to_owned(),
        project_snapshot_id: ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        base_ref: "refs/heads/feature/policy-writer".to_owned(),
        base_commit_id: "a".repeat(40),
        goal: "Verify Writer Lease owner composition.".to_owned(),
        non_goals: vec!["No I/O.".to_owned()],
        risk_class: RiskClass::R0,
        depends_on: vec![],
        scope: TaskScope {
            allowed_paths: vec!["crates/lattice-policy/**".to_owned()],
            forbidden_paths: vec![".git/**".to_owned()],
            allowed_operations: vec![ScopeOperation::Modify],
        },
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-WRITER".to_owned(),
            description: "Owner receipt and current head compose with Policy.".to_owned(),
            evidence_type: EvidenceType::Test,
            expected_result: "Exact holder release is admitted.".to_owned(),
        }],
        verification_commands: vec!["cargo test -p lattice-policy".to_owned()],
        required_checks: vec![RequiredCheck::Test],
        requested_capabilities: vec![CapabilityRequest {
            capability: Capability::StopRuntime,
            contract_version: "1".to_owned(),
        }],
        budget: TaskBudget {
            accounting_currency: "USD".to_owned(),
            max_agents: "1".to_owned(),
            max_duration_seconds: "60".to_owned(),
            max_attempts: "1".to_owned(),
            max_model_calls: "0".to_owned(),
            max_external_cost: "0".to_owned(),
        },
        runtime_profile: RuntimeProfile::Fake,
        network_policy: NetworkPolicy::Deny,
        deployment_policy: DeploymentPolicy::Deny,
        approval_requirements: ApprovalRequirements {
            execution: ApprovalRequirement::NotRequired,
            merge: ApprovalRequirement::NotRequired,
            protected_release: ApprovalRequirement::ProtectedGuardian,
        },
    })
    .expect("valid Task Spec")
}

fn spec_digest(spec: &TaskSpec) -> ContentDigest {
    ContentDigest::from_sha256(spec.spec_hash().to_hex()).expect("Task Spec digest")
}

fn subject_binding(spec: &TaskSpec) -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        spec.fields().task_id.clone(),
        spec.fields().revision.clone(),
        spec_digest(spec),
    )
    .expect("binding")
}

fn project_fact(spec: &TaskSpec) -> ProjectAuthorityFact {
    let receipt = ProjectAuthorityReceipt::new(
        CONTRACT_VERSION,
        PROJECT_AUTHORITY_PRODUCER_ID,
        PROJECT_AUTHORITY_PRODUCER_VERSION,
        RuntimeKind::Fake,
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        1,
        ProjectLifecycle::Active,
        ProjectClass::UserProject,
        GitRefIdentity::new("refs/heads/main", digest('5')).expect("primary ref"),
        digest('6'),
        digest('7'),
    )
    .expect("project receipt");
    ProjectAuthorityFact {
        binding: subject_binding(spec),
        current_head: receipt.head(),
        receipt,
    }
}

fn acquire_claim(spec: &TaskSpec, project_id: ProjectId) -> AcquireClaim {
    AcquireClaim {
        project_id,
        project_snapshot_id: spec.fields().project_snapshot_id.clone(),
        task_id: spec.fields().task_id.clone(),
        task_revision: spec.fields().revision.clone(),
        task_spec_digest: spec_digest(spec),
        attempt_id: AttemptId::new("attempt-1").expect("attempt"),
        lease_id: "lease-policy-writer".to_owned(),
        lease_holder_id: "implementer-1".to_owned(),
        worktree_id: "worktree-policy-writer".to_owned(),
        holder_process_id: HolderProcessId::new(42).expect("process"),
        holder_process_start_identity: digest('8'),
        daemon_instance_id: "daemon-1".to_owned(),
        daemon_epoch: DaemonEpoch::new(7).expect("epoch"),
    }
}

fn writer_subject(claim: &AcquireClaim, fencing_token: FencingToken) -> WriterLeaseSubject {
    WriterLeaseSubject {
        lease_holder_id: claim.lease_holder_id.clone(),
        lease_id: claim.lease_id.clone(),
        attempt_id: claim.attempt_id.clone(),
        worktree_id: claim.worktree_id.clone(),
        holder_process_id: claim.holder_process_id,
        holder_process_start_identity: claim.holder_process_start_identity.clone(),
        daemon_instance_id: claim.daemon_instance_id.clone(),
        daemon_epoch: claim.daemon_epoch,
        fencing_token,
        runtime: RuntimeKind::Fake,
    }
}

fn current_fact(owner: &FakeWriterLease, project_id: &ProjectId) -> WriterLeaseFact {
    WriterLeaseFact {
        receipt: owner.current_receipt(project_id).expect("owner receipt"),
        current_head: owner.current_head(project_id),
    }
}

fn release_gate<'a>(
    spec: &'a TaskSpec,
    writer_subject: WriterLeaseSubject,
    writer: WriterLeaseFact,
    actor_id: &str,
    runtime_admission: RuntimeAdmission,
) -> AgentActionGate<'a> {
    AgentActionGate {
        context: TaskContext {
            task_spec: Some(spec),
            project: Some(project_fact(spec)),
            state: Boundary::Known(TaskState::Blocked),
            runtime_admission: Boundary::Known(runtime_admission),
        },
        role: Boundary::Known(AgentRole::Implementer),
        action: Boundary::Known(PolicyAction::ReleaseWriter),
        actor_id: actor_id.to_owned(),
        approval: None,
        provider_capability: None,
        external_cost: None,
        writer_subject: Some(writer_subject),
        writer: Some(writer),
        resource_subject: None,
        resources: None,
        network: NetworkIntent::None,
        deployment: DeploymentIntent::None,
    }
}
