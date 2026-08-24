use lattice_contracts::{
    ContentDigest, DaemonEpoch, ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind,
    StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId, TaskId,
    TaskLedgerStreamIdentity,
};
use lattice_postgres_store::{
    DatabaseRole, MigrationApplyOutcome, MigrationStatus, MigrationTarget, PostgresTaskLedger,
    PostgresTaskLedgerError, PostgresTaskLedgerErrorKind, apply_migrations, migration_manifest,
    verify_postgres_schema,
};
use lattice_task_ledger::{
    ActorId, AppendCommand, AutonomyAppendMetadata, AutonomyAuthorityEvidence,
    AutonomyDecisionReason, AutonomyIntent, AutonomyObservedTaskState, AutonomyRecommendation,
    AutonomyRiskClass, AutonomyTaskKind, CommandId, CorrelationId, ReasonCode,
    VerifiedAutonomyReceiptState, plan_autonomy_receipt_append,
};
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls};
use sha2::{Digest, Sha256};

#[test]
fn postgres_task_ledger_errors_are_static_and_exhaustive() {
    let expected = [
        "POSTGRES_TASK_LEDGER_MALFORMED",
        "POSTGRES_TASK_LEDGER_COMMAND_SUBSTITUTED",
        "POSTGRES_TASK_LEDGER_ADMISSION_DENIED",
        "POSTGRES_TASK_LEDGER_AUTHORITY_MISMATCH",
        "POSTGRES_TASK_LEDGER_PHYSICAL_STATE_MISMATCH",
        "POSTGRES_TASK_LEDGER_CHECKPOINT_CORRUPT",
        "POSTGRES_TASK_LEDGER_RETAINED_ROW_CORRUPT",
        "POSTGRES_TASK_LEDGER_UNSUPPORTED_RETAINED_SCHEMA",
        "POSTGRES_TASK_LEDGER_REVISION_OVERFLOW",
        "POSTGRES_TASK_LEDGER_SERIALIZATION_EXHAUSTED",
        "POSTGRES_TASK_LEDGER_TRANSACTION_FAILED",
        "POSTGRES_TASK_LEDGER_UNAVAILABLE",
        "POSTGRES_TASK_LEDGER_COMMIT_OUTCOME_UNKNOWN",
    ];

    assert_eq!(PostgresTaskLedgerErrorKind::ALL.len(), expected.len());
    for (kind, code) in PostgresTaskLedgerErrorKind::ALL.into_iter().zip(expected) {
        let error = PostgresTaskLedgerError::new(kind);
        assert_eq!(error.kind(), kind);
        assert_eq!(error.code(), code);
        assert_eq!(error.to_string(), code);
        assert!(!format!("{error:?}").contains("postgres://"));
    }
}

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn required_environment(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn connect_as(database: &str, role: &str) -> Client {
    let mut config = Config::new();
    let login_role = format!("{role}_login");
    config
        .host(&required_environment("LATTICE_TASK019_HOST"))
        .port(
            required_environment("LATTICE_TASK019_PORT")
                .parse::<u16>()
                .expect("port"),
        )
        .user(&login_role)
        .password(required_environment("LATTICE_TASK019_PASSWORD"))
        .dbname(database)
        .application_name("lattice-devos-task019")
        .ssl_mode(SslMode::Disable);
    let mut client = config.connect(NoTls).expect("fixed role connection");
    client
        .batch_execute(&format!("SET ROLE {role}"))
        .expect("fixed role activation");
    client
}

fn connect_superuser(database: &str) -> Client {
    let mut config = Config::new();
    config
        .host(&required_environment("LATTICE_TASK019_HOST"))
        .port(
            required_environment("LATTICE_TASK019_PORT")
                .parse::<u16>()
                .expect("port"),
        )
        .user("task019_harness")
        .password(required_environment("LATTICE_TASK019_PASSWORD"))
        .dbname(database)
        .application_name("lattice-devos-task050-provision")
        .ssl_mode(SslMode::Disable);
    config.connect(NoTls).expect("harness superuser connection")
}

#[allow(clippy::too_many_lines)]
fn provision_fresh_database(target: &MigrationTarget) {
    let password = required_environment("LATTICE_TASK019_PASSWORD");
    let mut admin = connect_superuser("postgres");
    let quoted_password = admin
        .query_one("SELECT quote_literal($1::text)", &[&password])
        .expect("quote password")
        .get::<_, String>(0);
    admin
        .batch_execute(&format!(
            "CREATE ROLE lattice_migrator NOLOGIN NOSUPERUSER INHERIT NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_runtime NOLOGIN NOSUPERUSER INHERIT NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_guardian NOLOGIN NOSUPERUSER INHERIT NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_readonly NOLOGIN NOSUPERUSER INHERIT NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_migrator_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_runtime_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_guardian_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_readonly_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB \
                 NOCREATEROLE NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
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
        .expect("fixed role provisioning");
    admin
        .batch_execute(&format!(
            "CREATE DATABASE {} OWNER lattice_migrator",
            target.database_name()
        ))
        .expect("fixed database provisioning");
    admin
        .batch_execute(&format!(
            "REVOKE ALL ON DATABASE {} FROM PUBLIC; \
             GRANT CONNECT ON DATABASE {} TO lattice_migrator, lattice_runtime, \
                 lattice_guardian, lattice_readonly, lattice_migrator_login, \
                 lattice_runtime_login, lattice_guardian_login, lattice_readonly_login; \
             SET ROLE lattice_migrator; COMMENT ON DATABASE {} IS '{}'; RESET ROLE",
            target.database_name(),
            target.database_name(),
            target.database_name(),
            target.database_comment()
        ))
        .expect("database boundary");
    drop(admin);

    let mut target_admin = connect_superuser(target.database_name());
    target_admin
        .batch_execute(
            "REVOKE ALL PRIVILEGES ON FUNCTION \
                 pg_catalog.lo_creat(integer), pg_catalog.lo_create(oid), \
                 pg_catalog.lo_from_bytea(oid, bytea), pg_catalog.lo_import(text), \
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
                 pg_catalog.pg_export_snapshot(), pg_catalog.pg_current_xact_id(), \
                 pg_catalog.txid_current() \
             FROM PUBLIC, lattice_migrator, lattice_runtime, lattice_guardian, \
                 lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             GRANT EXECUTE ON FUNCTION pg_catalog.pg_try_advisory_lock(bigint), \
                 pg_catalog.pg_advisory_xact_lock(bigint), \
                 pg_catalog.pg_current_xact_id() TO lattice_migrator",
        )
        .expect("pre-role function boundary");

    let mut migrator = connect_as(target.database_name(), "lattice_migrator");
    assert_eq!(
        apply_migrations(&mut migrator, target).expect("fresh migration"),
        MigrationApplyOutcome::Applied {
            executable_count: 5
        }
    );
    let evidence = verify_postgres_schema(&mut migrator, target, DatabaseRole::Migrator)
        .expect("fresh schema verification");
    println!(
        "TASK019_EVIDENCE database_uuid={} manifest_sha256={}",
        evidence.database_uuid(),
        evidence.manifest_sha256().as_str()
    );
}

fn provision_upgrade_database(target: &MigrationTarget) {
    let mut admin = connect_superuser("postgres");
    admin
        .batch_execute(&format!(
            "CREATE DATABASE {} OWNER lattice_migrator",
            target.database_name()
        ))
        .expect("upgrade database create");
    admin
        .batch_execute(&format!(
            "REVOKE ALL ON DATABASE {} FROM PUBLIC; \
             GRANT CONNECT ON DATABASE {} TO lattice_migrator, lattice_runtime, \
                 lattice_guardian, lattice_readonly, lattice_migrator_login, \
                 lattice_runtime_login, lattice_guardian_login, lattice_readonly_login; \
             SET ROLE lattice_migrator; COMMENT ON DATABASE {} IS '{}'; RESET ROLE",
            target.database_name(),
            target.database_name(),
            target.database_name(),
            target.database_comment()
        ))
        .expect("upgrade database boundary");
    drop(admin);

    set_exact_database_access(target.database_name());

    connect_superuser(target.database_name())
        .batch_execute(
            "REVOKE ALL PRIVILEGES ON FUNCTION \
                 pg_catalog.lo_creat(integer), pg_catalog.lo_create(oid), \
                 pg_catalog.lo_from_bytea(oid, bytea), pg_catalog.lo_import(text), \
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
                 pg_catalog.pg_export_snapshot(), pg_catalog.pg_current_xact_id(), \
                 pg_catalog.txid_current() \
             FROM PUBLIC, lattice_migrator, lattice_runtime, lattice_guardian, \
                 lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             GRANT EXECUTE ON FUNCTION pg_catalog.pg_try_advisory_lock(bigint), \
                 pg_catalog.pg_advisory_xact_lock(bigint), \
                 pg_catalog.pg_current_xact_id() TO lattice_migrator",
        )
        .expect("upgrade pre-role function boundary");
}

fn set_exact_database_access(target_database: &str) {
    let mut admin = connect_superuser("postgres");
    let databases: Vec<String> = admin
        .query(
            "SELECT datname::text FROM pg_database ORDER BY datname",
            &[],
        )
        .expect("database inventory")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    for database in databases {
        let quoted: String = admin
            .query_one("SELECT quote_ident($1::text)", &[&database])
            .expect("quoted database")
            .get(0);
        admin
            .batch_execute(&format!(
                "REVOKE ALL ON DATABASE {quoted} FROM PUBLIC; \
                 REVOKE ALL ON DATABASE {quoted} FROM lattice_migrator_login, \
                     lattice_runtime_login, lattice_guardian_login, lattice_readonly_login"
            ))
            .expect("database access revoke");
    }
    let quoted_target: String = admin
        .query_one("SELECT quote_ident($1::text)", &[&target_database])
        .expect("quoted target database")
        .get(0);
    admin
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; \
             GRANT CONNECT ON DATABASE {quoted_target} TO lattice_migrator, lattice_runtime, \
                 lattice_guardian, lattice_readonly, lattice_migrator_login, \
                 lattice_runtime_login, lattice_guardian_login, lattice_readonly_login; \
             RESET ROLE"
        ))
        .expect("database access grant");
}

fn prefix_manifest_sha256(prefix_len: usize) -> String {
    fn field(hasher: &mut Sha256, value: &[u8]) {
        hasher.update(
            u64::try_from(value.len())
                .expect("field length")
                .to_be_bytes(),
        );
        hasher.update(value);
    }

    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_POSTGRES_MIGRATION_MANIFEST_V1\0");
    for entry in &migration_manifest()[..prefix_len] {
        field(&mut hasher, &entry.ordinal().to_be_bytes());
        field(&mut hasher, entry.id().as_bytes());
        field(&mut hasher, entry.path().as_bytes());
        field(
            &mut hasher,
            &u64::try_from(entry.byte_length())
                .expect("migration byte length")
                .to_be_bytes(),
        );
        field(&mut hasher, entry.sha256().as_bytes());
        field(&mut hasher, entry.status().as_str().as_bytes());
        field(&mut hasher, entry.transaction_mode().as_str().as_bytes());
        for value in [
            entry.schema_version(),
            *entry.reader_compatibility().start(),
            *entry.reader_compatibility().end(),
            *entry.writer_compatibility().start(),
            *entry.writer_compatibility().end(),
        ] {
            field(&mut hasher, &value.to_be_bytes());
        }
    }
    let hex = b"0123456789abcdef";
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(char::from(hex[usize::from(byte >> 4)]));
        encoded.push(char::from(hex[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn install_exact_prefix(target: &MigrationTarget, prefix_len: usize, manifest_sha256: &str) {
    assert!(matches!(prefix_len, 2..=4));
    let schema_version = i16::try_from(migration_manifest()[prefix_len - 1].schema_version())
        .expect("schema version");
    let mut client = connect_as(target.database_name(), "lattice_migrator");
    let mut transaction = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::ReadCommitted)
        .start()
        .expect("prefix transaction");
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .expect("prefix settings");
    for entry in migration_manifest().iter().take(prefix_len).skip(1) {
        assert_eq!(entry.status(), MigrationStatus::Executable);
        transaction
            .batch_execute(std::str::from_utf8(entry.bytes()).expect("prefix utf8"))
            .expect("prefix migration");
    }
    for entry in &migration_manifest()[..prefix_len] {
        transaction
            .execute(
                "INSERT INTO control.migration_history (ordinal, migration_id, migration_path, \
                    byte_length, checksum_sha256, migration_status, transaction_mode, \
                    schema_version, min_reader, max_reader, min_writer, max_writer) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
                &[
                    &i16::try_from(entry.ordinal()).expect("ordinal"),
                    &entry.id(),
                    &entry.path(),
                    &i64::try_from(entry.byte_length()).expect("length"),
                    &entry.sha256(),
                    &entry.status().as_str(),
                    &entry.transaction_mode().as_str(),
                    &i16::try_from(entry.schema_version()).expect("entry schema"),
                    &i16::try_from(*entry.reader_compatibility().start()).expect("min reader"),
                    &i16::try_from(*entry.reader_compatibility().end()).expect("max reader"),
                    &i16::try_from(*entry.writer_compatibility().start()).expect("min writer"),
                    &i16::try_from(*entry.writer_compatibility().end()).expect("max writer"),
                ],
            )
            .expect("prefix history");
    }
    transaction
        .execute(
            "INSERT INTO control.schema_compatibility (singleton, manifest_sha256, \
                current_schema_version, min_reader, max_reader, min_writer, max_writer) \
             VALUES (true,$1,$2,$2,$2,$2,$2)",
            &[&manifest_sha256, &schema_version],
        )
        .expect("prefix compatibility");
    transaction
        .execute(
            "INSERT INTO control.database_identity (singleton, database_uuid) \
             VALUES (true,$1::text::uuid)",
            &[&target.expected_database_uuid()],
        )
        .expect("prefix identity");
    transaction.commit().expect("prefix commit");
}

fn install_memory_v2_prefix(target: &MigrationTarget) {
    let mut client = connect_as(target.database_name(), "lattice_migrator");
    let mut transaction = client
        .build_transaction()
        .isolation_level(postgres::IsolationLevel::ReadCommitted)
        .start()
        .expect("memory fixture transaction");
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .expect("memory fixture settings");
    transaction
        .batch_execute(include_str!(
            "../../../db/extensions/codebase-memory/v2.sql"
        ))
        .expect("memory fixture schema");
    let global_manifest = prefix_manifest_sha256(4);
    let sql_sha256 = "9db54342b88f554ca76054c7a33ae72f04b412d2dfe21fae6eb4d8faf3e854e2";
    let extension_manifest = "0aedbd7d9ef7ca07fc2910d0da34c163cc83e3dd56f9b28292ae1f4f0c3c4d7e";
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_identity (singleton, extension_id, \
                extension_schema_version, extension_path, extension_sql_sha256, \
                extension_manifest_sha256, database_uuid, database_identity_sha256, \
                global_schema_version, global_manifest_sha256) \
             VALUES (true,'lattice-codebase-memory',2, \
                'db/extensions/codebase-memory/v2.sql',$1,$2,$3::text::uuid,$4,3,$5)",
            &[
                &sql_sha256,
                &extension_manifest,
                &target.expected_database_uuid(),
                &target.expected_database_identity_sha256().as_str(),
                &global_manifest,
            ],
        )
        .expect("memory fixture identity");
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_ledger (ledger_ordinal, singleton, \
                extension_id, extension_schema_version, extension_sql_sha256, \
                extension_manifest_sha256, database_uuid, database_identity_sha256, \
                global_schema_version, global_manifest_sha256, event_kind) \
             VALUES (1,true,'lattice-codebase-memory',2,$1,$2,$3::text::uuid,$4,3,$5,'INSTALLED')",
            &[
                &sql_sha256,
                &extension_manifest,
                &target.expected_database_uuid(),
                &target.expected_database_identity_sha256().as_str(),
                &global_manifest,
            ],
        )
        .expect("memory fixture ledger");
    transaction.commit().expect("memory fixture commit");
}

fn prove_live_upgrade(run_id: &str, suffix: &str, prefix_len: usize, manifest_sha256: &str) {
    let database = format!("lattice_task019_{}_{}", &run_id[..8], suffix);
    let target = MigrationTarget::new(database, run_id.to_owned()).expect("upgrade target");
    provision_upgrade_database(&target);
    install_exact_prefix(&target, prefix_len, manifest_sha256);
    let before: Vec<(i16, String)> = connect_as(target.database_name(), "lattice_migrator")
        .query(
            "SELECT ordinal, checksum_sha256::text FROM ONLY control.migration_history \
             ORDER BY ordinal",
            &[],
        )
        .expect("prefix history read")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    let mut migrator = connect_as(target.database_name(), "lattice_migrator");
    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("prefix upgrade"),
        MigrationApplyOutcome::Applied {
            executable_count: 6 - prefix_len
        }
    );
    verify_postgres_schema(&mut migrator, &target, DatabaseRole::Migrator)
        .expect("upgraded schema");
    let after: Vec<(i16, String)> = migrator
        .query(
            "SELECT ordinal, checksum_sha256::text FROM ONLY control.migration_history \
             WHERE ordinal <= $1 ORDER BY ordinal",
            &[&i16::try_from(prefix_len).expect("prefix bound")],
        )
        .expect("upgraded history read")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    assert_eq!(after, before, "upgrade rewrote historical manifest rows");
}

fn prove_live_memory_upgrade(run_id: &str) {
    let database = format!("lattice_task019_{}_upgrade_memory", &run_id[..8]);
    let target = MigrationTarget::new(database, run_id.to_owned()).expect("memory upgrade target");
    provision_upgrade_database(&target);
    install_exact_prefix(&target, 4, &prefix_manifest_sha256(4));
    install_memory_v2_prefix(&target);
    let mut migrator = connect_as(target.database_name(), "lattice_migrator");
    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("memory prefix upgrade"),
        MigrationApplyOutcome::Applied {
            executable_count: 2
        }
    );
    verify_postgres_schema(&mut migrator, &target, DatabaseRole::Migrator)
        .expect("upgraded memory schema");
}

