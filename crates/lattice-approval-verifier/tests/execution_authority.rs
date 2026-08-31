use lattice_approval_verifier::{
    ApprovalCommandOutcome, ApprovalDenial, ApprovalVerifierCheckpoint,
    BindExecutionApprovalCommand, ClosedPolicyExecutionContext, ExecutionApprovalBindingReceipt,
    ExecutionApprovalChallenge, ExecutionApprovalSubject, ExecutionAuthorityError,
    ExecutionAuthoritySource, ExecutionCapability, FakeApprovalVerifier,
    FakeExecutionApprovalProof, FakeNormalSigner, IssueApprovalCommand, SecretMaterial,
    UntrustedApprovalSnapshot, VerifiedApprovalExecutionContext, VerifyApprovalCommand,
    issue_closed_policy_execution_authority, issue_verified_approval_execution_authority,
    nonce_commitment, reverify_closed_policy_execution_authority,
    reverify_verified_approval_execution_authority, verify_snapshot,
    verify_untrusted_execution_authority,
};
use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    ApprovalAuthority, ApprovalAuthorityReceipt, ApprovalIdentity, ApprovalLane, ApprovalOrigin,
    ApprovalSubject, CONTRACT_VERSION, ContentDigest, GitRefIdentity, ProjectAuthorityReceipt,
    ProjectClass, ProjectId, ProjectLifecycle, ProjectSnapshotId, RuntimeKind, SubjectBinding,
    TaskId,
};
use lattice_policy::{
    Boundary, ExecutionGate, ExecutionGateDecisionEvidence, ManagedExecutionBindingFact,
    ProjectAuthorityFact, RuntimeAdmission, TaskContext, evaluate_execution_gate_with_evidence,
    evaluate_managed_execution_gate_with_evidence,
};
use lattice_task_domain::{
    AcceptanceCriterion, ApprovalRequirement, ApprovalRequirements, Capability, CapabilityRequest,
    DeploymentPolicy, EvidenceType, NetworkPolicy, RequiredCheck, RiskClass, RuntimeProfile,
    ScopeOperation, TASK_SPEC_SCHEMA_VERSION, TaskBudget, TaskScope, TaskSpec, TaskSpecInput,
    TaskState,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn task_spec() -> TaskSpec {
    TaskSpec::new(TaskSpecInput {
        schema_version: TASK_SPEC_SCHEMA_VERSION.to_owned(),
        task_id: TaskId::new("TASK-POLICY-AUTHORITY").expect("task"),
        revision: "1".to_owned(),
        created_at: "2026-08-26T13:00:00Z".to_owned(),
        created_by: "lattice-managed-foreman".to_owned(),
        project_id: "phase4-project".to_owned(),
        project_snapshot_id: ProjectSnapshotId::new("phase4-project:snapshot:1").expect("snapshot"),
        base_ref: "feature/phase4".to_owned(),
        base_commit_id: "a".repeat(40),
        goal: "Create one bounded local marker.".to_owned(),
        non_goals: vec!["Do not merge, deploy, publish, or pay.".to_owned()],
        risk_class: RiskClass::R1,
        depends_on: Vec::new(),
        scope: TaskScope {
            allowed_paths: vec!["**/*".to_owned()],
            forbidden_paths: vec![".git/**".to_owned()],
            allowed_operations: vec![ScopeOperation::Create, ScopeOperation::Modify],
        },
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "AC-01".to_owned(),
            description: "The local marker exists.".to_owned(),
            evidence_type: EvidenceType::Test,
            expected_result: "Focused checks pass.".to_owned(),
        }],
        verification_commands: vec!["trusted-project-checks-v1".to_owned()],
        required_checks: vec![
            RequiredCheck::Scope,
            RequiredCheck::Test,
            RequiredCheck::Security,
        ],
        requested_capabilities: [
            Capability::ReadRepository,
            Capability::WriteProductCode,
            Capability::RunTests,
            Capability::GitWorktree,
            Capability::UseCodex,
        ]
        .into_iter()
        .map(|capability| CapabilityRequest {
            capability,
            contract_version: "1".to_owned(),
        })
        .collect(),
        budget: TaskBudget {
            accounting_currency: "TWD".to_owned(),
            max_agents: "1".to_owned(),
            max_duration_seconds: "900".to_owned(),
            max_attempts: "3".to_owned(),
            max_model_calls: "6".to_owned(),
            max_external_cost: "0".to_owned(),
        },
        runtime_profile: RuntimeProfile::Codex,
        network_policy: NetworkPolicy::Deny,
        deployment_policy: DeploymentPolicy::Deny,
        approval_requirements: ApprovalRequirements {
            execution: ApprovalRequirement::NotRequired,
            merge: ApprovalRequirement::ResponsibleUser,
            protected_release: ApprovalRequirement::ProtectedGuardian,
        },
    })
    .expect("TaskSpec")
}

