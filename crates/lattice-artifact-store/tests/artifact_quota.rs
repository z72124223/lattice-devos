use lattice_artifact_store::{
    ArtifactCommandIdentity, ArtifactCommandQuotaRecord, ArtifactLimitKind,
    ArtifactObjectQuotaRecord, ArtifactObjectQuotaState, ArtifactQuotaDelta, ArtifactQuotaError,
    ArtifactQuotaProjection, ArtifactQuotaScope, ArtifactQuotaSnapshot, ArtifactReadIdentity,
    ArtifactReadQuotaRecord, ArtifactReadQuotaState, ArtifactReferenceIdentity,
    ArtifactReferenceQuotaRecord, ArtifactReferenceQuotaState, ArtifactStagingIdentity,
    ArtifactStagingReservation, ArtifactStagingState, ArtifactStoreIdentity, ArtifactStoreLimits,
    FakeArtifactStagingTerminalAuthority,
};
use lattice_contracts::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactGeneration,
    ArtifactObjectIdentity, ArtifactObjectKey, ContentDigest, ProjectId, RuntimeKind, TaskId,
};

fn project(value: &str) -> ProjectId {
    ProjectId::new(value).expect("project identity")
}

fn task(value: &str) -> TaskId {
    TaskId::new(value).expect("task identity")
}

fn object(project: &ProjectId, digest_digit: char, generation: u64) -> ArtifactObjectIdentity {
    let digest = ContentDigest::from_sha256(digest_digit.to_string().repeat(64))
        .expect("64 lowercase hex digest");
    ArtifactObjectIdentity::new(
        ArtifactObjectKey::new(project.clone(), digest),
        ArtifactGeneration::new(generation).expect("generation"),
    )
}

#[test]
#[allow(clippy::too_many_lines)]
fn recompute_counts_exact_identities_at_the_correct_scopes() {
    let store = ArtifactStoreIdentity::new("store-a").expect("store");
    let project_a = project("project-a");
    let project_b = project("project-b");
    let task_a = task("task-a");
    let task_b = task("task-b");
    let task_c = task("task-c");
    let object_a = object(&project_a, 'a', 1);
    let object_b = object(&project_b, 'b', 1);

    let snapshot = ArtifactQuotaSnapshot::new(
        store.clone(),
        vec![
            ArtifactObjectQuotaRecord::new(
                object_a.clone(),
                100,
                12,
                3,
                2,
                ArtifactObjectQuotaState::Available,
            )
            .expect("object a"),
            ArtifactObjectQuotaRecord::new(
                object_b.clone(),
                40,
                8,
                1,
                1,
                ArtifactObjectQuotaState::Available,
            )
            .expect("object b"),
        ],
        vec![
            ArtifactReferenceQuotaRecord::new(
                ArtifactReferenceIdentity::new(task_a.clone(), object_a.key().clone(), "ref-a1")
                    .expect("ref"),
                object_a.clone(),
                21,
                ArtifactReferenceQuotaState::Active,
            )
            .expect("reference"),
            ArtifactReferenceQuotaRecord::new(
                ArtifactReferenceIdentity::new(task_a.clone(), object_a.key().clone(), "ref-a2")
                    .expect("ref"),
                object_a.clone(),
                22,
                ArtifactReferenceQuotaState::Active,
            )
            .expect("reference"),
            ArtifactReferenceQuotaRecord::new(
                ArtifactReferenceIdentity::new(task_b.clone(), object_a.key().clone(), "ref-b1")
                    .expect("ref"),
                object_a.clone(),
                20,
                ArtifactReferenceQuotaState::Active,
            )
            .expect("reference"),
            ArtifactReferenceQuotaRecord::new(
                ArtifactReferenceIdentity::new(task_c.clone(), object_b.key().clone(), "ref-c1")
                    .expect("ref"),
                object_b.clone(),
                10,
                ArtifactReferenceQuotaState::Active,
            )
            .expect("reference"),
        ],
        vec![],
        vec![],
        vec![],
    );

    let report = snapshot
        .recompute(ArtifactStoreLimits::hard_maximums())
        .expect("quota report");
    let total = report.projection();
    assert_eq!(total.get(ArtifactLimitKind::ObjectBytes), 100);
    assert_eq!(total.get(ArtifactLimitKind::ActiveReferencesPerObject), 3);
    assert_eq!(total.get(ArtifactLimitKind::ReferencesPerTask), 2);
    assert_eq!(total.get(ArtifactLimitKind::ReferencesPerProject), 3);
    assert_eq!(total.get(ArtifactLimitKind::ReferencesPerStore), 4);
    assert_eq!(total.get(ArtifactLimitKind::UniqueBytesPerProject), 100);
    assert_eq!(total.get(ArtifactLimitKind::UniqueBytesPerStore), 140);

    let task_a_projection = report
        .task_projection(&project_a, &task_a)
        .expect("task a projection");
    let other_task_projection = report
        .task_projection(&project_a, &task_b)
        .expect("task b projection");
    assert_eq!(
        task_a_projection.get(ArtifactLimitKind::ActiveBytesPerTask),
        100
    );
    assert_eq!(
        task_a_projection.get(ArtifactLimitKind::ReferencesPerTask),
        2
    );
    assert_eq!(
        other_task_projection.get(ArtifactLimitKind::ActiveBytesPerTask),
        100
    );
    assert_eq!(
        other_task_projection.get(ArtifactLimitKind::ReferencesPerTask),
        1
    );

    assert_eq!(ArtifactLimitKind::ALL.len(), 30);
    assert_eq!(
        report.limit_snapshot_digest(),
        &ArtifactStoreLimits::hard_maximums()
            .limit_snapshot_digest()
            .expect("limit snapshot")
    );
}

