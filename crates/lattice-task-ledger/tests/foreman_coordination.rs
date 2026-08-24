use lattice_contracts::{ContentDigest, RuntimeKind};
use lattice_foreman_state::{
    ForemanCheckpointIntent, ForemanSnapshot, ForemanState, SoleForemanBinding, reconstruct,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CorrelationId, ForemanAppendMetadata, LedgerError,
    LedgerEventKind, LedgerOutcome, ReasonCode, UntrustedForemanSnapshotRow,
    VerifiedForemanSnapshotRecord, VerifiedStream, apply_append_plan,
    foreman_coordination_identity, plan_append, plan_foreman_snapshot_append,
    preflight_foreman_checkpoint, verify_untrusted_foreman_snapshot_rows,
};

fn snapshot(generation: u64, state: ForemanState) -> ForemanSnapshot {
    snapshot_with_thread(SoleForemanBinding::THREAD, generation, state)
}

fn snapshot_with_thread(
    thread: impl Into<String>,
    generation: u64,
    state: ForemanState,
) -> ForemanSnapshot {
    ForemanSnapshot::new(
        SoleForemanBinding::WORKER,
        thread,
        SoleForemanBinding::TASK,
        "feature/task-079-durable-foreman-state",
        "lattice-worktrees/task-079-durable-foreman-state",
        "1234567890abcdef1234567890abcdef12345678",
        state,
        (state == ForemanState::Blocked).then(|| "dependency:TASK-087".to_owned()),
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        generation,
    )
    .expect("valid snapshot")
}

fn metadata(command: &str, second: u8) -> ForemanAppendMetadata {
    ForemanAppendMetadata::new(
        CommandId::new(command).expect("command"),
        CorrelationId::new(format!("correlation-{command}")).expect("correlation"),
        format!("2026-08-21T00:00:{second:02}Z"),
    )
    .expect("metadata")
}

