mod support;

use lattice_artifact_store::{
    ArtifactCommandExecutionDisposition, ArtifactCommandKind, ArtifactCommandOutcome,
    ArtifactLimitKind, ArtifactQuotaScope, ArtifactStagingIdentity, ArtifactStagingReservation,
    ArtifactStagingState, ArtifactStoreAggregateError, ArtifactStoreIdentity, ArtifactStoreLimits,
    FakeArtifactStagingTerminalAuthority, FakeArtifactStore,
};
use lattice_contracts::TaskId;

use support::{marker, object_identity};

fn empty_owner(store_id: &str) -> FakeArtifactStore {
    FakeArtifactStore::new(
        ArtifactStoreIdentity::new(store_id).expect("store identity"),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("empty owner")
}

fn reservation(
    project_id: &str,
    object_bytes: &[u8],
    task_id: &str,
    reservation_id: &str,
    declared_bytes: i64,
    streams: i64,
) -> ArtifactStagingReservation {
    let object = object_identity(project_id, object_bytes, 1);
    ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(
            object.key().clone(),
            TaskId::new(task_id).expect("task identity"),
            reservation_id,
        )
        .expect("staging identity"),
        declared_bytes,
        streams,
    )
    .expect("staging reservation")
}

fn assert_staging_quota(
    owner: &FakeArtifactStore,
    identity: &ArtifactStagingIdentity,
    expected_bytes: i64,
    expected_streams: i64,
) {
    let task_scope = ArtifactQuotaScope::Task {
        project_id: identity.project_id().clone(),
        task_id: identity.task_id().clone(),
    };
    let task = owner.quota_head(&task_scope).expect("task quota head");
    assert_eq!(
        task.projection()
            .get(ArtifactLimitKind::StagingBytesPerTask),
        expected_bytes
    );
    assert_eq!(
        task.projection()
            .get(ArtifactLimitKind::StagingStreamsPerTask),
        expected_streams
    );

    let store_scope = ArtifactQuotaScope::Store(owner.store_id().clone());
    let store = owner.quota_head(&store_scope).expect("store quota head");
    assert_eq!(
        store
            .projection()
            .get(ArtifactLimitKind::StagingBytesPerStore),
        expected_bytes
    );
    assert_eq!(
        store
            .projection()
            .get(ArtifactLimitKind::StagingStreamsPerStore),
        expected_streams
    );
}

fn assert_recorded_staging_command(
    owner: &FakeArtifactStore,
    execution: &lattice_artifact_store::ArtifactStoreCommandExecution,
    outcome: ArtifactCommandOutcome,
) {
    assert_eq!(
        execution.disposition(),
        ArtifactCommandExecutionDisposition::Recorded
    );
    assert_eq!(
        execution.receipt().history().request().kind(),
        ArtifactCommandKind::Staging
    );
    assert_eq!(execution.receipt().history().outcome(), outcome);
    assert!(execution.receipt().lifecycle().is_none());
    assert_eq!(
        execution.receipt().quota_checkpoint_digest(),
        owner
            .quota_checkpoint_digest()
            .expect("current quota checkpoint")
    );
}

#[test]
fn reserve_is_exactly_idempotent_and_changed_command_reuse_is_zero_mutation() {
    let mut owner = empty_owner("fake-store-staging-reserve");
    let reservation = reservation(
        "project-staging-reserve",
        b"staging-reserve-object",
        "task-staging-reserve",
        "reservation-exact",
        128,
        1,
    );
    let identity = reservation.identity().clone();
    let checkpoint_before = owner
        .quota_checkpoint_digest()
        .expect("initial quota checkpoint")
        .clone();

    let first = owner
        .reserve_staging("command-reserve-exact", reservation.clone())
        .expect("first reservation");
    assert_recorded_staging_command(&owner, &first, ArtifactCommandOutcome::Applied);
    assert_ne!(
        owner
            .quota_checkpoint_digest()
            .expect("reserved quota checkpoint"),
        &checkpoint_before
    );
    assert_eq!(owner.terminal_command_count(), 1);
    assert_eq!(owner.staging_reservation_count(), 1);
    assert_eq!(
        owner
            .staging_reservation(&identity)
            .expect("active reservation")
            .state(),
        ArtifactStagingState::Active
    );
    assert_staging_quota(&owner, &identity, 128, 1);

    let after_first = owner.clone();
    let retry = owner
        .reserve_staging("command-reserve-exact", reservation)
        .expect("exact retry");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(retry.receipt(), first.receipt());
    assert_eq!(owner, after_first);

    let changed =
        ArtifactStagingReservation::new(identity, 129, 1).expect("changed reservation metrics");
    assert_eq!(
        owner.reserve_staging("command-reserve-exact", changed),
        Err(ArtifactStoreAggregateError::CommandIdReuse)
    );
    assert_eq!(owner, after_first);
}