#[test]
fn retained_object_states_hold_worst_case_quota_until_verified_deleted() {
    let store = ArtifactStoreIdentity::new("store-retention").expect("store");
    let project = project("project-retention");
    let states = [
        ArtifactObjectQuotaState::Available,
        ArtifactObjectQuotaState::DeleteClaimed,
        ArtifactObjectQuotaState::ReconciliationRequired,
        ArtifactObjectQuotaState::SealedOrphan,
        ArtifactObjectQuotaState::VerifiedDeleted,
    ];
    let objects = states
        .into_iter()
        .zip(['1', '2', '3', '4', '5'])
        .map(|(state, digest_digit)| {
            ArtifactObjectQuotaRecord::new(object(&project, digest_digit, 1), 10, 1, 0, 0, state)
                .expect("object")
        })
        .collect();
    let snapshot = ArtifactQuotaSnapshot::new(store, objects, vec![], vec![], vec![], vec![]);

    let report = snapshot
        .recompute(ArtifactStoreLimits::hard_maximums())
        .expect("quota report");
    assert_eq!(
        report
            .projection()
            .get(ArtifactLimitKind::ObjectsPerProject),
        4
    );
    assert_eq!(
        report
            .projection()
            .get(ArtifactLimitKind::UniqueBytesPerProject),
        40
    );
}

