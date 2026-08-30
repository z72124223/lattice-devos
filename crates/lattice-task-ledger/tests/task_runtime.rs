use lattice_contracts::{
    AttemptId, ContentDigest, ProjectId, ProjectSnapshotId, RuntimeKind, TaskId,
    TaskLedgerStreamIdentity,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CorrelationId, LedgerError, LedgerEventKind,
    LedgerOutcome, ModelReason, NO_PROVIDER_EFFECT_CLOSURE_OWNER, ReasonCode, ReasoningEffort,
    TASK_EXECUTION_BINDING_RECORD_SCHEMA, TASK_VERIFICATION_RECORD_SCHEMA,
    TaskExecutionBindingInput, TaskRuntimeAppendMetadata, TaskSubmissionEnvelope,
    TaskVerificationInput, UntrustedTaskExecutionBinding, UntrustedTaskVerificationRow,
    UntrustedWorkerAttemptRow, UntrustedWorkerObservationRow, VerificationOutcome,
    VerifiedNoProviderEffectPredecessor, VerifiedStream, VerifiedTaskExecutionBinding,
    WORKER_ATTEMPT_RECORD_SCHEMA, WORKER_OBSERVATION_RECORD_SCHEMA, WorkerAttemptInput,
    WorkerModel, WorkerObservationInput, WorkerObservationKind, apply_append_plan, plan_append,
    plan_approval_evidence_append, plan_artifact_reference_append, plan_task_execution_binding,
    plan_task_verification_append, plan_worker_attempt_append,
    plan_worker_attempt_append_with_no_provider_effect_predecessor, plan_worker_observation_append,
    recover_task_verification_record, recover_worker_attempt_record,
    recover_worker_observation_record, task_execution_binding_is_recorded,
    verify_approval_evidence_links, verify_artifact_reference_links,
    verify_untrusted_task_execution_binding, verify_untrusted_task_runtime_records,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn intake_identity() -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("project-1:registry:1").expect("snapshot"),
        TaskId::new("TASK-MANAGED-001").expect("task"),
        "1",
        digest('a'),
    )
    .expect("intake identity")
}

fn task_spec_identity(task_spec_digest: ContentDigest) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("project-1:registry:1").expect("snapshot"),
        TaskId::new("TASK-MANAGED-001").expect("task"),
        "1",
        task_spec_digest,
        "TWD",
    )
    .expect("TaskSpec identity")
}

fn append_task_created(
    vacant: &VerifiedStream,
    command_id: &str,
    subject_digest: ContentDigest,
) -> VerifiedStream {
    let command = AppendCommand::new(
        vacant.head().clone(),
        CommandId::new(command_id).expect("command"),
        CorrelationId::new(format!("correlation-{command_id}")).expect("correlation"),
        "2026-08-26T01:00:00Z",
        LedgerEventKind::TaskCreated,
        ActorId::new("lattice-runtime").expect("actor"),
        ActionId::new("RECORD_MANAGED_TASK_SPEC_V1").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("TASK_SPEC_CAPTURED").expect("reason"),
        subject_digest,
        None,
        None,
    )
    .expect("TaskSpec create");
    let plan = plan_append(vacant, command).expect("TaskSpec create plan");
    apply_append_plan(vacant, &plan).expect("TaskSpec create apply")
}

fn lineage() -> (TaskSubmissionEnvelope, VerifiedStream, VerifiedStream) {
    let submission = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "managed-request-1",
        "完成有界的本機修改",
        "Project One",
        intake_identity(),
        digest('b'),
    )
    .expect("submission");
    let intake_vacant =
        VerifiedStream::vacant(submission.identity().clone(), RuntimeKind::Live).expect("vacant");
    let intake_command = AppendCommand::new_general_task_created(
        intake_vacant.head().clone(),
        CommandId::new("general-create-1").expect("command"),
        CorrelationId::new("general-create-correlation-1").expect("correlation"),
        "2026-08-26T00:59:59Z",
        ActorId::new("lattice-runtime").expect("actor"),
        &submission,
    )
    .expect("intake create");
    let intake_plan = plan_append(&intake_vacant, intake_command).expect("intake plan");
    let intake = apply_append_plan(&intake_vacant, &intake_plan).expect("intake apply");

    let spec_digest = digest('c');
    let spec_vacant =
        VerifiedStream::vacant(task_spec_identity(spec_digest.clone()), RuntimeKind::Live)
            .expect("spec vacant");
    let successor = append_task_created(&spec_vacant, "task-spec-create-1", spec_digest);
    (submission, intake, successor)
}

fn metadata(command: &str, second: u8) -> TaskRuntimeAppendMetadata {
    TaskRuntimeAppendMetadata::new(
        CommandId::new(command).expect("command"),
        CorrelationId::new(format!("correlation-{command}")).expect("correlation"),
        format!("2026-08-26T01:00:{second:02}Z"),
    )
    .expect("metadata")
}

fn bound_lineage() -> (
    TaskSubmissionEnvelope,
    VerifiedStream,
    VerifiedStream,
    VerifiedTaskExecutionBinding,
) {
    let (submission, intake, successor) = lineage();
    let plan = plan_task_execution_binding(
        &intake,
        &successor,
        &submission,
        &[],
        metadata("bind-runtime", 1),
        TaskExecutionBindingInput::new(digest('d'), digest('e'), digest('f'))
            .expect("binding input"),
    )
    .expect("binding");
    let binding = plan.new_binding().expect("new binding").clone();
    let current = apply_append_plan(&successor, plan.ledger_plan()).expect("apply binding");
    (submission, intake, current, binding)
}

fn attempt(number: u64, fence: u64) -> WorkerAttemptInput {
    WorkerAttemptInput::new(
        AttemptId::new(format!("attempt-{number}")).expect("attempt id"),
        number,
        number,
        WorkerModel::Terra,
        ReasoningEffort::High,
        ModelReason::RoutineEngineering,
        fence,
        digest('1'),
        digest('2'),
        digest('3'),
        digest('4'),
        digest('5'),
        digest('6'),
    )
    .expect("attempt")
}