#[test]
fn sealed_orphan_and_reconciliation_required_retain_exact_staging_quota() {
    let mut owner = empty_owner("fake-store-staging-fail-safe");
    let reservation = reservation(
        "project-staging-fail-safe",
        b"staging-fail-safe-object",
        "task-staging-fail-safe",
        "reservation-fail-safe",
        256,
        2,
    );
    let identity = reservation.identity().clone();
    owner
        .reserve_staging("command-reserve-fail-safe", reservation)
        .expect("reserve");
    let reserved_checkpoint = owner
        .quota_checkpoint_digest()
        .expect("reserved checkpoint")
        .clone();

    let sealed = owner
        .mark_staging_sealed_orphan("command-sealed-orphan", &identity)
        .expect("sealed orphan");
    assert_recorded_staging_command(&owner, &sealed, ArtifactCommandOutcome::Applied);
    assert_eq!(
        owner
            .staging_reservation(&identity)
            .expect("sealed reservation")
            .state(),
        ArtifactStagingState::SealedOrphan
    );
    assert_staging_quota(&owner, &identity, 256, 2);
    assert_ne!(
        owner.quota_checkpoint_digest().expect("sealed checkpoint"),
        &reserved_checkpoint
    );

    let sealed_checkpoint = owner
        .quota_checkpoint_digest()
        .expect("sealed checkpoint")
        .clone();
    let unknown = owner
        .mark_staging_reconciliation_required("command-staging-unknown", &identity)
        .expect("reconciliation required");
    assert_recorded_staging_command(&owner, &unknown, ArtifactCommandOutcome::Applied);
    assert_eq!(
        owner
            .staging_reservation(&identity)
            .expect("unknown reservation")
            .state(),
        ArtifactStagingState::ReconciliationRequired
    );
    assert_staging_quota(&owner, &identity, 256, 2);
    assert_ne!(
        owner
            .quota_checkpoint_digest()
            .expect("reconciliation checkpoint"),
        &sealed_checkpoint
    );
    assert_eq!(owner.terminal_command_count(), 3);
}

