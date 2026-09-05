use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    AttemptId, CONTRACT_VERSION, ContentDigest, DaemonEpoch, FencingToken, HolderProcessId,
    ProjectId, ProjectSnapshotId, ResourceCounters, ResourceRequest, RuntimeAdmissionMode,
    RuntimeKind, TaskId, TaskLedgerStreamIdentity, WRITER_LEASE_PRODUCER_ID,
    WRITER_LEASE_PRODUCER_VERSION, WriterLeaseAuthorityHead, WriterLeaseAuthorityReceipt,
    WriterLeaseIdentity, WriterLeaseRevision, WriterLeaseStatus,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, AutonomyAppendMetadata, AutonomyAuthorityEvidence,
    AutonomyDecisionReason, AutonomyIntent, AutonomyModel, AutonomyObservedTaskState,
    AutonomyRecommendation, AutonomyRiskClass, AutonomyTaskKind, AutonomyVerification, CommandId,
    CommandOutcome, CorrelationId, Diagnostic, EffectClaimId, ExternalVerifiedResultAdoption,
    FakeTaskLedger, LedgerCheckpoint, LedgerDenial, LedgerError, LedgerEventKind, LedgerOutcome,
    OutboxAdmissionState, ReasonCode, ResourceSnapshot, TaskCreatedProfile, TaskSubmissionEnvelope,
    UntrustedAutonomyReceiptRow, VerifiedAutonomyReceiptState, VerifiedStream, apply_append_plan,
    classify_task_created_profile, export_untrusted_snapshot, plan_append,
    plan_autonomy_receipt_append, verify_exact_autonomy_receipt_retry,
    verify_untrusted_autonomy_receipt_rows, verify_untrusted_external_verified_result_adoption,
    verify_untrusted_snapshot, verify_untrusted_snapshot_against_checkpoint,
    verify_untrusted_task_submission,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn identity(project: &str, task: &str) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new(
        ProjectId::new(project).expect("project"),
        ProjectSnapshotId::new(format!("{project}:snapshot:1")).expect("snapshot"),
        TaskId::new(task).expect("task"),
        "1",
        digest('a'),
        "TWD",
    )
    .expect("stream identity")
}

fn autonomy_identity() -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-050").expect("task"),
        "1",
        digest('a'),
        "TWD",
    )
    .expect("stream identity")
}

fn general_identity(project: &str, task: &str) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new(project).expect("project"),
        ProjectSnapshotId::new(format!("{project}:snapshot:1")).expect("snapshot"),
        TaskId::new(task).expect("task"),
        "1",
        digest('a'),
    )
    .expect("general intake identity")
}

fn general_submission() -> TaskSubmissionEnvelope {
    TaskSubmissionEnvelope::new(
        "lattice-mcp",
        "request-1",
        "完成角色系統",
        "AI 劇本",
        general_identity("project-1", "TASK-GENERAL-1"),
        digest('9'),
    )
    .expect("valid general-task submission")
}

#[test]
fn general_submission_is_canonical_secret_safe_and_tamper_evident() {
    let submission = general_submission();
    assert_eq!(
        submission.schema_version(),
        "lattice.task-ledger.task-submission/1.0"
    );
    assert_eq!(submission.ingress_id(), "lattice-mcp");
    assert_eq!(submission.client_request_id(), "request-1");
    assert_eq!(submission.objective(), "完成角色系統");
    assert_eq!(submission.project_display_name(), "AI 劇本");
    assert_eq!(submission.project_authority_receipt_digest(), &digest('9'));
    assert_eq!(
        submission.identity(),
        &general_identity("project-1", "TASK-GENERAL-1")
    );
    assert_eq!(submission.task_ref().as_str().len(), 64);
    assert_eq!(submission.admission_action(), "GENERAL_TASK_INTAKE_V1");
    assert!(!format!("{submission:?}").contains("完成角色系統"));

    let retained = submission.to_untrusted();
    assert_eq!(
        verify_untrusted_task_submission(&retained).expect("verified retained envelope"),
        submission
    );

    let mut changed_objective = retained.clone();
    changed_objective.objective = "完成另一個系統".to_owned();
    assert_eq!(
        verify_untrusted_task_submission(&changed_objective),
        Err(LedgerError::SubmissionEnvelopeMismatch)
    );

    let mut changed_ref = retained;
    changed_ref.task_ref = digest('f');
    assert_eq!(
        verify_untrusted_task_submission(&changed_ref),
        Err(LedgerError::SubmissionEnvelopeMismatch)
    );
}

#[test]
fn general_submission_rejects_blank_controls_oversize_non_nfc_and_secret_shapes() {
    let make = |objective: &str| {
        TaskSubmissionEnvelope::new(
            "lattice-mcp",
            "request-1",
            objective,
            "AI 劇本",
            general_identity("project-1", "TASK-GENERAL-1"),
            digest('9'),
        )
    };
    for invalid in [
        "",
        "   ",
        " leading",
        "trailing ",
        "line\nbreak",
        "nul\0byte",
    ] {
        assert!(matches!(
            make(invalid),
            Err(LedgerError::InvalidSubmissionEnvelope { field: "objective" })
        ));
    }
    assert!(matches!(
        make(&"目".repeat(683)),
        Err(LedgerError::SubmissionEnvelopeLimitExceeded { field: "objective" })
    ));
    assert!(matches!(
        make("e\u{301}"),
        Err(LedgerError::NonCanonicalText { field: "objective" })
    ));
    for secret in [
        "password=hunter2",
        "authorization: Bearer abcdefghijklmnopqrstuvwxyz",
        "-----BEGIN PRIVATE KEY-----",
        "sk-abcdefghijklmnopqrstuvwxyz0123456789",
        "完成設定 secret=hunter2",
        "credential: do-not-store",
        "Cookie = session-value",
        "refresh_token=do-not-store",
        r#"{"password":"hunter2"}"#,
        r#"{"api_key":"do-not-store"}"#,
        "password\u{2003}=hunter2",
        "api_key\u{a0}:do-not-store",
        "使用 AKIAIOSFODNN7EXAMPLE 完成設定",
    ] {
        assert_eq!(make(secret), Err(LedgerError::SubmissionSecretRejected));
    }
    assert!(make("finish mask-based validation").is_ok());
}

#[test]
fn general_task_created_profile_binds_the_submission_digest_and_is_create_only() {
    let submission = general_submission();
    let vacant = VerifiedStream::vacant(submission.identity().clone(), RuntimeKind::Live)
        .expect("vacant live stream");
    let command = AppendCommand::new_general_task_created(
        vacant.head().clone(),
        CommandId::new("general-create-1").expect("command"),
        CorrelationId::new("general-correlation-1").expect("correlation"),
        "2026-08-26T00:00:00Z",
        ActorId::new("lattice-mcp").expect("actor"),
        &submission,
    )
    .expect("general task-created command");
    assert_eq!(command.subject_digest(), submission.envelope_digest());
    let plan = plan_append(&vacant, command).expect("plan general task creation");
    let created = apply_append_plan(&vacant, &plan).expect("apply general task creation");
    assert_eq!(
        classify_task_created_profile(&created.events()[0]).expect("known profile"),
        Some(TaskCreatedProfile::GeneralTaskIntakeV1)
    );
    assert_eq!(
        plan_append(
            &created,
            AppendCommand::new(
                created.head().clone(),
                CommandId::new("skip-receipt").expect("command"),
                CorrelationId::new("skip-receipt").expect("correlation"),
                "2026-08-26T00:00:01Z",
                LedgerEventKind::EvidenceRecorded,
                ActorId::new("worker").expect("actor"),
                ActionId::new("RECORD").expect("action"),
                LedgerOutcome::Recorded,
                ReasonCode::new("RECORDED").expect("reason"),
                digest('b'),
                None,
                None,
            )
            .expect("ordinary command")
        ),
        Err(LedgerError::GeneralTaskIntakeCreateOnly)
    );
}