fn project_receipt(spec: &TaskSpec) -> ProjectAuthorityReceipt {
    ProjectAuthorityReceipt::new(
        CONTRACT_VERSION,
        "lattice-project-registry",
        "1.0",
        RuntimeKind::Live,
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        1,
        ProjectLifecycle::Active,
        ProjectClass::UserProject,
        GitRefIdentity::new("refs/heads/main", digest('9')).expect("primary branch"),
        digest('8'),
        digest('7'),
    )
    .expect("project receipt")
}

fn binding(spec: &TaskSpec) -> SubjectBinding {
    let spec_digest = ContentDigest::from_sha256(spec.spec_hash().to_hex()).expect("spec digest");
    SubjectBinding::new(
        ProjectId::new(spec.fields().project_id.clone()).expect("project"),
        spec.fields().project_snapshot_id.clone(),
        spec.fields().task_id.clone(),
        &spec.fields().revision,
        spec_digest,
    )
    .expect("binding")
}

fn context() -> ClosedPolicyExecutionContext {
    let spec = task_spec();
    let binding = binding(&spec);
    let project = project_receipt(&spec);
    ClosedPolicyExecutionContext::new(
        digest('1'),
        digest('2'),
        binding,
        digest('4'),
        digest('5'),
        project.clone(),
        project.head(),
        "2026-08-26T13:00:00Z",
        "2026-08-26T13:30:00Z",
    )
    .expect("context")
}

fn context_with_substituted_spec() -> ClosedPolicyExecutionContext {
    let spec = task_spec();
    let original = binding(&spec);
    let substituted = SubjectBinding::new(
        original.project_id().clone(),
        original.project_snapshot_id().clone(),
        original.task_id().clone(),
        original.task_revision(),
        digest('6'),
    )
    .expect("substituted binding");
    let project = project_receipt(&spec);
    ClosedPolicyExecutionContext::new(
        digest('1'),
        digest('2'),
        substituted,
        digest('4'),
        digest('5'),
        project.clone(),
        project.head(),
        "2026-08-26T13:00:00Z",
        "2026-08-26T13:30:00Z",
    )
    .expect("substituted context shape")
}

fn normal_signer() -> FakeNormalSigner {
    FakeNormalSigner::new(
        "responsible-user",
        "os-authenticator",
        "local-key",
        SecretMaterial::new(b"phase4-execution-signing-key".to_vec()).expect("secret"),
    )
    .expect("normal signer")
}

fn approval_identity() -> ApprovalIdentity {
    let spec = task_spec();
    let subject_binding = binding(&spec);
    ApprovalIdentity::new(
        "approval-phase4-execution",
        "challenge-phase4-execution",
        subject_binding.clone(),
        ApprovalSubject::Execution {
            task_spec_hash: subject_binding.task_spec_digest().clone(),
            external_cost: None,
        },
        "lattice-runtime",
        "responsible-user",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "local-approval-channel",
        "local-approval-session",
    )
    .expect("approval identity")
}