#[test]
fn model_reason_is_canonical_distinct_and_tamper_evident() {
    let (_submission, _intake, stream, binding) = bound_lineage();
    let reasons = [
        ModelReason::P0,
        ModelReason::Architecture,
        ModelReason::Security,
        ModelReason::HighRisk,
        ModelReason::TerraInsufficient,
    ];
    let mut payload_digests = Vec::new();
    for (index, reason) in reasons.into_iter().enumerate() {
        let command = format!("sol-route-{}", reason.as_str().to_ascii_lowercase());
        let plan = plan_worker_attempt_append(
            &stream,
            &binding,
            &[],
            &[],
            metadata(&command, u8::try_from(index + 10).expect("bounded second")),
            WorkerAttemptInput::new(
                AttemptId::new("attempt-sol-1").expect("attempt id"),
                1,
                1,
                WorkerModel::Sol,
                ReasoningEffort::High,
                reason,
                10,
                digest('1'),
                digest('2'),
                digest('3'),
                digest('4'),
                digest('5'),
                digest('6'),
            )
            .expect("closed Sol reason"),
        )
        .expect("attempt plan");
        let record = plan.new_record().expect("attempt record");
        assert_eq!(record.model_reason(), reason);
        let canonical =
            String::from_utf8(record.payload_canonical_bytes().expect("canonical bytes"))
                .expect("canonical utf8");
        assert!(canonical.contains(reason.as_str()));
        payload_digests.push(record.payload_digest().as_str().to_owned());
    }
    payload_digests.sort();
    payload_digests.dedup();
    assert_eq!(payload_digests.len(), reasons.len());

    let p0_plan = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        metadata("sol-route-p0", 10),
        WorkerAttemptInput::new(
            AttemptId::new("attempt-sol-1").expect("attempt id"),
            1,
            1,
            WorkerModel::Sol,
            ReasoningEffort::High,
            ModelReason::P0,
            10,
            digest('1'),
            digest('2'),
            digest('3'),
            digest('4'),
            digest('5'),
            digest('6'),
        )
        .expect("P0 attempt"),
    )
    .expect("P0 plan");
    let retained = p0_plan.new_record().expect("P0 record").clone();
    let retained_stream =
        apply_append_plan(&stream, p0_plan.ledger_plan()).expect("apply P0 attempt");
    assert!(
        verify_untrusted_task_runtime_records(
            &retained_stream,
            &binding,
            &[retained
                .to_untrusted()
                .with_model_reason(ModelReason::Architecture)],
            &[],
            &[],
        )
        .is_err()
    );
    assert!(
        verify_untrusted_task_runtime_records(
            &retained_stream,
            &binding,
            &[retained
                .to_untrusted()
                .with_model_reason(ModelReason::RoutineEngineering)],
            &[],
            &[],
        )
        .is_err()
    );
}

fn observation(
    attempt_number: u64,
    kind: WorkerObservationKind,
    thread_id: &str,
    turn_id: Option<&str>,
    evidence: char,
) -> WorkerObservationInput {
    observation_with_identity(attempt_number, kind, thread_id, turn_id, 1, '6', evidence)
}

fn observation_with_identity(
    attempt_number: u64,
    kind: WorkerObservationKind,
    thread_id: &str,
    turn_id: Option<&str>,
    app_server_generation: u64,
    app_server_identity: char,
    evidence: char,
) -> WorkerObservationInput {
    WorkerObservationInput::new(
        attempt_number,
        kind,
        Some(thread_id),
        turn_id,
        app_server_generation,
        digest(app_server_identity),
        digest(evidence),
    )
    .expect("observation")
}