#[test]
fn staging_unknown_and_sealed_orphan_hold_bytes_until_verified_terminal_state() {
    let store = ArtifactStoreIdentity::new("store-staging").expect("store");
    let project = project("project-staging");
    let task = task("task-staging");
    let active_key = object(&project, '6', 1).key().clone();
    let unknown_key = object(&project, '7', 1).key().clone();
    let published_key = object(&project, '8', 1).key().clone();
    let cleaned_key = object(&project, '9', 1).key().clone();
    let mut active = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(active_key, task.clone(), "reservation-active")
            .expect("reservation"),
        30,
        1,
    )
    .expect("staging");
    active.mark_sealed_orphan().expect("seal");
    let mut unknown = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(unknown_key, task.clone(), "reservation-unknown")
            .expect("reservation"),
        20,
        1,
    )
    .expect("staging");
    unknown.mark_reconciliation_required().expect("unknown");
    let mut published = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(published_key, task.clone(), "reservation-published")
            .expect("reservation"),
        40,
        1,
    )
    .expect("staging");
    let mut cleaned = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(cleaned_key, task, "reservation-cleaned")
            .expect("reservation"),
        50,
        1,
    )
    .expect("staging");
    let mut authority = FakeArtifactStagingTerminalAuthority::default();
    let published_evidence = authority
        .issue(
            &published,
            ArtifactStagingState::VerifiedPublished,
            ContentDigest::from_sha256("9".repeat(64)).expect("observation"),
        )
        .expect("published evidence");
    published
        .apply_verified_terminal(&published_evidence, &authority)
        .expect("published");
    let cleaned_evidence = authority
        .issue(
            &cleaned,
            ArtifactStagingState::VerifiedCleaned,
            ContentDigest::from_sha256("a".repeat(64)).expect("observation"),
        )
        .expect("cleaned evidence");
    cleaned
        .apply_verified_terminal(&cleaned_evidence, &authority)
        .expect("cleaned");
    let snapshot = ArtifactQuotaSnapshot::new(
        store,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![active, unknown, published, cleaned],
    );
    let report = snapshot
        .recompute(ArtifactStoreLimits::hard_maximums())
        .expect("quota report");
    assert_eq!(
        report
            .projection()
            .get(ArtifactLimitKind::StagingBytesPerTask),
        50
    );
    assert_eq!(
        report
            .projection()
            .get(ArtifactLimitKind::StagingStreamsPerStore),
        2
    );
}

#[test]
fn staging_terminal_evidence_is_fixed_owner_exact_and_stale_safe() {
    let project = project("project-staging-evidence");
    let task = task("task-staging-evidence");
    let object_key = object(&project, 'b', 1).key().clone();
    let mut reservation = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(object_key, task.clone(), "reservation-evidence")
            .expect("reservation"),
        64,
        2,
    )
    .expect("staging");
    let mut authority = FakeArtifactStagingTerminalAuthority::default();
    let evidence = authority
        .issue(
            &reservation,
            ArtifactStagingState::VerifiedPublished,
            ContentDigest::from_sha256("b".repeat(64)).expect("observation"),
        )
        .expect("evidence");

    assert_eq!(evidence.receipt().producer_id(), ARTIFACT_STORE_PRODUCER_ID);
    assert_eq!(
        evidence.receipt().producer_version(),
        ARTIFACT_STORE_PRODUCER_VERSION
    );
    assert_eq!(evidence.receipt().runtime(), RuntimeKind::Fake);
    assert_eq!(
        authority
            .current_head(reservation.identity())
            .expect("current head"),
        evidence.current_head()
    );

    let replacement = authority
        .issue(
            &reservation,
            ArtifactStagingState::VerifiedCleaned,
            ContentDigest::from_sha256("c".repeat(64)).expect("new observation"),
        )
        .expect("replacement current evidence");
    let active_state = reservation.clone();
    assert_eq!(
        reservation.apply_verified_terminal(&evidence, &authority),
        Err(ArtifactQuotaError::StagingEvidenceMismatch)
    );
    assert_eq!(reservation, active_state);

    reservation
        .mark_reconciliation_required()
        .expect("state advances before stale evidence is applied");
    let stale_state = reservation.clone();
    assert_eq!(
        reservation.apply_verified_terminal(&replacement, &authority),
        Err(ArtifactQuotaError::StagingEvidenceMismatch)
    );
    assert_eq!(reservation, stale_state);

    let other_key = object(&project, 'c', 1).key().clone();
    let other = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(other_key, task, "reservation-other").expect("reservation"),
        64,
        2,
    )
    .expect("staging");
    assert_eq!(
        other.clone().apply_verified_terminal(&evidence, &authority),
        Err(ArtifactQuotaError::StagingEvidenceMismatch)
    );
}

