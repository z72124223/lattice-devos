#![allow(clippy::similar_names, clippy::too_many_lines)]

mod support;

use lattice_artifact_store::{
    ArtifactCommandOutcome, ArtifactLimitKind, ArtifactQuotaScope, ArtifactStagingIdentity,
    ArtifactStagingReservation, ArtifactStagingState, ArtifactStoreAggregateError,
    ArtifactStoreCommandExecution, ArtifactStoreIdentity, ArtifactStoreLimits,
    FakeArtifactAuthorityDirectory, FakeArtifactStore,
};
use lattice_contracts::{
    ArtifactObjectIdentity, ArtifactReadAuthorityAction, ArtifactReferenceAuthorityAction,
    ArtifactReferenceManifest, RuntimeKind, TaskId,
};

fn tightened_limits(entries: &[(ArtifactLimitKind, u64)]) -> ArtifactStoreLimits {
    entries.iter().copied().fold(
        ArtifactStoreLimits::hard_maximums(),
        |limits, (kind, value)| limits.tighten(kind, value).expect("legal lowered limit"),
    )
}

fn owner(store_id: &str, limits: ArtifactStoreLimits) -> FakeArtifactStore {
    FakeArtifactStore::new(
        ArtifactStoreIdentity::new(store_id).expect("store identity"),
        limits,
    )
    .expect("fake owner")
}

fn manifest(
    owner: &FakeArtifactStore,
    object: &ArtifactObjectIdentity,
    bytes: &[u8],
    task_id: &str,
    reference_id: &str,
) -> ArtifactReferenceManifest {
    support::manifest(
        object,
        bytes.len() as u64,
        task_id,
        reference_id,
        &format!("owner-{reference_id}"),
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        owner.limits(),
        RuntimeKind::Fake,
    )
}

fn quota_value(
    owner: &FakeArtifactStore,
    scope: &ArtifactQuotaScope,
    kind: ArtifactLimitKind,
) -> i64 {
    owner
        .quota_head(scope)
        .expect("quota head")
        .projection()
        .get(kind)
}

fn assert_current_checkpoint(owner: &FakeArtifactStore, execution: &ArtifactStoreCommandExecution) {
    assert_eq!(
        execution.receipt().quota_checkpoint_digest(),
        owner
            .quota_checkpoint_digest()
            .expect("owner quota checkpoint")
    );
}

