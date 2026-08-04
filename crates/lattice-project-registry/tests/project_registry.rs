use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_contracts::{
    ContentDigest, GitRefIdentity, ProjectClass, ProjectId, ProjectLifecycle, RuntimeKind,
};
use lattice_project_registry::{
    CommandId, FakeProjectRegistry, IdentityDimension, IdentityDrift, ReconciliationDecision,
    RegistryCheckpoint, RegistryCommand, RegistryCommandOutcome, RegistryDenial, RegistryError,
    RepositoryObservation, VerifiedRegistryState, export_untrusted_registry_snapshot,
    verify_untrusted_registry_snapshot, verify_untrusted_registry_snapshot_against_checkpoint,
};

const REGISTRY_1_1_OBSERVATION_DIGEST: &str =
    "5d97f120476be3ef83ff3b5facd86d657ebeac0f9b2f156143632442b1063c3d";
const REGISTRY_1_1_REQUEST_DIGEST: &str =
    "a034ca0119be1458abe6c0e87be487c17ab6ef86c4f806ce439080757b8441e5";
const REGISTRY_1_1_AUTHORITY_RECEIPT_DIGEST: &str =
    "7e4cb665c0a06cf56a70f3a1d3c869c5fe9fa31cce820585b6288afbe6d1a5a4";
const REGISTRY_1_1_COMMAND_RESULT_DIGEST: &str =
    "822cd564849ee0f8175dcb003fc7d2a6fea9d66c2a547738c18316afe372e3e2";
const REGISTRY_1_2_VACANT_LIVE_LOGICAL_STATE: &[u8] =
    br#"{"commands":[],"observations":[],"projects":[],"reservations":[],"runtime":"LIVE","schema_version":"1"}"#;
const REGISTRY_1_2_VACANT_RETAINED_BYTES: usize = 103;
const REGISTRY_1_2_VACANT_FAKE_CHECKPOINT_DIGEST: &str =
    "22ad9599c05ab384e720b8f1d91bdfbe1262360850602aa0f6b5fd79c1797f4f";
const REGISTRY_1_2_VACANT_LIVE_CHECKPOINT_DIGEST: &str =
    "5bb1f9d9adf7228bef7ea45cc029e79d71761c4dfed3abe086634917db38c173";

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid test digest")
}

fn project(value: &str) -> ProjectId {
    ProjectId::new(value).expect("valid project id")
}

fn command(value: &str) -> CommandId {
    CommandId::new(value).expect("valid command id")
}

fn observation(
    root: &str,
    root_identity: char,
    repository_identity: char,
    file_identity: char,
    primary_ref: &str,
    primary_identity: char,
) -> RepositoryObservation {
    RepositoryObservation::new(
        root,
        digest(root_identity),
        digest(repository_identity),
        digest(file_identity),
        GitRefIdentity::new(primary_ref, digest(primary_identity)).expect("valid primary ref"),
    )
    .expect("valid observation")
}

fn primary_observation() -> RepositoryObservation {
    observation(r"C:\work\lattice", '1', '2', '3', "refs/heads/main", '4')
}

fn canonical_text(name: &str, value: &str) -> (String, CanonicalValue) {
    (name.to_owned(), CanonicalValue::String(value.to_owned()))
}

fn registry_1_2_vacant_logical_state(runtime: &str) -> CanonicalValue {
    CanonicalValue::Object(vec![
        canonical_text("schema_version", "1"),
        canonical_text("runtime", runtime),
        ("observations".to_owned(), CanonicalValue::Array(Vec::new())),
        ("projects".to_owned(), CanonicalValue::Array(Vec::new())),
        ("commands".to_owned(), CanonicalValue::Array(Vec::new())),
        ("reservations".to_owned(), CanonicalValue::Array(Vec::new())),
    ])
}

fn registry_1_2_vacant_checkpoint_value(
    runtime: &str,
    logical_state: CanonicalValue,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        canonical_text("schema_version", "1"),
        canonical_text("runtime", runtime),
        canonical_text("command_ordinal", "0"),
        canonical_text("observation_count", "0"),
        canonical_text("project_count", "0"),
        canonical_text("command_count", "0"),
        canonical_text("reservation_count", "0"),
        canonical_text("retained_bytes", "103"),
        ("logical_state".to_owned(), logical_state),
    ])
}