fn hex_bytes(value: &ContentDigest) -> Vec<u8> {
    value
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex utf8");
            u8::from_str_radix(text, 16).expect("hex byte")
        })
        .collect()
}

fn store_authority() -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new("task050-fresh-process").expect("daemon"),
        DaemonEpoch::new(50).expect("epoch"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(50).expect("revision"),
        digest('a'),
        digest('b'),
    )
    .expect("authority")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AutonomyPersistenceFaultBehavior {
    RaiseAfterWrite,
    CorruptBeforeReload,
    RaiseAtCommit,
}

#[derive(Clone, Copy)]
struct AutonomyPersistenceFaultBoundary {
    name: &'static str,
    table: &'static str,
    operation: &'static str,
    behavior: AutonomyPersistenceFaultBehavior,
}

const AUTONOMY_PERSISTENCE_FAULT_BOUNDARIES: [AutonomyPersistenceFaultBoundary; 8] = [
    AutonomyPersistenceFaultBoundary {
        name: "physical_head",
        table: "physical_heads",
        operation: "AFTER UPDATE",
        behavior: AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
    },
    AutonomyPersistenceFaultBoundary {
        name: "physical_store_receipt",
        table: "terminal_transactions",
        operation: "AFTER INSERT",
        behavior: AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
    },
    AutonomyPersistenceFaultBoundary {
        name: "ledger_head_projection_checkpoint",
        table: "task_ledger_streams",
        operation: "AFTER UPDATE",
        behavior: AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
    },
    AutonomyPersistenceFaultBoundary {
        name: "ledger_command_receipt",
        table: "task_ledger_commands",
        operation: "AFTER INSERT",
        behavior: AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
    },
    AutonomyPersistenceFaultBoundary {
        name: "ledger_event",
        table: "task_ledger_events",
        operation: "AFTER INSERT",
        behavior: AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
    },
    AutonomyPersistenceFaultBoundary {
        name: "autonomy_subject",
        table: "task_ledger_autonomy_receipts",
        operation: "AFTER INSERT",
        behavior: AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
    },
    AutonomyPersistenceFaultBoundary {
        name: "reload_verification",
        table: "task_ledger_autonomy_receipts",
        operation: "AFTER INSERT",
        behavior: AutonomyPersistenceFaultBehavior::CorruptBeforeReload,
    },
    AutonomyPersistenceFaultBoundary {
        name: "transaction_commit",
        table: "task_ledger_autonomy_receipts",
        operation: "AFTER INSERT",
        behavior: AutonomyPersistenceFaultBehavior::RaiseAtCommit,
    },
];

