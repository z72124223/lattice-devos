use lattice_cjson::CanonicalValue;
use lattice_contracts::{
    ContentDigest, ProjectId, ProjectSnapshotId, ResourceCounters, ResourceRequest, RuntimeKind,
    TaskId, TaskLedgerStreamIdentity,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CommandOutcome, CorrelationId, Diagnostic,
    EffectClaimId, FakeTaskLedger, LedgerCheckpoint, LedgerDenial, LedgerError, LedgerEventKind,
    LedgerOutcome, OutboxAdmissionState, ReasonCode, ResourceSnapshot, VerifiedStream,
    apply_append_plan, export_untrusted_snapshot, plan_append, verify_untrusted_snapshot,
    verify_untrusted_snapshot_against_checkpoint,
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