struct VerifiedApprovalFixture {
    context: VerifiedApprovalExecutionContext,
    verifier: FakeApprovalVerifier,
    approval_receipt: ApprovalAuthorityReceipt,
    binding_receipt: ExecutionApprovalBindingReceipt,
    execution_challenge: ExecutionApprovalChallenge,
    execution_proof: FakeExecutionApprovalProof,
    legacy_snapshot: UntrustedApprovalSnapshot,
    legacy_checkpoint: ApprovalVerifierCheckpoint,
}

fn verified_approval_fixture(
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    budget_digest: ContentDigest,
) -> VerifiedApprovalFixture {
    let spec = task_spec();
    let signer = normal_signer();
    let mut verifier = FakeApprovalVerifier::new();
    let issue = verifier
        .issue(IssueApprovalCommand {
            command_id: "issue-phase4-execution".to_owned(),
            expected_head: None,
            runtime: RuntimeKind::Fake,
            identity: approval_identity(),
            nonce_id: "nonce-phase4-execution".to_owned(),
            nonce_commitment: nonce_commitment(
                &SecretMaterial::new(b"phase4-execution-nonce".to_vec()).expect("nonce"),
            )
            .expect("nonce commitment"),
            issued_at: "2026-08-26T13:00:00Z".to_owned(),
            expires_at: "2026-08-26T13:30:00Z".to_owned(),
            authenticator_id: signer.authenticator_id().to_owned(),
            key_id: signer.key_id().to_owned(),
            verification_key_commitment: signer.verification_key_commitment().clone(),
            evidence_digest: signer.evidence_digest().clone(),
            review_set_digest: None,
        })
        .expect("issue approval challenge");
    let base_challenge = issue.challenge.expect("base challenge");
    let subject = ExecutionApprovalSubject::new(
        task_ref,
        successor_stream_id,
        binding(&spec),
        base_challenge.subject_digest().clone(),
        budget_digest,
    )
    .expect("execution subject");
    let execution_challenge =
        ExecutionApprovalChallenge::new(base_challenge, subject).expect("execution challenge");
    let execution_proof = signer
        .sign_execution(&execution_challenge)
        .expect("execution proof");
    let verify = verifier
        .verify(VerifyApprovalCommand {
            command_id: "verify-phase4-execution".to_owned(),
            approval_id: "approval-phase4-execution".to_owned(),
            expected_head: verifier
                .state_head("approval-phase4-execution")
                .expect("challenged head"),
            observed_at: "2026-08-26T13:05:00Z".to_owned(),
            proof: execution_proof.base_proof().clone(),
        })
        .expect("verify base approval");
    let approval_receipt = verify.authority_receipt.expect("authority receipt");
    let legacy_snapshot = verifier.export_snapshot();
    let legacy_checkpoint = verifier.current_checkpoint().expect("legacy checkpoint");
    let bind = verifier
        .bind_execution(BindExecutionApprovalCommand {
            command_id: "bind-phase4-execution".to_owned(),
            approval_id: "approval-phase4-execution".to_owned(),
            expected_head: verifier
                .state_head("approval-phase4-execution")
                .expect("verified available head"),
            observed_at: "2026-08-26T13:05:00Z".to_owned(),
            execution_challenge: execution_challenge.clone(),
            execution_proof: execution_proof.clone(),
        })
        .expect("bind exact execution approval");
    let binding_receipt = bind
        .execution_binding_receipt
        .expect("execution binding receipt");
    let current_head = verifier
        .current_head_at("approval-phase4-execution", "2026-08-26T13:05:00Z")
        .expect("current lookup")
        .expect("current approval head");
    let context = VerifiedApprovalExecutionContext::new_with_binding_receipt(
        binding_receipt.clone(),
        approval_receipt.clone(),
        current_head,
    )
    .expect("verified approval context");
    VerifiedApprovalFixture {
        context,
        verifier,
        approval_receipt,
        binding_receipt,
        execution_challenge,
        execution_proof,
        legacy_snapshot,
        legacy_checkpoint,
    }
}