#[test]
fn active_and_suspect_reads_retain_quota_but_verified_closed_reads_do_not() {
    let store = ArtifactStoreIdentity::new("store-read").expect("store");
    let project = project("project-read");
    let task = task("task-read");
    let object = object(&project, 'c', 1);
    let object_record = ArtifactObjectQuotaRecord::new(
        object.clone(),
        8,
        1,
        0,
        0,
        ArtifactObjectQuotaState::Available,
    )
    .expect("object");
    let reads = [
        ArtifactReadQuotaState::Active,
        ArtifactReadQuotaState::ExpiredSuspect,
        ArtifactReadQuotaState::ReconciliationRequired,
        ArtifactReadQuotaState::VerifiedClosed,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, state)| {
        ArtifactReadQuotaRecord::new(
            ArtifactReadIdentity::new(task.clone(), object.clone(), format!("read-{index}"))
                .expect("read"),
            object.clone(),
            state,
        )
    })
    .collect();
    let snapshot =
        ArtifactQuotaSnapshot::new(store, vec![object_record], vec![], reads, vec![], vec![]);

    let report = snapshot
        .recompute(ArtifactStoreLimits::hard_maximums())
        .expect("quota report");
    assert_eq!(
        report
            .projection()
            .get(ArtifactLimitKind::ActiveReadsPerObject),
        3
    );
    assert_eq!(report.projection().get(ArtifactLimitKind::ReadsPerTask), 3);
}

#[test]
fn checked_delta_exact_limit_plus_one_and_multi_field_failure_are_atomic() {
    let limits = ArtifactStoreLimits::hard_maximums()
        .tighten(ArtifactLimitKind::ObjectBytes, 10)
        .expect("limit")
        .tighten(ArtifactLimitKind::ReferencesPerStore, 10)
        .expect("limit");
    let mut projection = ArtifactQuotaProjection::zero()
        .with_value(ArtifactLimitKind::ObjectBytes, 9)
        .expect("projection")
        .with_value(ArtifactLimitKind::ReferencesPerStore, 5)
        .expect("projection");

    projection
        .checked_apply(
            &ArtifactQuotaDelta::single(ArtifactLimitKind::ObjectBytes, 1),
            limits,
        )
        .expect("exact limit succeeds");
    let exact = projection.clone();
    assert_eq!(projection.get(ArtifactLimitKind::ObjectBytes), 10);

    let exceeded = projection.checked_apply(
        &ArtifactQuotaDelta::single(ArtifactLimitKind::ObjectBytes, 1),
        limits,
    );
    assert!(matches!(
        exceeded,
        Err(ArtifactQuotaError::LimitExceeded {
            kind: ArtifactLimitKind::ObjectBytes,
            ..
        })
    ));
    assert_eq!(projection, exact);

    let mixed = ArtifactQuotaDelta::zero()
        .with_change(ArtifactLimitKind::ReferencesPerStore, 1)
        .expect("delta")
        .with_change(ArtifactLimitKind::ObjectBytes, -11)
        .expect("delta");
    let underflow = projection.checked_apply(&mixed, limits);
    assert!(matches!(
        underflow,
        Err(ArtifactQuotaError::Underflow {
            kind: ArtifactLimitKind::ObjectBytes,
            ..
        })
    ));
    assert_eq!(projection, exact);
}

