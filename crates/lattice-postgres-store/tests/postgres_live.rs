use std::env;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, DaemonEpoch, ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind,
    STORE_CONTRACT_VERSION, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId,
    StoreDurability, StoreMutationCommitment, StorePhysicalHead, StoreReceiptDisposition,
    StoreRepositoryOwner, StoreRevision, StoreScope, StoreTransactionId, StoreTransactionRequest,
    TaskId, TaskLedgerStreamIdentity,
};
use lattice_ports::{ControlStore, ControlStoreErrorKind};
use lattice_postgres_store::{
    DatabaseRole, FakePostgresStore, MigrationApplyOutcome, MigrationStatus, MigrationTarget,
    PostgresControlStore, PostgresSchemaEvidence, PostgresStoreSetupError,
    PostgresStoreSetupErrorKind, PostgresTaskLedger, PostgresTaskLedgerErrorKind, apply_migrations,
    migration_manifest, verify_embedded_manifest, verify_postgres_schema,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CommandOutcome, CorrelationId, LedgerDenial,
    LedgerEventKind, LedgerOutcome, ReasonCode, VerifiedStream,
};
use postgres::config::SslMode;
use postgres::error::SqlState;
use postgres::{Client, Config, NoTls};

const REQUIRED_APPLICATION_NAME: &str = "lattice-devos-task019";
const HARNESS_ROLE: &str = "task019_harness";
const LEGACY_V1_MANIFEST_SHA256: &str =
    "9b126a41e542b71d434b5786e35acb66575967d055a6733b9d6bf0b8c9f0eada";
const STORE_V2_MANIFEST_SHA256: &str =
    "4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129";

#[derive(Clone)]
struct LiveConfig {
    host: String,
    port: u16,
    password: String,
    run_id: String,
    phase: String,
}

impl LiveConfig {
    fn from_environment() -> Option<Self> {
        if env::var("LATTICE_TASK019_LIVE").ok().as_deref() != Some("1") {
            return None;
        }
        let host = required_environment("LATTICE_TASK019_HOST");
        let port = required_environment("LATTICE_TASK019_PORT")
            .parse::<u16>()
            .unwrap_or_else(|_| panic!("TASK019_LIVE_PORT_INVALID"));
        let password = required_environment("LATTICE_TASK019_PASSWORD");
        let run_id = required_environment("LATTICE_TASK019_RUN_ID");
        let phase = required_environment("LATTICE_TASK019_PHASE");
        assert!(host == "127.0.0.1", "TASK019_LIVE_HOST_INVALID");
        assert!(port != 0 && port != 5432, "TASK019_LIVE_PORT_INVALID");
        assert!(!password.is_empty(), "TASK019_LIVE_PASSWORD_MISSING");
        assert!(is_lower_hex(&run_id, 32), "TASK019_LIVE_RUN_ID_INVALID");
        assert!(matches!(phase.as_str(), "initial" | "restart"));
        Some(Self {
            host,
            port,
            password,
            run_id,
            phase,
        })
    }

    fn connect(&self, database: &str, application_name: &str) -> Client {
        self.connect_as(database, HARNESS_ROLE, application_name)
    }

    fn connect_as(&self, database: &str, user: &str, application_name: &str) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user(user)
            .password(&self.password)
            .dbname(database)
            .application_name(application_name)
            .ssl_mode(SslMode::Disable);
        config
            .connect(NoTls)
            .unwrap_or_else(|_| panic!("TASK019_LIVE_CONNECT_FAILED"))
    }

    fn role_client(&self, database: &str, role: DatabaseRole, application_name: &str) -> Client {
        let mut client = self.connect_as(database, role.login_role(), application_name);
        client
            .batch_execute(set_role_sql(role))
            .unwrap_or_else(|_| panic!("TASK019_SET_ROLE_FAILED"));
        client
    }

    fn database_name(&self, tag: &str) -> String {
        assert!(
            !tag.is_empty()
                && tag.len() <= 15
                && tag
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
            "TASK019_DATABASE_TAG_INVALID"
        );
        format!("lattice_task019_{}_{}", &self.run_id[..8], tag)
    }

    fn target(&self, tag: &str) -> MigrationTarget {
        MigrationTarget::new(self.database_name(tag), self.run_id.clone())
            .unwrap_or_else(|_| panic!("TASK019_TARGET_CONSTRUCTION_FAILED"))
    }
}

#[test]
fn marker_owned_postgres_17_foundation() {
    let Some(config) = LiveConfig::from_environment() else {
        return;
    };
    if config.phase == "initial" {
        run_initial_phase(&config);
    } else {
        run_restart_phase(&config);
    }
}

fn run_initial_phase(config: &LiveConfig) {
    let mut admin = config.connect("postgres", "lattice-devos-task019-admin");
    create_fixed_roles(&mut admin, &config.password);

    let (base, evidence) = prove_first_apply_and_reconciliation(config, &mut admin);
    prove_exact_v1_upgrade(config, &mut admin);
    prove_concurrent_v1_upgrade(config, &mut admin);
    prove_v1_upgrade_rejection_matrix(config, &mut admin);
    prove_v1_upgrade_transaction_rollback(config, &mut admin);
    prove_exact_nonempty_v2_upgrade_and_replay(config, &mut admin);
    prove_commit_response_loss_reconciliation(config, &mut admin);
    prove_post_apply_verification_failure(config, &mut admin);
    prove_concurrent_runners(config, &mut admin);
    prove_transaction_rollback(config, &mut admin);
    prove_preflight_denials(config, &mut admin);
    prove_login_requires_set_role(config, &mut admin, &base);
    prove_catalog_and_permission_denials(config, &mut admin, &base);
    prove_nonwriter_denials(config, &mut admin, &base);
    prove_live_commit_response_loss_reconciliation(config, &mut admin);
    prove_live_serialization_retry_bound(config, &mut admin);
    prove_live_revision_overflow(config, &mut admin);
    prove_live_retained_corruption(config, &mut admin);
    println!("STORE_TASK021_STAGE_01_CONCURRENCY");
    prove_live_task_ledger_concurrency(config, &mut admin);
    println!("STORE_TASK021_STAGE_02_ATOMIC_ROLLBACK");
    prove_live_task_ledger_atomic_rollback(config, &mut admin);
    println!("STORE_TASK021_STAGE_03_SERIALIZATION_BOUND");
    prove_live_task_ledger_serialization_retry_bound(config, &mut admin);
    println!("STORE_TASK021_STAGE_04_COMMIT_RESPONSE_LOSS");
    prove_live_task_ledger_commit_response_loss(config, &mut admin);
    println!("STORE_TASK021_STAGE_05_MANIFEST_DRIFT");
    prove_live_task_ledger_manifest_drift(config, &mut admin);
    println!("STORE_TASK021_STAGE_06_LOCK_TIMEOUT");
    prove_live_task_ledger_lock_timeout(config, &mut admin);
    println!("STORE_TASK021_STAGE_07_RETAINED_CORRUPTION");
    prove_live_task_ledger_corruption(config, &mut admin);
    set_exact_database_access(&mut admin, base.database_name());
    prove_live_control_store(config, &base);
    println!("STORE_TASK021_STAGE_08_BASE_LEDGER");
    prove_live_task_ledger(config, &base);
    println!("STORE_TASK021_STAGE_09_XMIN_PROVENANCE");
    prove_task021_transaction_provenance_primitive(config, &base);

    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        evidence.database_uuid(),
        evidence.manifest_sha256().as_str()
    );
}

fn run_restart_phase(config: &LiveConfig) {
    let expected_uuid = required_environment("LATTICE_TASK019_EXPECTED_UUID");
    let expected_manifest = required_environment("LATTICE_TASK019_EXPECTED_MANIFEST");
    assert!(
        is_canonical_uuid(&expected_uuid),
        "TASK019_EXPECTED_UUID_INVALID"
    );
    assert!(
        is_lower_hex(&expected_manifest, 64),
        "TASK019_EXPECTED_MANIFEST_INVALID"
    );
    let target = config.target("base");
    for role in DatabaseRole::ALL {
        println!(
            "{}",
            match role {
                DatabaseRole::Migrator => "STORE_TASK021_RESTART_VERIFY_01_MIGRATOR",
                DatabaseRole::Runtime => "STORE_TASK021_RESTART_VERIFY_02_RUNTIME",
                DatabaseRole::Guardian => "STORE_TASK021_RESTART_VERIFY_03_GUARDIAN",
                DatabaseRole::ReadOnly => "STORE_TASK021_RESTART_VERIFY_04_READONLY",
            }
        );
        let mut client =
            config.role_client(target.database_name(), role, REQUIRED_APPLICATION_NAME);
        let evidence = must_setup(verify_postgres_schema(&mut client, &target, role));
        assert_eq!(evidence.database_uuid(), expected_uuid);
        assert_eq!(evidence.manifest_sha256().as_str(), expected_manifest);
    }
    prove_live_control_store_restart(config, &target);
    println!("STORE_TASK021_RESTART_STAGE_01_EXACT_REPLAY");
    prove_live_task_ledger_restart(config, &target);
    println!("TASK019_RESTART_OK");
}

