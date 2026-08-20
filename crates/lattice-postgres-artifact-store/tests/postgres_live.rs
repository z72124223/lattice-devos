use std::env;
use std::thread;

use lattice_artifact_store::{
    ArtifactRepository, ArtifactRepositoryErrorKind, ArtifactRepositorySnapshot,
    ArtifactStagingIdentity, ArtifactStagingReservation, ArtifactStoreIdentity,
    ArtifactStoreLimits, FakeArtifactStore,
};
use lattice_contracts::{ArtifactObjectKey, ContentDigest, ProjectId, TaskId};
use lattice_postgres_artifact_store::{
    ExtensionTarget, PostgresArtifactStore, SetupErrorKind, install_or_verify,
};
use postgres::{Client, NoTls};
use sha2::{Digest, Sha256};

#[test]
fn exact_artifact_extension_install_and_restart_profile() {
    if env::var("LATTICE_TASK025_ARTIFACT_LIVE").as_deref() != Ok("1") {
        eprintln!("SKIP: LATTICE_TASK025_ARTIFACT_LIVE is not 1");
        return;
    }
    let phase = required("LATTICE_TASK025_ARTIFACT_PHASE");
    let target = target();
    let mut migrator = connect("LATTICE_ARTIFACT_MIGRATOR_URL");
    eprintln!("TASK025_STAGE_ENTER_SETUP");
    setup_or_report(&mut migrator, &target);
    setup_or_report(&mut migrator, &target);
    eprintln!("TASK025_STAGE_PASS_SETUP");

    let store_id = ArtifactStoreIdentity::new("task025-live-store").expect("store id");
    if phase == "initial" {
        initial_matrix(&mut migrator, &target, &store_id);
        println!("TASK025_ARTIFACT_EXTENSION_INITIAL_PASS");
    } else if phase == "restart" {
        let mut repository = runtime_repository(target);
        let loaded = repository
            .load(&store_id)
            .expect("restart load")
            .expect("durable store");
        assert_eq!(
            loaded
                .replay()
                .expect("restart replay")
                .staging_reservation_count(),
            1
        );
        assert_eq!(
            repository.load(&store_id).expect("repeat load"),
            Some(loaded)
        );
        println!("TASK025_ARTIFACT_EXTENSION_RESTART_PASS");
    } else {
        panic!("invalid live phase");
    }
}