#[test]
fn signed_bigint_overflow_and_recompute_failure_leave_existing_projection_unchanged() {
    let limits = ArtifactStoreLimits::hard_maximums();
    let mut projection = ArtifactQuotaProjection::zero()
        .with_value(ArtifactLimitKind::CommandsPerStore, i64::MAX)
        .expect("projection");
    let original = projection.clone();
    let overflow = projection.checked_apply(
        &ArtifactQuotaDelta::single(ArtifactLimitKind::CommandsPerStore, 1),
        limits,
    );
    assert!(matches!(
        overflow,
        Err(ArtifactQuotaError::Overflow {
            kind: ArtifactLimitKind::CommandsPerStore,
            ..
        })
    ));
    assert_eq!(projection, original);

    let overflow_project_a = project("project-overflow-a");
    let overflow_project_b = project("project-overflow-b");
    let overflow_snapshot = ArtifactQuotaSnapshot::new(
        ArtifactStoreIdentity::new("store-recompute-overflow").expect("store"),
        vec![
            ArtifactObjectQuotaRecord::new(
                object(&overflow_project_a, '6', 1),
                i64::MAX,
                1,
                0,
                0,
                ArtifactObjectQuotaState::Available,
            )
            .expect("large object"),
            ArtifactObjectQuotaRecord::new(
                object(&overflow_project_b, '7', 1),
                1,
                1,
                0,
                0,
                ArtifactObjectQuotaState::Available,
            )
            .expect("one-byte object"),
        ],
        vec![],
        vec![],
        vec![],
        vec![],
    );
    let recompute_overflow =
        projection.checked_recompute(&overflow_snapshot, ArtifactStoreLimits::hard_maximums());
    assert!(matches!(
        recompute_overflow,
        Err(ArtifactQuotaError::Overflow {
            kind: ArtifactLimitKind::UniqueBytesPerStore,
            ..
        })
    ));
    assert_eq!(projection, original);

    let project = project("project-over-limit");
    let snapshot = ArtifactQuotaSnapshot::new(
        ArtifactStoreIdentity::new("store-over-limit").expect("store"),
        vec![
            ArtifactObjectQuotaRecord::new(
                object(&project, 'd', 1),
                11,
                1,
                0,
                0,
                ArtifactObjectQuotaState::Available,
            )
            .expect("object"),
        ],
        vec![],
        vec![],
        vec![],
        vec![],
    );
    let tight = limits
        .tighten(ArtifactLimitKind::ObjectBytes, 10)
        .expect("tight limit");
    let failure = projection.checked_recompute(&snapshot, tight);
    assert!(matches!(
        failure,
        Err(ArtifactQuotaError::LimitExceeded {
            kind: ArtifactLimitKind::ObjectBytes,
            attempted: 11,
            limit: 10,
        })
    ));
    assert_eq!(projection, original);
}

#[test]
fn every_limit_has_exact_plus_one_overflow_underflow_and_zero_mutation_evidence() {
    for kind in ArtifactLimitKind::ALL {
        let limits = ArtifactStoreLimits::hard_maximums()
            .tighten(kind, 1)
            .expect("one is inside every hard maximum");
        let mut exact = ArtifactQuotaProjection::zero();
        exact
            .checked_apply(&ArtifactQuotaDelta::single(kind, 1), limits)
            .unwrap_or_else(|error| panic!("{} exact limit failed: {error}", kind.as_str()));
        assert_eq!(exact.get(kind), 1, "{} exact value", kind.as_str());

        let before_plus_one = exact.clone();
        assert_eq!(
            exact.checked_apply(&ArtifactQuotaDelta::single(kind, 1), limits),
            Err(ArtifactQuotaError::LimitExceeded {
                kind,
                attempted: 2,
                limit: 1,
            }),
            "{} plus one",
            kind.as_str()
        );
        assert_eq!(
            exact,
            before_plus_one,
            "{} plus-one failure must be atomic",
            kind.as_str()
        );

        let mut underflow = ArtifactQuotaProjection::zero();
        let before_underflow = underflow.clone();
        assert_eq!(
            underflow.checked_apply(&ArtifactQuotaDelta::single(kind, -1), limits),
            Err(ArtifactQuotaError::Underflow {
                kind,
                current: 0,
                delta: -1,
            }),
            "{} underflow",
            kind.as_str()
        );
        assert_eq!(
            underflow,
            before_underflow,
            "{} underflow failure must be atomic",
            kind.as_str()
        );

        let mut overflow = ArtifactQuotaProjection::zero()
            .with_value(kind, i64::MAX)
            .expect("signed bigint maximum");
        let before_overflow = overflow.clone();
        assert_eq!(
            overflow.checked_apply(
                &ArtifactQuotaDelta::single(kind, 1),
                ArtifactStoreLimits::hard_maximums(),
            ),
            Err(ArtifactQuotaError::Overflow {
                kind,
                current: i64::MAX,
                delta: 1,
            }),
            "{} overflow",
            kind.as_str()
        );
        assert_eq!(
            overflow,
            before_overflow,
            "{} overflow failure must be atomic",
            kind.as_str()
        );
    }
}