#[test]
fn exact_object_byte_and_reference_limits_hold_while_plus_one_is_terminally_denied() {
    let limits = tightened_limits(&[
        (ArtifactLimitKind::ObjectBytes, 8),
        (ArtifactLimitKind::ObjectsPerTask, 2),
        (ArtifactLimitKind::ReferencesPerTask, 2),
        (ArtifactLimitKind::ActiveBytesPerTask, 14),
        (ArtifactLimitKind::ObjectsPerProject, 2),
        (ArtifactLimitKind::ReferencesPerProject, 2),
        (ArtifactLimitKind::UniqueBytesPerProject, 14),
        (ArtifactLimitKind::ObjectsPerStore, 2),
        (ArtifactLimitKind::ReferencesPerStore, 2),
        (ArtifactLimitKind::UniqueBytesPerStore, 14),
    ]);
    let mut owner = owner("fake-store-quota-object", limits);
    let task_id = TaskId::new("task-quota-object").expect("task");
    let project_id = "project-quota-object";
    let bytes_a = b"aaa";
    let bytes_b = b"bbbb";
    let bytes_c = b"c";
    let object_a = support::object_identity(project_id, bytes_a, 1);
    let object_b = support::object_identity(project_id, bytes_b, 1);
    let object_c = support::object_identity(project_id, bytes_c, 1);
    let manifest_a = manifest(
        &owner,
        &object_a,
        bytes_a,
        task_id.as_str(),
        "reference-quota-a",
    );
    let manifest_b = manifest(
        &owner,
        &object_b,
        bytes_b,
        task_id.as_str(),
        "reference-quota-b",
    );
    let manifest_c = manifest(
        &owner,
        &object_c,
        bytes_c,
        task_id.as_str(),
        "reference-quota-c",
    );
    let mut directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut directory, &manifest_a);
    support::install_manifest_authority(&mut directory, &manifest_b);
    support::install_manifest_authority(&mut directory, &manifest_c);

    let published_a = owner
        .publish(
            "command-quota-publish-a",
            manifest_a,
            bytes_a,
            None,
            &directory,
        )
        .expect("publish A");
    assert_eq!(
        published_a.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_current_checkpoint(&owner, &published_a);
    let old_a_full_head = owner.current_head(&object_a).expect("A head before B");
    let store_scope = ArtifactQuotaScope::Store(owner.store_id().clone());
    let store_head_before_b = owner
        .quota_head(&store_scope)
        .expect("store head before B")
        .clone();

    let published_b = owner
        .publish(
            "command-quota-publish-b",
            manifest_b,
            bytes_b,
            None,
            &directory,
        )
        .expect("publish B");
    assert_eq!(
        published_b.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_current_checkpoint(&owner, &published_b);
    let refreshed_a_full_head = owner.current_head(&object_a).expect("A head after B");
    assert_ne!(refreshed_a_full_head, old_a_full_head);
    assert_eq!(
        refreshed_a_full_head.object().revision(),
        old_a_full_head.object().revision()
    );
    assert_eq!(
        refreshed_a_full_head.object().active_reference_count(),
        old_a_full_head.object().active_reference_count()
    );
    assert_ne!(
        owner
            .quota_head(&store_scope)
            .expect("store head after B")
            .head_digest(),
        store_head_before_b.head_digest()
    );

    let task_scope = ArtifactQuotaScope::Task {
        project_id: object_a.key().project_id().clone(),
        task_id: task_id.clone(),
    };
    let project_scope = ArtifactQuotaScope::Project(object_a.key().project_id().clone());
    for (scope, kind, expected) in [
        (&task_scope, ArtifactLimitKind::ObjectsPerTask, 2),
        (&task_scope, ArtifactLimitKind::ReferencesPerTask, 2),
        (&task_scope, ArtifactLimitKind::ActiveBytesPerTask, 14),
        (&project_scope, ArtifactLimitKind::ObjectsPerProject, 2),
        (&project_scope, ArtifactLimitKind::ReferencesPerProject, 2),
        (&project_scope, ArtifactLimitKind::UniqueBytesPerProject, 14),
        (&store_scope, ArtifactLimitKind::ObjectsPerStore, 2),
        (&store_scope, ArtifactLimitKind::ReferencesPerStore, 2),
        (&store_scope, ArtifactLimitKind::UniqueBytesPerStore, 14),
    ] {
        assert_eq!(quota_value(&owner, scope, kind), expected);
    }
    let commands_before_denial =
        quota_value(&owner, &store_scope, ArtifactLimitKind::CommandsPerStore);
    let history_before_denial = quota_value(
        &owner,
        &store_scope,
        ArtifactLimitKind::HistoryBytesPerStore,
    );
    let semantic_a_before_denial = owner
        .current_head(&object_a)
        .expect("A before denial")
        .object()
        .clone();
    let semantic_b_before_denial = owner
        .current_head(&object_b)
        .expect("B before denial")
        .object()
        .clone();

    let denied_c = owner
        .publish(
            "command-quota-publish-c",
            manifest_c,
            bytes_c,
            None,
            &directory,
        )
        .expect("quota +1 is a retained terminal denial");
    assert_eq!(
        denied_c.receipt().history().outcome(),
        ArtifactCommandOutcome::Denied
    );
    assert_eq!(
        denied_c.receipt().history().denial_code(),
        Some("ARTIFACT_LIMIT_EXCEEDED")
    );
    assert!(denied_c.receipt().lifecycle().is_none());
    assert_current_checkpoint(&owner, &denied_c);
    assert_eq!(owner.object_count(), 2);
    assert!(
        owner
            .current_head_for_key(object_c.key())
            .expect("C lookup")
            .is_none()
    );
    let semantic_a_after_denial = owner
        .current_head(&object_a)
        .expect("A after denial")
        .object()
        .clone();
    let semantic_b_after_denial = owner
        .current_head(&object_b)
        .expect("B after denial")
        .object()
        .clone();
    assert_eq!(
        semantic_a_after_denial.revision(),
        semantic_a_before_denial.revision()
    );
    assert_eq!(
        semantic_a_after_denial.active_reference_count(),
        semantic_a_before_denial.active_reference_count()
    );
    assert_eq!(
        semantic_b_after_denial.revision(),
        semantic_b_before_denial.revision()
    );
    assert_eq!(
        semantic_b_after_denial.active_reference_count(),
        semantic_b_before_denial.active_reference_count()
    );
    assert_eq!(
        quota_value(&owner, &store_scope, ArtifactLimitKind::CommandsPerStore),
        commands_before_denial + 1
    );
    assert!(
        quota_value(
            &owner,
            &store_scope,
            ArtifactLimitKind::HistoryBytesPerStore
        ) > history_before_denial
    );
    assert_eq!(owner.terminal_command_count(), 3);
}

#[test]
fn released_reference_removes_task_object_and_active_byte_attribution() {
    let limits = tightened_limits(&[(ArtifactLimitKind::ObjectsPerTask, 1)]);
    let mut owner = owner("fake-store-quota-released-attribution", limits);
    let task_id = TaskId::new("task-quota-released-attribution").expect("task");
    let project_id = "project-quota-released-attribution";
    let first_bytes = b"quota-released-first";
    let first_object = support::object_identity(project_id, first_bytes, 1);
    let first_manifest = manifest(
        &owner,
        &first_object,
        first_bytes,
        task_id.as_str(),
        "reference-quota-released-first",
    );
    let mut directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut directory, &first_manifest);
    owner
        .publish(
            "command-quota-released-publish-first",
            first_manifest,
            first_bytes,
            None,
            &directory,
        )
        .expect("publish first object");
    let published_head = owner.current_head(&first_object).expect("published head");
    let release = support::release_reference_pair(
        &first_object,
        task_id.as_str(),
        "reference-quota-released-first",
        "owner-quota-released-first-release",
    );
    support::install_reference_authority(&mut directory, &release);
    owner
        .release_reference(
            "command-quota-released-release-first",
            &first_object,
            "reference-quota-released-first",
            release,
            &published_head,
            &directory,
        )
        .expect("release first reference");

    let task_scope = ArtifactQuotaScope::Task {
        project_id: first_object.key().project_id().clone(),
        task_id: task_id.clone(),
    };
    assert_eq!(
        quota_value(&owner, &task_scope, ArtifactLimitKind::ObjectsPerTask),
        0,
        "an object without an active reference is no longer attributed to the task"
    );
    assert_eq!(
        quota_value(&owner, &task_scope, ArtifactLimitKind::ActiveBytesPerTask),
        0,
        "released references no longer retain active task bytes"
    );

    let second_bytes = b"quota-released-second";
    let second_object = support::object_identity(project_id, second_bytes, 1);
    let second_manifest = manifest(
        &owner,
        &second_object,
        second_bytes,
        task_id.as_str(),
        "reference-quota-released-second",
    );
    support::install_manifest_authority(&mut directory, &second_manifest);
    let published = owner
        .publish(
            "command-quota-released-publish-second",
            second_manifest,
            second_bytes,
            None,
            &directory,
        )
        .expect("released attribution leaves room for the second object");
    assert_eq!(
        published.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_eq!(owner.object_count(), 2);
}

#[test]
fn configured_field_bytes_bounds_read_holder_and_projects_lifecycle_metadata() {
    let limits = tightened_limits(&[(ArtifactLimitKind::FieldBytes, 80)]);
    let mut owner = owner("fake-store-quota-field-read", limits);
    let bytes = b"quota-field-read-object";
    let object = support::object_identity("project-quota-field-read", bytes, 1);
    let task_id = TaskId::new("task-quota-field-read").expect("task");
    let manifest = manifest(
        &owner,
        &object,
        bytes,
        task_id.as_str(),
        "reference-quota-field-read",
    );
    let mut directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut directory, &manifest);
    owner
        .publish(
            "command-quota-field-read-publish",
            manifest,
            bytes,
            None,
            &directory,
        )
        .expect("publish");
    let published_head = owner.current_head(&object).expect("published head");

    let exact_holder = "h".repeat(80);
    let first_authority = support::read_pair(
        &object,
        &task_id,
        "read-quota-field-exact",
        "read-owner-quota-field-exact",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &first_authority);
    let acquired = owner
        .acquire_read(
            "command-quota-field-read-exact",
            &object,
            &exact_holder,
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            first_authority,
            &published_head,
            &directory,
        )
        .expect("holder at the configured field limit");
    assert_eq!(
        acquired.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );

    let object_scope = ArtifactQuotaScope::Object(object.clone());
    let task_scope = ArtifactQuotaScope::Task {
        project_id: object.key().project_id().clone(),
        task_id: task_id.clone(),
    };
    let project_scope = ArtifactQuotaScope::Project(object.key().project_id().clone());
    let store_scope = ArtifactQuotaScope::Store(owner.store_id().clone());
    for scope in [&object_scope, &task_scope, &project_scope, &store_scope] {
        assert_eq!(
            quota_value(&owner, scope, ArtifactLimitKind::FieldBytes),
            80,
            "every containing quota scope projects the retained holder metadata"
        );
    }

    let exact_head = owner.current_head(&object).expect("exact holder head");
    let oversized_authority = support::read_pair(
        &object,
        &task_id,
        "read-quota-field-oversized",
        "read-owner-quota-field-oversized",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &oversized_authority);
    let denied = owner
        .acquire_read(
            "command-quota-field-read-oversized",
            &object,
            &"x".repeat(81),
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            oversized_authority,
            &exact_head,
            &directory,
        )
        .expect("oversized holder is retained as a terminal denial");
    assert_eq!(
        denied.receipt().history().outcome(),
        ArtifactCommandOutcome::Denied
    );
    assert_eq!(
        denied.receipt().history().denial_code(),
        Some("ARTIFACT_LIMIT_EXCEEDED")
    );
    let after_denial = owner.current_head(&object).expect("head after denial");
    assert_eq!(
        after_denial.object().revision(),
        exact_head.object().revision()
    );
    assert_eq!(after_denial.object().active_read_count().get(), 1);
    for scope in [&object_scope, &task_scope, &project_scope, &store_scope] {
        assert_eq!(
            quota_value(&owner, scope, ArtifactLimitKind::FieldBytes),
            80
        );
    }
}

