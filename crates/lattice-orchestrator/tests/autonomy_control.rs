use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, ContentDigest, DaemonEpoch, FencingToken, HolderProcessId,
    ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind, SubjectBinding, TaskId,
    WRITER_LEASE_PRODUCER_ID, WRITER_LEASE_PRODUCER_VERSION, WriterLeaseAuthorityHead,
    WriterLeaseAuthorityReceipt, WriterLeaseIdentity, WriterLeaseRevision, WriterLeaseStatus,
};
use lattice_orchestrator::{
    AutonomyAuthorityEvidence, AutonomyContractError, AutonomyDecision, AutonomyDecisionReason,
    AutonomyIntent, AutonomyIntentVersion, ModelRecommendation, TaskKind,
    VerificationRecommendation, build_autonomy_receipt, classify_autonomy,
};
use lattice_task_domain::{RiskClass, TaskState};

fn intent(kind: TaskKind, risk: RiskClass) -> AutonomyIntent {
    AutonomyIntent {
        version: AutonomyIntentVersion::V1,
        kind,
        risk,
        execution_preapproved: true,
        requires_new_authority: false,
        irreversible_or_high_risk: false,
    }
}

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn binding() -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-050").expect("task"),
        "1",
        digest('a'),
    )
    .expect("binding")
}

fn writer_authority(binding: &SubjectBinding) -> WriterLeaseAuthorityHead {
    let identity = WriterLeaseIdentity::new(
        binding.project_id().clone(),
        binding.project_snapshot_id().clone(),
        binding.task_id().clone(),
        binding.task_revision(),
        binding.task_spec_digest().clone(),
        AttemptId::new("attempt-1").expect("attempt"),
        "lease-1",
        "codex-writer-1",
        "workspace-1",
        HolderProcessId::new(42).expect("process"),
        digest('b'),
        "daemon-1",
        DaemonEpoch::new(1).expect("daemon epoch"),
        FencingToken::new(7).expect("fence"),
    )
    .expect("identity");
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        RuntimeKind::Live,
        identity,
        WriterLeaseStatus::Active,
        WriterLeaseRevision::new(1).expect("revision"),
        RuntimeAdmissionMode::Active,
        "2026-08-12T00:00:00Z",
        "2026-08-12T00:00:00Z",
        "2026-08-12T00:10:00Z",
        digest('c'),
        digest('d'),
        digest('e'),
        digest('f'),
    )
    .expect("receipt")
    .head()
}

fn authority(writer: Option<WriterLeaseAuthorityHead>) -> AutonomyAuthorityEvidence {
    AutonomyAuthorityEvidence::new_p0_process_start_profile(
        digest('1'),
        digest('2'),
        digest('3'),
        writer,
    )
    .expect("authority")
}

#[test]
fn authorized_feature_selects_only_existing_governed_writer_and_focused_checks() {
    let receipt = classify_autonomy(intent(TaskKind::Feature, RiskClass::R1), TaskState::Draft);
    assert_eq!(
        receipt.decision,
        AutonomyDecision::Proceed {
            model: ModelRecommendation::GovernedCodexWriter,
            verification: VerificationRecommendation::FocusedChecks,
            reason: AutonomyDecisionReason::RoutineAuthorized
        }
    );
}

#[test]
fn r2_configuration_selects_build_and_focused_checks_without_a_model_claim() {
    let receipt = classify_autonomy(
        intent(TaskKind::Configuration, RiskClass::R2),
        TaskState::Preparing,
    );
    assert_eq!(
        receipt.decision,
        AutonomyDecision::Proceed {
            model: ModelRecommendation::NoModel,
            verification: VerificationRecommendation::BuildAndFocusedChecks,
            reason: AutonomyDecisionReason::RoutineAuthorized
        }
    );
}

#[test]
fn missing_decision_new_authority_and_high_risk_escalate() {
    let mut decision = intent(TaskKind::Feature, RiskClass::R0);
    decision.execution_preapproved = false;
    assert_eq!(
        classify_autonomy(decision, TaskState::Draft).decision,
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::NewUserDecision
        }
    );
    let mut authority = intent(TaskKind::Feature, RiskClass::R0);
    authority.requires_new_authority = true;
    assert_eq!(
        classify_autonomy(authority, TaskState::Draft).decision,
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::NewAuthority
        }
    );
    assert_eq!(
        classify_autonomy(intent(TaskKind::Research, RiskClass::R3), TaskState::Draft).decision,
        AutonomyDecision::AskUser {
            reason: AutonomyDecisionReason::HighRiskOrIrreversible
        }
    );
}

#[test]
fn canonical_proceed_receipt_binds_intent_state_authority_and_writer_fence() {
    let binding = binding();
    let intent = intent(TaskKind::Feature, RiskClass::R1);
    let writer = writer_authority(&binding);
    let authority = authority(Some(writer));

    let receipt =
        build_autonomy_receipt(binding.clone(), intent, TaskState::Draft, authority.clone())
            .expect("canonical receipt");
    let exact_retry = build_autonomy_receipt(binding, intent, TaskState::Draft, authority)
        .expect("exact receipt");

    assert_eq!(receipt, exact_retry);
    assert_eq!(receipt.schema_version(), "lattice.autonomy-receipt/1.0");
    assert_eq!(receipt.authority_mode(), "P0_PROCESS_START_PROFILE_V1");
    assert_eq!(receipt.writer_fencing_token(), Some(7));
    assert!(
        !receipt
            .receipt_digest()
            .as_str()
            .bytes()
            .all(|byte| byte == b'0')
    );
    assert!(
        !receipt
            .authority_digest()
            .as_str()
            .bytes()
            .all(|byte| byte == b'0')
    );
}

#[test]
fn proceed_requires_exact_live_writer_binding_and_ask_user_rejects_ambient_writer() {
    let binding = binding();
    let mut wrong_binding = binding.clone();
    wrong_binding = SubjectBinding::new(
        wrong_binding.project_id().clone(),
        wrong_binding.project_snapshot_id().clone(),
        TaskId::new("TASK-OTHER").expect("task"),
        wrong_binding.task_revision(),
        wrong_binding.task_spec_digest().clone(),
    )
    .expect("wrong binding");

    let error = build_autonomy_receipt(
        binding.clone(),
        intent(TaskKind::Feature, RiskClass::R1),
        TaskState::Draft,
        authority(Some(writer_authority(&wrong_binding))),
    )
    .expect_err("wrong writer binding must fail closed");
    assert_eq!(error, AutonomyContractError::WriterAuthorityMismatch);

    let mut ask = intent(TaskKind::Feature, RiskClass::R0);
    ask.requires_new_authority = true;
    let error = build_autonomy_receipt(
        binding.clone(),
        ask,
        TaskState::Draft,
        authority(Some(writer_authority(&binding))),
    )
    .expect_err("ASK_USER cannot carry ambient writer authority");
    assert_eq!(error, AutonomyContractError::UnexpectedWriterAuthority);

    let error = build_autonomy_receipt(
        binding,
        intent(TaskKind::Feature, RiskClass::R1),
        TaskState::Draft,
        authority(None),
    )
    .expect_err("PROCEED requires writer authority");
    assert_eq!(error, AutonomyContractError::WriterAuthorityRequired);
}
