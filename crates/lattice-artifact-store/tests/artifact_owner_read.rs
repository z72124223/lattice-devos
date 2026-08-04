#![allow(clippy::too_many_lines)]

mod support;

use lattice_artifact_store::{
    ArtifactCommandExecutionDisposition, ArtifactCommandOutcome, ArtifactStoreIdentity,
    ArtifactStoreLimits, ArtifactVerifiedReadExecution, FakeArtifactAuthorityDirectory,
    FakeArtifactStore, FakeDeleteOutcome,
};
use lattice_contracts::{
    ArtifactAuthorityHead, ArtifactObjectIdentity, ArtifactReadAuthorityAction,
    ArtifactReadClosureEvidenceKind, ArtifactReadStatus, ArtifactReferenceAuthorityAction,
    RuntimeKind, TaskId,
};

fn empty_owner() -> FakeArtifactStore {
    FakeArtifactStore::new(
        ArtifactStoreIdentity::new("fake-store-read").expect("store identity"),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("empty fake Artifact Store owner")
}

fn published_owner(
    project_id: &str,
    task_id: &str,
    reference_id: &str,
    bytes: &[u8],
) -> (
    FakeArtifactStore,
    FakeArtifactAuthorityDirectory,
    ArtifactObjectIdentity,
    ArtifactAuthorityHead,
) {
    let object = support::object_identity(project_id, bytes, 1);
    let mut owner = empty_owner();
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
    let mut directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut directory, &manifest);
    owner
        .publish(
            format!("command-publish-{reference_id}"),
            manifest,
            bytes,
            None,
            &directory,
        )
        .expect("publish fixture");
    let head = owner.current_head(&object).expect("published head");
    (owner, directory, object, head)
}