#[test]
#[allow(clippy::too_many_lines)]
fn exact_start_provider_time_is_durable_and_tamper_evident() {
    let (_submission, _intake, mut stream, binding) = bound_lineage();
    let attempt_plan = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        metadata("provider-time-attempt", 20),
        attempt(1, 10),
    )
    .expect("attempt");
    let attempts = vec![attempt_plan.new_record().expect("attempt row").clone()];
    stream = apply_append_plan(&stream, attempt_plan.ledger_plan()).expect("apply attempt");

    let mut observations = Vec::new();
    for (command, second, input) in [
        (
            "provider-time-thread",
            21,
            observation(
                1,
                WorkerObservationKind::ThreadAccepted,
                "thread-provider-time",
                None,
                '7',
            ),
        ),
        (
            "provider-time-turn",
            22,
            observation(
                1,
                WorkerObservationKind::TurnAccepted,
                "thread-provider-time",
                Some("turn-provider-time"),
                '8',
            ),
        ),
    ] {
        let plan = plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata(command, second),
            input,
        )
        .expect("accepted observation");
        observations.push(plan.new_record().expect("observation row").clone());
        stream = apply_append_plan(&stream, plan.ledger_plan()).expect("apply observation");
    }

    let started = WorkerObservationInput::exact_started(
        1,
        "thread-provider-time",
        "turn-provider-time",
        1,
        digest('6'),
        "2026-08-26T02:00:22.12Z",
        digest('9'),
    )
    .expect("exact provider start");
    let started_plan = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        metadata("provider-time-started", 23),
        started,
    )
    .expect("started observation");
    let started_record = started_plan.new_record().expect("started row").clone();
    assert_eq!(started_record.observed_at(), "2026-08-26T02:00:22.12Z");
    observations.push(started_record);
    stream = apply_append_plan(&stream, started_plan.ledger_plan()).expect("apply started");

    let rows = observations
        .iter()
        .map(lattice_task_ledger::VerifiedWorkerObservationRecord::to_untrusted)
        .collect::<Vec<_>>();
    verify_untrusted_task_runtime_records(
        &stream,
        &binding,
        &attempts
            .iter()
            .map(lattice_task_ledger::VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>(),
        &rows,
        &[],
    )
    .expect("provider time replays");

    let mut tampered = rows;
    tampered[2] = tampered[2]
        .clone()
        .with_observed_at("2026-08-26T02:00:21.12Z");
    assert_eq!(
        verify_untrusted_task_runtime_records(
            &stream,
            &binding,
            &attempts
                .iter()
                .map(lattice_task_ledger::VerifiedWorkerAttemptRecord::to_untrusted)
                .collect::<Vec<_>>(),
            &tampered,
            &[],
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );
    assert_eq!(
        WorkerObservationInput::new(
            1,
            WorkerObservationKind::TurnStarted,
            Some("thread-provider-time"),
            Some("turn-provider-time"),
            1,
            digest('6'),
            digest('9'),
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );
}

#[test]
fn worker_observation_semantic_owner_rejects_impossible_lifecycle_order() {
    let (_submission, _intake, mut stream, binding) = bound_lineage();
    let attempt_plan = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        metadata("ordered-attempt", 20),
        attempt(1, 10),
    )
    .expect("attempt");
    let attempts = vec![attempt_plan.new_record().expect("attempt row").clone()];
    stream = apply_append_plan(&stream, attempt_plan.ledger_plan()).expect("apply attempt");

    assert_eq!(
        plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &[],
            metadata("turn-before-thread", 21),
            observation(
                1,
                WorkerObservationKind::TurnAccepted,
                "thread-ordered",
                Some("turn-ordered"),
                '7',
            ),
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );

    let thread = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &[],
        metadata("ordered-thread", 22),
        observation(
            1,
            WorkerObservationKind::ThreadAccepted,
            "thread-ordered",
            None,
            '8',
        ),
    )
    .expect("thread accepted");
    let mut observations = vec![thread.new_record().expect("thread row").clone()];
    stream = apply_append_plan(&stream, thread.ledger_plan()).expect("apply thread");
    let turn = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        metadata("ordered-turn", 23),
        observation(
            1,
            WorkerObservationKind::TurnAccepted,
            "thread-ordered",
            Some("turn-ordered"),
            '9',
        ),
    )
    .expect("turn accepted");
    observations.push(turn.new_record().expect("turn row").clone());
    stream = apply_append_plan(&stream, turn.ledger_plan()).expect("apply turn");

    assert_eq!(
        plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata("terminal-before-start", 24),
            observation(
                1,
                WorkerObservationKind::TerminalCompleted,
                "thread-ordered",
                Some("turn-ordered"),
                'a',
            ),
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );

    assert_eq!(
        plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata("ordinary-failure-before-start", 24),
            observation(
                1,
                WorkerObservationKind::TerminalFailed,
                "thread-ordered",
                Some("turn-ordered"),
                'b',
            ),
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );

    let prestart_terminal = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        metadata("typed-prestart-failure", 24),
        observation(
            1,
            WorkerObservationKind::PrestartTerminalFailed,
            "thread-ordered",
            Some("turn-ordered"),
            'c',
        ),
    )
    .expect("typed prestart failure");
    observations.push(
        prestart_terminal
            .new_record()
            .expect("prestart terminal row")
            .clone(),
    );
    stream = apply_append_plan(&stream, prestart_terminal.ledger_plan())
        .expect("apply prestart terminal");
    verify_untrusted_task_runtime_records(
        &stream,
        &binding,
        &attempts
            .iter()
            .map(lattice_task_ledger::VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>(),
        &observations
            .iter()
            .map(lattice_task_ledger::VerifiedWorkerObservationRecord::to_untrusted)
            .collect::<Vec<_>>(),
        &[],
    )
    .expect("typed prestart failure replays");

    let mut tampered = observations
        .iter()
        .map(lattice_task_ledger::VerifiedWorkerObservationRecord::to_untrusted)
        .collect::<Vec<_>>();
    tampered[2] = tampered[2]
        .clone()
        .with_kind(WorkerObservationKind::TerminalFailed);
    assert_eq!(
        verify_untrusted_task_runtime_records(
            &stream,
            &binding,
            &attempts
                .iter()
                .map(lattice_task_ledger::VerifiedWorkerAttemptRecord::to_untrusted)
                .collect::<Vec<_>>(),
            &tampered,
            &[],
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );
}

fn terminal_attempt() -> (
    VerifiedStream,
    VerifiedTaskExecutionBinding,
    Vec<lattice_task_ledger::VerifiedWorkerAttemptRecord>,
    Vec<lattice_task_ledger::VerifiedWorkerObservationRecord>,
) {
    let (_submission, _intake, mut stream, binding) = bound_lineage();
    let attempt_plan = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        metadata("verify-attempt-1", 20),
        attempt(1, 10),
    )
    .expect("attempt");
    let attempts = vec![attempt_plan.new_record().expect("attempt row").clone()];
    stream = apply_append_plan(&stream, attempt_plan.ledger_plan()).expect("apply attempt");

    let mut observations = Vec::new();
    for (command, second, input) in [
        (
            "verify-thread",
            21,
            observation(
                1,
                WorkerObservationKind::ThreadAccepted,
                "thread-verify",
                None,
                '7',
            ),
        ),
        (
            "verify-turn",
            22,
            observation(
                1,
                WorkerObservationKind::TurnAccepted,
                "thread-verify",
                Some("turn-verify"),
                '8',
            ),
        ),
        (
            "verify-started",
            23,
            WorkerObservationInput::exact_started(
                1,
                "thread-verify",
                "turn-verify",
                1,
                digest('6'),
                "2026-08-26T02:00:23Z",
                digest('9'),
            )
            .expect("verify exact start"),
        ),
        (
            "verify-terminal",
            24,
            observation(
                1,
                WorkerObservationKind::TerminalCompleted,
                "thread-verify",
                Some("turn-verify"),
                '9',
            ),
        ),
    ] {
        let plan = plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata(command, second),
            input,
        )
        .expect("observation");
        observations.push(plan.new_record().expect("observation row").clone());
        stream = apply_append_plan(&stream, plan.ledger_plan()).expect("apply observation");
    }
    (stream, binding, attempts, observations)
}