#[test]
fn general_intake_allows_only_the_typed_external_verified_result_adoption_terminal() {
    let submission = general_submission();
    let vacant = VerifiedStream::vacant(submission.identity().clone(), RuntimeKind::Live)
        .expect("vacant live stream");
    let created = apply_append_plan(
        &vacant,
        &plan_append(
            &vacant,
            AppendCommand::new_general_task_created(
                vacant.head().clone(),
                CommandId::new("general-create-adoption-1").expect("command"),
                CorrelationId::new("general-adoption-correlation-1").expect("correlation"),
                "2026-08-31T00:00:00Z",
                ActorId::new("lattice-mcp").expect("actor"),
                &submission,
            )
            .expect("general task-created command"),
        )
        .expect("plan intake"),
    )
    .expect("apply intake");
    let evidence = |byte: char| format!("evidence:sha256:{}", byte.to_string().repeat(64));
    let adoption = ExternalVerifiedResultAdoption::new(
        submission.task_ref().clone(),
        "adopt-verified-001",
        created.head().head_digest().clone(),
        "1".repeat(40),
        "2".repeat(40),
        evidence('3'),
        evidence('4'),
        evidence('5'),
        evidence('6'),
        vec![evidence('7'), evidence('8')],
    )
    .expect("bounded adoption");
    let command = AppendCommand::new_external_verified_result_adopted(
        created.head().clone(),
        CommandId::new(adoption.command_id()).expect("command"),
        CorrelationId::new("external-result-adoption-v1").expect("correlation"),
        "2026-08-31T00:00:01Z",
        ActorId::new("lattice-mcp").expect("actor"),
        &adoption,
    )
    .expect("typed terminal command");
    let plan = plan_append(&created, command.clone()).expect("typed adoption is allowed");
    assert_eq!(
        plan.new_event().expect("terminal event").kind(),
        LedgerEventKind::ExternalVerifiedResultAdopted
    );
    assert_eq!(
        plan.new_event().expect("terminal event").subject_digest(),
        adoption.result_digest()
    );
    let completed = apply_append_plan(&created, &plan).expect("apply terminal adoption");
    assert_eq!(completed.events().len(), 2);
    assert!(
        plan_append(&completed, command)
            .expect("exact retry")
            .is_exact_retry()
    );
    assert_eq!(
        plan_append(
            &completed,
            AppendCommand::new(
                completed.head().clone(),
                CommandId::new("ordinary-after-adoption").expect("command"),
                CorrelationId::new("ordinary-after-adoption").expect("correlation"),
                "2026-08-31T00:00:02Z",
                LedgerEventKind::EvidenceRecorded,
                ActorId::new("worker").expect("actor"),
                ActionId::new("RECORD").expect("action"),
                LedgerOutcome::Recorded,
                ReasonCode::new("RECORDED").expect("reason"),
                digest('b'),
                None,
                None,
            )
            .expect("ordinary command"),
        ),
        Err(LedgerError::GeneralTaskIntakeCreateOnly)
    );
}

#[test]
fn external_verified_adoption_persistence_rejects_changed_evidence_on_replay() {
    let adoption = ExternalVerifiedResultAdoption::new(
        digest('a'),
        "adoption-replay-1",
        digest('b'),
        "1".repeat(40),
        "2".repeat(40),
        format!("evidence:sha256:{}", "3".repeat(64)),
        format!("evidence:sha256:{}", "4".repeat(64)),
        format!("evidence:sha256:{}", "5".repeat(64)),
        format!("evidence:sha256:{}", "6".repeat(64)),
        vec![format!("evidence:sha256:{}", "7".repeat(64))],
    )
    .unwrap();
    let retained = adoption.to_untrusted();
    assert_eq!(
        verify_untrusted_external_verified_result_adoption(&retained).unwrap(),
        adoption
    );
    let mut changed = retained;
    changed.deployment_receipt_ref = format!("evidence:sha256:{}", "8".repeat(64));
    assert_eq!(
        verify_untrusted_external_verified_result_adoption(&changed),
        Err(LedgerError::ExternalVerifiedResultAdoptionMismatch)
    );
}

#[test]
fn managed_general_task_created_binds_exact_spec_digest_and_requires_receipt() {
    let identity = identity("project-1", "TASK-MANAGED-1");
    let expected_spec_digest = identity
        .task_spec_digest()
        .expect("managed successor is TaskSpec-bound")
        .clone();
    let vacant = VerifiedStream::vacant(identity, RuntimeKind::Live).expect("vacant live stream");
    let command = AppendCommand::new_managed_general_task_created(
        vacant.head().clone(),
        CommandId::new("managed-create-1").expect("command"),
        CorrelationId::new("managed-correlation-1").expect("correlation"),
        "2026-08-26T00:00:00Z",
        ActorId::new("lattice-foreman").expect("actor"),
        ReasonCode::new("MANAGED_GENERAL_TASK_ACCEPTED").expect("reason"),
    )
    .expect("managed task-created command");

    assert_eq!(command.action().as_str(), "MANAGED_GENERAL_TASK_V1");
    assert_eq!(command.subject_digest(), &expected_spec_digest);
    assert!(command.diagnostic().is_none());
    assert!(TaskCreatedProfile::ManagedGeneralTaskV1.requires_autonomy_receipt());

    let plan = plan_append(&vacant, command).expect("plan managed task creation");
    let created = apply_append_plan(&vacant, &plan).expect("apply managed task creation");
    assert_eq!(
        classify_task_created_profile(&created.events()[0]).expect("known managed profile"),
        Some(TaskCreatedProfile::ManagedGeneralTaskV1)
    );
    assert_eq!(created.events()[0].subject_digest(), &expected_spec_digest);
    assert_eq!(
        verify_untrusted_autonomy_receipt_rows(&created, &[]).expect("pending receipt state"),
        VerifiedAutonomyReceiptState::PendingRequiredReceipt
    );

    let ordinary = AppendCommand::new(
        created.head().clone(),
        CommandId::new("managed-skip-receipt").expect("command"),
        CorrelationId::new("managed-correlation-1").expect("correlation"),
        "2026-08-26T00:00:01Z",
        LedgerEventKind::EvidenceRecorded,
        ActorId::new("worker").expect("actor"),
        ActionId::new("RECORD").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("RECORDED").expect("reason"),
        digest('b'),
        None,
        None,
    )
    .expect("ordinary command");
    assert_eq!(
        plan_append(&created, ordinary),
        Err(LedgerError::InvalidAutonomyReceipt)
    );
}

fn writer_authority(identity: &TaskLedgerStreamIdentity) -> WriterLeaseAuthorityHead {
    writer_authority_variant(identity, 1, 'e')
}

fn writer_authority_variant(
    identity: &TaskLedgerStreamIdentity,
    revision: u64,
    transition_digest: char,
) -> WriterLeaseAuthorityHead {
    let lease_identity = WriterLeaseIdentity::new(
        identity.project_id().clone(),
        identity.project_snapshot_id().clone(),
        identity.task_id().clone(),
        identity.task_revision(),
        identity
            .task_spec_digest()
            .expect("writer identity requires TaskSpec")
            .clone(),
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
    .expect("lease identity");
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        RuntimeKind::Live,
        lease_identity,
        WriterLeaseStatus::Active,
        WriterLeaseRevision::new(revision).expect("revision"),
        RuntimeAdmissionMode::Active,
        "2026-08-12T00:00:00Z",
        "2026-08-12T00:00:00Z",
        "2026-08-12T00:10:00Z",
        digest('c'),
        digest('d'),
        digest(transition_digest),
        digest('f'),
    )
    .expect("writer receipt")
    .head()
}

#[derive(Clone, Copy, Debug)]
enum WriterAuthoritySubstitution {
    Project,
    Attempt,
    Lease,
    Holder,
    Process,
    Daemon,
}

#[derive(Clone, Copy, Debug)]
enum AutonomyRowScalar {
    StreamId,
    EventSequence,
    EventDigest,
    ReceiptSchemaVersion,
    IntentVersion,
    TaskKind,
    RiskClass,
    ExecutionPreapproved,
    RequiresNewAuthority,
    IrreversibleOrHighRisk,
    ObservedTaskState,
    Disposition,
    DecisionReason,
    Model,
    Verification,
    AuthorityMode,
    ProcessStartAuthorityDigest,
    IngressProfileAdapterCommitment,
    StoreAuthorityHeadDigest,
    WriterLeaseReceiptDigest,
    WriterLeaseHeadDigest,
    WriterFencingToken,
    AuthorityDigest,
    ReceiptDigest,
}

const AUTONOMY_ROW_SCALARS: [AutonomyRowScalar; 24] = [
    AutonomyRowScalar::StreamId,
    AutonomyRowScalar::EventSequence,
    AutonomyRowScalar::EventDigest,
    AutonomyRowScalar::ReceiptSchemaVersion,
    AutonomyRowScalar::IntentVersion,
    AutonomyRowScalar::TaskKind,
    AutonomyRowScalar::RiskClass,
    AutonomyRowScalar::ExecutionPreapproved,
    AutonomyRowScalar::RequiresNewAuthority,
    AutonomyRowScalar::IrreversibleOrHighRisk,
    AutonomyRowScalar::ObservedTaskState,
    AutonomyRowScalar::Disposition,
    AutonomyRowScalar::DecisionReason,
    AutonomyRowScalar::Model,
    AutonomyRowScalar::Verification,
    AutonomyRowScalar::AuthorityMode,
    AutonomyRowScalar::ProcessStartAuthorityDigest,
    AutonomyRowScalar::IngressProfileAdapterCommitment,
    AutonomyRowScalar::StoreAuthorityHeadDigest,
    AutonomyRowScalar::WriterLeaseReceiptDigest,
    AutonomyRowScalar::WriterLeaseHeadDigest,
    AutonomyRowScalar::WriterFencingToken,
    AutonomyRowScalar::AuthorityDigest,
    AutonomyRowScalar::ReceiptDigest,
];

