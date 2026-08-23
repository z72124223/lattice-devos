use std::env;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, DaemonEpoch, GitRefIdentity, ProjectClass, ProjectId, ProjectLifecycle,
    ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind, STORE_CONTRACT_VERSION,
    StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId, StoreDurability,
    StoreMutationCommitment, StorePhysicalHead, StoreReceiptDisposition, StoreRepositoryOwner,
    StoreRevision, StoreScope, StoreTransactionId, StoreTransactionRequest, TaskId,
    TaskLedgerStreamIdentity,
};
use lattice_ports::{ControlStore, ControlStoreErrorKind};
use lattice_postgres_store::{
    DatabaseRole, FakePostgresStore, MigrationApplyOutcome, MigrationStatus, MigrationTarget,
    PostgresControlStore, PostgresProjectRegistry, PostgresProjectRegistryErrorKind,
    PostgresProjectRegistryExecution, PostgresSchemaEvidence, PostgresStoreSetupError,
    PostgresStoreSetupErrorKind, PostgresTaskLedger, PostgresTaskLedgerErrorKind, apply_migrations,
    migration_manifest, verify_embedded_manifest, verify_postgres_schema,
};
use lattice_project_registry::{
    CommandId as RegistryCommandId, IdentityDrift, ReconciliationDecision, RegistryCheckpoint,
    RegistryCommand as ProjectRegistryCommand, RegistryCommandOutcome, RegistryCommandPlan,
    RepositoryObservation, VerifiedRegistryState, apply_command_plan, plan_command,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CommandOutcome, CorrelationId, Diagnostic,
    LedgerDenial, LedgerEventKind, LedgerOutcome, ReasonCode, VerifiedStream, apply_append_plan,
    plan_append,
};
use postgres::config::SslMode;
use postgres::error::SqlState;
use postgres::types::ToSql;
use postgres::{Client, Config, IsolationLevel, NoTls};

const REQUIRED_APPLICATION_NAME: &str = "lattice-devos-task019";
const HARNESS_ROLE: &str = "task019_harness";
const LEGACY_V1_MANIFEST_SHA256: &str =
    "9b126a41e542b71d434b5786e35acb66575967d055a6733b9d6bf0b8c9f0eada";
const STORE_V2_MANIFEST_SHA256: &str =
    "4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129";
const TASK_LEDGER_V3_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const REGISTRY_V4_MANIFEST_SHA256: &str =
    "df3f7ca3687afaa0d1f676158725e6d2f06670e0612df7482aa9d4d244b59f0f";
const CURRENT_V5_MANIFEST_SHA256: &str =
    "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
const CODEBASE_MEMORY_V2_PATH: &str = "db/extensions/codebase-memory/v2.sql";
const CODEBASE_MEMORY_V2_SQL_SHA256: &str =
    "9db54342b88f554ca76054c7a33ae72f04b412d2dfe21fae6eb4d8faf3e854e2";
const CODEBASE_MEMORY_V2_MANIFEST_SHA256: &str =
    "0aedbd7d9ef7ca07fc2910d0da34c163cc83e3dd56f9b28292ae1f4f0c3c4d7e";
const CODEBASE_MEMORY_V3_PATH: &str = "db/extensions/codebase-memory/v3.sql";
const CODEBASE_MEMORY_V3_SQL_SHA256: &str =
    "7388f6bfe4c2d30a20306e4f9ebdff5862125bcab58f769ba286af542cb051c3";
const CODEBASE_MEMORY_V3_MANIFEST_SHA256: &str =
    "d4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0";
const CODEBASE_MEMORY_V2_SQL: &str = include_str!("../../../db/extensions/codebase-memory/v2.sql");
const CODEBASE_MEMORY_V3_SQL: &str = include_str!("../../../db/extensions/codebase-memory/v3.sql");

macro_rules! task075_stage {
    ($name:literal, $body:expr) => {{
        println!(concat!("TASK075_STAGE_ENTER_", $name));
        let result = $body;
        println!(concat!("TASK075_STAGE_PASS_", $name));
        result
    }};
}

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
        assert!(matches!(
            phase.as_str(),
            "initial"
                | "disconnect"
                | "restart"
                | "memory_setup"
                | "task075_memory_setup"
                | "task076_writer_source_setup"
                | "task076_global_upgrade"
                | "task076_final_verify"
                | "task076_writer_fresh_setup"
                | "task076_writer_fresh_access"
                | "task076_writer_base_access"
                | "task076_writer_restart"
        ));
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
            .unwrap_or_else(|error| panic!("TASK019_LIVE_CONNECT_FAILED:{error:?}"))
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
    } else if config.phase == "disconnect" {
        run_disconnect_phase(&config);
    } else if config.phase == "restart" {
        run_restart_phase(&config);
    } else if config.phase == "memory_setup" {
        run_memory_setup_phase(&config);
    } else if config.phase == "task075_memory_setup" {
        run_task075_memory_setup_phase(&config);
    } else if config.phase == "task076_writer_source_setup" {
        run_task076_writer_source_setup_phase(&config);
    } else if config.phase == "task076_global_upgrade" {
        run_task076_global_upgrade_phase(&config);
    } else if config.phase == "task076_final_verify" {
        run_task076_final_verify_phase(&config);
    } else if config.phase == "task076_writer_fresh_setup" {
        run_task076_writer_fresh_setup_phase(&config);
    } else if config.phase == "task076_writer_fresh_access" {
        run_task076_writer_access_phase(&config, "writer_fresh");
    } else if config.phase == "task076_writer_base_access" {
        run_task076_writer_access_phase(&config, "base");
    } else {
        run_task076_writer_restart_phase(&config);
    }
}

fn run_disconnect_phase(config: &LiveConfig) {
    let mut admin = config.connect("postgres", "lattice-devos-task092-disconnect-admin");
    prove_live_task_ledger_commit_response_loss(config, &mut admin, "tl_lost_disc");
    // The isolated loss-ack fixture narrows CONNECT grants to its own database.
    // Re-establish the persisted base boundary before the physical restart phase.
    set_exact_database_access(&mut admin, config.target("base").database_name());
    println!("TASK092_COMMIT_RESPONSE_LOSS_RECONCILED_ONCE");
}

#[test]
#[ignore = "requires the coordinated marker-owned disposable PostgreSQL fixture"]
fn task075_catalog_signature_fixture() {
    let config = LiveConfig::from_environment().expect("TASK075_CATALOG_FIXTURE_ENV_MISSING");
    if config.phase == "restart" {
        let current_only = env::var("LATTICE_TASK075_CURRENT_CATALOG_ONLY").as_deref() == Ok("1");
        let tags: &[&str] = if current_only {
            &["catalog_vthree"]
        } else {
            &["catalog_bare", "catalog_vtwo", "catalog_vthree"]
        };
        for tag in tags {
            let mut client = config.connect(&config.database_name(tag), "task075-catalog-restart");
            let present: bool = client
                .query_one("SELECT to_regnamespace('control') IS NOT NULL", &[])
                .unwrap_or_else(|_| panic!("TASK075_CATALOG_RESTART_QUERY_FAILED"))
                .get(0);
            assert!(present, "TASK075_CATALOG_RESTART_PROFILE_MISSING");
        }
        println!("TASK019_RESTART_OK");
        return;
    }
    assert_eq!(
        config.phase, "initial",
        "TASK075_CATALOG_FIXTURE_PHASE_INVALID"
    );
    assert_eq!(
        verify_embedded_manifest()
            .expect("TASK075_CATALOG_MANIFEST_INVALID")
            .manifest_sha256()
            .as_str(),
        CURRENT_V5_MANIFEST_SHA256
    );

    let mut admin = config.connect("postgres", "task075-catalog-admin");
    create_fixed_roles(&mut admin, &config.password);

    let current_only = env::var("LATTICE_TASK075_CURRENT_CATALOG_ONLY").as_deref() == Ok("1");
    let evidence_target = if current_only {
        None
    } else {
        let bare = provision_database(&config, &mut admin, "catalog_bare", true);
        install_exact_v5(&config, &bare);

        let pending = provision_database(&config, &mut admin, "catalog_vtwo", true);
        install_exact_v5(&config, &pending);
        install_codebase_memory_v2(&config, &pending);
        Some(bare)
    };

    let current = provision_database(&config, &mut admin, "catalog_vthree", true);
    install_exact_v5(&config, &current);
    install_codebase_memory_v2(&config, &current);
    upgrade_codebase_memory_v3(&config, &current);

    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        evidence_target
            .as_ref()
            .unwrap_or(&current)
            .expected_database_uuid(),
        CURRENT_V5_MANIFEST_SHA256
    );
    println!("TASK075_CATALOG_FIXTURE_OK");
}

fn run_memory_setup_phase(config: &LiveConfig) {
    let mut admin = config.connect("postgres", "lattice-devos-memory-setup");
    create_fixed_roles(&mut admin, &config.password);
    let base = provision_database(config, &mut admin, "base", true);
    install_exact_v3(config, &base);
    set_exact_database_access(&mut admin, base.database_name());
    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        base.expected_database_uuid(),
        TASK_LEDGER_V3_MANIFEST_SHA256
    );
    println!("TASK019_MEMORY_SETUP_OK");
}

fn run_task075_memory_setup_phase(config: &LiveConfig) {
    let mut admin = config.connect("postgres", "lattice-devos-task075-memory-setup");
    create_fixed_roles(&mut admin, &config.password);
    let base = provision_database(config, &mut admin, "base", true);
    install_exact_v5(config, &base);
    set_exact_database_access(&mut admin, base.database_name());
    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        base.expected_database_uuid(),
        CURRENT_V5_MANIFEST_SHA256
    );
    println!("TASK075_MEMORY_V5_SETUP_OK");
}

fn run_task076_writer_source_setup_phase(config: &LiveConfig) {
    let mut admin = config.connect("postgres", "lattice-devos-task076-writer-source");
    create_fixed_roles(&mut admin, &config.password);
    let base = provision_database(config, &mut admin, "base", true);
    install_exact_v3(config, &base);
    set_exact_database_access(&mut admin, base.database_name());
    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        base.expected_database_uuid(),
        TASK_LEDGER_V3_MANIFEST_SHA256
    );
    println!("TASK076_WRITER_SOURCE_SETUP_OK");
}

fn run_task076_global_upgrade_phase(config: &LiveConfig) {
    let target = config.target("base");
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let writer_before = task076_writer_lease_fingerprint(&mut migrator);
    assert_eq!(
        must_setup(apply_migrations(&mut migrator, &target)),
        MigrationApplyOutcome::Applied {
            executable_count: 2,
        },
        "TASK076_GLOBAL_UPGRADE_OUTCOME_MISMATCH"
    );
    let writer_after = task076_writer_lease_fingerprint(&mut migrator);
    assert_eq!(
        writer_after, writer_before,
        "TASK076_GLOBAL_UPGRADE_CHANGED_WRITER_DATA"
    );
    let evidence = must_setup(verify_postgres_schema(
        &mut migrator,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(evidence.schema_version(), 5);
    assert_eq!(
        evidence.manifest_sha256().as_str(),
        CURRENT_V5_MANIFEST_SHA256
    );
    drop(migrator);

    let mut runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let pending = verify_postgres_schema(&mut runtime, &target, DatabaseRole::Runtime)
        .expect_err("TASK076_PENDING_RUNTIME_ADMITTED");
    assert_eq!(
        pending.kind(),
        PostgresStoreSetupErrorKind::CompatibilityMismatch
    );
    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        evidence.database_uuid(),
        evidence.manifest_sha256().as_str()
    );
    println!("TASK076_GLOBAL_UPGRADE_OK");
}

fn run_task076_final_verify_phase(config: &LiveConfig) {
    let target = config.target("base");
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let writer_before = task076_writer_lease_fingerprint(&mut migrator);
    assert_eq!(
        must_setup(apply_migrations(&mut migrator, &target)),
        MigrationApplyOutcome::AlreadyCurrent,
        "TASK076_FINAL_STORE_APPLY_NOT_NOOP"
    );
    let evidence = must_setup(verify_postgres_schema(
        &mut migrator,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(
        task076_writer_lease_fingerprint(&mut migrator),
        writer_before,
        "TASK076_FINAL_STORE_VERIFY_CHANGED_WRITER_DATA"
    );
    drop(migrator);
    verify_task076_current_store_roles(config, &target);
    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        evidence.database_uuid(),
        evidence.manifest_sha256().as_str()
    );
    println!("TASK076_FINAL_VERIFY_OK");
}

fn run_task076_writer_fresh_setup_phase(config: &LiveConfig) {
    let mut admin = config.connect("postgres", "lattice-devos-task076-writer-fresh");
    let target = provision_database(config, &mut admin, "writer_fresh", true);
    install_exact_v5(config, &target);

    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let evidence = must_setup(verify_postgres_schema(
        &mut migrator,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(evidence.schema_version(), 5);
    assert_eq!(
        evidence.manifest_sha256().as_str(),
        CURRENT_V5_MANIFEST_SHA256
    );
    println!(
        "TASK076_FRESH_G5_EVIDENCE database_uuid={} manifest_sha256={}",
        evidence.database_uuid(),
        evidence.manifest_sha256().as_str()
    );
    println!("TASK076_WRITER_FRESH_G5_SETUP_OK");
}

fn run_task076_writer_access_phase(config: &LiveConfig, database_tag: &str) {
    assert!(
        matches!(database_tag, "base" | "writer_fresh"),
        "TASK076_DATABASE_ACCESS_TARGET_REJECTED"
    );
    let mut admin = config.connect("postgres", "lattice-devos-task076-access");
    let database_name = config.database_name(database_tag);
    set_exact_database_access(&mut admin, &database_name);
    if database_tag == "base" {
        println!("TASK076_WRITER_BASE_ACCESS_OK");
    } else {
        println!("TASK076_WRITER_FRESH_ACCESS_OK");
    }
}

fn run_task076_writer_restart_phase(config: &LiveConfig) {
    let target = config.target("base");
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let writer_before = task076_writer_lease_fingerprint(&mut migrator);
    let evidence = must_setup(verify_postgres_schema(
        &mut migrator,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(
        task076_writer_lease_fingerprint(&mut migrator),
        writer_before,
        "TASK076_RESTART_STORE_VERIFY_CHANGED_WRITER_DATA"
    );
    drop(migrator);
    verify_task076_current_store_roles(config, &target);
    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        evidence.database_uuid(),
        evidence.manifest_sha256().as_str()
    );
    println!("TASK076_WRITER_RESTART_OK");
}

fn verify_task076_current_store_roles(config: &LiveConfig, target: &MigrationTarget) {
    for role in DatabaseRole::ALL {
        let mut client =
            config.role_client(target.database_name(), role, REQUIRED_APPLICATION_NAME);
        let evidence = must_setup(verify_postgres_schema(&mut client, target, role));
        assert_eq!(evidence.schema_version(), 5);
        assert_eq!(
            evidence.manifest_sha256().as_str(),
            CURRENT_V5_MANIFEST_SHA256
        );
    }
}

fn task076_writer_lease_fingerprint(client: &mut Client) -> Vec<String> {
    [
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(t)::text,E'\n' ORDER BY t.singleton),'')) \
           FROM ONLY writer_lease.writer_lease_extension_identity t",
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(t)::text,E'\n' ORDER BY t.ledger_ordinal),'')) \
           FROM ONLY writer_lease.writer_lease_extension_ledger t",
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(t)::text,E'\n' ORDER BY t.project_id),'')) \
           FROM ONLY writer_lease.writer_lease_heads t",
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(t)::text,E'\n' ORDER BY t.project_id,t.ordinal),'')) \
           FROM ONLY writer_lease.writer_lease_commands t",
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(t)::text,E'\n' ORDER BY t.project_id,t.ordinal),'')) \
           FROM ONLY writer_lease.writer_lease_transitions t",
    ]
    .into_iter()
    .map(|query| {
        client
            .query_one(query, &[])
            .unwrap_or_else(|_| panic!("TASK076_WRITER_FINGERPRINT_QUERY_FAILED"))
            .get(0)
    })
    .collect()
}