#[test]
fn quota_scopes_use_distinct_head_domains() {
    let store = ArtifactStoreIdentity::new("store-domain").expect("store");
    let project = project("project-domain");
    let task = task("task-domain");
    let object = object(&project, 'e', 1);
    let command = ArtifactCommandQuotaRecord::new(
        ArtifactCommandIdentity::new(task.clone(), object.key().clone(), "command-domain")
            .expect("command"),
        7,
    )
    .expect("command record");
    let snapshot = ArtifactQuotaSnapshot::new(
        store.clone(),
        vec![
            ArtifactObjectQuotaRecord::new(
                object.clone(),
                9,
                1,
                0,
                0,
                ArtifactObjectQuotaState::Available,
            )
            .expect("object"),
        ],
        vec![],
        vec![],
        vec![command],
        vec![],
    );
    let report = snapshot
        .recompute(ArtifactStoreLimits::hard_maximums())
        .expect("quota report");
    assert_eq!(
        report
            .object_key_projection(object.key())
            .expect("logical object projection")
            .get(ArtifactLimitKind::CommandsPerObject),
        1
    );
    let object_scope = ArtifactQuotaScope::Object(object);
    let task_scope = ArtifactQuotaScope::Task {
        project_id: project.clone(),
        task_id: task,
    };
    let project_scope = ArtifactQuotaScope::Project(project);
    let store_scope = ArtifactQuotaScope::Store(store);
    assert_eq!(object_scope.domain(), "lattice.artifact.quota.object-head");
    assert_eq!(task_scope.domain(), "lattice.artifact.quota.task-head");
    assert_eq!(
        project_scope.domain(),
        "lattice.artifact.quota.project-head"
    );
    assert_eq!(store_scope.domain(), "lattice.artifact.quota.store-head");
    assert_ne!(object_scope.domain(), task_scope.domain());
    assert_ne!(task_scope.domain(), project_scope.domain());
    assert_ne!(project_scope.domain(), store_scope.domain());
}

#[test]
fn duplicate_exact_identity_and_cross_project_relation_fail_closed() {
    let store = ArtifactStoreIdentity::new("store-invalid").expect("store");
    let project_a = project("project-invalid-a");
    let project_b = project("project-invalid-b");
    let task_b = task("task-b");
    let object_a = object(&project_a, 'f', 1);
    let object_b = object(&project_b, '0', 1);
    let object_record = ArtifactObjectQuotaRecord::new(
        object_a.clone(),
        1,
        1,
        0,
        0,
        ArtifactObjectQuotaState::Available,
    )
    .expect("object");
    let reference_identity =
        ArtifactReferenceIdentity::new(task_b, object_b.key().clone(), "cross-project-ref")
            .expect("reference id");
    let cross_project = ArtifactReferenceQuotaRecord::new(
        reference_identity.clone(),
        object_a.clone(),
        1,
        ArtifactReferenceQuotaState::Active,
    )
    .expect("reference");
    let duplicate = ArtifactReferenceQuotaRecord::new(
        reference_identity,
        object_a,
        1,
        ArtifactReferenceQuotaState::Released,
    )
    .expect("reference");
    let snapshot = ArtifactQuotaSnapshot::new(
        store,
        vec![object_record],
        vec![cross_project, duplicate],
        vec![],
        vec![],
        vec![],
    );

    assert!(matches!(
        snapshot.recompute(ArtifactStoreLimits::hard_maximums()),
        Err(ArtifactQuotaError::DuplicateIdentity { kind: "reference" }
            | ArtifactQuotaError::ProjectMismatch
            | ArtifactQuotaError::ObjectIdentityMismatch)
    ));
}

