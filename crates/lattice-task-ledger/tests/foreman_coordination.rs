use lattice_contracts::RuntimeKind;
use lattice_foreman_state::{ForemanSnapshot, ForemanState, reconstruct};
use lattice_task_ledger::{
    CommandId, CorrelationId, ForemanAppendMetadata, LedgerError, LedgerEventKind,
    UntrustedForemanSnapshotRow, VerifiedForemanSnapshotRecord, VerifiedStream, apply_append_plan,
    foreman_coordination_identity, plan_foreman_snapshot_append,
    verify_untrusted_foreman_snapshot_rows,
};

fn snapshot(worker: &str, generation: u64, state: ForemanState) -> ForemanSnapshot {
    snapshot_with_thread(worker, format!("thread-{worker}"), generation, state)
}

fn snapshot_with_thread(
    worker: &str,
    thread: impl Into<String>,
    generation: u64,
    state: ForemanState,
) -> ForemanSnapshot {
    ForemanSnapshot::new(
        worker,
        thread,
        "TASK-079",
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

#[test]
fn fixed_stream_appends_exact_generations_and_replays_after_fresh_load() {
    let identity = foreman_coordination_identity().expect("fixed identity");
    let mut stream = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let mut records = Vec::new();

    let first = plan_foreman_snapshot_append(
        &stream,
        &records,
        metadata("foreman-1", 1),
        snapshot("worker-1", 1, ForemanState::Active),
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
        snapshot("worker-1", 2, ForemanState::Blocked),
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
        "unblock worker-1: dependency:TASK-087"
    );
}

#[test]
fn exact_retry_is_idempotent_but_changed_command_and_generation_rollback_fail_closed() {
    let identity = foreman_coordination_identity().expect("fixed identity");
    let vacant = VerifiedStream::vacant(identity, RuntimeKind::Fake).expect("vacant");
    let original_snapshot = snapshot("worker-1", 1, ForemanState::Active);
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
            snapshot("worker-1", 2, ForemanState::Blocked),
        ),
        Err(LedgerError::CommandIdReuse)
    );
    assert_eq!(
        plan_foreman_snapshot_append(
            &current,
            &records,
            metadata("rollback", 2),
            snapshot("worker-1", 1, ForemanState::Active),
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
        snapshot("worker-1", 1, ForemanState::Active),
    )
    .expect("first");
    stream = apply_append_plan(&stream, first.ledger_plan()).expect("apply first");
    let retained = first.new_record().expect("record").clone();

    assert_eq!(
        plan_foreman_snapshot_append(
            &stream,
            &[retained],
            metadata("foreman-gap-3", 3),
            snapshot("worker-1", 3, ForemanState::Completed),
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
            snapshot("worker-1", 2, ForemanState::Active),
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
        snapshot_with_thread("worker-1", "thread-a", 1, ForemanState::Active),
    )
    .expect("first");
    let current = apply_append_plan(&vacant, first.ledger_plan()).expect("apply");
    let records = [first.new_record().expect("record").clone()];

    assert_eq!(
        plan_foreman_snapshot_append(
            &current,
            &records,
            metadata("foreman-thread-2", 2),
            snapshot_with_thread("worker-1", "thread-b", 2, ForemanState::Blocked),
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
        snapshot("worker-1", 1, ForemanState::Completed),
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