#[allow(clippy::too_many_lines)]
fn mutate_untrusted_autonomy_row(
    row: &UntrustedAutonomyReceiptRow,
    scalar: AutonomyRowScalar,
) -> UntrustedAutonomyReceiptRow {
    UntrustedAutonomyReceiptRow::new(
        if matches!(scalar, AutonomyRowScalar::StreamId) {
            digest('0')
        } else {
            row.stream_id().clone()
        },
        if matches!(scalar, AutonomyRowScalar::EventSequence) {
            row.event_sequence() + 1
        } else {
            row.event_sequence()
        },
        if matches!(scalar, AutonomyRowScalar::EventDigest) {
            digest('0')
        } else {
            row.event_digest().clone()
        },
        if matches!(scalar, AutonomyRowScalar::ReceiptSchemaVersion) {
            "lattice.autonomy-receipt/unknown"
        } else {
            row.receipt_schema_version()
        },
        if matches!(scalar, AutonomyRowScalar::IntentVersion) {
            "unknown"
        } else {
            row.intent_version()
        },
        if matches!(scalar, AutonomyRowScalar::TaskKind) {
            "BUG_FIX"
        } else {
            row.task_kind()
        },
        if matches!(scalar, AutonomyRowScalar::RiskClass) {
            "R1"
        } else {
            row.risk_class()
        },
        if matches!(scalar, AutonomyRowScalar::ExecutionPreapproved) {
            !row.execution_preapproved()
        } else {
            row.execution_preapproved()
        },
        if matches!(scalar, AutonomyRowScalar::RequiresNewAuthority) {
            !row.requires_new_authority()
        } else {
            row.requires_new_authority()
        },
        if matches!(scalar, AutonomyRowScalar::IrreversibleOrHighRisk) {
            !row.irreversible_or_high_risk()
        } else {
            row.irreversible_or_high_risk()
        },
        if matches!(scalar, AutonomyRowScalar::ObservedTaskState) {
            "COMPLETED"
        } else {
            row.observed_task_state()
        },
        if matches!(scalar, AutonomyRowScalar::Disposition) {
            "ASK_USER"
        } else {
            row.disposition()
        },
        if matches!(scalar, AutonomyRowScalar::DecisionReason) {
            "UNKNOWN_REASON"
        } else {
            row.decision_reason()
        },
        if matches!(scalar, AutonomyRowScalar::Model) {
            None
        } else {
            row.model().map(str::to_owned)
        },
        if matches!(scalar, AutonomyRowScalar::Verification) {
            None
        } else {
            row.verification().map(str::to_owned)
        },
        if matches!(scalar, AutonomyRowScalar::AuthorityMode) {
            "UNKNOWN_AUTHORITY_MODE"
        } else {
            row.authority_mode()
        },
        if matches!(scalar, AutonomyRowScalar::ProcessStartAuthorityDigest) {
            digest('0')
        } else {
            row.process_start_authority_digest().clone()
        },
        if matches!(scalar, AutonomyRowScalar::IngressProfileAdapterCommitment) {
            digest('0')
        } else {
            row.ingress_profile_adapter_commitment().clone()
        },
        if matches!(scalar, AutonomyRowScalar::StoreAuthorityHeadDigest) {
            digest('0')
        } else {
            row.store_authority_head_digest().clone()
        },
        if matches!(scalar, AutonomyRowScalar::WriterLeaseReceiptDigest) {
            None
        } else {
            row.writer_lease_receipt_digest().cloned()
        },
        if matches!(scalar, AutonomyRowScalar::WriterLeaseHeadDigest) {
            None
        } else {
            row.writer_lease_head_digest().cloned()
        },
        if matches!(scalar, AutonomyRowScalar::WriterFencingToken) {
            Some(0)
        } else {
            row.writer_fencing_token()
        },
        if matches!(scalar, AutonomyRowScalar::AuthorityDigest) {
            digest('0')
        } else {
            row.authority_digest().clone()
        },
        if matches!(scalar, AutonomyRowScalar::ReceiptDigest) {
            digest('0')
        } else {
            row.receipt_digest().clone()
        },
    )
}

fn substituted_writer_authority(
    identity: &TaskLedgerStreamIdentity,
    substitution: WriterAuthoritySubstitution,
) -> WriterLeaseAuthorityHead {
    let lease_identity = WriterLeaseIdentity::new(
        if matches!(substitution, WriterAuthoritySubstitution::Project) {
            ProjectId::new("project-2").expect("project")
        } else {
            identity.project_id().clone()
        },
        identity.project_snapshot_id().clone(),
        identity.task_id().clone(),
        identity.task_revision(),
        identity
            .task_spec_digest()
            .expect("writer substitution requires TaskSpec")
            .clone(),
        AttemptId::new(
            if matches!(substitution, WriterAuthoritySubstitution::Attempt) {
                "attempt-2"
            } else {
                "attempt-1"
            },
        )
        .expect("attempt"),
        if matches!(substitution, WriterAuthoritySubstitution::Lease) {
            "lease-2"
        } else {
            "lease-1"
        },
        if matches!(substitution, WriterAuthoritySubstitution::Holder) {
            "codex-writer-2"
        } else {
            "codex-writer-1"
        },
        "workspace-1",
        HolderProcessId::new(
            if matches!(substitution, WriterAuthoritySubstitution::Process) {
                43
            } else {
                42
            },
        )
        .expect("process"),
        digest('b'),
        if matches!(substitution, WriterAuthoritySubstitution::Daemon) {
            "daemon-2"
        } else {
            "daemon-1"
        },
        DaemonEpoch::new(1).expect("daemon epoch"),
        FencingToken::new(7).expect("fence"),
    )
    .expect("lease identity");
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        RuntimeKind::Live,
        lease_identity,
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
    .expect("writer receipt")
    .head()
}

fn append(
    head: lattice_contracts::TaskLedgerStreamHead,
    command_id: &str,
    subject: char,
) -> AppendCommand {
    AppendCommand::new(
        head,
        CommandId::new(command_id).expect("command"),
        CorrelationId::new("correlation-1").expect("correlation"),
        "2026-07-29T00:00:00Z",
        LedgerEventKind::TaskCreated,
        ActorId::new("lattice-pm").expect("actor"),
        ActionId::new("record-task").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("TASK_ACCEPTED").expect("reason"),
        digest(subject),
        None,
        None,
    )
    .expect("append command")
}

#[test]
fn generic_append_cannot_forge_autonomy_receipt_subject() {
    let zero = FakeTaskLedger::zero_head(identity("project-1", "TASK-050")).expect("zero");
    assert_eq!(
        AppendCommand::new(
            zero,
            CommandId::new("forged-autonomy").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:01Z",
            LedgerEventKind::AutonomyReceiptRecorded,
            ActorId::new("caller-controlled").expect("actor"),
            ActionId::new("RECORD_AUTONOMY_RECEIPT_V1").expect("action"),
            LedgerOutcome::Recorded,
            ReasonCode::new("AUTONOMY_DECISION_RECORDED").expect("reason"),
            digest('d'),
            None,
            None,
        ),
        Err(LedgerError::InvalidAutonomyReceipt)
    );
}

#[test]
fn generic_append_cannot_select_controlled_task_profile() {
    let zero = FakeTaskLedger::zero_head(identity("project-1", "TASK-050")).expect("zero");
    let error = AppendCommand::new(
        zero,
        CommandId::new("forged-profile").expect("command"),
        CorrelationId::new("correlation-1").expect("correlation"),
        "2026-08-13T00:00:00Z",
        LedgerEventKind::TaskCreated,
        ActorId::new("caller-controlled").expect("actor"),
        ActionId::new("CONTROLLED_CODEX_CANARY").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("TASK_ACCEPTED").expect("reason"),
        digest('c'),
        None,
        None,
    )
    .expect_err("generic append must not choose a governed task-created profile");
    assert_eq!(error.code(), "LEDGER_UNKNOWN_TASK_CREATED_PROFILE");
}

#[test]
fn generic_append_cannot_mint_managed_general_task_profile() {
    let zero =
        FakeTaskLedger::zero_head(identity("project-1", "TASK-MANAGED-1")).expect("zero head");
    let error = AppendCommand::new(
        zero,
        CommandId::new("forged-managed-profile").expect("command"),
        CorrelationId::new("correlation-1").expect("correlation"),
        "2026-08-26T00:00:00Z",
        LedgerEventKind::TaskCreated,
        ActorId::new("caller-controlled").expect("actor"),
        ActionId::new(TaskCreatedProfile::ManagedGeneralTaskV1.action()).expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("MANAGED_GENERAL_TASK_ACCEPTED").expect("reason"),
        digest('a'),
        None,
        None,
    )
    .expect_err("generic append must not mint the reserved managed profile");
    assert_eq!(error, LedgerError::UnknownTaskCreatedProfile);
}

#[test]
fn typed_task_admission_uses_required_profile_marker() {
    let stream = VerifiedStream::vacant(identity("project-1", "TASK-050"), RuntimeKind::Fake)
        .expect("vacant stream");
    let command = AppendCommand::new_autonomy_required_task_created(
        stream.head().clone(),
        CommandId::new("typed-profile").expect("command"),
        CorrelationId::new("correlation-1").expect("correlation"),
        "2026-08-13T00:00:00Z",
        ActorId::new("lattice-runtime").expect("actor"),
        ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
        digest('c'),
        None,
    )
    .expect("typed required profile");
    let plan = plan_append(&stream, command).expect("typed profile plan");
    let event = plan.new_event().expect("task-created event");
    assert_eq!(
        event.action().as_str(),
        "CONTROLLED_CODEX_CANARY_AUTONOMY_V1"
    );
    assert_eq!(
        classify_task_created_profile(event),
        Ok(Some(TaskCreatedProfile::AutonomyReceiptRequiredV1))
    );
}