#[test]
fn configured_field_bytes_bounds_and_projects_delete_claim_token() {
    let limits = tightened_limits(&[(ArtifactLimitKind::FieldBytes, 64)]);
    let mut owner = owner("fake-store-quota-field-delete", limits);
    let bytes = b"quota-field-delete-object";
    let object = support::object_identity("project-quota-field-delete", bytes, 1);
    let task_id = TaskId::new("task-quota-field-delete").expect("task");
    let reference_id = "reference-quota-field-delete";
    let manifest = manifest(&owner, &object, bytes, task_id.as_str(), reference_id);
    let mut directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut directory, &manifest);
    owner
        .publish(
            "command-quota-field-delete-publish",
            manifest,
            bytes,
            None,
            &directory,
        )
        .expect("publish");
    let published_head = owner.current_head(&object).expect("published head");
    let release = support::release_reference_pair(
        &object,
        task_id.as_str(),
        reference_id,
        "release-owner-quota-field-delete",
    );
    support::install_reference_authority(&mut directory, &release);
    owner
        .release_reference(
            "command-quota-field-delete-release",
            &object,
            reference_id,
            release,
            &published_head,
            &directory,
        )
        .expect("release");
    let released_head = owner.current_head(&object).expect("released head");
    let plan = owner
        .plan_delete(
            &object,
            &released_head,
            support::AFTER_RETENTION,
            support::GRACE,
        )
        .expect("delete plan at exact configured field limit");
    assert_eq!(
        plan.claim_token().len(),
        64,
        "the generated claim token must fit the configured field limit"
    );
    assert!(
        plan.claim_token()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()),
        "the claim token remains the domain-separated SHA-256 hex value"
    );

    let authority = support::sweep_pair(released_head.object(), &plan, "sweep-quota-field-delete");
    support::install_sweep_authority(&mut directory, &authority);
    let claimed = owner
        .claim_delete(
            "command-quota-field-delete-claim",
            &plan,
            &authority,
            &directory,
        )
        .expect("claim delete");
    assert_eq!(
        claimed.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_eq!(
        owner
            .current_head(&object)
            .expect("claimed head")
            .object()
            .delete_claim_token(),
        Some(plan.claim_token())
    );

    let object_scope = ArtifactQuotaScope::Object(object.clone());
    let project_scope = ArtifactQuotaScope::Project(object.key().project_id().clone());
    let store_scope = ArtifactQuotaScope::Store(owner.store_id().clone());
    for scope in [&object_scope, &project_scope, &store_scope] {
        assert_eq!(
            quota_value(&owner, scope, ArtifactLimitKind::FieldBytes),
            64,
            "claim metadata remains part of every containing quota projection"
        );
    }
}