#[allow(clippy::too_many_lines)]
fn initial_matrix(
    migrator: &mut Client,
    target: &ExtensionTarget,
    store_id: &ArtifactStoreIdentity,
) {
    let mut repository = runtime_repository(target.clone());
    eprintln!("TASK025_STAGE_ENTER_VACANT_LOAD");
    probe_vacant_load(target, store_id);
    let vacant = repository.load(store_id).unwrap_or_else(|error| {
        report_repository_error(error.kind());
        panic!("vacant load failed");
    });
    assert!(vacant.is_none());

    let empty = FakeArtifactStore::new(store_id.clone(), ArtifactStoreLimits::hard_maximums())
        .expect("empty owner");
    let initial = ArtifactRepositorySnapshot::capture(&empty).expect("initial snapshot");
    let committed = repository
        .compare_and_swap(initial.checkpoint_digest(), &initial)
        .expect("initial commit");
    assert_eq!(committed, initial);
    eprintln!("TASK025_STAGE_PASS_INITIAL_COMMIT");

    let mut staged = empty;
    let staging_identity = staging_identity("main");
    staged
        .reserve_staging(
            "reserve-main",
            ArtifactStagingReservation::new(staging_identity.clone(), 17, 1).expect("reservation"),
        )
        .expect("reserve staging");
    let staged_snapshot = ArtifactRepositorySnapshot::capture(&staged).expect("staged snapshot");
    repository
        .compare_and_swap(initial.checkpoint_digest(), &staged_snapshot)
        .expect("staged commit");
    assert_eq!(
        repository
            .compare_and_swap(initial.checkpoint_digest(), &staged_snapshot)
            .expect("exact retry"),
        staged_snapshot
    );
    eprintln!("TASK025_STAGE_PASS_STAGED_RETRY");

    let mut left = staged.clone();
    left.mark_staging_sealed_orphan("seal-left", &staging_identity)
        .expect("left transition");
    let left = ArtifactRepositorySnapshot::capture(&left).expect("left snapshot");
    let mut right = staged;
    right
        .mark_staging_reconciliation_required("reconcile-right", &staging_identity)
        .expect("right transition");
    let right = ArtifactRepositorySnapshot::capture(&right).expect("right snapshot");
    let expected = staged_snapshot.checkpoint_digest().clone();
    let left_target = target.clone();
    let right_target = target.clone();
    let left_expected = expected.clone();
    let right_expected = expected;
    let left_thread = thread::spawn(move || {
        let mut repository = runtime_repository(left_target);
        repository.compare_and_swap(&left_expected, &left)
    });
    let right_thread = thread::spawn(move || {
        let mut repository = runtime_repository(right_target);
        repository.compare_and_swap(&right_expected, &right)
    });
    let outcomes = [
        left_thread.join().expect("left thread"),
        right_thread.join().expect("right thread"),
    ];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_err_and(|error| error.kind() == ArtifactRepositoryErrorKind::StaleWrite)
            })
            .count(),
        1
    );
    eprintln!("TASK025_STAGE_PASS_CONCURRENCY");
    let winner = repository
        .load(store_id)
        .expect("winner load")
        .expect("winner");
    assert_eq!(
        winner
            .replay()
            .expect("winner replay")
            .staging_reservation_count(),
        1
    );

    let mut direct = connect("LATTICE_ARTIFACT_RUNTIME_URL");
    direct
        .batch_execute("SET ROLE lattice_runtime")
        .expect("direct runtime role");
    let direct_error = direct
        .query(
            "SELECT * FROM artifact_store.artifact_store_load_current_v1($1,$2,$3,$4,$5,$6)",
            &[
                &store_id.as_str(),
                &target.database_name(),
                &target.database_identity_digest().as_str(),
                &target.global_manifest_digest().as_str(),
                &target.memory_manifest_digest().as_str(),
                &lattice_postgres_artifact_store::verify_embedded_extension_manifest()
                    .expect("manifest")
                    .manifest_sha256()
                    .as_str(),
            ],
        )
        .expect_err("read-committed bypass");
    assert_eq!(
        direct_error.code().map(postgres::error::SqlState::code),
        Some("LAS03")
    );
    eprintln!("TASK025_STAGE_PASS_TRANSACTION_GUARD");

    let corrupt_id = ArtifactStoreIdentity::new("task025-corrupt-store").expect("corrupt id");
    let corrupt_owner =
        FakeArtifactStore::new(corrupt_id.clone(), ArtifactStoreLimits::hard_maximums())
            .expect("corrupt owner");
    let corrupt = ArtifactRepositorySnapshot::capture(&corrupt_owner).expect("corrupt snapshot");
    repository
        .compare_and_swap(corrupt.checkpoint_digest(), &corrupt)
        .expect("corrupt baseline");
    migrator
        .batch_execute("SET ROLE lattice_migrator")
        .expect("migrator role");
    migrator
        .execute(
            "UPDATE ONLY artifact_store.artifact_store_head SET snapshot_bytes=\
             pg_catalog.set_byte(snapshot_bytes,0,(pg_catalog.get_byte(snapshot_bytes,0)+1)%256) \
             WHERE store_id=$1",
            &[&corrupt_id.as_str()],
        )
        .expect("physical corruption");
    migrator.batch_execute("RESET ROLE").expect("reset role");
    assert_eq!(
        repository
            .load(&corrupt_id)
            .expect_err("corruption rejected")
            .kind(),
        ArtifactRepositoryErrorKind::Corrupt
    );

    let corrupt_chain_id =
        ArtifactStoreIdentity::new("task025-corrupt-chain").expect("corrupt chain id");
    let corrupt_chain_owner = FakeArtifactStore::new(
        corrupt_chain_id.clone(),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("corrupt chain owner");
    let corrupt_chain =
        ArtifactRepositorySnapshot::capture(&corrupt_chain_owner).expect("corrupt chain snapshot");
    repository
        .compare_and_swap(corrupt_chain.checkpoint_digest(), &corrupt_chain)
        .expect("corrupt chain baseline");
    migrator
        .batch_execute("SET ROLE lattice_migrator")
        .expect("chain corruption role");
    migrator
        .execute(
            "UPDATE ONLY artifact_store.artifact_store_transition SET expected_checkpoint_digest=\
             pg_catalog.set_byte(expected_checkpoint_digest,0,\
             (pg_catalog.get_byte(expected_checkpoint_digest,0)+1)%256) \
             WHERE store_id=$1 AND ordinal=1",
            &[&corrupt_chain_id.as_str()],
        )
        .expect("chain corruption");
    migrator.batch_execute("RESET ROLE").expect("reset role");
    assert_eq!(
        repository
            .load(&corrupt_chain_id)
            .expect_err("chain corruption rejected")
            .kind(),
        ArtifactRepositoryErrorKind::Corrupt
    );
    eprintln!("TASK025_STAGE_PASS_CORRUPTION");

    migrator
        .batch_execute("SET ROLE lattice_migrator")
        .expect("transition query role");
    let transition_count: i64 = migrator
        .query_one(
            "SELECT pg_catalog.count(*) FROM ONLY artifact_store.artifact_store_transition \
             WHERE store_id=$1",
            &[&store_id.as_str()],
        )
        .and_then(|row| row.try_get(0))
        .expect("transition count");
    assert_eq!(transition_count, 3);
    eprintln!("TASK025_STAGE_ENTER_EXACT_PROFILE_DRIFT");
    migrator
        .batch_execute(
            "CREATE TABLE artifact_store.unexpected_profile_drift(id bigint PRIMARY KEY)",
        )
        .expect("create profile drift");
    migrator.batch_execute("RESET ROLE").expect("reset role");
    assert_eq!(
        install_or_verify(migrator, target)
            .expect_err("extra catalog object rejected")
            .kind(),
        SetupErrorKind::ProfileCollision
    );
    eprintln!("TASK025_STAGE_PASS_EXACT_PROFILE_DRIFT_REJECT");
    migrator
        .batch_execute("SET ROLE lattice_migrator")
        .expect("profile cleanup role");
    if let Err(error) = migrator.batch_execute("DROP TABLE artifact_store.unexpected_profile_drift")
    {
        eprintln!(
            "TASK025_PROFILE_CLEANUP_SQLSTATE_{}",
            error.code().map_or("NONE", postgres::error::SqlState::code)
        );
        panic!("remove profile drift fixture");
    }
    migrator.batch_execute("RESET ROLE").expect("reset role");
    eprintln!("TASK025_STAGE_PASS_EXACT_PROFILE_DRIFT_CLEANUP");
    setup_or_report(migrator, target);
    eprintln!("TASK025_STAGE_PASS_EXACT_PROFILE_DRIFT");
}