#[derive(Debug, Eq, PartialEq)]
struct AutonomyPersistenceSnapshot {
    physical_heads: Vec<String>,
    terminal_transactions: Vec<String>,
    streams: Vec<String>,
    commands: Vec<String>,
    events: Vec<String>,
    outbox: Vec<String>,
    autonomy_receipts: Vec<String>,
}

fn json_rows(client: &mut Client, sql: &str, stream_id: &[u8]) -> Vec<String> {
    client
        .query(sql, &[&stream_id])
        .expect("durable boundary snapshot")
        .into_iter()
        .map(|row| row.get(0))
        .collect()
}

fn autonomy_persistence_snapshot(
    client: &mut Client,
    stream_id: &ContentDigest,
) -> AutonomyPersistenceSnapshot {
    let stream_id = hex_bytes(stream_id);
    AutonomyPersistenceSnapshot {
        physical_heads: json_rows(
            client,
            "SELECT pg_catalog.to_jsonb(retained)::text FROM (\
                 SELECT * FROM ONLY control.physical_heads \
                 WHERE aggregate_key_digest=$1::bytea \
                 ORDER BY project_id, project_snapshot_id, repository_owner\
             ) AS retained",
            &stream_id,
        ),
        terminal_transactions: json_rows(
            client,
            "SELECT pg_catalog.to_jsonb(retained)::text FROM (\
                 SELECT * FROM ONLY control.terminal_transactions \
                 WHERE aggregate_key_digest=$1::bytea ORDER BY transaction_id\
             ) AS retained",
            &stream_id,
        ),
        streams: json_rows(
            client,
            "SELECT pg_catalog.to_jsonb(retained)::text FROM (\
                 SELECT * FROM ONLY control.task_ledger_streams \
                 WHERE stream_id=$1::bytea ORDER BY stream_id\
             ) AS retained",
            &stream_id,
        ),
        commands: json_rows(
            client,
            "SELECT pg_catalog.to_jsonb(retained)::text FROM (\
                 SELECT * FROM ONLY control.task_ledger_commands \
                 WHERE stream_id=$1::bytea ORDER BY command_id\
             ) AS retained",
            &stream_id,
        ),
        events: json_rows(
            client,
            "SELECT pg_catalog.to_jsonb(retained)::text FROM (\
                 SELECT * FROM ONLY control.task_ledger_events \
                 WHERE stream_id=$1::bytea ORDER BY sequence\
             ) AS retained",
            &stream_id,
        ),
        outbox: json_rows(
            client,
            "SELECT pg_catalog.to_jsonb(retained)::text FROM (\
                 SELECT * FROM ONLY control.task_ledger_outbox \
                 WHERE stream_id=$1::bytea ORDER BY event_sequence\
             ) AS retained",
            &stream_id,
        ),
        autonomy_receipts: json_rows(
            client,
            "SELECT pg_catalog.to_jsonb(retained)::text FROM (\
                 SELECT * FROM ONLY control.task_ledger_autonomy_receipts \
                 WHERE stream_id=$1::bytea ORDER BY event_sequence\
             ) AS retained",
            &stream_id,
        ),
    }
}