#[test]
fn only_typed_verified_published_or_cleaned_terminal_evidence_releases_quota() {
    let mut owner = empty_owner("fake-store-staging-terminal");
    let published = reservation(
        "project-staging-terminal",
        b"staging-published-object",
        "task-staging-terminal",
        "reservation-published",
        300,
        1,
    );
    let cleaned = reservation(
        "project-staging-terminal",
        b"staging-cleaned-object",
        "task-staging-terminal",
        "reservation-cleaned",
        200,
        1,
    );
    let published_identity = published.identity().clone();
    let cleaned_identity = cleaned.identity().clone();
    owner
        .reserve_staging("command-reserve-published", published.clone())
        .expect("published reservation");
    owner
        .reserve_staging("command-reserve-cleaned", cleaned)
        .expect("cleaned reservation");
    assert_staging_quota(&owner, &published_identity, 500, 2);

    let mut authority = FakeArtifactStagingTerminalAuthority::default();
    let published_evidence = authority
        .issue(
            &published,
            ArtifactStagingState::VerifiedPublished,
            marker('c'),
        )
        .expect("published evidence");
    let published_terminal = owner
        .apply_verified_staging_terminal(
            "command-terminal-published",
            &published_identity,
            &published_evidence,
            &authority,
        )
        .expect("verified published");
    assert_recorded_staging_command(&owner, &published_terminal, ArtifactCommandOutcome::Applied);
    assert_eq!(
        owner
            .staging_reservation(&published_identity)
            .expect("published reservation")
            .state(),
        ArtifactStagingState::VerifiedPublished
    );
    assert_staging_quota(&owner, &published_identity, 200, 1);

    let after_published = owner.clone();
    let exact_retry = owner
        .apply_verified_staging_terminal(
            "command-terminal-published",
            &published_identity,
            &published_evidence,
            &FakeArtifactStagingTerminalAuthority::default(),
        )
        .expect("exact retry must precede evidence and current-state evaluation");
    assert_eq!(
        exact_retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(exact_retry.receipt(), published_terminal.receipt());
    assert_eq!(owner, after_published);

    owner
        .mark_staging_sealed_orphan("command-seal-before-clean", &cleaned_identity)
        .expect("seal before cleanup");
    let sealed = owner
        .staging_reservation(&cleaned_identity)
        .expect("sealed reservation")
        .clone();
    let cleaned_evidence = authority
        .issue(&sealed, ArtifactStagingState::VerifiedCleaned, marker('d'))
        .expect("cleaned evidence");
    let cleaned_terminal = owner
        .apply_verified_staging_terminal(
            "command-terminal-cleaned",
            &cleaned_identity,
            &cleaned_evidence,
            &authority,
        )
        .expect("verified cleaned");
    assert_recorded_staging_command(&owner, &cleaned_terminal, ArtifactCommandOutcome::Applied);
    assert_eq!(
        owner
            .staging_reservation(&cleaned_identity)
            .expect("cleaned reservation")
            .state(),
        ArtifactStagingState::VerifiedCleaned
    );
    assert_staging_quota(&owner, &cleaned_identity, 0, 0);
    assert_eq!(owner.staging_reservation_count(), 2);
    assert_eq!(owner.terminal_command_count(), 5);
}

#[test]
fn stale_terminal_evidence_is_denied_retained_and_updates_history_checkpoint() {
    let mut owner = empty_owner("fake-store-staging-stale");
    let reservation = reservation(
        "project-staging-stale",
        b"staging-stale-object",
        "task-staging-stale",
        "reservation-stale",
        384,
        1,
    );
    let identity = reservation.identity().clone();
    owner
        .reserve_staging("command-reserve-stale", reservation.clone())
        .expect("reserve");

    let mut authority = FakeArtifactStagingTerminalAuthority::default();
    let stale = authority
        .issue(
            &reservation,
            ArtifactStagingState::VerifiedPublished,
            marker('e'),
        )
        .expect("first evidence");
    authority
        .issue(
            &reservation,
            ArtifactStagingState::VerifiedPublished,
            marker('f'),
        )
        .expect("replacement current evidence");
    let before_denial = owner.clone();
    let checkpoint_before = owner
        .quota_checkpoint_digest()
        .expect("checkpoint before denial")
        .clone();

    let denied = owner
        .apply_verified_staging_terminal("command-terminal-stale", &identity, &stale, &authority)
        .expect("stale evidence is a retained terminal denial");
    assert_recorded_staging_command(&owner, &denied, ArtifactCommandOutcome::Denied);
    assert_eq!(
        denied.receipt().history().denial_code(),
        Some("ARTIFACT_QUOTA_STAGING_EVIDENCE_MISMATCH")
    );
    assert_eq!(
        owner
            .staging_reservation(&identity)
            .expect("unchanged active reservation"),
        before_denial
            .staging_reservation(&identity)
            .expect("prior active reservation")
    );
    assert_staging_quota(&owner, &identity, 384, 1);
    assert_eq!(
        owner.terminal_command_count(),
        before_denial.terminal_command_count() + 1
    );
    assert_ne!(
        owner
            .quota_checkpoint_digest()
            .expect("checkpoint after denial"),
        &checkpoint_before
    );
}