#[test]
fn required_profile_is_first_and_receipt_is_immediate_sequence_two() {
    let vacant = VerifiedStream::vacant(identity("project-1", "TASK-050"), RuntimeKind::Fake)
        .expect("vacant stream");
    let historical_first = plan_append(&vacant, append(vacant.head().clone(), "first", '1'))
        .expect("ordinary first event");
    let existing = apply_append_plan(&vacant, &historical_first).expect("existing stream");
    assert_eq!(
        AppendCommand::new_autonomy_required_task_created(
            existing.head().clone(),
            CommandId::new("late-required").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('9'),
            None,
        ),
        Err(LedgerError::InvalidAutonomyReceipt)
    );

    let required_plan = plan_append(
        &vacant,
        AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new("required-first").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('9'),
            None,
        )
        .expect("required command"),
    )
    .expect("required plan");
    let pending = apply_append_plan(&vacant, &required_plan).expect("pending stream");
    assert_eq!(
        plan_append(
            &pending,
            append(pending.head().clone(), "skip-receipt", '2')
        ),
        Err(LedgerError::InvalidAutonomyReceipt)
    );
}

#[test]
fn task_ledger_owns_canonical_autonomy_plan_and_golden_digests() {
    let identity = autonomy_identity();
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Fake).expect("vacant");
    let created_plan = plan_append(
        &vacant,
        AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new("typed-profile").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('9'),
            None,
        )
        .expect("typed profile"),
    )
    .expect("create plan");
    let created = apply_append_plan(&vacant, &created_plan).expect("created stream");
    let input = AutonomyIntent::new(
        AutonomyTaskKind::Feature,
        AutonomyRiskClass::R0,
        true,
        false,
        false,
        AutonomyObservedTaskState::Draft,
        AutonomyRecommendation::Proceed {
            model: AutonomyModel::GovernedCodexWriter,
            verification: AutonomyVerification::FocusedChecks,
            reason: AutonomyDecisionReason::RoutineAuthorized,
        },
    );
    let authority = AutonomyAuthorityEvidence::new_p0_process_start_profile(
        digest('1'),
        digest('2'),
        digest('3'),
        Some(writer_authority(&identity)),
    )
    .expect("authority");
    let plan = plan_autonomy_receipt_append(
        &created,
        AutonomyAppendMetadata::new(
            CommandId::new("task050-autonomy-receipt-v1").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:01Z",
            ActorId::new("lattice-runtime").expect("actor"),
        )
        .expect("metadata"),
        input,
        authority,
    )
    .expect("autonomy plan");
    assert_eq!(plan.append_plan().new_event().expect("event").sequence(), 2);
    assert_eq!(
        plan.append_plan()
            .new_event()
            .expect("event")
            .subject_digest(),
        plan.receipt().receipt_digest()
    );
    assert_eq!(
        plan.receipt().authority_digest().as_str(),
        "076aabfeb37d459ca1e001d765c81a42c0f0a5167f01ed8cc7b0d9a6ff8b2164"
    );
    assert_eq!(
        plan.receipt().receipt_digest().as_str(),
        "ce283bd49ecba4ba040757d74bdd915aa356d7fcc0f065a83e7917ea97c53673"
    );
}

#[test]
fn autonomy_writer_head_digest_uses_only_the_owner_asserted_tuple() {
    let identity = autonomy_identity();
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Fake).expect("vacant");
    let created_plan = plan_append(
        &vacant,
        AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new("typed-profile").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('9'),
            None,
        )
        .expect("typed profile"),
    )
    .expect("create plan");
    let created = apply_append_plan(&vacant, &created_plan).expect("created stream");
    let intent = AutonomyIntent::new(
        AutonomyTaskKind::Feature,
        AutonomyRiskClass::R0,
        true,
        false,
        false,
        AutonomyObservedTaskState::Draft,
        AutonomyRecommendation::Proceed {
            model: AutonomyModel::GovernedCodexWriter,
            verification: AutonomyVerification::FocusedChecks,
            reason: AutonomyDecisionReason::RoutineAuthorized,
        },
    );
    let metadata = AutonomyAppendMetadata::new(
        CommandId::new("task050-autonomy-receipt-v1").expect("command"),
        CorrelationId::new("correlation-1").expect("correlation"),
        "2026-08-13T00:00:01Z",
        ActorId::new("lattice-runtime").expect("actor"),
    )
    .expect("metadata");
    let plan = |writer| {
        plan_autonomy_receipt_append(
            &created,
            metadata.clone(),
            intent,
            AutonomyAuthorityEvidence::new_p0_process_start_profile(
                digest('1'),
                digest('2'),
                digest('3'),
                Some(writer),
            )
            .expect("authority"),
        )
        .expect("autonomy plan")
    };
    let owner_projection = plan(writer_authority_variant(&identity, 1, 'e'));
    let substituted_unasserted_projection = plan(writer_authority_variant(&identity, 2, '6'));

    assert_eq!(
        owner_projection.receipt().writer_lease_head_digest(),
        substituted_unasserted_projection
            .receipt()
            .writer_lease_head_digest()
    );
    assert_eq!(
        owner_projection.receipt().authority_digest(),
        substituted_unasserted_projection
            .receipt()
            .authority_digest()
    );
    assert_eq!(
        owner_projection.receipt().receipt_digest(),
        substituted_unasserted_projection.receipt().receipt_digest()
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn exact_autonomy_retry_rejects_every_writer_identity_substitution() {
    let identity = autonomy_identity();
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Fake).expect("vacant");
    let created_plan = plan_append(
        &vacant,
        AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new("typed-profile").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('9'),
            None,
        )
        .expect("typed profile"),
    )
    .expect("create plan");
    let created = apply_append_plan(&vacant, &created_plan).expect("created stream");
    let intent = AutonomyIntent::new(
        AutonomyTaskKind::Feature,
        AutonomyRiskClass::R0,
        true,
        false,
        false,
        AutonomyObservedTaskState::Draft,
        AutonomyRecommendation::Proceed {
            model: AutonomyModel::GovernedCodexWriter,
            verification: AutonomyVerification::FocusedChecks,
            reason: AutonomyDecisionReason::RoutineAuthorized,
        },
    );
    let authority = AutonomyAuthorityEvidence::new_p0_process_start_profile(
        digest('1'),
        digest('2'),
        digest('3'),
        Some(writer_authority(&identity)),
    )
    .expect("authority");
    let plan = plan_autonomy_receipt_append(
        &created,
        AutonomyAppendMetadata::new(
            CommandId::new("task050-autonomy-receipt-v1").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:01Z",
            ActorId::new("lattice-runtime").expect("actor"),
        )
        .expect("metadata"),
        intent,
        authority.clone(),
    )
    .expect("autonomy plan");

    verify_exact_autonomy_receipt_retry(&identity, plan.receipt(), intent, &authority)
        .expect("exact owner retry");

    for substitution in [
        WriterAuthoritySubstitution::Project,
        WriterAuthoritySubstitution::Attempt,
        WriterAuthoritySubstitution::Lease,
        WriterAuthoritySubstitution::Holder,
        WriterAuthoritySubstitution::Process,
        WriterAuthoritySubstitution::Daemon,
    ] {
        let candidate = AutonomyAuthorityEvidence::new_p0_process_start_profile(
            digest('1'),
            digest('2'),
            digest('3'),
            Some(substituted_writer_authority(&identity, substitution)),
        )
        .expect("candidate authority");
        assert_eq!(
            verify_exact_autonomy_receipt_retry(&identity, plan.receipt(), intent, &candidate),
            Err(LedgerError::InvalidAutonomyReceipt),
            "{substitution:?} substitution must fail closed"
        );
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn untrusted_autonomy_rows_roundtrip_only_through_task_ledger() {
    let identity = autonomy_identity();
    let vacant = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let created_plan = plan_append(
        &vacant,
        AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new("typed-profile").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('9'),
            None,
        )
        .expect("typed profile"),
    )
    .expect("create plan");
    let created = apply_append_plan(&vacant, &created_plan).expect("created stream");
    assert_eq!(
        verify_untrusted_autonomy_receipt_rows(&created, &[]),
        Ok(VerifiedAutonomyReceiptState::PendingRequiredReceipt)
    );
    let plan = plan_autonomy_receipt_append(
        &created,
        AutonomyAppendMetadata::new(
            CommandId::new("task050-autonomy-receipt-v1").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:01Z",
            ActorId::new("lattice-runtime").expect("actor"),
        )
        .expect("metadata"),
        AutonomyIntent::new(
            AutonomyTaskKind::Feature,
            AutonomyRiskClass::R0,
            false,
            false,
            false,
            AutonomyObservedTaskState::Draft,
            AutonomyRecommendation::AskUser {
                reason: AutonomyDecisionReason::NewUserDecision,
            },
        ),
        AutonomyAuthorityEvidence::new_p0_process_start_profile(
            digest('1'),
            digest('2'),
            digest('3'),
            None,
        )
        .expect("authority"),
    )
    .expect("autonomy plan");
    let completed = apply_append_plan(&created, plan.append_plan()).expect("complete stream");
    let row = plan.receipt().to_untrusted();
    assert_eq!(
        verify_untrusted_autonomy_receipt_rows(&completed, std::slice::from_ref(&row)),
        Ok(VerifiedAutonomyReceiptState::RequiredComplete(
            plan.receipt().clone()
        ))
    );

    let later_plan = plan_append(
        &completed,
        append(completed.head().clone(), "post-receipt-event", '7'),
    )
    .expect("later lifecycle event");
    let later = apply_append_plan(&completed, &later_plan).expect("later stream");
    assert_eq!(
        verify_untrusted_autonomy_receipt_rows(&later, std::slice::from_ref(&row)),
        Ok(VerifiedAutonomyReceiptState::RequiredComplete(
            plan.receipt().clone()
        ))
    );

    let proceed_plan = plan_autonomy_receipt_append(
        &created,
        AutonomyAppendMetadata::new(
            CommandId::new("task050-autonomy-proceed-v1").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:02Z",
            ActorId::new("lattice-runtime").expect("actor"),
        )
        .expect("metadata"),
        AutonomyIntent::new(
            AutonomyTaskKind::Feature,
            AutonomyRiskClass::R0,
            true,
            false,
            false,
            AutonomyObservedTaskState::Draft,
            AutonomyRecommendation::Proceed {
                model: AutonomyModel::GovernedCodexWriter,
                verification: AutonomyVerification::FocusedChecks,
                reason: AutonomyDecisionReason::RoutineAuthorized,
            },
        ),
        AutonomyAuthorityEvidence::new_p0_process_start_profile(
            digest('1'),
            digest('2'),
            digest('3'),
            Some(writer_authority(created.identity())),
        )
        .expect("authority"),
    )
    .expect("proceed autonomy plan");
    let proceed_completed =
        apply_append_plan(&created, proceed_plan.append_plan()).expect("proceed complete stream");
    let proceed_row = proceed_plan.receipt().to_untrusted();
    assert_eq!(
        verify_untrusted_autonomy_receipt_rows(
            &proceed_completed,
            std::slice::from_ref(&proceed_row),
        ),
        Ok(VerifiedAutonomyReceiptState::RequiredComplete(
            proceed_plan.receipt().clone()
        ))
    );

    assert_eq!(AUTONOMY_ROW_SCALARS.len(), 24);
    for scalar in AUTONOMY_ROW_SCALARS {
        let corrupted = mutate_untrusted_autonomy_row(&proceed_row, scalar);
        assert_eq!(
            verify_untrusted_autonomy_receipt_rows(&proceed_completed, &[corrupted]),
            Err(LedgerError::InvalidAutonomyReceipt),
            "{scalar:?} corruption must fail closed"
        );
    }
}