fn object_field_mut<'a>(value: &'a mut CanonicalValue, key: &str) -> &'a mut CanonicalValue {
    let CanonicalValue::Object(entries) = value else {
        panic!("expected object while finding {key}");
    };
    entries
        .iter_mut()
        .find_map(|(name, value)| (name == key).then_some(value))
        .unwrap_or_else(|| panic!("missing object field {key}"))
}

fn policy_decision(state: TaskState, runtime: RuntimeAdmission) -> ExecutionGateDecisionEvidence {
    let spec = task_spec();
    let subject_binding = binding(&spec);
    let project = project_receipt(&spec);
    evaluate_managed_execution_gate_with_evidence(
        ExecutionGate {
            context: TaskContext {
                task_spec: Some(&spec),
                project: Some(ProjectAuthorityFact {
                    binding: subject_binding.clone(),
                    receipt: project.clone(),
                    current_head: project.head(),
                }),
                state: Boundary::Known(state),
                runtime_admission: Boundary::Known(runtime),
            },
            approval: None,
        },
        ManagedExecutionBindingFact {
            task_ref: digest('1'),
            successor_stream_id: digest('2'),
            task_spec_digest: subject_binding.task_spec_digest().clone(),
            approval_subject_digest: digest('4'),
            budget_digest: digest('5'),
        },
    )
}

#[test]
fn formal_policy_authority_is_exact_and_restart_reverifiable() {
    let context = context();
    let decision = policy_decision(
        TaskState::AwaitingExecutionApproval,
        RuntimeAdmission::Active,
    );
    let record =
        issue_closed_policy_execution_authority(&context, &decision).expect("formal authority");
    assert_eq!(
        record.source(),
        ExecutionAuthoritySource::ClosedPolicyNoApprovalRequired
    );
    assert_eq!(
        record.capability(),
        ExecutionCapability::LocalReversibleTaskExecution
    );
    assert_eq!(record.task_ref(), &digest('1'));
    assert_eq!(record.budget_digest(), &digest('5'));
    let replayed = verify_untrusted_execution_authority(&record.to_untrusted())
        .expect("structural persistence replay");
    reverify_closed_policy_execution_authority(
        &replayed,
        &context,
        &decision,
        "2026-08-26T13:05:00Z",
    )
    .expect("fresh policy replay");
}

#[test]
fn verified_approval_requires_actual_current_receipt_and_preserves_separate_authority() {
    let fixture = verified_approval_fixture(digest('1'), digest('2'), digest('5'));
    let authority =
        issue_verified_approval_execution_authority(&fixture.context, "2026-08-26T13:05:00Z")
            .expect("receipt-backed execution authority");
    assert_eq!(
        authority.source(),
        ExecutionAuthoritySource::VerifiedApproval
    );
    assert_eq!(
        authority.approval_receipt_digest(),
        Some(fixture.approval_receipt.receipt_digest())
    );
    reverify_verified_approval_execution_authority(
        &authority,
        &fixture.context,
        "2026-08-26T13:10:00Z",
    )
    .expect("fresh current approval head");
    assert_eq!(
        issue_verified_approval_execution_authority(&fixture.context, "2026-08-26T13:30:00Z"),
        Err(ExecutionAuthorityError::Expired)
    );

    let snapshot = fixture.verifier.export_snapshot();
    let snapshot_bytes = fixture
        .verifier
        .export_snapshot_bytes()
        .expect("canonical snapshot bytes");
    let checkpoint = fixture.verifier.current_checkpoint().expect("checkpoint");
    let mut restarted = FakeApprovalVerifier::new();
    assert_eq!(
        UntrustedApprovalSnapshot::from_canonical_bytes(&snapshot_bytes)
            .expect("strict snapshot bytes"),
        snapshot
    );
    restarted
        .restore_snapshot_bytes(&snapshot_bytes, &checkpoint)
        .expect("fresh-process restore");
    let restarted_head = restarted
        .current_head_at("approval-phase4-execution", "2026-08-26T13:10:00Z")
        .expect("restart lookup")
        .expect("restart current head");
    let restarted_binding = restarted
        .execution_binding_receipt("approval-phase4-execution")
        .expect("restart retained execution binding")
        .clone();
    assert_eq!(restarted_binding, fixture.binding_receipt);
    let restarted_context = VerifiedApprovalExecutionContext::new_with_binding_receipt(
        restarted_binding,
        fixture.approval_receipt.clone(),
        restarted_head,
    )
    .expect("restart context");
    reverify_verified_approval_execution_authority(
        &authority,
        &restarted_context,
        "2026-08-26T13:10:00Z",
    )
    .expect("fresh-process owner replay");

    assert_eq!(
        VerifiedApprovalExecutionContext::new(
            digest('1'),
            digest('2'),
            digest('5'),
            fixture.approval_receipt.subject_digest().clone(),
            fixture.approval_receipt.clone(),
            fixture.approval_receipt.head(),
        ),
        Err(ExecutionAuthorityError::BindingMismatch)
    );
}