#[test]
fn exact_read_limit_denies_plus_one_without_adding_a_second_read() {
    let limits = tightened_limits(&[
        (ArtifactLimitKind::ActiveReadsPerObject, 1),
        (ArtifactLimitKind::ReadsPerTask, 1),
        (ArtifactLimitKind::ReadsPerProject, 1),
        (ArtifactLimitKind::ReadsPerStore, 1),
    ]);
    let mut owner = owner("fake-store-quota-read", limits);
    let bytes = b"quota-read-object";
    let object = support::object_identity("project-quota-read", bytes, 1);
    let task_id = TaskId::new("task-quota-read").expect("task");
    let manifest = manifest(
        &owner,
        &object,
        bytes,
        task_id.as_str(),
        "reference-quota-read",
    );
    let mut directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut directory, &manifest);
    owner
        .publish(
            "command-quota-read-publish",
            manifest,
            bytes,
            None,
            &directory,
        )
        .expect("publish");
    let published_head = owner.current_head(&object).expect("published head");

    let first_authority = support::read_pair(
        &object,
        &task_id,
        "read-quota-one",
        "read-quota-owner-one",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &first_authority);
    let first = owner
        .acquire_read(
            "command-quota-read-one",
            &object,
            "holder-quota-one",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            first_authority,
            &published_head,
            &directory,
        )
        .expect("first read reaches exact limit");
    assert_eq!(
        first.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_current_checkpoint(&owner, &first);
    let one_read_head = owner.current_head(&object).expect("one-read head");
    assert_eq!(one_read_head.object().active_read_count().get(), 1);

    let object_scope = ArtifactQuotaScope::Object(object.clone());
    let task_scope = ArtifactQuotaScope::Task {
        project_id: object.key().project_id().clone(),
        task_id: task_id.clone(),
    };
    let project_scope = ArtifactQuotaScope::Project(object.key().project_id().clone());
    let store_scope = ArtifactQuotaScope::Store(owner.store_id().clone());
    for (scope, kind) in [
        (&object_scope, ArtifactLimitKind::ActiveReadsPerObject),
        (&task_scope, ArtifactLimitKind::ReadsPerTask),
        (&project_scope, ArtifactLimitKind::ReadsPerProject),
        (&store_scope, ArtifactLimitKind::ReadsPerStore),
    ] {
        assert_eq!(quota_value(&owner, scope, kind), 1);
    }
    assert_eq!(
        owner
            .quota_head(&object_scope)
            .expect("object quota head")
            .limit_snapshot_digest(),
        &owner
            .limits()
            .limit_snapshot_digest()
            .expect("owner limit snapshot")
    );

    let second_authority = support::read_pair(
        &object,
        &task_id,
        "read-quota-two",
        "read-quota-owner-two",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    support::install_read_authority(&mut directory, &second_authority);
    let denied = owner
        .acquire_read(
            "command-quota-read-two",
            &object,
            "holder-quota-two",
            support::READ_ACQUIRED_AT,
            support::READ_EXPIRES_AT,
            second_authority,
            &one_read_head,
            &directory,
        )
        .expect("read +1 is retained as a terminal denial");
    assert_eq!(
        denied.receipt().history().outcome(),
        ArtifactCommandOutcome::Denied
    );
    assert_eq!(
        denied.receipt().history().denial_code(),
        Some("ARTIFACT_LIMIT_EXCEEDED")
    );
    assert_current_checkpoint(&owner, &denied);
    let after_denial = owner.current_head(&object).expect("head after denial");
    assert_eq!(after_denial.object().active_read_count().get(), 1);
    assert_eq!(
        after_denial.object().active_read_set_digest(),
        one_read_head.object().active_read_set_digest()
    );
    assert_eq!(
        after_denial.object().revision(),
        one_read_head.object().revision()
    );
    assert_eq!(owner.terminal_command_count(), 3);
}

#[test]
fn exact_staging_limit_denies_plus_one_without_retaining_the_candidate_reservation() {
    let limits = tightened_limits(&[
        (ArtifactLimitKind::StagingBytesPerTask, 10),
        (ArtifactLimitKind::StagingStreamsPerTask, 1),
        (ArtifactLimitKind::StagingBytesPerStore, 10),
        (ArtifactLimitKind::StagingStreamsPerStore, 1),
    ]);
    let mut owner = owner("fake-store-quota-staging", limits);
    let task_id = TaskId::new("task-quota-staging").expect("task");
    let object_a = support::object_identity("project-quota-staging", b"quota-staging-a", 1);
    let object_b = support::object_identity("project-quota-staging", b"quota-staging-b", 1);
    let first = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(
            object_a.key().clone(),
            task_id.clone(),
            "reservation-quota-one",
        )
        .expect("first staging identity"),
        10,
        1,
    )
    .expect("first reservation");
    let first_identity = first.identity().clone();
    let reserved = owner
        .reserve_staging("command-quota-staging-one", first)
        .expect("first reservation reaches exact limit");
    assert_eq!(
        reserved.receipt().history().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_current_checkpoint(&owner, &reserved);
    let task_scope = ArtifactQuotaScope::Task {
        project_id: object_a.key().project_id().clone(),
        task_id: task_id.clone(),
    };
    let store_scope = ArtifactQuotaScope::Store(owner.store_id().clone());
    assert_eq!(
        quota_value(&owner, &task_scope, ArtifactLimitKind::StagingBytesPerTask),
        10
    );
    assert_eq!(
        quota_value(
            &owner,
            &task_scope,
            ArtifactLimitKind::StagingStreamsPerTask
        ),
        1
    );
    assert_eq!(
        quota_value(
            &owner,
            &store_scope,
            ArtifactLimitKind::StagingBytesPerStore
        ),
        10
    );
    assert_eq!(
        quota_value(
            &owner,
            &store_scope,
            ArtifactLimitKind::StagingStreamsPerStore
        ),
        1
    );
    let history_before_denial = quota_value(
        &owner,
        &store_scope,
        ArtifactLimitKind::HistoryBytesPerStore,
    );

    let second = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(object_b.key().clone(), task_id, "reservation-quota-two")
            .expect("second staging identity"),
        1,
        1,
    )
    .expect("second reservation");
    let second_identity = second.identity().clone();
    let denied = owner
        .reserve_staging("command-quota-staging-two", second)
        .expect("staging +1 is retained as a terminal denial");
    assert_eq!(
        denied.receipt().history().outcome(),
        ArtifactCommandOutcome::Denied
    );
    assert_eq!(
        denied.receipt().history().denial_code(),
        Some("ARTIFACT_QUOTA_LIMIT_EXCEEDED")
    );
    assert_current_checkpoint(&owner, &denied);
    assert_eq!(owner.staging_reservation_count(), 1);
    assert_eq!(
        owner
            .staging_reservation(&first_identity)
            .expect("first retained reservation")
            .state(),
        ArtifactStagingState::Active
    );
    assert!(owner.staging_reservation(&second_identity).is_none());
    assert_eq!(
        quota_value(
            &owner,
            &store_scope,
            ArtifactLimitKind::StagingBytesPerStore
        ),
        10
    );
    assert_eq!(
        quota_value(
            &owner,
            &store_scope,
            ArtifactLimitKind::StagingStreamsPerStore
        ),
        1
    );
    assert_eq!(
        quota_value(&owner, &store_scope, ArtifactLimitKind::CommandsPerStore),
        2
    );
    assert!(
        quota_value(
            &owner,
            &store_scope,
            ArtifactLimitKind::HistoryBytesPerStore
        ) > history_before_denial
    );
}