#[allow(clippy::too_many_lines)]
fn prove_live_control_store(config: &LiveConfig, target: &MigrationTarget) {
    set_live_admission(config, target, true);
    let authority = live_authority('a', 'b');
    let scope = live_scope("project-live", "snapshot-live", '1');
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut store = PostgresControlStore::new(runtime, target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let genesis = store
        .current_head(&scope)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(genesis.runtime(), RuntimeKind::Live);
    assert_eq!(genesis.revision().get(), 0);

    let applied_request = live_request(
        "live-transaction-1",
        scope.clone(),
        authority.clone(),
        genesis.clone(),
        live_mutation(0),
    );
    let applied = store
        .transact(applied_request.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(applied.disposition(), StoreReceiptDisposition::Applied);
    assert_eq!(applied.runtime(), RuntimeKind::Live);
    assert_eq!(applied.durability(), StoreDurability::DurablePostgres);
    assert_eq!(applied.before_head(), &genesis);
    assert_eq!(applied.after_head().revision().get(), 1);
    let persistence = applied.persistence().expect("live persistence evidence");
    let expected_identity =
        ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
            .expect("target identity digest");
    assert_eq!(persistence.database_identity_digest(), &expected_identity);
    assert_eq!(persistence.schema_version(), 2);
    assert_eq!(
        persistence.manifest_digest().as_str(),
        STORE_V2_MANIFEST_SHA256
    );
    assert_eq!(
        store
            .current_head(&scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *applied.after_head()
    );
    assert_eq!(
        store
            .transact(applied_request.clone())
            .unwrap_or_else(|error| panic!("{}", error.code())),
        applied
    );

    let stale_request = live_request(
        "live-transaction-stale",
        scope.clone(),
        authority.clone(),
        genesis,
        live_mutation(1),
    );
    let stale = store
        .transact(stale_request.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        stale.disposition(),
        StoreReceiptDisposition::StalePhysicalHead
    );
    assert_eq!(stale.before_head(), applied.after_head());
    assert_eq!(stale.after_head(), applied.after_head());
    assert_eq!(
        store
            .transact(stale_request)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        stale
    );

    let changed = live_request(
        "live-transaction-1",
        scope.clone(),
        authority.clone(),
        applied.before_head().clone(),
        live_mutation(2),
    );
    expect_store_kind(
        store.transact(changed.clone()),
        ControlStoreErrorKind::CommandSubstitution,
    );
    let changed_admission = live_request(
        "live-transaction-1",
        scope.clone(),
        live_authority_with_admission(RuntimeAdmissionMode::Stopped, 'a', 'b'),
        applied.before_head().clone(),
        live_mutation(0),
    );
    expect_store_kind(
        store.transact(changed_admission.clone()),
        ControlStoreErrorKind::CommandSubstitution,
    );
    let substituted_head = StorePhysicalHead::new(
        RuntimeKind::Live,
        scope.clone(),
        applied.before_head().revision(),
        applied.before_head().state_digest().clone(),
        live_digest('f'),
    )
    .expect("structurally valid substituted head");
    let changed_head = live_request(
        "live-transaction-1",
        scope.clone(),
        authority.clone(),
        substituted_head.clone(),
        live_mutation(0),
    );
    expect_store_kind(
        store.transact(changed_head),
        ControlStoreErrorKind::CorruptState,
    );
    let invalid_new_head = live_request(
        "live-invalid-new-head",
        scope.clone(),
        authority.clone(),
        substituted_head,
        live_mutation(3),
    );
    expect_store_kind(
        store.transact(invalid_new_head),
        ControlStoreErrorKind::CorruptState,
    );

    let fake_authority = StoreAuthorityHead::new(
        RuntimeKind::Fake,
        StoreDaemonInstanceId::new("daemon-live-1").expect("valid daemon"),
        DaemonEpoch::new(7).expect("valid epoch"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(3).expect("valid authority revision"),
        live_digest('a'),
        live_digest('b'),
    )
    .expect("valid fake authority");
    let mut fake = FakePostgresStore::new(fake_authority.clone(), 2).expect("fake fixture");
    let fake_head = fake.current_head(&scope).expect("fake genesis");
    let changed_version_runtime = StoreTransactionRequest::new(
        1,
        StoreTransactionId::new("live-transaction-1").expect("valid transaction id"),
        scope.clone(),
        fake_authority.clone(),
        fake_head.clone(),
        live_mutation(0),
    )
    .expect("valid v1 fake request");
    expect_store_kind(
        store.transact(changed_version_runtime),
        ControlStoreErrorKind::UnsupportedVersion,
    );
    let unsupported_new = StoreTransactionRequest::new(
        1,
        StoreTransactionId::new("live-unsupported-new").expect("valid transaction id"),
        scope.clone(),
        fake_authority,
        fake_head,
        live_mutation(3),
    )
    .expect("valid v1 fake request");
    expect_store_kind(
        store.transact(unsupported_new),
        ControlStoreErrorKind::UnsupportedVersion,
    );
    let claimed_stopped = live_request(
        "live-claimed-stopped",
        scope.clone(),
        live_authority_with_admission(RuntimeAdmissionMode::Stopped, 'a', 'b'),
        applied.after_head().clone(),
        live_mutation(3),
    );
    expect_store_kind(
        store.transact(claimed_stopped),
        ControlStoreErrorKind::AdmissionDenied,
    );

    let authority_mismatch = live_request(
        "live-authority-mismatch",
        scope.clone(),
        live_authority('c', 'b'),
        applied.after_head().clone(),
        live_mutation(3),
    );
    expect_store_kind(
        store.transact(authority_mismatch),
        ControlStoreErrorKind::AuthorityMismatch,
    );

    prove_live_store_concurrency(config, target, &mut store, authority.clone());

    set_live_admission(config, target, false);
    expect_store_kind(
        store.transact(changed),
        ControlStoreErrorKind::CommandSubstitution,
    );
    expect_store_kind(
        store.transact(changed_admission),
        ControlStoreErrorKind::CommandSubstitution,
    );
    let denied = live_request(
        "live-admission-denied",
        scope.clone(),
        authority,
        applied.after_head().clone(),
        live_mutation(4),
    );
    expect_store_kind(
        store.transact(denied),
        ControlStoreErrorKind::AdmissionDenied,
    );
    assert_eq!(
        store
            .transact(applied_request)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        applied
    );
    assert_eq!(
        store
            .current_head(&scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *applied.after_head()
    );
    drop(store);

    let mut admin = config.connect(target.database_name(), "lattice-devos-task020-live-counts");
    let counts = admin
        .query_one(
            "SELECT (SELECT count(*) FROM ONLY control.physical_heads \
                     WHERE project_id = $1::text AND project_snapshot_id = $2::text), \
                    (SELECT count(*) FROM ONLY control.terminal_transactions \
                     WHERE project_id = $1::text AND project_snapshot_id = $2::text)",
            &[
                &scope.project_id().as_str(),
                &scope.project_snapshot_id().as_str(),
            ],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_LIVE_COUNT_FAILED"));
    assert_eq!(counts.get::<_, i64>(0), 1);
    assert_eq!(counts.get::<_, i64>(1), 2);
}

#[allow(clippy::too_many_lines)]
fn prove_live_commit_response_loss_reconciliation(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "live_lost_ack");
    set_live_admission(config, &target, true);
    let scope = live_scope("project-live-lost", "snapshot-live-lost", '6');
    let authority = live_authority('a', 'b');
    let direct_runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut observer = PostgresControlStore::new(direct_runtime, &target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let genesis = observer
        .current_head(&scope)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    drop(observer);
    let request = live_request(
        "live-lost-response",
        scope.clone(),
        authority,
        genesis,
        live_mutation(10),
    );

    let proxy = CommitResponseDropProxy::start_at_commit(&config.host, config.port, 2);
    let mut proxied_config = config.clone();
    proxied_config.port = proxy.port();
    let proxied_runtime = proxied_config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut uncertain = PostgresControlStore::new(proxied_runtime, &target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let unknown = uncertain
        .transact(request.clone())
        .expect_err("lost commit response must not return a receipt");
    assert_eq!(unknown.kind(), ControlStoreErrorKind::CommitOutcomeUnknown);
    assert_eq!(unknown.code(), "STORE_COMMIT_OUTCOME_UNKNOWN");
    let poisoned_transaction = uncertain
        .transact(request.clone())
        .expect_err("poisoned Store must require reconciliation");
    assert_eq!(
        poisoned_transaction.kind(),
        ControlStoreErrorKind::CommitOutcomeUnknown
    );
    assert_eq!(
        poisoned_transaction.code(),
        "STORE_LIVE_RECONCILIATION_REQUIRED"
    );
    let poisoned_read = uncertain
        .current_head(&scope)
        .expect_err("poisoned Store must reject reads");
    assert_eq!(
        poisoned_read.kind(),
        ControlStoreErrorKind::CommitOutcomeUnknown
    );
    assert_eq!(poisoned_read.code(), "STORE_LIVE_RECONCILIATION_REQUIRED");
    drop(uncertain);
    assert!(proxy.finish(), "STORE_TASK020_LIVE_COMMIT_ACK_NOT_DROPPED");

    let mut retained = config.connect(target.database_name(), "task020-live-lost-proof");
    let row = retained
        .query_one(
            "SELECT transaction_digest, receipt_digest, \
               (SELECT count(*) FROM ONLY control.physical_heads \
                WHERE project_id = $2::text AND project_snapshot_id = $3::text), \
               (SELECT count(*) FROM ONLY control.terminal_transactions \
                WHERE project_id = $2::text AND project_snapshot_id = $3::text) \
             FROM ONLY control.terminal_transactions \
             WHERE transaction_id = $1::text",
            &[
                &request.transaction_id().as_str(),
                &scope.project_id().as_str(),
                &scope.project_snapshot_id().as_str(),
            ],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_LIVE_COMMIT_RETAINED_MISSING"));
    let retained_transaction = live_digest_from_bytes(&row.get::<_, Vec<u8>>(0));
    let retained_receipt = live_digest_from_bytes(&row.get::<_, Vec<u8>>(1));
    assert_eq!(row.get::<_, i64>(2), 1);
    assert_eq!(row.get::<_, i64>(3), 1);
    drop(retained);

    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut fresh_store = PostgresControlStore::new(runtime, &target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let replayed_receipt = fresh_store
        .transact(request.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(replayed_receipt.transaction_digest(), &retained_transaction);
    assert_eq!(replayed_receipt.receipt_digest(), &retained_receipt);
    assert_eq!(
        fresh_store
            .transact(request)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        replayed_receipt
    );
    assert_eq!(
        fresh_store
            .current_head(&scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *replayed_receipt.after_head()
    );
}

#[allow(clippy::too_many_lines)]
fn prove_live_serialization_retry_bound(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "live_retry");
    set_live_admission(config, &target, true);
    let scope = live_scope("project-live-retry", "snapshot-live-retry", '7');
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut store = PostgresControlStore::new(runtime, &target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let genesis = store
        .current_head(&scope)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let request = live_request(
        "live-retry-bound",
        scope.clone(),
        live_authority('a', 'b'),
        genesis.clone(),
        live_mutation(11),
    );

    let mut fixture = config.connect(target.database_name(), "task020-live-retry-fixture");
    fixture
        .batch_execute(
            "CREATE SEQUENCE public.task020_retry_counter; \
             REVOKE ALL ON SEQUENCE public.task020_retry_counter FROM PUBLIC, \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE FUNCTION public.task020_force_serialization() RETURNS trigger \
             LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$ \
             BEGIN \
               PERFORM pg_catalog.nextval('public.task020_retry_counter'::pg_catalog.regclass); \
               RAISE EXCEPTION USING ERRCODE = '40001', MESSAGE = 'task020 retry fixture'; \
             END $$; \
             REVOKE ALL ON FUNCTION public.task020_force_serialization() FROM PUBLIC, \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE TRIGGER task020_force_serialization \
             BEFORE INSERT ON control.terminal_transactions FOR EACH ROW \
             EXECUTE FUNCTION public.task020_force_serialization()",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_RETRY_FIXTURE_FAILED"));
    let exhausted = store
        .transact(request.clone())
        .expect_err("four serialization failures must exhaust the retry bound");
    assert_eq!(
        exhausted.kind(),
        ControlStoreErrorKind::SerializationExhausted
    );
    assert_eq!(exhausted.code(), "STORE_SERIALIZATION_RETRIES_EXHAUSTED");
    let proof = fixture
        .query_one(
            "SELECT (SELECT last_value FROM public.task020_retry_counter), \
                    (SELECT count(*) FROM ONLY control.physical_heads \
                     WHERE project_id = $1::text AND project_snapshot_id = $2::text), \
                    (SELECT count(*) FROM ONLY control.terminal_transactions \
                     WHERE project_id = $1::text AND project_snapshot_id = $2::text)",
            &[
                &scope.project_id().as_str(),
                &scope.project_snapshot_id().as_str(),
            ],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_RETRY_PROOF_FAILED"));
    assert_eq!(proof.get::<_, i64>(0), 4);
    assert_eq!(proof.get::<_, i64>(1), 0);
    assert_eq!(proof.get::<_, i64>(2), 0);
    assert_eq!(
        store
            .current_head(&scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        genesis
    );
    fixture
        .batch_execute(
            "DROP TRIGGER task020_force_serialization ON control.terminal_transactions; \
             DROP FUNCTION public.task020_force_serialization(); \
             DROP SEQUENCE public.task020_retry_counter",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_RETRY_FIXTURE_CLEANUP_FAILED"));
    assert_eq!(
        store
            .transact(request)
            .unwrap_or_else(|error| panic!("{}", error.code()))
            .disposition(),
        StoreReceiptDisposition::Applied
    );
}

fn prove_live_revision_overflow(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "live_overflow");
    set_live_admission(config, &target, true);
    let scope = live_scope("project-live-overflow", "snapshot-live-overflow", '8');
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut store = PostgresControlStore::new(runtime, &target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let max_head = fixture_live_head(scope.clone(), i64::MAX as u64, 'c');
    let mut fixture = config.connect(target.database_name(), "task020-live-overflow-fixture");
    fixture
        .execute(
            "INSERT INTO control.physical_heads (\
                 project_id, project_snapshot_id, repository_owner, aggregate_key_digest, \
                 physical_revision, state_digest, head_digest\
             ) VALUES ($1::text, $2::text, $3::text, $4::bytea, $5::bigint, $6::bytea, $7::bytea)",
            &[
                &scope.project_id().as_str(),
                &scope.project_snapshot_id().as_str(),
                &scope.owner().as_str(),
                &live_digest_value_bytes(scope.aggregate_key_digest()),
                &i64::MAX,
                &live_digest_value_bytes(max_head.state_digest()),
                &live_digest_value_bytes(max_head.head_digest()),
            ],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_OVERFLOW_FIXTURE_FAILED"));
    assert_eq!(
        store
            .current_head(&scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        max_head
    );
    let request = live_request(
        "live-revision-overflow",
        scope.clone(),
        live_authority('a', 'b'),
        max_head.clone(),
        live_mutation(12),
    );
    let overflow = store
        .transact(request)
        .expect_err("signed bigint maximum must not advance");
    assert_eq!(overflow.kind(), ControlStoreErrorKind::RevisionOverflow);
    assert_eq!(overflow.code(), "STORE_REVISION_OVERFLOW");
    let proof = fixture
        .query_one(
            "SELECT (SELECT physical_revision FROM ONLY control.physical_heads \
                     WHERE project_id = $1::text AND project_snapshot_id = $2::text), \
                    (SELECT count(*) FROM ONLY control.terminal_transactions \
                     WHERE project_id = $1::text AND project_snapshot_id = $2::text)",
            &[
                &scope.project_id().as_str(),
                &scope.project_snapshot_id().as_str(),
            ],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_OVERFLOW_PROOF_FAILED"));
    assert_eq!(proof.get::<_, i64>(0), i64::MAX);
    assert_eq!(proof.get::<_, i64>(1), 0);
}

#[allow(clippy::too_many_lines)]
fn prove_live_retained_corruption(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "live_corrupt");
    set_live_admission(config, &target, true);
    let scope = live_scope("project-live-corrupt", "snapshot-live-corrupt", '9');
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut store = PostgresControlStore::new(runtime, &target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let genesis = store
        .current_head(&scope)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let request = live_request(
        "live-retained-corrupt",
        scope.clone(),
        live_authority('a', 'b'),
        genesis,
        live_mutation(13),
    );
    let applied = store
        .transact(request.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let mut fixture = config.connect(target.database_name(), "task020-live-corrupt-fixture");
    assert_sqlstate(
        fixture.execute(
            "UPDATE ONLY control.terminal_transactions SET receipt_digest = NULL \
             WHERE transaction_id = $1::text",
            &[&request.transaction_id().as_str()],
        ),
        &SqlState::NOT_NULL_VIOLATION,
        "STORE_TASK020_RECEIPT_NULL_ACCEPTED",
    );

    fixture
        .batch_execute(
            "ALTER TABLE ONLY control.terminal_transactions \
             ALTER COLUMN producer_id DROP NOT NULL",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_PRODUCER_NULL_FIXTURE_FAILED"));
    fixture
        .execute(
            "UPDATE ONLY control.terminal_transactions SET producer_id = NULL \
             WHERE transaction_id = $1::text",
            &[&request.transaction_id().as_str()],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_PRODUCER_NULL_FIXTURE_FAILED"));
    let null_corruption = store
        .transact(request.clone())
        .expect_err("NULL retained producer must fail closed");
    assert_eq!(null_corruption.kind(), ControlStoreErrorKind::CorruptState);
    assert_eq!(null_corruption.code(), "STORE_LIVE_STATE_CORRUPT");
    fixture
        .execute(
            "UPDATE ONLY control.terminal_transactions \
             SET producer_id = 'lattice-postgres-store' WHERE transaction_id = $1::text",
            &[&request.transaction_id().as_str()],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_PRODUCER_NULL_RESTORE_FAILED"));
    fixture
        .batch_execute(
            "ALTER TABLE ONLY control.terminal_transactions \
             ALTER COLUMN producer_id SET NOT NULL",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_PRODUCER_NULL_RESTORE_FAILED"));

    fixture
        .execute(
            "UPDATE ONLY control.terminal_transactions \
             SET receipt_digest = decode(repeat('ee', 32), 'hex') \
             WHERE transaction_id = $1::text",
            &[&request.transaction_id().as_str()],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_RECEIPT_CORRUPTION_FAILED"));
    let digest_corruption = store
        .transact(request)
        .expect_err("substituted retained digest must fail closed");
    assert_eq!(
        digest_corruption.kind(),
        ControlStoreErrorKind::CorruptState
    );
    assert_eq!(digest_corruption.code(), "STORE_REPLAY_DIGEST_CORRUPT");
    assert_eq!(
        store
            .current_head(&scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *applied.after_head()
    );
}

#[allow(clippy::similar_names, clippy::too_many_lines)]
fn prove_live_store_concurrency(
    config: &LiveConfig,
    target: &MigrationTarget,
    observer: &mut PostgresControlStore,
    authority: StoreAuthorityHead,
) {
    let same_id_scope = live_scope("project-con-sameid", "snapshot-con-sameid", '2');
    let same_id_genesis = observer
        .current_head(&same_id_scope)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let same_id_request = live_request(
        "live-concurrent-same-id",
        same_id_scope.clone(),
        authority.clone(),
        same_id_genesis,
        live_mutation(5),
    );
    let same_id =
        run_concurrent_live_transactions(config, target, same_id_request.clone(), same_id_request);
    assert_eq!(same_id.0, same_id.1);
    assert_eq!(same_id.0.disposition(), StoreReceiptDisposition::Applied);
    assert_eq!(same_id.0.after_head().revision().get(), 1);
    assert_eq!(
        observer
            .current_head(&same_id_scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *same_id.0.after_head()
    );

    let same_scope = live_scope("project-con-scope", "snapshot-con-scope", '3');
    let same_scope_genesis = observer
        .current_head(&same_scope)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let different_ids = run_concurrent_live_transactions(
        config,
        target,
        live_request(
            "live-concurrent-scope-a",
            same_scope.clone(),
            authority.clone(),
            same_scope_genesis.clone(),
            live_mutation(6),
        ),
        live_request(
            "live-concurrent-scope-b",
            same_scope.clone(),
            authority,
            same_scope_genesis,
            live_mutation(7),
        ),
    );
    let (applied, stale) = match (different_ids.0.disposition(), different_ids.1.disposition()) {
        (StoreReceiptDisposition::Applied, StoreReceiptDisposition::StalePhysicalHead) => {
            (&different_ids.0, &different_ids.1)
        }
        (StoreReceiptDisposition::StalePhysicalHead, StoreReceiptDisposition::Applied) => {
            (&different_ids.1, &different_ids.0)
        }
        _ => panic!("STORE_TASK020_CONCURRENT_SCOPE_DISPOSITION_INVALID"),
    };
    assert_eq!(applied.after_head().revision().get(), 1);
    assert_eq!(stale.before_head(), applied.after_head());
    assert_eq!(stale.after_head(), applied.after_head());
    assert_eq!(
        observer
            .current_head(&same_scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *applied.after_head()
    );

    let cross_a = live_scope("project-con-cross-a", "snapshot-con-cross-a", '4');
    let cross_b = live_scope("project-con-cross-b", "snapshot-con-cross-b", '5');
    let cross_a_genesis = observer
        .current_head(&cross_a)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let cross_b_genesis = observer
        .current_head(&cross_b)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let cross = run_concurrent_live_transactions(
        config,
        target,
        live_request(
            "live-concurrent-cross-a",
            cross_a.clone(),
            live_authority('a', 'b'),
            cross_a_genesis,
            live_mutation(8),
        ),
        live_request(
            "live-concurrent-cross-b",
            cross_b.clone(),
            live_authority('a', 'b'),
            cross_b_genesis,
            live_mutation(9),
        ),
    );
    assert_eq!(cross.0.disposition(), StoreReceiptDisposition::Applied);
    assert_eq!(cross.1.disposition(), StoreReceiptDisposition::Applied);
    assert_eq!(
        observer
            .current_head(&cross_a)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *cross.0.after_head()
    );
    assert_eq!(
        observer
            .current_head(&cross_b)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *cross.1.after_head()
    );
}

fn run_concurrent_live_transactions(
    config: &LiveConfig,
    target: &MigrationTarget,
    first: StoreTransactionRequest,
    second: StoreTransactionRequest,
) -> (
    lattice_contracts::StoreTransactionReceipt,
    lattice_contracts::StoreTransactionReceipt,
) {
    let barrier = Arc::new(Barrier::new(3));
    let runtime_a = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let runtime_b = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let store_a = PostgresControlStore::new(runtime_a, target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let store_b = PostgresControlStore::new(runtime_b, target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let spawn = |mut store: PostgresControlStore, request: StoreTransactionRequest| {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            store.transact(request)
        })
    };
    let first_handle = spawn(store_a, first);
    let second_handle = spawn(store_b, second);
    barrier.wait();
    let first_receipt = first_handle
        .join()
        .expect("first live transaction thread")
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let second_receipt = second_handle
        .join()
        .expect("second live transaction thread")
        .unwrap_or_else(|error| panic!("{}", error.code()));
    (first_receipt, second_receipt)
}

fn prove_live_control_store_restart(config: &LiveConfig, target: &MigrationTarget) {
    let scope = live_scope("project-live", "snapshot-live", '1');
    let expected_head = read_retained_expected_head(config, target, &scope, "live-transaction-1");
    let request = live_request(
        "live-transaction-1",
        scope.clone(),
        live_authority('a', 'b'),
        expected_head,
        live_mutation(0),
    );
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut store = PostgresControlStore::new(runtime, target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let replay = store
        .transact(request)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(replay.disposition(), StoreReceiptDisposition::Applied);
    assert_eq!(replay.runtime(), RuntimeKind::Live);
    assert_eq!(replay.durability(), StoreDurability::DurablePostgres);
    assert_eq!(
        store
            .current_head(&scope)
            .unwrap_or_else(|error| panic!("{}", error.code())),
        *replay.after_head()
    );
}

#[allow(clippy::too_many_lines)]
fn prove_live_task_ledger(config: &LiveConfig, target: &MigrationTarget) {
    set_live_admission(config, target, true);
    let identity = live_task_identity("ledger-main", "TASK-021");
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut ledger =
        PostgresTaskLedger::new(runtime, target).unwrap_or_else(|error| panic!("{}", error.code()));

    println!("STORE_TASK021_BASE_STAGE_01_LOAD_VACANT");
    let vacant = ledger
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        vacant.stream().head().is_zero(),
        "STORE_TASK021_BASE_VACANT_HEAD_INVALID"
    );
    assert!(
        vacant.stream().events().is_empty(),
        "STORE_TASK021_BASE_VACANT_EVENTS_PRESENT"
    );
    assert!(
        vacant.stream().commands().is_empty(),
        "STORE_TASK021_BASE_VACANT_COMMANDS_PRESENT"
    );
    assert!(
        vacant.stream().outboxes().is_empty(),
        "STORE_TASK021_BASE_VACANT_OUTBOX_PRESENT"
    );
    assert_eq!(
        vacant.stream().checkpoint(),
        vacant.retained_checkpoint(),
        "STORE_TASK021_BASE_VACANT_CHECKPOINT_MISMATCH"
    );
    assert_eq!(
        vacant.physical_head().revision().get(),
        0,
        "STORE_TASK021_BASE_VACANT_REVISION_INVALID"
    );
    // A vacant stream has no retained Ledger/Store pair yet. The adapter has
    // already proved this is the deterministic Store genesis for the scope;
    // checkpoint equality begins with the first atomic Ledger mutation.
    assert_global_task_ledger_persistence(vacant.persistence());

    let zero_head = vacant.stream().head().clone();
    let first_command = live_task_command(
        zero_head.clone(),
        "ledger-main-command-1",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'b',
    );
    println!("STORE_TASK021_BASE_STAGE_02_FIRST_EXECUTE");
    let first = ledger
        .execute(first_command.clone(), live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        !first.is_exact_retry(),
        "STORE_TASK021_BASE_FIRST_FALSE_RETRY"
    );
    assert_eq!(
        first.receipt().outcome(),
        &CommandOutcome::Appended,
        "STORE_TASK021_BASE_FIRST_NOT_APPENDED"
    );
    assert_eq!(
        first.receipt().after().sequence(),
        1,
        "STORE_TASK021_BASE_FIRST_SEQUENCE_INVALID"
    );
    assert!(
        first.outbox_admission().is_none(),
        "STORE_TASK021_BASE_FIRST_UNEXPECTED_OUTBOX"
    );
    assert_eq!(
        first.store_receipt().after_head().state_digest(),
        first.result_checkpoint().checkpoint_digest(),
        "STORE_TASK021_BASE_FIRST_PHYSICAL_CHECKPOINT_MISMATCH"
    );
    assert_live_task_ledger_persistence(&first);

    let second_command = live_task_command(
        first.receipt().after().clone(),
        "ledger-main-command-2",
        LedgerEventKind::EvidenceRecorded,
        LedgerOutcome::Recorded,
        'c',
    );
    println!("STORE_TASK021_BASE_STAGE_03_SECOND_EXECUTE");
    let second = ledger
        .execute(second_command, live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        second.receipt().after().sequence(),
        2,
        "STORE_TASK021_BASE_SECOND_SEQUENCE_INVALID"
    );

    set_live_admission(config, target, false);
    let stopped_retry = ledger
        .execute(first_command.clone(), live_authority('c', 'd'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        stopped_retry.is_exact_retry(),
        "STORE_TASK021_STOPPED_RETRY_NOT_EXACT"
    );
    assert_eq!(
        stopped_retry.receipt(),
        first.receipt(),
        "STORE_TASK021_STOPPED_RETRY_RECEIPT_CHANGED"
    );
    assert_eq!(
        stopped_retry.store_receipt(),
        first.store_receipt(),
        "STORE_TASK021_STOPPED_RETRY_STORE_CHANGED"
    );

    set_changed_live_admission(config, target);
    let changed_epoch_retry = ledger
        .execute(first_command.clone(), live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        changed_epoch_retry.is_exact_retry(),
        "STORE_TASK021_EPOCH_RETRY_NOT_EXACT"
    );
    assert_eq!(
        changed_epoch_retry.receipt(),
        first.receipt(),
        "STORE_TASK021_EPOCH_RETRY_RECEIPT_CHANGED"
    );
    set_live_admission(config, target, true);

    let before_substitution = task_ledger_counts(config, target, &identity);
    expect_task_ledger_kind(
        ledger.execute(
            live_task_command(
                zero_head.clone(),
                "ledger-main-command-1",
                LedgerEventKind::TaskCreated,
                LedgerOutcome::Recorded,
                'd',
            ),
            live_authority('a', 'b'),
        ),
        PostgresTaskLedgerErrorKind::CommandSubstitution,
    );
    assert_eq!(
        task_ledger_counts(config, target, &identity),
        before_substitution,
        "STORE_TASK021_CHANGED_REUSE_MUTATED"
    );

    let stale_command = live_task_command(
        zero_head,
        "ledger-main-stale",
        LedgerEventKind::EvidenceRecorded,
        LedgerOutcome::Recorded,
        'e',
    );
    let stale = ledger
        .execute(stale_command.clone(), live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        stale.receipt().outcome(),
        &CommandOutcome::Denied(LedgerDenial::StaleHead),
        "STORE_TASK021_STALE_NOT_DURABLY_DENIED"
    );
    assert_eq!(
        stale.receipt().before(),
        stale.receipt().after(),
        "STORE_TASK021_STALE_CHANGED_HEAD"
    );
    assert!(
        stale.outbox_admission().is_none(),
        "STORE_TASK021_STALE_UNEXPECTED_OUTBOX"
    );
    let stale_retry = ledger
        .execute(stale_command, live_authority('f', '1'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        stale_retry.is_exact_retry(),
        "STORE_TASK021_STALE_RETRY_NOT_EXACT"
    );
    assert_eq!(
        stale_retry.receipt(),
        stale.receipt(),
        "STORE_TASK021_STALE_RETRY_RECEIPT_CHANGED"
    );
    assert_eq!(
        stale_retry.store_receipt(),
        stale.store_receipt(),
        "STORE_TASK021_STALE_RETRY_STORE_CHANGED"
    );

    let after_stale = ledger
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        after_stale.stream().head().sequence(),
        2,
        "STORE_TASK021_STALE_SEQUENCE_MUTATED"
    );
    assert_eq!(
        after_stale.stream().events().len(),
        2,
        "STORE_TASK021_STALE_EVENT_COUNT_INVALID"
    );
    assert_eq!(
        after_stale.stream().commands().len(),
        3,
        "STORE_TASK021_STALE_COMMAND_COUNT_INVALID"
    );
    assert!(
        after_stale.stream().outboxes().is_empty(),
        "STORE_TASK021_STALE_OUTBOX_COUNT_INVALID"
    );
    assert_eq!(
        after_stale.physical_head().revision().get(),
        3,
        "STORE_TASK021_STALE_PHYSICAL_REVISION_INVALID"
    );

    let recorded_effect = ledger
        .execute(
            live_task_command(
                after_stale.stream().head().clone(),
                "ledger-main-effect-recorded",
                LedgerEventKind::EffectIntent,
                LedgerOutcome::Recorded,
                '6',
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let admission = recorded_effect
        .outbox_admission()
        .expect("STORE_TASK021_RECORDED_EFFECT_OUTBOX_MISSING");
    assert_eq!(
        admission.intent_digest(),
        &live_digest('6'),
        "STORE_TASK021_RECORDED_EFFECT_INTENT_MISMATCH"
    );
    assert_eq!(
        admission.command_id().as_str(),
        "ledger-main-effect-recorded",
        "STORE_TASK021_RECORDED_EFFECT_COMMAND_MISMATCH"
    );

    let failed_effect = ledger
        .execute(
            live_task_command(
                recorded_effect.receipt().after().clone(),
                "ledger-main-effect-failed",
                LedgerEventKind::EffectIntent,
                LedgerOutcome::Failed,
                '7',
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        failed_effect.receipt().outcome(),
        &CommandOutcome::Appended,
        "STORE_TASK021_FAILED_EFFECT_NOT_APPENDED"
    );
    assert!(
        failed_effect.outbox_admission().is_none(),
        "STORE_TASK021_FAILED_EFFECT_UNEXPECTED_OUTBOX"
    );

    let retained = ledger
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        retained.stream().head().sequence(),
        4,
        "STORE_TASK021_BASE_FINAL_SEQUENCE_INVALID"
    );
    assert_eq!(
        retained.stream().events().len(),
        4,
        "STORE_TASK021_BASE_FINAL_EVENT_COUNT_INVALID"
    );
    assert_eq!(
        retained.stream().commands().len(),
        5,
        "STORE_TASK021_BASE_FINAL_COMMAND_COUNT_INVALID"
    );
    assert_eq!(
        retained.stream().outboxes().len(),
        1,
        "STORE_TASK021_BASE_FINAL_OUTBOX_COUNT_INVALID"
    );
    assert_eq!(
        retained.physical_head().revision().get(),
        5,
        "STORE_TASK021_BASE_FINAL_PHYSICAL_REVISION_INVALID"
    );
    assert_eq!(
        task_ledger_counts(config, target, &identity),
        [1, 5, 1, 5, 4, 1],
        "STORE_TASK021_BASE_FINAL_ROW_COUNTS_INVALID"
    );
    prove_task_ledger_direct_dml_denials(config, target);
    set_live_admission(config, target, false);
}

fn prove_live_task_ledger_restart(config: &LiveConfig, target: &MigrationTarget) {
    let identity = live_task_identity("ledger-main", "TASK-021");
    let zero = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_RESTART_STRUCTURAL_ZERO_FAILED");
    let first_command = live_task_command(
        zero.head().clone(),
        "ledger-main-command-1",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'b',
    );
    set_live_admission(config, target, false);
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut ledger =
        PostgresTaskLedger::new(runtime, target).unwrap_or_else(|error| panic!("{}", error.code()));
    let replay = ledger
        .execute(first_command, live_authority('f', 'e'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        replay.is_exact_retry(),
        "STORE_TASK021_RESTART_REPLAY_NOT_EXACT"
    );
    assert_eq!(
        replay.receipt().after().sequence(),
        1,
        "STORE_TASK021_RESTART_ORIGINAL_RECEIPT_CHANGED"
    );
    let retained = ledger
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        retained.stream().head().sequence(),
        4,
        "STORE_TASK021_RESTART_SEQUENCE_INVALID"
    );
    assert_eq!(
        retained.stream().events().len(),
        4,
        "STORE_TASK021_RESTART_EVENT_COUNT_INVALID"
    );
    assert_eq!(
        retained.stream().commands().len(),
        5,
        "STORE_TASK021_RESTART_COMMAND_COUNT_INVALID"
    );
    assert_eq!(
        retained.stream().outboxes().len(),
        1,
        "STORE_TASK021_RESTART_OUTBOX_COUNT_INVALID"
    );
    assert_eq!(
        task_ledger_counts(config, target, &identity),
        [1, 5, 1, 5, 4, 1],
        "STORE_TASK021_RESTART_ROW_COUNTS_INVALID"
    );
    set_live_admission(config, target, true);
}

fn prove_task021_transaction_provenance_primitive(config: &LiveConfig, target: &MigrationTarget) {
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut initial = migrator
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::ReadCommitted)
        .start()
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_INITIAL_TRANSACTION_FAILED"));
    initial
        .batch_execute(
            "SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; \
             CREATE TEMP TABLE pg_temp.task021_provenance_probe (id integer PRIMARY KEY); \
             INSERT INTO pg_temp.task021_provenance_probe (id) VALUES (1)",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_INITIAL_FIXTURE_FAILED"));
    let same_transaction = initial
        .query_one(
            "SELECT xmin = pg_catalog.pg_current_xact_id()::xid \
             FROM pg_temp.task021_provenance_probe WHERE id = 1",
            &[],
        )
        .and_then(|row| row.try_get::<_, bool>(0))
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_INITIAL_PROOF_FAILED"));
    assert!(same_transaction, "STORE_TASK021_XMIN_NOT_CURRENT");
    initial
        .commit()
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_INITIAL_COMMIT_FAILED"));

    let mut later = migrator
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::ReadCommitted)
        .start()
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_LATER_TRANSACTION_FAILED"));
    later
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_LATER_HARDEN_FAILED"));
    let delayed_backfill = later
        .query_one(
            "SELECT xmin = pg_catalog.pg_current_xact_id()::xid \
             FROM pg_temp.task021_provenance_probe WHERE id = 1",
            &[],
        )
        .and_then(|row| row.try_get::<_, bool>(0))
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_LATER_PROOF_FAILED"));
    assert!(
        !delayed_backfill,
        "STORE_TASK021_DELAYED_BACKFILL_NOT_DISTINCT"
    );
    later
        .batch_execute("DROP TABLE pg_temp.task021_provenance_probe")
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_FIXTURE_CLEANUP_FAILED"));
    later
        .commit()
        .unwrap_or_else(|_| panic!("STORE_TASK021_XMIN_CLEANUP_COMMIT_FAILED"));
}

fn prove_task_ledger_direct_dml_denials(config: &LiveConfig, target: &MigrationTarget) {
    let mut runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    for sql in [
        "SELECT * FROM control.task_ledger_streams",
        "SELECT * FROM control.task_ledger_commands",
        "SELECT * FROM control.task_ledger_events",
        "SELECT * FROM control.task_ledger_outbox",
        "INSERT INTO control.task_ledger_streams DEFAULT VALUES",
        "UPDATE ONLY control.task_ledger_streams SET sequence = sequence",
        "DELETE FROM ONLY control.task_ledger_streams",
    ] {
        assert_sqlstate(
            runtime.batch_execute(sql),
            &SqlState::INSUFFICIENT_PRIVILEGE,
            "STORE_TASK021_DIRECT_DML_NOT_DENIED",
        );
    }
}

fn set_changed_live_admission(config: &LiveConfig, target: &MigrationTarget) {
    let mut fixture = config.connect(
        target.database_name(),
        "lattice-devos-task021-changed-admission",
    );
    let updated = fixture
        .execute(
            "UPDATE ONLY control.runtime_admission \
             SET admission_mode = 'ACTIVE', daemon_instance_id = 'daemon-live-2', \
                 daemon_epoch = 8, authority_revision = 4, \
                 observation_digest = $1::bytea, authority_head_digest = $2::bytea, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true",
            &[&live_digest_bytes('c'), &live_digest_bytes('d')],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_CHANGED_ADMISSION_FIXTURE_FAILED"));
    assert_eq!(updated, 1, "STORE_TASK021_CHANGED_ADMISSION_ROW_MISSING");
}

fn assert_global_task_ledger_persistence(
    persistence: &lattice_postgres_store::PostgresTaskLedgerPersistenceEvidence,
) {
    let embedded = verify_embedded_manifest().unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        persistence.schema_version(),
        3,
        "STORE_TASK021_GLOBAL_SCHEMA_NOT_V3"
    );
    assert_eq!(
        persistence.schema_version(),
        embedded.schema_version(),
        "STORE_TASK021_GLOBAL_SCHEMA_EVIDENCE_MISMATCH"
    );
    assert_eq!(
        persistence.manifest_digest().as_str(),
        embedded.manifest_sha256().as_str(),
        "STORE_TASK021_GLOBAL_MANIFEST_EVIDENCE_MISMATCH"
    );
}

fn assert_live_task_ledger_persistence(
    execution: &lattice_postgres_store::PostgresTaskLedgerExecution,
) {
    assert_global_task_ledger_persistence(execution.persistence());
    let store = execution
        .store_receipt()
        .persistence()
        .expect("STORE_TASK021_STORE_PERSISTENCE_EVIDENCE_MISSING");
    assert_eq!(
        store.schema_version(),
        2,
        "STORE_TASK021_STORE_RECEIPT_SCHEMA_NOT_V2"
    );
    assert_eq!(
        store.manifest_digest().as_str(),
        STORE_V2_MANIFEST_SHA256,
        "STORE_TASK021_STORE_RECEIPT_MANIFEST_MISMATCH"
    );
}

fn live_task_identity(project: &str, task: &str) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new(
        ProjectId::new(project).expect("STORE_TASK021_PROJECT_FIXTURE_INVALID"),
        ProjectSnapshotId::new(format!("{project}:snapshot:1"))
            .expect("STORE_TASK021_SNAPSHOT_FIXTURE_INVALID"),
        TaskId::new(task).expect("STORE_TASK021_TASK_FIXTURE_INVALID"),
        "1",
        live_digest('a'),
        "TWD",
    )
    .expect("STORE_TASK021_IDENTITY_FIXTURE_INVALID")
}

fn live_task_command(
    expected_head: lattice_contracts::TaskLedgerStreamHead,
    command_id: &str,
    kind: LedgerEventKind,
    outcome: LedgerOutcome,
    subject: char,
) -> AppendCommand {
    AppendCommand::new(
        expected_head,
        CommandId::new(command_id).expect("STORE_TASK021_COMMAND_FIXTURE_INVALID"),
        CorrelationId::new("task021-live-correlation")
            .expect("STORE_TASK021_CORRELATION_FIXTURE_INVALID"),
        "2026-08-02T00:00:00Z",
        kind,
        ActorId::new("lattice-runtime").expect("STORE_TASK021_ACTOR_FIXTURE_INVALID"),
        ActionId::new("record-live-evidence").expect("STORE_TASK021_ACTION_FIXTURE_INVALID"),
        outcome,
        ReasonCode::new("TASK021_LIVE_EVIDENCE").expect("STORE_TASK021_REASON_FIXTURE_INVALID"),
        live_digest(subject),
        None,
        None,
    )
    .expect("STORE_TASK021_APPEND_FIXTURE_INVALID")
}

fn task_ledger_counts(
    config: &LiveConfig,
    target: &MigrationTarget,
    identity: &TaskLedgerStreamIdentity,
) -> [i64; 6] {
    let stream = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_COUNT_STRUCTURAL_ZERO_FAILED");
    let stream_id = live_digest_value_bytes(stream.head().stream_id());
    let mut fixture = config.connect(target.database_name(), "task021-ledger-counts");
    let row = fixture
        .query_one(
            "SELECT \
               (SELECT count(*) FROM ONLY control.physical_heads \
                 WHERE repository_owner = 'TASK_LEDGER' \
                   AND aggregate_key_digest = $1::bytea), \
               (SELECT count(*) FROM ONLY control.terminal_transactions \
                 WHERE repository_owner = 'TASK_LEDGER' \
                   AND aggregate_key_digest = $1::bytea), \
               (SELECT count(*) FROM ONLY control.task_ledger_streams \
                 WHERE stream_id = $1::bytea), \
               (SELECT count(*) FROM ONLY control.task_ledger_commands \
                 WHERE stream_id = $1::bytea), \
               (SELECT count(*) FROM ONLY control.task_ledger_events \
                 WHERE stream_id = $1::bytea), \
               (SELECT count(*) FROM ONLY control.task_ledger_outbox \
                 WHERE stream_id = $1::bytea)",
            &[&stream_id],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_LEDGER_COUNT_FAILED"));
    std::array::from_fn(|index| row.get::<_, i64>(index))
}

fn expect_task_ledger_kind<T>(
    result: Result<T, lattice_postgres_store::PostgresTaskLedgerError>,
    expected: PostgresTaskLedgerErrorKind,
) where
    T: std::fmt::Debug,
{
    let error = result.expect_err("STORE_TASK021_EXPECTED_FAILURE_MISSING");
    assert_eq!(
        error.kind(),
        expected,
        "STORE_TASK021_ERROR_KIND_MISMATCH {}",
        error.code()
    );
}

#[allow(clippy::too_many_lines)]
fn prove_live_task_ledger_concurrency(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "tl_race");
    set_live_admission(config, &target, true);

    println!("STORE_TASK021_CONCURRENCY_01_SAME_COMMAND");
    let same_identity = live_task_identity("ledger-race-same", "TASK-021");
    let same_zero = VerifiedStream::vacant(same_identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_SAME_COMMAND_STRUCTURAL_ZERO_FAILED");
    let same_command = live_task_command(
        same_zero.head().clone(),
        "ledger-race-same-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'b',
    );
    let mut same_a = new_live_task_ledger(config, &target);
    let mut same_b = new_live_task_ledger(config, &target);
    let same_barrier = Arc::new(Barrier::new(3));
    let same_barrier_a = Arc::clone(&same_barrier);
    let same_command_a = same_command.clone();
    let same_handle_a = thread::spawn(move || {
        same_barrier_a.wait();
        same_a.execute(same_command_a, live_authority('a', 'b'))
    });
    let same_barrier_b = Arc::clone(&same_barrier);
    let same_handle_b = thread::spawn(move || {
        same_barrier_b.wait();
        same_b.execute(same_command, live_authority('a', 'b'))
    });
    same_barrier.wait();
    let same_result_a = same_handle_a
        .join()
        .expect("STORE_TASK021_SAME_COMMAND_THREAD_A_PANICKED")
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let same_result_b = same_handle_b
        .join()
        .expect("STORE_TASK021_SAME_COMMAND_THREAD_B_PANICKED")
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        same_result_a.receipt(),
        same_result_b.receipt(),
        "STORE_TASK021_SAME_COMMAND_RECEIPT_DIVERGED"
    );
    assert_eq!(
        same_result_a.store_receipt(),
        same_result_b.store_receipt(),
        "STORE_TASK021_SAME_COMMAND_STORE_RECEIPT_DIVERGED"
    );
    assert_eq!(
        usize::from(same_result_a.is_exact_retry()) + usize::from(same_result_b.is_exact_retry()),
        1,
        "STORE_TASK021_SAME_COMMAND_RETRY_COUNT_INVALID"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &same_identity),
        [1, 1, 1, 1, 1, 0],
        "STORE_TASK021_SAME_COMMAND_ROW_COUNTS_INVALID"
    );

    println!("STORE_TASK021_CONCURRENCY_02_DIFFERENT_COMMAND");
    let different_identity = live_task_identity("ledger-race-different", "TASK-021");
    let different_zero = VerifiedStream::vacant(different_identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_DIFFERENT_COMMAND_STRUCTURAL_ZERO_FAILED");
    let different_command_a = live_task_command(
        different_zero.head().clone(),
        "ledger-race-different-a",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'c',
    );
    let different_command_b = live_task_command(
        different_zero.head().clone(),
        "ledger-race-different-b",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'd',
    );
    let mut different_a = new_live_task_ledger(config, &target);
    let mut different_b = new_live_task_ledger(config, &target);
    let different_barrier = Arc::new(Barrier::new(3));
    let different_barrier_a = Arc::clone(&different_barrier);
    let different_handle_a = thread::spawn(move || {
        different_barrier_a.wait();
        different_a.execute(different_command_a, live_authority('a', 'b'))
    });
    let different_barrier_b = Arc::clone(&different_barrier);
    let different_handle_b = thread::spawn(move || {
        different_barrier_b.wait();
        different_b.execute(different_command_b, live_authority('a', 'b'))
    });
    different_barrier.wait();
    let different_result_a = different_handle_a
        .join()
        .expect("STORE_TASK021_DIFFERENT_COMMAND_THREAD_A_PANICKED")
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let different_result_b = different_handle_b
        .join()
        .expect("STORE_TASK021_DIFFERENT_COMMAND_THREAD_B_PANICKED")
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let different_results = [&different_result_a, &different_result_b];
    assert_eq!(
        different_results
            .iter()
            .filter(|result| result.receipt().outcome() == &CommandOutcome::Appended)
            .count(),
        1,
        "STORE_TASK021_DIFFERENT_COMMAND_APPEND_COUNT_INVALID"
    );
    assert_eq!(
        different_results
            .iter()
            .filter(|result| {
                result.receipt().outcome() == &CommandOutcome::Denied(LedgerDenial::StaleHead)
            })
            .count(),
        1,
        "STORE_TASK021_DIFFERENT_COMMAND_DENIAL_COUNT_INVALID"
    );
    assert!(
        different_results
            .iter()
            .all(|result| !result.is_exact_retry()),
        "STORE_TASK021_DIFFERENT_COMMAND_FALSE_RETRY"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &different_identity),
        [1, 2, 1, 2, 1, 0],
        "STORE_TASK021_DIFFERENT_COMMAND_ROW_COUNTS_INVALID"
    );

    println!("STORE_TASK021_CONCURRENCY_03_CROSS_STREAM");
    let cross_identity_a = live_task_identity("ledger-race-cross-a", "TASK-021");
    let cross_identity_b = live_task_identity("ledger-race-cross-b", "TASK-021");
    let cross_zero_a = VerifiedStream::vacant(cross_identity_a.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_CROSS_STREAM_A_STRUCTURAL_ZERO_FAILED");
    let cross_zero_b = VerifiedStream::vacant(cross_identity_b.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_CROSS_STREAM_B_STRUCTURAL_ZERO_FAILED");
    let cross_command_a = live_task_command(
        cross_zero_a.head().clone(),
        "ledger-race-cross-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'e',
    );
    let cross_command_b = live_task_command(
        cross_zero_b.head().clone(),
        "ledger-race-cross-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'f',
    );
    let mut cross_a = new_live_task_ledger(config, &target);
    let mut cross_b = new_live_task_ledger(config, &target);
    let cross_barrier = Arc::new(Barrier::new(3));
    let cross_barrier_a = Arc::clone(&cross_barrier);
    let cross_handle_a = thread::spawn(move || {
        cross_barrier_a.wait();
        cross_a.execute(cross_command_a, live_authority('a', 'b'))
    });
    let cross_barrier_b = Arc::clone(&cross_barrier);
    let cross_handle_b = thread::spawn(move || {
        cross_barrier_b.wait();
        cross_b.execute(cross_command_b, live_authority('a', 'b'))
    });
    cross_barrier.wait();
    let cross_result_a = cross_handle_a
        .join()
        .expect("STORE_TASK021_CROSS_STREAM_THREAD_A_PANICKED")
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let cross_result_b = cross_handle_b
        .join()
        .expect("STORE_TASK021_CROSS_STREAM_THREAD_B_PANICKED")
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        cross_result_a.receipt().outcome(),
        &CommandOutcome::Appended,
        "STORE_TASK021_CROSS_STREAM_A_NOT_APPENDED"
    );
    assert_eq!(
        cross_result_b.receipt().outcome(),
        &CommandOutcome::Appended,
        "STORE_TASK021_CROSS_STREAM_B_NOT_APPENDED"
    );
    assert!(
        !cross_result_a.is_exact_retry(),
        "STORE_TASK021_CROSS_STREAM_A_FALSE_RETRY"
    );
    assert!(
        !cross_result_b.is_exact_retry(),
        "STORE_TASK021_CROSS_STREAM_B_FALSE_RETRY"
    );
    assert_ne!(
        cross_result_a.receipt().after().stream_id(),
        cross_result_b.receipt().after().stream_id(),
        "STORE_TASK021_CROSS_STREAM_ID_COLLISION"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &cross_identity_a),
        [1, 1, 1, 1, 1, 0],
        "STORE_TASK021_CROSS_STREAM_A_ROW_COUNTS_INVALID"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &cross_identity_b),
        [1, 1, 1, 1, 1, 0],
        "STORE_TASK021_CROSS_STREAM_B_ROW_COUNTS_INVALID"
    );
}

fn new_live_task_ledger(config: &LiveConfig, target: &MigrationTarget) -> PostgresTaskLedger {
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    PostgresTaskLedger::new(runtime, target).unwrap_or_else(|error| panic!("{}", error.code()))
}

fn prove_live_task_ledger_atomic_rollback(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "tl_rollback");
    set_live_admission(config, &target, true);
    let identity = live_task_identity("ledger-rollback", "TASK-021");
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_ROLLBACK_STRUCTURAL_ZERO_FAILED");
    let command = live_task_command(
        vacant.head().clone(),
        "ledger-rollback-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'b',
    );
    let mut ledger = new_live_task_ledger(config, &target);
    let mut fixture = config.connect(target.database_name(), "task021-ledger-rollback-fixture");
    fixture
        .batch_execute(
            "CREATE SEQUENCE public.task021_rollback_counter; \
             REVOKE ALL ON SEQUENCE public.task021_rollback_counter FROM PUBLIC, \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE FUNCTION public.task021_fail_ledger_insert() RETURNS trigger \
             LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$ \
             BEGIN \
               PERFORM pg_catalog.nextval('public.task021_rollback_counter'::pg_catalog.regclass); \
               RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'task021 ledger insert fixture'; \
             END $$; \
             REVOKE ALL ON FUNCTION public.task021_fail_ledger_insert() FROM PUBLIC, \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE TRIGGER task021_fail_ledger_insert \
             BEFORE INSERT ON control.task_ledger_commands FOR EACH ROW \
             EXECUTE FUNCTION public.task021_fail_ledger_insert()",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_ROLLBACK_FIXTURE_FAILED"));
    expect_task_ledger_kind(
        ledger.execute(command.clone(), live_authority('a', 'b')),
        PostgresTaskLedgerErrorKind::TransactionFailed,
    );
    let trigger_count = fixture
        .query_one(
            "SELECT last_value FROM public.task021_rollback_counter",
            &[],
        )
        .and_then(|row| row.try_get::<_, i64>(0))
        .unwrap_or_else(|_| panic!("STORE_TASK021_ROLLBACK_TRIGGER_PROOF_FAILED"));
    assert_eq!(
        trigger_count, 1,
        "STORE_TASK021_ROLLBACK_TRIGGER_COUNT_INVALID"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        [0, 0, 0, 0, 0, 0],
        "STORE_TASK021_STORE_FINALIZE_NOT_ROLLED_BACK"
    );
    fixture
        .batch_execute(
            "DROP TRIGGER task021_fail_ledger_insert ON control.task_ledger_commands; \
             DROP FUNCTION public.task021_fail_ledger_insert(); \
             DROP SEQUENCE public.task021_rollback_counter",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_ROLLBACK_FIXTURE_CLEANUP_FAILED"));
    let applied = ledger
        .execute(command, live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        applied.receipt().outcome(),
        &CommandOutcome::Appended,
        "STORE_TASK021_ROLLBACK_RECOVERY_NOT_APPENDED"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        [1, 1, 1, 1, 1, 0],
        "STORE_TASK021_ROLLBACK_RECOVERY_COUNTS_INVALID"
    );
}

fn prove_live_task_ledger_serialization_retry_bound(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "tl_retry");
    set_live_admission(config, &target, true);
    let identity = live_task_identity("ledger-retry", "TASK-021");
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_RETRY_STRUCTURAL_ZERO_FAILED");
    let command = live_task_command(
        vacant.head().clone(),
        "ledger-retry-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'b',
    );
    let mut ledger = new_live_task_ledger(config, &target);
    let mut fixture = config.connect(target.database_name(), "task021-ledger-retry-fixture");
    fixture
        .batch_execute(
            "CREATE SEQUENCE public.task021_retry_counter; \
             REVOKE ALL ON SEQUENCE public.task021_retry_counter FROM PUBLIC, \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE FUNCTION public.task021_force_serialization() RETURNS trigger \
             LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$ \
             BEGIN \
               PERFORM pg_catalog.nextval('public.task021_retry_counter'::pg_catalog.regclass); \
               RAISE EXCEPTION USING ERRCODE = '40001', MESSAGE = 'task021 retry fixture'; \
             END $$; \
             REVOKE ALL ON FUNCTION public.task021_force_serialization() FROM PUBLIC, \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE TRIGGER task021_force_serialization \
             BEFORE INSERT ON control.terminal_transactions FOR EACH ROW \
             EXECUTE FUNCTION public.task021_force_serialization()",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_RETRY_FIXTURE_FAILED"));
    expect_task_ledger_kind(
        ledger.execute(command.clone(), live_authority('a', 'b')),
        PostgresTaskLedgerErrorKind::SerializationExhausted,
    );
    let trigger_count = fixture
        .query_one("SELECT last_value FROM public.task021_retry_counter", &[])
        .and_then(|row| row.try_get::<_, i64>(0))
        .unwrap_or_else(|_| panic!("STORE_TASK021_RETRY_PROOF_FAILED"));
    assert_eq!(
        trigger_count, 4,
        "STORE_TASK021_RETRY_ATTEMPT_COUNT_INVALID"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        [0, 0, 0, 0, 0, 0],
        "STORE_TASK021_RETRY_EXHAUSTION_MUTATED"
    );
    fixture
        .batch_execute(
            "DROP TRIGGER task021_force_serialization ON control.terminal_transactions; \
             DROP FUNCTION public.task021_force_serialization(); \
             DROP SEQUENCE public.task021_retry_counter",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_RETRY_FIXTURE_CLEANUP_FAILED"));
    assert_eq!(
        ledger
            .execute(command, live_authority('a', 'b'))
            .unwrap_or_else(|error| panic!("{}", error.code()))
            .receipt()
            .outcome(),
        &CommandOutcome::Appended,
        "STORE_TASK021_RETRY_RECOVERY_NOT_APPENDED"
    );
}

#[allow(clippy::too_many_lines)]
fn prove_live_task_ledger_commit_response_loss(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "tl_lost_ack");
    set_live_admission(config, &target, true);
    let identity = live_task_identity("ledger-lost-ack", "TASK-021");
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_LOST_ACK_STRUCTURAL_ZERO_FAILED");
    let command = live_task_command(
        vacant.head().clone(),
        "ledger-lost-ack-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'b',
    );

    let proxy = CommitResponseDropProxy::start_at_commit(&config.host, config.port, 2);
    let mut proxied_config = config.clone();
    proxied_config.port = proxy.port();
    let runtime = proxied_config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut uncertain = PostgresTaskLedger::new(runtime, &target)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    expect_task_ledger_kind(
        uncertain.execute(command.clone(), live_authority('a', 'b')),
        PostgresTaskLedgerErrorKind::CommitOutcomeUnknown,
    );
    expect_task_ledger_kind(
        uncertain.execute(command.clone(), live_authority('a', 'b')),
        PostgresTaskLedgerErrorKind::CommitOutcomeUnknown,
    );
    expect_task_ledger_kind(
        uncertain.load_stream(identity.clone()),
        PostgresTaskLedgerErrorKind::CommitOutcomeUnknown,
    );
    drop(uncertain);
    assert!(proxy.finish(), "STORE_TASK021_COMMIT_ACK_NOT_DROPPED");
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        [1, 1, 1, 1, 1, 0],
        "STORE_TASK021_LOST_ACK_RETAINED_COUNTS_INVALID"
    );

    let stream_id = live_digest_value_bytes(vacant.head().stream_id());
    let mut fixture = config.connect(target.database_name(), "task021-ledger-lost-proof");
    let retained = fixture
        .query_one(
            "SELECT transaction_digest, receipt_digest \
             FROM ONLY control.terminal_transactions \
             WHERE repository_owner = 'TASK_LEDGER' \
               AND aggregate_key_digest = $1::bytea",
            &[&stream_id],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_COMMIT_RETAINED_MISSING"));
    let retained_transaction = live_digest_from_bytes(&retained.get::<_, Vec<u8>>(0));
    let retained_receipt = live_digest_from_bytes(&retained.get::<_, Vec<u8>>(1));

    let mut fresh = new_live_task_ledger(config, &target);
    let loaded = fresh
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        loaded.stream().head().sequence(),
        1,
        "STORE_TASK021_LOST_ACK_RECONNECT_SEQUENCE_INVALID"
    );
    let replay = fresh
        .execute(command, live_authority('f', 'e'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        replay.is_exact_retry(),
        "STORE_TASK021_LOST_ACK_REPLAY_NOT_EXACT"
    );
    assert_eq!(
        replay.store_receipt().transaction_digest(),
        &retained_transaction,
        "STORE_TASK021_LOST_ACK_TRANSACTION_DIGEST_CHANGED"
    );
    assert_eq!(
        replay.store_receipt().receipt_digest(),
        &retained_receipt,
        "STORE_TASK021_LOST_ACK_RECEIPT_DIGEST_CHANGED"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        [1, 1, 1, 1, 1, 0],
        "STORE_TASK021_LOST_ACK_REPLAY_MUTATED"
    );
}

fn prove_live_task_ledger_manifest_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "tl_manifest");
    set_live_admission(config, &target, true);
    let identity = live_task_identity("ledger-manifest", "TASK-021");
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_MANIFEST_STRUCTURAL_ZERO_FAILED");
    let command = live_task_command(
        vacant.head().clone(),
        "ledger-manifest-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'b',
    );
    let mut ledger = new_live_task_ledger(config, &target);
    let expected_manifest = verify_embedded_manifest()
        .unwrap_or_else(|error| panic!("{}", error.code()))
        .manifest_sha256()
        .as_str()
        .to_owned();
    let mut fixture = config.connect(target.database_name(), "task021-manifest-drift-fixture");
    fixture
        .batch_execute(
            "UPDATE ONLY control.migration_history \
                SET checksum_sha256 = repeat('1', 64) WHERE ordinal = 4; \
             WITH history_payload AS ( \
                 SELECT h.ordinal, \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.migration_id, 'UTF8') || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.migration_path, 'UTF8') || \
                        pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.migration_status, 'UTF8') || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.transaction_mode, 'UTF8') || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer) AS payload \
                   FROM ONLY control.migration_history AS h \
             ), drift AS ( \
                 SELECT pg_catalog.encode( \
                            pg_catalog.sha256( \
                                pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') || \
                                pg_catalog.decode('00', 'hex') || \
                                pg_catalog.string_agg( \
                                    payload, pg_catalog.decode('', 'hex') ORDER BY ordinal \
                                ) \
                            ), \
                            'hex' \
                        ) AS manifest_sha256 \
                   FROM history_payload \
             ) \
             UPDATE ONLY control.schema_compatibility AS c \
                SET manifest_sha256 = drift.manifest_sha256 \
               FROM drift WHERE c.singleton = true",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_COHERENT_MANIFEST_DRIFT_FAILED"));
    let changed_manifest = fixture
        .query_one(
            "SELECT btrim(manifest_sha256::text) \
             FROM ONLY control.schema_compatibility WHERE singleton = true",
            &[],
        )
        .and_then(|row| row.try_get::<_, String>(0))
        .unwrap_or_else(|_| panic!("STORE_TASK021_COHERENT_MANIFEST_PROOF_FAILED"));
    assert_ne!(
        changed_manifest, expected_manifest,
        "STORE_TASK021_MANIFEST_FIXTURE_UNCHANGED"
    );
    expect_task_ledger_kind(
        ledger.execute(command, live_authority('a', 'b')),
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
    );
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        [0, 0, 0, 0, 0, 0],
        "STORE_TASK021_MANIFEST_DRIFT_MUTATED"
    );
}

fn prove_live_task_ledger_lock_timeout(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "tl_lock");
    set_live_admission(config, &target, true);
    let identity = live_task_identity("ledger-lock", "TASK-021");
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_LOCK_STRUCTURAL_ZERO_FAILED");
    let command = live_task_command(
        vacant.head().clone(),
        "ledger-lock-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        'b',
    );
    let stream_id = live_digest_value_bytes(vacant.head().stream_id());
    let mut ledger = new_live_task_ledger(config, &target);
    let mut fixture = config.connect(target.database_name(), "task021-ledger-lock-fixture");
    fixture
        .query(
            "SELECT pg_catalog.pg_advisory_lock( \
                 pg_catalog.hashtextextended( \
                    'lattice.task-ledger.stream.v1:' || pg_catalog.encode($1::bytea, 'hex'), \
                    0 \
                 ) \
             )",
            &[&stream_id],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_LOCK_FIXTURE_FAILED"));
    expect_task_ledger_kind(
        ledger.execute(command.clone(), live_authority('a', 'b')),
        PostgresTaskLedgerErrorKind::Unavailable,
    );
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        [0, 0, 0, 0, 0, 0],
        "STORE_TASK021_LOCK_TIMEOUT_MUTATED"
    );
    let unlocked = fixture
        .query_one(
            "SELECT pg_catalog.pg_advisory_unlock( \
                 pg_catalog.hashtextextended( \
                    'lattice.task-ledger.stream.v1:' || pg_catalog.encode($1::bytea, 'hex'), \
                    0 \
                 ) \
             )",
            &[&stream_id],
        )
        .and_then(|row| row.try_get::<_, bool>(0))
        .unwrap_or_else(|_| panic!("STORE_TASK021_UNLOCK_FIXTURE_FAILED"));
    assert!(unlocked, "STORE_TASK021_LOCK_FIXTURE_NOT_RELEASED");
    assert_eq!(
        ledger
            .execute(command, live_authority('a', 'b'))
            .unwrap_or_else(|error| panic!("{}", error.code()))
            .receipt()
            .outcome(),
        &CommandOutcome::Appended,
        "STORE_TASK021_LOCK_RECOVERY_NOT_APPENDED"
    );
}

fn prove_live_task_ledger_corruption(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "tl_corrupt");
    set_live_admission(config, &target, true);
    let outbox_identity = live_task_identity("ledger-corrupt-outbox", "TASK-021");
    let outbox_vacant = VerifiedStream::vacant(outbox_identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_OUTBOX_CORRUPTION_STRUCTURAL_ZERO_FAILED");
    let scope_identity = live_task_identity("ledger-corrupt-scope", "TASK-021");
    let scope_vacant = VerifiedStream::vacant(scope_identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_SCOPE_CORRUPTION_STRUCTURAL_ZERO_FAILED");
    let mut ledger = new_live_task_ledger(config, &target);
    let outbox_execution = ledger
        .execute(
            live_task_command(
                outbox_vacant.head().clone(),
                "ledger-corrupt-outbox-command",
                LedgerEventKind::EffectIntent,
                LedgerOutcome::Recorded,
                'b',
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        outbox_execution.outbox_admission().is_some(),
        "STORE_TASK021_OUTBOX_CORRUPTION_FIXTURE_MISSING"
    );
    let scope_execution = ledger
        .execute(
            live_task_command(
                scope_vacant.head().clone(),
                "ledger-corrupt-scope-command",
                LedgerEventKind::TaskCreated,
                LedgerOutcome::Recorded,
                'c',
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("{}", error.code()));

    let outbox_stream_id = live_digest_value_bytes(outbox_vacant.head().stream_id());
    let scope_transaction_id = scope_execution
        .store_receipt()
        .request()
        .transaction_id()
        .as_str()
        .to_owned();
    let mut fixture = config.connect(target.database_name(), "task021-ledger-corruption-fixture");

    prove_vacant_wrong_scope_physical_corruption(config, &target, &mut ledger, &mut fixture);

    println!("STORE_TASK021_CORRUPTION_02_OUTBOX_AND_TERMINAL");
    fixture
        .batch_execute(
            "ALTER TABLE ONLY control.task_ledger_outbox \
             DROP CONSTRAINT task_ledger_outbox_event_fk",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_OUTBOX_LINK_CORRUPTION_FAILED"));
    let changed_outbox = fixture
        .execute(
            "UPDATE ONLY control.task_ledger_outbox \
                SET event_sequence = event_sequence + 100 \
              WHERE stream_id = $1::bytea",
            &[&outbox_stream_id],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_OUTBOX_LINK_CORRUPTION_FAILED"));
    assert_eq!(
        changed_outbox, 1,
        "STORE_TASK021_OUTBOX_CORRUPTION_ROW_MISSING"
    );
    let changed_scope = fixture
        .execute(
            "UPDATE ONLY control.terminal_transactions \
                SET project_id = 'ledger-wrong-scope' \
              WHERE transaction_id = $1::text",
            &[&scope_transaction_id],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_SCOPE_CORRUPTION_FAILED"));
    assert_eq!(
        changed_scope, 1,
        "STORE_TASK021_SCOPE_CORRUPTION_ROW_MISSING"
    );

    expect_task_ledger_kind(
        ledger.load_stream(outbox_identity.clone()),
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
    );
    expect_task_ledger_kind(
        ledger.load_stream(scope_identity.clone()),
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
    );
    assert_eq!(
        task_ledger_counts(config, &target, &outbox_identity),
        [1, 1, 1, 1, 1, 1],
        "STORE_TASK021_OUTBOX_CORRUPTION_COUNTS_INVALID"
    );
    assert_eq!(
        task_ledger_counts(config, &target, &scope_identity),
        [1, 1, 1, 1, 1, 0],
        "STORE_TASK021_SCOPE_CORRUPTION_COUNTS_INVALID"
    );
}

fn prove_vacant_wrong_scope_physical_corruption(
    config: &LiveConfig,
    target: &MigrationTarget,
    ledger: &mut PostgresTaskLedger,
    fixture: &mut Client,
) {
    println!("STORE_TASK021_CORRUPTION_01_VACANT_WRONG_SCOPE");
    let identity = live_task_identity("ledger-corrupt-vacant", "TASK-021");
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK021_VACANT_SCOPE_STRUCTURAL_ZERO_FAILED");
    let wrong_scope = StoreScope::new(
        ProjectId::new("ledger-wrong-project").expect("STORE_TASK021_VACANT_WRONG_PROJECT_INVALID"),
        ProjectSnapshotId::new("ledger-wrong-project:snapshot:9")
            .expect("STORE_TASK021_VACANT_WRONG_SNAPSHOT_INVALID"),
        StoreRepositoryOwner::TaskLedger,
        vacant.head().stream_id().clone(),
    )
    .expect("STORE_TASK021_VACANT_WRONG_SCOPE_INVALID");
    let wrong_physical = fixture_live_head(wrong_scope, 0, 'd');
    let inserted = fixture
        .execute(
            "INSERT INTO control.physical_heads (\
                 project_id, project_snapshot_id, repository_owner, \
                 aggregate_key_digest, physical_revision, state_digest, head_digest\
             ) VALUES ($1::text, $2::text, 'TASK_LEDGER', $3::bytea, 0, $4::bytea, $5::bytea)",
            &[
                &wrong_physical.scope().project_id().as_str(),
                &wrong_physical.scope().project_snapshot_id().as_str(),
                &live_digest_value_bytes(wrong_physical.scope().aggregate_key_digest()),
                &live_digest_value_bytes(wrong_physical.state_digest()),
                &live_digest_value_bytes(wrong_physical.head_digest()),
            ],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_VACANT_WRONG_SCOPE_INSERT_FAILED"));
    assert_eq!(inserted, 1, "STORE_TASK021_VACANT_WRONG_SCOPE_ROW_MISSING");
    expect_task_ledger_kind(
        ledger.load_stream(identity.clone()),
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
    );
    assert_eq!(
        task_ledger_counts(config, target, &identity),
        [1, 0, 0, 0, 0, 0],
        "STORE_TASK021_VACANT_WRONG_SCOPE_COUNTS_INVALID"
    );
}

fn set_live_admission(config: &LiveConfig, target: &MigrationTarget, active: bool) {
    let mut admin = config.connect(
        target.database_name(),
        "lattice-devos-task020-admission-fixture",
    );
    let updated = if active {
        admin.execute(
            "UPDATE ONLY control.runtime_admission \
             SET admission_mode = 'ACTIVE', daemon_instance_id = 'daemon-live-1', \
                 daemon_epoch = 7, authority_revision = 3, \
                 observation_digest = $1::bytea, authority_head_digest = $2::bytea, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true",
            &[&live_digest_bytes('a'), &live_digest_bytes('b')],
        )
    } else {
        admin.execute(
            "UPDATE ONLY control.runtime_admission \
             SET admission_mode = 'STOPPED', daemon_instance_id = NULL, \
                 daemon_epoch = NULL, authority_revision = 0, \
                 observation_digest = NULL, authority_head_digest = NULL, \
                 updated_at = clock_timestamp() \
             WHERE singleton = true",
            &[],
        )
    }
    .unwrap_or_else(|_| panic!("STORE_TASK020_ADMISSION_FIXTURE_FAILED"));
    assert_eq!(updated, 1, "STORE_TASK020_ADMISSION_FIXTURE_MISSING");
}

fn read_retained_expected_head(
    config: &LiveConfig,
    target: &MigrationTarget,
    scope: &StoreScope,
    transaction_id: &str,
) -> StorePhysicalHead {
    let mut admin = config.connect(
        target.database_name(),
        "lattice-devos-task020-restart-fixture",
    );
    let row = admin
        .query_one(
            "SELECT expected_revision, expected_state_digest, expected_head_digest \
             FROM ONLY control.terminal_transactions \
             WHERE transaction_id = $1::text",
            &[&transaction_id],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_RESTART_FIXTURE_MISSING"));
    let revision = row.get::<_, i64>(0);
    let revision = u64::try_from(revision)
        .ok()
        .and_then(|value| StoreRevision::new(value).ok())
        .expect("retained revision");
    StorePhysicalHead::new(
        RuntimeKind::Live,
        scope.clone(),
        revision,
        live_digest_from_bytes(&row.get::<_, Vec<u8>>(1)),
        live_digest_from_bytes(&row.get::<_, Vec<u8>>(2)),
    )
    .expect("retained head")
}

fn live_digest(value: char) -> ContentDigest {
    ContentDigest::from_sha256(value.to_string().repeat(64)).expect("valid digest")
}

fn live_digest_bytes(value: char) -> Vec<u8> {
    let nibble = u8::try_from(value.to_digit(16).expect("hex fixture")).expect("hex nibble");
    vec![(nibble << 4) | nibble; 32]
}

fn live_digest_from_bytes(bytes: &[u8]) -> ContentDigest {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    assert_eq!(bytes.len(), 32, "STORE_TASK020_DIGEST_LENGTH_INVALID");
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    ContentDigest::from_sha256(output).expect("retained digest")
}

fn live_digest_value_bytes(digest: &ContentDigest) -> Vec<u8> {
    digest
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = u8::try_from(char::from(pair[0]).to_digit(16).expect("hex digest"))
                .expect("hex nibble");
            let low = u8::try_from(char::from(pair[1]).to_digit(16).expect("hex digest"))
                .expect("hex nibble");
            (high << 4) | low
        })
        .collect()
}

fn fixture_live_head(scope: StoreScope, revision: u64, state: char) -> StorePhysicalHead {
    let state_digest = live_digest(state);
    let string = |value: &str| CanonicalValue::String(value.to_owned());
    let scope_value = CanonicalValue::Object(vec![
        ("project_id".to_owned(), string(scope.project_id().as_str())),
        (
            "project_snapshot_id".to_owned(),
            string(scope.project_snapshot_id().as_str()),
        ),
        (
            "repository_owner".to_owned(),
            string(scope.owner().as_str()),
        ),
        (
            "aggregate_key_digest".to_owned(),
            string(scope.aggregate_key_digest().as_str()),
        ),
    ]);
    let subject = CanonicalValue::Object(vec![
        ("runtime".to_owned(), string("LIVE")),
        ("scope".to_owned(), scope_value),
        ("revision".to_owned(), string(&revision.to_string())),
        ("state_digest".to_owned(), string(state_digest.as_str())),
    ]);
    let domain = HashDomain::new("lattice.postgres-store.physical-head", "1.0")
        .expect("fixture hash domain");
    let head_digest = ContentDigest::from_sha256(
        canonical_sha256(&domain, &subject)
            .expect("fixture canonical head")
            .to_hex(),
    )
    .expect("fixture head digest");
    StorePhysicalHead::new(
        RuntimeKind::Live,
        scope,
        StoreRevision::new(revision).expect("fixture revision"),
        state_digest,
        head_digest,
    )
    .expect("fixture live head")
}

fn live_scope(project: &str, snapshot: &str, aggregate: char) -> StoreScope {
    StoreScope::new(
        ProjectId::new(project).expect("valid project"),
        ProjectSnapshotId::new(snapshot).expect("valid snapshot"),
        StoreRepositoryOwner::TaskLedger,
        live_digest(aggregate),
    )
    .expect("valid scope")
}

fn live_authority(observation: char, head: char) -> StoreAuthorityHead {
    live_authority_with_admission(RuntimeAdmissionMode::Active, observation, head)
}

fn live_authority_with_admission(
    admission: RuntimeAdmissionMode,
    observation: char,
    head: char,
) -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new("daemon-live-1").expect("valid daemon"),
        DaemonEpoch::new(7).expect("valid epoch"),
        admission,
        StoreAuthorityRevision::new(3).expect("valid authority revision"),
        live_digest(observation),
        live_digest(head),
    )
    .expect("valid authority")
}

fn live_mutation(seed: usize) -> StoreMutationCommitment {
    let values = b"123456789abcdef";
    let at = |offset: usize| char::from(values[(seed + offset) % values.len()]);
    StoreMutationCommitment::new(
        live_digest(at(0)),
        live_digest(at(1)),
        live_digest(at(2)),
        live_digest(at(3)),
        Some(live_digest(at(4))),
        Some(live_digest(at(5))),
    )
    .expect("valid mutation")
}

fn live_request(
    transaction_id: &str,
    scope: StoreScope,
    authority: StoreAuthorityHead,
    expected_head: StorePhysicalHead,
    mutation: StoreMutationCommitment,
) -> StoreTransactionRequest {
    StoreTransactionRequest::new(
        STORE_CONTRACT_VERSION,
        StoreTransactionId::new(transaction_id).expect("valid transaction id"),
        scope,
        authority,
        expected_head,
        mutation,
    )
    .expect("valid Store request")
}

fn expect_store_kind<T: std::fmt::Debug>(
    result: Result<T, lattice_ports::ControlStoreError>,
    expected: ControlStoreErrorKind,
) {
    let error = result.expect_err("expected live Store failure");
    assert_eq!(error.kind(), expected, "{}", error.code());
}

fn create_fixed_roles(admin: &mut Client, password: &str) {
    let quoted_password = admin
        .query_one("SELECT quote_literal($1::text)", &[&password])
        .and_then(|row| row.try_get::<_, String>(0))
        .unwrap_or_else(|_| panic!("TASK019_PASSWORD_QUOTE_FAILED"));
    admin
        .batch_execute(&format!(
            "CREATE ROLE lattice_migrator \
                 NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_runtime \
                NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_guardian \
                NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_readonly \
                 NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_migrator_login \
                 LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_runtime_login \
                 LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_guardian_login \
                 LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_readonly_login \
                 LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
              GRANT lattice_migrator TO lattice_migrator_login \
                  WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
              GRANT lattice_runtime TO lattice_runtime_login \
                  WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
              GRANT lattice_guardian TO lattice_guardian_login \
                  WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
              GRANT lattice_readonly TO lattice_readonly_login \
                  WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
              REVOKE ALL ON DATABASE postgres FROM PUBLIC; \
              REVOKE ALL ON DATABASE template0 FROM PUBLIC; \
              REVOKE ALL ON DATABASE template1 FROM PUBLIC"
        ))
        .unwrap_or_else(|_| panic!("TASK019_ROLE_PROVISION_FAILED"));
}

fn provision_database(
    config: &LiveConfig,
    admin: &mut Client,
    tag: &str,
    exact_sentinel: bool,
) -> MigrationTarget {
    let target = config.target(tag);
    let quoted_name = quoted_database_name(target.database_name());
    admin
        .batch_execute(&format!(
            "CREATE DATABASE {quoted_name} OWNER lattice_migrator"
        ))
        .unwrap_or_else(|_| panic!("TASK019_DATABASE_CREATE_FAILED"));

    let comment = if exact_sentinel {
        target.database_comment()
    } else {
        format!("{}x", target.database_comment())
    };
    set_exact_database_access(admin, target.database_name());
    set_exact_pre_role_function_access(config, target.database_name());
    admin
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; \
             COMMENT ON DATABASE {quoted_name} IS '{comment}'; \
             RESET ROLE"
        ))
        .unwrap_or_else(|_| panic!("TASK019_DATABASE_BOUNDARY_PROVISION_FAILED"));
    target
}

fn set_exact_database_access(admin: &mut Client, target_database: &str) {
    let database_names = admin
        .query(
            "SELECT datname::text FROM pg_database ORDER BY datname",
            &[],
        )
        .unwrap_or_else(|_| panic!("TASK019_DATABASE_INVENTORY_FAILED"));
    for row in &database_names {
        let database = row
            .try_get::<_, String>(0)
            .unwrap_or_else(|_| panic!("TASK019_DATABASE_INVENTORY_TYPE_FAILED"));
        let quoted = quoted_database_name(&database);
        admin
            .batch_execute(&format!(
                "REVOKE ALL ON DATABASE {quoted} FROM PUBLIC; \
                 REVOKE ALL ON DATABASE {quoted} FROM \
                     lattice_migrator_login, lattice_runtime_login, \
                     lattice_guardian_login, lattice_readonly_login"
            ))
            .unwrap_or_else(|_| panic!("TASK019_DATABASE_ACCESS_REVOKE_FAILED"));
    }
    let quoted_target = quoted_database_name(target_database);
    admin
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; \
             GRANT CONNECT ON DATABASE {quoted_target} TO \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             RESET ROLE"
        ))
        .unwrap_or_else(|_| panic!("TASK019_DATABASE_ACCESS_GRANT_FAILED"));
}

fn set_exact_pre_role_function_access(config: &LiveConfig, target_database: &str) {
    let mut admin = config.connect(target_database, "task019-function-boundary-provision");
    admin
        .batch_execute(
            "REVOKE ALL PRIVILEGES ON FUNCTION \
                 pg_catalog.lo_creat(integer), \
                 pg_catalog.lo_create(oid), \
                 pg_catalog.lo_from_bytea(oid, bytea), \
                 pg_catalog.lo_import(text), \
                 pg_catalog.lo_import(text, oid), \
                 pg_catalog.pg_logical_emit_message(boolean, text, text, boolean), \
                 pg_catalog.pg_logical_emit_message(boolean, text, bytea, boolean), \
                 pg_catalog.pg_advisory_lock(bigint), \
                 pg_catalog.pg_advisory_lock(integer, integer), \
                 pg_catalog.pg_advisory_lock_shared(bigint), \
                 pg_catalog.pg_advisory_lock_shared(integer, integer), \
                 pg_catalog.pg_try_advisory_lock(bigint), \
                 pg_catalog.pg_try_advisory_lock(integer, integer), \
                 pg_catalog.pg_try_advisory_lock_shared(bigint), \
                 pg_catalog.pg_try_advisory_lock_shared(integer, integer), \
                 pg_catalog.pg_advisory_xact_lock(bigint), \
                 pg_catalog.pg_advisory_xact_lock(integer, integer), \
                 pg_catalog.pg_advisory_xact_lock_shared(bigint), \
                 pg_catalog.pg_advisory_xact_lock_shared(integer, integer), \
                 pg_catalog.pg_try_advisory_xact_lock(bigint), \
                 pg_catalog.pg_try_advisory_xact_lock(integer, integer), \
                 pg_catalog.pg_try_advisory_xact_lock_shared(bigint), \
                 pg_catalog.pg_try_advisory_xact_lock_shared(integer, integer), \
                 pg_catalog.pg_cancel_backend(integer), \
                 pg_catalog.pg_terminate_backend(integer, bigint), \
                 pg_catalog.pg_export_snapshot(), \
                 pg_catalog.pg_current_xact_id(), \
                 pg_catalog.txid_current() \
             FROM PUBLIC, lattice_migrator, lattice_runtime, lattice_guardian, \
                 lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             GRANT EXECUTE ON FUNCTION \
                 pg_catalog.pg_advisory_xact_lock(bigint), \
                 pg_catalog.pg_current_xact_id() \
             TO lattice_migrator",
        )
        .unwrap_or_else(|_| panic!("TASK019_FUNCTION_BOUNDARY_PROVISION_FAILED"));
}

fn prove_first_apply_and_reconciliation(
    config: &LiveConfig,
    admin: &mut Client,
) -> (MigrationTarget, PostgresSchemaEvidence) {
    let target = provision_database(config, admin, "base", true);
    let mut first = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        must_setup(apply_migrations(&mut first, &target)),
        MigrationApplyOutcome::Applied {
            executable_count: 3
        }
    );
    let evidence = must_setup(verify_postgres_schema(
        &mut first,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_history_manifest_recomputation(&mut first, evidence.manifest_sha256().as_str());
    drop(first);

    let mut reconciler = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        must_setup(apply_migrations(&mut reconciler, &target)),
        MigrationApplyOutcome::AlreadyCurrent
    );
    let reconciled_evidence = must_setup(verify_postgres_schema(
        &mut reconciler,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(
        reconciled_evidence.database_uuid(),
        evidence.database_uuid()
    );
    assert_eq!(
        reconciled_evidence.manifest_sha256(),
        evidence.manifest_sha256()
    );
    (target, evidence)
}

fn assert_history_manifest_recomputation(client: &mut Client, expected_manifest: &str) {
    let row = client
        .query_one(
            "WITH history_payload AS (\
                 SELECT h.ordinal, \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.ordinal) || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_id, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.migration_id, 'UTF8') || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_path, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.migration_path, 'UTF8') || \
                        pg_catalog.int8send(8::bigint) || pg_catalog.int8send(h.byte_length) || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.checksum_sha256::text, 'UTF8') || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.migration_status, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.migration_status, 'UTF8') || \
                        pg_catalog.int8send(pg_catalog.octet_length(pg_catalog.convert_to(h.transaction_mode, 'UTF8'))::bigint) || \
                            pg_catalog.convert_to(h.transaction_mode, 'UTF8') || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.schema_version) || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_reader) || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_reader) || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.min_writer) || \
                        pg_catalog.int8send(2::bigint) || pg_catalog.int2send(h.max_writer) AS payload \
                   FROM ONLY control.migration_history AS h\
             ) \
             SELECT pg_catalog.encode(\
                        pg_catalog.sha256(\
                            pg_catalog.convert_to('LATTICE_POSTGRES_MIGRATION_MANIFEST_V1', 'UTF8') || \
                            pg_catalog.decode('00', 'hex') || \
                            pg_catalog.string_agg(\
                                payload, pg_catalog.decode('', 'hex') ORDER BY ordinal\
                            )\
                        ), \
                        'hex'\
                    ), \
                    pg_catalog.count(*) \
               FROM history_payload",
            &[],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_MANIFEST_RECOMPUTE_QUERY_FAILED"));
    assert_eq!(row.get::<_, String>(0), expected_manifest);
    assert_eq!(row.get::<_, i64>(1), 4);
}

fn prove_exact_v1_upgrade(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "one_upgrade", true);
    install_exact_v1(config, &target);
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        must_setup(apply_migrations(&mut migrator, &target)),
        MigrationApplyOutcome::Applied {
            executable_count: 2
        }
    );
    let evidence = must_setup(verify_postgres_schema(
        &mut migrator,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(evidence.schema_version(), 3);
}

fn prove_concurrent_v1_upgrade(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "one_race", true);
    install_exact_v1(config, &target);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let thread_config = config.clone();
        let thread_target = target.clone();
        let thread_barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mut client = thread_config.role_client(
                thread_target.database_name(),
                DatabaseRole::Migrator,
                REQUIRED_APPLICATION_NAME,
            );
            thread_barrier.wait();
            apply_migrations(&mut client, &thread_target)
        }));
    }
    let mut applied = 0;
    let mut current = 0;
    for handle in handles {
        match must_setup(
            handle
                .join()
                .unwrap_or_else(|_| panic!("STORE_TASK020_V1_CONCURRENT_RUNNER_PANICKED")),
        ) {
            MigrationApplyOutcome::Applied {
                executable_count: 2,
            } => applied += 1,
            MigrationApplyOutcome::AlreadyCurrent => current += 1,
            MigrationApplyOutcome::Applied { .. } => {
                panic!("STORE_TASK020_V1_CONCURRENT_COUNT_INVALID")
            }
        }
    }
    assert_eq!((applied, current), (1, 1));
}

fn prove_v1_upgrade_rejection_matrix(config: &LiveConfig, admin: &mut Client) {
    for (tag, fixture_sql, expected) in [
        (
            "one_nonempty",
            "INSERT INTO control.physical_heads (\
                 project_id, project_snapshot_id, repository_owner, \
                 aggregate_key_digest, physical_revision, state_digest, head_digest\
             ) VALUES (\
                 'v1-project', 'v1-snapshot', 'TASK_LEDGER', \
                 decode(repeat('11', 32), 'hex'), 0, \
                 decode(repeat('22', 32), 'hex'), \
                 decode(repeat('33', 32), 'hex')\
             )",
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        ),
        (
            "one_partial",
            "ALTER TABLE ONLY control.terminal_transactions \
             DROP CONSTRAINT terminal_transactions_scope_head_fk",
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "one_edited",
            "UPDATE ONLY control.migration_history \
             SET checksum_sha256 = repeat('1', 64) WHERE ordinal = 2",
            PostgresStoreSetupErrorKind::HistoryMismatch,
        ),
        (
            "one_order",
            "UPDATE ONLY control.migration_history SET ordinal = 99 WHERE ordinal = 1; \
             UPDATE ONLY control.migration_history SET ordinal = 1 WHERE ordinal = 2; \
             UPDATE ONLY control.migration_history SET ordinal = 2 WHERE ordinal = 99",
            PostgresStoreSetupErrorKind::HistoryMismatch,
        ),
        (
            "one_unknown",
            "INSERT INTO control.migration_history (\
                 ordinal, migration_id, migration_path, byte_length, checksum_sha256, \
                 migration_status, transaction_mode, schema_version, min_reader, \
                 max_reader, min_writer, max_writer\
             ) VALUES (\
                 3, '0003_unknown', 'db/migrations/0003_unknown.sql', 1, \
                 repeat('1', 64), 'EXECUTABLE', 'RUNNER_OWNED', 2, 2, 2, 2, 2\
             )",
            PostgresStoreSetupErrorKind::HistoryMismatch,
        ),
    ] {
        prove_v1_upgrade_rejection(config, admin, tag, fixture_sql, expected);
    }
}

fn prove_v1_upgrade_rejection(
    config: &LiveConfig,
    admin: &mut Client,
    tag: &str,
    fixture_sql: &str,
    expected: PostgresStoreSetupErrorKind,
) {
    let target = provision_database(config, admin, tag, true);
    install_exact_v1(config, &target);
    let mut fixture = config.connect(target.database_name(), "task020-v1-rejection-fixture");
    fixture
        .batch_execute(fixture_sql)
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_REJECTION_FIXTURE_FAILED"));
    drop(fixture);
    expect_apply_kind(
        config,
        &target,
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
        expected,
    );
    assert_v1_upgrade_not_committed(config, &target);
}

fn prove_v1_upgrade_transaction_rollback(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "one_rollback", true);
    install_exact_v1(config, &target);
    let mut fixture = config.connect(target.database_name(), "task020-v1-rollback-fixture");
    fixture
        .batch_execute("DROP EXTENSION plpgsql")
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_ROLLBACK_FIXTURE_FAILED"));
    drop(fixture);

    expect_apply_kind(
        config,
        &target,
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
        PostgresStoreSetupErrorKind::TransactionFailed,
    );
    assert_v1_upgrade_not_committed(config, &target);

    let mut restore = config.connect(target.database_name(), "task020-v1-rollback-restore");
    restore
        .batch_execute("CREATE EXTENSION plpgsql")
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_ROLLBACK_RESTORE_FAILED"));
    drop(restore);
    let mut retry = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        must_setup(apply_migrations(&mut retry, &target)),
        MigrationApplyOutcome::Applied {
            executable_count: 2
        }
    );
    assert_eq!(
        must_setup(verify_postgres_schema(
            &mut retry,
            &target,
            DatabaseRole::Migrator,
        ))
        .schema_version(),
        3
    );
}

fn prove_exact_nonempty_v2_upgrade_and_replay(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "two_replay", true);
    install_exact_v2(config, &target);
    seed_historical_v2_receipt(config, &target);

    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        must_setup(apply_migrations(&mut migrator, &target)),
        MigrationApplyOutcome::Applied {
            executable_count: 1
        }
    );
    assert_eq!(
        must_setup(verify_postgres_schema(
            &mut migrator,
            &target,
            DatabaseRole::Migrator,
        ))
        .schema_version(),
        3
    );
    drop(migrator);

    let mut runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = runtime
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::Serializable)
        .start()
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_REPLAY_TRANSACTION_FAILED"));
    transaction
        .batch_execute(
            "SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; \
             SET LOCAL synchronous_commit = on",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_REPLAY_HARDEN_FAILED"));
    let row = transaction
        .query_one(
            "SELECT prepare_status, database_uuid::text, \
                    encode(database_identity_digest, 'hex'), schema_version, \
                    manifest_sha256, head_found, before_revision, after_revision, \
                    terminal_disposition, encode(terminal_transaction_digest, 'hex'), \
                    encode(terminal_receipt_digest, 'hex') \
             FROM control.store_prepare_v3(\
                 2::smallint, 'v2-replay-transaction', 'v2-project', 'v2-snapshot', \
                 'TASK_LEDGER', decode(repeat('11', 32), 'hex'), \
                 decode(repeat('31', 32), 'hex'), 'LIVE', 'v2-daemon', 1, \
                 'ACTIVE', 1, decode(repeat('32', 32), 'hex'), \
                 decode(repeat('33', 32), 'hex'), 'LIVE', 0, \
                 decode(repeat('21', 32), 'hex'), decode(repeat('22', 32), 'hex'), \
                 decode(repeat('34', 32), 'hex'), decode(repeat('35', 32), 'hex'), \
                 decode(repeat('23', 32), 'hex'), decode(repeat('36', 32), 'hex'), \
                 NULL, NULL, decode(repeat('41', 32), 'hex'), \
                 decode(repeat('42', 32), 'hex')\
             )",
            &[],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_REPLAY_CALL_FAILED"));
    assert_eq!(row.get::<_, String>(0), "REPLAY");
    assert_eq!(row.get::<_, String>(1), target.expected_database_uuid());
    assert_eq!(
        row.get::<_, String>(2),
        target.expected_database_identity_sha256().as_str()
    );
    assert_eq!(row.get::<_, i16>(3), 2);
    assert_eq!(row.get::<_, String>(4), STORE_V2_MANIFEST_SHA256);
    assert!(row.get::<_, bool>(5));
    assert_eq!(row.get::<_, i64>(6), 0);
    assert_eq!(row.get::<_, i64>(7), 1);
    assert_eq!(row.get::<_, String>(8), "APPLIED");
    assert_eq!(row.get::<_, String>(9), "37".repeat(32));
    assert_eq!(row.get::<_, String>(10), "38".repeat(32));
    transaction
        .commit()
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_REPLAY_COMMIT_FAILED"));
}

fn seed_historical_v2_receipt(config: &LiveConfig, target: &MigrationTarget) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::ReadCommitted)
        .start()
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_SEED_TRANSACTION_FAILED"));
    transaction
        .batch_execute(
            "SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on; \
             INSERT INTO control.physical_heads (\
                 project_id, project_snapshot_id, repository_owner, \
                 aggregate_key_digest, physical_revision, state_digest, head_digest\
             ) VALUES (\
                 'v2-project', 'v2-snapshot', 'TASK_LEDGER', \
                 decode(repeat('11', 32), 'hex'), 1, \
                 decode(repeat('23', 32), 'hex'), decode(repeat('24', 32), 'hex')\
             )",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_SEED_HEAD_FAILED"));
    let inserted = transaction
        .execute(
            "INSERT INTO control.terminal_transactions (\
                 transaction_id, project_id, project_snapshot_id, repository_owner, \
                 aggregate_key_digest, request_digest, daemon_instance_id, daemon_epoch, \
                 admission_mode, authority_revision, authority_observation_digest, \
                 authority_head_digest, expected_revision, expected_state_digest, \
                 expected_head_digest, domain_command_digest, record_set_digest, \
                 next_state_digest, domain_receipt_digest, checkpoint_digest, \
                 outbox_intent_digest, disposition, before_revision, before_state_digest, \
                 before_head_digest, after_revision, after_state_digest, after_head_digest, \
                 transaction_digest, receipt_digest, store_contract_version, producer_id, \
                 producer_version, runtime, durability, database_uuid, \
                 database_identity_digest, schema_version, manifest_sha256\
             ) VALUES (\
                 'v2-replay-transaction', 'v2-project', 'v2-snapshot', 'TASK_LEDGER', \
                 decode(repeat('11', 32), 'hex'), decode(repeat('31', 32), 'hex'), \
                 'v2-daemon', 1, 'ACTIVE', 1, decode(repeat('32', 32), 'hex'), \
                 decode(repeat('33', 32), 'hex'), 0, decode(repeat('21', 32), 'hex'), \
                 decode(repeat('22', 32), 'hex'), decode(repeat('34', 32), 'hex'), \
                 decode(repeat('35', 32), 'hex'), decode(repeat('23', 32), 'hex'), \
                 decode(repeat('36', 32), 'hex'), NULL, NULL, 'APPLIED', 0, \
                 decode(repeat('21', 32), 'hex'), decode(repeat('22', 32), 'hex'), 1, \
                 decode(repeat('23', 32), 'hex'), decode(repeat('24', 32), 'hex'), \
                 decode(repeat('37', 32), 'hex'), decode(repeat('38', 32), 'hex'), \
                 2, 'lattice-postgres-store', '1.0', 'LIVE', 'DURABLE_POSTGRES', \
                 $1::text::uuid, decode($2, 'hex'), 2, $3\
             )",
            &[
                &target.expected_database_uuid(),
                &target.expected_database_identity_sha256().as_str(),
                &STORE_V2_MANIFEST_SHA256,
            ],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_SEED_RECEIPT_FAILED"));
    assert_eq!(inserted, 1);
    transaction
        .commit()
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_SEED_COMMIT_FAILED"));
}

fn assert_v1_upgrade_not_committed(config: &LiveConfig, target: &MigrationTarget) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let row = client
        .query_one(
            "SELECT \
               (SELECT current_schema_version FROM ONLY control.schema_compatibility \
                WHERE singleton = true), \
               (SELECT btrim(manifest_sha256::text) FROM ONLY control.schema_compatibility \
                WHERE singleton = true), \
               (SELECT count(*) FROM ONLY control.migration_history \
                WHERE migration_id = '0003_live_control_store'), \
               (SELECT count(*) FROM pg_catalog.pg_attribute AS a \
                JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                WHERE n.nspname = 'control' \
                  AND c.relname = 'terminal_transactions' \
                  AND a.attname = 'store_contract_version' \
                  AND a.attnum > 0 AND NOT a.attisdropped), \
               (SELECT count(*) FROM pg_catalog.pg_proc AS p \
                JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                WHERE n.nspname = 'control' \
                  AND p.proname IN (\
                    'store_prepare_v2', 'store_finalize_v2', 'store_current_head_v2'\
                  ))",
            &[],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_ROLLBACK_PROOF_FAILED"));
    assert_eq!(row.get::<_, i16>(0), 1);
    assert_eq!(row.get::<_, String>(1), LEGACY_V1_MANIFEST_SHA256);
    assert_eq!(row.get::<_, i64>(2), 0);
    assert_eq!(row.get::<_, i64>(3), 0);
    assert_eq!(row.get::<_, i64>(4), 0);
}

fn install_exact_v1(config: &LiveConfig, target: &MigrationTarget) {
    install_exact_prefix(config, target, 2, LEGACY_V1_MANIFEST_SHA256, 1);
}

fn install_exact_v2(config: &LiveConfig, target: &MigrationTarget) {
    install_exact_prefix(config, target, 3, STORE_V2_MANIFEST_SHA256, 2);
}

fn install_exact_prefix(
    config: &LiveConfig,
    target: &MigrationTarget,
    prefix_len: usize,
    manifest_sha256: &str,
    schema_version: i16,
) {
    let manifest = migration_manifest();
    assert_eq!(manifest.len(), 4);
    assert!(matches!(prefix_len, 2 | 3));
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::ReadCommitted)
        .start()
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_TRANSACTION_FAILED"));
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_HARDEN_FAILED"));
    for entry in manifest.iter().take(prefix_len).skip(1) {
        assert_eq!(entry.status(), MigrationStatus::Executable);
        let sql = std::str::from_utf8(entry.bytes())
            .unwrap_or_else(|_| panic!("STORE_TASK021_PREFIX_SQL_UTF8_INVALID"));
        transaction
            .batch_execute(sql)
            .unwrap_or_else(|_| panic!("STORE_TASK021_PREFIX_SQL_FAILED"));
    }
    for entry in &manifest[..prefix_len] {
        let ordinal = i16::try_from(entry.ordinal())
            .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_ORDINAL_INVALID"));
        let byte_length = i64::try_from(entry.byte_length())
            .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_LENGTH_INVALID"));
        let schema_version = i16::try_from(entry.schema_version())
            .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_SCHEMA_INVALID"));
        let min_reader = i16::try_from(*entry.reader_compatibility().start())
            .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_READER_INVALID"));
        let max_reader = i16::try_from(*entry.reader_compatibility().end())
            .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_READER_INVALID"));
        let min_writer = i16::try_from(*entry.writer_compatibility().start())
            .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_WRITER_INVALID"));
        let max_writer = i16::try_from(*entry.writer_compatibility().end())
            .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_WRITER_INVALID"));
        transaction
            .execute(
                "INSERT INTO control.migration_history (\
                    ordinal, migration_id, migration_path, byte_length, checksum_sha256, \
                    migration_status, transaction_mode, schema_version, min_reader, \
                    max_reader, min_writer, max_writer\
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &ordinal,
                    &entry.id(),
                    &entry.path(),
                    &byte_length,
                    &entry.sha256(),
                    &entry.status().as_str(),
                    &entry.transaction_mode().as_str(),
                    &schema_version,
                    &min_reader,
                    &max_reader,
                    &min_writer,
                    &max_writer,
                ],
            )
            .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_HISTORY_FAILED"));
    }
    transaction
        .execute(
            "INSERT INTO control.schema_compatibility (\
                singleton, manifest_sha256, current_schema_version, \
                min_reader, max_reader, min_writer, max_writer\
             ) VALUES (true, $1, $2, $2, $2, $2, $2)",
            &[&manifest_sha256, &schema_version],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_COMPATIBILITY_FAILED"));
    transaction
        .execute(
            "INSERT INTO control.database_identity (singleton, database_uuid) \
             VALUES (true, $1::text::uuid)",
            &[&target.expected_database_uuid()],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_IDENTITY_FAILED"));
    transaction
        .commit()
        .unwrap_or_else(|_| panic!("STORE_TASK020_V1_FIXTURE_COMMIT_FAILED"));
}

fn prove_commit_response_loss_reconciliation(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "lost_ack", true);
    let proxy = CommitResponseDropProxy::start(&config.host, config.port);
    let mut proxied_config = config.clone();
    proxied_config.port = proxy.port();
    let mut first = proxied_config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let injected = apply_migrations(&mut first, &target);
    expect_setup_kind(injected, PostgresStoreSetupErrorKind::CommitOutcomeUnknown);
    drop(first);
    assert!(proxy.finish(), "TASK019_COMMIT_RESPONSE_NOT_DROPPED");

    let mut reconciler = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        must_setup(apply_migrations(&mut reconciler, &target)),
        MigrationApplyOutcome::AlreadyCurrent
    );
    must_setup(verify_postgres_schema(
        &mut reconciler,
        &target,
        DatabaseRole::Migrator,
    ));
}

fn prove_post_apply_verification_failure(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "post_apply", true);
    let proxy = CommitAckThenCloseProxy::start(&config.host, config.port);
    let mut proxied_config = config.clone();
    proxied_config.port = proxy.port();
    let mut first = proxied_config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let first_result = apply_migrations(&mut first, &target);
    expect_setup_kind(
        first_result,
        PostgresStoreSetupErrorKind::PostApplyVerificationFailed,
    );
    drop(first);
    assert!(proxy.finish(), "TASK019_POST_APPLY_BOUNDARY_NOT_OBSERVED");

    let mut reconciler = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        must_setup(apply_migrations(&mut reconciler, &target)),
        MigrationApplyOutcome::AlreadyCurrent
    );
    must_setup(verify_postgres_schema(
        &mut reconciler,
        &target,
        DatabaseRole::Migrator,
    ));
}

struct CommitResponseDropProxy {
    port: u16,
    commit_ack_dropped: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl CommitResponseDropProxy {
    fn start(backend_host: &str, backend_port: u16) -> Self {
        Self::start_at_commit(backend_host, backend_port, 1)
    }

    fn start_at_commit(backend_host: &str, backend_port: u16, commit_ordinal: usize) -> Self {
        let (port, commit_ack_dropped, handle) =
            start_commit_boundary_proxy(backend_host, backend_port, false, commit_ordinal);
        Self {
            port,
            commit_ack_dropped,
            handle,
        }
    }

    const fn port(&self) -> u16 {
        self.port
    }

    fn finish(self) -> bool {
        let Self {
            commit_ack_dropped,
            handle,
            ..
        } = self;
        handle
            .join()
            .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_PANICKED"));
        commit_ack_dropped.load(Ordering::SeqCst)
    }
}

struct CommitAckThenCloseProxy {
    port: u16,
    commit_boundary_observed: Arc<AtomicBool>,
    handle: thread::JoinHandle<()>,
}

impl CommitAckThenCloseProxy {
    fn start(backend_host: &str, backend_port: u16) -> Self {
        let (port, commit_boundary_observed, handle) =
            start_commit_boundary_proxy(backend_host, backend_port, true, 1);
        Self {
            port,
            commit_boundary_observed,
            handle,
        }
    }

    const fn port(&self) -> u16 {
        self.port
    }

    fn finish(self) -> bool {
        let Self {
            commit_boundary_observed,
            handle,
            ..
        } = self;
        handle
            .join()
            .unwrap_or_else(|_| panic!("TASK019_POST_APPLY_PROXY_PANICKED"));
        commit_boundary_observed.load(Ordering::SeqCst)
    }
}

fn start_commit_boundary_proxy(
    backend_host: &str,
    backend_port: u16,
    forward_commit_ack: bool,
    target_commit_ordinal: usize,
) -> (u16, Arc<AtomicBool>, thread::JoinHandle<()>) {
    assert!(
        target_commit_ordinal > 0,
        "TASK019_COMMIT_PROXY_TARGET_INVALID"
    );
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_BIND_FAILED"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_ADDRESS_FAILED"))
        .port();
    let backend_host = backend_host.to_owned();
    let commit_ack_dropped = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&commit_ack_dropped);
    let handle = thread::spawn(move || {
        let (mut downstream, _) = listener
            .accept()
            .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_ACCEPT_FAILED"));
        let mut upstream = TcpStream::connect((backend_host.as_str(), backend_port))
            .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_CONNECT_FAILED"));
        downstream
            .set_nodelay(true)
            .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_SOCKET_FAILED"));
        upstream
            .set_nodelay(true)
            .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_SOCKET_FAILED"));
        for socket in [&downstream, &upstream] {
            socket
                .set_read_timeout(Some(Duration::from_secs(30)))
                .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_SOCKET_FAILED"));
            socket
                .set_write_timeout(Some(Duration::from_secs(30)))
                .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_SOCKET_FAILED"));
        }
        let request_reader = downstream
            .try_clone()
            .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_CLONE_FAILED"));
        let request_writer = upstream
            .try_clone()
            .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_CLONE_FAILED"));
        let commit_sent = Arc::new(AtomicUsize::new(0));
        let request_commit_sent = Arc::clone(&commit_sent);
        let request_handle = thread::spawn(move || {
            relay_frontend(request_reader, request_writer, &request_commit_sent);
        });
        let observed_boundary = relay_backend_until_commit_ack(
            &mut upstream,
            &mut downstream,
            &commit_sent,
            forward_commit_ack,
            target_commit_ordinal,
        );
        observed.store(observed_boundary, Ordering::SeqCst);
        let _ = downstream.shutdown(Shutdown::Both);
        let _ = upstream.shutdown(Shutdown::Both);
        request_handle
            .join()
            .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_REQUEST_PANICKED"));
    });
    (port, commit_ack_dropped, handle)
}

fn relay_frontend(mut downstream: TcpStream, mut upstream: TcpStream, commit_sent: &AtomicUsize) {
    const COMMIT_QUERY_FRAME: &[u8] = b"Q\0\0\0\x0bCOMMIT\0";
    let mut tail = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let bytes_read = match downstream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(bytes_read) => bytes_read,
        };
        let mut inspection = Vec::with_capacity(tail.len() + bytes_read);
        inspection.extend_from_slice(&tail);
        inspection.extend_from_slice(&buffer[..bytes_read]);
        let commits = inspection
            .windows(COMMIT_QUERY_FRAME.len())
            .filter(|window| *window == COMMIT_QUERY_FRAME)
            .count();
        if commits > 0 {
            commit_sent.fetch_add(commits, Ordering::SeqCst);
        }
        if upstream.write_all(&buffer[..bytes_read]).is_err() {
            return;
        }
        let tail_length = COMMIT_QUERY_FRAME
            .len()
            .saturating_sub(1)
            .min(inspection.len());
        tail.clear();
        tail.extend_from_slice(&inspection[inspection.len() - tail_length..]);
    }
}

fn relay_backend_until_commit_ack(
    upstream: &mut TcpStream,
    downstream: &mut TcpStream,
    commit_sent: &AtomicUsize,
    forward_commit_ack: bool,
    target_commit_ordinal: usize,
) -> bool {
    const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
    let mut pending = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut commit_complete = false;
    let mut completed_commits = 0_usize;
    loop {
        let bytes_read = match upstream.read(&mut buffer) {
            Ok(0) | Err(_) => return false,
            Ok(bytes_read) => bytes_read,
        };
        pending.extend_from_slice(&buffer[..bytes_read]);
        let mut consumed = 0;
        while pending.len().saturating_sub(consumed) >= 5 {
            let length_bytes: [u8; 4] = pending[consumed + 1..consumed + 5]
                .try_into()
                .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_FRAME_INVALID"));
            let payload_length = usize::try_from(u32::from_be_bytes(length_bytes))
                .unwrap_or_else(|_| panic!("TASK019_COMMIT_PROXY_FRAME_INVALID"));
            if !(4..=MAX_FRAME_BYTES).contains(&payload_length) {
                return false;
            }
            let frame_length = payload_length + 1;
            if pending.len() - consumed < frame_length {
                break;
            }
            let frame = &pending[consumed..consumed + frame_length];
            let commit_in_flight = commit_sent.load(Ordering::SeqCst) > completed_commits;
            let intercepting =
                commit_in_flight && completed_commits.saturating_add(1) == target_commit_ordinal;
            let mut commit_boundary_complete = false;
            if commit_in_flight {
                if frame[0] == b'E' {
                    return false;
                }
                if frame[0] == b'C' && frame[5..].starts_with(b"COMMIT\0") {
                    commit_complete = true;
                } else if commit_complete && frame[0] == b'Z' && frame.get(5) == Some(&b'I') {
                    commit_boundary_complete = true;
                }
            }
            if (!intercepting || forward_commit_ack) && downstream.write_all(frame).is_err() {
                return false;
            }
            consumed += frame_length;
            if commit_boundary_complete {
                completed_commits = completed_commits.saturating_add(1);
                commit_complete = false;
                if completed_commits == target_commit_ordinal {
                    if forward_commit_ack {
                        thread::sleep(Duration::from_millis(250));
                    }
                    return true;
                }
            }
        }
        if consumed > 0 {
            pending.drain(..consumed);
        }
    }
}

fn prove_concurrent_runners(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "race", true);
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let thread_config = config.clone();
        let thread_target = target.clone();
        let thread_barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mut client = thread_config.role_client(
                thread_target.database_name(),
                DatabaseRole::Migrator,
                REQUIRED_APPLICATION_NAME,
            );
            thread_barrier.wait();
            apply_migrations(&mut client, &thread_target)
        }));
    }
    let mut applied = 0;
    let mut current = 0;
    for handle in handles {
        match must_setup(
            handle
                .join()
                .unwrap_or_else(|_| panic!("TASK019_CONCURRENT_RUNNER_PANICKED")),
        ) {
            MigrationApplyOutcome::Applied {
                executable_count: 3,
            } => applied += 1,
            MigrationApplyOutcome::AlreadyCurrent => current += 1,
            MigrationApplyOutcome::Applied { .. } => panic!("TASK019_CONCURRENT_COUNT_INVALID"),
        }
    }
    assert_eq!((applied, current), (1, 1));
}

fn prove_transaction_rollback(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "rollback", true);
    let mut trigger_admin = config.connect(target.database_name(), "task019-trigger-fixture");
    trigger_admin
        .batch_execute(
            "CREATE FUNCTION public.task019_abort_foundation() RETURNS event_trigger \
             LANGUAGE plpgsql AS $$ \
             BEGIN \
               IF EXISTS (SELECT 1 FROM pg_event_trigger_ddl_commands() \
                          WHERE object_identity = 'control.database_identity') THEN \
                 RAISE EXCEPTION 'intentional task019 rollback'; \
               END IF; \
             END $$; \
             REVOKE ALL PRIVILEGES ON FUNCTION public.task019_abort_foundation() \
             FROM PUBLIC, lattice_migrator, lattice_runtime, lattice_guardian, \
                 lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE EVENT TRIGGER task019_abort_foundation ON ddl_command_end \
             EXECUTE FUNCTION public.task019_abort_foundation()",
        )
        .unwrap_or_else(|_| panic!("TASK019_ROLLBACK_FIXTURE_CREATE_FAILED"));
    drop(trigger_admin);

    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    expect_setup_kind(
        apply_migrations(&mut migrator, &target),
        PostgresStoreSetupErrorKind::TransactionFailed,
    );
    drop(migrator);
    assert_owned_schema_count(config, &target, 0);

    let mut cleanup = config.connect(target.database_name(), "task019-trigger-cleanup");
    cleanup
        .batch_execute(
            "DROP EVENT TRIGGER task019_abort_foundation; \
             DROP FUNCTION public.task019_abort_foundation()",
        )
        .unwrap_or_else(|_| panic!("TASK019_ROLLBACK_FIXTURE_DROP_FAILED"));
    drop(cleanup);
    let mut retry = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert!(matches!(
        must_setup(apply_migrations(&mut retry, &target)),
        MigrationApplyOutcome::Applied { .. }
    ));
}

fn prove_preflight_denials(config: &LiveConfig, admin: &mut Client) {
    let sentinel = provision_database(config, admin, "sentinel", false);
    expect_apply_kind(
        config,
        &sentinel,
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
        PostgresStoreSetupErrorKind::TargetUnowned,
    );
    assert_owned_schema_count(config, &sentinel, 0);

    let namespace = provision_database(config, admin, "namespace", true);
    let mut namespace_admin =
        config.connect(namespace.database_name(), "task019-namespace-fixture");
    namespace_admin
        .batch_execute("CREATE SCHEMA control")
        .unwrap_or_else(|_| panic!("TASK019_NAMESPACE_FIXTURE_FAILED"));
    expect_apply_kind(
        config,
        &namespace,
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
        PostgresStoreSetupErrorKind::SchemaCollision,
    );

    let wrong_role = provision_database(config, admin, "wrong_role", true);
    expect_apply_kind(
        config,
        &wrong_role,
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    assert_owned_schema_count(config, &wrong_role, 0);

    let wrong_setting = provision_database(config, admin, "setting", true);
    expect_apply_kind(
        config,
        &wrong_setting,
        DatabaseRole::Migrator,
        "wrong-application-name",
        PostgresStoreSetupErrorKind::UnsafeSetting,
    );
    assert_owned_schema_count(config, &wrong_setting, 0);

    let actual = provision_database(config, admin, "target", true);
    let other = config.target("absent");
    let mut client = config.role_client(
        actual.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    expect_setup_kind(
        apply_migrations(&mut client, &other),
        PostgresStoreSetupErrorKind::TargetMismatch,
    );
    assert_owned_schema_count(config, &actual, 0);
}

fn prove_catalog_and_permission_denials(
    config: &LiveConfig,
    admin: &mut Client,
    base: &MigrationTarget,
) {
    prove_cross_database_acl_drift(config, admin);
    prove_parameter_acl_drift(config, admin);
    prove_history_drift(config, admin);
    prove_constraint_and_type_drift(config, admin);
    prove_owned_type_closure(config, admin);
    prove_owner_acl_function_and_default_drift(config, admin);
    prove_column_acl_drift(config, admin);
    prove_external_column_acl_drift(config, admin);
    prove_external_capability_acl_drift(config, admin);
    prove_external_public_acl_drift(config, admin);
    prove_external_function_acl_drift(config, admin);
    prove_external_function_fixed_acl_drift(config, admin);
    prove_non_migrator_default_acl_drift(config, admin);
    prove_login_owner_dependency_drift(config, admin);
    prove_large_object_acl_drift(config, admin);
    prove_role_drift(config, admin, base);
    prove_schema_create_drift(config, admin);
    prove_inheritance_and_tombstone_drift(config, admin);
    prove_identity_and_extra_object_drift(config, admin);
    prove_history_shape_drift(config, admin);
    prove_setting_drift(config, admin);
}

fn prove_cross_database_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "db_acl");
    let other = config.target("db_acl_other");
    let quoted_other = quoted_database_name(other.database_name());
    admin
        .batch_execute(&format!(
            "CREATE DATABASE {quoted_other} OWNER lattice_migrator"
        ))
        .unwrap_or_else(|_| panic!("TASK019_CROSS_DATABASE_ACL_FIXTURE_FAILED"));
    admin
        .batch_execute(&format!(
            "REVOKE ALL ON DATABASE {quoted_other} FROM PUBLIC; \
             SET ROLE lattice_migrator; \
             GRANT CONNECT ON DATABASE {quoted_other} TO lattice_runtime_login; \
             RESET ROLE"
        ))
        .unwrap_or_else(|_| panic!("TASK019_CROSS_DATABASE_ACL_FIXTURE_FAILED"));
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    expect_setup_kind(
        verify_postgres_schema(&mut client, &target, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_parameter_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "param_acl");
    admin
        .batch_execute("GRANT SET ON PARAMETER session_replication_role TO lattice_runtime_login")
        .unwrap_or_else(|_| panic!("TASK019_PARAMETER_ACL_FIXTURE_FAILED"));
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let result = verify_postgres_schema(&mut client, &target, DatabaseRole::Migrator);
    admin
        .batch_execute(
            "REVOKE ALL ON PARAMETER session_replication_role FROM lattice_runtime_login",
        )
        .unwrap_or_else(|_| panic!("TASK019_PARAMETER_ACL_RESTORE_FAILED"));
    expect_setup_kind(result, PostgresStoreSetupErrorKind::PermissionDenied);
}

fn prove_owned_type_closure(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "shell_type");
    let mut client = config.connect(target.database_name(), "task019-shell-type-fixture");
    client
        .batch_execute("CREATE TYPE control.task019_shell")
        .unwrap_or_else(|_| panic!("TASK019_SHELL_TYPE_FIXTURE_FAILED"));
    drop(client);
    expect_verify_kind(config, &target, PostgresStoreSetupErrorKind::CorruptCatalog);
}

fn prove_column_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "column_acl");
    let mut owner = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    owner
        .batch_execute(
            "GRANT UPDATE (admission_mode, daemon_instance_id, daemon_epoch, \
                 observation_digest, authority_head_digest) \
             ON control.runtime_admission TO lattice_runtime",
        )
        .unwrap_or_else(|_| panic!("TASK019_COLUMN_ACL_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut owner, &target, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
}

fn prove_external_column_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "external_acl");
    let mut fixture = config.connect(target.database_name(), "task019-external-acl-fixture");
    fixture
        .batch_execute(
            "CREATE TABLE public.task019_external(secret text NOT NULL); \
             INSERT INTO public.task019_external(secret) VALUES ('fixture'); \
             GRANT SELECT(secret) ON public.task019_external TO lattice_runtime_login",
        )
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_COLUMN_ACL_FIXTURE_FAILED"));
    drop(fixture);

    let mut raw_login = config.connect_as(
        target.database_name(),
        DatabaseRole::Runtime.login_role(),
        "task019-external-acl-proof",
    );
    let leaked = raw_login
        .query_one("SELECT secret FROM public.task019_external", &[])
        .and_then(|row| row.try_get::<_, String>(0))
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_COLUMN_ACL_PROOF_FAILED"));
    assert_eq!(leaked, "fixture");
    drop(raw_login);

    expect_verify_kind(
        config,
        &target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_external_capability_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "external_cap");
    let mut fixture = config.connect(target.database_name(), "task019-external-cap-fixture");
    fixture
        .batch_execute(
            "CREATE TABLE public.task019_external_capability(secret text NOT NULL); \
             GRANT INSERT ON public.task019_external_capability TO lattice_readonly",
        )
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_CAPABILITY_ACL_FIXTURE_FAILED"));
    drop(fixture);
    expect_verify_kind(
        config,
        &target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_external_public_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "external_public");
    let mut fixture = config.connect(target.database_name(), "task019-external-public-fixture");
    fixture
        .batch_execute(
            "CREATE TABLE public.task019_external_public(secret text NOT NULL); \
             INSERT INTO public.task019_external_public(secret) VALUES ('fixture'); \
             GRANT SELECT ON public.task019_external_public TO PUBLIC",
        )
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_PUBLIC_ACL_FIXTURE_FAILED"));
    drop(fixture);

    let mut raw_login = config.connect_as(
        target.database_name(),
        DatabaseRole::Runtime.login_role(),
        "task019-external-public-proof",
    );
    let leaked = raw_login
        .query_one("SELECT secret FROM public.task019_external_public", &[])
        .and_then(|row| row.try_get::<_, String>(0))
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_PUBLIC_ACL_PROOF_FAILED"));
    assert_eq!(leaked, "fixture");
    drop(raw_login);

    expect_verify_kind(
        config,
        &target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_external_function_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "external_func");
    let mut fixture = config.connect(target.database_name(), "task019-external-function-fixture");
    fixture
        .batch_execute(
            "CREATE FUNCTION public.task019_external_function() RETURNS integer \
             LANGUAGE sql IMMUTABLE AS 'SELECT 1'",
        )
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_FUNCTION_ACL_FIXTURE_FAILED"));
    drop(fixture);

    let mut raw_login = config.connect_as(
        target.database_name(),
        DatabaseRole::Runtime.login_role(),
        "task019-external-function-proof",
    );
    let value = raw_login
        .query_one("SELECT public.task019_external_function()", &[])
        .and_then(|row| row.try_get::<_, i32>(0))
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_FUNCTION_ACL_PROOF_FAILED"));
    assert_eq!(value, 1);
    drop(raw_login);

    expect_verify_kind(
        config,
        &target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_external_function_fixed_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let login_target = migrated_database(config, admin, "ext_login_func");
    let mut login_fixture = config.connect(
        login_target.database_name(),
        "task019-external-login-function-fixture",
    );
    login_fixture
        .batch_execute(
            "CREATE FUNCTION public.task019_external_login_function() RETURNS integer \
             LANGUAGE sql IMMUTABLE AS 'SELECT 2'; \
             REVOKE ALL ON FUNCTION public.task019_external_login_function() FROM PUBLIC; \
             GRANT EXECUTE ON FUNCTION public.task019_external_login_function() \
             TO lattice_runtime_login",
        )
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_LOGIN_FUNCTION_FIXTURE_FAILED"));
    drop(login_fixture);
    let mut raw_login = config.connect_as(
        login_target.database_name(),
        DatabaseRole::Runtime.login_role(),
        "task019-external-login-function-proof",
    );
    assert_eq!(
        raw_login
            .query_one("SELECT public.task019_external_login_function()", &[])
            .and_then(|row| row.try_get::<_, i32>(0))
            .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_LOGIN_FUNCTION_PROOF_FAILED")),
        2
    );
    drop(raw_login);
    expect_verify_kind(
        config,
        &login_target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );

    let capability_target = migrated_database(config, admin, "ext_cap_func");
    let mut capability_fixture = config.connect(
        capability_target.database_name(),
        "task019-external-capability-function-fixture",
    );
    capability_fixture
        .batch_execute(
            "CREATE FUNCTION public.task019_external_capability_function() RETURNS integer \
             LANGUAGE sql IMMUTABLE AS 'SELECT 3'; \
             REVOKE ALL ON FUNCTION public.task019_external_capability_function() FROM PUBLIC; \
             GRANT EXECUTE ON FUNCTION public.task019_external_capability_function() \
             TO lattice_runtime",
        )
        .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_CAPABILITY_FUNCTION_FIXTURE_FAILED"));
    drop(capability_fixture);
    let mut capability = config.role_client(
        capability_target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        capability
            .query_one("SELECT public.task019_external_capability_function()", &[],)
            .and_then(|row| row.try_get::<_, i32>(0))
            .unwrap_or_else(|_| panic!("TASK019_EXTERNAL_CAPABILITY_FUNCTION_PROOF_FAILED")),
        3
    );
    drop(capability);
    expect_verify_kind(
        config,
        &capability_target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_non_migrator_default_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "default_other");
    let mut fixture = config.connect(target.database_name(), "task019-default-owner-fixture");
    fixture
        .batch_execute(
            "ALTER DEFAULT PRIVILEGES FOR ROLE lattice_runtime \
             GRANT SELECT ON TABLES TO PUBLIC",
        )
        .unwrap_or_else(|_| panic!("TASK019_DEFAULT_OWNER_ACL_FIXTURE_FAILED"));
    drop(fixture);
    expect_verify_kind(
        config,
        &target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_large_object_acl_drift(config: &LiveConfig, admin: &mut Client) {
    let owner_target = migrated_database(config, admin, "lo_owner");
    let mut owner_fixture = config.connect(
        owner_target.database_name(),
        "task019-large-object-owner-fixture",
    );
    let owner_object_oid = owner_fixture
        .query_one("SELECT pg_catalog.lo_create(0)", &[])
        .and_then(|row| row.try_get::<_, u32>(0))
        .unwrap_or_else(|_| panic!("TASK019_LARGE_OBJECT_CREATE_FAILED"));
    owner_fixture
        .batch_execute(&format!(
            "ALTER LARGE OBJECT {owner_object_oid} OWNER TO lattice_runtime_login"
        ))
        .unwrap_or_else(|_| panic!("TASK019_LARGE_OBJECT_ACL_FIXTURE_FAILED"));
    drop(owner_fixture);
    expect_verify_kind(
        config,
        &owner_target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    let mut owner_cleanup = config.connect(
        owner_target.database_name(),
        "task019-large-object-owner-cleanup",
    );
    owner_cleanup
        .batch_execute(&format!(
            "ALTER LARGE OBJECT {owner_object_oid} OWNER TO task019_harness"
        ))
        .unwrap_or_else(|_| panic!("TASK019_LARGE_OBJECT_OWNER_CLEANUP_FAILED"));
    drop(owner_cleanup);

    let acl_target = migrated_database(config, admin, "lo_acl");
    let mut acl_fixture = config.connect(
        acl_target.database_name(),
        "task019-large-object-acl-fixture",
    );
    let acl_object_oid = acl_fixture
        .query_one("SELECT pg_catalog.lo_create(0)", &[])
        .and_then(|row| row.try_get::<_, u32>(0))
        .unwrap_or_else(|_| panic!("TASK019_LARGE_OBJECT_CREATE_FAILED"));
    acl_fixture
        .batch_execute(&format!(
            "GRANT SELECT ON LARGE OBJECT {acl_object_oid} TO PUBLIC"
        ))
        .unwrap_or_else(|_| panic!("TASK019_LARGE_OBJECT_ACL_FIXTURE_FAILED"));
    drop(acl_fixture);
    expect_verify_kind(
        config,
        &acl_target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_login_owner_dependency_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "login_probe");
    let owned_database = provision_database(config, admin, "login_owned", true);
    let mut fixture = config.connect(
        owned_database.database_name(),
        "task019-login-owner-fixture",
    );
    fixture
        .batch_execute(
            "CREATE COLLATION public.task019_external_collation FROM pg_catalog.\"C\"; \
             ALTER COLLATION public.task019_external_collation \
             OWNER TO lattice_runtime_login",
        )
        .unwrap_or_else(|_| panic!("TASK019_LOGIN_OWNER_FIXTURE_FAILED"));
    drop(fixture);
    set_exact_database_access(admin, target.database_name());
    expect_verify_kind(
        config,
        &target,
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
    let mut cleanup = config.connect(
        owned_database.database_name(),
        "task019-login-owner-cleanup",
    );
    cleanup
        .batch_execute(
            "ALTER COLLATION public.task019_external_collation \
             OWNER TO task019_harness",
        )
        .unwrap_or_else(|_| panic!("TASK019_LOGIN_OWNER_CLEANUP_FAILED"));
}

fn prove_identity_and_extra_object_drift(config: &LiveConfig, admin: &mut Client) {
    let identity = migrated_database(config, admin, "identity_drift");
    let mut identity_client = config.role_client(
        identity.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    identity_client
        .batch_execute(
            "UPDATE ONLY control.database_identity \
             SET database_uuid = '11111111-1111-8111-8111-111111111111'::uuid",
        )
        .unwrap_or_else(|_| panic!("TASK019_IDENTITY_DRIFT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut identity_client, &identity, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );

    let extra_object = migrated_database(config, admin, "extra_object");
    let mut object_client = config.role_client(
        extra_object.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    object_client
        .batch_execute("CREATE COLLATION control.task019_unexpected FROM pg_catalog.\"C\"")
        .unwrap_or_else(|_| panic!("TASK019_EXTRA_OBJECT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut object_client, &extra_object, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
}

fn prove_history_shape_drift(config: &LiveConfig, admin: &mut Client) {
    let missing = migrated_database(config, admin, "history_missing");
    let mut missing_client = config.role_client(
        missing.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    missing_client
        .batch_execute("DELETE FROM ONLY control.migration_history WHERE ordinal = 2")
        .unwrap_or_else(|_| panic!("TASK019_HISTORY_MISSING_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut missing_client, &missing, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::HistoryMismatch,
    );

    let unknown = migrated_database(config, admin, "history_unknown");
    let mut unknown_client = config.role_client(
        unknown.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    unknown_client
        .batch_execute(
            "INSERT INTO control.migration_history ( \
                 ordinal, migration_id, migration_path, byte_length, checksum_sha256, \
                 migration_status, transaction_mode, schema_version, min_reader, \
                 max_reader, min_writer, max_writer \
             ) VALUES ( \
                 5, '0005_unknown', 'db/migrations/0005_unknown.sql', 1, repeat('1', 64), \
                 'EXECUTABLE', 'RUNNER_OWNED', 3, 3, 3, 3, 3 \
             )",
        )
        .unwrap_or_else(|_| panic!("TASK019_HISTORY_UNKNOWN_FIXTURE_FAILED"));
    prove_runtime_manifest_boundaries_fail_closed(config, &unknown);
    expect_setup_kind(
        verify_postgres_schema(&mut unknown_client, &unknown, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::HistoryMismatch,
    );

    let reordered = migrated_database(config, admin, "history_order");
    let mut reordered_client = config.role_client(
        reordered.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    reordered_client
        .batch_execute(
            "UPDATE ONLY control.migration_history SET ordinal = 99 WHERE ordinal = 1; \
             UPDATE ONLY control.migration_history SET ordinal = 1 WHERE ordinal = 2; \
             UPDATE ONLY control.migration_history SET ordinal = 2 WHERE ordinal = 99",
        )
        .unwrap_or_else(|_| panic!("TASK019_HISTORY_ORDER_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut reordered_client, &reordered, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::HistoryMismatch,
    );
}

fn prove_runtime_manifest_boundaries_fail_closed(config: &LiveConfig, target: &MigrationTarget) {
    let mut runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let current = runtime
        .query(
            "SELECT * FROM control.store_current_head_v3(\
                 'manifest-project', 'manifest-snapshot', 'TASK_LEDGER', \
                 pg_catalog.decode(pg_catalog.repeat('11', 32), 'hex')\
             )",
            &[],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK021_MANIFEST_CURRENT_HEAD_QUERY_FAILED"));
    assert!(
        current.is_empty(),
        "STORE_TASK021_MANIFEST_CURRENT_HEAD_ACCEPTED_DRIFT"
    );

    for (sql, marker) in [
        (
            "SELECT * FROM control.store_prepare_v3(\
                 2::smallint, 'manifest-drift-transaction', 'manifest-project', \
                 'manifest-snapshot', 'TASK_LEDGER', \
                 pg_catalog.decode(pg_catalog.repeat('11', 32), 'hex'), \
                 pg_catalog.decode(pg_catalog.repeat('12', 32), 'hex'), \
                 'LIVE', 'manifest-daemon', 1, 'ACTIVE', 1, \
                 pg_catalog.decode(pg_catalog.repeat('13', 32), 'hex'), \
                 pg_catalog.decode(pg_catalog.repeat('14', 32), 'hex'), 'LIVE', 0, \
                 pg_catalog.decode(pg_catalog.repeat('15', 32), 'hex'), \
                 pg_catalog.decode(pg_catalog.repeat('16', 32), 'hex'), \
                 pg_catalog.decode(pg_catalog.repeat('17', 32), 'hex'), \
                 pg_catalog.decode(pg_catalog.repeat('18', 32), 'hex'), \
                 pg_catalog.decode(pg_catalog.repeat('19', 32), 'hex'), \
                 pg_catalog.decode(pg_catalog.repeat('20', 32), 'hex'), \
                 NULL, NULL, \
                 pg_catalog.decode(pg_catalog.repeat('21', 32), 'hex'), \
                 pg_catalog.decode(pg_catalog.repeat('22', 32), 'hex')\
             )",
            "STORE_TASK021_MANIFEST_PREPARE_ACCEPTED_DRIFT",
        ),
        (
            "SELECT * FROM control.task_ledger_prepare_v1(\
                 pg_catalog.decode(pg_catalog.repeat('23', 32), 'hex'), \
                 'manifest-drift-command'\
             )",
            "STORE_TASK021_MANIFEST_LEDGER_ACCEPTED_DRIFT",
        ),
    ] {
        let mut transaction = runtime
            .build_transaction()
            .isolation_level(postgres::IsolationLevel::Serializable)
            .start()
            .unwrap_or_else(|_| panic!("STORE_TASK021_MANIFEST_TRANSACTION_FAILED"));
        transaction
            .batch_execute(
                "SET LOCAL search_path = pg_catalog; \
                 SET LOCAL row_security = on; \
                 SET LOCAL synchronous_commit = on",
            )
            .unwrap_or_else(|_| panic!("STORE_TASK021_MANIFEST_HARDEN_FAILED"));
        let error = transaction.query_one(sql, &[]).expect_err(marker);
        assert_eq!(
            error.as_db_error().map(|database| database.code().code()),
            Some("LST01"),
            "{marker}"
        );
        transaction
            .rollback()
            .unwrap_or_else(|_| panic!("STORE_TASK021_MANIFEST_ROLLBACK_FAILED"));
    }
}

fn prove_setting_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "setting_drift");
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    client
        .batch_execute("SET synchronous_commit = off")
        .unwrap_or_else(|_| panic!("TASK019_SETTING_DRIFT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut client, &target, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::UnsafeSetting,
    );
}

fn prove_schema_create_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "schema_ddl");
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    client
        .batch_execute("GRANT CREATE ON SCHEMA public TO lattice_runtime")
        .unwrap_or_else(|_| panic!("TASK019_SCHEMA_DDL_DRIFT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut client, &target, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_inheritance_and_tombstone_drift(config: &LiveConfig, admin: &mut Client) {
    let inherited = migrated_database(config, admin, "inherit_drift");
    let mut inherited_client = config.connect(
        inherited.database_name(),
        "task019-inheritance-drift-fixture",
    );
    inherited_client
        .batch_execute(
            "CREATE TABLE public.task019_admission_child () \
             INHERITS (control.runtime_admission)",
        )
        .unwrap_or_else(|_| panic!("TASK019_INHERITANCE_DRIFT_FIXTURE_FAILED"));
    drop(inherited_client);
    expect_verify_kind(
        config,
        &inherited,
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );

    let tombstone = migrated_database(config, admin, "drop_column");
    let mut tombstone_client =
        config.connect(tombstone.database_name(), "task019-dropped-column-fixture");
    tombstone_client
        .batch_execute(
            "ALTER TABLE control.schema_compatibility ADD COLUMN task019_ghost integer; \
             ALTER TABLE control.schema_compatibility DROP COLUMN task019_ghost",
        )
        .unwrap_or_else(|_| panic!("TASK019_DROPPED_COLUMN_FIXTURE_FAILED"));
    drop(tombstone_client);
    expect_verify_kind(
        config,
        &tombstone,
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
}

fn prove_history_drift(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "history");
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    client
        .batch_execute(
            "UPDATE control.migration_history \
             SET checksum_sha256 = repeat('1', 64) WHERE ordinal = 1",
        )
        .unwrap_or_else(|_| panic!("TASK019_HISTORY_DRIFT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut client, &target, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::HistoryMismatch,
    );
}

fn prove_constraint_and_type_drift(config: &LiveConfig, admin: &mut Client) {
    let weak = migrated_database(config, admin, "weak_check");
    let mut weak_client = config.role_client(
        weak.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    weak_client
        .batch_execute(
            "ALTER TABLE control.runtime_admission \
             DROP CONSTRAINT runtime_admission_authority_shape; \
             ALTER TABLE control.runtime_admission \
             ADD CONSTRAINT runtime_admission_authority_shape CHECK (true)",
        )
        .unwrap_or_else(|_| panic!("TASK019_WEAK_CONSTRAINT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut weak_client, &weak, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );

    let changed_type = migrated_database(config, admin, "type_drift");
    let mut type_client = config.role_client(
        changed_type.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    type_client
        .batch_execute(
            "ALTER TABLE control.migration_history \
             ALTER COLUMN byte_length TYPE numeric USING byte_length::numeric",
        )
        .unwrap_or_else(|_| panic!("TASK019_TYPE_DRIFT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut type_client, &changed_type, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
}

fn prove_owner_acl_function_and_default_drift(config: &LiveConfig, admin: &mut Client) {
    let owner = migrated_database(config, admin, "owner_drift");
    let mut owner_admin = config.connect(owner.database_name(), "task019-owner-drift");
    owner_admin
        .batch_execute("ALTER TABLE control.physical_heads OWNER TO lattice_guardian")
        .unwrap_or_else(|_| panic!("TASK019_OWNER_DRIFT_FIXTURE_FAILED"));
    drop(owner_admin);
    expect_verify_kind(config, &owner, PostgresStoreSetupErrorKind::CorruptCatalog);

    let grant = migrated_database(config, admin, "grant_drift");
    let mut grant_client = config.role_client(
        grant.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    grant_client
        .batch_execute("GRANT UPDATE ON control.runtime_admission TO lattice_runtime")
        .unwrap_or_else(|_| panic!("TASK019_GRANT_DRIFT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut grant_client, &grant, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::PermissionDenied,
    );

    let function = migrated_database(config, admin, "function_drift");
    let mut function_fixture = config.connect(function.database_name(), "task019-function-drift");
    function_fixture
        .batch_execute(
            "CREATE FUNCTION control.unexpected_function() RETURNS integer \
             LANGUAGE sql AS 'SELECT 1'; \
             REVOKE ALL ON FUNCTION control.unexpected_function() FROM PUBLIC",
        )
        .unwrap_or_else(|_| panic!("TASK019_FUNCTION_DRIFT_FIXTURE_FAILED"));
    drop(function_fixture);
    expect_verify_kind(
        config,
        &function,
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );

    let defaults = migrated_database(config, admin, "default_drift");
    let mut default_client = config.role_client(
        defaults.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    default_client
        .batch_execute(
            "ALTER DEFAULT PRIVILEGES FOR ROLE lattice_migrator \
             GRANT EXECUTE ON FUNCTIONS TO lattice_runtime_login",
        )
        .unwrap_or_else(|_| panic!("TASK019_DEFAULT_DRIFT_FIXTURE_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut default_client, &defaults, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::PermissionDenied,
    );
}

fn prove_role_drift(config: &LiveConfig, admin: &mut Client, base: &MigrationTarget) {
    admin
        .batch_execute("ALTER ROLE lattice_runtime CREATEDB")
        .unwrap_or_else(|_| panic!("TASK019_ROLE_DRIFT_FIXTURE_FAILED"));
    set_exact_database_access(admin, base.database_name());
    let mut base_client = config.role_client(
        base.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let result = verify_postgres_schema(&mut base_client, base, DatabaseRole::Migrator);
    admin
        .batch_execute("ALTER ROLE lattice_runtime NOCREATEDB")
        .unwrap_or_else(|_| panic!("TASK019_ROLE_DRIFT_RESTORE_FAILED"));
    expect_setup_kind(result, PostgresStoreSetupErrorKind::PermissionDenied);
}

fn prove_nonwriter_denials(config: &LiveConfig, admin: &mut Client, target: &MigrationTarget) {
    set_exact_database_access(admin, target.database_name());
    for role in [
        DatabaseRole::Runtime,
        DatabaseRole::Guardian,
        DatabaseRole::ReadOnly,
    ] {
        let mut client =
            config.role_client(target.database_name(), role, REQUIRED_APPLICATION_NAME);
        let identity = client
            .query_one("SELECT session_user::text, current_user::text", &[])
            .unwrap_or_else(|_| panic!("TASK019_SESSION_IDENTITY_QUERY_FAILED"));
        assert_eq!(
            identity
                .try_get::<_, String>(0)
                .unwrap_or_else(|_| panic!("TASK019_SESSION_IDENTITY_TYPE_FAILED")),
            role.login_role()
        );
        assert_eq!(
            identity
                .try_get::<_, String>(1)
                .unwrap_or_else(|_| panic!("TASK019_SESSION_IDENTITY_TYPE_FAILED")),
            role.as_str()
        );
        must_setup(verify_postgres_schema(&mut client, target, role));
        for sql in [
            "UPDATE control.runtime_admission SET admission_mode = 'ACTIVE'",
            "SELECT * FROM control.physical_heads",
            "CREATE TABLE control.forbidden_table (id integer)",
            "SET ROLE lattice_migrator",
        ] {
            assert!(
                client.batch_execute(sql).is_err(),
                "TASK019_NONWRITER_ESCAPE"
            );
        }
    }
}

fn prove_login_requires_set_role(
    config: &LiveConfig,
    admin: &mut Client,
    target: &MigrationTarget,
) {
    set_exact_database_access(admin, target.database_name());
    prove_notifications_are_non_authoritative(config, target);
    for role in DatabaseRole::ALL {
        prove_single_login_requires_set_role(config, target, role);
    }
}

fn prove_single_login_requires_set_role(
    config: &LiveConfig,
    target: &MigrationTarget,
    role: DatabaseRole,
) {
    let mut client = config.connect_as(
        target.database_name(),
        role.login_role(),
        REQUIRED_APPLICATION_NAME,
    );
    let capability = role.as_str();
    let boundary = client
        .query_one(
            "SELECT session_user::text, current_user::text, \
                 pg_has_role(session_user, $1::text, 'MEMBER'), \
                 pg_has_role(session_user, $1::text, 'USAGE'), \
                 pg_has_role(session_user, $1::text, 'SET'), \
                 has_database_privilege(session_user, current_database(), 'CONNECT'), \
                 has_database_privilege(session_user, current_database(), 'CREATE'), \
                 has_database_privilege(session_user, current_database(), 'TEMPORARY'), \
                 (SELECT count(*) FROM pg_namespace n \
                  WHERE n.nspname !~ '^pg_' AND n.nspname <> 'information_schema' \
                    AND has_schema_privilege(session_user, n.oid, 'CREATE'))",
            &[&capability],
        )
        .unwrap_or_else(|_| panic!("TASK019_PRE_SET_ROLE_BOUNDARY_QUERY_FAILED"));
    assert_eq!(
        boundary
            .try_get::<_, String>(0)
            .unwrap_or_else(|_| panic!("TASK019_PRE_SET_ROLE_TYPE_FAILED")),
        role.login_role()
    );
    assert_eq!(
        boundary
            .try_get::<_, String>(1)
            .unwrap_or_else(|_| panic!("TASK019_PRE_SET_ROLE_TYPE_FAILED")),
        role.login_role()
    );
    let expected_booleans = [true, false, true, true, false, false];
    for (offset, expected) in expected_booleans.into_iter().enumerate() {
        assert_eq!(
            boundary
                .try_get::<_, bool>(offset + 2)
                .unwrap_or_else(|_| panic!("TASK019_PRE_SET_ROLE_TYPE_FAILED")),
            expected,
            "TASK019_PRE_SET_ROLE_CAPABILITY_ESCAPE"
        );
    }
    assert_eq!(
        boundary
            .try_get::<_, i64>(8)
            .unwrap_or_else(|_| panic!("TASK019_PRE_SET_ROLE_TYPE_FAILED")),
        0
    );
    for sql in [
        "SELECT singleton FROM control.database_identity",
        "CREATE SCHEMA task019_pre_set_role_escape",
    ] {
        assert!(
            client.batch_execute(sql).is_err(),
            "TASK019_PRE_SET_ROLE_CAPABILITY_ESCAPE"
        );
    }
    prove_protected_function_denials(config, target, role, &mut client);
}

fn prove_protected_function_denials(
    config: &LiveConfig,
    target: &MigrationTarget,
    role: DatabaseRole,
    client: &mut Client,
) {
    let mut peer = config.connect_as(
        target.database_name(),
        role.login_role(),
        "lattice-task019-pre-role-peer",
    );
    let peer_pid = peer
        .query_one("SELECT pg_backend_pid()", &[])
        .and_then(|row| row.try_get::<_, i32>(0))
        .unwrap_or_else(|_| panic!("TASK019_PRE_SET_ROLE_PEER_PID_FAILED"));
    for sql in [
        "SELECT pg_catalog.lo_creat(-1)",
        "SELECT pg_catalog.lo_create(0)",
        "SELECT pg_catalog.lo_from_bytea(0, '\\x00'::bytea)",
        "SELECT pg_catalog.lo_import('task019-denied')",
        "SELECT pg_catalog.lo_import('task019-denied', 0::oid)",
        "SELECT pg_catalog.pg_logical_emit_message(\
                 false, 'lattice-task019', 'blocked')",
        "SELECT pg_catalog.pg_logical_emit_message(\
                 false, 'lattice-task019', '\\x00'::bytea)",
        "SELECT pg_catalog.pg_advisory_lock(1::bigint)",
        "SELECT pg_catalog.pg_advisory_lock(1::integer, 2::integer)",
        "SELECT pg_catalog.pg_advisory_lock_shared(1::bigint)",
        "SELECT pg_catalog.pg_advisory_lock_shared(1::integer, 2::integer)",
        "SELECT pg_catalog.pg_try_advisory_lock(1::bigint)",
        "SELECT pg_catalog.pg_try_advisory_lock(1::integer, 2::integer)",
        "SELECT pg_catalog.pg_try_advisory_lock_shared(1::bigint)",
        "SELECT pg_catalog.pg_try_advisory_lock_shared(1::integer, 2::integer)",
        "SELECT pg_catalog.pg_advisory_xact_lock(1::bigint)",
        "SELECT pg_catalog.pg_advisory_xact_lock(1::integer, 2::integer)",
        "SELECT pg_catalog.pg_advisory_xact_lock_shared(1::bigint)",
        "SELECT pg_catalog.pg_advisory_xact_lock_shared(1::integer, 2::integer)",
        "SELECT pg_catalog.pg_try_advisory_xact_lock(1::bigint)",
        "SELECT pg_catalog.pg_try_advisory_xact_lock(1::integer, 2::integer)",
        "SELECT pg_catalog.pg_try_advisory_xact_lock_shared(1::bigint)",
        "SELECT pg_catalog.pg_try_advisory_xact_lock_shared(\
                 1::integer, 2::integer)",
        "SELECT pg_catalog.pg_export_snapshot()",
        "SELECT pg_catalog.pg_current_xact_id()",
        "SELECT pg_catalog.txid_current()",
    ] {
        assert_sqlstate(
            client.batch_execute(sql),
            &SqlState::INSUFFICIENT_PRIVILEGE,
            "TASK019_EXPECTED_SQLSTATE_42501",
        );
    }
    for sql in [
        "SELECT pg_catalog.pg_cancel_backend($1)",
        "SELECT pg_catalog.pg_terminate_backend($1, 0::bigint)",
    ] {
        assert_sqlstate(
            client.query_one(sql, &[&peer_pid]),
            &SqlState::INSUFFICIENT_PRIVILEGE,
            "TASK019_BACKEND_CONTROL_SQLSTATE_42501",
        );
    }
    assert_eq!(
        peer.query_one("SELECT 1", &[])
            .and_then(|row| row.try_get::<_, i32>(0))
            .unwrap_or_else(|_| panic!("TASK019_PRE_SET_ROLE_PEER_TERMINATED")),
        1
    );
    assert_sqlstate(
        client.batch_execute("BEGIN; PREPARE TRANSACTION 'task019_pre_set_role_forbidden'"),
        &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE,
        "TASK019_PREPARED_TRANSACTION_SQLSTATE",
    );
}

fn prove_notifications_are_non_authoritative(config: &LiveConfig, target: &MigrationTarget) {
    let mut before_client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let before = must_setup(verify_postgres_schema(
        &mut before_client,
        target,
        DatabaseRole::Migrator,
    ));
    drop(before_client);

    let mut raw_login = config.connect_as(
        target.database_name(),
        DatabaseRole::Runtime.login_role(),
        "task019-notify-nonauthority",
    );
    raw_login
        .batch_execute(
            "LISTEN lattice_task019; \
             NOTIFY lattice_task019, 'ignored'; \
             UNLISTEN *",
        )
        .unwrap_or_else(|_| panic!("TASK019_NOTIFY_NONAUTHORITY_PROOF_FAILED"));
    drop(raw_login);

    let mut after_client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let after = must_setup(verify_postgres_schema(
        &mut after_client,
        target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(after, before, "TASK019_NOTIFY_CHANGED_SCHEMA_EVIDENCE");
}

fn assert_sqlstate<T>(
    result: Result<T, postgres::Error>,
    expected: &SqlState,
    marker: &'static str,
) {
    let error = match result {
        Ok(value) => {
            drop(value);
            panic!("{marker}");
        }
        Err(error) => error,
    };
    assert_eq!(
        error.as_db_error().map(postgres::error::DbError::code),
        Some(expected),
        "{marker}"
    );
}

fn migrated_database(config: &LiveConfig, admin: &mut Client, tag: &str) -> MigrationTarget {
    let target = provision_database(config, admin, tag, true);
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    must_setup(apply_migrations(&mut client, &target));
    target
}

fn expect_apply_kind(
    config: &LiveConfig,
    target: &MigrationTarget,
    role: DatabaseRole,
    application_name: &str,
    expected: PostgresStoreSetupErrorKind,
) {
    let mut client = config.role_client(target.database_name(), role, application_name);
    expect_setup_kind(apply_migrations(&mut client, target), expected);
}

fn expect_verify_kind(
    config: &LiveConfig,
    target: &MigrationTarget,
    expected: PostgresStoreSetupErrorKind,
) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    expect_setup_kind(
        verify_postgres_schema(&mut client, target, DatabaseRole::Migrator),
        expected,
    );
}

fn assert_owned_schema_count(config: &LiveConfig, target: &MigrationTarget, expected: i64) {
    let mut client = config.connect(target.database_name(), "task019-schema-count");
    let row = client
        .query_one(
            "SELECT count(*) FROM pg_namespace \
             WHERE nspname IN ('control', 'memory', 'readmodel')",
            &[],
        )
        .unwrap_or_else(|_| panic!("TASK019_SCHEMA_COUNT_FAILED"));
    let count = row
        .try_get::<_, i64>(0)
        .unwrap_or_else(|_| panic!("TASK019_SCHEMA_COUNT_TYPE_FAILED"));
    assert_eq!(count, expected);
}

fn expect_setup_kind<T>(
    result: Result<T, PostgresStoreSetupError>,
    expected: PostgresStoreSetupErrorKind,
) {
    match result {
        Ok(value) => {
            drop(value);
            panic!("TASK019_EXPECTED_FAILURE_MISSING");
        }
        Err(error) => assert_eq!(error.kind(), expected),
    }
}

fn must_setup<T>(result: Result<T, PostgresStoreSetupError>) -> T {
    result.unwrap_or_else(|error| panic!("{}", error.code()))
}

fn set_role_sql(role: DatabaseRole) -> &'static str {
    match role {
        DatabaseRole::Migrator => "SET ROLE lattice_migrator",
        DatabaseRole::Runtime => "SET ROLE lattice_runtime",
        DatabaseRole::Guardian => "SET ROLE lattice_guardian",
        DatabaseRole::ReadOnly => "SET ROLE lattice_readonly",
    }
}

fn quoted_database_name(value: &str) -> String {
    assert!(
        value.len() <= 63
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'),
        "TASK019_DATABASE_IDENTIFIER_INVALID"
    );
    format!("\"{value}\"")
}

fn required_environment(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("TASK019_REQUIRED_ENVIRONMENT_MISSING"))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}