#[test]
fn typed_autonomy_plan_rejects_substituted_recommendation() {
    let identity = autonomy_identity();
    let vacant = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let created_plan = plan_append(
        &vacant,
        AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new("typed-profile").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('9'),
            None,
        )
        .expect("typed profile"),
    )
    .expect("create plan");
    let created = apply_append_plan(&vacant, &created_plan).expect("created stream");
    let substituted = AutonomyIntent::new(
        AutonomyTaskKind::Feature,
        AutonomyRiskClass::R0,
        false,
        false,
        false,
        AutonomyObservedTaskState::Draft,
        AutonomyRecommendation::Proceed {
            model: AutonomyModel::GovernedCodexWriter,
            verification: AutonomyVerification::FocusedChecks,
            reason: AutonomyDecisionReason::RoutineAuthorized,
        },
    );
    let error = plan_autonomy_receipt_append(
        &created,
        AutonomyAppendMetadata::new(
            CommandId::new("task050-autonomy-receipt-v1").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:01Z",
            ActorId::new("lattice-runtime").expect("actor"),
        )
        .expect("metadata"),
        substituted,
        AutonomyAuthorityEvidence::new_p0_process_start_profile(
            digest('1'),
            digest('2'),
            digest('3'),
            None,
        )
        .expect("authority"),
    )
    .expect_err("substituted recommendation");
    assert_eq!(error.code(), "LEDGER_AUTONOMY_RECOMMENDATION_MISMATCH");
}

#[test]
fn typed_autonomy_plan_rejects_every_non_p0_risk() {
    let identity = autonomy_identity();
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Fake).expect("vacant");
    let created_plan = plan_append(
        &vacant,
        AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new("typed-profile").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('9'),
            None,
        )
        .expect("typed profile"),
    )
    .expect("create plan");
    let created = apply_append_plan(&vacant, &created_plan).expect("created stream");
    let cases = [
        (
            AutonomyRiskClass::R1,
            AutonomyRecommendation::Proceed {
                model: AutonomyModel::GovernedCodexWriter,
                verification: AutonomyVerification::FocusedChecks,
                reason: AutonomyDecisionReason::RoutineAuthorized,
            },
            Some(writer_authority(&identity)),
        ),
        (
            AutonomyRiskClass::R2,
            AutonomyRecommendation::Proceed {
                model: AutonomyModel::GovernedCodexWriter,
                verification: AutonomyVerification::BuildAndFocusedChecks,
                reason: AutonomyDecisionReason::RoutineAuthorized,
            },
            Some(writer_authority(&identity)),
        ),
        (
            AutonomyRiskClass::R3,
            AutonomyRecommendation::AskUser {
                reason: AutonomyDecisionReason::HighRiskOrIrreversible,
            },
            None,
        ),
    ];
    for (index, (risk, recommendation, writer)) in cases.into_iter().enumerate() {
        let error = plan_autonomy_receipt_append(
            &created,
            AutonomyAppendMetadata::new(
                CommandId::new(format!("non-p0-{index}")).expect("command"),
                CorrelationId::new("correlation-1").expect("correlation"),
                "2026-08-13T00:00:01Z",
                ActorId::new("lattice-runtime").expect("actor"),
            )
            .expect("metadata"),
            AutonomyIntent::new(
                AutonomyTaskKind::Feature,
                risk,
                true,
                false,
                false,
                AutonomyObservedTaskState::Draft,
                recommendation,
            ),
            AutonomyAuthorityEvidence::new_p0_process_start_profile(
                digest('1'),
                digest('2'),
                digest('3'),
                writer,
            )
            .expect("authority"),
        )
        .expect_err("non-P0 risk must fail closed");
        assert_eq!(error, LedgerError::InvalidAutonomyReceipt);
    }
}

#[test]
fn autonomy_receipt_event_is_closed_ordered_and_exactly_once() {
    assert_eq!(
        LedgerEventKind::parse("AUTONOMY_RECEIPT_RECORDED"),
        Ok(LedgerEventKind::AutonomyReceiptRecorded)
    );
    assert_eq!(
        LedgerEventKind::parse("AUTONOMY_RECEIPT_V2"),
        Err(LedgerError::UnknownEventKind)
    );

    let vacant = VerifiedStream::vacant(autonomy_identity(), RuntimeKind::Fake).expect("vacant");
    let autonomy_input = || {
        AutonomyIntent::new(
            AutonomyTaskKind::Feature,
            AutonomyRiskClass::R0,
            false,
            false,
            false,
            AutonomyObservedTaskState::Draft,
            AutonomyRecommendation::AskUser {
                reason: AutonomyDecisionReason::NewUserDecision,
            },
        )
    };
    let autonomy_authority = || {
        AutonomyAuthorityEvidence::new_p0_process_start_profile(
            digest('1'),
            digest('2'),
            digest('3'),
            None,
        )
        .expect("authority")
    };
    let autonomy_metadata = |command_id: &str| {
        AutonomyAppendMetadata::new(
            CommandId::new(command_id).expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:01Z",
            ActorId::new("lattice-runtime").expect("actor"),
        )
        .expect("metadata")
    };
    assert_eq!(
        plan_autonomy_receipt_append(
            &vacant,
            autonomy_metadata("autonomy-before-create"),
            autonomy_input(),
            autonomy_authority(),
        ),
        Err(LedgerError::InvalidAutonomyReceipt)
    );
    let created_plan = plan_append(
        &vacant,
        AppendCommand::new_autonomy_required_task_created(
            vacant.head().clone(),
            CommandId::new("create").expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-13T00:00:00Z",
            ActorId::new("lattice-runtime").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('c'),
            None,
        )
        .expect("create command"),
    )
    .expect("create plan");
    let created = apply_append_plan(&vacant, &created_plan).expect("created");
    let receipt_plan = plan_autonomy_receipt_append(
        &created,
        autonomy_metadata("autonomy"),
        autonomy_input(),
        autonomy_authority(),
    )
    .expect("receipt plan");
    assert_eq!(
        receipt_plan.receipt().authority_digest().as_str(),
        "f83650cc14a6e05b1150597fa80ce26131f8a6d69d2280c7c82b567892b2bb1f"
    );
    assert_eq!(
        receipt_plan.receipt().receipt_digest().as_str(),
        "68d59dd274d151d0c37ca6bfaceafbbb12d7b5c424f34370aeb6f5352037b536"
    );
    let receipt = apply_append_plan(&created, receipt_plan.append_plan()).expect("receipt");
    assert_eq!(receipt.head().sequence(), 2);
    assert_eq!(
        plan_autonomy_receipt_append(
            &receipt,
            autonomy_metadata("autonomy-2"),
            autonomy_input(),
            autonomy_authority(),
        ),
        Err(LedgerError::InvalidAutonomyReceipt)
    );
}