fn create_autonomy_persistence_fault_function(client: &mut Client) {
    client
        .batch_execute(
            "CREATE FUNCTION control.task050_raise_autonomy_persistence_fault() \
             RETURNS trigger LANGUAGE plpgsql AS $task050$ \
             BEGIN \
                 IF TG_NAME = 'task050_autonomy_reload_corruption' THEN \
                     UPDATE ONLY control.task_ledger_autonomy_receipts \
                        SET task_kind = CASE task_kind \
                            WHEN 'FEATURE' THEN 'BUG_FIX' ELSE 'FEATURE' END \
                      WHERE stream_id = NEW.stream_id \
                        AND event_sequence = NEW.event_sequence; \
                     RETURN NEW; \
                 END IF; \
                 RAISE EXCEPTION USING ERRCODE='P0500', \
                     MESSAGE='TASK050_INJECTED_PERSISTENCE_FAULT'; \
             END \
             $task050$; \
             REVOKE ALL ON FUNCTION control.task050_raise_autonomy_persistence_fault() \
                 FROM PUBLIC",
        )
        .expect("fault function");
}

const fn autonomy_persistence_fault_trigger_name(
    behavior: AutonomyPersistenceFaultBehavior,
) -> &'static str {
    match behavior {
        AutonomyPersistenceFaultBehavior::RaiseAfterWrite => "task050_autonomy_persistence_fault",
        AutonomyPersistenceFaultBehavior::CorruptBeforeReload => {
            "task050_autonomy_reload_corruption"
        }
        AutonomyPersistenceFaultBehavior::RaiseAtCommit => "task050_autonomy_commit_fault",
    }
}