#[test]
fn registry_1_2_vacant_checkpoint_literals_match_canonical_fixture() {
    let domain = HashDomain::new("lattice.project-registry.checkpoint", "1")
        .expect("valid checkpoint hash domain");
    for (runtime, expected_digest) in [
        ("FAKE", REGISTRY_1_2_VACANT_FAKE_CHECKPOINT_DIGEST),
        ("LIVE", REGISTRY_1_2_VACANT_LIVE_CHECKPOINT_DIGEST),
    ] {
        let logical_state = registry_1_2_vacant_logical_state(runtime);
        let canonical = canonicalize(&logical_state).expect("canonical vacant logical state");
        assert_eq!(
            canonical.as_slice().len(),
            REGISTRY_1_2_VACANT_RETAINED_BYTES
        );
        if runtime == "LIVE" {
            assert_eq!(canonical.as_slice(), REGISTRY_1_2_VACANT_LIVE_LOGICAL_STATE);
        }
        let checkpoint = registry_1_2_vacant_checkpoint_value(runtime, logical_state);
        assert_eq!(
            canonical_sha256(&domain, &checkpoint)
                .expect("canonical vacant checkpoint")
                .to_hex(),
            expected_digest
        );
    }
}

#[test]
fn runtime_aware_vacant_state_freezes_checkpoint_and_retained_bytes() {
    for (runtime, expected_digest) in [
        (
            RuntimeKind::Fake,
            REGISTRY_1_2_VACANT_FAKE_CHECKPOINT_DIGEST,
        ),
        (
            RuntimeKind::Live,
            REGISTRY_1_2_VACANT_LIVE_CHECKPOINT_DIGEST,
        ),
    ] {
        let vacant = VerifiedRegistryState::vacant(runtime)
            .expect("construct runtime-aware structural vacant Registry");
        let checkpoint = vacant.checkpoint();

        assert_eq!(checkpoint.runtime(), runtime);
        assert_eq!(checkpoint.command_ordinal(), 0);
        assert_eq!(checkpoint.observation_count(), 0);
        assert_eq!(checkpoint.project_count(), 0);
        assert_eq!(checkpoint.command_count(), 0);
        assert_eq!(checkpoint.reservation_count(), 0);
        assert_eq!(
            checkpoint.retained_bytes(),
            REGISTRY_1_2_VACANT_RETAINED_BYTES as u64
        );
        assert_eq!(checkpoint.checkpoint_digest().as_str(), expected_digest);
        assert!(vacant.is_vacant());

        let retained = RegistryCheckpoint::from_retained(
            checkpoint.runtime(),
            checkpoint.command_ordinal(),
            checkpoint.observation_count(),
            checkpoint.project_count(),
            checkpoint.command_count(),
            checkpoint.reservation_count(),
            checkpoint.retained_bytes(),
            checkpoint.checkpoint_digest().clone(),
        );
        assert_eq!(&retained, checkpoint);
    }
}

#[test]
fn vacant_snapshot_self_consistency_is_distinct_from_retained_currentness() {
    let vacant = VerifiedRegistryState::vacant(RuntimeKind::Live)
        .expect("construct structural Live vacant Registry");
    let snapshot = export_untrusted_registry_snapshot(&vacant);

    assert_eq!(
        verify_untrusted_registry_snapshot(&snapshot),
        Ok(vacant.clone()),
        "plain verification proves only snapshot self-consistency"
    );

    let checkpoint = vacant.checkpoint();
    let retained = RegistryCheckpoint::from_retained(
        checkpoint.runtime(),
        checkpoint.command_ordinal(),
        checkpoint.observation_count(),
        checkpoint.project_count(),
        checkpoint.command_count(),
        checkpoint.reservation_count(),
        checkpoint.retained_bytes(),
        checkpoint.checkpoint_digest().clone(),
    );
    assert_eq!(
        verify_untrusted_registry_snapshot_against_checkpoint(&snapshot, &retained),
        Ok(vacant.clone())
    );

    let substituted_checkpoint = RegistryCheckpoint::from_retained(
        checkpoint.runtime(),
        checkpoint.command_ordinal(),
        checkpoint.observation_count(),
        checkpoint.project_count(),
        checkpoint.command_count(),
        checkpoint.reservation_count(),
        checkpoint.retained_bytes(),
        digest('f'),
    );
    assert_eq!(
        verify_untrusted_registry_snapshot_against_checkpoint(&snapshot, &substituted_checkpoint,),
        Err(RegistryError::CheckpointMismatch)
    );
    assert_eq!(
        verify_untrusted_registry_snapshot(&snapshot),
        Ok(vacant),
        "retained-checkpoint substitution does not change snapshot self-consistency"
    );
}