#[test]
fn complete_identity_produces_deterministic_zero_head_and_distinct_streams() {
    let first = FakeTaskLedger::zero_head(identity("project-1", "TASK-013")).expect("head");
    let same = FakeTaskLedger::zero_head(identity("project-1", "TASK-013")).expect("head");
    let other_project = FakeTaskLedger::zero_head(identity("project-2", "TASK-013")).expect("head");

    assert_eq!(first, same);
    assert!(first.is_zero());
    assert_eq!(first.sequence(), 0);
    assert_ne!(first.stream_id(), other_project.stream_id());

    for invalid in ["TASK-_ABC", "TASK--ABC"] {
        assert_eq!(
            FakeTaskLedger::zero_head(identity("project-1", invalid)),
            Err(LedgerError::InvalidIdentifier { field: "task_id" }),
            "Task Ledger must enforce the same leading suffix character as Task Domain"
        );
    }
}

#[test]
fn append_retry_and_cross_stream_command_scope_are_exact() {
    let mut ledger = FakeTaskLedger::new();
    let zero = FakeTaskLedger::zero_head(identity("project-1", "TASK-013")).expect("zero");
    let first = ledger
        .execute(append(zero.clone(), "command-1", 'b'))
        .expect("append");
    let first_head = first.after().clone();
    assert_eq!(first.outcome(), &CommandOutcome::Appended);
    assert_eq!(first_head.sequence(), 1);

    let second = ledger
        .execute(append(first_head, "command-2", 'c'))
        .expect("second");
    assert_eq!(second.after().sequence(), 2);

    let retry = ledger
        .execute(append(zero, "command-1", 'b'))
        .expect("exact retry");
    assert_eq!(retry, first);
    assert_eq!(
        ledger
            .current_head(first.after().stream_id())
            .expect("current")
            .sequence(),
        2
    );

    let other_zero = FakeTaskLedger::zero_head(identity("project-2", "TASK-013")).expect("zero");
    let other = ledger
        .execute(append(other_zero, "command-1", 'b'))
        .expect("same command in another stream");
    assert_eq!(other.after().sequence(), 1);
    assert_ne!(other.after().stream_id(), first.after().stream_id());
}

#[test]
fn changed_retry_rejects_and_stale_new_command_is_stable_without_stream_mutation() {
    let mut ledger = FakeTaskLedger::new();
    let zero = FakeTaskLedger::zero_head(identity("project-1", "TASK-013")).expect("zero");
    let first = ledger
        .execute(append(zero.clone(), "command-1", 'b'))
        .expect("append");

    assert!(matches!(
        ledger.execute(append(zero.clone(), "command-1", 'c')),
        Err(LedgerError::CommandIdReuse)
    ));

    let stale = ledger
        .execute(append(zero.clone(), "stale-command", 'd'))
        .expect("terminal denial");
    assert_eq!(
        stale.outcome(),
        &CommandOutcome::Denied(LedgerDenial::StaleHead)
    );
    assert_eq!(stale.before(), stale.after());
    assert_eq!(
        ledger
            .current_head(first.after().stream_id())
            .expect("current"),
        first.after().clone()
    );
    assert_eq!(
        ledger
            .execute(append(zero, "stale-command", 'd'))
            .expect("stable denial retry"),
        stale
    );
}

#[test]
fn uncreated_stream_terminal_denial_exports_through_public_replay_boundary() {
    let mut source = FakeTaskLedger::new();
    let zero = FakeTaskLedger::zero_head(identity("project-1", "TASK-013")).expect("zero");
    let existing = source
        .execute(append(zero, "source-command", 'b'))
        .expect("source append");

    let mut empty = FakeTaskLedger::new();
    let denied = empty
        .execute(append(
            existing.after().clone(),
            "stale-uncreated-command",
            'c',
        ))
        .expect("terminal stale denial");
    assert_eq!(
        denied.outcome(),
        &CommandOutcome::Denied(LedgerDenial::StaleHead)
    );
    assert!(empty.current_head(existing.after().stream_id()).is_none());

    let snapshot = empty
        .untrusted_snapshot(existing.after().stream_id())
        .expect("terminal command must remain exportable");
    let verified = verify_untrusted_snapshot(&snapshot).expect("public replay");
    assert!(verified.head().is_zero());
    assert_eq!(verified.head(), denied.after());
}

#[test]
fn resource_projection_observation_and_currentness_are_owner_bound() {
    let mut ledger = FakeTaskLedger::new();
    let zero = FakeTaskLedger::zero_head(identity("project-1", "TASK-013")).expect("zero");
    let first = ledger
        .execute(append(zero, "command-1", 'b'))
        .expect("append");
    let counters = ResourceCounters::new(2, 1, 60, 1, 3, "1.5").expect("counters");
    let resource = AppendCommand::new(
        first.after().clone(),
        CommandId::new("resource-1").expect("command"),
        CorrelationId::new("correlation-1").expect("correlation"),
        "2026-07-29T00:01:00Z",
        LedgerEventKind::ResourceSnapshot,
        ActorId::new("runtime-supervisor").expect("actor"),
        ActionId::new("record-resources").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("RESOURCE_SNAPSHOT").expect("reason"),
        digest('d'),
        None,
        Some(ResourceSnapshot::new(counters.clone())),
    )
    .expect("resource command");
    let updated = ledger.execute(resource).expect("resource append");

    let request = ResourceRequest::new(1, 0, 30, 0, 1, Some("0.5")).expect("request");
    let receipt = ledger
        .issue_resource_observation(
            updated.after().clone(),
            &EffectClaimId::new("effect-claim-1").expect("claim"),
            digest('e'),
            request,
        )
        .expect("observation");
    assert_eq!(receipt.counters(), &counters);
    assert_eq!(ledger.current_resource_head(&receipt), Some(receipt.head()));

    let later = ledger
        .execute(append(
            updated.after().clone(),
            "command-after-observation",
            'f',
        ))
        .expect("later append");
    assert_eq!(later.after().sequence(), 3);
    assert_eq!(ledger.current_resource_head(&receipt), None);
}