#[test]
fn public_owner_acquires_exact_retries_and_releases_one_read_claim() {
    let bytes = b"artifact-read-lifecycle";
    let task_id = TaskId::new("task-read-lifecycle").expect("task id");
    let (mut owner, mut directory, object, published_head) = published_owner(
        "project-read-lifecycle",
        task_id.as_str(),
        "reference-read-lifecycle",
        bytes,
    );
    let acquire = support::read_pair(
        &object,
        &task_id,
        "read-lifecycle",
        "read-owner-acquire",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &acquire);

    let acquired = owner
        .acquire_read(
            "command-acquire-read-lifecycle",
            &object,
            "holder-read-lifecycle",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            acquire.clone(),
            &published_head,
            &directory,
        )
        .expect("acquire read");
    assert_eq!(
        acquired.disposition(),
        ArtifactCommandExecutionDisposition::Recorded
    );
    assert_eq!(
        acquired.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_eq!(
        acquired
            .receipt()
            .lifecycle()
            .expect("read lifecycle receipt")
            .read()
            .expect("read head")
            .status(),
        ArtifactReadStatus::Active
    );
    let acquired_head = owner.current_head(&object).expect("acquired head");
    assert_eq!(acquired_head.object().active_read_count().get(), 1);
    assert_eq!(owner.terminal_command_count(), 2);

    let retry = owner
        .acquire_read(
            "command-acquire-read-lifecycle",
            &object,
            "holder-read-lifecycle",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            acquire,
            &published_head,
            &FakeArtifactAuthorityDirectory::default(),
        )
        .expect("exact retry precedes stale head and authority checks");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(retry.receipt(), acquired.receipt());
    assert_eq!(owner.terminal_command_count(), 2);

    let release = support::read_pair(
        &object,
        &task_id,
        "read-lifecycle",
        "read-owner-release",
        2,
        ArtifactReadAuthorityAction::ReleaseRead,
    );
    support::install_read_authority(&mut directory, &release);
    let released = owner
        .release_read(
            "command-release-read-lifecycle",
            &object,
            "read-lifecycle",
            release,
            &acquired_head,
            &directory,
        )
        .expect("release read");
    assert_eq!(
        released.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_eq!(
        released
            .receipt()
            .lifecycle()
            .expect("release lifecycle receipt")
            .read()
            .expect("released read head")
            .status(),
        ArtifactReadStatus::Released
    );
    assert_eq!(
        owner
            .current_head(&object)
            .expect("released head")
            .object()
            .active_read_count()
            .get(),
        0
    );
    assert_eq!(owner.terminal_command_count(), 3);
}

#[test]
fn stale_read_current_head_is_a_terminal_zero_lifecycle_mutation_denial() {
    let bytes = b"artifact-read-stale";
    let task_id = TaskId::new("task-read-stale").expect("task id");
    let (mut owner, mut directory, object, published_head) = published_owner(
        "project-read-stale",
        task_id.as_str(),
        "reference-read-stale",
        bytes,
    );
    let first = support::read_pair(
        &object,
        &task_id,
        "read-current",
        "read-owner-current",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &first);
    owner
        .acquire_read(
            "command-acquire-read-current",
            &object,
            "holder-current",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            first,
            &published_head,
            &directory,
        )
        .expect("first acquire");
    let current_head = owner.current_head(&object).expect("current read head");

    let stale = support::read_pair(
        &object,
        &task_id,
        "read-stale",
        "read-owner-stale",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &stale);
    let denied = owner
        .acquire_read(
            "command-acquire-read-stale",
            &object,
            "holder-stale",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            stale,
            &published_head,
            &directory,
        )
        .expect("stale head is retained as a terminal denial");
    assert_eq!(
        denied.disposition(),
        ArtifactCommandExecutionDisposition::Recorded
    );
    assert_eq!(
        denied.receipt().history().outcome(),
        ArtifactCommandOutcome::Denied
    );
    assert_eq!(
        denied.receipt().history().denial_code(),
        Some("ARTIFACT_STALE_CURRENT_HEAD")
    );
    assert!(denied.receipt().lifecycle().is_none());
    let after_denial = owner.current_head(&object).expect("head after denial");
    assert_eq!(after_denial.object().active_read_count().get(), 1);
    assert_eq!(
        after_denial.object().active_read_set_digest(),
        current_head.object().active_read_set_digest()
    );
    assert_eq!(owner.terminal_command_count(), 3);
}

#[test]
fn expired_read_remains_blocking_until_exact_closure_reconciliation() {
    let bytes = b"artifact-read-expiry";
    let task_id = TaskId::new("task-read-expiry").expect("task id");
    let (mut owner, mut directory, object, published_head) = published_owner(
        "project-read-expiry",
        task_id.as_str(),
        "reference-read-expiry",
        bytes,
    );
    let acquire = support::read_pair(
        &object,
        &task_id,
        "read-expiry",
        "read-owner-expiry-acquire",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &acquire);
    owner
        .acquire_read(
            "command-acquire-read-expiry",
            &object,
            "holder-read-expiry",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            acquire,
            &published_head,
            &directory,
        )
        .expect("acquire expiring read");
    let acquired_head = owner.current_head(&object).expect("acquired head");

    let expired = owner
        .expire_read(
            "command-expire-read",
            &object,
            "read-expiry",
            support::READ_CLOSED_AT,
            &acquired_head,
        )
        .expect("expire read");
    assert_eq!(
        expired.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_eq!(
        expired
            .receipt()
            .lifecycle()
            .expect("expiry lifecycle receipt")
            .read()
            .expect("expired read head")
            .status(),
        ArtifactReadStatus::ExpiredSuspect
    );
    let suspect_head = owner.current_head(&object).expect("suspect head");
    assert_eq!(suspect_head.object().active_read_count().get(), 1);

    let release = support::read_pair(
        &object,
        &task_id,
        "read-expiry",
        "read-owner-expiry-release",
        2,
        ArtifactReadAuthorityAction::ReleaseRead,
    );
    let closure = support::read_closure_pair(
        &object,
        &task_id,
        "read-expiry",
        "holder-read-expiry",
        "closure-read-expiry",
        1,
        ArtifactReadClosureEvidenceKind::HandleClosed,
        RuntimeKind::Fake,
        support::READ_CLOSED_AT,
    );
    support::install_read_authority(&mut directory, &release);
    support::install_read_closure_authority(&mut directory, &closure);
    let reconciled = owner
        .reconcile_read(
            "command-reconcile-read",
            &object,
            "read-expiry",
            release,
            &closure,
            &suspect_head,
            &directory,
        )
        .expect("reconcile exact closure");
    assert_eq!(
        reconciled.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_eq!(
        reconciled
            .receipt()
            .lifecycle()
            .expect("reconcile lifecycle receipt")
            .read()
            .expect("reconciled read head")
            .status(),
        ArtifactReadStatus::Released
    );
    assert_eq!(
        owner
            .current_head(&object)
            .expect("reconciled head")
            .object()
            .active_read_count()
            .get(),
        0
    );
    assert_eq!(owner.terminal_command_count(), 4);

    let retry = owner
        .expire_read(
            "command-expire-read",
            &object,
            "read-expiry",
            support::READ_CLOSED_AT,
            &acquired_head,
        )
        .expect("exact expiry retry must precede reconciled read-state lookup");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(retry.receipt(), expired.receipt());
    assert_eq!(owner.terminal_command_count(), 4);
}

#[test]
fn expired_read_exact_retry_survives_generation_retirement_and_reintroduction() {
    let bytes = b"artifact-read-expiry-generation-retirement";
    let task_id = TaskId::new("task-read-expiry-generation").expect("task id");
    let reference_id = "reference-read-expiry-generation";
    let (mut owner, mut directory, object, published_head) = published_owner(
        "project-read-expiry-generation",
        task_id.as_str(),
        reference_id,
        bytes,
    );
    let acquire = support::read_pair(
        &object,
        &task_id,
        "read-expiry-generation",
        "read-owner-expiry-generation-acquire",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &acquire);
    owner
        .acquire_read(
            "command-acquire-read-expiry-generation",
            &object,
            "holder-read-expiry-generation",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            acquire,
            &published_head,
            &directory,
        )
        .expect("acquire expiring read");
    let acquired_head = owner.current_head(&object).expect("acquired head");
    let expired = owner
        .expire_read(
            "command-expire-read-generation",
            &object,
            "read-expiry-generation",
            support::READ_CLOSED_AT,
            &acquired_head,
        )
        .expect("expire read");
    let suspect_head = owner.current_head(&object).expect("suspect head");

    let release_read = support::read_pair(
        &object,
        &task_id,
        "read-expiry-generation",
        "read-owner-expiry-generation-release",
        2,
        ArtifactReadAuthorityAction::ReleaseRead,
    );
    let closure = support::read_closure_pair(
        &object,
        &task_id,
        "read-expiry-generation",
        "holder-read-expiry-generation",
        "closure-read-expiry-generation",
        1,
        ArtifactReadClosureEvidenceKind::HandleClosed,
        RuntimeKind::Fake,
        support::READ_CLOSED_AT,
    );
    support::install_read_authority(&mut directory, &release_read);
    support::install_read_closure_authority(&mut directory, &closure);
    owner
        .reconcile_read(
            "command-reconcile-read-generation",
            &object,
            "read-expiry-generation",
            release_read,
            &closure,
            &suspect_head,
            &directory,
        )
        .expect("reconcile exact closure");

    let reconciled_head = owner.current_head(&object).expect("reconciled head");
    let release_reference = support::release_reference_pair(
        &object,
        task_id.as_str(),
        reference_id,
        "reference-owner-expiry-generation-release",
    );
    support::install_reference_authority(&mut directory, &release_reference);
    owner
        .release_reference(
            "command-release-reference-expiry-generation",
            &object,
            reference_id,
            release_reference,
            &reconciled_head,
            &directory,
        )
        .expect("release final reference");
    let released_head = owner.current_head(&object).expect("released head");
    let plan = owner
        .plan_delete(
            &object,
            &released_head,
            support::AFTER_RETENTION,
            support::GRACE,
        )
        .expect("delete plan");
    let sweep = support::sweep_pair(
        released_head.object(),
        &plan,
        "sweep-read-expiry-generation",
    );
    support::install_sweep_authority(&mut directory, &sweep);
    owner
        .claim_delete(
            "command-claim-delete-read-expiry-generation",
            &plan,
            &sweep,
            &directory,
        )
        .expect("claim old generation");
    let claimed_head = owner.current_head(&object).expect("claimed head");
    owner
        .apply_delete_outcome(
            "command-delete-read-expiry-generation",
            &object,
            plan.claim_token(),
            FakeDeleteOutcome::VerifiedDeleted,
            &claimed_head,
        )
        .expect("delete old generation");
    let deleted_head = owner.current_head(&object).expect("deleted head");

    let generation_two = support::object_identity("project-read-expiry-generation", bytes, 2);
    let generation_two_manifest = support::manifest(
        &generation_two,
        bytes.len() as u64,
        task_id.as_str(),
        "reference-read-expiry-generation-two",
        "reference-owner-read-expiry-generation-two",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    support::install_manifest_authority(&mut directory, &generation_two_manifest);
    owner
        .publish(
            "command-publish-read-expiry-generation-two",
            generation_two_manifest,
            bytes,
            Some(&deleted_head),
            &directory,
        )
        .expect("publish generation two");
    owner
        .current_head(&generation_two)
        .expect("generation two is current");

    let terminal_count = owner.terminal_command_count();
    let retry = owner
        .expire_read(
            "command-expire-read-generation",
            &object,
            "read-expiry-generation",
            support::READ_CLOSED_AT,
            &acquired_head,
        )
        .expect("exact expiry retry must precede retired-generation lookup");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(retry.receipt(), expired.receipt());
    assert_eq!(owner.terminal_command_count(), terminal_count);
}

#[test]
fn verified_read_returns_a_neutral_expiry_command_result_for_exact_denial_retry() {
    let bytes = b"artifact-read-denied-expiry-retry";
    let task_id = TaskId::new("task-read-denied-expiry").expect("task id");
    let (mut owner, mut directory, object, published_head) = published_owner(
        "project-read-denied-expiry",
        task_id.as_str(),
        "reference-read-denied-expiry",
        bytes,
    );
    let acquire = support::read_pair(
        &object,
        &task_id,
        "read-denied-expiry",
        "read-owner-denied-expiry",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &acquire);
    owner
        .acquire_read(
            "command-acquire-read-denied-expiry",
            &object,
            "holder-read-denied-expiry",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            acquire,
            &published_head,
            &directory,
        )
        .expect("acquire read");
    let acquired_head = owner.current_head(&object).expect("acquired head");

    let denied = owner
        .expire_read(
            "command-expire-read-denied",
            &object,
            "read-denied-expiry",
            "2026-07-30T00:05:00Z",
            &acquired_head,
        )
        .expect("early expiry is retained as a denial");
    assert_eq!(
        denied.receipt().history().outcome(),
        ArtifactCommandOutcome::Denied
    );
    assert_eq!(
        denied.receipt().history().denial_code(),
        Some("ARTIFACT_INVALID_READ_EVIDENCE")
    );

    let retry = owner
        .read_verified(
            "command-expire-read-denied",
            &object,
            "read-denied-expiry",
            "2026-07-30T00:05:00Z",
            &acquired_head,
        )
        .expect("exact denied expiry retry precedes a byte read");
    let ArtifactVerifiedReadExecution::ExpiryCommand(execution) = retry else {
        panic!("an exact expiry command retry must return its terminal receipt");
    };
    assert_eq!(
        execution.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(execution.receipt(), denied.receipt());
    assert_eq!(
        owner
            .current_head(&object)
            .expect("read remains active")
            .object()
            .active_read_count()
            .get(),
        1
    );
}

#[test]
fn verified_read_returns_exact_bytes_without_debug_or_receipt_disclosure() {
    let bytes = b"RAW-ARTIFACT-SECRET-DO-NOT-LOG";
    let task_id = TaskId::new("task-read-bytes").expect("task id");
    let (mut owner, mut directory, object, published_head) = published_owner(
        "project-read-bytes",
        task_id.as_str(),
        "reference-read-bytes",
        bytes,
    );
    let acquire = support::read_pair(
        &object,
        &task_id,
        "read-bytes",
        "read-owner-bytes",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &acquire);
    owner
        .acquire_read(
            "command-acquire-read-bytes",
            &object,
            "holder-read-bytes",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            acquire,
            &published_head,
            &directory,
        )
        .expect("acquire byte read");
    let acquired_head = owner.current_head(&object).expect("acquired head");
    let command_count_before = owner.terminal_command_count();

    let verified = owner
        .read_verified(
            "command-expire-read-bytes-if-needed",
            &object,
            "read-bytes",
            "2026-07-30T00:05:00Z",
            &acquired_head,
        )
        .expect("verified read");
    match &verified {
        ArtifactVerifiedReadExecution::Bytes(returned) => {
            assert_eq!(returned.as_slice(), bytes);
        }
        ArtifactVerifiedReadExecution::ExpiryCommand(_) => {
            panic!("an unexpired read must return bytes");
        }
    }
    assert_eq!(
        owner.terminal_command_count(),
        command_count_before,
        "a successful byte read must not consume the expiry command id"
    );

    let debug = format!("{verified:?}");
    let raw_text = std::str::from_utf8(bytes).expect("fixture utf-8");
    let raw_vector_debug = format!("{:?}", bytes.to_vec());
    assert!(!debug.contains(raw_text));
    assert!(!debug.contains(&raw_vector_debug));
    assert!(debug.contains("[ELIDED]"));
    let owner_debug = format!("{owner:?}");
    assert!(!owner_debug.contains(raw_text));
    assert!(!owner_debug.contains(&raw_vector_debug));
}