fn install_autonomy_persistence_fault(
    client: &mut Client,
    boundary: AutonomyPersistenceFaultBoundary,
) {
    let trigger_name = autonomy_persistence_fault_trigger_name(boundary.behavior);
    let trigger_kind = if boundary.behavior == AutonomyPersistenceFaultBehavior::RaiseAtCommit {
        "CREATE CONSTRAINT TRIGGER"
    } else {
        "CREATE TRIGGER"
    };
    let deferral = if boundary.behavior == AutonomyPersistenceFaultBehavior::RaiseAtCommit {
        "DEFERRABLE INITIALLY DEFERRED"
    } else {
        ""
    };
    client
        .batch_execute(&format!(
            "{trigger_kind} {trigger_name} {} ON control.{} {deferral} \
             FOR EACH ROW EXECUTE FUNCTION control.task050_raise_autonomy_persistence_fault()",
            boundary.operation, boundary.table,
        ))
        .unwrap_or_else(|error| panic!("install {} fault: {error}", boundary.name));
}

fn remove_autonomy_persistence_fault(
    client: &mut Client,
    boundary: AutonomyPersistenceFaultBoundary,
) {
    let trigger_name = autonomy_persistence_fault_trigger_name(boundary.behavior);
    client
        .batch_execute(&format!(
            "DROP TRIGGER {trigger_name} ON control.{}",
            boundary.table,
        ))
        .unwrap_or_else(|error| panic!("remove {} fault: {error}", boundary.name));
}