#[test]
fn bind_execution_is_append_only_idempotent_and_rejects_rebinding() {
    let fixture = verified_approval_fixture(digest('1'), digest('2'), digest('5'));
    let original = fixture
        .verifier
        .command_receipts()
        .last()
        .expect("bind terminal receipt")
        .clone();
    let exact_retry = fixture
        .verifier
        .clone()
        .bind_execution(match original.request.clone() {
            lattice_approval_verifier::ApprovalCommand::BindExecution(command) => command,
            other => panic!("expected bind command, got {other:?}"),
        })
        .expect("exact bind retry");
    assert_eq!(exact_retry, original);

    let mut verifier = fixture.verifier;
    let rebound = verifier
        .bind_execution(BindExecutionApprovalCommand {
            command_id: "bind-phase4-execution-replacement".to_owned(),
            approval_id: "approval-phase4-execution".to_owned(),
            expected_head: verifier
                .state_head("approval-phase4-execution")
                .expect("post-bind head"),
            observed_at: "2026-08-26T13:06:00Z".to_owned(),
            execution_challenge: fixture.execution_challenge,
            execution_proof: fixture.execution_proof,
        })
        .expect("terminal rebinding denial");
    assert_eq!(
        rebound.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::InvalidState)
    );
    assert!(rebound.execution_binding_receipt.is_none());
}

#[test]
fn owner_snapshot_replay_detects_execution_binding_tamper_and_accepts_legacy_unbound_state() {
    let fixture = verified_approval_fixture(digest('1'), digest('2'), digest('5'));

    let mut legacy = FakeApprovalVerifier::new();
    legacy
        .restore_snapshot(&fixture.legacy_snapshot, &fixture.legacy_checkpoint)
        .expect("legacy unbound snapshot remains replay-compatible");
    assert!(
        legacy
            .execution_binding_receipt("approval-phase4-execution")
            .is_none(),
        "legacy owner facts do not silently acquire execution authority"
    );

    let mut tampered = fixture.verifier.export_snapshot();
    let CanonicalValue::Array(commands) = object_field_mut(&mut tampered.payload, "commands")
    else {
        panic!("commands array");
    };
    let terminal = commands.last_mut().expect("bind terminal");
    let binding_receipt = object_field_mut(terminal, "execution_binding_receipt");
    let bound_subject = object_field_mut(binding_receipt, "subject");
    *object_field_mut(bound_subject, "budget_digest") = CanonicalValue::String("f".repeat(64));
    assert_eq!(
        verify_snapshot(&tampered),
        Err(lattice_approval_verifier::ApprovalVerifierError::CorruptSnapshot)
    );
}