#[test]
fn registry_1_1_literal_golden_vectors() {
    let observation = primary_observation();
    let mut registry = FakeProjectRegistry::new();
    let receipt = registry
        .execute(RegistryCommand::register(
            command("register-1"),
            project("lattice-devos"),
            ProjectClass::LatticeSystem,
            observation.clone(),
        ))
        .expect("golden registration succeeds");
    let authority = receipt.authority().expect("golden authority exists");

    assert_eq!(
        observation.digest().as_str(),
        REGISTRY_1_1_OBSERVATION_DIGEST
    );
    assert_eq!(
        receipt.request_digest().as_str(),
        REGISTRY_1_1_REQUEST_DIGEST
    );
    assert_eq!(
        authority.receipt_digest().as_str(),
        REGISTRY_1_1_AUTHORITY_RECEIPT_DIGEST
    );
    assert_eq!(
        receipt.result_digest().as_str(),
        REGISTRY_1_1_COMMAND_RESULT_DIGEST
    );
}

#[test]
fn registration_issues_deterministic_active_snapshot_and_exact_receipts() {
    let request = RegistryCommand::register(
        command("register-1"),
        project("lattice-devos"),
        ProjectClass::LatticeSystem,
        primary_observation(),
    );
    let mut first_registry = FakeProjectRegistry::new();
    let mut second_registry = FakeProjectRegistry::new();

    let first = first_registry
        .execute(request.clone())
        .expect("register succeeds");
    let second = second_registry.execute(request).expect("register succeeds");

    assert_eq!(first, second);
    assert_eq!(first.outcome(), RegistryCommandOutcome::Applied);
    assert!(first.before().is_none());
    let authority = first.authority().expect("active authority");
    assert_eq!(authority.runtime(), RuntimeKind::Fake);
    assert_eq!(authority.project_id(), &project("lattice-devos"));
    assert_eq!(authority.registry_revision(), 1);
    assert_eq!(authority.lifecycle(), ProjectLifecycle::Active);
    assert_eq!(authority.project_class(), ProjectClass::LatticeSystem);
    assert_eq!(authority.primary_branch().reference(), "refs/heads/main");
    assert_eq!(
        authority.observation_digest(),
        primary_observation().digest()
    );
    assert_eq!(first.after(), Some(&authority.head()));
    assert_eq!(
        first_registry.latest(&project("lattice-devos")),
        Some(authority)
    );
}