fn staging_identity(suffix: &str) -> ArtifactStagingIdentity {
    let digest =
        ContentDigest::from_sha256(hex(&Sha256::digest(b"task025-live-object"))).expect("digest");
    ArtifactStagingIdentity::new(
        ArtifactObjectKey::new(ProjectId::new("task025-project").expect("project"), digest),
        TaskId::new("TASK-025").expect("task"),
        format!("stage-{suffix}"),
    )
    .expect("staging identity")
}

fn runtime_repository(target: ExtensionTarget) -> PostgresArtifactStore {
    PostgresArtifactStore::new(connect("LATTICE_ARTIFACT_RUNTIME_URL"), target)
        .expect("runtime adapter")
}

fn target() -> ExtensionTarget {
    ExtensionTarget::new(
        required("LATTICE_ARTIFACT_DATABASE_NAME"),
        digest_env("LATTICE_ARTIFACT_DATABASE_IDENTITY_SHA256"),
        digest_env("LATTICE_ARTIFACT_GLOBAL_MANIFEST_SHA256"),
        digest_env("LATTICE_ARTIFACT_MEMORY_MANIFEST_SHA256"),
    )
    .expect("target")
}

fn connect(name: &str) -> Client {
    Client::connect(&required(name), NoTls).expect("database connection")
}