#[test]
fn diagnostics_are_bounded_sanitized_and_never_authoritative() {
    let diagnostic = Diagnostic::new(CanonicalValue::Object(vec![
        (
            "message".to_owned(),
            CanonicalValue::String("Bearer abcdefghijklmnop".to_owned()),
        ),
        (
            "api_key".to_owned(),
            CanonicalValue::String("sk-super-secret-value".to_owned()),
        ),
        (
            "apiKey".to_owned(),
            CanonicalValue::String("ghp_abcdefghijklmnopqrstuvwxyz".to_owned()),
        ),
        (
            "ordinary".to_owned(),
            CanonicalValue::String("TASK-013".to_owned()),
        ),
    ]))
    .expect("sanitized");
    let rendered = format!("{diagnostic:?}");
    assert!(rendered.contains("[REDACTED]"));
    assert!(!rendered.contains("abcdefghijklmnop"));
    assert!(!rendered.contains("super-secret"));
    assert!(!rendered.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
    assert!(rendered.contains("TASK-013"));

    let too_deep = (0..18).fold(CanonicalValue::Null, |value, _| {
        CanonicalValue::Array(vec![value])
    });
    assert!(matches!(
        Diagnostic::new(too_deep),
        Err(LedgerError::DiagnosticLimitExceeded)
    ));

    assert_eq!(
        Diagnostic::new(CanonicalValue::Object(vec![(
            "ghp_abcdefghijklmnopqrstuvwxyz".to_owned(),
            CanonicalValue::String("value".to_owned()),
        )])),
        Err(LedgerError::InvalidDiagnostic)
    );
    assert!(matches!(
        Diagnostic::new(CanonicalValue::Object(vec![(
            "secret".to_owned(),
            CanonicalValue::String("hidden\0value".to_owned()),
        )])),
        Err(LedgerError::NonCanonicalText { .. })
    ));
    let nested_secret = (0..18).fold(CanonicalValue::Null, |value, _| {
        CanonicalValue::Array(vec![value])
    });
    assert!(matches!(
        Diagnostic::new(CanonicalValue::Object(vec![(
            "secret".to_owned(),
            nested_secret,
        )])),
        Err(LedgerError::DiagnosticLimitExceeded)
    ));
    assert!(matches!(
        Diagnostic::new(CanonicalValue::Object(vec![(
            "password".to_owned(),
            CanonicalValue::String("x".repeat(20 * 1024)),
        )])),
        Err(LedgerError::DiagnosticLimitExceeded)
    ));
    let embedded_token = Diagnostic::new(CanonicalValue::String(
        "prefixghp_abcdefghijklmnopqrstuvwxyz".to_owned(),
    ))
    .expect("recognized embedded token is sanitized");
    assert!(!format!("{embedded_token:?}").contains("ghp_abcdefghijklmnopqrstuvwxyz"));
}

#[test]
fn recognized_secret_shapes_cannot_enter_authoritative_identifiers() {
    assert!(matches!(
        ActorId::new("ghp_abcdefghijklmnopqrstuvwxyz"),
        Err(LedgerError::InvalidIdentifier { field: "actor_id" })
    ));
    assert!(matches!(
        ActorId::new("prefixghp_abcdefghijklmnopqrstuvwxyz"),
        Err(LedgerError::InvalidIdentifier { field: "actor_id" })
    ));
    assert!(
        ActionId::new("task-013").is_ok(),
        "the sk- substring inside task- must not be a false positive"
    );

    let secret_project = TaskLedgerStreamIdentity::new(
        ProjectId::new("ghp_abcdefghijklmnopqrstuvwxyz").expect("project shape"),
        lattice_contracts::ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        lattice_contracts::TaskId::new("TASK-013").expect("task"),
        "1",
        digest('a'),
        "TWD",
    )
    .expect("shared identity shape");
    assert!(matches!(
        FakeTaskLedger::zero_head(secret_project),
        Err(LedgerError::InvalidIdentifier {
            field: "project_id"
        })
    ));
}

#[test]
fn public_untrusted_snapshot_verifier_rejects_storage_corruption() {
    let mut ledger = FakeTaskLedger::new();
    let zero = FakeTaskLedger::zero_head(identity("project-1", "TASK-013")).expect("zero");
    let first = ledger
        .execute(append(zero, "command-1", 'b'))
        .expect("append");
    let stream_id = first.after().stream_id().clone();
    let snapshot = ledger
        .untrusted_snapshot(&stream_id)
        .expect("untrusted snapshot");
    assert_eq!(
        verify_untrusted_snapshot(&snapshot)
            .expect("verified")
            .head(),
        first.after()
    );

    let mut unknown_schema = snapshot.clone();
    unknown_schema.events[0].schema_version = "9.0".to_owned();
    assert_eq!(
        verify_untrusted_snapshot(&unknown_schema),
        Err(LedgerError::UnknownEventVersion)
    );

    let mut unknown_kind = snapshot.clone();
    unknown_kind.events[0].kind = "ARBITRARY_EVENT".to_owned();
    assert_eq!(
        verify_untrusted_snapshot(&unknown_kind),
        Err(LedgerError::UnknownEventKind)
    );

    let mut tampered = snapshot.clone();
    tampered.events[0].subject_digest = digest('f');
    assert_eq!(
        verify_untrusted_snapshot(&tampered),
        Err(LedgerError::RequestBindingMismatch)
    );

    let mut raw_secret = snapshot.clone();
    raw_secret.events[0].diagnostic = Some(CanonicalValue::Object(vec![(
        "message".to_owned(),
        CanonicalValue::String("ghp_abcdefghijklmnopqrstuvwxyz".to_owned()),
    )]));
    assert!(!format!("{raw_secret:?}").contains("ghp_abcdefghijklmnopqrstuvwxyz"));

    let mut orphan = snapshot;
    orphan.commands.clear();
    assert_eq!(
        verify_untrusted_snapshot(&orphan),
        Err(LedgerError::OrphanReceipt)
    );
}

#[test]
fn runtime_aware_vacant_plan_apply_and_exact_retry_are_pure() {
    let vacant = VerifiedStream::vacant(identity("project-1", "TASK-021"), RuntimeKind::Live)
        .expect("live structural genesis");
    assert_eq!(vacant.runtime(), RuntimeKind::Live);
    assert!(vacant.head().is_zero());
    assert!(vacant.commands().is_empty());
    assert!(vacant.outboxes().is_empty());

    let command = append(vacant.head().clone(), "command-1", 'b');
    let plan = plan_append(&vacant, command.clone()).expect("pure append plan");
    assert!(!plan.is_exact_retry());
    assert_eq!(
        vacant.head().sequence(),
        0,
        "planning must not mutate input"
    );
    assert_eq!(plan.base_checkpoint(), vacant.checkpoint());
    assert_ne!(plan.next_checkpoint(), plan.base_checkpoint());
    assert!(plan.new_command().is_some());
    assert!(plan.new_event().is_some());
    assert!(plan.new_outbox().is_none());

    let applied = apply_append_plan(&vacant, &plan).expect("matching base checkpoint");
    assert_eq!(&applied, plan.next_state());
    assert_eq!(applied.head().sequence(), 1);
    assert_eq!(
        applied.receipt(&CommandId::new("command-1").expect("command")),
        Some(plan.receipt())
    );

    let retry = plan_append(&applied, command).expect("exact retry before stale evaluation");
    assert!(retry.is_exact_retry());
    assert!(retry.new_command().is_none());
    assert!(retry.new_event().is_none());
    assert!(retry.new_outbox().is_none());
    assert_eq!(retry.base_checkpoint(), retry.next_checkpoint());
    assert_eq!(retry.receipt(), plan.receipt());

    assert_eq!(
        apply_append_plan(&applied, &plan),
        Err(LedgerError::CheckpointMismatch)
    );
}

#[test]
fn only_recorded_appended_effect_intent_derives_one_outbox_admission() {
    let vacant = VerifiedStream::vacant(identity("project-1", "TASK-021"), RuntimeKind::Live)
        .expect("live structural genesis");
    let effect = |head, command_id, outcome, subject| {
        AppendCommand::new(
            head,
            CommandId::new(command_id).expect("command"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-07-29T00:00:00Z",
            LedgerEventKind::EffectIntent,
            ActorId::new("orchestrator").expect("actor"),
            ActionId::new("admit-effect").expect("action"),
            outcome,
            ReasonCode::new("EFFECT_AUDIT").expect("reason"),
            digest(subject),
            None,
            None,
        )
        .expect("effect command")
    };

    let recorded_command = effect(
        vacant.head().clone(),
        "effect-recorded",
        LedgerOutcome::Recorded,
        'b',
    );
    let recorded = plan_append(&vacant, recorded_command).expect("recorded effect plan");
    let admission = recorded.new_outbox().expect("one admission");
    assert_eq!(admission.state(), OutboxAdmissionState::Admitted);
    assert_eq!(
        admission.intent_digest(),
        recorded
            .new_event()
            .expect("appended event")
            .subject_digest()
    );
    let after_recorded = apply_append_plan(&vacant, &recorded).expect("apply");
    assert_eq!(after_recorded.outboxes(), std::slice::from_ref(admission));

    let failed = plan_append(
        &after_recorded,
        effect(
            after_recorded.head().clone(),
            "effect-failed",
            LedgerOutcome::Failed,
            'c',
        ),
    )
    .expect("non-recorded effect still appends");
    assert_eq!(failed.receipt().outcome(), &CommandOutcome::Appended);
    assert!(failed.new_event().is_some());
    assert!(failed.new_outbox().is_none());
    let after_failed = apply_append_plan(&after_recorded, &failed).expect("apply");

    let non_effect = plan_append(
        &after_failed,
        append(after_failed.head().clone(), "ordinary-event", 'd'),
    )
    .expect("ordinary append");
    assert!(non_effect.new_outbox().is_none());
    let after_non_effect = apply_append_plan(&after_failed, &non_effect).expect("apply");

    let stale = plan_append(
        &after_non_effect,
        effect(
            vacant.head().clone(),
            "stale-effect",
            LedgerOutcome::Recorded,
            'e',
        ),
    )
    .expect("terminal stale denial");
    assert_eq!(
        stale.receipt().outcome(),
        &CommandOutcome::Denied(LedgerDenial::StaleHead)
    );
    assert!(stale.new_event().is_none());
    assert!(stale.new_outbox().is_none());
    assert_eq!(stale.receipt().before(), stale.receipt().after());
    assert_ne!(stale.base_checkpoint(), stale.next_checkpoint());
}

#[test]
fn independent_checkpoint_binds_commands_events_projection_and_outbox() {
    let vacant = VerifiedStream::vacant(identity("project-1", "TASK-021"), RuntimeKind::Live)
        .expect("live structural genesis");
    let first_command = append(vacant.head().clone(), "command-z", 'b');
    let first = plan_append(&vacant, first_command.clone()).expect("first plan");
    let first_record_set = first.record_set_digest().clone();
    let after_first = apply_append_plan(&vacant, &first).expect("first apply");

    let effect_command = AppendCommand::new(
        after_first.head().clone(),
        CommandId::new("command-a").expect("command"),
        CorrelationId::new("correlation-1").expect("correlation"),
        "2026-07-29T00:00:01Z",
        LedgerEventKind::EffectIntent,
        ActorId::new("orchestrator").expect("actor"),
        ActionId::new("admit-effect").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("EFFECT_RECORDED").expect("reason"),
        digest('c'),
        None,
        None,
    )
    .expect("effect command");
    let effect = plan_append(&after_first, effect_command).expect("effect plan");
    let after_effect = apply_append_plan(&after_first, &effect).expect("effect apply");

    let stale = plan_append(
        &after_effect,
        append(vacant.head().clone(), "command-m", 'd'),
    )
    .expect("durable stale denial");
    let complete = apply_append_plan(&after_effect, &stale).expect("denial apply");
    assert_eq!(complete.events().len(), 2);
    assert_eq!(complete.commands().len(), 3);
    assert_eq!(complete.outboxes().len(), 1);
    assert_eq!(
        complete
            .commands()
            .iter()
            .map(|record| record.request().command_id().as_str())
            .collect::<Vec<_>>(),
        vec!["command-a", "command-m", "command-z"],
        "verified command order is canonical rather than append order"
    );

    let retry = plan_append(&complete, first_command).expect("retry after later work");
    assert!(retry.is_exact_retry());
    assert_eq!(retry.record_set_digest(), &first_record_set);
    assert_eq!(
        retry.command_record().result_checkpoint(),
        first.command_record().result_checkpoint(),
        "exact retry retains the original result checkpoint"
    );

    let retained = LedgerCheckpoint::from_retained(
        complete.checkpoint().stream_id().clone(),
        complete.checkpoint().runtime(),
        complete.checkpoint().checkpoint_digest().clone(),
    );
    let mut snapshot = export_untrusted_snapshot(&complete);
    snapshot.commands.reverse();
    let replayed = verify_untrusted_snapshot_against_checkpoint(&snapshot, &retained)
        .expect("query order is not a hash input");
    assert_eq!(replayed, complete);
    assert_eq!(
        replayed.receipt(&CommandId::new("command-m").expect("command")),
        Some(stale.receipt())
    );
    let restarted_retry = plan_append(&replayed, append(vacant.head().clone(), "command-z", 'b'))
        .expect("typed exact retry after replay");
    assert!(restarted_retry.is_exact_retry());
    assert_eq!(restarted_retry.receipt(), first.receipt());
    assert_eq!(
        restarted_retry.command_record().base_checkpoint(),
        first.command_record().base_checkpoint()
    );
    assert_eq!(
        restarted_retry.command_record().result_checkpoint(),
        first.command_record().result_checkpoint()
    );

    assert_checkpoint_corruption_matrix(&snapshot, &retained, &vacant);
}

fn assert_checkpoint_corruption_matrix(
    snapshot: &lattice_task_ledger::UntrustedLedgerSnapshot,
    retained: &LedgerCheckpoint,
    vacant: &VerifiedStream,
) {
    let mut truncated_denial = snapshot.clone();
    truncated_denial
        .commands
        .retain(|record| record.command_id != "command-m");
    assert_eq!(
        verify_untrusted_snapshot_against_checkpoint(&truncated_denial, retained),
        Err(LedgerError::CheckpointMismatch)
    );

    let mut tampered_outbox = snapshot.clone();
    tampered_outbox.outboxes[0].intent_digest = digest('f');
    assert_eq!(
        verify_untrusted_snapshot_against_checkpoint(&tampered_outbox, retained),
        Err(LedgerError::OutboxBindingMismatch)
    );

    let mut missing_outbox = snapshot.clone();
    missing_outbox.outboxes.clear();
    assert_eq!(
        verify_untrusted_snapshot_against_checkpoint(&missing_outbox, retained),
        Err(LedgerError::OutboxBindingMismatch)
    );

    let mut injected_outbox = snapshot.clone();
    injected_outbox
        .outboxes
        .push(injected_outbox.outboxes[0].clone());
    assert_eq!(
        verify_untrusted_snapshot_against_checkpoint(&injected_outbox, retained),
        Err(LedgerError::OutboxBindingMismatch)
    );

    let mut duplicated_command = snapshot.clone();
    duplicated_command
        .commands
        .push(duplicated_command.commands[0].clone());
    assert_eq!(
        verify_untrusted_snapshot_against_checkpoint(&duplicated_command, retained),
        Err(LedgerError::ReceiptBindingMismatch)
    );

    let mut wrong_command_checkpoint = snapshot.clone();
    wrong_command_checkpoint.commands[0].result_checkpoint = vacant.checkpoint().clone();
    assert_eq!(
        verify_untrusted_snapshot_against_checkpoint(&wrong_command_checkpoint, retained),
        Err(LedgerError::CheckpointMismatch)
    );
}

#[test]
fn legacy_v2_request_event_head_and_receipt_hash_fixture_is_stable() {
    let vacant = VerifiedStream::vacant(identity("project-1", "TASK-013"), RuntimeKind::Fake)
        .expect("fake structural genesis");
    let plan = plan_append(&vacant, append(vacant.head().clone(), "command-1", 'b')).expect("plan");
    assert_eq!(
        vacant.head().stream_id().as_str(),
        "09afa097a44d041b57ac2b535d22c3dbc8bc50adfe9b4d78d727df5b634af7c0"
    );
    assert_eq!(
        vacant.head().head_digest().as_str(),
        "e43fe893b303a4104a8cea21ab36b97e65d05dbb1a742ff5c7818a804c0377c6"
    );
    assert_eq!(
        plan.receipt().request_digest().as_str(),
        "65d45916025e4e9511c9611ffb927e07769a0ab9dad2c60557cd0973b2a41bff"
    );
    assert_eq!(
        plan.new_event().expect("event").event_digest().as_str(),
        "1ac556d35a2fc9ca1da2e6d2f8453a5cf0100d8c95341ceaf8df2d6adaff96b6"
    );
    assert_eq!(
        plan.receipt().after().head_digest().as_str(),
        "c08389d640c299610334f2ac6b68cbde45b2be1725d6a03ed249ade7bf5ad82f"
    );
    assert_eq!(
        plan.receipt().receipt_digest().as_str(),
        "599516b91e8e0e932b6682b1017136ccfaca6934c9292d727b27185353b28810"
    );
}

#[test]
fn fake_execution_is_byte_equal_to_the_shared_pure_planner() {
    let vacant = VerifiedStream::vacant(identity("project-1", "TASK-021"), RuntimeKind::Fake)
        .expect("vacant");
    let command = append(vacant.head().clone(), "command-1", 'b');
    let plan = plan_append(&vacant, command.clone()).expect("pure plan");

    let mut fake = FakeTaskLedger::new();
    let receipt = fake.execute(command).expect("fake execute");
    assert_eq!(&receipt, plan.receipt());
    assert_eq!(
        fake.verified_stream(receipt.after().stream_id())
            .expect("fake replay"),
        plan.next_state().clone()
    );

    let stale_command = append(vacant.head().clone(), "stale-command", 'c');
    let pure_stale = plan_append(plan.next_state(), stale_command.clone()).expect("pure denial");
    let fake_stale = fake.execute(stale_command).expect("fake denial");
    assert_eq!(&fake_stale, pure_stale.receipt());
    assert_eq!(
        fake.verified_stream(receipt.after().stream_id())
            .expect("fake denial replay"),
        pure_stale.next_state().clone()
    );
}

#[test]
fn local_verified_result_has_typed_terminal_replay_and_substitution_guards() {
    use lattice_task_ledger::{
        LocalVerifiedResultAdoption, export_untrusted_snapshot,
        verify_untrusted_snapshot_against_checkpoint,
    };
    let submission = general_submission();
    let vacant = VerifiedStream::vacant(submission.identity().clone(), RuntimeKind::Live).unwrap();
    let created = apply_append_plan(
        &vacant,
        &plan_append(
            &vacant,
            AppendCommand::new_general_task_created(
                vacant.head().clone(),
                CommandId::new(format!("mcp-submit:{}", submission.client_request_id())).unwrap(),
                CorrelationId::new("general-task-intake-v1").unwrap(),
                "2000-01-01T00:00:00Z",
                ActorId::new("lattice-mcp").unwrap(),
                &submission,
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let artifact = format!("evidence:sha256:{}", "a".repeat(64));
    let acceptance = format!("evidence:sha256:{}", "b".repeat(64));
    let adoption = LocalVerifiedResultAdoption::new(
        submission.task_ref().clone(),
        submission.client_request_id(),
        created.head().head_digest().clone(),
        artifact,
        acceptance.clone(),
    )
    .unwrap();
    let command = AppendCommand::new_local_verified_result_adopted(
        created.head().clone(),
        "2026-09-05T02:00:00Z",
        ActorId::new("lattice-mcp").unwrap(),
        &adoption,
    )
    .unwrap();
    assert!(
        AppendCommand::new(
            created.head().clone(),
            CommandId::new(adoption.command_id()).unwrap(),
            CorrelationId::new("general-task-intake-v1").unwrap(),
            "2026-09-05T02:00:00Z",
            LedgerEventKind::EvidenceRecorded,
            ActorId::new("lattice-mcp").unwrap(),
            ActionId::new("LOCAL_VERIFIED_RESULT_ADOPTED").unwrap(),
            LedgerOutcome::Recorded,
            ReasonCode::new("LOCAL_VERIFIED_RESULT_ADOPTED").unwrap(),
            adoption.result_digest().clone(),
            None,
            None
        )
        .is_err()
    );
    let completed =
        apply_append_plan(&created, &plan_append(&created, command.clone()).unwrap()).unwrap();
    let replayed = verify_untrusted_snapshot_against_checkpoint(
        &export_untrusted_snapshot(&completed),
        completed.checkpoint(),
    )
    .unwrap();
    assert_eq!(replayed.events().len(), 2);
    assert_eq!(
        replayed.events()[1].subject_digest(),
        adoption.result_digest()
    );
    assert!(plan_append(&replayed, command).unwrap().is_exact_retry());
    let changed = LocalVerifiedResultAdoption::new(
        submission.task_ref().clone(),
        submission.client_request_id(),
        created.head().head_digest().clone(),
        format!("evidence:sha256:{}", "c".repeat(64)),
        acceptance,
    )
    .unwrap();
    let changed_command = AppendCommand::new_local_verified_result_adopted(
        created.head().clone(),
        "2026-09-05T02:00:00Z",
        ActorId::new("lattice-mcp").unwrap(),
        &changed,
    )
    .unwrap();
    assert_eq!(
        plan_append(&replayed, changed_command),
        Err(LedgerError::CommandIdReuse)
    );
    assert!(
        AppendCommand::new_local_verified_result_adopted(
            completed.head().clone(),
            "2026-09-05T02:00:01Z",
            ActorId::new("lattice-mcp").unwrap(),
            &adoption
        )
        .is_err()
    );
}