#[test]
fn snapshot_bytes_reject_noncanonical_duplicate_numeric_and_trailing_input() {
    let fixture = verified_approval_fixture(digest('1'), digest('2'), digest('5'));
    let bytes = fixture
        .verifier
        .export_snapshot_bytes()
        .expect("snapshot bytes");

    let mut whitespace = b" ".to_vec();
    whitespace.extend_from_slice(&bytes);
    for invalid in [
        whitespace,
        br#"{"version":"1.0","version":"1.0"}"#.to_vec(),
        br#"{"version":1}"#.to_vec(),
        [bytes.clone(), b"x".to_vec()].concat(),
    ] {
        assert_eq!(
            UntrustedApprovalSnapshot::from_canonical_bytes(&invalid),
            Err(lattice_approval_verifier::ApprovalVerifierError::CorruptSnapshot)
        );
    }
}

#[test]
fn verified_approval_receipt_directly_commits_task_successor_and_budget() {
    let original = verified_approval_fixture(digest('1'), digest('2'), digest('5'));
    let authority =
        issue_verified_approval_execution_authority(&original.context, "2026-08-26T13:05:00Z")
            .expect("original authority");

    for (task_ref, successor_stream_id, budget_digest) in [
        (digest('b'), digest('2'), digest('5')),
        (digest('1'), digest('c'), digest('5')),
        (digest('1'), digest('2'), digest('d')),
    ] {
        let substituted = verified_approval_fixture(task_ref, successor_stream_id, budget_digest);
        assert_eq!(
            reverify_verified_approval_execution_authority(
                &authority,
                &substituted.context,
                "2026-08-26T13:10:00Z",
            ),
            Err(ExecutionAuthorityError::BindingMismatch)
        );
    }

    assert_eq!(
        original.execution_proof.base_proof().evidence_digest(),
        normal_signer().evidence_digest(),
        "execution binding must preserve signer/authenticator evidence semantics"
    );
    assert_ne!(
        original.execution_proof.base_proof().evidence_digest(),
        original.execution_challenge.subject().subject_digest(),
        "signer evidence must never be repurposed as the execution subject"
    );

    let substituted = verified_approval_fixture(digest('b'), digest('2'), digest('5'));
    let mut unbound = FakeApprovalVerifier::new();
    unbound
        .restore_snapshot(&original.legacy_snapshot, &original.legacy_checkpoint)
        .expect("restore original unbound approval");
    let substituted_proof = unbound
        .bind_execution(BindExecutionApprovalCommand {
            command_id: "bind-substituted-execution-proof".to_owned(),
            approval_id: "approval-phase4-execution".to_owned(),
            expected_head: unbound
                .state_head("approval-phase4-execution")
                .expect("unbound verified head"),
            observed_at: "2026-08-26T13:10:00Z".to_owned(),
            execution_challenge: original.execution_challenge.clone(),
            execution_proof: substituted.execution_proof,
        })
        .expect("substituted proof has one terminal denial");
    assert_eq!(
        substituted_proof.outcome,
        ApprovalCommandOutcome::Denied(ApprovalDenial::ProofMismatch),
        "a proof signed for another exact execution challenge cannot replay"
    );

    let original_binding = original.execution_challenge.subject().binding();
    let cross_task_binding = SubjectBinding::new(
        original_binding.project_id().clone(),
        original_binding.project_snapshot_id().clone(),
        TaskId::new("TASK-POLICY-AUTHORITY-OTHER").expect("other task"),
        original_binding.task_revision(),
        original_binding.task_spec_digest().clone(),
    )
    .expect("same-spec cross-task binding");
    let cross_task_subject = ExecutionApprovalSubject::new(
        digest('1'),
        digest('2'),
        cross_task_binding,
        original
            .execution_challenge
            .approval_challenge()
            .subject_digest()
            .clone(),
        digest('5'),
    )
    .expect("cross-task subject shape");
    assert_eq!(
        ExecutionApprovalChallenge::new(
            original.execution_challenge.approval_challenge().clone(),
            cross_task_subject,
        ),
        Err(ExecutionAuthorityError::BindingMismatch),
        "the same Task Spec digest cannot cross an exact task identity"
    );
}

