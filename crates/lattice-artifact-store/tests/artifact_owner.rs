mod support;

use lattice_artifact_store::{
    ArtifactCommandExecutionDisposition, ArtifactCommandOutcome, ArtifactQuotaScope,
    ArtifactStoreAggregateError, ArtifactStoreIdentity, ArtifactStoreLimits,
    FakeArtifactAuthorityDirectory, FakeArtifactStore,
};
use lattice_contracts::{ArtifactReferenceAuthorityAction, RuntimeKind};

use support::{install_manifest_authority, manifest, object_identity};

fn empty_owner() -> FakeArtifactStore {
    FakeArtifactStore::new(
        ArtifactStoreIdentity::new("fake-store-primary").expect("store identity"),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("empty owner")
}

#[test]
fn public_fake_starts_as_one_empty_owner_not_three_disconnected_mechanisms() {
    let owner = empty_owner();

    assert_eq!(owner.object_count(), 0);
    assert_eq!(owner.terminal_command_count(), 0);
    assert_eq!(owner.staging_reservation_count(), 0);
    let store_scope = ArtifactQuotaScope::Store(owner.store_id().clone());
    assert_eq!(
        owner
            .quota_head(&store_scope)
            .expect("initial store quota head")
            .revision()
            .get(),
        1
    );
    assert_ne!(
        owner
            .quota_head(&store_scope)
            .expect("initial store quota head")
            .head_digest(),
        owner
            .quota_checkpoint_digest()
            .expect("initial quota checkpoint")
    );
}

#[test]
fn publish_records_once_and_exact_retry_returns_the_identical_terminal_receipt() {
    let bytes = b"artifact-owner-alpha";
    let object = object_identity("project-alpha", bytes, 1);
    let mut owner = empty_owner();
    let artifact_manifest = manifest(
        &object,
        bytes.len() as u64,
        "task-alpha",
        "reference-alpha",
        "owner-alpha",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    let mut directory = FakeArtifactAuthorityDirectory::default();
    install_manifest_authority(&mut directory, &artifact_manifest);

    let first = owner
        .publish(
            "command-publish-alpha",
            artifact_manifest.clone(),
            bytes,
            None,
            &directory,
        )
        .expect("first publish");
    assert_eq!(
        first.disposition(),
        ArtifactCommandExecutionDisposition::Recorded
    );
    assert_eq!(
        first.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert!(first.receipt().lifecycle().is_some());
    assert_eq!(
        first.receipt().quota_checkpoint_digest(),
        owner
            .quota_checkpoint_digest()
            .expect("committed quota checkpoint")
    );
    assert_eq!(owner.object_count(), 1);
    assert_eq!(owner.terminal_command_count(), 1);

    let empty_directory = FakeArtifactAuthorityDirectory::default();
    let retry = owner
        .publish(
            "command-publish-alpha",
            artifact_manifest,
            bytes,
            None,
            &empty_directory,
        )
        .expect("exact retry must not re-evaluate changed authority state");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(retry.receipt(), first.receipt());
    assert_eq!(owner.object_count(), 1);
    assert_eq!(owner.terminal_command_count(), 1);
}

#[test]
fn publish_command_id_reuse_with_changed_bytes_is_zero_mutation() {
    let bytes = b"artifact-owner-beta";
    let object = object_identity("project-beta", bytes, 1);
    let mut owner = empty_owner();
    let artifact_manifest = manifest(
        &object,
        bytes.len() as u64,
        "task-beta",
        "reference-beta",
        "owner-beta",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    let mut directory = FakeArtifactAuthorityDirectory::default();
    install_manifest_authority(&mut directory, &artifact_manifest);
    owner
        .publish(
            "command-publish-beta",
            artifact_manifest.clone(),
            bytes,
            None,
            &directory,
        )
        .expect("first publish");
    let head_before = owner.current_head(&object).expect("current head");
    let receipt_count_before = owner.terminal_command_count();

    let error = owner
        .publish(
            "command-publish-beta",
            artifact_manifest,
            b"artifact-owner-BETA",
            None,
            &directory,
        )
        .expect_err("changed exact bytes must be command-id reuse");
    assert_eq!(error, ArtifactStoreAggregateError::CommandIdReuse);
    assert_eq!(
        owner.current_head(&object).expect("unchanged head"),
        head_before
    );
    assert_eq!(owner.terminal_command_count(), receipt_count_before);
}

#[test]
fn stale_publish_is_terminally_denied_and_exact_denied_retry_is_identical() {
    let bytes = b"artifact-owner-gamma";
    let object = object_identity("project-gamma", bytes, 1);
    let mut owner = empty_owner();
    let artifact_manifest = manifest(
        &object,
        bytes.len() as u64,
        "task-gamma",
        "reference-gamma",
        "owner-gamma",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    let mut directory = FakeArtifactAuthorityDirectory::default();
    install_manifest_authority(&mut directory, &artifact_manifest);
    owner
        .publish(
            "command-publish-gamma",
            artifact_manifest.clone(),
            bytes,
            None,
            &directory,
        )
        .expect("initial publish");
    let head_before_denial = owner.current_head(&object).expect("published head");

    let denied = owner
        .publish(
            "command-stale-gamma",
            artifact_manifest.clone(),
            bytes,
            None,
            &directory,
        )
        .expect("normal stale currentness is retained");
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
        Some("ARTIFACT_STALE_PUBLISH_HEAD")
    );
    assert!(denied.receipt().lifecycle().is_none());
    let head_after_denial = owner
        .current_head(&object)
        .expect("denial retains object with refreshed root evidence");
    assert_ne!(head_after_denial, head_before_denial);
    assert_eq!(
        head_after_denial.object().revision(),
        head_before_denial.object().revision()
    );
    assert_eq!(
        head_after_denial.object().availability(),
        head_before_denial.object().availability()
    );
    assert_eq!(
        head_after_denial.object().active_reference_count(),
        head_before_denial.object().active_reference_count()
    );
    assert_eq!(
        head_after_denial.object().active_read_count(),
        head_before_denial.object().active_read_count()
    );
    assert_eq!(
        head_after_denial.object().command_high_water().get(),
        head_before_denial.object().command_high_water().get() + 1
    );
    assert_eq!(owner.terminal_command_count(), 2);

    let retry = owner
        .publish(
            "command-stale-gamma",
            artifact_manifest,
            bytes,
            None,
            &FakeArtifactAuthorityDirectory::default(),
        )
        .expect("denied exact retry does not re-evaluate authority");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(retry.receipt(), denied.receipt());
    assert_eq!(owner.terminal_command_count(), 2);
}