fn digest_env(name: &str) -> ContentDigest {
    ContentDigest::from_sha256(required(name)).expect("digest environment")
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing {name}"))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn setup_or_report(client: &mut Client, target: &ExtensionTarget) {
    if let Err(error) = install_or_verify(client, target) {
        let token = match error.kind() {
            SetupErrorKind::InvalidTarget => "TASK025_SETUP_ERROR_INVALID_TARGET",
            SetupErrorKind::EmbeddedManifest => "TASK025_SETUP_ERROR_EMBEDDED_MANIFEST",
            SetupErrorKind::Database => "TASK025_SETUP_ERROR_DATABASE",
            SetupErrorKind::FoundationMismatch => "TASK025_SETUP_ERROR_FOUNDATION_MISMATCH",
            SetupErrorKind::InstallSchema => "TASK025_SETUP_ERROR_INSTALL_SCHEMA",
            SetupErrorKind::IdentityRecord => "TASK025_SETUP_ERROR_IDENTITY_RECORD",
            SetupErrorKind::ProfileCollision => "TASK025_SETUP_ERROR_PROFILE_COLLISION",
            SetupErrorKind::SerializationExhausted => "TASK025_SETUP_ERROR_SERIALIZATION_EXHAUSTED",
            SetupErrorKind::CommitOutcomeUnknown => "TASK025_SETUP_ERROR_COMMIT_OUTCOME_UNKNOWN",
        };
        eprintln!("{token}");
        panic!("artifact extension setup failed");
    }
}

fn report_repository_error(kind: ArtifactRepositoryErrorKind) {
    let token = match kind {
        ArtifactRepositoryErrorKind::Domain => "TASK025_REPOSITORY_ERROR_DOMAIN",
        ArtifactRepositoryErrorKind::Unavailable => "TASK025_REPOSITORY_ERROR_UNAVAILABLE",
        ArtifactRepositoryErrorKind::SerializationExhausted => {
            "TASK025_REPOSITORY_ERROR_SERIALIZATION_EXHAUSTED"
        }
        ArtifactRepositoryErrorKind::CommitOutcomeUnknown => {
            "TASK025_REPOSITORY_ERROR_COMMIT_OUTCOME_UNKNOWN"
        }
        ArtifactRepositoryErrorKind::Corrupt => "TASK025_REPOSITORY_ERROR_CORRUPT",
        ArtifactRepositoryErrorKind::StaleWrite => "TASK025_REPOSITORY_ERROR_STALE_WRITE",
        ArtifactRepositoryErrorKind::AuthorityMismatch => {
            "TASK025_REPOSITORY_ERROR_AUTHORITY_MISMATCH"
        }
    };
    eprintln!("{token}");
}

fn probe_vacant_load(target: &ExtensionTarget, store_id: &ArtifactStoreIdentity) {
    let mut client = connect("LATTICE_ARTIFACT_RUNTIME_URL");
    let mut transaction = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .expect("probe transaction");
    transaction
        .batch_execute("SET LOCAL ROLE lattice_runtime")
        .expect("probe role");
    let manifest = lattice_postgres_artifact_store::verify_embedded_extension_manifest()
        .expect("probe manifest");
    let result = transaction.query(
        "SELECT * FROM artifact_store.artifact_store_load_current_v1($1,$2,$3,$4,$5,$6)",
        &[
            &store_id.as_str(),
            &target.database_name(),
            &target.database_identity_digest().as_str(),
            &target.global_manifest_digest().as_str(),
            &target.memory_manifest_digest().as_str(),
            &manifest.manifest_sha256().as_str(),
        ],
    );
    if let Err(error) = result {
        let code = error.code().map_or("NONE", postgres::error::SqlState::code);
        eprintln!("TASK025_PROBE_SQLSTATE_{code}");
        panic!("probe vacant load failed");
    }
    transaction.commit().expect("probe commit");
}