#[test]
fn closed_policy_evidence_directly_commits_task_successor_and_budget() {
    let decision = policy_decision(
        TaskState::AwaitingExecutionApproval,
        RuntimeAdmission::Active,
    );
    let spec = task_spec();
    let project = project_receipt(&spec);

    for (task_ref, successor_stream_id, budget_digest) in [
        (digest('b'), digest('2'), digest('5')),
        (digest('1'), digest('c'), digest('5')),
        (digest('1'), digest('2'), digest('d')),
    ] {
        let substituted = ClosedPolicyExecutionContext::new(
            task_ref,
            successor_stream_id,
            binding(&spec),
            digest('4'),
            budget_digest,
            project.clone(),
            project.head(),
            "2026-08-26T13:00:00Z",
            "2026-08-26T13:30:00Z",
        )
        .expect("substituted context shape");
        assert_eq!(
            issue_closed_policy_execution_authority(&substituted, &decision),
            Err(ExecutionAuthorityError::PolicyEvidenceMismatch)
        );
    }
}

#[test]
fn legacy_unbound_policy_evidence_fails_closed() {
    let spec = task_spec();
    let project = project_receipt(&spec);
    let legacy = evaluate_execution_gate_with_evidence(ExecutionGate {
        context: TaskContext {
            task_spec: Some(&spec),
            project: Some(ProjectAuthorityFact {
                binding: binding(&spec),
                receipt: project.clone(),
                current_head: project.head(),
            }),
            state: Boundary::Known(TaskState::AwaitingExecutionApproval),
            runtime_admission: Boundary::Known(RuntimeAdmission::Active),
        },
        approval: None,
    });

    assert_eq!(
        issue_closed_policy_execution_authority(&context(), &legacy),
        Err(ExecutionAuthorityError::PolicyEvidenceMismatch)
    );
}

#[test]
fn current_policy_denial_tamper_and_expiry_fail_closed() {
    let active_context = context();
    let active_decision = policy_decision(
        TaskState::AwaitingExecutionApproval,
        RuntimeAdmission::Active,
    );
    let authority = issue_closed_policy_execution_authority(&active_context, &active_decision)
        .expect("authority");
    assert_eq!(
        issue_closed_policy_execution_authority(&context_with_substituted_spec(), &active_decision,),
        Err(ExecutionAuthorityError::PolicyEvidenceMismatch)
    );
    assert_eq!(
        reverify_closed_policy_execution_authority(
            &authority,
            &active_context,
            &active_decision,
            "2026-08-26T13:30:00Z"
        ),
        Err(ExecutionAuthorityError::Expired)
    );
    assert_eq!(
        verify_untrusted_execution_authority(
            &authority.to_untrusted().with_budget_digest(digest('6'))
        ),
        Err(ExecutionAuthorityError::DigestMismatch)
    );
    assert_eq!(
        issue_closed_policy_execution_authority(
            &context(),
            &policy_decision(TaskState::Draft, RuntimeAdmission::Active),
        ),
        Err(ExecutionAuthorityError::PolicyDenied)
    );
    assert_eq!(
        issue_closed_policy_execution_authority(
            &context(),
            &policy_decision(
                TaskState::AwaitingExecutionApproval,
                RuntimeAdmission::Stopped,
            ),
        ),
        Err(ExecutionAuthorityError::PolicyDenied)
    );
}

#[test]
fn every_external_or_irreversible_gate_remains_separate() {
    for effect in [
        "MERGE",
        "DEFAULT_BRANCH",
        "PUSH",
        "DEPLOY",
        "PUBLISH",
        "PAYMENT",
        "EXTERNAL_MESSAGE",
        "PERMANENT_DELETE",
    ] {
        assert!(
            !ExecutionCapability::LocalReversibleTaskExecution.allows_external_effect(effect),
            "local execution authority must not grant {effect}"
        );
    }
}
