mod support;

use std::mem::size_of;

use lattice_artifact_store::{
    ArtifactCommandExecutionDisposition, ArtifactLifecycleError, ArtifactQuotaScope,
    ArtifactStagingIdentity, ArtifactStagingReservation, ArtifactStoreAggregateError,
    ArtifactStoreIdentity, ArtifactStoreLimits, ArtifactStoreReplayError,
    FakeArtifactAuthorityDirectory, FakeArtifactStore,
};
use lattice_cjson::{CanonicalValue, canonicalize};
use lattice_contracts::{
    ArtifactAvailability, ArtifactDeleteStatus, ArtifactObjectIdentity,
    ArtifactReadAuthorityAction, ArtifactReferenceAuthorityAction, RuntimeKind, TaskId,
};

fn empty_owner() -> FakeArtifactStore {
    FakeArtifactStore::new(
        ArtifactStoreIdentity::new("fake-store-replay").expect("store identity"),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("empty owner")
}

fn publish(
    owner: &mut FakeArtifactStore,
    directory: &mut FakeArtifactAuthorityDirectory,
    project_id: &str,
    task_id: &str,
    reference_id: &str,
    bytes: &[u8],
) -> ArtifactObjectIdentity {
    let object = support::object_identity(project_id, bytes, 1);
    let manifest = support::manifest(
        &object,
        bytes.len() as u64,
        task_id,
        reference_id,
        &format!("owner-{reference_id}"),
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    support::install_manifest_authority(directory, &manifest);
    owner
        .publish(
            format!("command-publish-{reference_id}"),
            manifest,
            bytes,
            None,
            directory,
        )
        .expect("publish replay fixture");
    object
}

fn populated_owner() -> (
    FakeArtifactStore,
    ArtifactObjectIdentity,
    ArtifactObjectIdentity,
    ArtifactStagingIdentity,
) {
    let mut owner = empty_owner();
    let mut directory = FakeArtifactAuthorityDirectory::default();

    let readable = publish(
        &mut owner,
        &mut directory,
        "project-replay-read",
        "task-replay-read",
        "reference-replay-read",
        b"snapshot-byte-secret-readable",
    );
    let published_read_head = owner.current_head(&readable).expect("published read head");
    let read_task = TaskId::new("task-replay-read").expect("read task");
    let read = support::read_pair(
        &readable,
        &read_task,
        "read-replay",
        "read-owner-replay",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &read);
    owner
        .acquire_read(
            "command-acquire-read-replay",
            &readable,
            "holder-replay",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            read,
            &published_read_head,
            &directory,
        )
        .expect("acquire replay read");

    let deleting = publish(
        &mut owner,
        &mut directory,
        "project-replay-delete",
        "task-replay-delete",
        "reference-replay-delete",
        b"snapshot-byte-secret-deleting",
    );
    let published_delete_head = owner
        .current_head(&deleting)
        .expect("published delete head");
    let release = support::release_reference_pair(
        &deleting,
        "task-replay-delete",
        "reference-replay-delete",
        "release-replay-delete",
    );
    support::install_reference_authority(&mut directory, &release);
    owner
        .release_reference(
            "command-release-reference-replay",
            &deleting,
            "reference-replay-delete",
            release,
            &published_delete_head,
            &directory,
        )
        .expect("release replay reference");
    let released_head = owner.current_head(&deleting).expect("released delete head");
    let plan = owner
        .plan_delete(
            &deleting,
            &released_head,
            support::AFTER_RETENTION,
            support::GRACE,
        )
        .expect("plan replay delete");
    let sweep = support::sweep_pair(released_head.object(), &plan, "sweep-replay-delete");
    support::install_sweep_authority(&mut directory, &sweep);
    owner
        .claim_delete("command-claim-delete-replay", &plan, &sweep, &directory)
        .expect("claim replay delete");

    let staged_object =
        support::object_identity("project-replay-stage", b"snapshot-stage-secret", 1);
    let staging = ArtifactStagingIdentity::new(
        staged_object.key().clone(),
        TaskId::new("task-replay-stage").expect("staging task"),
        "reservation-replay",
    )
    .expect("staging identity");
    let reservation =
        ArtifactStagingReservation::new(staging.clone(), 321, 1).expect("staging reservation");
    owner
        .reserve_staging("command-reserve-staging-replay", reservation.clone())
        .expect("reserve replay staging");
    owner
        .reserve_staging("command-denied-staging-replay", reservation)
        .expect("retain replay denial and denial tail");

    (owner, readable, deleting, staging)
}

#[test]
fn aggregate_snapshot_is_deterministic_secret_free_and_debug_elided() {
    let (owner, _, _, _) = populated_owner();

    let first_raw = owner.export_untrusted().expect("first strict snapshot");
    let second_raw = owner.export_untrusted().expect("second strict snapshot");
    let first_checkpoint = owner.checkpoint().expect("first checkpoint");
    let second_checkpoint = owner.checkpoint().expect("second checkpoint");

    assert_eq!(first_raw, second_raw);
    assert_eq!(first_checkpoint, second_checkpoint);
    assert_eq!(
        first_checkpoint.snapshot_digest(),
        second_checkpoint.snapshot_digest()
    );
    assert_eq!(
        first_checkpoint.checkpoint_digest(),
        second_checkpoint.checkpoint_digest()
    );
    assert_ne!(
        first_checkpoint.snapshot_digest(),
        first_checkpoint.checkpoint_digest(),
        "snapshot and checkpoint use separate hash domains"
    );

    let raw_text = String::from_utf8(canonicalize(&first_raw).expect("canonical raw").into_vec())
        .expect("canonical UTF-8");
    assert!(raw_text.contains("\"denial_count\":\"1\""));
    let checkpoint_debug = format!("{first_checkpoint:?}");
    let owner_debug = format!("{owner:?}");
    for secret in [
        "snapshot-byte-secret-readable",
        "snapshot-byte-secret-deleting",
        "snapshot-stage-secret",
    ] {
        assert!(!raw_text.contains(secret));
        assert!(!checkpoint_debug.contains(secret));
        assert!(!owner_debug.contains(secret));
    }
    assert!(
        !checkpoint_debug.contains("metadata"),
        "a rollback checkpoint must not retain a second complete owner"
    );
    assert!(checkpoint_debug.contains("trust_anchor"));
    assert!(checkpoint_debug.contains("payload_bytes: \"[ABSENT]\""));
    assert!(
        size_of::<lattice_artifact_store::ArtifactStoreCheckpoint>() <= 512,
        "the independently retained checkpoint must remain a compact trust anchor"
    );
}

#[test]
fn replay_round_trip_preserves_all_metadata_heads_but_no_payload_bytes() {
    let (mut owner, readable, deleting, staging) = populated_owner();
    let retry_reservation =
        ArtifactStagingReservation::new(staging.clone(), 321, 1).expect("retry reservation");
    let original_applied_retry = owner
        .reserve_staging("command-reserve-staging-replay", retry_reservation.clone())
        .expect("original exact applied retry");
    assert_eq!(
        original_applied_retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    let original_retry = owner
        .reserve_staging("command-denied-staging-replay", retry_reservation.clone())
        .expect("original exact denial retry");
    assert_eq!(
        original_retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    let raw = owner.export_untrusted().expect("strict snapshot");
    let checkpoint = owner.checkpoint().expect("trusted checkpoint");
    let mut replayed =
        FakeArtifactStore::replay_untrusted(&raw, &checkpoint).expect("trusted replay");

    assert_eq!(replayed.export_untrusted().expect("replayed snapshot"), raw);
    assert_eq!(replayed.object_count(), owner.object_count());
    assert_eq!(
        replayed.terminal_command_count(),
        owner.terminal_command_count()
    );
    assert_eq!(
        replayed.staging_reservation_count(),
        owner.staging_reservation_count()
    );
    assert_eq!(
        replayed
            .quota_checkpoint_digest()
            .expect("replayed quota checkpoint"),
        owner
            .quota_checkpoint_digest()
            .expect("original quota checkpoint")
    );
    let store_scope = ArtifactQuotaScope::Store(owner.store_id().clone());
    assert_eq!(
        replayed
            .quota_head(&store_scope)
            .expect("replayed store head"),
        owner.quota_head(&store_scope).expect("original store head")
    );
    assert_eq!(
        replayed
            .current_head(&readable)
            .expect("replayed read head"),
        owner.current_head(&readable).expect("original read head")
    );
    let replayed_delete = replayed
        .current_head(&deleting)
        .expect("replayed delete head");
    assert_eq!(
        replayed_delete,
        owner.current_head(&deleting).expect("original delete head")
    );
    assert_eq!(
        replayed_delete.object().availability(),
        ArtifactAvailability::DeleteClaimed
    );
    assert_eq!(
        replayed_delete.object().delete_status(),
        ArtifactDeleteStatus::Claimed
    );
    assert_eq!(
        replayed
            .staging_reservation(&staging)
            .expect("replayed staging"),
        owner
            .staging_reservation(&staging)
            .expect("original staging")
    );
    let replayed_applied_retry = replayed
        .reserve_staging("command-reserve-staging-replay", retry_reservation.clone())
        .expect("replayed exact applied retry");
    assert_eq!(replayed_applied_retry, original_applied_retry);
    let replayed_retry = replayed
        .reserve_staging("command-denied-staging-replay", retry_reservation)
        .expect("replayed exact denial retry");
    assert_eq!(replayed_retry, original_retry);

    let expected_head = replayed
        .current_head(&readable)
        .expect("current replayed read head");
    let missing = replayed
        .read_verified(
            "command-read-after-replay",
            &readable,
            "read-replay",
            "2026-07-30T00:02:00Z",
            &expected_head,
        )
        .expect_err("payload bytes are deliberately absent after replay");
    assert_eq!(
        missing,
        ArtifactStoreAggregateError::Lifecycle(ArtifactLifecycleError::MissingBytes)
    );
}

#[test]
fn duplicate_or_orphan_rows_are_rejected_after_context_free_reconstruction() {
    let (owner, _, _, _) = populated_owner();
    let raw = owner.export_untrusted().expect("strict snapshot");
    let checkpoint = owner.checkpoint().expect("trusted checkpoint");

    let mut duplicate_history = raw.clone();
    let histories = array_mut(field_mut(&mut duplicate_history, "histories"));
    histories.push(histories.first().expect("history row").clone());

    let mut orphan_reference = raw.clone();
    let lifecycle = field_mut(&mut orphan_reference, "lifecycle");
    let object = array_mut(field_mut(lifecycle, "objects"))
        .iter_mut()
        .find(|object| {
            matches!(
                field(object, "references"),
                CanonicalValue::Array(values) if !values.is_empty()
            )
        })
        .expect("object with a reference");
    let reference = array_mut(field_mut(object, "references"))
        .first_mut()
        .expect("reference row");
    *field_mut(reference, "map_reference_id") =
        CanonicalValue::String("orphan-reference-map-key".to_owned());

    for tampered in [duplicate_history, orphan_reference] {
        assert_snapshot_mismatch(&tampered, &checkpoint);
    }
}

#[test]
fn quota_and_full_terminal_receipt_tampering_are_rejected() {
    let (owner, _, _, _) = populated_owner();
    let raw = owner.export_untrusted().expect("strict snapshot");
    let checkpoint = owner.checkpoint().expect("trusted checkpoint");

    let terminals = field(&raw, "terminal_receipts");
    let CanonicalValue::Array(terminals) = terminals else {
        panic!("terminal receipts must be an array");
    };
    assert!(
        terminals
            .iter()
            .any(|terminal| !matches!(field(terminal, "lifecycle_receipt"), CanonicalValue::Null)),
        "the raw snapshot must retain the full lifecycle receipt, not only its digest"
    );

    let mut quota_tamper = raw.clone();
    let quota = field_mut(&mut quota_tamper, "quota");
    let quota_head = array_mut(field_mut(quota, "heads"))
        .first_mut()
        .expect("quota head");
    let projection = object_entries_mut(field_mut(quota_head, "projection"));
    let projection_value = &mut projection.first_mut().expect("quota projection field").1;
    let CanonicalValue::String(current) = projection_value else {
        panic!("quota projection must be a decimal string");
    };
    *current = if current == "0" { "1" } else { "0" }.to_owned();

    let mut terminal_tamper = raw.clone();
    let terminal = array_mut(field_mut(&mut terminal_tamper, "terminal_receipts"))
        .iter_mut()
        .find(|terminal| !matches!(field(terminal, "lifecycle_receipt"), CanonicalValue::Null))
        .expect("terminal lifecycle receipt");
    let lifecycle_receipt = field_mut(terminal, "lifecycle_receipt");
    *field_mut(lifecycle_receipt, "receipt_digest") = CanonicalValue::String("0".repeat(64));

    for tampered in [quota_tamper, terminal_tamper] {
        assert_snapshot_mismatch(&tampered, &checkpoint);
    }
}

#[test]
fn changed_unknown_extra_truncated_reordered_cross_scope_and_live_raw_are_rejected() {
    let (owner, _, _, _) = populated_owner();
    let raw = owner.export_untrusted().expect("strict snapshot");
    let checkpoint = owner.checkpoint().expect("trusted checkpoint");
    let before = owner.export_untrusted().expect("before tamper attempts");
    let checkpoint_digest = checkpoint.checkpoint_digest().clone();

    let mut changed = raw.clone();
    *field_mut(&mut changed, "store_id") = CanonicalValue::String("other-store".to_owned());

    let mut unknown = raw.clone();
    *field_mut(&mut unknown, "version") = CanonicalValue::String("9.9".to_owned());

    let mut extra = raw.clone();
    object_entries_mut(&mut extra).push((
        "unknown_extra".to_owned(),
        CanonicalValue::String("forbidden".to_owned()),
    ));

    let mut truncated = raw.clone();
    let histories = array_mut(field_mut(&mut truncated, "histories"));
    let first_history = histories.first_mut().expect("non-empty histories");
    let strict_history = field_mut(first_history, "strict_history");
    let records = array_mut(field_mut(strict_history, "records"));
    records.pop().expect("non-empty strict history");

    let mut reordered = raw.clone();
    object_entries_mut(&mut reordered).swap(0, 1);

    let mut cross_scope = raw.clone();
    let histories = array_mut(field_mut(&mut cross_scope, "histories"));
    *field_mut(
        histories.first_mut().expect("non-empty histories"),
        "project_id",
    ) = CanonicalValue::String("project-cross-scope".to_owned());

    let mut live = raw.clone();
    *field_mut(&mut live, "runtime") = CanonicalValue::String("LIVE".to_owned());

    for tampered in [
        changed,
        unknown,
        extra,
        truncated,
        reordered,
        cross_scope,
        live,
    ] {
        let error = FakeArtifactStore::replay_untrusted(&tampered, &checkpoint)
            .expect_err("strict replay must reject tampering");
        assert_eq!(error, ArtifactStoreReplayError::SnapshotMismatch);
        assert_eq!(error.code(), "ARTIFACT_STORE_REPLAY_SNAPSHOT_MISMATCH");
    }

    assert_eq!(
        owner.export_untrusted().expect("owner remains unchanged"),
        before
    );
    assert_eq!(checkpoint.checkpoint_digest(), &checkpoint_digest);
}

#[test]
fn coherent_older_prefix_is_rejected_by_the_newer_checkpoint() {
    let mut owner = empty_owner();
    let first_object = support::object_identity("project-prefix-a", b"prefix-a", 1);
    let first = ArtifactStagingIdentity::new(
        first_object.key().clone(),
        TaskId::new("task-prefix-a").expect("first task"),
        "reservation-prefix-a",
    )
    .expect("first staging identity");
    owner
        .reserve_staging(
            "command-prefix-a",
            ArtifactStagingReservation::new(first, 1, 1).expect("first reservation"),
        )
        .expect("first terminal command");
    let old_raw = owner.export_untrusted().expect("coherent old snapshot");
    let old_checkpoint = owner.checkpoint().expect("coherent old checkpoint");
    FakeArtifactStore::replay_untrusted(&old_raw, &old_checkpoint).expect("old pair is coherent");

    let second_object = support::object_identity("project-prefix-b", b"prefix-b", 1);
    let second = ArtifactStagingIdentity::new(
        second_object.key().clone(),
        TaskId::new("task-prefix-b").expect("second task"),
        "reservation-prefix-b",
    )
    .expect("second staging identity");
    owner
        .reserve_staging(
            "command-prefix-b",
            ArtifactStagingReservation::new(second, 1, 1).expect("second reservation"),
        )
        .expect("second terminal command");
    let current_checkpoint = owner.checkpoint().expect("newer checkpoint");

    assert_eq!(
        FakeArtifactStore::replay_untrusted(&old_raw, &current_checkpoint)
            .expect_err("new checkpoint rejects old coherent prefix"),
        ArtifactStoreReplayError::SnapshotMismatch
    );
}

#[test]
fn iterative_preflight_rejects_excessive_depth_before_checkpoint_use() {
    let owner = empty_owner();
    let checkpoint = owner.checkpoint().expect("checkpoint");
    let mut too_deep = CanonicalValue::Null;
    for _ in 0..100 {
        too_deep = CanonicalValue::Array(vec![too_deep]);
    }

    assert_eq!(
        FakeArtifactStore::replay_untrusted(&too_deep, &checkpoint).expect_err("depth bound"),
        ArtifactStoreReplayError::ReplayLimit { field: "depth" }
    );
    assert_eq!(
        ArtifactStoreReplayError::ReplayLimit { field: "depth" }.code(),
        "ARTIFACT_STORE_REPLAY_LIMIT"
    );
}

#[test]
fn iterative_preflight_accounts_for_control_character_escape_expansion() {
    let owner = empty_owner();
    let checkpoint = owner.checkpoint().expect("checkpoint");
    let mut expanded = owner.export_untrusted().expect("empty snapshot");
    object_entries_mut(&mut expanded).push((
        "unknown_control_expansion".to_owned(),
        CanonicalValue::String("\u{0001}".repeat(200_000)),
    ));

    assert_eq!(
        FakeArtifactStore::replay_untrusted(&expanded, &checkpoint)
            .expect_err("encoded canonical bytes exceed the checkpoint-bound limit"),
        ArtifactStoreReplayError::ReplayLimit {
            field: "canonical_bytes"
        }
    );
}

fn object_entries_mut(value: &mut CanonicalValue) -> &mut Vec<(String, CanonicalValue)> {
    let CanonicalValue::Object(entries) = value else {
        panic!("expected canonical object");
    };
    entries
}

fn array_mut(value: &mut CanonicalValue) -> &mut Vec<CanonicalValue> {
    let CanonicalValue::Array(values) = value else {
        panic!("expected canonical array");
    };
    values
}

fn field<'a>(value: &'a CanonicalValue, name: &str) -> &'a CanonicalValue {
    let CanonicalValue::Object(entries) = value else {
        panic!("expected canonical object");
    };
    entries
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value))
        .unwrap_or_else(|| panic!("missing canonical field {name}"))
}

fn field_mut<'a>(value: &'a mut CanonicalValue, name: &str) -> &'a mut CanonicalValue {
    object_entries_mut(value)
        .iter_mut()
        .find_map(|(key, value)| (key == name).then_some(value))
        .unwrap_or_else(|| panic!("missing canonical field {name}"))
}

fn assert_snapshot_mismatch(
    raw: &CanonicalValue,
    checkpoint: &lattice_artifact_store::ArtifactStoreCheckpoint,
) {
    let error = FakeArtifactStore::replay_untrusted(raw, checkpoint)
        .expect_err("strict replay must reject tampering");
    assert_eq!(error, ArtifactStoreReplayError::SnapshotMismatch);
}