fn run_initial_phase(config: &LiveConfig) {
    let mut admin = config.connect("postgres", "lattice-devos-task019-admin");
    create_fixed_roles(&mut admin, &config.password);

    let (base, evidence) = task075_stage!(
        "FRESH_V5_RECONCILIATION",
        prove_first_apply_and_reconciliation(config, &mut admin)
    );
    task075_stage!(
        "CURRENT_MANIFEST_SUBSTITUTION",
        prove_runtime_manifest_boundaries_fail_closed(config, &base)
    );
    task075_stage!(
        "MISPLACED_AUTONOMY_0005_PRE_DDL",
        prove_misplaced_autonomy_0005_pre_ddl_rejection(config, &mut admin)
    );
    set_exact_database_access(&mut admin, base.database_name());
    println!("STORE_TASK022_STAGE_01_PROJECT_REGISTRY");
    prove_live_project_registry(config, &base);
    prove_exact_v1_upgrade(config, &mut admin);
    prove_concurrent_v1_upgrade(config, &mut admin);
    prove_v1_upgrade_rejection_matrix(config, &mut admin);
    prove_v1_upgrade_transaction_rollback(config, &mut admin);
    prove_exact_nonempty_v2_upgrade_and_replay(config, &mut admin);
    task075_stage!(
        "REGISTRY_V4_V5_MIXED_REPLAY",
        prove_exact_nonempty_v4_registry_upgrade_and_mixed_replay(config, &mut admin)
    );
    task075_stage!(
        "REGISTRY_PROVENANCE_CORRUPTION",
        prove_task075_registry_provenance_corruption(config, &mut admin)
    );
    println!("STORE_TASK022_STAGE_06_V3_LEDGER_UPGRADE");
    prove_exact_nonempty_v3_ledger_upgrade_and_replay(config, &mut admin);
    task075_stage!(
        "V3_MEMORY_V2_GLOBAL_UPGRADE",
        prove_exact_v3_memory_v2_global_upgrade(config, &mut admin)
    );
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
    prove_live_task_ledger_commit_response_loss(config, &mut admin, "tl_lost_ack");
    println!("STORE_TASK021_STAGE_05_MANIFEST_DRIFT");
    prove_live_task_ledger_manifest_drift(config, &mut admin);
    println!("STORE_TASK021_STAGE_06_LOCK_TIMEOUT");
    prove_live_task_ledger_lock_timeout(config, &mut admin);
    println!("STORE_TASK021_STAGE_07_RETAINED_CORRUPTION");
    prove_live_task_ledger_corruption(config, &mut admin);
    println!("STORE_TASK022_STAGE_02_ATOMIC_ROLLBACK");
    prove_live_project_registry_atomic_rollback(config, &mut admin);
    println!("STORE_TASK022_STAGE_03_COMMIT_RESPONSE_LOSS");
    prove_live_project_registry_commit_response_loss(config, &mut admin);
    println!("STORE_TASK022_STAGE_04_LOCK_TIMEOUT");
    prove_live_project_registry_lock_timeout(config, &mut admin);
    println!("STORE_TASK022_STAGE_05_PARTIAL_AND_CORRUPTION");
    prove_live_project_registry_corruption(config, &mut admin);
    set_exact_database_access(&mut admin, base.database_name());
    prove_live_control_store(config, &base);
    println!("STORE_TASK021_STAGE_08_BASE_LEDGER");
    prove_live_task_ledger(config, &base);
    println!("STORE_TASK038_TASK_CREATED_JSONB_ROUND_TRIP");
    prove_task038_task_created_jsonb_round_trip(config, &base);
    println!("STORE_TASK021_STAGE_09_XMIN_PROVENANCE");
    prove_task021_transaction_provenance_primitive(config, &base);
    // Later isolated fixtures intentionally narrow database CONNECT grants to
    // their own target. Restore the base target's exact grants before its
    // durable evidence is handed to the fresh-process restart verifier.
    set_exact_database_access(&mut admin, base.database_name());
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
    println!("STORE_TASK022_RESTART_STAGE_01_PROJECT_REGISTRY");
    prove_live_project_registry_restart(config, &target);
    let mixed_database = config.database_name("reg_mixed");
    let mut restart_admin = config.connect("postgres", "task075-registry-mixed-restart-access");
    set_exact_database_access(&mut restart_admin, &mixed_database);
    drop(restart_admin);
    task075_stage!(
        "REGISTRY_V4_V5_MIXED_RESTART",
        prove_task075_registry_mixed_restart(config)
    );
    println!("TASK019_RESTART_OK");
}

fn registry_observation() -> RepositoryObservation {
    registry_observation_fixture("C:/lattice/registry-live", ['2', '3', '4', '5'])
}

fn registry_observation_fixture(root: &str, digests: [char; 4]) -> RepositoryObservation {
    RepositoryObservation::new(
        root,
        live_digest(digests[0]),
        live_digest(digests[1]),
        live_digest(digests[2]),
        GitRefIdentity::new("refs/heads/main", live_digest(digests[3])).expect("registry ref"),
    )
    .expect("registry observation")
}

fn registry_registration(command_id: &str, project_id: &str) -> ProjectRegistryCommand {
    registry_registration_with(command_id, project_id, registry_observation())
}

fn registry_registration_with(
    command_id: &str,
    project_id: &str,
    observation: RepositoryObservation,
) -> ProjectRegistryCommand {
    ProjectRegistryCommand::register(
        RegistryCommandId::new(command_id).expect("registry command id"),
        ProjectId::new(project_id).expect("registry project id"),
        ProjectClass::UserProject,
        observation,
    )
}

const TASK075_REGISTRY_V1_STAGE_SURFACE: &str = "PROJECT_REGISTRY_STAGE_COMMAND_V1";
const REGISTRY_V1_WRITE_SETTINGS: &str = "\
    SET LOCAL search_path = pg_catalog; \
    SET LOCAL row_security = on; \
    SET LOCAL synchronous_commit = on; \
    SET LOCAL lock_timeout = '5s'; \
    SET LOCAL statement_timeout = '30s'; \
    SET LOCAL idle_in_transaction_session_timeout = '30s'";
const REGISTRY_V1_PREPARE_SQL: &str = "\
    SELECT prepare_status, retained_request_digest, retained_result_digest, \
           retained_record_set_digest, retained_persistence_receipt_digest, \
           retained_base_checkpoint_digest, retained_result_checkpoint_digest, \
           current_ordinal, current_observation_count, current_project_count, \
           current_command_count, current_reservation_count, current_retained_bytes, \
           current_checkpoint_digest \
      FROM control.project_registry_prepare_v1(\
           $1::smallint,$2::text,$3::text,$4::bytea,$5::text,$6::text,$7::bigint,\
           $8::text,$9::bigint,$10::bytea,$11::bytea,$12::bytea)";
const REGISTRY_V1_STAGE_COMMAND_SQL: &str = "\
    SELECT control.project_registry_stage_command_v1(\
      $1::smallint,$2::text,$3::bigint,$4::text,$5::text,$6::text,$7::text,$8::bytea,\
      $9::boolean,$10::text,$11::text,$12::text,$13::text,$14::text,$15::text::numeric,\
      $16::text,$17::text,$18::text,$19::bytea,$20::bytea,$21::bytea,$22::text,\
      $23::bytea,$24::bytea,$25::text,$26::text,$27::text,$28::text,$29::text,\
      $30::text,$31::text,$32::bytea,$33::bytea,$34::bytea,$35::boolean,$36::boolean,\
      $37::boolean,$38::boolean,$39::boolean,$40::bytea,$41::text,$42::bigint,\
      $43::bigint,$44::bigint,$45::bigint,$46::bigint,$47::bigint,$48::bytea,\
      $49::text,$50::bigint,$51::bigint,$52::bigint,$53::bigint,$54::bigint,\
      $55::bigint,$56::bytea,$57::bytea,$58::text,$59::text,$60::bigint,$61::text,\
      $62::bigint,$63::bytea,$64::bytea,$65::bytea,$66::bytea,$67::boolean,\
      $68::text,$69::bytea,$70::bytea,$71::bytea,$72::text,$73::bytea)";
const REGISTRY_V1_STAGE_PROJECT_SQL: &str = "\
    SELECT control.project_registry_stage_project_v1(\
      $1::smallint,$2::text,$3::text,$4::text,$5::bytea,$6::bytea,$7::boolean,\
      $8::boolean,$9::boolean,$10::boolean,$11::boolean,$12::smallint,$13::text,\
      $14::text,$15::text,$16::text,$17::text::numeric,$18::text,$19::text,\
      $20::bytea,$21::bytea,$22::bytea)";
const REGISTRY_V1_FINALIZE_SQL: &str = "\
    SELECT control.project_registry_finalize_v1(\
      $1::smallint,$2::text,$3::text,$4::bigint,$5::text,$6::bigint,$7::bigint,\
      $8::bigint,$9::bigint,$10::bigint,$11::bigint,$12::bytea,$13::text,\
      $14::bigint,$15::bigint,$16::bigint,$17::bigint,$18::bigint,$19::bigint,\
      $20::bytea,$21::bytea,$22::bytea,$23::bytea,$24::boolean,$25::boolean,\
      $26::bigint,$27::bigint)";

#[derive(Clone)]
struct RegistryPersistenceFixture {
    database_identity_digest: ContentDigest,
    schema_version: u16,
    manifest_digest: ContentDigest,
    transaction_digest: ContentDigest,
    receipt_digest: ContentDigest,
    daemon_authority: StoreAuthorityHead,
}

struct RegistryMixedFixture {
    v4_command: ProjectRegistryCommand,
    v4_plan: RegistryCommandPlan,
    v4_state: VerifiedRegistryState,
    v4_persistence: RegistryPersistenceFixture,
    v5_command: ProjectRegistryCommand,
    v5_plan: RegistryCommandPlan,
    final_state: VerifiedRegistryState,
    v5_persistence: RegistryPersistenceFixture,
}

fn registry_fixture_string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn registry_fixture_object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn registry_checkpoint_value(checkpoint: &RegistryCheckpoint) -> CanonicalValue {
    registry_fixture_object(vec![
        ("runtime", registry_fixture_string("LIVE")),
        (
            "ordinal",
            registry_fixture_string(checkpoint.command_ordinal().to_string()),
        ),
        (
            "observation_count",
            registry_fixture_string(checkpoint.observation_count().to_string()),
        ),
        (
            "project_count",
            registry_fixture_string(checkpoint.project_count().to_string()),
        ),
        (
            "command_count",
            registry_fixture_string(checkpoint.command_count().to_string()),
        ),
        (
            "reservation_count",
            registry_fixture_string(checkpoint.reservation_count().to_string()),
        ),
        (
            "retained_bytes",
            registry_fixture_string(checkpoint.retained_bytes().to_string()),
        ),
        (
            "digest",
            registry_fixture_string(checkpoint.checkpoint_digest().as_str()),
        ),
    ])
}

fn registry_daemon_authority_value(authority: &StoreAuthorityHead) -> CanonicalValue {
    registry_fixture_object(vec![
        ("runtime", registry_fixture_string("LIVE")),
        (
            "daemon_instance_id",
            registry_fixture_string(authority.daemon_instance_id().as_str()),
        ),
        (
            "daemon_epoch",
            registry_fixture_string(authority.daemon_epoch().get().to_string()),
        ),
        (
            "admission",
            registry_fixture_string(authority.admission().as_str()),
        ),
        (
            "revision",
            registry_fixture_string(authority.revision().get().to_string()),
        ),
        (
            "observation_digest",
            registry_fixture_string(authority.observation_digest().as_str()),
        ),
        (
            "head_digest",
            registry_fixture_string(authority.head_digest().as_str()),
        ),
    ])
}

fn registry_fixture_hash(schema: &str, value: &CanonicalValue) -> ContentDigest {
    let domain = HashDomain::new(schema, "1").expect("TASK075_REGISTRY_HASH_DOMAIN");
    let digest = canonical_sha256(&domain, value).expect("TASK075_REGISTRY_HASH");
    ContentDigest::from_sha256(digest.to_hex()).expect("TASK075_REGISTRY_DIGEST")
}

// The transaction and receipt hashes intentionally share one fixture builder
// so their common Registry state cannot silently diverge in a replay vector.
#[allow(clippy::too_many_lines)]
fn registry_persistence_fixture(
    target: &MigrationTarget,
    command: &ProjectRegistryCommand,
    project_id: &ProjectId,
    plan: &RegistryCommandPlan,
    schema_version: u16,
    manifest: &str,
) -> RegistryPersistenceFixture {
    let daemon_authority = live_authority('a', 'b');
    let transaction_digest = registry_fixture_hash(
        "lattice.postgres-project-registry.transaction",
        &registry_fixture_object(vec![
            (
                "command_id",
                registry_fixture_string(command.command_id().as_str()),
            ),
            ("project_id", registry_fixture_string(project_id.as_str())),
            (
                "request_digest",
                registry_fixture_string(plan.receipt().request_digest().as_str()),
            ),
            (
                "result_digest",
                registry_fixture_string(plan.receipt().result_digest().as_str()),
            ),
            (
                "record_set_digest",
                registry_fixture_string(plan.record_set().record_set_digest().as_str()),
            ),
            (
                "base_checkpoint",
                registry_checkpoint_value(plan.base_checkpoint()),
            ),
            (
                "result_checkpoint",
                registry_checkpoint_value(plan.result_checkpoint()),
            ),
            (
                "daemon_authority",
                registry_daemon_authority_value(&daemon_authority),
            ),
        ]),
    );
    let database_identity_digest =
        ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
            .expect("TASK075_REGISTRY_DATABASE_IDENTITY");
    let manifest_digest =
        ContentDigest::from_sha256(manifest).expect("TASK075_REGISTRY_MANIFEST_DIGEST");
    let receipt_digest = registry_fixture_hash(
        "lattice.postgres-project-registry.receipt",
        &registry_fixture_object(vec![
            (
                "producer_id",
                registry_fixture_string("lattice-postgres-store"),
            ),
            ("producer_version", registry_fixture_string("1.4")),
            ("runtime", registry_fixture_string("LIVE")),
            ("durability", registry_fixture_string("DURABLE_POSTGRES")),
            (
                "registry_catalog",
                registry_fixture_string("PROJECT_REGISTRY_V1"),
            ),
            (
                "command_id",
                registry_fixture_string(command.command_id().as_str()),
            ),
            ("project_id", registry_fixture_string(project_id.as_str())),
            (
                "request_digest",
                registry_fixture_string(plan.receipt().request_digest().as_str()),
            ),
            (
                "result_digest",
                registry_fixture_string(plan.receipt().result_digest().as_str()),
            ),
            (
                "record_set_digest",
                registry_fixture_string(plan.record_set().record_set_digest().as_str()),
            ),
            (
                "base_checkpoint",
                registry_checkpoint_value(plan.base_checkpoint()),
            ),
            (
                "result_checkpoint",
                registry_checkpoint_value(plan.result_checkpoint()),
            ),
            (
                "daemon_authority",
                registry_daemon_authority_value(&daemon_authority),
            ),
            (
                "database_identity_digest",
                registry_fixture_string(database_identity_digest.as_str()),
            ),
            (
                "schema_version",
                registry_fixture_string(schema_version.to_string()),
            ),
            (
                "manifest_digest",
                registry_fixture_string(manifest_digest.as_str()),
            ),
            (
                "transaction_digest",
                registry_fixture_string(transaction_digest.as_str()),
            ),
        ]),
    );
    RegistryPersistenceFixture {
        database_identity_digest,
        schema_version,
        manifest_digest,
        transaction_digest,
        receipt_digest,
        daemon_authority,
    }
}