fn drop_autonomy_persistence_fault_function(client: &mut Client) {
    client
        .batch_execute("DROP FUNCTION control.task050_raise_autonomy_persistence_fault()")
        .expect("drop fault function");
}

#[test]
fn autonomy_persistence_fault_matrix_covers_every_durable_boundary() {
    let boundaries: Vec<_> = AUTONOMY_PERSISTENCE_FAULT_BOUNDARIES
        .iter()
        .map(|boundary| {
            (
                boundary.name,
                boundary.table,
                boundary.operation,
                boundary.behavior,
            )
        })
        .collect();
    assert_eq!(
        boundaries,
        [
            (
                "physical_head",
                "physical_heads",
                "AFTER UPDATE",
                AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
            ),
            (
                "physical_store_receipt",
                "terminal_transactions",
                "AFTER INSERT",
                AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
            ),
            (
                "ledger_head_projection_checkpoint",
                "task_ledger_streams",
                "AFTER UPDATE",
                AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
            ),
            (
                "ledger_command_receipt",
                "task_ledger_commands",
                "AFTER INSERT",
                AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
            ),
            (
                "ledger_event",
                "task_ledger_events",
                "AFTER INSERT",
                AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
            ),
            (
                "autonomy_subject",
                "task_ledger_autonomy_receipts",
                "AFTER INSERT",
                AutonomyPersistenceFaultBehavior::RaiseAfterWrite,
            ),
            (
                "reload_verification",
                "task_ledger_autonomy_receipts",
                "AFTER INSERT",
                AutonomyPersistenceFaultBehavior::CorruptBeforeReload,
            ),
            (
                "transaction_commit",
                "task_ledger_autonomy_receipts",
                "AFTER INSERT",
                AutonomyPersistenceFaultBehavior::RaiseAtCommit,
            ),
        ]
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn autonomy_receipt_survives_postgres_restart_and_fresh_process_when_provisioned() {
    if std::env::var("LATTICE_TASK050_LIVE").ok().as_deref() != Some("1") {
        eprintln!("SKIP: LATTICE_TASK050_LIVE is not configured");
        return;
    }
    assert_eq!(required_environment("LATTICE_TASK019_LIVE"), "1");
    let phase = required_environment("LATTICE_TASK019_PHASE");
    assert!(matches!(phase.as_str(), "initial" | "restart"));
    let run_id = required_environment("LATTICE_TASK019_RUN_ID");
    let database = format!("lattice_task019_{}_base", &run_id[..8]);
    let target = MigrationTarget::new(database.clone(), run_id.clone()).expect("target");
    let authority = store_authority();

    if phase == "initial" {
        provision_fresh_database(&target);
        prove_live_upgrade(&run_id, "upgrade_v1", 2, &prefix_manifest_sha256(2));
        prove_live_upgrade(&run_id, "upgrade_v2", 3, &prefix_manifest_sha256(3));
        prove_live_upgrade(&run_id, "upgrade_v3", 4, &prefix_manifest_sha256(4));
        prove_live_memory_upgrade(&run_id);
        println!("TASK050_POSTGRES_UPGRADE_V1_V2_V3_OK");
        set_exact_database_access(&database);
        let mut migrator = connect_as(&database, "lattice_migrator");
        assert_eq!(
            migrator
                .execute(
                    "UPDATE ONLY control.runtime_admission SET admission_mode='ACTIVE', \
                     daemon_instance_id='task050-fresh-process', daemon_epoch=50, \
                     authority_revision=50, observation_digest=$1::bytea, \
                     authority_head_digest=$2::bytea, updated_at=clock_timestamp() \
                     WHERE singleton=true",
                    &[&hex_bytes(&digest('a')), &hex_bytes(&digest('b'))],
                )
                .expect("activate disposable runtime"),
            1
        );
    }

    let identity = TaskLedgerStreamIdentity::new(
        ProjectId::new("task050-fresh-process").expect("project"),
        ProjectSnapshotId::new("task050-snapshot").expect("snapshot"),
        TaskId::new("TASK-050-FRESH").expect("task"),
        "1",
        digest('e'),
        "TWD",
    )
    .expect("identity");
    let runtime = connect_as(&database, "lattice_runtime");
    let mut ledger = PostgresTaskLedger::new(runtime, &target).expect("ledger");

    if phase == "initial" {
        let vacant = ledger.load_stream(identity.clone()).expect("vacant");
        let create = AppendCommand::new_autonomy_required_task_created(
            vacant.stream().head().clone(),
            CommandId::new("task050-create").expect("command"),
            CorrelationId::new("task050-fresh-process").expect("correlation"),
            "2000-01-01T00:00:00Z",
            ActorId::new("task050-local-acceptance").expect("actor"),
            ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
            digest('f'),
            None,
        )
        .expect("required task creation");
        ledger
            .execute(create, authority.clone())
            .expect("task created");
        let created = ledger.load_stream(identity.clone()).expect("created");
        assert_eq!(
            created.autonomy_state(),
            &VerifiedAutonomyReceiptState::PendingRequiredReceipt
        );
        let build_plan = |task_kind| {
            plan_autonomy_receipt_append(
                created.stream(),
                AutonomyAppendMetadata::new(
                    CommandId::new("task050-autonomy-receipt-v1").expect("command"),
                    CorrelationId::new("task050-fresh-process").expect("correlation"),
                    "2000-01-01T00:00:00Z",
                    ActorId::new("task050-local-acceptance").expect("actor"),
                )
                .expect("metadata"),
                AutonomyIntent::new(
                    task_kind,
                    AutonomyRiskClass::R0,
                    false,
                    false,
                    false,
                    AutonomyObservedTaskState::Draft,
                    AutonomyRecommendation::AskUser {
                        reason: AutonomyDecisionReason::NewUserDecision,
                    },
                ),
                AutonomyAuthorityEvidence::new_p0_process_start_profile(
                    digest('c'),
                    digest('d'),
                    digest('b'),
                    None,
                )
                .expect("authority"),
            )
            .expect("typed autonomy plan")
        };
        let plan = build_plan(AutonomyTaskKind::Feature);
        let substitution_plan = build_plan(AutonomyTaskKind::BugFix);
        let receipt_digest = plan.receipt().receipt_digest().clone();
        let exact_retry_plan = plan.clone();
        let mut fault_admin = connect_superuser(&database);
        let baseline =
            autonomy_persistence_snapshot(&mut fault_admin, created.stream().head().stream_id());
        create_autonomy_persistence_fault_function(&mut fault_admin);
        for boundary in AUTONOMY_PERSISTENCE_FAULT_BOUNDARIES {
            install_autonomy_persistence_fault(&mut fault_admin, boundary);
            let failure = ledger
                .execute_autonomy(plan.clone(), authority.clone())
                .expect_err("injected boundary failure must reject the transaction");
            remove_autonomy_persistence_fault(&mut fault_admin, boundary);
            let expected_failure = match boundary.behavior {
                AutonomyPersistenceFaultBehavior::CorruptBeforeReload => {
                    PostgresTaskLedgerErrorKind::RetainedRowCorrupt
                }
                AutonomyPersistenceFaultBehavior::RaiseAfterWrite
                | AutonomyPersistenceFaultBehavior::RaiseAtCommit => {
                    PostgresTaskLedgerErrorKind::TransactionFailed
                }
            };
            assert_eq!(
                failure.kind(),
                expected_failure,
                "{} fault returned the wrong fail-closed class",
                boundary.name
            );
            assert_eq!(
                autonomy_persistence_snapshot(
                    &mut fault_admin,
                    created.stream().head().stream_id()
                ),
                baseline,
                "{} fault left a partial durable record",
                boundary.name
            );
            let after_failure = ledger
                .load_stream(identity.clone())
                .unwrap_or_else(|error| panic!("{} rollback replay: {error}", boundary.name));
            assert_eq!(
                after_failure.stream().events().len(),
                1,
                "{}",
                boundary.name
            );
            assert_eq!(
                after_failure.autonomy_state(),
                &VerifiedAutonomyReceiptState::PendingRequiredReceipt,
                "{}",
                boundary.name
            );
        }
        drop_autonomy_persistence_fault_function(&mut fault_admin);
        println!("TASK050_AUTONOMY_ATOMICITY_FAULT_MATRIX_OK boundaries=8");
        ledger
            .execute_autonomy(plan, authority.clone())
            .expect("atomic autonomy receipt");
        let retry = ledger
            .execute_autonomy(exact_retry_plan, authority.clone())
            .expect("exact autonomy retry");
        assert!(retry.is_exact_retry());
        let substitution = ledger
            .execute_autonomy(substitution_plan, authority)
            .expect_err("changed exact retry must fail closed");
        assert_eq!(
            substitution.kind(),
            PostgresTaskLedgerErrorKind::CommandSubstitution
        );
        let loaded = ledger.load_stream(identity).expect("initial readback");
        assert_eq!(loaded.stream().events().len(), 2);
        assert_eq!(
            loaded.autonomy_receipt().expect("receipt").receipt_digest(),
            &receipt_digest
        );
        println!("TASK050_AUTONOMY_INITIAL_OK");
    } else {
        let loaded = ledger.load_stream(identity).expect("fresh process replay");
        let receipt = loaded.autonomy_receipt().expect("durable receipt");
        let row = receipt.to_untrusted();
        assert_eq!(loaded.stream().events().len(), 2);
        assert_eq!(row.observed_task_state(), "DRAFT");
        assert_eq!(row.disposition(), "ASK_USER");
        assert_eq!(row.decision_reason(), "NEW_USER_DECISION");
        assert_eq!(row.model(), None);
        assert_eq!(row.verification(), None);
        println!("TASK050_AUTONOMY_RESTART_OK");
    }
}