#[test]
fn command_storage_key_cannot_be_reused_with_different_task_attribution() {
    let store = ArtifactStoreIdentity::new("store-command-key").expect("store");
    let project = project("project-command-key");
    let object = object(&project, '8', 1);
    let object_record = ArtifactObjectQuotaRecord::new(
        object.clone(),
        1,
        1,
        0,
        0,
        ArtifactObjectQuotaState::Available,
    )
    .expect("object");
    let commands = ["task-command-a", "task-command-b"]
        .into_iter()
        .map(|task_id| {
            ArtifactCommandQuotaRecord::new(
                ArtifactCommandIdentity::new(
                    task(task_id),
                    object.key().clone(),
                    "same-command-id",
                )
                .expect("command identity"),
                10,
            )
            .expect("command record")
        })
        .collect();
    let snapshot =
        ArtifactQuotaSnapshot::new(store, vec![object_record], vec![], vec![], commands, vec![]);

    assert_eq!(
        snapshot.recompute(ArtifactStoreLimits::hard_maximums()),
        Err(ArtifactQuotaError::DuplicateIdentity { kind: "command" })
    );
}

#[test]
fn staging_and_command_quota_bind_logical_object_key_before_generation_exists() {
    let store = ArtifactStoreIdentity::new("store-prepublication").expect("store");
    let project = project("project-prepublication");
    let task = task("task-prepublication");
    let object_key = object(&project, 'd', 1).key().clone();
    let command = ArtifactCommandQuotaRecord::new(
        ArtifactCommandIdentity::new(task.clone(), object_key.clone(), "stage-command")
            .expect("command"),
        12,
    )
    .expect("command record");
    let staging = ArtifactStagingReservation::new(
        ArtifactStagingIdentity::new(object_key.clone(), task.clone(), "stage-reservation")
            .expect("staging identity"),
        128,
        1,
    )
    .expect("staging");
    let snapshot =
        ArtifactQuotaSnapshot::new(store, vec![], vec![], vec![], vec![command], vec![staging]);

    let report = snapshot
        .recompute(ArtifactStoreLimits::hard_maximums())
        .expect("prepublication quota has no generation dependency");
    assert_eq!(
        report
            .object_key_projection(&object_key)
            .expect("logical object command projection")
            .get(ArtifactLimitKind::CommandsPerObject),
        1
    );
    let task_projection = report
        .task_projection(&project, &task)
        .expect("task projection");
    assert_eq!(
        task_projection.get(ArtifactLimitKind::StagingBytesPerTask),
        128
    );
    assert_eq!(task_projection.get(ArtifactLimitKind::CommandsPerTask), 1);
}

#[test]
fn quota_local_identities_are_ascii_bounded_and_path_free() {
    assert!(ArtifactStoreIdentity::new("store_1-quota").is_ok());
    assert!(ArtifactStoreIdentity::new(" store").is_err());
    assert!(ArtifactStoreIdentity::new("store/path").is_err());
    assert!(ArtifactStoreIdentity::new("store\\path").is_err());
    assert!(ArtifactStoreIdentity::new("store:ads").is_err());
    assert!(ArtifactStoreIdentity::new("儲存區").is_err());
    assert!(ArtifactStoreIdentity::new(format!("s{}", "a".repeat(255))).is_ok());
    assert!(ArtifactStoreIdentity::new(format!("s{}", "a".repeat(256))).is_err());
}