fn task075_registry_mixed_fixture(target: &MigrationTarget) -> RegistryMixedFixture {
    let vacant =
        VerifiedRegistryState::vacant(RuntimeKind::Live).expect("TASK075_REGISTRY_VACANT_STATE");
    let v4_project = ProjectId::new("task075-registry-v4").expect("TASK075_REGISTRY_V4_PROJECT");
    let v4_command = registry_registration_with(
        "task075-registry-v4-command",
        v4_project.as_str(),
        registry_observation_fixture("C:/lattice/task075-registry-v4", ['1', '2', '3', '4']),
    );
    let v4_plan = plan_command(&vacant, v4_command.clone()).expect("TASK075_REGISTRY_V4_PLAN");
    let v4_applied = apply_command_plan(&vacant, &v4_plan).expect("TASK075_REGISTRY_V4_APPLY");
    let v4_state = v4_applied.state().clone();
    let v4_persistence = registry_persistence_fixture(
        target,
        &v4_command,
        &v4_project,
        &v4_plan,
        4,
        REGISTRY_V4_MANIFEST_SHA256,
    );

    let v5_project = ProjectId::new("task075-registry-v5").expect("TASK075_REGISTRY_V5_PROJECT");
    let v5_command = registry_registration_with(
        "task075-registry-v5-command",
        v5_project.as_str(),
        registry_observation_fixture("C:/lattice/task075-registry-v5", ['5', '6', '7', '8']),
    );
    let v5_plan = plan_command(&v4_state, v5_command.clone()).expect("TASK075_REGISTRY_V5_PLAN");
    let v5_applied = apply_command_plan(&v4_state, &v5_plan).expect("TASK075_REGISTRY_V5_APPLY");
    let final_state = v5_applied.state().clone();
    let v5_persistence = registry_persistence_fixture(
        target,
        &v5_command,
        &v5_project,
        &v5_plan,
        5,
        CURRENT_V5_MANIFEST_SHA256,
    );
    RegistryMixedFixture {
        v4_command,
        v4_plan,
        v4_state,
        v4_persistence,
        v5_command,
        v5_plan,
        final_state,
        v5_persistence,
    }
}

fn registry_sql_i64(value: u64) -> i64 {
    i64::try_from(value).expect("TASK075_REGISTRY_I64")
}

struct RegistryCheckpointSql {
    runtime: String,
    ordinal: i64,
    observations: i64,
    projects: i64,
    commands: i64,
    reservations: i64,
    retained_bytes: i64,
    digest: Vec<u8>,
}

fn registry_checkpoint_sql(checkpoint: &RegistryCheckpoint) -> RegistryCheckpointSql {
    RegistryCheckpointSql {
        runtime: "LIVE".to_owned(),
        ordinal: registry_sql_i64(checkpoint.command_ordinal()),
        observations: registry_sql_i64(checkpoint.observation_count()),
        projects: registry_sql_i64(checkpoint.project_count()),
        commands: registry_sql_i64(checkpoint.command_count()),
        reservations: registry_sql_i64(checkpoint.reservation_count()),
        retained_bytes: registry_sql_i64(checkpoint.retained_bytes()),
        digest: live_digest_value_bytes(checkpoint.checkpoint_digest()),
    }
}

fn registry_drift_flags(drift: &[IdentityDrift]) -> [bool; 5] {
    [
        IdentityDrift::CanonicalRoot,
        IdentityDrift::Repository,
        IdentityDrift::File,
        IdentityDrift::PrimaryRefName,
        IdentityDrift::PrimaryRefStorage,
    ]
    .map(|dimension| drift.contains(&dimension))
}

fn registry_query_one_boxed(
    transaction: &mut postgres::Transaction<'_>,
    sql: &str,
    values: &[Box<dyn ToSql + Sync>],
) -> postgres::Row {
    let params = values
        .iter()
        .map(|value| &**value as &(dyn ToSql + Sync))
        .collect::<Vec<_>>();
    transaction
        .query_one(sql, &params)
        .expect("TASK075_REGISTRY_V1_QUERY")
}

#[allow(clippy::too_many_lines)]
fn seed_exact_v4_registry_registration(
    config: &LiveConfig,
    target: &MigrationTarget,
    plan: &RegistryCommandPlan,
    applied_state: &VerifiedRegistryState,
    durable: &RegistryPersistenceFixture,
) {
    assert_eq!(
        TASK075_REGISTRY_V1_STAGE_SURFACE,
        "PROJECT_REGISTRY_STAGE_COMMAND_V1"
    );
    assert!(!plan.is_replay(), "TASK075_REGISTRY_V4_PLAN_REPLAY");
    assert!(plan.receipt().before().is_none());
    assert!(matches!(
        plan.receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    let (project_id, project_class, observation) = match plan.record_set().command() {
        ProjectRegistryCommand::Register {
            project_id,
            project_class,
            observation,
            ..
        } => (project_id, *project_class, observation),
        _ => panic!("TASK075_REGISTRY_V4_COMMAND_KIND"),
    };
    let (replacement_id, projection) = plan
        .record_set()
        .project_replacement()
        .expect("TASK075_REGISTRY_V4_PROJECT_REPLACEMENT");
    assert_eq!(replacement_id, project_id);
    let base = registry_checkpoint_sql(plan.base_checkpoint());
    let result = registry_checkpoint_sql(plan.result_checkpoint());
    let drift = registry_drift_flags(plan.receipt().drift());
    let authority_receipt = plan
        .receipt()
        .authority()
        .expect("TASK075_REGISTRY_V4_AUTHORITY");
    let semantic_after = plan.receipt().after().expect("TASK075_REGISTRY_V4_AFTER");
    let daemon = &durable.daemon_authority;

    let mut runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = runtime
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("TASK075_REGISTRY_V4_TRANSACTION");
    transaction
        .batch_execute(REGISTRY_V1_WRITE_SETTINGS)
        .expect("TASK075_REGISTRY_V4_SETTINGS");
    let prepare_values: Vec<Box<dyn ToSql + Sync>> = vec![
        Box::new(4_i16),
        Box::new(REGISTRY_V4_MANIFEST_SHA256.to_owned()),
        Box::new(plan.record_set().command().command_id().as_str().to_owned()),
        Box::new(live_digest_value_bytes(plan.receipt().request_digest())),
        Box::new("LIVE".to_owned()),
        Box::new(daemon.daemon_instance_id().as_str().to_owned()),
        Box::new(registry_sql_i64(daemon.daemon_epoch().get())),
        Box::new(daemon.admission().as_str().to_owned()),
        Box::new(registry_sql_i64(daemon.revision().get())),
        Box::new(live_digest_value_bytes(daemon.observation_digest())),
        Box::new(live_digest_value_bytes(daemon.head_digest())),
        Box::new(base.digest.clone()),
    ];
    let prepare =
        registry_query_one_boxed(&mut transaction, REGISTRY_V1_PREPARE_SQL, &prepare_values);
    assert_eq!(prepare.get::<_, String>(0), "NEW");

    let command_values: Vec<Box<dyn ToSql + Sync>> = vec![
        Box::new(4_i16),
        Box::new(REGISTRY_V4_MANIFEST_SHA256.to_owned()),
        Box::new(base.ordinal + 1),
        Box::new(plan.record_set().command().command_id().as_str().to_owned()),
        Box::new("REGISTER".to_owned()),
        Box::new(project_id.as_str().to_owned()),
        Box::new(Some(project_class.as_str().to_owned())),
        Box::new(Some(live_digest_value_bytes(observation.digest()))),
        Box::new(false),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<Vec<u8>>),
        Box::new(None::<Vec<u8>>),
        Box::new(None::<Vec<u8>>),
        Box::new(None::<String>),
        Box::new(None::<Vec<u8>>),
        Box::new(live_digest_value_bytes(plan.receipt().request_digest())),
        Box::new("APPLIED".to_owned()),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<String>),
        Box::new(None::<Vec<u8>>),
        Box::new(Some(live_digest_value_bytes(
            semantic_after.receipt_digest(),
        ))),
        Box::new(Some(live_digest_value_bytes(
            authority_receipt.receipt_digest(),
        ))),
        Box::new(drift[0]),
        Box::new(drift[1]),
        Box::new(drift[2]),
        Box::new(drift[3]),
        Box::new(drift[4]),
        Box::new(live_digest_value_bytes(plan.receipt().result_digest())),
        Box::new(base.runtime.clone()),
        Box::new(base.ordinal),
        Box::new(base.observations),
        Box::new(base.projects),
        Box::new(base.commands),
        Box::new(base.reservations),
        Box::new(base.retained_bytes),
        Box::new(base.digest.clone()),
        Box::new(result.runtime.clone()),
        Box::new(result.ordinal),
        Box::new(result.observations),
        Box::new(result.projects),
        Box::new(result.commands),
        Box::new(result.reservations),
        Box::new(result.retained_bytes),
        Box::new(result.digest.clone()),
        Box::new(live_digest_value_bytes(
            plan.record_set().record_set_digest(),
        )),
        Box::new("LIVE".to_owned()),
        Box::new(daemon.daemon_instance_id().as_str().to_owned()),
        Box::new(registry_sql_i64(daemon.daemon_epoch().get())),
        Box::new(daemon.admission().as_str().to_owned()),
        Box::new(registry_sql_i64(daemon.revision().get())),
        Box::new(live_digest_value_bytes(daemon.observation_digest())),
        Box::new(live_digest_value_bytes(daemon.head_digest())),
        Box::new(live_digest_value_bytes(&durable.transaction_digest)),
        Box::new(live_digest_value_bytes(&durable.receipt_digest)),
        Box::new(true),
        Box::new(Some(observation.canonical_root().to_owned())),
        Box::new(Some(live_digest_value_bytes(
            observation.canonical_root_identity_digest(),
        ))),
        Box::new(Some(live_digest_value_bytes(
            observation.repository_identity_digest(),
        ))),
        Box::new(Some(live_digest_value_bytes(
            observation.file_identity_digest(),
        ))),
        Box::new(Some(observation.primary_branch().reference().to_owned())),
        Box::new(Some(live_digest_value_bytes(
            observation.primary_branch().storage_identity_digest(),
        ))),
    ];
    assert_eq!(command_values.len(), 73);
    let staged = registry_query_one_boxed(
        &mut transaction,
        REGISTRY_V1_STAGE_COMMAND_SQL,
        &command_values,
    );
    assert_eq!(staged.get::<_, String>(0), "STAGED");

    let project_authority = projection.authority();
    let project_drift = registry_drift_flags(projection.drift());
    let project_values: Vec<Box<dyn ToSql + Sync>> = vec![
        Box::new(4_i16),
        Box::new(REGISTRY_V4_MANIFEST_SHA256.to_owned()),
        Box::new(project_id.as_str().to_owned()),
        Box::new(projection.project_class().as_str().to_owned()),
        Box::new(live_digest_value_bytes(projection.observation().digest())),
        Box::new(
            projection
                .pending_observation()
                .map(|value| live_digest_value_bytes(value.digest())),
        ),
        Box::new(project_drift[0]),
        Box::new(project_drift[1]),
        Box::new(project_drift[2]),
        Box::new(project_drift[3]),
        Box::new(project_drift[4]),
        Box::new(i16::try_from(project_authority.version()).expect("TASK075_REGISTRY_VERSION")),
        Box::new(project_authority.producer_id().to_owned()),
        Box::new(project_authority.producer_version().to_owned()),
        Box::new("LIVE".to_owned()),
        Box::new(project_authority.project_snapshot_id().as_str().to_owned()),
        Box::new(project_authority.registry_revision().to_string()),
        Box::new(project_authority.lifecycle().as_str().to_owned()),
        Box::new(project_authority.primary_branch().reference().to_owned()),
        Box::new(live_digest_value_bytes(
            project_authority.primary_branch().storage_identity_digest(),
        )),
        Box::new(live_digest_value_bytes(
            project_authority.observation_digest(),
        )),
        Box::new(live_digest_value_bytes(project_authority.receipt_digest())),
    ];
    assert_eq!(project_values.len(), 22);
    let project_staged = registry_query_one_boxed(
        &mut transaction,
        REGISTRY_V1_STAGE_PROJECT_SQL,
        &project_values,
    );
    assert_eq!(project_staged.get::<_, String>(0), "STAGED");

    let inserted_reservations = applied_state
        .reservations()
        .iter()
        .filter(|row| row.project_id() == project_id)
        .count();
    let finalize_values: Vec<Box<dyn ToSql + Sync>> = vec![
        Box::new(4_i16),
        Box::new(REGISTRY_V4_MANIFEST_SHA256.to_owned()),
        Box::new(plan.record_set().command().command_id().as_str().to_owned()),
        Box::new(result.ordinal),
        Box::new(base.runtime),
        Box::new(base.ordinal),
        Box::new(base.observations),
        Box::new(base.projects),
        Box::new(base.commands),
        Box::new(base.reservations),
        Box::new(base.retained_bytes),
        Box::new(base.digest),
        Box::new(result.runtime),
        Box::new(result.ordinal),
        Box::new(result.observations),
        Box::new(result.projects),
        Box::new(result.commands),
        Box::new(result.reservations),
        Box::new(result.retained_bytes),
        Box::new(result.digest),
        Box::new(live_digest_value_bytes(
            plan.record_set().record_set_digest(),
        )),
        Box::new(live_digest_value_bytes(&durable.transaction_digest)),
        Box::new(live_digest_value_bytes(&durable.receipt_digest)),
        Box::new(true),
        Box::new(true),
        Box::new(0_i64),
        Box::new(i64::try_from(inserted_reservations).expect("TASK075_REGISTRY_RESERVATIONS")),
    ];
    assert_eq!(finalize_values.len(), 27);
    let finalized =
        registry_query_one_boxed(&mut transaction, REGISTRY_V1_FINALIZE_SQL, &finalize_values);
    assert_eq!(finalized.get::<_, String>(0), "FINALIZED");
    transaction.commit().expect("TASK075_REGISTRY_V4_COMMIT");
}

fn assert_registry_execution_exact(
    execution: &PostgresProjectRegistryExecution,
    plan: &RegistryCommandPlan,
    expected: &RegistryPersistenceFixture,
    exact_retry: bool,
) {
    assert_eq!(execution.semantic_receipt(), plan.receipt());
    assert_eq!(execution.result_checkpoint(), plan.result_checkpoint());
    assert_eq!(execution.is_exact_retry(), exact_retry);
    let receipt = execution.persistence_receipt();
    assert_eq!(
        receipt.command_id(),
        plan.record_set().command().command_id()
    );
    assert_eq!(receipt.request_digest(), plan.receipt().request_digest());
    assert_eq!(receipt.result_digest(), plan.receipt().result_digest());
    assert_eq!(
        receipt.record_set_digest(),
        plan.record_set().record_set_digest()
    );
    assert_eq!(receipt.base_checkpoint(), plan.base_checkpoint());
    assert_eq!(receipt.result_checkpoint(), plan.result_checkpoint());
    assert_eq!(receipt.daemon_authority(), &expected.daemon_authority);
    assert_eq!(
        receipt.persistence().database_identity_digest(),
        &expected.database_identity_digest
    );
    assert_eq!(
        receipt.persistence().schema_version(),
        expected.schema_version
    );
    assert_eq!(
        receipt.persistence().manifest_digest(),
        &expected.manifest_digest
    );
    assert_eq!(receipt.transaction_digest(), &expected.transaction_digest);
    assert_eq!(receipt.receipt_digest(), &expected.receipt_digest);
}