#[test]
fn promotion_is_exactly_once_and_changed_budget_is_substitution() {
    let (submission, intake, successor) = lineage();
    let input = TaskExecutionBindingInput::new(digest('d'), digest('e'), digest('f'))
        .expect("binding input");
    assert!(
        !task_execution_binding_is_recorded(&intake, &successor, &submission)
            .expect("vacant promotion")
    );

    let first = plan_task_execution_binding(
        &intake,
        &successor,
        &submission,
        &[],
        metadata("bind-task-1", 1),
        input.clone(),
    )
    .expect("first binding");
    assert!(!first.is_exact_retry());
    let retained = first.new_binding().expect("new binding").clone();
    assert_eq!(retained.task_ref(), submission.task_ref());
    assert_eq!(retained.intake_stream_id(), submission.stream_id());
    assert_eq!(retained.successor_stream_id(), successor.head().stream_id());
    assert_eq!(
        first
            .ledger_plan()
            .new_event()
            .expect("binding event")
            .kind(),
        LedgerEventKind::EvidenceRecorded
    );
    let current = apply_append_plan(&successor, first.ledger_plan()).expect("apply binding");
    assert!(
        task_execution_binding_is_recorded(&intake, &current, &submission)
            .expect("recorded promotion")
    );

    let retry = plan_task_execution_binding(
        &intake,
        &current,
        &submission,
        std::slice::from_ref(&retained),
        metadata("bind-task-1", 1),
        input,
    )
    .expect("exact retry");
    assert!(retry.is_exact_retry());
    assert!(retry.new_binding().is_none());
    assert_eq!(retry.binding(), &retained);

    let changed_budget =
        TaskExecutionBindingInput::new(digest('d'), digest('9'), digest('f')).expect("changed");
    assert_eq!(
        plan_task_execution_binding(
            &intake,
            &current,
            &submission,
            std::slice::from_ref(&retained),
            metadata("bind-task-1", 1),
            changed_budget,
        ),
        Err(LedgerError::TaskRuntimeSubstitution)
    );

    assert_eq!(
        verify_untrusted_task_execution_binding(
            &intake,
            &current,
            &submission,
            &retained.to_untrusted(),
        )
        .expect("replayed binding"),
        retained
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn dual_write_crash_recovery_rebuilds_only_exact_missing_extension_rows() {
    let (submission, intake, successor) = lineage();
    let binding_input =
        TaskExecutionBindingInput::new(digest('d'), digest('e'), digest('f')).expect("binding");
    let binding_metadata = metadata("crash-binding", 1);
    let binding_plan = plan_task_execution_binding(
        &intake,
        &successor,
        &submission,
        &[],
        binding_metadata.clone(),
        binding_input.clone(),
    )
    .expect("binding intent");
    let mut stream =
        apply_append_plan(&successor, binding_plan.ledger_plan()).expect("binding ledger commit");
    let binding_recovery = plan_task_execution_binding(
        &intake,
        &stream,
        &submission,
        &[],
        binding_metadata,
        binding_input,
    )
    .expect("binding extension recovery");
    assert!(binding_recovery.is_exact_retry());
    let binding = binding_recovery
        .new_binding()
        .expect("missing binding row is recoverable")
        .clone();

    let attempt_metadata = metadata("crash-attempt", 2);
    let attempt_input = attempt(1, 10);
    let attempt_plan = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        attempt_metadata.clone(),
        attempt_input.clone(),
    )
    .expect("attempt intent");
    assert_eq!(
        recover_worker_attempt_record(&stream, &binding, &attempt_metadata, &attempt_input)
            .expect("uncommitted attempt"),
        None
    );
    stream = apply_append_plan(&stream, attempt_plan.ledger_plan()).expect("attempt ledger commit");
    let recovered_attempt =
        recover_worker_attempt_record(&stream, &binding, &attempt_metadata, &attempt_input)
            .expect("attempt recovery classification")
            .expect("retained attempt intent");
    assert_eq!(
        plan_worker_attempt_append(
            &stream,
            &binding,
            &[],
            &[],
            attempt_metadata.clone(),
            attempt(1, 11),
        ),
        Err(LedgerError::CommandIdReuse)
    );
    assert_eq!(
        recover_worker_attempt_record(&stream, &binding, &attempt_metadata, &attempt(1, 11),),
        Err(LedgerError::CommandIdReuse)
    );
    let attempt_recovery = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        attempt_metadata.clone(),
        attempt_input.clone(),
    )
    .expect("attempt extension recovery");
    assert!(attempt_recovery.ledger_plan().is_exact_retry());
    let attempts = vec![
        attempt_recovery
            .new_record()
            .expect("missing attempt row is recoverable")
            .clone(),
    ];
    assert_eq!(attempts[0], recovered_attempt);
    let pending_rows = vec![attempts[0].to_untrusted()];
    let pending_records =
        verify_untrusted_task_runtime_records(&stream, &binding, &pending_rows, &[], &[])
            .expect("capacity-wait row remains owner verified");
    let exact_later_claim = plan_worker_attempt_append(
        &stream,
        &binding,
        pending_records.attempts(),
        &[],
        attempt_metadata,
        attempt_input,
    )
    .expect("later capacity claim reuses the retained intent");
    assert!(exact_later_claim.ledger_plan().is_exact_retry());
    assert!(exact_later_claim.new_record().is_none());
    assert_eq!(exact_later_claim.record(), &attempts[0]);
    assert_eq!(
        verify_untrusted_task_runtime_records(
            &stream,
            &binding,
            &[pending_rows[0].clone().with_attempt_number(2)],
            &[],
            &[],
        ),
        Err(LedgerError::WorkerAttemptNotMonotonic)
    );
    assert_eq!(
        verify_untrusted_task_runtime_records(
            &stream,
            &binding,
            &[pending_rows[0].clone().with_event_digest(digest('0'))],
            &[],
            &[],
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );

    let thread_input = observation(
        1,
        WorkerObservationKind::ThreadAccepted,
        "thread-crash",
        None,
        '7',
    );
    let thread_metadata = metadata("crash-observation", 3);
    let thread_plan = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &[],
        thread_metadata.clone(),
        thread_input.clone(),
    )
    .expect("observation intent");
    assert_eq!(
        recover_worker_observation_record(
            &stream,
            &binding,
            &attempts,
            &thread_metadata,
            &thread_input,
        )
        .expect("uncommitted observation"),
        None
    );
    stream =
        apply_append_plan(&stream, thread_plan.ledger_plan()).expect("observation ledger commit");
    let recovered_observation = recover_worker_observation_record(
        &stream,
        &binding,
        &attempts,
        &thread_metadata,
        &thread_input,
    )
    .expect("observation recovery classification")
    .expect("retained observation intent");
    let thread_recovery = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &[],
        thread_metadata,
        thread_input,
    )
    .expect("observation extension recovery");
    assert!(thread_recovery.ledger_plan().is_exact_retry());
    let mut observations = vec![
        thread_recovery
            .new_record()
            .expect("missing observation row is recoverable")
            .clone(),
    ];
    assert_eq!(observations[0], recovered_observation);

    let approval_metadata = metadata("crash-approval", 4);
    let approval_plan = plan_approval_evidence_append(
        &stream,
        &binding,
        &[],
        approval_metadata.clone(),
        digest('8'),
    )
    .expect("approval intent");
    stream =
        apply_append_plan(&stream, approval_plan.ledger_plan()).expect("approval ledger commit");
    let approval_recovery =
        plan_approval_evidence_append(&stream, &binding, &[], approval_metadata, digest('8'))
            .expect("approval extension recovery");
    assert!(approval_recovery.is_exact_retry());
    assert!(approval_recovery.new_link().is_some());

    let artifact_metadata = metadata("crash-artifact", 5);
    let artifact_plan = plan_artifact_reference_append(
        &stream,
        &binding,
        &attempts,
        &[],
        artifact_metadata.clone(),
        1,
        digest('9'),
    )
    .expect("artifact intent");
    stream =
        apply_append_plan(&stream, artifact_plan.ledger_plan()).expect("artifact ledger commit");
    let artifact_recovery = plan_artifact_reference_append(
        &stream,
        &binding,
        &attempts,
        &[],
        artifact_metadata,
        1,
        digest('9'),
    )
    .expect("artifact extension recovery");
    assert!(artifact_recovery.is_exact_retry());
    assert!(artifact_recovery.new_link().is_some());

    for (command, second, input) in [
        (
            "crash-turn",
            6,
            observation(
                1,
                WorkerObservationKind::TurnAccepted,
                "thread-crash",
                Some("turn-crash"),
                'a',
            ),
        ),
        (
            "crash-started",
            7,
            WorkerObservationInput::exact_started(
                1,
                "thread-crash",
                "turn-crash",
                1,
                digest('6'),
                "2026-08-26T02:00:07Z",
                digest('b'),
            )
            .expect("crash exact start"),
        ),
        (
            "crash-terminal",
            8,
            observation(
                1,
                WorkerObservationKind::TerminalCompleted,
                "thread-crash",
                Some("turn-crash"),
                'b',
            ),
        ),
    ] {
        let plan = plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata(command, second),
            input,
        )
        .expect("retained observation");
        observations.push(plan.new_record().expect("observation row").clone());
        stream = apply_append_plan(&stream, plan.ledger_plan()).expect("apply observation");
    }

    let verification_input = TaskVerificationInput::new(
        1,
        VerificationOutcome::Passed,
        digest('c'),
        digest('d'),
        digest('e'),
        digest('f'),
        digest('1'),
        digest('2'),
        digest('9'),
        Some(digest('3')),
    )
    .expect("verification");
    let verification_metadata = metadata("crash-verification", 8);
    let verification_plan = plan_task_verification_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        &[],
        verification_metadata.clone(),
        verification_input.clone(),
    )
    .expect("verification intent");
    assert_eq!(
        recover_task_verification_record(
            &stream,
            &binding,
            &attempts,
            &observations,
            &verification_metadata,
            &verification_input,
        )
        .expect("uncommitted verification"),
        None
    );
    stream = apply_append_plan(&stream, verification_plan.ledger_plan())
        .expect("verification ledger commit");
    let recovered_verification = recover_task_verification_record(
        &stream,
        &binding,
        &attempts,
        &observations,
        &verification_metadata,
        &verification_input,
    )
    .expect("verification recovery classification")
    .expect("retained verification intent");
    let verification_recovery = plan_task_verification_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        &[],
        verification_metadata,
        verification_input,
    )
    .expect("verification extension recovery");
    assert!(verification_recovery.ledger_plan().is_exact_retry());
    assert_eq!(
        verification_recovery
            .new_record()
            .expect("missing verification row is recoverable"),
        &recovered_verification
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn attempts_are_monotonic_and_worker_ids_are_immutable_until_exact_terminal() {
    let (_submission, _intake, mut stream, binding) = bound_lineage();
    let first = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        metadata("attempt-1", 2),
        attempt(1, 10),
    )
    .expect("first attempt");
    assert_eq!(
        first
            .ledger_plan()
            .new_event()
            .expect("attempt event")
            .kind(),
        LedgerEventKind::EffectIntent
    );
    let mut attempts = vec![first.new_record().expect("attempt row").clone()];
    stream = apply_append_plan(&stream, first.ledger_plan()).expect("apply attempt");

    assert_eq!(
        plan_worker_attempt_append(
            &stream,
            &binding,
            &attempts,
            &[],
            metadata("attempt-gap", 3),
            attempt(3, 30),
        ),
        Err(LedgerError::WorkerAttemptNotMonotonic)
    );
    assert_eq!(
        plan_worker_attempt_append(
            &stream,
            &binding,
            &attempts,
            &[],
            metadata("attempt-before-terminal", 3),
            attempt(2, 20),
        ),
        Err(LedgerError::WorkerAttemptBeforeTerminal)
    );

    let thread = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &[],
        metadata("thread-accepted", 4),
        observation_with_identity(
            1,
            WorkerObservationKind::ThreadAccepted,
            "thread-1",
            None,
            7,
            '6',
            '7',
        ),
    )
    .expect("thread accepted");
    let mut observations = vec![thread.new_record().expect("thread row").clone()];
    stream = apply_append_plan(&stream, thread.ledger_plan()).expect("apply thread");

    let turn = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        metadata("turn-accepted", 5),
        observation_with_identity(
            1,
            WorkerObservationKind::TurnAccepted,
            "thread-1",
            Some("turn-1"),
            7,
            '6',
            '8',
        ),
    )
    .expect("turn accepted");
    observations.push(turn.new_record().expect("turn row").clone());
    stream = apply_append_plan(&stream, turn.ledger_plan()).expect("apply turn");

    assert_eq!(
        plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata("thread-drift", 6),
            observation(
                1,
                WorkerObservationKind::MeaningfulProgress,
                "thread-2",
                Some("turn-1"),
                '9',
            ),
        ),
        Err(LedgerError::WorkerIdentityDrift)
    );
    assert_eq!(
        plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata("turn-drift", 6),
            observation(
                1,
                WorkerObservationKind::MeaningfulProgress,
                "thread-1",
                Some("turn-2"),
                '9',
            ),
        ),
        Err(LedgerError::WorkerIdentityDrift)
    );

    let started = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        metadata("turn-started", 7),
        WorkerObservationInput::exact_started(
            1,
            "thread-1",
            "turn-1",
            7,
            digest('6'),
            "2026-08-26T02:00:07Z",
            digest('a'),
        )
        .expect("exact start"),
    )
    .expect("turn started");
    observations.push(started.new_record().expect("started row").clone());
    stream = apply_append_plan(&stream, started.ledger_plan()).expect("apply started");

    assert_eq!(
        plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata("identity-drift-without-reconcile", 8),
            observation_with_identity(
                1,
                WorkerObservationKind::MeaningfulProgress,
                "thread-1",
                Some("turn-1"),
                1,
                '5',
                'b',
            ),
        ),
        Err(LedgerError::WorkerIdentityDrift)
    );

    let reconciled = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        metadata("identity-reconciled", 8),
        observation_with_identity(
            1,
            WorkerObservationKind::Reconciled,
            "thread-1",
            Some("turn-1"),
            1,
            '5',
            'c',
        ),
    )
    .expect("reconciliation may bind a fresh session whose local generation restarted at one");
    observations.push(reconciled.new_record().expect("reconciled row").clone());
    stream = apply_append_plan(&stream, reconciled.ledger_plan()).expect("apply reconciliation");

    assert_eq!(
        plan_worker_observation_append(
            &stream,
            &binding,
            &attempts,
            &observations,
            metadata("old-identity-after-reconcile", 9),
            observation_with_identity(
                1,
                WorkerObservationKind::MeaningfulProgress,
                "thread-1",
                Some("turn-1"),
                7,
                '6',
                'd',
            ),
        ),
        Err(LedgerError::WorkerIdentityDrift)
    );

    let terminal = plan_worker_observation_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        metadata("turn-terminal", 9),
        observation_with_identity(
            1,
            WorkerObservationKind::TerminalFailed,
            "thread-1",
            Some("turn-1"),
            1,
            '5',
            'a',
        ),
    )
    .expect("terminal");
    assert_eq!(
        terminal
            .ledger_plan()
            .new_event()
            .expect("terminal event")
            .kind(),
        LedgerEventKind::EffectOutcome
    );
    observations.push(terminal.new_record().expect("terminal row").clone());
    stream = apply_append_plan(&stream, terminal.ledger_plan()).expect("apply terminal");

    let second = plan_worker_attempt_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        metadata("attempt-2", 8),
        attempt(2, 20),
    )
    .expect("second attempt after terminal");
    attempts.push(second.new_record().expect("second row").clone());
    assert_eq!(attempts.len(), 2);
}