#[test]
#[allow(clippy::too_many_lines)]
fn reference_identity_is_object_key_scoped_and_cannot_be_reused_after_generation_change() {
    let store = ArtifactStoreIdentity::new("store-reference-scope").expect("store");
    let project = project("project-reference-scope");
    let task = task("task-reference-scope");
    let object_a = object(&project, '1', 1);
    let object_b = object(&project, '2', 1);
    let same_local_id_a =
        ArtifactReferenceIdentity::new(task.clone(), object_a.key().clone(), "same-ref")
            .expect("reference a");
    let same_local_id_b =
        ArtifactReferenceIdentity::new(task.clone(), object_b.key().clone(), "same-ref")
            .expect("reference b");
    let allowed = ArtifactQuotaSnapshot::new(
        store.clone(),
        vec![
            ArtifactObjectQuotaRecord::new(
                object_a.clone(),
                10,
                1,
                0,
                0,
                ArtifactObjectQuotaState::Available,
            )
            .expect("object a"),
            ArtifactObjectQuotaRecord::new(
                object_b,
                20,
                1,
                0,
                0,
                ArtifactObjectQuotaState::Available,
            )
            .expect("object b"),
        ],
        vec![
            ArtifactReferenceQuotaRecord::new(
                same_local_id_a.clone(),
                object_a.clone(),
                1,
                ArtifactReferenceQuotaState::Active,
            )
            .expect("reference a"),
            ArtifactReferenceQuotaRecord::new(
                same_local_id_b,
                object(&project, '2', 1),
                1,
                ArtifactReferenceQuotaState::Active,
            )
            .expect("reference b"),
        ],
        vec![],
        vec![],
        vec![],
    );
    assert_eq!(
        allowed
            .recompute(ArtifactStoreLimits::hard_maximums())
            .expect("different keys may share local id")
            .projection()
            .get(ArtifactLimitKind::ReferencesPerStore),
        2
    );

    let generation_two = ArtifactObjectIdentity::new(
        object_a.key().clone(),
        ArtifactGeneration::new(2).expect("generation two"),
    );
    let reused = ArtifactQuotaSnapshot::new(
        store,
        vec![
            ArtifactObjectQuotaRecord::new(
                object_a.clone(),
                10,
                1,
                0,
                0,
                ArtifactObjectQuotaState::VerifiedDeleted,
            )
            .expect("old object"),
            ArtifactObjectQuotaRecord::new(
                generation_two.clone(),
                10,
                1,
                0,
                0,
                ArtifactObjectQuotaState::Available,
            )
            .expect("new object"),
        ],
        vec![
            ArtifactReferenceQuotaRecord::new(
                same_local_id_a.clone(),
                object_a,
                1,
                ArtifactReferenceQuotaState::Released,
            )
            .expect("old reference"),
            ArtifactReferenceQuotaRecord::new(
                same_local_id_a,
                generation_two,
                1,
                ArtifactReferenceQuotaState::Active,
            )
            .expect("reused reference"),
        ],
        vec![],
        vec![],
        vec![],
    );
    assert!(matches!(
        reused.recompute(ArtifactStoreLimits::hard_maximums()),
        Err(ArtifactQuotaError::DuplicateIdentity { kind: "reference" })
    ));
}

#[test]
fn only_one_generation_of_an_object_key_may_retain_quota() {
    let store = ArtifactStoreIdentity::new("store-generation").expect("store");
    let project = project("project-generation");
    let generation_one = object(&project, '3', 1);
    let generation_two = ArtifactObjectIdentity::new(
        generation_one.key().clone(),
        ArtifactGeneration::new(2).expect("generation two"),
    );
    let retained = |identity| {
        ArtifactObjectQuotaRecord::new(identity, 10, 1, 0, 0, ArtifactObjectQuotaState::Available)
            .expect("object")
    };
    let conflict = ArtifactQuotaSnapshot::new(
        store.clone(),
        vec![
            retained(generation_one.clone()),
            retained(generation_two.clone()),
        ],
        vec![],
        vec![],
        vec![],
        vec![],
    );
    assert_eq!(
        conflict.recompute(ArtifactStoreLimits::hard_maximums()),
        Err(ArtifactQuotaError::ConflictingRetainedGeneration)
    );

    let replacement = ArtifactQuotaSnapshot::new(
        store,
        vec![
            ArtifactObjectQuotaRecord::new(
                generation_one,
                10,
                1,
                0,
                0,
                ArtifactObjectQuotaState::VerifiedDeleted,
            )
            .expect("deleted old generation"),
            retained(generation_two),
        ],
        vec![],
        vec![],
        vec![],
        vec![],
    );
    assert_eq!(
        replacement
            .recompute(ArtifactStoreLimits::hard_maximums())
            .expect("one retained generation")
            .projection()
            .get(ArtifactLimitKind::ObjectsPerStore),
        1
    );
}