#[allow(clippy::too_many_lines)]
fn prove_exact_nonempty_v4_registry_upgrade_and_mixed_replay(
    config: &LiveConfig,
    admin: &mut Client,
) {
    let target = provision_database(config, admin, "reg_mixed", true);
    install_exact_v4(config, &target);
    set_live_admission(config, &target, true);
    let fixture = task075_registry_mixed_fixture(&target);
    seed_exact_v4_registry_registration(
        config,
        &target,
        &fixture.v4_plan,
        &fixture.v4_state,
        &fixture.v4_persistence,
    );
    assert_eq!(project_registry_counts(config, &target), [1, 1, 1, 1, 3]);
    set_live_admission(config, &target, false);

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
    let evidence = must_setup(verify_postgres_schema(
        &mut migrator,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(evidence.schema_version(), 5);
    assert_eq!(
        evidence.manifest_sha256().as_str(),
        CURRENT_V5_MANIFEST_SHA256
    );
    drop(migrator);

    let mut profile = config.connect(target.database_name(), "task075-registry-v4-backfill");
    let backfilled = profile
        .query_one(
            "SELECT persistence_schema_version, btrim(persistence_manifest_sha256)::text, \
                    encode(transaction_digest, 'hex'), encode(persistence_receipt_digest, 'hex') \
               FROM ONLY control.project_registry_commands WHERE ordinal = 1",
            &[],
        )
        .expect("TASK075_REGISTRY_V4_BACKFILL_READ");
    assert_eq!(backfilled.get::<_, i16>(0), 4);
    assert_eq!(backfilled.get::<_, String>(1), REGISTRY_V4_MANIFEST_SHA256);
    assert_eq!(
        backfilled.get::<_, String>(2),
        fixture.v4_persistence.transaction_digest.as_str()
    );
    assert_eq!(
        backfilled.get::<_, String>(3),
        fixture.v4_persistence.receipt_digest.as_str()
    );
    drop(profile);

    set_live_admission(config, &target, true);
    let mut registry = new_live_project_registry(config, &target);
    let v4_replay = registry
        .execute(fixture.v4_command.clone(), live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("TASK075_REGISTRY_V4_REPLAY {}", error.code()));
    assert_registry_execution_exact(&v4_replay, &fixture.v4_plan, &fixture.v4_persistence, true);
    let v5_new = registry
        .execute(fixture.v5_command.clone(), live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("TASK075_REGISTRY_V5_NEW {}", error.code()));
    assert_registry_execution_exact(&v5_new, &fixture.v5_plan, &fixture.v5_persistence, false);
    let loaded = registry
        .load()
        .unwrap_or_else(|error| panic!("TASK075_REGISTRY_MIXED_LOAD {}", error.code()));
    assert_eq!(loaded.state(), &fixture.final_state);
    assert_eq!(
        loaded.retained_checkpoint(),
        fixture.final_state.checkpoint()
    );
    assert_eq!(loaded.persistence().schema_version(), 5);
    let mut profiles = config.connect(target.database_name(), "task075-registry-mixed-profiles");
    let rows = profiles
        .query(
            "SELECT ordinal, persistence_schema_version, \
                    btrim(persistence_manifest_sha256)::text \
               FROM ONLY control.project_registry_commands ORDER BY ordinal",
            &[],
        )
        .expect("TASK075_REGISTRY_MIXED_PROFILES");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (
            rows[0].get::<_, i64>(0),
            rows[0].get::<_, i16>(1),
            rows[0].get::<_, String>(2)
        ),
        (1, 4, REGISTRY_V4_MANIFEST_SHA256.to_owned())
    );
    assert_eq!(
        (
            rows[1].get::<_, i64>(0),
            rows[1].get::<_, i16>(1),
            rows[1].get::<_, String>(2)
        ),
        (2, 5, CURRENT_V5_MANIFEST_SHA256.to_owned())
    );
    set_live_admission(config, &target, false);
}

fn prove_task075_registry_mixed_restart(config: &LiveConfig) {
    let target = config.target("reg_mixed");
    let fixture = task075_registry_mixed_fixture(&target);
    let mut registry = new_live_project_registry(config, &target);
    let loaded = registry
        .load()
        .unwrap_or_else(|error| panic!("TASK075_REGISTRY_RESTART_LOAD {}", error.code()));
    assert_eq!(loaded.state(), &fixture.final_state);
    assert_eq!(
        loaded.retained_checkpoint(),
        fixture.final_state.checkpoint()
    );
    let v4 = registry
        .execute(fixture.v4_command, live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("TASK075_REGISTRY_RESTART_V4 {}", error.code()));
    assert_registry_execution_exact(&v4, &fixture.v4_plan, &fixture.v4_persistence, true);
    let v5 = registry
        .execute(fixture.v5_command, live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("TASK075_REGISTRY_RESTART_V5 {}", error.code()));
    assert_registry_execution_exact(&v5, &fixture.v5_plan, &fixture.v5_persistence, true);
}

#[derive(Debug, Eq, PartialEq)]
struct RegistryCorruptionCommand {
    ordinal: i64,
    schema_version: Option<i16>,
    manifest: Option<String>,
    transaction_digest: String,
    receipt_digest: String,
}

#[derive(Debug, Eq, PartialEq)]
struct RegistryCorruptionSnapshot {
    state: (i64, i64, i64, i64, i64, String, Option<String>),
    commands: Vec<RegistryCorruptionCommand>,
}

fn task075_registry_corruption_snapshot(
    config: &LiveConfig,
    target: &MigrationTarget,
) -> RegistryCorruptionSnapshot {
    let mut fixture = config.connect(
        target.database_name(),
        "task075-registry-corruption-snapshot",
    );
    let state = fixture
        .query_one(
            "SELECT command_ordinal, observation_count, project_count, command_count, \
                    reservation_count, encode(checkpoint_digest, 'hex'), stage_command_id \
               FROM ONLY control.project_registry_state WHERE singleton = true",
            &[],
        )
        .expect("TASK075_REGISTRY_CORRUPTION_STATE");
    let commands = fixture
        .query(
            "SELECT ordinal, persistence_schema_version, \
                    btrim(persistence_manifest_sha256)::text, \
                    encode(transaction_digest, 'hex'), \
                    encode(persistence_receipt_digest, 'hex') \
               FROM ONLY control.project_registry_commands ORDER BY ordinal",
            &[],
        )
        .expect("TASK075_REGISTRY_CORRUPTION_COMMANDS")
        .into_iter()
        .map(|row| RegistryCorruptionCommand {
            ordinal: row.get(0),
            schema_version: row.get(1),
            manifest: row.get(2),
            transaction_digest: row.get(3),
            receipt_digest: row.get(4),
        })
        .collect();
    RegistryCorruptionSnapshot {
        state: (
            state.get(0),
            state.get(1),
            state.get(2),
            state.get(3),
            state.get(4),
            state.get(5),
            state.get(6),
        ),
        commands,
    }
}

fn task075_seed_v5_registry(
    config: &LiveConfig,
    admin: &mut Client,
    tag: &str,
) -> (MigrationTarget, PostgresProjectRegistry) {
    let target = migrated_database(config, admin, tag);
    set_live_admission(config, &target, true);
    let mut registry = new_live_project_registry(config, &target);
    let seeded = registry
        .execute(
            registry_registration_with(
                &format!("task075-{tag}-seed"),
                &format!("task075-{tag}"),
                registry_observation_fixture(
                    &format!("C:/lattice/task075-{tag}"),
                    ['9', 'a', 'b', 'c'],
                ),
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("TASK075_REGISTRY_CORRUPTION_SEED {}", error.code()));
    assert!(!seeded.is_exact_retry());
    (target, registry)
}

fn assert_task075_registry_corruption_is_terminal(
    config: &LiveConfig,
    target: &MigrationTarget,
    registry: &mut PostgresProjectRegistry,
    tag: &str,
    marker: &'static str,
) {
    let corrupted = task075_registry_corruption_snapshot(config, target);
    let failure = registry
        .execute(
            registry_registration_with(
                &format!("task075-{tag}-after-corruption"),
                &format!("task075-{tag}-after-corruption"),
                registry_observation_fixture(
                    &format!("C:/lattice/task075-{tag}-after-corruption"),
                    ['d', 'e', 'f', '1'],
                ),
            ),
            live_authority('a', 'b'),
        )
        .expect_err(marker);
    assert_eq!(
        failure.kind(),
        PostgresProjectRegistryErrorKind::RetainedRowCorrupt,
        "{marker}"
    );
    assert_eq!(
        task075_registry_corruption_snapshot(config, target),
        corrupted,
        "{marker}_AUTOMATIC_REPAIR"
    );
}

#[allow(clippy::too_many_lines)]
fn prove_task075_registry_provenance_corruption(config: &LiveConfig, admin: &mut Client) {
    let (omission_target, mut omission_registry) =
        task075_seed_v5_registry(config, admin, "reg_prov_null");
    let mut omission = config.connect(
        omission_target.database_name(),
        "task075-registry-provenance-omission",
    );
    omission
        .batch_execute(
            "ALTER TABLE control.project_registry_commands \
                 ALTER COLUMN persistence_schema_version DROP NOT NULL; \
             UPDATE ONLY control.project_registry_commands \
                 SET persistence_schema_version = NULL WHERE ordinal = 1",
        )
        .expect("PROVENANCE_OMISSION_FIXTURE");
    drop(omission);
    assert_task075_registry_corruption_is_terminal(
        config,
        &omission_target,
        &mut omission_registry,
        "reg-prov-null",
        "PROVENANCE_OMISSION",
    );

    let (mutation_target, mut mutation_registry) =
        task075_seed_v5_registry(config, admin, "reg_prov_mut");
    let mut mutation = config.connect(
        mutation_target.database_name(),
        "task075-registry-provenance-mutation",
    );
    assert_eq!(
        mutation
            .execute(
                "UPDATE ONLY control.project_registry_commands \
                    SET persistence_manifest_sha256 = pg_catalog.repeat('1', 64) \
                  WHERE ordinal = 1",
                &[],
            )
            .expect("PROVENANCE_MUTATION_FIXTURE"),
        1
    );
    drop(mutation);
    assert_task075_registry_corruption_is_terminal(
        config,
        &mutation_target,
        &mut mutation_registry,
        "reg-prov-mut",
        "PROVENANCE_MUTATION",
    );

    let (cross_v4_target, mut cross_v4_registry) =
        task075_seed_v5_registry(config, admin, "reg_cross_four");
    let mut cross_v4 = config.connect(
        cross_v4_target.database_name(),
        "task075-registry-cross-v4-current",
    );
    assert_eq!(
        cross_v4
            .execute(
                "UPDATE ONLY control.project_registry_commands \
                    SET persistence_schema_version = 4 \
                  WHERE ordinal = 1",
                &[],
            )
            .expect("PROVENANCE_CROSS_PAIR_V4_CURRENT_FIXTURE"),
        1
    );
    drop(cross_v4);
    assert_task075_registry_corruption_is_terminal(
        config,
        &cross_v4_target,
        &mut cross_v4_registry,
        "reg-cross-v4",
        "PROVENANCE_CROSS_PAIR_V4_CURRENT",
    );

    let (cross_v5_target, mut cross_v5_registry) =
        task075_seed_v5_registry(config, admin, "reg_cross_five");
    let mut cross_v5 = config.connect(
        cross_v5_target.database_name(),
        "task075-registry-cross-v5-v4",
    );
    assert_eq!(
        cross_v5
            .execute(
                "UPDATE ONLY control.project_registry_commands \
                    SET persistence_manifest_sha256 = $1::text \
                  WHERE ordinal = 1",
                &[&REGISTRY_V4_MANIFEST_SHA256],
            )
            .expect("PROVENANCE_CROSS_PAIR_V5_V4_FIXTURE"),
        1
    );
    drop(cross_v5);
    assert_task075_registry_corruption_is_terminal(
        config,
        &cross_v5_target,
        &mut cross_v5_registry,
        "reg-cross-v5",
        "PROVENANCE_CROSS_PAIR_V5_V4",
    );

    let current_target = provision_database(config, admin, "reg_curr_sub", true);
    install_exact_v4(config, &current_target);
    set_live_admission(config, &current_target, true);
    let current_fixture = task075_registry_mixed_fixture(&current_target);
    seed_exact_v4_registry_registration(
        config,
        &current_target,
        &current_fixture.v4_plan,
        &current_fixture.v4_state,
        &current_fixture.v4_persistence,
    );
    set_live_admission(config, &current_target, false);
    let mut current_migrator = config.role_client(
        current_target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        must_setup(apply_migrations(&mut current_migrator, &current_target)),
        MigrationApplyOutcome::Applied {
            executable_count: 1
        }
    );
    drop(current_migrator);
    set_live_admission(config, &current_target, true);
    let mut current_registry = new_live_project_registry(config, &current_target);
    let mut current_substitution = config.connect(
        current_target.database_name(),
        "task075-registry-current-profile-substitution",
    );
    assert_eq!(
        current_substitution
            .execute(
                "UPDATE ONLY control.project_registry_commands \
                    SET persistence_schema_version = 5, \
                        persistence_manifest_sha256 = $1::text \
                  WHERE ordinal = 1",
                &[&CURRENT_V5_MANIFEST_SHA256],
            )
            .expect("CURRENT_PROFILE_SUBSTITUTION_FIXTURE"),
        1
    );
    drop(current_substitution);
    assert_task075_registry_corruption_is_terminal(
        config,
        &current_target,
        &mut current_registry,
        "reg-curr-sub",
        "CURRENT_PROFILE_SUBSTITUTION",
    );

    let (prefix_target, mut prefix_registry) =
        task075_seed_v5_registry(config, admin, "reg_prefix");
    let denial = prefix_registry
        .execute(
            registry_registration_with(
                "task075-reg-prefix-denied-tail",
                "task075-reg-prefix-denied-project",
                registry_observation_fixture("C:/lattice/task075-reg_prefix", ['9', 'a', 'b', 'c']),
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("COHERENT_PREFIX_ROLLBACK_SETUP {}", error.code()));
    assert!(matches!(
        denial.semantic_receipt().outcome(),
        RegistryCommandOutcome::Denied(_)
    ));
    let mut prefix = config.connect(
        prefix_target.database_name(),
        "task075-registry-coherent-prefix-rollback",
    );
    assert_eq!(
        prefix
            .execute(
                "DELETE FROM ONLY control.project_registry_commands WHERE ordinal = 2",
                &[],
            )
            .expect("COHERENT_PREFIX_ROLLBACK_FIXTURE"),
        1
    );
    drop(prefix);
    assert_task075_registry_corruption_is_terminal(
        config,
        &prefix_target,
        &mut prefix_registry,
        "reg-prefix",
        "COHERENT_PREFIX_ROLLBACK",
    );
}

#[allow(clippy::too_many_lines)]
fn prove_live_project_registry(config: &LiveConfig, target: &MigrationTarget) {
    set_live_admission(config, target, true);
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut registry = PostgresProjectRegistry::new(runtime, target)
        .unwrap_or_else(|error| panic!("STORE_TASK022_CONSTRUCTOR_FAILED {}", error.code()));
    let vacant = registry
        .load()
        .unwrap_or_else(|error| panic!("STORE_TASK022_VACANT_LOAD_FAILED {}", error.code()));
    assert!(
        vacant.state().is_vacant(),
        "STORE_TASK022_VACANT_STATE_INVALID"
    );
    assert_eq!(vacant.persistence().schema_version(), 5);

    let registration = registry_registration("registry-live-register", "registry-live");
    let first = registry
        .execute(registration.clone(), live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("STORE_TASK022_REGISTER_FAILED {}", error.code()));
    assert!(!first.is_exact_retry());
    assert_eq!(first.result_checkpoint().command_ordinal(), 1);
    assert!(matches!(
        first.semantic_receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    assert!(first.semantic_receipt().authority().is_some());
    assert_eq!(
        first.persistence_receipt().persistence().schema_version(),
        5
    );

    let replay = registry
        .execute(registration.clone(), live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("STORE_TASK022_REPLAY_FAILED {}", error.code()));
    assert!(replay.is_exact_retry());
    assert_eq!(replay.persistence_receipt(), first.persistence_receipt());

    let stopped_authority = live_authority_with_admission(RuntimeAdmissionMode::Stopped, 'a', 'b');
    let stopped_replay = registry
        .execute(registration, stopped_authority.clone())
        .unwrap_or_else(|error| panic!("STORE_TASK022_STOPPED_REPLAY_FAILED {}", error.code()));
    assert!(stopped_replay.is_exact_retry());
    assert_eq!(stopped_replay.semantic_receipt(), first.semantic_receipt());
    assert_eq!(
        stopped_replay.persistence_receipt(),
        first.persistence_receipt()
    );
    let stopped_changed = registry
        .execute(
            registry_registration("registry-live-register", "registry-conflict-stopped"),
            stopped_authority.clone(),
        )
        .expect_err("changed command reuse must precede stopped admission");
    assert_eq!(
        stopped_changed.kind(),
        PostgresProjectRegistryErrorKind::CommandSubstitution
    );
    let stopped_new = registry
        .execute(
            registry_registration("registry-live-stopped-new", "registry-stopped-new"),
            stopped_authority,
        )
        .expect_err("first-seen command must require active admission");
    assert_eq!(
        stopped_new.kind(),
        PostgresProjectRegistryErrorKind::AdmissionDenied
    );

    let exact_observation = registry
        .execute(
            ProjectRegistryCommand::observe(
                RegistryCommandId::new("registry-live-observe-exact").expect("registry command id"),
                ProjectId::new("registry-live").expect("registry project id"),
                first
                    .semantic_receipt()
                    .authority()
                    .expect("registered authority")
                    .head(),
                registry_observation(),
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_OBSERVE_EXACT_FAILED {}", error.code()));
    assert!(matches!(
        exact_observation.semantic_receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    assert_eq!(exact_observation.result_checkpoint().command_ordinal(), 2);
    let moved = registry_observation_fixture("D:/lattice/registry-live", ['b', '3', '4', '5']);
    let drifted = registry
        .execute(
            ProjectRegistryCommand::observe(
                RegistryCommandId::new("registry-live-observe-move").expect("registry command id"),
                ProjectId::new("registry-live").expect("registry project id"),
                first
                    .semantic_receipt()
                    .authority()
                    .expect("registered authority")
                    .head(),
                moved.clone(),
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_OBSERVE_MOVE_FAILED {}", error.code()));
    let drifted_authority = drifted
        .semantic_receipt()
        .authority()
        .expect("drifted authority")
        .clone();
    assert_eq!(
        drifted_authority.lifecycle(),
        ProjectLifecycle::ReconciliationRequired
    );
    let pending_front_run = registry
        .execute(
            registry_registration_with(
                "registry-live-pending-front-run",
                "registry-pending-front-run",
                moved.clone(),
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_PENDING_FRONT_RUN_FAILED {}", error.code()));
    assert!(matches!(
        pending_front_run.semantic_receipt().outcome(),
        RegistryCommandOutcome::Denied(_)
    ));
    assert!(pending_front_run.semantic_receipt().authority().is_none());
    let reconciled = registry
        .execute(
            ProjectRegistryCommand::reconcile(
                RegistryCommandId::new("registry-live-accept-move").expect("registry command id"),
                ProjectId::new("registry-live").expect("registry project id"),
                drifted_authority.head(),
                moved.clone(),
                ReconciliationDecision::AcceptMove,
                live_digest('6'),
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_RECONCILE_FAILED {}", error.code()));
    let active = reconciled
        .semantic_receipt()
        .authority()
        .expect("reconciled authority")
        .clone();
    assert_eq!(active.lifecycle(), ProjectLifecycle::Active);
    let suspended = registry
        .execute(
            ProjectRegistryCommand::suspend(
                RegistryCommandId::new("registry-live-suspend").expect("registry command id"),
                ProjectId::new("registry-live").expect("registry project id"),
                active.head(),
                live_digest('7'),
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_SUSPEND_FAILED {}", error.code()));
    let suspended_authority = suspended
        .semantic_receipt()
        .authority()
        .expect("suspended authority")
        .clone();
    assert_eq!(suspended_authority.lifecycle(), ProjectLifecycle::Suspended);
    let reactivated = registry
        .execute(
            ProjectRegistryCommand::reconcile(
                RegistryCommandId::new("registry-live-reactivate").expect("registry command id"),
                ProjectId::new("registry-live").expect("registry project id"),
                suspended_authority.head(),
                moved,
                ReconciliationDecision::Reactivate,
                live_digest('8'),
            ),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_REACTIVATE_FAILED {}", error.code()));
    assert_eq!(
        reactivated
            .semantic_receipt()
            .authority()
            .expect("reactivated authority")
            .lifecycle(),
        ProjectLifecycle::Active
    );

    let conflict = registry
        .execute(
            registry_registration("registry-live-register", "registry-conflict"),
            live_authority('a', 'b'),
        )
        .expect_err("changed command id reuse");
    assert_eq!(
        conflict.kind(),
        PostgresProjectRegistryErrorKind::CommandSubstitution
    );

    let denial = registry
        .execute(
            registry_registration("registry-live-duplicate", "registry-duplicate"),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_DENIAL_FAILED {}", error.code()));
    assert!(matches!(
        denial.semantic_receipt().outcome(),
        RegistryCommandOutcome::Denied(_)
    ));
    assert!(denial.semantic_receipt().authority().is_none());
    assert_eq!(denial.result_checkpoint().command_ordinal(), 8);
    let loaded = registry
        .load()
        .unwrap_or_else(|error| panic!("STORE_TASK022_POST_LOAD_FAILED {}", error.code()));
    assert_eq!(loaded.retained_checkpoint().command_ordinal(), 8);
    assert_eq!(loaded.retained_checkpoint().project_count(), 1);

    let same = registry_registration_with(
        "registry-concurrent-same",
        "registry-concurrent-same",
        registry_observation_fixture("C:/lattice/registry-concurrent-same", ['6', '7', '8', '9']),
    );
    let (same_left, same_right) = run_concurrent_registry_commands(
        config,
        target,
        same.clone(),
        same,
        "STORE_TASK022_CONCURRENT_SAME",
    );
    assert_eq!(same_left.semantic_receipt(), same_right.semantic_receipt());
    assert_eq!(
        same_left.persistence_receipt(),
        same_right.persistence_receipt()
    );
    assert_ne!(same_left.is_exact_retry(), same_right.is_exact_retry());

    let cross_observation =
        registry_observation_fixture("C:/lattice/registry-concurrent-cross", ['a', 'b', 'c', 'd']);
    let (cross_left, cross_right) = run_concurrent_registry_commands(
        config,
        target,
        registry_registration_with(
            "registry-concurrent-cross-a",
            "registry-concurrent-a",
            cross_observation.clone(),
        ),
        registry_registration_with(
            "registry-concurrent-cross-b",
            "registry-concurrent-b",
            cross_observation,
        ),
        "STORE_TASK022_CONCURRENT_CROSS",
    );
    let cross_outcomes = [
        cross_left.semantic_receipt().outcome(),
        cross_right.semantic_receipt().outcome(),
    ];
    assert_eq!(
        cross_outcomes
            .iter()
            .filter(|outcome| matches!(outcome, RegistryCommandOutcome::Applied))
            .count(),
        1
    );
    assert_eq!(
        cross_outcomes
            .iter()
            .filter(|outcome| matches!(outcome, RegistryCommandOutcome::Denied(_)))
            .count(),
        1
    );

    let (unrelated_left, unrelated_right) = run_concurrent_registry_commands(
        config,
        target,
        registry_registration_with(
            "registry-concurrent-unrelated-a",
            "registry-unrelated-a",
            registry_observation_fixture("C:/lattice/registry-unrelated-a", ['e', 'f', '1', '2']),
        ),
        registry_registration_with(
            "registry-concurrent-unrelated-b",
            "registry-unrelated-b",
            registry_observation_fixture("C:/lattice/registry-unrelated-b", ['5', '9', 'd', '6']),
        ),
        "STORE_TASK022_CONCURRENT_UNRELATED",
    );
    assert!(matches!(
        unrelated_left.semantic_receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    assert!(matches!(
        unrelated_right.semantic_receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    let concurrent = registry
        .load()
        .unwrap_or_else(|error| panic!("STORE_TASK022_CONCURRENT_LOAD_FAILED {}", error.code()));
    assert_eq!(concurrent.retained_checkpoint().command_ordinal(), 13);
    assert_eq!(concurrent.retained_checkpoint().project_count(), 5);
    assert_eq!(concurrent.retained_checkpoint().reservation_count(), 15);
    set_live_admission(config, target, false);
}

fn run_concurrent_registry_commands(
    config: &LiveConfig,
    target: &MigrationTarget,
    left_command: ProjectRegistryCommand,
    right_command: ProjectRegistryCommand,
    failure_code: &'static str,
) -> (
    PostgresProjectRegistryExecution,
    PostgresProjectRegistryExecution,
) {
    let barrier = Arc::new(Barrier::new(2));
    let spawn = |command: ProjectRegistryCommand, barrier: Arc<Barrier>| {
        let config = config.clone();
        let target = target.clone();
        thread::spawn(move || {
            let runtime = config.role_client(
                target.database_name(),
                DatabaseRole::Runtime,
                REQUIRED_APPLICATION_NAME,
            );
            let mut registry = PostgresProjectRegistry::new(runtime, &target)
                .unwrap_or_else(|error| panic!("{failure_code}_CONSTRUCTOR {}", error.code()));
            barrier.wait();
            registry
                .execute(command, live_authority('a', 'b'))
                .unwrap_or_else(|error| panic!("{failure_code}_EXECUTE {}", error.code()))
        })
    };
    let left = spawn(left_command, Arc::clone(&barrier));
    let right = spawn(right_command, barrier);
    (
        left.join()
            .unwrap_or_else(|_| panic!("{failure_code}_LEFT_JOIN")),
        right
            .join()
            .unwrap_or_else(|_| panic!("{failure_code}_RIGHT_JOIN")),
    )
}

fn prove_live_project_registry_restart(config: &LiveConfig, target: &MigrationTarget) {
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let mut registry = PostgresProjectRegistry::new(runtime, target).unwrap_or_else(|error| {
        panic!("STORE_TASK022_RESTART_CONSTRUCTOR_FAILED {}", error.code())
    });
    let loaded = registry
        .load()
        .unwrap_or_else(|error| panic!("STORE_TASK022_RESTART_LOAD_FAILED {}", error.code()));
    assert_eq!(loaded.retained_checkpoint().command_ordinal(), 13);
    assert_eq!(loaded.retained_checkpoint().project_count(), 5);
    let replay = registry
        .execute(
            registry_registration("registry-live-register", "registry-live"),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_RESTART_REPLAY_FAILED {}", error.code()));
    assert!(replay.is_exact_retry());
    assert_eq!(replay.result_checkpoint().command_ordinal(), 1);
    assert_eq!(
        registry
            .load()
            .expect("post-restart replay load")
            .retained_checkpoint()
            .command_ordinal(),
        13
    );
}

fn new_live_project_registry(
    config: &LiveConfig,
    target: &MigrationTarget,
) -> PostgresProjectRegistry {
    let runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    PostgresProjectRegistry::new(runtime, target)
        .unwrap_or_else(|error| panic!("STORE_TASK022_CONSTRUCTOR_FAILED {}", error.code()))
}

fn prove_live_project_registry_atomic_rollback(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "reg_rollback");
    set_live_admission(config, &target, true);
    let mut registry = new_live_project_registry(config, &target);
    let mut fixture = config.connect(target.database_name(), "task022-registry-rollback-fixture");
    fixture
        .batch_execute(
            "CREATE SEQUENCE public.task022_registry_rollback_counter; \
             REVOKE ALL ON SEQUENCE public.task022_registry_rollback_counter FROM PUBLIC, \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE FUNCTION public.task022_fail_registry_insert() RETURNS trigger \
             LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$ \
             BEGIN \
               PERFORM pg_catalog.nextval( \
                   'public.task022_registry_rollback_counter'::pg_catalog.regclass \
               ); \
               RAISE EXCEPTION USING ERRCODE = 'P0001', MESSAGE = 'task022 registry insert fixture'; \
             END $$; \
             REVOKE ALL ON FUNCTION public.task022_fail_registry_insert() FROM PUBLIC, \
                 lattice_migrator, lattice_runtime, lattice_guardian, lattice_readonly, \
                 lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             CREATE TRIGGER task022_fail_registry_insert \
             BEFORE INSERT ON control.project_registry_commands FOR EACH ROW \
             EXECUTE FUNCTION public.task022_fail_registry_insert()",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK022_ROLLBACK_FIXTURE_FAILED"));
    let failed = registry
        .execute(
            registry_registration("registry-rollback-command", "registry-rollback"),
            live_authority('a', 'b'),
        )
        .expect_err("injected command insert failure must roll back");
    assert_eq!(
        failed.kind(),
        PostgresProjectRegistryErrorKind::TransactionFailed
    );
    assert_eq!(
        project_registry_counts(config, &target),
        [0, 0, 0, 0, 0],
        "STORE_TASK022_ATOMIC_ROLLBACK_MUTATED"
    );
    assert!(
        registry
            .load()
            .unwrap_or_else(|error| panic!("STORE_TASK022_ROLLBACK_LOAD_FAILED {}", error.code()))
            .state()
            .is_vacant(),
        "STORE_TASK022_ROLLBACK_NOT_VACANT"
    );
    fixture
        .batch_execute(
            "DROP TRIGGER task022_fail_registry_insert ON control.project_registry_commands; \
             DROP FUNCTION public.task022_fail_registry_insert(); \
             DROP SEQUENCE public.task022_registry_rollback_counter",
        )
        .unwrap_or_else(|_| panic!("STORE_TASK022_ROLLBACK_FIXTURE_CLEANUP_FAILED"));
    let applied = registry
        .execute(
            registry_registration("registry-rollback-command", "registry-rollback"),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_ROLLBACK_RECOVERY_FAILED {}", error.code()));
    assert!(!applied.is_exact_retry());
    assert_eq!(project_registry_counts(config, &target), [1, 1, 1, 1, 3]);
}

fn prove_live_project_registry_commit_response_loss(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "reg_ack");
    set_live_admission(config, &target, true);
    let request = registry_registration("registry-lost-ack-command", "registry-lost-ack");
    let proxy = CommitResponseDropProxy::start_at_commit(&config.host, config.port, 3);
    let mut proxied = config.clone();
    proxied.port = proxy.port();
    let mut uncertain = new_live_project_registry(&proxied, &target);
    let unknown = uncertain
        .execute(request.clone(), live_authority('a', 'b'))
        .expect_err("lost Registry commit response must not return authority");
    assert_eq!(
        unknown.kind(),
        PostgresProjectRegistryErrorKind::CommitOutcomeUnknown
    );
    let poisoned = uncertain
        .execute(request.clone(), live_authority('a', 'b'))
        .expect_err("uncertain Registry adapter must remain poisoned");
    assert_eq!(
        poisoned.kind(),
        PostgresProjectRegistryErrorKind::CommitOutcomeUnknown
    );
    drop(uncertain);
    assert!(proxy.finish(), "STORE_TASK022_COMMIT_RESPONSE_NOT_DROPPED");

    let mut reconciler = new_live_project_registry(config, &target);
    let replay = reconciler
        .execute(request, live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("STORE_TASK022_COMMIT_RECONCILE_FAILED {}", error.code()));
    assert!(replay.is_exact_retry());
    assert_eq!(replay.result_checkpoint().command_ordinal(), 1);
    assert_eq!(project_registry_counts(config, &target), [1, 1, 1, 1, 3]);
}

fn prove_live_project_registry_lock_timeout(config: &LiveConfig, admin: &mut Client) {
    let target = migrated_database(config, admin, "reg_lock");
    set_live_admission(config, &target, true);
    let mut registry = new_live_project_registry(config, &target);
    let command = registry_registration("registry-lock-command", "registry-lock");
    let mut fixture = config.connect(target.database_name(), "task022-registry-lock-fixture");
    let mut lock = fixture
        .transaction()
        .unwrap_or_else(|_| panic!("STORE_TASK022_LOCK_TRANSACTION_FAILED"));
    lock.query_one(
        "SELECT singleton FROM ONLY control.project_registry_state \
         WHERE singleton = true FOR UPDATE",
        &[],
    )
    .unwrap_or_else(|_| panic!("STORE_TASK022_LOCK_FIXTURE_FAILED"));
    let timed_out = registry
        .execute(command.clone(), live_authority('a', 'b'))
        .expect_err("locked Registry singleton must time out");
    assert_eq!(
        timed_out.kind(),
        PostgresProjectRegistryErrorKind::Unavailable
    );
    assert_eq!(project_registry_counts(config, &target), [0, 0, 0, 0, 0]);
    lock.rollback()
        .unwrap_or_else(|_| panic!("STORE_TASK022_LOCK_RELEASE_FAILED"));
    let applied = registry
        .execute(command, live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("STORE_TASK022_LOCK_RECOVERY_FAILED {}", error.code()));
    assert!(!applied.is_exact_retry());
}

fn prove_live_project_registry_corruption(config: &LiveConfig, admin: &mut Client) {
    let stage_target = migrated_database(config, admin, "reg_stage");
    let mut stage_registry = new_live_project_registry(config, &stage_target);
    let mut stage_fixture = config.connect(
        stage_target.database_name(),
        "task022-registry-partial-stage-fixture",
    );
    stage_fixture
        .execute(
            "UPDATE ONLY control.project_registry_state \
             SET stage_command_id = 'registry-direct-stage', stage_ordinal = 1, \
                 stage_base_checkpoint_digest = checkpoint_digest, \
                 stage_result_checkpoint_digest = pg_catalog.decode(pg_catalog.repeat('11', 32), 'hex'), \
                 stage_record_set_digest = pg_catalog.decode(pg_catalog.repeat('22', 32), 'hex'), \
                 stage_observation = false, stage_project = false, \
                 stage_reservation_delete_count = 0, stage_reservation_insert_count = 0 \
             WHERE singleton = true",
            &[],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK022_PARTIAL_STAGE_FIXTURE_FAILED"));
    let partial = stage_registry
        .load()
        .expect_err("directly committed partial Registry stage must fail closed");
    assert_eq!(
        partial.kind(),
        PostgresProjectRegistryErrorKind::RetainedRowCorrupt
    );
    assert_eq!(
        project_registry_counts(config, &stage_target),
        [0, 0, 0, 0, 0]
    );

    let checkpoint_target = migrated_database(config, admin, "reg_checkpoint");
    let mut checkpoint_registry = new_live_project_registry(config, &checkpoint_target);
    let mut checkpoint_fixture = config.connect(
        checkpoint_target.database_name(),
        "task022-registry-checkpoint-fixture",
    );
    checkpoint_fixture
        .execute(
            "UPDATE ONLY control.project_registry_state \
             SET checkpoint_digest = pg_catalog.decode(pg_catalog.repeat('33', 32), 'hex') \
             WHERE singleton = true",
            &[],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK022_CHECKPOINT_FIXTURE_FAILED"));
    let checkpoint = checkpoint_registry
        .load()
        .expect_err("substituted Registry checkpoint must fail closed");
    assert_eq!(
        checkpoint.kind(),
        PostgresProjectRegistryErrorKind::RetainedRowCorrupt
    );

    let reservation_target = migrated_database(config, admin, "reg_reserve");
    set_live_admission(config, &reservation_target, true);
    let mut reservation_registry = new_live_project_registry(config, &reservation_target);
    reservation_registry
        .execute(
            registry_registration("registry-reservation-command", "registry-reservation"),
            live_authority('a', 'b'),
        )
        .unwrap_or_else(|error| panic!("STORE_TASK022_RESERVATION_SETUP_FAILED {}", error.code()));
    let mut reservation_fixture = config.connect(
        reservation_target.database_name(),
        "task022-registry-reservation-fixture",
    );
    reservation_fixture
        .execute(
            "DELETE FROM ONLY control.project_registry_identity_reservations \
             WHERE (dimension, identity_digest) IN ( \
                 SELECT dimension, identity_digest \
                 FROM ONLY control.project_registry_identity_reservations \
                 ORDER BY dimension, identity_digest LIMIT 1 \
             )",
            &[],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK022_RESERVATION_FIXTURE_FAILED"));
    let reservation = reservation_registry
        .load()
        .expect_err("missing Registry reservation must fail closed");
    assert_eq!(
        reservation.kind(),
        PostgresProjectRegistryErrorKind::RetainedRowCorrupt
    );
}

fn project_registry_counts(config: &LiveConfig, target: &MigrationTarget) -> [i64; 5] {
    let mut fixture = config.connect(target.database_name(), "task022-registry-counts");
    let row = fixture
        .query_one(
            "SELECT command_ordinal, \
                 (SELECT count(*) FROM ONLY control.project_registry_observations), \
                 (SELECT count(*) FROM ONLY control.project_registry_projects), \
                 (SELECT count(*) FROM ONLY control.project_registry_commands), \
                 (SELECT count(*) FROM ONLY control.project_registry_identity_reservations) \
             FROM ONLY control.project_registry_state WHERE singleton = true",
            &[],
        )
        .unwrap_or_else(|_| panic!("STORE_TASK022_COUNT_FIXTURE_FAILED"));
    [row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)]
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

#[allow(clippy::too_many_lines)]
fn prove_task038_task_created_jsonb_round_trip(config: &LiveConfig, target: &MigrationTarget) {
    set_live_admission(config, target, true);
    let identity = live_task_identity("task038-jsonb", "TASK-038");
    let mut ledger = new_live_task_ledger(config, target);
    let vacant = ledger
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        vacant.stream().head().is_zero(),
        "STORE_TASK038_JSONB_STREAM_NOT_VACANT"
    );

    let command = AppendCommand::new(
        vacant.stream().head().clone(),
        CommandId::new("task038-jsonb-task-created")
            .expect("STORE_TASK038_JSONB_COMMAND_FIXTURE_INVALID"),
        CorrelationId::new("task038-jsonb-correlation")
            .expect("STORE_TASK038_JSONB_CORRELATION_FIXTURE_INVALID"),
        "2026-08-10T00:00:00Z",
        LedgerEventKind::TaskCreated,
        ActorId::new("lattice-runtime").expect("STORE_TASK038_JSONB_ACTOR_FIXTURE_INVALID"),
        ActionId::new("admit-controlled-task").expect("STORE_TASK038_JSONB_ACTION_FIXTURE_INVALID"),
        LedgerOutcome::Recorded,
        ReasonCode::new("TASK038_TASK_CREATED")
            .expect("STORE_TASK038_JSONB_REASON_FIXTURE_INVALID"),
        live_digest('8'),
        Some(task038_task_created_diagnostic()),
        None,
    )
    .expect("STORE_TASK038_JSONB_APPEND_FIXTURE_INVALID");
    let plan = plan_append(vacant.stream(), command.clone())
        .expect("STORE_TASK038_JSONB_PURE_PLAN_FAILED");
    let expected =
        apply_append_plan(vacant.stream(), &plan).expect("STORE_TASK038_JSONB_PURE_APPLY_FAILED");

    let first = ledger
        .execute(command.clone(), live_authority('a', 'b'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(!first.is_exact_retry(), "STORE_TASK038_JSONB_FALSE_RETRY");
    assert_eq!(
        first.receipt(),
        plan.receipt(),
        "STORE_TASK038_JSONB_RECEIPT_MISMATCH"
    );
    assert_eq!(
        first.result_checkpoint(),
        plan.next_checkpoint(),
        "STORE_TASK038_JSONB_RESULT_CHECKPOINT_MISMATCH"
    );
    assert_eq!(
        first
            .store_receipt()
            .request()
            .mutation()
            .record_set_digest(),
        plan.record_set_digest(),
        "STORE_TASK038_JSONB_RECORD_SET_MISMATCH"
    );
    assert_eq!(
        first.store_receipt().after_head().state_digest(),
        plan.next_checkpoint().checkpoint_digest(),
        "STORE_TASK038_JSONB_STORE_CHECKPOINT_MISMATCH"
    );

    let fresh = ledger
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        fresh.stream(),
        &expected,
        "STORE_TASK038_JSONB_FRESH_STREAM_MISMATCH"
    );
    assert_eq!(
        fresh.retained_checkpoint(),
        plan.next_checkpoint(),
        "STORE_TASK038_JSONB_RETAINED_CHECKPOINT_MISMATCH"
    );
    assert_eq!(
        fresh.physical_head(),
        first.store_receipt().after_head(),
        "STORE_TASK038_JSONB_PHYSICAL_HEAD_MISMATCH"
    );

    let retry_plan = plan_append(fresh.stream(), command.clone())
        .expect("STORE_TASK038_JSONB_RETRY_PLAN_FAILED");
    assert!(
        retry_plan.is_exact_retry(),
        "STORE_TASK038_JSONB_RETRY_PLAN_NOT_EXACT"
    );
    assert_eq!(
        retry_plan.record_set_digest(),
        plan.record_set_digest(),
        "STORE_TASK038_JSONB_REPLAY_RECORD_SET_MISMATCH"
    );
    let retry = ledger
        .execute(command, live_authority('c', 'd'))
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(
        retry.is_exact_retry(),
        "STORE_TASK038_JSONB_EXECUTE_RETRY_NOT_EXACT"
    );
    assert_eq!(
        retry.store_receipt(),
        first.store_receipt(),
        "STORE_TASK038_JSONB_STORE_RECEIPT_CHANGED"
    );

    drop(ledger);
    let mut reconnected = new_live_task_ledger(config, target);
    let reloaded = reconnected
        .load_stream(identity)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(
        reloaded, fresh,
        "STORE_TASK038_JSONB_RECONNECTED_LOAD_MISMATCH"
    );
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
        5,
        "STORE_TASK021_GLOBAL_SCHEMA_NOT_V5"
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

fn task038_task_created_diagnostic() -> Diagnostic {
    let string = |value: &str| CanonicalValue::String(value.to_owned());
    let fields = vec![
        ("actor_kind".to_owned(), string("LOCAL_ACCEPTANCE_HARNESS")),
        (
            "adapter_id".to_owned(),
            string("lattice-local-canonical-mcp-acceptance"),
        ),
        (
            "admission_observation_commitment".to_owned(),
            string(live_digest('c').as_str()),
        ),
        (
            "client_kind".to_owned(),
            string("LOCAL_CANONICAL_MCP_ACCEPTANCE"),
        ),
        (
            "process_start_authority_digest".to_owned(),
            string(live_digest('d').as_str()),
        ),
        (
            "profile_adapter_commitment".to_owned(),
            string(live_digest('e').as_str()),
        ),
        (
            "schema".to_owned(),
            string("lattice.task-created-ingress-audit.v1"),
        ),
    ];
    assert!(
        fields
            .windows(2)
            .all(|pair| pair[0].0.as_bytes() < pair[1].0.as_bytes()),
        "STORE_TASK038_JSONB_DIAGNOSTIC_KEYS_NOT_SORTED"
    );
    Diagnostic::new(CanonicalValue::Object(fields))
        .expect("STORE_TASK038_JSONB_DIAGNOSTIC_FIXTURE_INVALID")
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
fn prove_live_task_ledger_commit_response_loss(
    config: &LiveConfig,
    admin: &mut Client,
    database_tag: &str,
) {
    let target = migrated_database(config, admin, database_tag);
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
                 pg_catalog.pg_try_advisory_lock(bigint), \
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
    task075_stage!("FIRST_APPLY", {
        assert_eq!(
            must_setup(apply_migrations(&mut first, &target)),
            MigrationApplyOutcome::Applied {
                executable_count: 5
            }
        );
    });
    let evidence = task075_stage!(
        "FIRST_VERIFY",
        must_setup(verify_postgres_schema(
            &mut first,
            &target,
            DatabaseRole::Migrator,
        ))
    );
    task075_stage!(
        "MANIFEST_RECOMPUTE",
        assert_history_manifest_recomputation(&mut first, evidence.manifest_sha256().as_str())
    );
    drop(first);

    let mut reconciler = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    task075_stage!("SECOND_APPLY", {
        assert_eq!(
            must_setup(apply_migrations(&mut reconciler, &target)),
            MigrationApplyOutcome::AlreadyCurrent
        );
    });
    let reconciled_evidence = task075_stage!(
        "SECOND_VERIFY",
        must_setup(verify_postgres_schema(
            &mut reconciler,
            &target,
            DatabaseRole::Migrator,
        ))
    );
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
    assert_eq!(row.get::<_, i64>(1), 6);
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
            executable_count: 4
        }
    );
    let evidence = must_setup(verify_postgres_schema(
        &mut migrator,
        &target,
        DatabaseRole::Migrator,
    ));
    assert_eq!(evidence.schema_version(), 5);
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
                executable_count: 4,
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
            executable_count: 4
        }
    );
    assert_eq!(
        must_setup(verify_postgres_schema(
            &mut retry,
            &target,
            DatabaseRole::Migrator,
        ))
        .schema_version(),
        5
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
            executable_count: 3
        }
    );
    assert_eq!(
        must_setup(verify_postgres_schema(
            &mut migrator,
            &target,
            DatabaseRole::Migrator,
        ))
        .schema_version(),
        5
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
    let global_manifest = must_setup(verify_embedded_manifest());
    let global_schema_version = i16::try_from(global_manifest.schema_version())
        .unwrap_or_else(|_| panic!("STORE_TASK021_V2_REPLAY_SCHEMA_VERSION_INVALID"));
    let row = transaction
        .query_one(
            "SELECT prepare_status, database_uuid::text, \
                    encode(database_identity_digest, 'hex'), schema_version, \
                    manifest_sha256, head_found, before_revision, after_revision, \
                    terminal_disposition, encode(terminal_transaction_digest, 'hex'), \
                    encode(terminal_receipt_digest, 'hex') \
             FROM control.store_prepare_v5(\
                 $1::smallint, $2::text, 2::smallint, \
                 'v2-replay-transaction', 'v2-project', 'v2-snapshot', \
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
            &[
                &global_schema_version,
                &global_manifest.manifest_sha256().as_str(),
            ],
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

fn prove_exact_nonempty_v3_ledger_upgrade_and_replay(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "three_ledger", true);
    install_exact_v3(config, &target);
    set_exact_database_access(admin, target.database_name());
    set_live_admission(config, &target, true);

    let identity = live_task_identity("ledger-v3-upgrade", "TASK-022");
    let vacant = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
        .expect("STORE_TASK022_V3_LEDGER_VACANT_INVALID");
    let command = live_task_command(
        vacant.head().clone(),
        "ledger-v3-upgrade-command",
        LedgerEventKind::TaskCreated,
        LedgerOutcome::Recorded,
        '8',
    );
    let authority = live_authority('a', 'b');
    let mut v3_ledger = new_live_task_ledger(config, &target);
    println!("STORE_TASK022_V3_LEDGER_01_ADAPTER_READY");
    let before_execution = v3_ledger
        .execute(command.clone(), authority.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    println!("STORE_TASK022_V3_LEDGER_02_SEEDED");
    assert_eq!(before_execution.persistence().schema_version(), 3);
    let before = v3_ledger
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    let before_counts = task_ledger_counts(config, &target, &identity);
    assert_eq!(before_counts, [1, 1, 1, 1, 1, 0]);
    drop(v3_ledger);

    set_live_admission(config, &target, false);
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
    println!("STORE_TASK022_V3_LEDGER_03_MIGRATED");
    assert_eq!(
        must_setup(verify_postgres_schema(
            &mut migrator,
            &target,
            DatabaseRole::Migrator,
        ))
        .schema_version(),
        5
    );
    drop(migrator);

    let mut v5_ledger = new_live_task_ledger(config, &target);
    println!("STORE_TASK075_V3_LEDGER_04_V5_ADAPTER_READY");
    let after = v5_ledger
        .load_stream(identity.clone())
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert_eq!(after.persistence().schema_version(), 5);
    assert_eq!(after.stream(), before.stream());
    assert_eq!(after.retained_checkpoint(), before.retained_checkpoint());
    assert_eq!(after.physical_head(), before.physical_head());
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        before_counts
    );

    let replay = v5_ledger
        .execute(command, authority)
        .unwrap_or_else(|error| panic!("{}", error.code()));
    assert!(replay.is_exact_retry());
    assert_eq!(replay.receipt(), before_execution.receipt());
    assert_eq!(
        replay.result_checkpoint(),
        before_execution.result_checkpoint()
    );
    assert_eq!(replay.store_receipt(), before_execution.store_receipt());
    assert_eq!(
        task_ledger_counts(config, &target, &identity),
        before_counts
    );
    println!("STORE_TASK022_V3_LEDGER_05_REPLAYED");
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

fn install_exact_v3(config: &LiveConfig, target: &MigrationTarget) {
    install_exact_prefix(config, target, 4, TASK_LEDGER_V3_MANIFEST_SHA256, 3);
}

fn install_exact_v4(config: &LiveConfig, target: &MigrationTarget) {
    install_exact_prefix(config, target, 5, REGISTRY_V4_MANIFEST_SHA256, 4);
}

fn install_exact_v5(config: &LiveConfig, target: &MigrationTarget) {
    install_exact_prefix(config, target, 6, CURRENT_V5_MANIFEST_SHA256, 5);
}

// This fixture proves the transition and every post-transition rejection on
// the same database; splitting it would lose the historical provenance chain.
#[allow(clippy::too_many_lines)]
fn prove_exact_v3_memory_v2_global_upgrade(config: &LiveConfig, admin: &mut Client) {
    let target = task075_stage!("V3_MEMORY_V2_SOURCE", {
        let target = provision_database(config, admin, "three_memory", true);
        install_exact_v3(config, &target);
        install_codebase_memory_v2(config, &target);
        target
    });

    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    task075_stage!("GLOBAL_V5_PENDING", {
        assert_eq!(
            must_setup(apply_migrations(&mut migrator, &target)),
            MigrationApplyOutcome::Applied {
                executable_count: 2
            }
        );
        let pending = must_setup(verify_postgres_schema(
            &mut migrator,
            &target,
            DatabaseRole::Migrator,
        ));
        assert_eq!(pending.schema_version(), 5);
        assert_eq!(
            pending.manifest_sha256().as_str(),
            CURRENT_V5_MANIFEST_SHA256
        );
        assert_eq!(
            must_setup(apply_migrations(&mut migrator, &target)),
            MigrationApplyOutcome::AlreadyCurrent
        );
    });
    drop(migrator);

    task075_stage!("PENDING_RUNTIME_REJECTION", {
        let mut runtime = config.role_client(
            target.database_name(),
            DatabaseRole::Runtime,
            REQUIRED_APPLICATION_NAME,
        );
        expect_setup_kind(
            verify_postgres_schema(&mut runtime, &target, DatabaseRole::Runtime),
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        );
    });

    task075_stage!(
        "MEMORY_V3_UPGRADE",
        upgrade_codebase_memory_v3(config, &target)
    );
    task075_stage!(
        "MEMORY_V3_LEDGER_FK",
        prove_memory_v3_ledger_identity_fk(config, &target)
    );
    task075_stage!("MEMORY_V3_CURRENT_ROLES", {
        for role in DatabaseRole::ALL {
            println!(
                "{}",
                match role {
                    DatabaseRole::Migrator => "TASK075_STAGE_ENTER_MEMORY_V3_ROLE_MIGRATOR",
                    DatabaseRole::Runtime => "TASK075_STAGE_ENTER_MEMORY_V3_ROLE_RUNTIME",
                    DatabaseRole::Guardian => "TASK075_STAGE_ENTER_MEMORY_V3_ROLE_GUARDIAN",
                    DatabaseRole::ReadOnly => "TASK075_STAGE_ENTER_MEMORY_V3_ROLE_READONLY",
                }
            );
            let mut client =
                config.role_client(target.database_name(), role, REQUIRED_APPLICATION_NAME);
            let current = must_setup(verify_postgres_schema(&mut client, &target, role));
            assert_eq!(current.schema_version(), 5);
            assert_eq!(
                current.manifest_sha256().as_str(),
                CURRENT_V5_MANIFEST_SHA256
            );
            println!(
                "{}",
                match role {
                    DatabaseRole::Migrator => "TASK075_STAGE_PASS_MEMORY_V3_ROLE_MIGRATOR",
                    DatabaseRole::Runtime => "TASK075_STAGE_PASS_MEMORY_V3_ROLE_RUNTIME",
                    DatabaseRole::Guardian => "TASK075_STAGE_PASS_MEMORY_V3_ROLE_GUARDIAN",
                    DatabaseRole::ReadOnly => "TASK075_STAGE_PASS_MEMORY_V3_ROLE_READONLY",
                }
            );
        }
    });

    task075_stage!("MEMORY_V3_IDENTITY_SUBSTITUTION", {
        let mut drift = config.role_client(
            target.database_name(),
            DatabaseRole::Migrator,
            REQUIRED_APPLICATION_NAME,
        );
        drift
            .execute(
                "UPDATE ONLY memory.codebase_memory_extension_identity \
                SET extension_manifest_sha256 = $1 \
              WHERE singleton",
                &[&"1111111111111111111111111111111111111111111111111111111111111111"],
            )
            .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_IDENTITY_DRIFT_FAILED"));
        expect_setup_kind(
            verify_postgres_schema(&mut drift, &target, DatabaseRole::Migrator),
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
        );
        drift
            .execute(
                "UPDATE ONLY memory.codebase_memory_extension_identity \
                SET extension_manifest_sha256 = $1 \
              WHERE singleton",
                &[&CODEBASE_MEMORY_V3_MANIFEST_SHA256],
            )
            .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_IDENTITY_RESTORE_FAILED"));
        must_setup(verify_postgres_schema(
            &mut drift,
            &target,
            DatabaseRole::Migrator,
        ));
    });

    task075_stage!("MEMORY_V3_ADMIN_IDENTITY_SUBSTITUTIONS", {
        for (table, predicate) in [
            ("codebase_memory_extension_identity", "singleton = true"),
            ("codebase_memory_extension_ledger", "ledger_ordinal = 1"),
            ("codebase_memory_extension_ledger", "ledger_ordinal = 2"),
        ] {
            let mut migrator = config.role_client(
                target.database_name(),
                DatabaseRole::Migrator,
                REQUIRED_APPLICATION_NAME,
            );
            let substitution = "2222222222222222222222222222222222222222222222222222222222222222";
            migrator
                .batch_execute(&format!(
                    "UPDATE ONLY memory.{table} \
                    SET database_identity_sha256 = '{substitution}' \
                  WHERE {predicate}"
                ))
                .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_ADMIN_SUBSTITUTION_FAILED"));
            expect_setup_kind(
                verify_postgres_schema(&mut migrator, &target, DatabaseRole::Migrator),
                PostgresStoreSetupErrorKind::CompatibilityMismatch,
            );
            migrator
                .execute(
                    &format!(
                        "UPDATE ONLY memory.{table} \
                        SET database_identity_sha256 = $1 \
                      WHERE {predicate}"
                    ),
                    &[&target.expected_database_identity_sha256().as_str()],
                )
                .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_ADMIN_SUBSTITUTION_RESTORE_FAILED"));
            must_setup(verify_postgres_schema(
                &mut migrator,
                &target,
                DatabaseRole::Migrator,
            ));
            drop(migrator);

            let runtime = config.role_client(
                target.database_name(),
                DatabaseRole::Runtime,
                REQUIRED_APPLICATION_NAME,
            );
            PostgresControlStore::new(runtime, &target)
                .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_RUNTIME_PROFILE_REJECTED"));
        }
    });
    println!("STORE_TASK075_V3_MEMORY_V2_GLOBAL_UPGRADE_OK");
}

fn prove_memory_v3_ledger_identity_fk(config: &LiveConfig, target: &MigrationTarget) {
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let boundary = migrator
        .query_one(
            "SELECT \
                 (SELECT count(*) FROM pg_constraint c \
                  JOIN pg_namespace n ON n.oid = c.connamespace \
                  WHERE n.nspname = 'memory' \
                    AND c.conname = 'codebase_memory_extension_ledger_identity_fk' \
                    AND c.contype = 'f' AND c.convalidated \
                    AND c.conrelid = 'memory.codebase_memory_extension_ledger'::regclass \
                    AND c.confrelid = 'memory.codebase_memory_extension_identity'::regclass \
                    AND c.conkey = ARRAY[(SELECT a.attnum FROM pg_attribute a \
                        WHERE a.attrelid = c.conrelid AND a.attname = 'singleton')]::smallint[] \
                    AND c.confkey = ARRAY[(SELECT a.attnum FROM pg_attribute a \
                        WHERE a.attrelid = c.confrelid AND a.attname = 'singleton')]::smallint[]), \
                 (SELECT count(*) FROM ONLY memory.codebase_memory_extension_ledger)",
            &[],
        )
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_LEDGER_FK_QUERY_FAILED"));
    assert_eq!(
        boundary.get::<_, i64>(0),
        1,
        "TASK075_MEMORY_V3_LEDGER_FK_NOT_EXACT"
    );
    assert_eq!(
        boundary.get::<_, i64>(1),
        2,
        "TASK075_MEMORY_V3_LEDGER_HISTORY_NOT_TWO_ROWS"
    );

    migrator
        .batch_execute(
            "ALTER TABLE ONLY memory.codebase_memory_extension_ledger \
             DROP CONSTRAINT codebase_memory_extension_ledger_identity_fk",
        )
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_LEDGER_FK_DROP_FAILED"));
    expect_setup_kind(
        verify_postgres_schema(&mut migrator, target, DatabaseRole::Migrator),
        PostgresStoreSetupErrorKind::CorruptCatalog,
    );
    migrator
        .batch_execute(
            "ALTER TABLE ONLY memory.codebase_memory_extension_ledger \
             ADD CONSTRAINT codebase_memory_extension_ledger_identity_fk \
             FOREIGN KEY (singleton) \
             REFERENCES memory.codebase_memory_extension_identity (singleton)",
        )
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_LEDGER_FK_RESTORE_FAILED"));
    must_setup(verify_postgres_schema(
        &mut migrator,
        target,
        DatabaseRole::Migrator,
    ));
}

fn install_codebase_memory_v2(config: &LiveConfig, target: &MigrationTarget) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::ReadCommitted)
        .start()
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V2_FIXTURE_TRANSACTION_FAILED"));
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V2_FIXTURE_HARDEN_FAILED"));
    transaction
        .batch_execute(CODEBASE_MEMORY_V2_SQL)
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V2_FIXTURE_SQL_FAILED"));
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_identity ( \
                 singleton, extension_id, extension_schema_version, extension_path, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256 \
             ) VALUES (true, 'lattice-codebase-memory', 2, $1, $2, $3, \
                 $4::text::uuid, $5, 3, $6)",
            &[
                &CODEBASE_MEMORY_V2_PATH,
                &CODEBASE_MEMORY_V2_SQL_SHA256,
                &CODEBASE_MEMORY_V2_MANIFEST_SHA256,
                &target.expected_database_uuid(),
                &target.expected_database_identity_sha256().as_str(),
                &TASK_LEDGER_V3_MANIFEST_SHA256,
            ],
        )
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V2_FIXTURE_IDENTITY_FAILED"));
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_ledger ( \
                 ledger_ordinal, singleton, extension_id, extension_schema_version, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 event_kind \
             ) VALUES (1, true, 'lattice-codebase-memory', 2, $1, $2, \
                 $3::text::uuid, $4, 3, $5, 'INSTALLED')",
            &[
                &CODEBASE_MEMORY_V2_SQL_SHA256,
                &CODEBASE_MEMORY_V2_MANIFEST_SHA256,
                &target.expected_database_uuid(),
                &target.expected_database_identity_sha256().as_str(),
                &TASK_LEDGER_V3_MANIFEST_SHA256,
            ],
        )
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V2_FIXTURE_LEDGER_FAILED"));
    transaction
        .commit()
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V2_FIXTURE_COMMIT_FAILED"));
}

fn upgrade_codebase_memory_v3(config: &LiveConfig, target: &MigrationTarget) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::ReadCommitted)
        .start()
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_FIXTURE_TRANSACTION_FAILED"));
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_FIXTURE_HARDEN_FAILED"));
    transaction
        .batch_execute(CODEBASE_MEMORY_V3_SQL)
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_FIXTURE_SQL_FAILED"));
    let changed = transaction
        .execute(
            "UPDATE ONLY memory.codebase_memory_extension_identity \
                SET extension_schema_version = 3, extension_path = $1, \
                    extension_sql_sha256 = $2, extension_manifest_sha256 = $3, \
                    global_schema_version = 5, global_manifest_sha256 = $4 \
              WHERE singleton AND extension_schema_version = 2 \
                AND extension_path = $5 AND global_schema_version = 3 \
                AND global_manifest_sha256 = $6",
            &[
                &CODEBASE_MEMORY_V3_PATH,
                &CODEBASE_MEMORY_V3_SQL_SHA256,
                &CODEBASE_MEMORY_V3_MANIFEST_SHA256,
                &CURRENT_V5_MANIFEST_SHA256,
                &CODEBASE_MEMORY_V2_PATH,
                &TASK_LEDGER_V3_MANIFEST_SHA256,
            ],
        )
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_FIXTURE_IDENTITY_FAILED"));
    assert_eq!(changed, 1, "TASK075_MEMORY_V3_FIXTURE_IDENTITY_MISSING");
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_ledger ( \
                 ledger_ordinal, singleton, extension_id, extension_schema_version, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 event_kind \
             ) VALUES (2, true, 'lattice-codebase-memory', 3, $1, $2, \
                 $3::text::uuid, $4, 5, $5, 'UPGRADED')",
            &[
                &CODEBASE_MEMORY_V3_SQL_SHA256,
                &CODEBASE_MEMORY_V3_MANIFEST_SHA256,
                &target.expected_database_uuid(),
                &target.expected_database_identity_sha256().as_str(),
                &CURRENT_V5_MANIFEST_SHA256,
            ],
        )
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_FIXTURE_LEDGER_FAILED"));
    transaction
        .commit()
        .unwrap_or_else(|_| panic!("TASK075_MEMORY_V3_FIXTURE_COMMIT_FAILED"));
}

fn install_exact_prefix(
    config: &LiveConfig,
    target: &MigrationTarget,
    prefix_len: usize,
    manifest_sha256: &str,
    schema_version: i16,
) {
    let manifest = migration_manifest();
    assert_eq!(manifest.len(), 6);
    assert!(matches!(prefix_len, 2..=6));
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
                executable_count: 5,
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
                 7, '0007_unknown', 'db/migrations/0007_unknown.sql', 1, repeat('1', 64), \
                 'EXECUTABLE', 'RUNNER_OWNED', 5, 5, 5, 5, 5 \
             )",
        )
        .unwrap_or_else(|_| panic!("TASK019_HISTORY_UNKNOWN_FIXTURE_FAILED"));
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

// The legacy EXECUTE denial and current-manifest mismatch checks must share
// the same runtime session and exact profile evidence.
#[allow(clippy::too_many_lines)]
fn prove_runtime_manifest_boundaries_fail_closed(config: &LiveConfig, target: &MigrationTarget) {
    let mut runtime = config.role_client(
        target.database_name(),
        DatabaseRole::Runtime,
        REQUIRED_APPLICATION_NAME,
    );
    let global_manifest = must_setup(verify_embedded_manifest());
    let global_schema_version = i16::try_from(global_manifest.schema_version())
        .unwrap_or_else(|_| panic!("STORE_TASK021_MANIFEST_SCHEMA_VERSION_INVALID"));
    let global_manifest_sha256 = global_manifest.manifest_sha256().as_str();
    let drifted_global_manifest_sha256 = "f".repeat(64);
    let current_head_sql = "SELECT * FROM control.store_current_head_v5(\
             $1::smallint, $2::text, 'manifest-project', 'manifest-snapshot', 'TASK_LEDGER', \
             pg_catalog.decode(pg_catalog.repeat('11', 32), 'hex')\
         )";
    let legacy_head_sql = current_head_sql.replacen(
        "control.store_current_head_v5",
        "control.store_current_head_v4",
        1,
    );
    let legacy_head_error = runtime
        .query(
            &legacy_head_sql,
            &[&global_schema_version, &global_manifest_sha256],
        )
        .expect_err("STORE_TASK075_LEGACY_CURRENT_HEAD_EXECUTE_ACCEPTED");
    assert_eq!(
        legacy_head_error
            .as_db_error()
            .map(postgres::error::DbError::code),
        Some(&SqlState::INSUFFICIENT_PRIVILEGE),
        "STORE_TASK075_LEGACY_CURRENT_HEAD_DENIAL_INVALID"
    );
    let current_error = runtime
        .query(
            current_head_sql,
            &[&global_schema_version, &drifted_global_manifest_sha256],
        )
        .expect_err("STORE_TASK021_MANIFEST_CURRENT_HEAD_ACCEPTED_DRIFT");
    assert_eq!(
        current_error
            .as_db_error()
            .map(|database| database.code().code()),
        Some("LST01"),
        "STORE_TASK021_MANIFEST_CURRENT_HEAD_QUERY_FAILED"
    );

    for (sql, current_function, legacy_function, marker) in [
        (
            "SELECT * FROM control.store_prepare_v5(\
                 $1::smallint, $2::text, 2::smallint, \
                 'manifest-drift-transaction', 'manifest-project', \
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
            "control.store_prepare_v5",
            "control.store_prepare_v4",
            "STORE_TASK021_MANIFEST_PREPARE_ACCEPTED_DRIFT",
        ),
        (
            "SELECT * FROM control.task_ledger_prepare_v3(\
                 $1::smallint, $2::text, \
                 pg_catalog.decode(pg_catalog.repeat('23', 32), 'hex'), \
                 'manifest-drift-command'\
             )",
            "control.task_ledger_prepare_v3",
            "control.task_ledger_prepare_v2",
            "STORE_TASK021_MANIFEST_LEDGER_ACCEPTED_DRIFT",
        ),
    ] {
        let legacy_sql = sql.replacen(current_function, legacy_function, 1);
        let legacy_error = runtime
            .query_one(
                &legacy_sql,
                &[&global_schema_version, &global_manifest_sha256],
            )
            .expect_err("STORE_TASK075_LEGACY_RUNTIME_EXECUTE_ACCEPTED");
        assert_eq!(
            legacy_error
                .as_db_error()
                .map(postgres::error::DbError::code),
            Some(&SqlState::INSUFFICIENT_PRIVILEGE),
            "STORE_TASK075_LEGACY_RUNTIME_DENIAL_INVALID"
        );
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
        let error = transaction
            .query_one(
                sql,
                &[&global_schema_version, &drifted_global_manifest_sha256],
            )
            .expect_err(marker);
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

fn prove_misplaced_autonomy_0005_pre_ddl_rejection(config: &LiveConfig, admin: &mut Client) {
    let target = provision_database(config, admin, "misplaced_auto", true);
    install_exact_v3(config, &target);

    let misplaced_migration_id = "0005_task_autonomy_receipt";
    let misplaced_migration_path = "db/migrations/0005_task_autonomy_receipt.sql";
    let misplaced_sql_sha256 = "5dbf7439887ba30e8070bcb8883c1994e42a3d3a7ce78dc174771d3b89049436";
    let misplaced_manifest_sha256 =
        "9378bbadf1e990e7d2617b66343b07193b2b8dd19bc8bb3dd6a3b618b134538a";

    let autonomy_migration = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0006_task_autonomy_receipt")
        .unwrap_or_else(|| panic!("TASK075_AUTONOMY_MIGRATION_MISSING"));
    let autonomy_sql = std::str::from_utf8(autonomy_migration.bytes())
        .unwrap_or_else(|_| panic!("TASK075_AUTONOMY_MIGRATION_UTF8_INVALID"));
    let table_start = autonomy_sql
        .find("CREATE TABLE control.task_ledger_autonomy_receipts (")
        .unwrap_or_else(|| panic!("TASK075_AUTONOMY_TABLE_DDL_MISSING"));
    let table_end = autonomy_sql[table_start..]
        .find("\n\nCREATE FUNCTION control.task_ledger_record_autonomy_receipt_v1(")
        .map_or_else(
            || panic!("TASK075_AUTONOMY_TABLE_DDL_BOUNDARY_MISSING"),
            |offset| table_start + offset,
        );

    let mut fixture = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    fixture
        .batch_execute(&autonomy_sql[table_start..table_end])
        .unwrap_or_else(|_| panic!("TASK075_MISPLACED_AUTONOMY_TABLE_FIXTURE_FAILED"));
    fixture
        .execute(
            "INSERT INTO control.migration_history ( \
                 ordinal, migration_id, migration_path, byte_length, checksum_sha256, \
                 migration_status, transaction_mode, schema_version, min_reader, \
                 max_reader, min_writer, max_writer \
             ) VALUES ( \
                 5, $1::text, $2::text, 19326, $3::text, \
                 'EXECUTABLE', 'RUNNER_OWNED', 3, 3, 3, 3, 3 \
             )",
            &[
                &misplaced_migration_id,
                &misplaced_migration_path,
                &misplaced_sql_sha256,
            ],
        )
        .unwrap_or_else(|_| panic!("TASK075_MISPLACED_AUTONOMY_HISTORY_FIXTURE_FAILED"));
    assert_eq!(
        fixture
            .execute(
                "UPDATE ONLY control.schema_compatibility \
                 SET manifest_sha256 = $1::text, current_schema_version = 3, \
                     min_reader = 3, max_reader = 3, min_writer = 3, max_writer = 3 \
                 WHERE singleton = true",
                &[&misplaced_manifest_sha256],
            )
            .unwrap_or_else(|_| panic!("TASK075_MISPLACED_AUTONOMY_PROFILE_FIXTURE_FAILED")),
        1
    );
    drop(fixture);

    set_exact_database_access(admin, target.database_name());
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let history_before = read_migration_history_fingerprint(&mut migrator);
    let catalog_before = read_owned_catalog_fingerprint(&mut migrator);
    assert_eq!(
        migrator
            .query_one(
                "SELECT count(*) FROM pg_catalog.pg_class c \
                 JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = 'control' \
                   AND c.relname = 'task_ledger_autonomy_receipts' \
                   AND c.relkind = 'r'",
                &[],
            )
            .unwrap_or_else(|_| panic!("TASK075_MISPLACED_AUTONOMY_TABLE_QUERY_FAILED"))
            .get::<_, i64>(0),
        1
    );

    expect_setup_kind(
        apply_migrations(&mut migrator, &target),
        PostgresStoreSetupErrorKind::HistoryMismatch,
    );

    let history_after = read_migration_history_fingerprint(&mut migrator);
    let catalog_after = read_owned_catalog_fingerprint(&mut migrator);
    assert_eq!(history_after, history_before);
    assert_eq!(catalog_after, catalog_before);
}

fn read_migration_history_fingerprint(client: &mut Client) -> String {
    client
        .query_one(
            "WITH fingerprint_rows AS ( \
                 SELECT pg_catalog.jsonb_build_array( \
                            'history', ordinal, migration_id, migration_path, byte_length, \
                            checksum_sha256, migration_status, transaction_mode, schema_version, \
                            min_reader, max_reader, min_writer, max_writer \
                        )::text AS payload \
                   FROM ONLY control.migration_history \
                 UNION ALL \
                 SELECT pg_catalog.jsonb_build_array( \
                            'compatibility', singleton, manifest_sha256, \
                            current_schema_version, min_reader, max_reader, min_writer, max_writer \
                        )::text AS payload \
                   FROM ONLY control.schema_compatibility \
             ) \
             SELECT pg_catalog.encode( \
                        pg_catalog.sha256(pg_catalog.convert_to( \
                            COALESCE(pg_catalog.string_agg(payload, E'\\n' ORDER BY payload), ''), \
                            'UTF8' \
                        )), \
                        'hex' \
                    ) \
               FROM fingerprint_rows",
            &[],
        )
        .unwrap_or_else(|_| panic!("TASK075_MISPLACED_AUTONOMY_HISTORY_FINGERPRINT_FAILED"))
        .get(0)
}

fn read_owned_catalog_fingerprint(client: &mut Client) -> String {
    client
        .query_one(
            "WITH catalog_rows AS ( \
                 SELECT pg_catalog.jsonb_build_array( \
                            'namespace', n.nspname, pg_catalog.pg_get_userbyid(n.nspowner), \
                            COALESCE(n.nspacl::text, '<NULL>') \
                        )::text AS payload \
                   FROM pg_catalog.pg_namespace n \
                  WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                 UNION ALL \
                 SELECT pg_catalog.jsonb_build_array( \
                            'class', n.nspname, c.relname, c.relkind::text, \
                            pg_catalog.pg_get_userbyid(c.relowner), \
                            COALESCE(c.relacl::text, '<NULL>'), c.relpersistence::text, \
                            c.relreplident::text, COALESCE(c.reloptions::text, '<NULL>'), \
                            COALESCE(pg_catalog.obj_description(c.oid, 'pg_class'), '<NULL>') \
                        )::text AS payload \
                   FROM pg_catalog.pg_class c \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                 UNION ALL \
                 SELECT pg_catalog.jsonb_build_array( \
                            'column', n.nspname, c.relname, a.attnum, a.attname, \
                            pg_catalog.format_type(a.atttypid, a.atttypmod), \
                            a.attnotnull, a.attisdropped, \
                            COALESCE(pg_catalog.pg_get_expr(d.adbin, d.adrelid, false), '<NULL>'), \
                            a.attidentity::text, a.attgenerated::text, a.attstorage::text, \
                            a.attcompression::text, a.attstattarget \
                        )::text AS payload \
                   FROM pg_catalog.pg_attribute a \
                   JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                   LEFT JOIN pg_catalog.pg_attrdef d \
                          ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                  WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                    AND a.attnum > 0 \
                 UNION ALL \
                 SELECT pg_catalog.jsonb_build_array( \
                            'constraint', n.nspname, c.relname, x.conname, x.contype::text, \
                            x.condeferrable, x.condeferred, x.convalidated, \
                            pg_catalog.pg_get_constraintdef(x.oid, false) \
                        )::text AS payload \
                   FROM pg_catalog.pg_constraint x \
                   JOIN pg_catalog.pg_class c ON c.oid = x.conrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                 UNION ALL \
                 SELECT pg_catalog.jsonb_build_array( \
                            'index', n.nspname, c.relname, i.relname, \
                            pg_catalog.pg_get_indexdef(x.indexrelid, 0, false) \
                        )::text AS payload \
                   FROM pg_catalog.pg_index x \
                   JOIN pg_catalog.pg_class c ON c.oid = x.indrelid \
                   JOIN pg_catalog.pg_class i ON i.oid = x.indexrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                 UNION ALL \
                 SELECT pg_catalog.jsonb_build_array( \
                            'function', n.nspname, p.proname, \
                            pg_catalog.pg_get_function_identity_arguments(p.oid), \
                            pg_catalog.pg_get_userbyid(p.proowner), p.provolatile::text, \
                            p.prosecdef, p.proleakproof, p.proparallel::text, \
                            COALESCE(p.proacl::text, '<NULL>'), \
                            pg_catalog.pg_get_functiondef(p.oid) \
                        )::text AS payload \
                   FROM pg_catalog.pg_proc p \
                   JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
                  WHERE n.nspname IN ('control', 'memory', 'readmodel') \
                 UNION ALL \
                 SELECT pg_catalog.jsonb_build_array( \
                            'trigger', n.nspname, c.relname, t.tgname, t.tgenabled::text, \
                            t.tgisinternal, pg_catalog.pg_get_triggerdef(t.oid, false) \
                        )::text AS payload \
                   FROM pg_catalog.pg_trigger t \
                   JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                  WHERE n.nspname IN ('control', 'memory', 'readmodel') \
             ) \
             SELECT pg_catalog.encode( \
                        pg_catalog.sha256(pg_catalog.convert_to( \
                            COALESCE(pg_catalog.string_agg(payload, E'\\n' ORDER BY payload), ''), \
                            'UTF8' \
                        )), \
                        'hex' \
                    ) \
               FROM catalog_rows",
            &[],
        )
        .unwrap_or_else(|_| panic!("TASK075_MISPLACED_AUTONOMY_CATALOG_FINGERPRINT_FAILED"))
        .get(0)
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