#[test]
fn verified_no_provider_effect_closure_is_an_exact_digest_bound_retry_predecessor() {
    let (_submission, _intake, mut stream, binding) = bound_lineage();
    let first = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        metadata("closure-attempt-1", 2),
        attempt(1, 10),
    )
    .expect("first attempt");
    let first_record = first.new_record().expect("first row").clone();
    stream = apply_append_plan(&stream, first.ledger_plan()).expect("apply first attempt");

    assert_eq!(
        plan_worker_attempt_append(
            &stream,
            &binding,
            std::slice::from_ref(&first_record),
            &[],
            metadata("closure-attempt-2", 3),
            attempt(2, 20),
        ),
        Err(LedgerError::WorkerAttemptBeforeTerminal)
    );

    let predecessor = VerifiedNoProviderEffectPredecessor::new(
        &binding,
        &first_record,
        NO_PROVIDER_EFFECT_CLOSURE_OWNER,
        binding.task_ref(),
        first_record.attempt_id(),
        first_record.attempt_number(),
        first_record.writer_fence(),
        "CODEX_APP_SERVER_TIMEOUT",
        digest('7'),
        digest('8'),
        digest('3'),
    )
    .expect("owner-verified no-provider-effect predecessor");
    assert_ne!(predecessor.digest(), &digest('0'));

    let substituted_packet = VerifiedNoProviderEffectPredecessor::new(
        &binding,
        &first_record,
        NO_PROVIDER_EFFECT_CLOSURE_OWNER,
        binding.task_ref(),
        first_record.attempt_id(),
        first_record.attempt_number(),
        first_record.writer_fence(),
        "CODEX_APP_SERVER_TIMEOUT",
        digest('7'),
        digest('8'),
        digest('9'),
    )
    .expect("different successor packet commitment");
    assert_eq!(
        plan_worker_attempt_append_with_no_provider_effect_predecessor(
            &stream,
            &binding,
            std::slice::from_ref(&first_record),
            &[],
            &substituted_packet,
            metadata("packet-substitution", 3),
            attempt(2, 20),
        ),
        Err(LedgerError::TaskRuntimeSubstitution)
    );

    let second_metadata = metadata("closure-attempt-2", 3);
    let second_input = attempt(2, 20);
    let second = plan_worker_attempt_append_with_no_provider_effect_predecessor(
        &stream,
        &binding,
        std::slice::from_ref(&first_record),
        &[],
        &predecessor,
        second_metadata.clone(),
        second_input.clone(),
    )
    .expect("closure-backed second attempt");
    let second_record = second.new_record().expect("second row").clone();
    stream = apply_append_plan(&stream, second.ledger_plan()).expect("apply second attempt");

    let exact_retry = plan_worker_attempt_append_with_no_provider_effect_predecessor(
        &stream,
        &binding,
        &[first_record.clone(), second_record],
        &[],
        &predecessor,
        second_metadata,
        second_input,
    )
    .expect("same closure-backed attempt replays exactly");
    assert!(exact_retry.ledger_plan().is_exact_retry());

    assert_eq!(
        VerifiedNoProviderEffectPredecessor::new(
            &binding,
            &first_record,
            "untrusted-project-file",
            binding.task_ref(),
            first_record.attempt_id(),
            first_record.attempt_number(),
            first_record.writer_fence(),
            "CODEX_APP_SERVER_TIMEOUT",
            digest('7'),
            digest('8'),
            digest('3'),
        ),
        Err(LedgerError::TaskRuntimeSubstitution)
    );
    assert_eq!(
        VerifiedNoProviderEffectPredecessor::new(
            &binding,
            &first_record,
            NO_PROVIDER_EFFECT_CLOSURE_OWNER,
            binding.task_ref(),
            &AttemptId::new("foreign-attempt").expect("foreign attempt"),
            first_record.attempt_number(),
            first_record.writer_fence(),
            "CODEX_APP_SERVER_TIMEOUT",
            digest('7'),
            digest('8'),
            digest('3'),
        ),
        Err(LedgerError::TaskRuntimeSubstitution)
    );
    assert_eq!(
        VerifiedNoProviderEffectPredecessor::new(
            &binding,
            &first_record,
            NO_PROVIDER_EFFECT_CLOSURE_OWNER,
            &digest('9'),
            first_record.attempt_id(),
            first_record.attempt_number(),
            first_record.writer_fence(),
            "CODEX_APP_SERVER_TIMEOUT",
            digest('7'),
            digest('8'),
            digest('3'),
        ),
        Err(LedgerError::TaskRuntimeSubstitution)
    );
    assert_eq!(
        VerifiedNoProviderEffectPredecessor::new(
            &binding,
            &first_record,
            NO_PROVIDER_EFFECT_CLOSURE_OWNER,
            binding.task_ref(),
            first_record.attempt_id(),
            first_record.attempt_number(),
            first_record.writer_fence() + 1,
            "CODEX_APP_SERVER_TIMEOUT",
            digest('7'),
            digest('8'),
            digest('3'),
        ),
        Err(LedgerError::TaskRuntimeSubstitution)
    );
    assert_eq!(
        VerifiedNoProviderEffectPredecessor::new(
            &binding,
            &first_record,
            NO_PROVIDER_EFFECT_CLOSURE_OWNER,
            binding.task_ref(),
            first_record.attempt_id(),
            first_record.attempt_number(),
            first_record.writer_fence(),
            "CODEX_APP_SERVER_TIMEOUT",
            digest('7'),
            digest('7'),
            digest('3'),
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );
}