#[test]
fn exact_observation_resolve_reuses_snapshot_and_revision() {
    let mut registry = FakeProjectRegistry::new();
    let registered = registry
        .execute(RegistryCommand::register(
            command("register-1"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register");
    let original = registered.authority().expect("authority").clone();

    let resolved = registry
        .execute(RegistryCommand::observe(
            command("observe-1"),
            project("project-1"),
            original.head(),
            primary_observation(),
        ))
        .expect("resolve");

    assert_eq!(resolved.outcome(), RegistryCommandOutcome::Applied);
    assert_eq!(resolved.before(), Some(&original.head()));
    assert_eq!(resolved.after(), Some(&original.head()));
    assert_eq!(resolved.authority(), Some(&original));
}

#[test]
fn command_replay_is_idempotent_and_subject_substitution_is_rejected() {
    let mut registry = FakeProjectRegistry::new();
    let original = RegistryCommand::register(
        command("register-1"),
        project("project-1"),
        ProjectClass::UserProject,
        primary_observation(),
    );

    let first = registry.execute(original.clone()).expect("first result");
    let replay = registry.execute(original).expect("replayed result");
    assert_eq!(replay, first);

    let substituted = RegistryCommand::register(
        command("register-1"),
        project("project-2"),
        ProjectClass::UserProject,
        observation(r"C:\work\other", '5', '6', '7', "refs/heads/main", '8'),
    );
    assert_eq!(
        registry.execute(substituted),
        Err(RegistryError::CommandIdReuse)
    );
}

#[test]
fn duplicate_project_and_physical_identity_matrix_denies_without_mutation() {
    let cases = [
        (
            IdentityDimension::ProjectId,
            project("project-1"),
            observation(r"C:\work\other", '5', '6', '7', "refs/heads/main", '8'),
        ),
        (
            IdentityDimension::CanonicalRoot,
            project("project-2"),
            observation(r"C:\WORK\LATTICE", '1', '6', '7', "refs/heads/main", '8'),
        ),
        (
            IdentityDimension::Repository,
            project("project-2"),
            observation(r"C:\work\repo-alias", '5', '2', '7', "refs/heads/main", '8'),
        ),
        (
            IdentityDimension::File,
            project("project-2"),
            observation(r"C:\work\junction", '5', '6', '3', "refs/heads/main", '8'),
        ),
    ];

    for (index, (dimension, candidate_project, candidate_observation)) in
        cases.into_iter().enumerate()
    {
        let mut registry = FakeProjectRegistry::new();
        let registered = registry
            .execute(RegistryCommand::register(
                command("register-owner"),
                project("project-1"),
                ProjectClass::UserProject,
                primary_observation(),
            ))
            .expect("owner registered");
        let owner_head = registered.authority().expect("authority").head();

        let denied = registry
            .execute(RegistryCommand::register(
                command(&format!("duplicate-{index}")),
                candidate_project,
                ProjectClass::UserProject,
                candidate_observation,
            ))
            .expect("terminal denial receipt");

        assert_eq!(
            denied.outcome(),
            RegistryCommandOutcome::Denied(RegistryDenial::DuplicateIdentity {
                dimension,
                existing_project_id: project("project-1"),
            })
        );
        assert!(denied.authority().is_none());
        assert_eq!(
            registry
                .latest(&project("project-1"))
                .expect("owner remains")
                .head(),
            owner_head
        );
    }
}

#[test]
fn every_identity_drift_rotates_to_reconciliation_required_and_never_active() {
    let cases = [
        (
            IdentityDrift::CanonicalRoot,
            observation(r"D:\moved\lattice", '5', '2', '3', "refs/heads/main", '4'),
        ),
        (
            IdentityDrift::Repository,
            observation(r"C:\work\lattice", '1', '5', '3', "refs/heads/main", '4'),
        ),
        (
            IdentityDrift::File,
            observation(r"C:\work\lattice", '1', '2', '5', "refs/heads/main", '4'),
        ),
        (
            IdentityDrift::PrimaryRefName,
            observation(r"C:\work\lattice", '1', '2', '3', "refs/heads/trunk", '4'),
        ),
        (
            IdentityDrift::PrimaryRefStorage,
            observation(r"C:\work\lattice", '1', '2', '3', "refs/heads/main", '5'),
        ),
    ];

    for (index, (expected_drift, changed)) in cases.into_iter().enumerate() {
        let mut registry = FakeProjectRegistry::new();
        let registered = registry
            .execute(RegistryCommand::register(
                command("register"),
                project("project-1"),
                ProjectClass::UserProject,
                primary_observation(),
            ))
            .expect("register");
        let original = registered.authority().expect("authority").clone();

        let observed = registry
            .execute(RegistryCommand::observe(
                command(&format!("observe-{index}")),
                project("project-1"),
                original.head(),
                changed,
            ))
            .expect("drift receipt");
        let blocked = observed.authority().expect("blocked authority");

        assert_eq!(observed.outcome(), RegistryCommandOutcome::Applied);
        assert_eq!(observed.drift(), &[expected_drift]);
        assert_eq!(blocked.registry_revision(), 2);
        assert_ne!(
            blocked.project_snapshot_id(),
            original.project_snapshot_id()
        );
        assert_eq!(
            blocked.lifecycle(),
            ProjectLifecycle::ReconciliationRequired
        );
        assert!(registry.scope_binding(&project("project-1")).is_none());
    }
}

#[test]
fn suspension_rotates_snapshot_and_stale_heads_cannot_mutate() {
    let mut registry = FakeProjectRegistry::new();
    let registered = registry
        .execute(RegistryCommand::register(
            command("register"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register");
    let original = registered.authority().expect("authority").clone();

    let suspended = registry
        .execute(RegistryCommand::suspend(
            command("suspend"),
            project("project-1"),
            original.head(),
            digest('9'),
        ))
        .expect("suspend");
    let blocked = suspended.authority().expect("suspended authority");

    assert_eq!(suspended.outcome(), RegistryCommandOutcome::Applied);
    assert_eq!(blocked.lifecycle(), ProjectLifecycle::Suspended);
    assert_eq!(blocked.registry_revision(), 2);
    assert_ne!(
        blocked.project_snapshot_id(),
        original.project_snapshot_id()
    );
    assert!(registry.scope_binding(&project("project-1")).is_none());

    let stale = registry
        .execute(RegistryCommand::observe(
            command("stale-observe"),
            project("project-1"),
            original.head(),
            primary_observation(),
        ))
        .expect("terminal denial");
    assert_eq!(
        stale.outcome(),
        RegistryCommandOutcome::Denied(RegistryDenial::StaleHead)
    );
    assert_eq!(stale.authority(), Some(blocked));
}

#[test]
fn move_reconciliation_requires_exact_head_candidate_decision_and_evidence() {
    let moved = observation(r"D:\moved\lattice", '5', '2', '3', "refs/heads/main", '4');
    let mut registry = FakeProjectRegistry::new();
    let registered = registry
        .execute(RegistryCommand::register(
            command("register"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register");
    let original = registered.authority().expect("authority").clone();
    let drifted = registry
        .execute(RegistryCommand::observe(
            command("observe-move"),
            project("project-1"),
            original.head(),
            moved.clone(),
        ))
        .expect("drift");
    let blocked = drifted.authority().expect("blocked").clone();

    let wrong_decision = registry
        .execute(RegistryCommand::reconcile(
            command("wrong-decision"),
            project("project-1"),
            blocked.head(),
            moved.clone(),
            ReconciliationDecision::AcceptIdentityChange,
            digest('a'),
        ))
        .expect("terminal denial");
    assert_eq!(
        wrong_decision.outcome(),
        RegistryCommandOutcome::Denied(RegistryDenial::ReconciliationDecisionMismatch {
            expected: ReconciliationDecision::AcceptMove,
            found: ReconciliationDecision::AcceptIdentityChange,
        })
    );
    assert_eq!(wrong_decision.authority(), Some(&blocked));

    let reconciled = registry
        .execute(RegistryCommand::reconcile(
            command("accept-move"),
            project("project-1"),
            blocked.head(),
            moved.clone(),
            ReconciliationDecision::AcceptMove,
            digest('b'),
        ))
        .expect("reconciled");
    let active = reconciled.authority().expect("active");
    assert_eq!(reconciled.outcome(), RegistryCommandOutcome::Applied);
    assert_eq!(active.lifecycle(), ProjectLifecycle::Active);
    assert_eq!(active.registry_revision(), 3);
    assert_eq!(active.project_class(), ProjectClass::UserProject);
    assert_eq!(active.observation_digest(), moved.digest());
    assert_ne!(active.project_snapshot_id(), blocked.project_snapshot_id());
    assert!(registry.scope_binding(&project("project-1")).is_some());
}

#[test]
fn replacement_reconciliation_requires_identity_decision_and_cross_project_isolation() {
    let replacement = observation(r"C:\work\lattice", '1', '5', '6', "refs/heads/main", '7');
    let mut registry = FakeProjectRegistry::new();
    let first = registry
        .execute(RegistryCommand::register(
            command("register-1"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register first");
    registry
        .execute(RegistryCommand::register(
            command("register-2"),
            project("project-2"),
            ProjectClass::UserProject,
            observation(r"C:\work\other", '8', '9', 'a', "refs/heads/main", 'b'),
        ))
        .expect("register second");
    let drifted = registry
        .execute(RegistryCommand::observe(
            command("observe-replacement"),
            project("project-1"),
            first.authority().expect("authority").head(),
            replacement,
        ))
        .expect("drift");
    let blocked = drifted.authority().expect("blocked").clone();

    let collision = observation(
        r"C:\work\other-alias",
        'c',
        '9',
        'd',
        "refs/heads/main",
        'e',
    );
    let denied = registry
        .execute(RegistryCommand::reconcile(
            command("collision"),
            project("project-1"),
            blocked.head(),
            collision,
            ReconciliationDecision::AcceptIdentityChange,
            digest('f'),
        ))
        .expect("terminal denial");

    assert_eq!(
        denied.outcome(),
        RegistryCommandOutcome::Denied(RegistryDenial::PendingObservationMismatch)
    );
    assert_eq!(denied.authority(), Some(&blocked));
}

#[test]
fn exact_replacement_reconciliation_rotates_back_to_active_without_class_change() {
    let replacement = observation(r"C:\work\lattice", '1', '5', '6', "refs/heads/trunk", '7');
    let mut registry = FakeProjectRegistry::new();
    let registered = registry
        .execute(RegistryCommand::register(
            command("register"),
            project("project-1"),
            ProjectClass::LatticeSystem,
            primary_observation(),
        ))
        .expect("register");
    let drifted = registry
        .execute(RegistryCommand::observe(
            command("observe"),
            project("project-1"),
            registered.authority().expect("authority").head(),
            replacement.clone(),
        ))
        .expect("drift");
    let blocked = drifted.authority().expect("blocked").clone();

    let reconciled = registry
        .execute(RegistryCommand::reconcile(
            command("accept-identity"),
            project("project-1"),
            blocked.head(),
            replacement.clone(),
            ReconciliationDecision::AcceptIdentityChange,
            digest('8'),
        ))
        .expect("reconcile");
    let active = reconciled.authority().expect("active");

    assert_eq!(reconciled.outcome(), RegistryCommandOutcome::Applied);
    assert_eq!(active.lifecycle(), ProjectLifecycle::Active);
    assert_eq!(active.registry_revision(), 3);
    assert_eq!(active.project_class(), ProjectClass::LatticeSystem);
    assert_eq!(active.observation_digest(), replacement.digest());
    assert_eq!(active.primary_branch(), replacement.primary_branch());
    assert_ne!(active.project_snapshot_id(), blocked.project_snapshot_id());
}

#[test]
fn suspended_project_reactivation_requires_exact_observation_and_decision() {
    let mut registry = FakeProjectRegistry::new();
    let registered = registry
        .execute(RegistryCommand::register(
            command("register"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register");
    let suspended = registry
        .execute(RegistryCommand::suspend(
            command("suspend"),
            project("project-1"),
            registered.authority().expect("authority").head(),
            digest('8'),
        ))
        .expect("suspend");
    let blocked = suspended.authority().expect("blocked").clone();

    let substituted = registry
        .execute(RegistryCommand::reconcile(
            command("substituted-observation"),
            project("project-1"),
            blocked.head(),
            observation(r"D:\substituted", '5', '2', '3', "refs/heads/main", '4'),
            ReconciliationDecision::Reactivate,
            digest('9'),
        ))
        .expect("terminal denial");
    assert_eq!(
        substituted.outcome(),
        RegistryCommandOutcome::Denied(RegistryDenial::PendingObservationMismatch)
    );

    let wrong_decision = registry
        .execute(RegistryCommand::reconcile(
            command("wrong-reactivation-decision"),
            project("project-1"),
            blocked.head(),
            primary_observation(),
            ReconciliationDecision::AcceptMove,
            digest('a'),
        ))
        .expect("terminal denial");
    assert_eq!(
        wrong_decision.outcome(),
        RegistryCommandOutcome::Denied(RegistryDenial::ReconciliationDecisionMismatch {
            expected: ReconciliationDecision::Reactivate,
            found: ReconciliationDecision::AcceptMove,
        })
    );

    let reactivated = registry
        .execute(RegistryCommand::reconcile(
            command("reactivate"),
            project("project-1"),
            blocked.head(),
            primary_observation(),
            ReconciliationDecision::Reactivate,
            digest('b'),
        ))
        .expect("reactivate");
    let active = reactivated.authority().expect("active");
    assert_eq!(active.lifecycle(), ProjectLifecycle::Active);
    assert_eq!(active.registry_revision(), 3);
    assert_eq!(active.project_class(), ProjectClass::UserProject);
    assert!(registry.scope_binding(&project("project-1")).is_some());
}

#[test]
fn authoritative_observation_collision_blocks_the_previously_active_project() {
    let mut registry = FakeProjectRegistry::new();
    let first = registry
        .execute(RegistryCommand::register(
            command("register-1"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register first");
    let original = first.authority().expect("authority").clone();
    registry
        .execute(RegistryCommand::register(
            command("register-2"),
            project("project-2"),
            ProjectClass::UserProject,
            observation(r"C:\work\other", '5', '6', '7', "refs/heads/main", '8'),
        ))
        .expect("register second");

    let denied = registry
        .execute(RegistryCommand::observe(
            command("observe-collision"),
            project("project-1"),
            original.head(),
            observation(
                r"C:\work\other-alias",
                '9',
                '6',
                'a',
                "refs/heads/main",
                'b',
            ),
        ))
        .expect("terminal denial");

    assert_eq!(
        denied.outcome(),
        RegistryCommandOutcome::Blocked(RegistryDenial::DuplicateIdentity {
            dimension: IdentityDimension::Repository,
            existing_project_id: project("project-2"),
        })
    );
    assert_eq!(
        denied.drift(),
        &[
            IdentityDrift::CanonicalRoot,
            IdentityDrift::Repository,
            IdentityDrift::File,
            IdentityDrift::PrimaryRefStorage,
        ]
    );
    let blocked = denied.authority().expect("blocked authority");
    assert_eq!(blocked.lifecycle(), ProjectLifecycle::Suspended);
    assert_eq!(blocked.registry_revision(), 2);
    assert_ne!(
        blocked.project_snapshot_id(),
        original.project_snapshot_id()
    );
    assert_eq!(registry.latest(&project("project-1")), Some(blocked));
    assert!(registry.scope_binding(&project("project-1")).is_none());
}

#[test]
fn pending_identity_is_reserved_against_front_running_registration() {
    let replacement = observation(r"D:\replacement", '5', '6', '7', "refs/heads/main", '8');
    let mut registry = FakeProjectRegistry::new();
    let first = registry
        .execute(RegistryCommand::register(
            command("register-1"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register first");
    let drifted = registry
        .execute(RegistryCommand::observe(
            command("observe-replacement"),
            project("project-1"),
            first.authority().expect("authority").head(),
            replacement.clone(),
        ))
        .expect("drift");
    let blocked = drifted.authority().expect("blocked").clone();

    let front_run = registry
        .execute(RegistryCommand::register(
            command("register-2"),
            project("project-2"),
            ProjectClass::UserProject,
            replacement.clone(),
        ))
        .expect("terminal denial");
    assert_eq!(
        front_run.outcome(),
        RegistryCommandOutcome::Denied(RegistryDenial::DuplicateIdentity {
            dimension: IdentityDimension::CanonicalRoot,
            existing_project_id: project("project-1"),
        })
    );
    assert!(front_run.authority().is_none());
    assert!(registry.latest(&project("project-2")).is_none());

    let reconciled = registry
        .execute(RegistryCommand::reconcile(
            command("reconcile"),
            project("project-1"),
            blocked.head(),
            replacement,
            ReconciliationDecision::AcceptIdentityChange,
            digest('9'),
        ))
        .expect("reconcile");
    assert_eq!(reconciled.outcome(), RegistryCommandOutcome::Applied);
    assert_eq!(
        reconciled.authority().expect("active").lifecycle(),
        ProjectLifecycle::Active
    );
}

#[test]
fn observing_another_projects_pending_identity_blocks_the_observer_too() {
    let pending = observation(r"D:\pending", '5', '6', '7', "refs/heads/main", '8');
    let second_identity = observation(r"C:\work\second", '9', 'a', 'b', "refs/heads/main", 'c');
    let mut registry = FakeProjectRegistry::new();
    let first = registry
        .execute(RegistryCommand::register(
            command("register-1"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register first");
    registry
        .execute(RegistryCommand::observe(
            command("observe-pending"),
            project("project-1"),
            first.authority().expect("authority").head(),
            pending,
        ))
        .expect("first project drifts");
    let second = registry
        .execute(RegistryCommand::register(
            command("register-2"),
            project("project-2"),
            ProjectClass::UserProject,
            second_identity.clone(),
        ))
        .expect("register second");

    let denied = registry
        .execute(RegistryCommand::observe(
            command("second-observe-collision"),
            project("project-2"),
            second.authority().expect("authority").head(),
            observation(
                r"C:\work\pending-alias",
                'd',
                '6',
                'e',
                "refs/heads/main",
                'f',
            ),
        ))
        .expect("terminal denial");

    assert_eq!(
        denied.outcome(),
        RegistryCommandOutcome::Blocked(RegistryDenial::DuplicateIdentity {
            dimension: IdentityDimension::Repository,
            existing_project_id: project("project-1"),
        })
    );
    assert_eq!(
        denied.authority().expect("blocked").lifecycle(),
        ProjectLifecycle::Suspended
    );
    assert!(registry.scope_binding(&project("project-2")).is_none());

    let reactivated = registry
        .execute(RegistryCommand::reconcile(
            command("reactivate-second"),
            project("project-2"),
            denied.authority().expect("blocked").head(),
            second_identity,
            ReconciliationDecision::Reactivate,
            digest('1'),
        ))
        .expect("reactivate accepted identity");
    assert_eq!(
        reactivated.authority().expect("active").lifecycle(),
        ProjectLifecycle::Active
    );

    let first_blocked = registry
        .latest(&project("project-1"))
        .expect("first remains pending")
        .clone();
    let first_reconciled = registry
        .execute(RegistryCommand::reconcile(
            command("reconcile-first"),
            project("project-1"),
            first_blocked.head(),
            observation(r"D:\pending", '5', '6', '7', "refs/heads/main", '8'),
            ReconciliationDecision::AcceptIdentityChange,
            digest('2'),
        ))
        .expect("first reservation remains usable");
    assert_eq!(
        first_reconciled.authority().expect("active").lifecycle(),
        ProjectLifecycle::Active
    );
}

#[test]
fn historical_command_replay_stays_immutable_after_the_registry_head_advances() {
    let register = RegistryCommand::register(
        command("register"),
        project("project-1"),
        ProjectClass::UserProject,
        primary_observation(),
    );
    let mut registry = FakeProjectRegistry::new();
    let historical = registry.execute(register.clone()).expect("register");
    let initial = historical.authority().expect("authority").clone();
    registry
        .execute(RegistryCommand::suspend(
            command("suspend"),
            project("project-1"),
            initial.head(),
            digest('9'),
        ))
        .expect("suspend");

    let replay = registry.execute(register).expect("historical replay");

    assert_eq!(replay, historical);
    assert_eq!(replay.authority(), Some(&initial));
    assert_ne!(
        replay.after(),
        registry.current_head(&project("project-1")).as_ref()
    );
    assert_ne!(
        replay.after(),
        Some(
            &registry
                .latest(&project("project-1"))
                .expect("latest")
                .head()
        )
    );
}

#[test]
fn scope_binding_is_the_exact_active_registry_subset() {
    let mut registry = FakeProjectRegistry::new();
    let registered = registry
        .execute(RegistryCommand::register(
            command("register"),
            project("project-1"),
            ProjectClass::UserProject,
            primary_observation(),
        ))
        .expect("register");
    let authority = registered.authority().expect("authority");

    let binding = registry
        .scope_binding(&project("project-1"))
        .expect("active binding");

    assert_eq!(binding.authority_head(), &authority.head());
    assert_eq!(binding.observation_digest(), primary_observation().digest());
    assert_eq!(
        binding.primary_branch(),
        primary_observation().primary_branch()
    );
}

#[test]
fn invalid_command_and_canonical_root_inputs_fail_before_state_exists() {
    assert_eq!(
        CommandId::new(" command"),
        Err(RegistryError::InvalidCommandId)
    );
    assert_eq!(
        RepositoryObservation::new(
            " ",
            digest('1'),
            digest('2'),
            digest('3'),
            GitRefIdentity::new("refs/heads/main", digest('4')).expect("primary ref"),
        ),
        Err(RegistryError::InvalidCanonicalRoot)
    );
    assert_eq!(
        CommandId::new("re\u{301}solve"),
        Err(RegistryError::NonCanonicalText {
            field: "command_id"
        })
    );
    assert_eq!(
        RepositoryObservation::new(
            "C:\\work\\cafe\u{301}",
            digest('1'),
            digest('2'),
            digest('3'),
            GitRefIdentity::new("refs/heads/main", digest('4')).expect("primary ref"),
        ),
        Err(RegistryError::NonCanonicalText {
            field: "canonical_root"
        })
    );
    assert_eq!(
        RepositoryObservation::new(
            r"C:\work\café",
            digest('1'),
            digest('2'),
            digest('3'),
            GitRefIdentity::new("refs/heads/cafe\u{301}", digest('4'))
                .expect("structurally valid primary ref"),
        ),
        Err(RegistryError::NonCanonicalText {
            field: "primary_branch"
        })
    );
}