#[test]
fn exact_command_and_history_capacity_rejects_an_unretainable_terminal_without_mutation() {
    let bytes = b"quota-capacity-object";
    let object = support::object_identity("project-quota-capacity", bytes, 1);
    let command_limits = tightened_limits(&[
        (ArtifactLimitKind::CommandsPerObject, 1),
        (ArtifactLimitKind::CommandsPerTask, 1),
        (ArtifactLimitKind::CommandsPerProject, 1),
        (ArtifactLimitKind::CommandsPerStore, 1),
    ]);
    let mut command_owner = owner("fake-store-quota-command", command_limits);
    let command_manifest = manifest(
        &command_owner,
        &object,
        bytes,
        "task-quota-capacity",
        "reference-quota-capacity",
    );
    let mut command_directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut command_directory, &command_manifest);
    let first = command_owner
        .publish(
            "command-capacity-1",
            command_manifest.clone(),
            bytes,
            None,
            &command_directory,
        )
        .expect("first command reaches exact count");
    assert_current_checkpoint(&command_owner, &first);
    let command_store_scope = ArtifactQuotaScope::Store(command_owner.store_id().clone());
    assert_eq!(
        quota_value(
            &command_owner,
            &command_store_scope,
            ArtifactLimitKind::CommandsPerStore
        ),
        1
    );
    let command_snapshot = command_owner.clone();
    assert_eq!(
        command_owner.publish(
            "command-capacity-2",
            command_manifest,
            bytes,
            None,
            &command_directory,
        ),
        Err(ArtifactStoreAggregateError::QuotaExhausted)
    );
    assert_eq!(command_owner, command_snapshot);

    let mut probe = owner(
        "fake-store-quota-history",
        ArtifactStoreLimits::hard_maximums(),
    );
    let probe_manifest = manifest(
        &probe,
        &object,
        bytes,
        "task-quota-capacity",
        "reference-quota-capacity",
    );
    let mut probe_directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut probe_directory, &probe_manifest);
    let probe_execution = probe
        .publish(
            "command-history-1",
            probe_manifest,
            bytes,
            None,
            &probe_directory,
        )
        .expect("measure one canonical history row");
    let exact_history_bytes = u64::try_from(
        probe_execution
            .receipt()
            .history()
            .canonical_bytes()
            .expect("canonical history bytes")
            .len(),
    )
    .expect("history byte count");
    assert_eq!(
        quota_value(
            &probe,
            &ArtifactQuotaScope::Store(probe.store_id().clone()),
            ArtifactLimitKind::HistoryBytesPerStore
        ),
        i64::try_from(exact_history_bytes).expect("history bytes fit signed BIGINT")
    );

    let history_limits = tightened_limits(&[
        (ArtifactLimitKind::HistoryBytesPerTask, exact_history_bytes),
        (
            ArtifactLimitKind::HistoryBytesPerProject,
            exact_history_bytes,
        ),
        (ArtifactLimitKind::HistoryBytesPerStore, exact_history_bytes),
    ]);
    let mut history_owner = owner("fake-store-quota-history", history_limits);
    let history_manifest = manifest(
        &history_owner,
        &object,
        bytes,
        "task-quota-capacity",
        "reference-quota-capacity",
    );
    let mut history_directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut history_directory, &history_manifest);
    let exact = history_owner
        .publish(
            "command-history-1",
            history_manifest.clone(),
            bytes,
            None,
            &history_directory,
        )
        .expect("one history row reaches exact byte limit");
    assert_current_checkpoint(&history_owner, &exact);
    let history_store_scope = ArtifactQuotaScope::Store(history_owner.store_id().clone());
    assert_eq!(
        quota_value(
            &history_owner,
            &history_store_scope,
            ArtifactLimitKind::HistoryBytesPerStore
        ),
        i64::try_from(exact_history_bytes).expect("history bytes fit signed BIGINT")
    );
    let history_snapshot = history_owner.clone();
    assert_eq!(
        history_owner.publish(
            "command-history-2",
            history_manifest,
            bytes,
            None,
            &history_directory,
        ),
        Err(ArtifactStoreAggregateError::QuotaExhausted)
    );
    assert_eq!(history_owner, history_snapshot);
}