#[test]
fn verification_replay_rejects_missing_or_tampered_child_records() {
    let (mut stream, binding, attempts, observations) = terminal_attempt();
    let verification_input = TaskVerificationInput::new(
        1,
        VerificationOutcome::Passed,
        digest('a'),
        digest('b'),
        digest('c'),
        digest('d'),
        digest('e'),
        digest('f'),
        digest('1'),
        Some(digest('2')),
    )
    .expect("verification input");
    let plan = plan_task_verification_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        &[],
        metadata("verification-1", 24),
        verification_input,
    )
    .expect("verification plan");
    assert_eq!(
        plan.ledger_plan()
            .new_event()
            .expect("verification event")
            .kind(),
        LedgerEventKind::EvidenceRecorded
    );
    let verification = plan.new_record().expect("verification row").clone();
    stream = apply_append_plan(&stream, plan.ledger_plan()).expect("apply verification");

    let attempt_rows = attempts
        .iter()
        .map(lattice_task_ledger::VerifiedWorkerAttemptRecord::to_untrusted)
        .collect::<Vec<_>>();
    let observation_rows = observations
        .iter()
        .map(lattice_task_ledger::VerifiedWorkerObservationRecord::to_untrusted)
        .collect::<Vec<_>>();
    let verification_rows = vec![verification.to_untrusted()];
    let replay = verify_untrusted_task_runtime_records(
        &stream,
        &binding,
        &attempt_rows,
        &observation_rows,
        &verification_rows,
    )
    .expect("fresh replay");
    assert_eq!(replay.attempts().len(), 1);
    assert_eq!(replay.observations().len(), 4);
    assert_eq!(replay.verifications().len(), 1);

    assert_eq!(
        verify_untrusted_task_runtime_records(
            &stream,
            &binding,
            &attempt_rows,
            &observation_rows,
            &[],
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );

    let tampered_attempts = vec![attempt_rows[0].clone().with_event_digest(digest('8'))];
    assert!(
        verify_untrusted_task_runtime_records(
            &stream,
            &binding,
            &tampered_attempts,
            &observation_rows,
            &verification_rows,
        )
        .is_err()
    );

    let mut tampered_observations = observation_rows.clone();
    tampered_observations[1] = tampered_observations[1]
        .clone()
        .with_turn_id(Some("turn-substituted"));
    assert!(
        verify_untrusted_task_runtime_records(
            &stream,
            &binding,
            &attempt_rows,
            &tampered_observations,
            &verification_rows,
        )
        .is_err()
    );

    let tampered_verifications = vec![verification_rows[0].clone().with_result_digest(digest('7'))];
    assert!(
        verify_untrusted_task_runtime_records(
            &stream,
            &binding,
            &attempt_rows,
            &observation_rows,
            &tampered_verifications,
        )
        .is_err()
    );
}