fn intent(command: &str, generation: u64, second: u8) -> ForemanCheckpointIntent {
    ForemanCheckpointIntent::new(
        command,
        generation,
        format!("2026-08-21T00:00:{second:02}Z"),
        ForemanState::Active,
        None,
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("intent")
}

fn foreign_command(head: lattice_contracts::TaskLedgerStreamHead, command: &str) -> AppendCommand {
    AppendCommand::new(
        head,
        CommandId::new(command).expect("command"),
        CorrelationId::new(format!("correlation-{command}")).expect("correlation"),
        "2026-08-21T00:00:09Z",
        LedgerEventKind::TaskCreated,
        ActorId::new("foreign-actor").expect("actor"),
        ActionId::new("foreign-action").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("FOREIGN_EVENT").expect("reason"),
        ContentDigest::from_sha256("f".repeat(64)).expect("digest"),
        None,
        None,
    )
    .expect("foreign command")
}

#[test]
fn fixed_foreman_replay_rejects_foreign_events_and_extra_denied_commands() {
    let identity = foreman_coordination_identity().expect("identity");
    let vacant = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let foreign = plan_append(
        &vacant,
        foreign_command(vacant.head().clone(), "foreign-event"),
    )
    .expect("foreign plan");
    let contaminated = apply_append_plan(&vacant, &foreign).expect("apply foreign event");
    assert_eq!(
        verify_untrusted_foreman_snapshot_rows(&contaminated, &[]),
        Err(LedgerError::InvalidForemanSnapshot)
    );

    let first = plan_foreman_snapshot_append(
        &vacant,
        &[],
        metadata("foreman-1", 1),
        snapshot(1, ForemanState::Active),
    )
    .expect("first");
    let record = first.new_record().expect("record").clone();
    let stream = apply_append_plan(&vacant, first.ledger_plan()).expect("apply first");
    let denied = plan_append(
        &stream,
        foreign_command(vacant.head().clone(), "extra-denied-command"),
    )
    .expect("denied plan");
    assert!(denied.new_event().is_none());
    let contaminated = apply_append_plan(&stream, &denied).expect("retain denial");
    assert_eq!(
        verify_untrusted_foreman_snapshot_rows(&contaminated, &[record.to_untrusted()]),
        Err(LedgerError::InvalidForemanSnapshot)
    );
}

#[test]
fn ledger_preflight_rejects_first_and_next_generation_gaps_before_effects() {
    let identity = foreman_coordination_identity().expect("identity");
    let mut stream = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let mut records = Vec::new();
    assert_eq!(
        preflight_foreman_checkpoint(&stream, &records, &intent("foreman-gap", 2, 1)),
        Err(LedgerError::ForemanGenerationRollback)
    );

    let first = plan_foreman_snapshot_append(
        &stream,
        &records,
        metadata("foreman-1", 1),
        snapshot(1, ForemanState::Active),
    )
    .expect("first");
    records.push(first.new_record().expect("record").clone());
    stream = apply_append_plan(&stream, first.ledger_plan()).expect("apply");
    assert_eq!(
        preflight_foreman_checkpoint(&stream, &records, &intent("foreman-gap", 3, 2)),
        Err(LedgerError::ForemanGenerationRollback)
    );
    assert!(
        !preflight_foreman_checkpoint(&stream, &records, &intent("foreman-2", 2, 2))
            .expect("exact next")
    );
}

#[test]
fn fixed_stream_appends_exact_generations_and_replays_after_fresh_load() {
    let identity = foreman_coordination_identity().expect("fixed identity");
    let mut stream = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let mut records = Vec::new();

    let first = plan_foreman_snapshot_append(
        &stream,
        &records,
        metadata("foreman-1", 1),
        snapshot(1, ForemanState::Active),
    )
    .expect("first append");
    assert_eq!(
        first.ledger_plan().new_event().expect("event").kind(),
        LedgerEventKind::ForemanSnapshotRecorded
    );
    records.push(first.new_record().expect("record").clone());
    stream = apply_append_plan(&stream, first.ledger_plan()).expect("apply");

    let second = plan_foreman_snapshot_append(
        &stream,
        &records,
        metadata("foreman-2", 2),
        snapshot(2, ForemanState::Blocked),
    )
    .expect("second append");
    records.push(second.new_record().expect("record").clone());
    stream = apply_append_plan(&stream, second.ledger_plan()).expect("apply");

    let untrusted = records
        .iter()
        .map(VerifiedForemanSnapshotRecord::to_untrusted)
        .collect::<Vec<_>>();
    let recovered = verify_untrusted_foreman_snapshot_rows(&stream, &untrusted)
        .expect("fresh-process verification");
    let projection = reconstruct(
        recovered
            .into_iter()
            .map(|record| record.snapshot().clone()),
    )
    .expect("projection");
    assert!(projection.active().is_empty());
    assert_eq!(projection.blocked().len(), 1);
    assert_eq!(
        projection.next_action(),
        "unblock sole-foreman-v1: dependency:TASK-087"
    );
}

#[test]
fn exact_retry_is_idempotent_but_changed_command_and_generation_rollback_fail_closed() {
    let identity = foreman_coordination_identity().expect("fixed identity");
    let vacant = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let original_snapshot = snapshot(1, ForemanState::Active);
    let original_metadata = metadata("same-command", 1);
    let first = plan_foreman_snapshot_append(
        &vacant,
        &[],
        original_metadata.clone(),
        original_snapshot.clone(),
    )
    .expect("first");
    let records = vec![first.new_record().expect("record").clone()];
    let current = apply_append_plan(&vacant, first.ledger_plan()).expect("apply");

    let retry = plan_foreman_snapshot_append(
        &current,
        &records,
        original_metadata.clone(),
        original_snapshot,
    )
    .expect("retry");
    assert!(retry.ledger_plan().is_exact_retry());
    assert!(retry.new_record().is_none());

    assert_eq!(
        plan_foreman_snapshot_append(
            &current,
            &records,
            original_metadata,
            snapshot(2, ForemanState::Blocked),
        ),
        Err(LedgerError::CommandIdReuse)
    );
    assert_eq!(
        plan_foreman_snapshot_append(
            &current,
            &records,
            metadata("rollback", 2),
            snapshot(1, ForemanState::Active),
        ),
        Err(LedgerError::ForemanGenerationRollback)
    );
}

#[test]
fn foreman_append_rejects_generation_gap() {
    let identity = foreman_coordination_identity().expect("fixed identity");
    let mut stream = VerifiedStream::vacant(identity, RuntimeKind::Live).expect("vacant");
    let first = plan_foreman_snapshot_append(
        &stream,
        &[],
        metadata("foreman-gap-1", 1),
        snapshot(1, ForemanState::Active),
    )
    .expect("first");
    stream = apply_append_plan(&stream, first.ledger_plan()).expect("apply first");
    let retained = first.new_record().expect("record").clone();

    assert_eq!(
        plan_foreman_snapshot_append(
            &stream,
            &[retained],
            metadata("foreman-gap-3", 3),
            snapshot(3, ForemanState::Completed),
        ),
        Err(LedgerError::ForemanGenerationRollback)
    );
}

#[test]
fn foreman_append_rejects_generation_other_than_one_on_empty_stream() {
    let identity = foreman_coordination_identity().expect("fixed identity");
    let stream = VerifiedStream::vacant(identity, RuntimeKind::Live).expect("vacant");

    assert_eq!(
        plan_foreman_snapshot_append(
            &stream,
            &[],
            metadata("foreman-first-2", 2),
            snapshot(2, ForemanState::Active),
        ),
        Err(LedgerError::ForemanGenerationRollback)
    );
}

#[test]
fn foreman_append_rejects_thread_drift_before_planning_mutation() {
    let identity = foreman_coordination_identity().expect("fixed identity");
    let vacant = VerifiedStream::vacant(identity, RuntimeKind::Live).expect("vacant");
    let first = plan_foreman_snapshot_append(
        &vacant,
        &[],
        metadata("foreman-thread-1", 1),
        snapshot_with_thread(SoleForemanBinding::THREAD, 1, ForemanState::Active),
    )
    .expect("first");
    let current = apply_append_plan(&vacant, first.ledger_plan()).expect("apply");
    let records = [first.new_record().expect("record").clone()];

    assert_eq!(
        plan_foreman_snapshot_append(
            &current,
            &records,
            metadata("foreman-thread-2", 2),
            snapshot_with_thread("thread-b", 2, ForemanState::Blocked),
        ),
        Err(LedgerError::InvalidForemanSnapshot)
    );
    assert_eq!(current.head().sequence(), 1);
}

#[test]
fn unknown_child_schema_and_missing_child_fail_replay() {
    let identity = foreman_coordination_identity().expect("fixed identity");
    let vacant = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let first = plan_foreman_snapshot_append(
        &vacant,
        &[],
        metadata("foreman-1", 1),
        snapshot(1, ForemanState::Completed),
    )
    .expect("first");
    let current = apply_append_plan(&vacant, first.ledger_plan()).expect("apply");
    let row = first.new_record().expect("record").to_untrusted();

    assert_eq!(
        verify_untrusted_foreman_snapshot_rows(&current, &[]),
        Err(LedgerError::InvalidForemanSnapshot)
    );
    assert_eq!(
        verify_untrusted_foreman_snapshot_rows(
            &current,
            &[row.with_record_schema("lattice.task-ledger.foreman-record/2.0")],
        ),
        Err(LedgerError::UnknownForemanSnapshotVersion)
    );

    let record = first.new_record().expect("record");
    let forged_head = UntrustedForemanSnapshotRow::new(
        lattice_task_ledger::FOREMAN_RECORD_SCHEMA,
        record.stream_id().clone(),
        record.event_digest().clone(),
        record.command_id().clone(),
        record.request_digest().clone(),
        record.payload_digest().clone(),
        record.snapshot().clone(),
        current.head().clone(),
    );
    assert_eq!(
        verify_untrusted_foreman_snapshot_rows(&current, &[forged_head]),
        Err(LedgerError::InvalidForemanSnapshot)
    );
}
