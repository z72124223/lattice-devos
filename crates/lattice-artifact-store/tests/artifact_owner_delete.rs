#![allow(clippy::too_many_lines)]

mod support;

use lattice_artifact_store::{
    ArtifactCommandExecutionDisposition, ArtifactCommandOutcome, ArtifactLifecycleError,
    ArtifactLimitKind, ArtifactQuotaScope, ArtifactReconciliationResult, ArtifactStoreIdentity,
    ArtifactStoreLimits, FakeArtifactAuthorityDirectory, FakeArtifactStore, FakeDeleteOutcome,
};
use lattice_contracts::{
    ArtifactAvailability, ArtifactDeleteStatus, ArtifactReadAuthorityAction,
    ArtifactReferenceAuthorityAction, RuntimeKind, TaskId,
};

use support::{
    AFTER_RETENTION, GRACE, READ_ACQUIRED_AT, READ_EXPIRES_AT, install_manifest_authority,
    install_read_authority, install_reference_authority, install_sweep_authority, manifest,
    object_identity, read_pair, release_reference_pair, sweep_pair,
};

fn empty_owner() -> FakeArtifactStore {
    FakeArtifactStore::new(
        ArtifactStoreIdentity::new("fake-store-delete").expect("store identity"),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("empty owner")
}

fn assert_applied(execution: &lattice_artifact_store::ArtifactStoreCommandExecution) {
    assert_eq!(
        execution.disposition(),
        ArtifactCommandExecutionDisposition::Recorded
    );
    assert_eq!(
        execution.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied,
        "unexpected denial: {:?}",
        execution.receipt().history().denial_code()
    );
    assert!(execution.receipt().lifecycle().is_some());
}

fn assert_denied(
    execution: &lattice_artifact_store::ArtifactStoreCommandExecution,
    expected_code: &str,
) {
    assert_eq!(
        execution.disposition(),
        ArtifactCommandExecutionDisposition::Recorded
    );
    assert_eq!(
        execution.receipt().history().outcome(),
        ArtifactCommandOutcome::Denied
    );
    assert_eq!(
        execution.receipt().history().denial_code(),
        Some(expected_code)
    );
    assert!(execution.receipt().lifecycle().is_none());
}

fn store_unique_bytes(owner: &FakeArtifactStore) -> i64 {
    owner
        .quota_head(&ArtifactQuotaScope::Store(owner.store_id().clone()))
        .expect("store quota head")
        .projection()
        .get(ArtifactLimitKind::UniqueBytesPerStore)
}

fn publish_initial(
    owner: &mut FakeArtifactStore,
    directory: &mut FakeArtifactAuthorityDirectory,
    project_id: &str,
    bytes: &[u8],
    task_id: &str,
    reference_id: &str,
) -> lattice_contracts::ArtifactObjectIdentity {
    let object = object_identity(project_id, bytes, 1);
    let artifact_manifest = manifest(
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
    install_manifest_authority(directory, &artifact_manifest);
    let published = owner
        .publish(
            format!("command-publish-{reference_id}"),
            artifact_manifest,
            bytes,
            None,
            directory,
        )
        .expect("publish");
    assert_applied(&published);
    object
}

fn release_last_reference(
    owner: &mut FakeArtifactStore,
    directory: &mut FakeArtifactAuthorityDirectory,
    object: &lattice_contracts::ArtifactObjectIdentity,
    task_id: &str,
    reference_id: &str,
) {
    let expected = owner.current_head(object).expect("published head");
    let authority = release_reference_pair(
        object,
        task_id,
        reference_id,
        &format!("release-{reference_id}"),
    );
    install_reference_authority(directory, &authority);
    let released = owner
        .release_reference(
            format!("command-release-{reference_id}"),
            object,
            reference_id,
            authority,
            &expected,
            directory,
        )
        .expect("release last reference");
    assert_applied(&released);
    assert_eq!(
        owner
            .current_head(object)
            .expect("released head")
            .object()
            .active_reference_count()
            .get(),
        0
    );
}

#[test]
fn release_plan_typed_claim_exact_retry_and_claimed_reference_block_are_atomic() {
    let bytes = b"delete-owner-claim";
    let task_id = "task-delete-claim";
    let reference_id = "reference-delete-claim";
    let mut owner = empty_owner();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object = publish_initial(
        &mut owner,
        &mut directory,
        "project-delete-claim",
        bytes,
        task_id,
        reference_id,
    );
    release_last_reference(&mut owner, &mut directory, &object, task_id, reference_id);

    let released_head = owner.current_head(&object).expect("released head");
    let plan = owner
        .plan_delete(&object, &released_head, AFTER_RETENTION, GRACE)
        .expect("delete plan");
    let authority = sweep_pair(released_head.object(), &plan, "sweep-claim");
    install_sweep_authority(&mut directory, &authority);
    let first = owner
        .claim_delete("command-delete-claim", &plan, &authority, &directory)
        .expect("typed sweep claim");
    assert_applied(&first);
    let claimed_head = owner.current_head(&object).expect("claimed head");
    assert_eq!(
        claimed_head.object().availability(),
        ArtifactAvailability::DeleteClaimed
    );
    assert_eq!(
        claimed_head.object().delete_status(),
        ArtifactDeleteStatus::Claimed
    );
    assert_eq!(
        claimed_head.object().delete_claim_token(),
        Some(plan.claim_token())
    );
    let terminal_count = owner.terminal_command_count();

    let retry = owner
        .claim_delete(
            "command-delete-claim",
            &plan,
            &authority,
            &FakeArtifactAuthorityDirectory::default(),
        )
        .expect("exact retry precedes authority/currentness checks");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(retry.receipt(), first.receipt());
    assert_eq!(owner.terminal_command_count(), terminal_count);

    let retained = manifest(
        &object,
        bytes.len() as u64,
        task_id,
        "reference-after-claim",
        "owner-after-claim",
        ArtifactReferenceAuthorityAction::AddReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &retained);
    let denied = owner
        .add_reference(
            "command-retain-after-claim",
            retained,
            &claimed_head,
            &directory,
        )
        .expect("claimed object denial is terminal");
    assert_denied(&denied, "ARTIFACT_OBJECT_UNAVAILABLE");
    let after_denial = owner.current_head(&object).expect("head after denial");
    assert_eq!(
        after_denial.object().availability(),
        ArtifactAvailability::DeleteClaimed
    );
    assert_eq!(
        after_denial.object().revision(),
        claimed_head.object().revision()
    );
}

#[test]
fn retain_race_stales_delete_plan_and_preserves_the_new_reference() {
    let bytes = b"delete-owner-retain-race";
    let task_id = "task-retain-race";
    let mut owner = empty_owner();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object = publish_initial(
        &mut owner,
        &mut directory,
        "project-retain-race",
        bytes,
        task_id,
        "reference-initial",
    );
    release_last_reference(
        &mut owner,
        &mut directory,
        &object,
        task_id,
        "reference-initial",
    );
    let planned_head = owner.current_head(&object).expect("planned head");
    let plan = owner
        .plan_delete(&object, &planned_head, AFTER_RETENTION, GRACE)
        .expect("delete plan");
    let sweep = sweep_pair(planned_head.object(), &plan, "sweep-retain-race");
    install_sweep_authority(&mut directory, &sweep);

    let retained = manifest(
        &object,
        bytes.len() as u64,
        task_id,
        "reference-retained",
        "owner-retained",
        ArtifactReferenceAuthorityAction::AddReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &retained);
    let retain = owner
        .add_reference("command-retain-race", retained, &planned_head, &directory)
        .expect("retain race applies first");
    assert_applied(&retain);
    let retained_head = owner.current_head(&object).expect("retained head");

    let denied = owner
        .claim_delete("command-stale-delete-plan", &plan, &sweep, &directory)
        .expect("stale plan is a terminal denial");
    assert_denied(&denied, "ARTIFACT_STALE_DELETE_PLAN");
    let after_denial = owner
        .current_head(&object)
        .expect("head after stale denial");
    assert_eq!(
        after_denial.object().availability(),
        ArtifactAvailability::Available
    );
    assert_eq!(after_denial.object().active_reference_count().get(), 1);
    assert_eq!(
        after_denial.object().revision(),
        retained_head.object().revision()
    );
    assert_eq!(
        owner
            .plan_delete(&object, &after_denial, AFTER_RETENTION, GRACE)
            .expect_err("retained reference blocks planning"),
        ArtifactLifecycleError::DeleteBlocked
    );
}

#[test]
fn known_no_effect_and_unknown_reconciliation_keep_worst_case_state_until_verified() {
    let bytes = b"delete-owner-reconcile";
    let task_id = "task-delete-reconcile";
    let mut owner = empty_owner();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object = publish_initial(
        &mut owner,
        &mut directory,
        "project-delete-reconcile",
        bytes,
        task_id,
        "reference-delete-reconcile",
    );
    let retained_bytes = store_unique_bytes(&owner);
    assert!(retained_bytes > 0);
    release_last_reference(
        &mut owner,
        &mut directory,
        &object,
        task_id,
        "reference-delete-reconcile",
    );

    let claim = |owner: &mut FakeArtifactStore,
                 directory: &mut FakeArtifactAuthorityDirectory,
                 suffix: &str| {
        let expected = owner.current_head(&object).expect("available head");
        let plan = owner
            .plan_delete(&object, &expected, AFTER_RETENTION, GRACE)
            .expect("delete plan");
        let sweep = sweep_pair(expected.object(), &plan, &format!("sweep-{suffix}"));
        install_sweep_authority(directory, &sweep);
        let execution = owner
            .claim_delete(format!("command-claim-{suffix}"), &plan, &sweep, directory)
            .expect("claim");
        assert_applied(&execution);
        plan
    };

    let first_plan = claim(&mut owner, &mut directory, "known-no-effect");
    let claimed_head = owner.current_head(&object).expect("claimed head");
    let no_effect = owner
        .apply_delete_outcome(
            "command-delete-no-effect",
            &object,
            first_plan.claim_token(),
            FakeDeleteOutcome::VerifiedNoEffect,
            &claimed_head,
        )
        .expect("known no-effect");
    assert_applied(&no_effect);
    let available = owner
        .current_head(&object)
        .expect("available after no-effect");
    assert_eq!(
        available.object().availability(),
        ArtifactAvailability::Available
    );
    assert_eq!(
        available.object().delete_status(),
        ArtifactDeleteStatus::VerifiedNoEffect
    );
    let outcome_count = owner.terminal_command_count();
    let outcome_retry = owner
        .apply_delete_outcome(
            "command-delete-no-effect",
            &object,
            first_plan.claim_token(),
            FakeDeleteOutcome::VerifiedNoEffect,
            &claimed_head,
        )
        .expect("delete-result exact retry");
    assert_eq!(
        outcome_retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(outcome_retry.receipt(), no_effect.receipt());
    assert_eq!(owner.terminal_command_count(), outcome_count);

    let second_plan = claim(&mut owner, &mut directory, "reconcile-available");
    let second_claimed = owner.current_head(&object).expect("second claim");
    let unknown = owner
        .apply_delete_outcome(
            "command-delete-unknown-available",
            &object,
            second_plan.claim_token(),
            FakeDeleteOutcome::Unknown,
            &second_claimed,
        )
        .expect("unknown result");
    assert_applied(&unknown);
    let reconciliation_head = owner.current_head(&object).expect("reconciliation head");
    assert_eq!(
        reconciliation_head.object().availability(),
        ArtifactAvailability::ReconciliationRequired
    );
    assert_eq!(
        reconciliation_head.object().delete_status(),
        ArtifactDeleteStatus::ReconciliationRequired
    );
    assert_eq!(store_unique_bytes(&owner), retained_bytes);
    let reconciled = owner
        .reconcile_delete(
            "command-reconcile-available",
            &object,
            second_plan.claim_token(),
            ArtifactReconciliationResult::VerifiedAvailable,
            &reconciliation_head,
        )
        .expect("verified available reconciliation");
    assert_applied(&reconciled);
    assert_eq!(
        owner
            .current_head(&object)
            .expect("available reconciliation head")
            .object()
            .availability(),
        ArtifactAvailability::Available
    );
    assert_eq!(store_unique_bytes(&owner), retained_bytes);

    let third_plan = claim(&mut owner, &mut directory, "reconcile-deleted");
    let third_claimed = owner.current_head(&object).expect("third claim");
    owner
        .apply_delete_outcome(
            "command-delete-unknown-deleted",
            &object,
            third_plan.claim_token(),
            FakeDeleteOutcome::Unknown,
            &third_claimed,
        )
        .expect("second unknown result");
    let third_reconciliation = owner.current_head(&object).expect("third reconciliation");
    owner.remove_bytes_for_test(&object);
    let deleted = owner
        .reconcile_delete(
            "command-reconcile-deleted",
            &object,
            third_plan.claim_token(),
            ArtifactReconciliationResult::VerifiedDeleted,
            &third_reconciliation,
        )
        .expect("verified deleted reconciliation");
    assert_applied(&deleted);
    assert_eq!(
        owner
            .current_head(&object)
            .expect("deleted head")
            .object()
            .availability(),
        ArtifactAvailability::Deleted
    );
    assert_eq!(store_unique_bytes(&owner), 0);

    let same_generation = manifest(
        &object,
        bytes.len() as u64,
        task_id,
        "reference-reintroduced-same-generation",
        "owner-reintroduced-same-generation",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &same_generation);
    let deleted_head = owner.current_head(&object).expect("current deleted head");
    let denied = owner
        .publish(
            "command-reintroduce-same-generation",
            same_generation,
            bytes,
            Some(&deleted_head),
            &directory,
        )
        .expect("same generation is a terminal denial");
    assert_denied(&denied, "ARTIFACT_GENERATION_MISMATCH");

    let generation_two = object_identity("project-delete-reconcile", bytes, 2);
    let next_manifest = manifest(
        &generation_two,
        bytes.len() as u64,
        task_id,
        "reference-reintroduced-generation-two",
        "owner-reintroduced-generation-two",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &next_manifest);
    let current_deleted_head = owner.current_head(&object).expect("refreshed deleted head");
    let reintroduced = owner
        .publish(
            "command-reintroduce-generation-two",
            next_manifest,
            bytes,
            Some(&current_deleted_head),
            &directory,
        )
        .expect("higher generation reintroduction");
    assert_applied(&reintroduced);
    let generation_two_head = owner
        .current_head(&generation_two)
        .expect("generation two head");
    assert_eq!(
        generation_two_head.object().availability(),
        ArtifactAvailability::Available
    );
    assert_eq!(generation_two_head.object().object().generation().get(), 2);
    assert_eq!(owner.object_count(), 1);
}

#[test]
fn active_reference_and_active_read_each_block_delete_planning() {
    let bytes = b"delete-owner-read-block";
    let task_id = "task-delete-read-block";
    let reference_id = "reference-delete-read-block";
    let mut owner = empty_owner();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object = publish_initial(
        &mut owner,
        &mut directory,
        "project-delete-read-block",
        bytes,
        task_id,
        reference_id,
    );
    let published_head = owner.current_head(&object).expect("published head");
    assert_eq!(
        owner
            .plan_delete(&object, &published_head, AFTER_RETENTION, GRACE)
            .expect_err("active reference blocks planning"),
        ArtifactLifecycleError::DeleteBlocked
    );

    let read = read_pair(
        &object,
        &TaskId::new(task_id).expect("task"),
        "read-delete-block",
        "read-owner-delete-block",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    install_read_authority(&mut directory, &read);
    let acquired = owner
        .acquire_read(
            "command-acquire-delete-block",
            &object,
            "holder-delete-block",
            READ_ACQUIRED_AT,
            READ_EXPIRES_AT,
            read,
            &published_head,
            &directory,
        )
        .expect("acquire blocking read");
    assert_applied(&acquired);
    release_last_reference(&mut owner, &mut directory, &object, task_id, reference_id);
    let read_blocked_head = owner.current_head(&object).expect("read-blocked head");
    assert_eq!(read_blocked_head.object().active_reference_count().get(), 0);
    assert_eq!(read_blocked_head.object().active_read_count().get(), 1);
    assert_eq!(
        owner
            .plan_delete(&object, &read_blocked_head, AFTER_RETENTION, GRACE)
            .expect_err("active read blocks planning"),
        ArtifactLifecycleError::DeleteBlocked
    );
}