#[test]
fn approval_and_artifact_references_get_exclusive_owner_planned_events() {
    let (_submission, _intake, mut stream, binding) = bound_lineage();
    let authority_digest = digest('7');
    let approval = plan_approval_evidence_append(
        &stream,
        &binding,
        &[],
        metadata("approval-reference", 2),
        authority_digest.clone(),
    )
    .expect("approval reference");
    let approval_event = approval.ledger_plan().new_event().expect("approval event");
    assert_eq!(approval_event.kind(), LedgerEventKind::EvidenceRecorded);
    assert_eq!(
        approval_event.action().as_str(),
        "RECORD_APPROVAL_EVIDENCE_V1"
    );
    assert_eq!(approval_event.subject_digest(), &authority_digest);
    let approval_link = approval.new_link().expect("new approval link").clone();
    stream = apply_append_plan(&stream, approval.ledger_plan()).expect("apply approval");

    let approval_retry = plan_approval_evidence_append(
        &stream,
        &binding,
        std::slice::from_ref(&approval_link),
        metadata("approval-reference", 2),
        authority_digest,
    )
    .expect("approval exact retry");
    assert!(approval_retry.is_exact_retry());
    assert!(approval_retry.new_link().is_none());
    assert_eq!(approval_retry.link(), &approval_link);
    assert_eq!(
        verify_approval_evidence_links(&stream, &binding, std::slice::from_ref(&approval_link),)
            .expect("approval replay"),
        vec![approval_link.clone()]
    );

    let attempt_plan = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        metadata("artifact-attempt", 3),
        attempt(1, 10),
    )
    .expect("attempt");
    let attempts = vec![attempt_plan.new_record().expect("attempt row").clone()];
    stream = apply_append_plan(&stream, attempt_plan.ledger_plan()).expect("apply attempt");

    // The same external digest under two owner families must still produce two
    // distinct child events; the family/action is part of the event identity.
    let descriptor_digest = digest('7');
    let artifact = plan_artifact_reference_append(
        &stream,
        &binding,
        &attempts,
        &[],
        metadata("artifact-reference", 4),
        1,
        descriptor_digest.clone(),
    )
    .expect("artifact reference");
    let artifact_event = artifact.ledger_plan().new_event().expect("artifact event");
    assert_eq!(artifact_event.kind(), LedgerEventKind::EvidenceRecorded);
    assert_eq!(
        artifact_event.action().as_str(),
        "RECORD_ARTIFACT_REFERENCE_V1"
    );
    assert_eq!(artifact_event.subject_digest(), &descriptor_digest);
    let artifact_link = artifact.new_link().expect("new artifact link").clone();
    assert_ne!(artifact_link.event_digest(), approval_link.event_digest());
    stream = apply_append_plan(&stream, artifact.ledger_plan()).expect("apply artifact");

    let artifact_retry = plan_artifact_reference_append(
        &stream,
        &binding,
        &attempts,
        std::slice::from_ref(&artifact_link),
        metadata("artifact-reference", 4),
        1,
        descriptor_digest,
    )
    .expect("artifact exact retry");
    assert!(artifact_retry.is_exact_retry());
    assert!(artifact_retry.new_link().is_none());
    assert_eq!(artifact_retry.link(), &artifact_link);
    assert_eq!(
        verify_artifact_reference_links(&stream, &binding, std::slice::from_ref(&artifact_link),)
            .expect("artifact replay"),
        vec![artifact_link.clone()]
    );

    assert_eq!(
        verify_artifact_reference_links(&stream, &binding, std::slice::from_ref(&approval_link),),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );
    assert_eq!(
        verify_approval_evidence_links(&stream, &binding, std::slice::from_ref(&artifact_link),),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );
}

#[test]
fn reference_replay_rejects_missing_rows_and_changed_commands() {
    let (_submission, _intake, mut stream, binding) = bound_lineage();
    let authority_digest = digest('7');
    let approval = plan_approval_evidence_append(
        &stream,
        &binding,
        &[],
        metadata("approval-reference-replay", 2),
        authority_digest.clone(),
    )
    .expect("approval reference");
    let approval_link = approval.new_link().expect("approval link").clone();
    stream = apply_append_plan(&stream, approval.ledger_plan()).expect("apply approval");

    assert_eq!(
        verify_approval_evidence_links(&stream, &binding, &[]),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );
    assert_eq!(
        plan_approval_evidence_append(
            &stream,
            &binding,
            std::slice::from_ref(&approval_link),
            metadata("changed-approval-command", 2),
            authority_digest,
        ),
        Err(LedgerError::TaskRuntimeSubstitution)
    );

    let attempt_plan = plan_worker_attempt_append(
        &stream,
        &binding,
        &[],
        &[],
        metadata("artifact-attempt-replay", 3),
        attempt(1, 10),
    )
    .expect("attempt");
    let attempts = vec![attempt_plan.new_record().expect("attempt row").clone()];
    stream = apply_append_plan(&stream, attempt_plan.ledger_plan()).expect("apply attempt");

    assert_eq!(
        plan_artifact_reference_append(
            &stream,
            &binding,
            &attempts,
            &[],
            metadata("unknown-attempt-artifact", 4),
            2,
            digest('8'),
        ),
        Err(LedgerError::InvalidTaskRuntimeRecord)
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn adapter_can_export_and_rehydrate_all_runtime_rows_without_reflection() {
    let (mut stream, binding, attempts, observations) = terminal_attempt();
    let verification_plan = plan_task_verification_append(
        &stream,
        &binding,
        &attempts,
        &observations,
        &[],
        metadata("adapter-verification", 25),
        TaskVerificationInput::new(
            1,
            VerificationOutcome::Passed,
            digest('a'),
            digest('b'),
            digest('c'),
            digest('d'),
            digest('e'),
            digest('f'),
            digest('1'),
            Some(digest('2')),
        )
        .expect("verification"),
    )
    .expect("verification plan");
    let verification = verification_plan
        .new_record()
        .expect("verification row")
        .clone();
    stream = apply_append_plan(&stream, verification_plan.ledger_plan()).expect("apply");

    let binding_row = UntrustedTaskExecutionBinding::new(
        TASK_EXECUTION_BINDING_RECORD_SCHEMA,
        binding.link().clone(),
        binding.task_ref().clone(),
        binding.intake_stream_id().clone(),
        binding.intake_event_digest().clone(),
        binding.project_authority_receipt_digest().clone(),
        binding.successor_stream_id().clone(),
        binding.successor_task_created_event_digest().clone(),
        binding.task_spec_digest().clone(),
        binding.approval_subject_digest().clone(),
        binding.budget_digest().clone(),
        binding.verification_policy_digest().clone(),
        binding.binding_digest().clone(),
    );
    assert_eq!(binding_row, binding.to_untrusted());
    assert_eq!(binding.payload_digest(), binding.binding_digest());
    assert!(
        !binding
            .payload_canonical_bytes()
            .expect("binding bytes")
            .is_empty()
    );

    let attempt = &attempts[0];
    let attempt_row = UntrustedWorkerAttemptRow::new(
        WORKER_ATTEMPT_RECORD_SCHEMA,
        attempt.link().clone(),
        attempt.task_ref().clone(),
        attempt.successor_stream_id().clone(),
        attempt.task_spec_digest().clone(),
        attempt.binding_digest().clone(),
        attempt.budget_digest().clone(),
        attempt.attempt_id().clone(),
        attempt.attempt_number(),
        attempt.foreman_generation(),
        attempt.model(),
        attempt.reasoning(),
        attempt.model_reason(),
        attempt.writer_fence(),
        attempt.foreman_checkpoint_digest().clone(),
        attempt.approval_receipt_digest().clone(),
        attempt.packet_digest().clone(),
        attempt.worktree_digest().clone(),
        attempt.base_commit_digest().clone(),
        attempt.model_reason_digest().clone(),
        attempt.claimed_at(),
        attempt.payload_digest().clone(),
    );
    assert!(
        !attempt
            .payload_canonical_bytes()
            .expect("attempt bytes")
            .is_empty()
    );

    let observation_rows = observations
        .iter()
        .map(|observation| {
            UntrustedWorkerObservationRow::new(
                WORKER_OBSERVATION_RECORD_SCHEMA,
                observation.link().clone(),
                observation.task_ref().clone(),
                observation.successor_stream_id().clone(),
                observation.binding_digest().clone(),
                observation.attempt_id().clone(),
                observation.attempt_number(),
                observation.kind(),
                observation.thread_id(),
                observation.turn_id(),
                observation.app_server_generation(),
                observation.app_server_identity_digest().clone(),
                observation.observed_at(),
                observation.evidence_digest().clone(),
                observation.payload_digest().clone(),
            )
        })
        .collect::<Vec<_>>();
    assert!(
        !observations[0]
            .payload_canonical_bytes()
            .expect("observation bytes")
            .is_empty()
    );

    let verification_row = UntrustedTaskVerificationRow::new(
        TASK_VERIFICATION_RECORD_SCHEMA,
        verification.link().clone(),
        verification.task_ref().clone(),
        verification.successor_stream_id().clone(),
        verification.task_spec_digest().clone(),
        verification.binding_digest().clone(),
        verification.attempt_id().clone(),
        verification.attempt_number(),
        verification.outcome(),
        verification.verification_profile_digest().clone(),
        verification.base_commit_digest().clone(),
        verification.result_commit_digest().clone(),
        verification.tree_digest().clone(),
        verification.diff_digest().clone(),
        verification.result_digest().clone(),
        verification.evidence_artifact_digest().clone(),
        verification.review_digest().cloned(),
        verification.verified_at(),
        verification.payload_digest().clone(),
    );
    assert!(
        !verification
            .payload_canonical_bytes()
            .expect("verification bytes")
            .is_empty()
    );

    let replay = verify_untrusted_task_runtime_records(
        &stream,
        &binding,
        &[attempt_row],
        &observation_rows,
        &[verification_row],
    )
    .expect("adapter replay");
    assert_eq!(replay.verifications().len(), 1);
}
