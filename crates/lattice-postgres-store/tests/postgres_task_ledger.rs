use lattice_contracts::{
    ContentDigest, DaemonEpoch, GitRefIdentity, ProjectClass, ProjectId, ProjectLifecycle,
    ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead,
    StoreAuthorityRevision, StoreDaemonInstanceId, TaskId, TaskLedgerStreamIdentity,
};
use lattice_postgres_store::{
    DatabaseRole, MigrationApplyOutcome, MigrationStatus, MigrationTarget, PostgresProjectRegistry,
    PostgresTaskLedger, PostgresTaskLedgerError, PostgresTaskLedgerErrorKind, apply_migrations,
    migration_manifest, verify_postgres_schema,
};
use lattice_project_registry::{
    CommandId as RegistryCommandId, RegistryCommand as ProjectRegistryCommand,
    RegistryCommandOutcome, RepositoryObservation,
};
use lattice_task_ledger::{
    ActorId, AppendCommand, AutonomyAppendMetadata, AutonomyAuthorityEvidence,
    AutonomyDecisionReason, AutonomyIntent, AutonomyObservedTaskState, AutonomyRecommendation,
    AutonomyRiskClass, AutonomyTaskKind, CommandId, CorrelationId, ReasonCode, TaskIngressClaim,
    TaskSubmissionEnvelope, VerifiedAutonomyReceiptState, plan_autonomy_receipt_append,
};
use postgres::config::SslMode;
use postgres::error::SqlState;
use postgres::{Client, Config, NoTls};
use sha2::{Digest, Sha256};

const TASK_SUBMIT_INGRESS_ID: &str = "lattice_task_submit.v1";

#[test]
fn postgres_task_ledger_errors_are_static_and_exhaustive() {
    let expected = [
        "POSTGRES_TASK_LEDGER_MALFORMED",
        "POSTGRES_TASK_LEDGER_COMMAND_SUBSTITUTED",
        "POSTGRES_TASK_LEDGER_PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
        "POSTGRES_TASK_LEDGER_PROJECT_REGISTRY_INACTIVE",
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
        .user(
            &std::env::var("LATTICE_TASK_SUBMISSION_SUPERUSER")
                .unwrap_or_else(|_| "task019_harness".to_owned()),
        )
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
    assert!(matches!(prefix_len, 2..=7));
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
            executable_count: 9 - prefix_len
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
            executable_count: 4
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

#[derive(Clone)]
struct GeneralProjectFixture {
    project_id: ProjectId,
    project_snapshot_id: ProjectSnapshotId,
    authority_receipt_digest: ContentDigest,
}

fn general_project_observation() -> RepositoryObservation {
    RepositoryObservation::new(
        "C:/lattice/ai-novel",
        digest('1'),
        digest('2'),
        digest('3'),
        GitRefIdentity::new("refs/heads/main", digest('4')).expect("general project ref"),
    )
    .expect("general project observation")
}

fn provision_general_project(database: &str, target: &MigrationTarget) -> GeneralProjectFixture {
    let project_id = ProjectId::new("ai-novel").expect("general project id");
    let observation = general_project_observation();
    let mut registry =
        PostgresProjectRegistry::new(connect_as(database, "lattice_runtime"), target)
            .expect("schema-v7 Project Registry adapter");
    let vacant = registry.load().expect("vacant schema-v7 Project Registry");
    assert_eq!(vacant.persistence().schema_version(), 7);
    assert!(vacant.state().is_vacant());

    let registered = registry
        .execute(
            ProjectRegistryCommand::register(
                RegistryCommandId::new("phase3-ai-novel-register")
                    .expect("general project register command"),
                project_id.clone(),
                ProjectClass::UserProject,
                observation.clone(),
            ),
            store_authority(),
        )
        .expect("register general submission project through exact v7 adapter");
    assert!(matches!(
        registered.semantic_receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    let registered_authority = registered
        .semantic_receipt()
        .authority()
        .expect("registered project authority")
        .clone();

    let observed = registry
        .execute(
            ProjectRegistryCommand::observe(
                RegistryCommandId::new("phase3-ai-novel-observe")
                    .expect("general project observe command"),
                project_id.clone(),
                registered_authority.head(),
                observation.clone(),
            ),
            store_authority(),
        )
        .expect("observe exact general submission project through v7 adapter");
    assert!(matches!(
        observed.semantic_receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    drop(registry);

    let mut reloaded =
        PostgresProjectRegistry::new(connect_as(database, "lattice_runtime"), target)
            .expect("fresh schema-v7 Project Registry adapter");
    let loaded = reloaded.load().expect("reload schema-v7 Project Registry");
    assert_eq!(loaded.persistence().schema_version(), 7);
    let projection = loaded
        .state()
        .project(&project_id)
        .expect("reloaded current general project");
    assert_eq!(projection.observation(), &observation);
    assert!(projection.pending_observation().is_none());
    assert!(projection.drift().is_empty());
    assert_eq!(projection.authority().lifecycle(), ProjectLifecycle::Active);
    assert_eq!(projection.authority().runtime(), RuntimeKind::Live);

    GeneralProjectFixture {
        project_id,
        project_snapshot_id: projection.authority().project_snapshot_id().clone(),
        authority_receipt_digest: projection.authority().receipt_digest().clone(),
    }
}

fn general_submission_identity(
    project: &GeneralProjectFixture,
    task_id: &str,
) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new_general_task_intake(
        project.project_id.clone(),
        project.project_snapshot_id.clone(),
        TaskId::new(task_id).expect("task"),
        "1",
        digest('e'),
    )
    .expect("general submission identity")
}

fn general_submission(
    client_request_id: &str,
    objective: &str,
    identity: TaskLedgerStreamIdentity,
    project: &GeneralProjectFixture,
) -> TaskSubmissionEnvelope {
    TaskSubmissionEnvelope::new(
        TASK_SUBMIT_INGRESS_ID,
        client_request_id,
        objective,
        "AI 劇本",
        identity,
        project.authority_receipt_digest.clone(),
    )
    .expect("general submission")
}

fn general_submission_command(submission: &TaskSubmissionEnvelope) -> AppendCommand {
    let vacant = lattice_task_ledger::VerifiedStream::vacant(
        submission.identity().clone(),
        RuntimeKind::Live,
    )
    .expect("vacant general stream");
    AppendCommand::new_general_task_created(
        vacant.head().clone(),
        CommandId::new(format!("mcp-submit:{}", submission.client_request_id())).expect("command"),
        CorrelationId::new("general-submission-acceptance").expect("correlation"),
        "2026-08-26T00:00:00Z",
        ActorId::new("lattice-runtime").expect("actor"),
        submission,
    )
    .expect("general task created command")
}

fn controlled_canary_identity(task_id: &str) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new(
        ProjectId::new("ai-novel").expect("project"),
        ProjectSnapshotId::new("ai-novel:snapshot:acceptance").expect("snapshot"),
        TaskId::new(task_id).expect("task"),
        "1",
        digest('7'),
        "TWD",
    )
    .expect("canary identity")
}

fn controlled_canary_ingress(
    client_request_id: &str,
    task_id: &str,
) -> (AppendCommand, TaskIngressClaim) {
    let stream = lattice_task_ledger::VerifiedStream::vacant(
        controlled_canary_identity(task_id),
        RuntimeKind::Live,
    )
    .expect("vacant canary stream");
    let command = AppendCommand::new_autonomy_required_task_created(
        stream.head().clone(),
        CommandId::new(format!("mcp-submit:{client_request_id}")).expect("command"),
        CorrelationId::new("controlled-canary-ingress-race").expect("correlation"),
        "2026-08-26T00:00:00Z",
        ActorId::new("lattice-runtime").expect("actor"),
        ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
        digest('8'),
        None,
    )
    .expect("canary task created command");
    let claim = TaskIngressClaim::controlled_canary(
        TASK_SUBMIT_INGRESS_ID,
        client_request_id,
        stream.head().stream_id().clone(),
    )
    .expect("canary claim");
    (command, claim)
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

#[allow(clippy::too_many_lines)]
fn prove_task_ingress_claim_races(
    database: &str,
    run_id: &str,
    target: &MigrationTarget,
    project: &GeneralProjectFixture,
) {
    let mut ledger = PostgresTaskLedger::new(connect_as(database, "lattice_runtime"), target)
        .expect("ingress race ledger");

    let canary_first_id = "ingress-claim-canary-first";
    let (canary_first_command, canary_first_claim) =
        controlled_canary_ingress(canary_first_id, "TASK-CANARY-FIRST");
    ledger
        .execute_task_ingress(canary_first_command, store_authority(), canary_first_claim)
        .expect("canary first claim");
    let loaded_canary = ledger
        .load_ingress_claim_by_request(TASK_SUBMIT_INGRESS_ID, canary_first_id)
        .expect("neutral canary claim preflight")
        .expect("retained canary claim");
    assert_eq!(
        loaded_canary.request_kind(),
        lattice_task_ledger::TaskIngressRequestKind::ControlledCodexCanary
    );
    let general_after_canary = general_submission(
        canary_first_id,
        "完成角色系統",
        general_submission_identity(project, "TASK-GENERAL-AFTER-CANARY"),
        project,
    );
    let conflict = ledger
        .execute_submission(
            general_submission_command(&general_after_canary),
            store_authority(),
            general_after_canary,
        )
        .expect_err("general must not reuse a canary ingress key");
    assert_eq!(
        conflict.kind(),
        PostgresTaskLedgerErrorKind::CommandSubstitution
    );

    let general_distinct_id = "ingress-claim-general-after-distinct-canary";
    let general_distinct = general_submission(
        general_distinct_id,
        "完成角色系統",
        general_submission_identity(project, "TASK-GENERAL-AFTER-DISTINCT-CANARY"),
        project,
    );
    ledger
        .execute_submission(
            general_submission_command(&general_distinct),
            store_authority(),
            general_distinct,
        )
        .expect("a canary claim under key A must not block a general claim under key B");
    let loaded_general = ledger
        .load_ingress_claim_by_request(TASK_SUBMIT_INGRESS_ID, general_distinct_id)
        .expect("neutral general claim preflight")
        .expect("retained general claim");
    assert_eq!(
        loaded_general.request_kind(),
        lattice_task_ledger::TaskIngressRequestKind::GeneralTask
    );

    let mut marker_admin = connect_superuser(database);
    assert_eq!(
        marker_admin
            .execute(
                "UPDATE ONLY control.task_ledger_events AS e \
                 SET action_id='GENERAL_TASK_INTAKE_V1' \
                 FROM ONLY control.task_ingress_claims AS c \
                 WHERE c.ingress_id=$1 AND c.client_request_id=$2 \
                   AND e.stream_id=c.stream_id AND e.sequence=c.event_sequence",
                &[&TASK_SUBMIT_INGRESS_ID, &canary_first_id],
            )
            .expect("corrupt retained canary marker"),
        1
    );
    assert_eq!(
        ledger
            .load_ingress_claim_by_request(TASK_SUBMIT_INGRESS_ID, canary_first_id)
            .expect_err("request kind and TaskCreated marker must agree")
            .kind(),
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt
    );
    assert_eq!(
        marker_admin
            .execute(
                "UPDATE ONLY control.task_ledger_events AS e \
                 SET action_id='CONTROLLED_CODEX_CANARY_AUTONOMY_V1' \
                 FROM ONLY control.task_ingress_claims AS c \
                 WHERE c.ingress_id=$1 AND c.client_request_id=$2 \
                   AND e.stream_id=c.stream_id AND e.sequence=c.event_sequence",
                &[&TASK_SUBMIT_INGRESS_ID, &canary_first_id],
            )
            .expect("restore retained canary marker"),
        1
    );
    let substituted_canary_id = "ingress-claim-canary-substituted";
    assert_eq!(
        marker_admin
            .execute(
                "UPDATE ONLY control.task_ingress_claims \
                 SET client_request_id=$1 \
                 WHERE ingress_id=$2 AND client_request_id=$3",
                &[
                    &substituted_canary_id,
                    &TASK_SUBMIT_INGRESS_ID,
                    &canary_first_id,
                ],
            )
            .expect("substitute retained canary client key"),
        1
    );
    assert_eq!(
        ledger
            .load_ingress_claim_by_request(TASK_SUBMIT_INGRESS_ID, substituted_canary_id)
            .expect_err("claim client key and command id must agree")
            .kind(),
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt
    );
    assert_eq!(
        marker_admin
            .execute(
                "UPDATE ONLY control.task_ingress_claims \
                 SET client_request_id=$1 \
                 WHERE ingress_id=$2 AND client_request_id=$3",
                &[
                    &canary_first_id,
                    &TASK_SUBMIT_INGRESS_ID,
                    &substituted_canary_id,
                ],
            )
            .expect("restore retained canary client key"),
        1
    );
    drop(marker_admin);
    assert_eq!(
        ledger
            .load_ingress_claim_by_request(TASK_SUBMIT_INGRESS_ID, canary_first_id)
            .expect("restored canary preflight")
            .expect("restored canary claim")
            .request_kind(),
        lattice_task_ledger::TaskIngressRequestKind::ControlledCodexCanary
    );

    let general_first_id = "ingress-claim-general-first";
    let general_first = general_submission(
        general_first_id,
        "完成角色系統",
        general_submission_identity(project, "TASK-GENERAL-FIRST"),
        project,
    );
    ledger
        .execute_submission(
            general_submission_command(&general_first),
            store_authority(),
            general_first,
        )
        .expect("general first claim");
    let fresh_verifier = PostgresTaskLedger::new(connect_as(database, "lattice_runtime"), target)
        .expect("general task envelope must remain outside historical closure");
    drop(fresh_verifier);
    let (canary_after_general_command, canary_after_general_claim) =
        controlled_canary_ingress(general_first_id, "TASK-CANARY-AFTER-GENERAL");
    let conflict = ledger
        .execute_task_ingress(
            canary_after_general_command,
            store_authority(),
            canary_after_general_claim,
        )
        .expect_err("canary must not reuse a general ingress key");
    assert_eq!(
        conflict.kind(),
        PostgresTaskLedgerErrorKind::CommandSubstitution
    );
    drop(ledger);

    let concurrent_id = "ingress-claim-cross-race";
    let concurrent_general = general_submission(
        concurrent_id,
        "完成角色系統",
        general_submission_identity(project, "TASK-GENERAL-CROSS-RACE"),
        project,
    );
    let concurrent_general_stream = concurrent_general.stream_id().clone();
    let general_command = general_submission_command(&concurrent_general);
    let (canary_command, canary_claim) =
        controlled_canary_ingress(concurrent_id, "TASK-CANARY-CROSS-RACE");
    let canary_stream = canary_claim.stream_id().clone();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let general_barrier = barrier.clone();
    let general_database = database.to_owned();
    let general_run_id = run_id.to_owned();
    let general_handle = std::thread::spawn(move || {
        let target = MigrationTarget::new(general_database.clone(), general_run_id)
            .expect("general race target");
        let mut ledger =
            PostgresTaskLedger::new(connect_as(&general_database, "lattice_runtime"), &target)
                .expect("general race ledger");
        general_barrier.wait();
        ledger
            .execute_submission(general_command, store_authority(), concurrent_general)
            .map(|_| "GENERAL")
            .map_err(PostgresTaskLedgerError::kind)
    });
    let canary_barrier = barrier.clone();
    let canary_database = database.to_owned();
    let canary_run_id = run_id.to_owned();
    let canary_handle = std::thread::spawn(move || {
        let target =
            MigrationTarget::new(canary_database.clone(), canary_run_id).expect("canary target");
        let mut ledger =
            PostgresTaskLedger::new(connect_as(&canary_database, "lattice_runtime"), &target)
                .expect("canary race ledger");
        canary_barrier.wait();
        ledger
            .execute_task_ingress(canary_command, store_authority(), canary_claim)
            .map(|_| "CANARY")
            .map_err(PostgresTaskLedgerError::kind)
    });
    barrier.wait();
    let race_results = [
        general_handle.join().expect("general race thread"),
        canary_handle.join().expect("canary race thread"),
    ];
    assert_eq!(
        race_results.iter().filter(|result| result.is_ok()).count(),
        1
    );
    assert_eq!(
        race_results
            .iter()
            .filter(|result| {
                result.as_ref().err() == Some(&PostgresTaskLedgerErrorKind::CommandSubstitution)
            })
            .count(),
        1
    );
    let general_won = race_results
        .iter()
        .any(|result| matches!(result, Ok(mode) if *mode == "GENERAL"));

    let mut admin = connect_superuser(database);
    let retained_counts = admin
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM ONLY control.task_ingress_claims \
                  WHERE ingress_id=$1 AND client_request_id=$2), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_streams \
                  WHERE stream_id=$3::bytea OR stream_id=$4::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_commands \
                  WHERE stream_id=$3::bytea OR stream_id=$4::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_events \
                  WHERE stream_id=$3::bytea OR stream_id=$4::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_submission_envelopes \
                  WHERE ingress_id=$1 AND client_request_id=$2), \
                (SELECT request_kind::text FROM ONLY control.task_ingress_claims \
                  WHERE ingress_id=$1 AND client_request_id=$2)",
            &[
                &TASK_SUBMIT_INGRESS_ID,
                &concurrent_id,
                &hex_bytes(&concurrent_general_stream),
                &hex_bytes(&canary_stream),
            ],
        )
        .expect("cross-race retained counts");
    for index in 0..4 {
        assert_eq!(retained_counts.get::<_, i64>(index), 1);
    }
    assert_eq!(retained_counts.get::<_, i64>(4), i64::from(general_won));
    assert_eq!(
        retained_counts.get::<_, String>(5),
        if general_won {
            "GENERAL_TASK"
        } else {
            "CONTROLLED_CODEX_CANARY"
        }
    );
    drop(admin);

    let exact_id = "ingress-claim-general-exact-race";
    let exact_submission = general_submission(
        exact_id,
        "完成角色系統",
        general_submission_identity(project, "TASK-GENERAL-EXACT-RACE"),
        project,
    );
    let exact_stream = exact_submission.stream_id().clone();
    let expected_ref = exact_submission.task_ref().clone();
    let exact_barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let barrier = exact_barrier.clone();
        let database = database.to_owned();
        let run_id = run_id.to_owned();
        let submission = exact_submission.clone();
        handles.push(std::thread::spawn(move || {
            let target = MigrationTarget::new(database.clone(), run_id).expect("exact target");
            let mut ledger =
                PostgresTaskLedger::new(connect_as(&database, "lattice_runtime"), &target)
                    .expect("exact race ledger");
            let command = general_submission_command(&submission);
            barrier.wait();
            ledger
                .execute_submission(command, store_authority(), submission)
                .map(|execution| execution.submission().task_ref().clone())
                .map_err(PostgresTaskLedgerError::kind)
        }));
    }
    exact_barrier.wait();
    for handle in handles {
        assert_eq!(
            handle.join().expect("exact race thread"),
            Ok(expected_ref.clone())
        );
    }
    let mut admin = connect_superuser(database);
    let exact_counts = admin
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM ONLY control.task_ingress_claims \
                  WHERE ingress_id=$1 AND client_request_id=$2), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_streams \
                  WHERE stream_id=$3::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_commands \
                  WHERE stream_id=$3::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_events \
                  WHERE stream_id=$3::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_submission_envelopes \
                  WHERE ingress_id=$1 AND client_request_id=$2)",
            &[
                &TASK_SUBMIT_INGRESS_ID,
                &exact_id,
                &hex_bytes(&exact_stream),
            ],
        )
        .expect("exact-race retained counts");
    for index in 0..5 {
        assert_eq!(exact_counts.get::<_, i64>(index), 1);
    }
}

fn assert_check_violation(error: &postgres::Error, context: &str) {
    assert_eq!(
        error.as_db_error().map(postgres::error::DbError::code),
        Some(&SqlState::CHECK_VIOLATION),
        "{context}"
    );
}

fn assert_sqlstate(error: &postgres::Error, expected: &str, context: &str) {
    assert_eq!(
        error.as_db_error().map(|database| database.code().code()),
        Some(expected),
        "{context}"
    );
}

fn assert_snapshot_width_violation(error: &postgres::Error, context: &str) {
    let sqlstate = error.as_db_error().map(|database| database.code().code());
    assert!(
        matches!(sqlstate, Some("22001" | "23514")),
        "{context}: unexpected SQLSTATE {sqlstate:?}"
    );
}

#[allow(clippy::too_many_lines, clippy::unicode_not_nfc)]
fn prove_runtime_functions_reject_secret_client_ids(database: &str, secret_ids: &[&str]) {
    let ingress_id = TASK_SUBMIT_INGRESS_ID;
    let digest_bytes = vec![0x11_u8; 32];
    let stream_bytes = vec![0x22_u8; 32];
    let event_bytes = vec![0x33_u8; 32];
    let request_bytes = vec![0x44_u8; 32];
    let mut runtime = connect_as(database, "lattice_runtime");
    for secret_id in secret_ids {
        let command_id = format!("mcp-submit:{secret_id}");
        let mut prepare = runtime
            .build_transaction()
            .isolation_level(postgres::IsolationLevel::Serializable)
            .start()
            .expect("secret guard prepare transaction");
        let error = prepare
            .query(
                "SELECT * FROM control.task_ingress_prepare_v1(\
                    $1::text,$2::text,'CONTROLLED_CODEX_CANARY',$3::bytea,$4::bytea)",
                &[&ingress_id, secret_id, &digest_bytes, &stream_bytes],
            )
            .expect_err("ingress prepare must reject a secret-shaped client id");
        assert_sqlstate(&error, "LCR01", "ingress prepare secret guard");
        prepare
            .rollback()
            .expect("rollback rejected ingress prepare");

        let mut record = runtime
            .build_transaction()
            .isolation_level(postgres::IsolationLevel::Serializable)
            .start()
            .expect("secret guard record transaction");
        let error = record
            .query_one(
                "SELECT control.task_ingress_record_v1(\
                    'lattice.task-ledger.task-ingress-claim/1.0',$1::text,$2::text,\
                    'CONTROLLED_CODEX_CANARY',$3::bytea,$4::bytea,'1',$5::bytea,\
                    $6::text,$7::bytea)",
                &[
                    &ingress_id,
                    secret_id,
                    &digest_bytes,
                    &stream_bytes,
                    &event_bytes,
                    &command_id,
                    &request_bytes,
                ],
            )
            .expect_err("ingress record must reject a secret-shaped client id");
        assert_sqlstate(&error, "LCR01", "ingress record secret guard");
        record.rollback().expect("rollback rejected ingress record");

        let mut submission_prepare = runtime
            .build_transaction()
            .isolation_level(postgres::IsolationLevel::Serializable)
            .start()
            .expect("secret guard submission prepare transaction");
        let error = submission_prepare
            .query(
                "SELECT * FROM control.task_submission_prepare_v1(\
                    $1::text,$2::text,$3::bytea)",
                &[&ingress_id, secret_id, &digest_bytes],
            )
            .expect_err("submission prepare must reject a secret-shaped client id");
        assert_sqlstate(&error, "LCR01", "submission prepare secret guard");
        submission_prepare
            .rollback()
            .expect("rollback rejected submission prepare");

        let mut submission_record = runtime
            .build_transaction()
            .isolation_level(postgres::IsolationLevel::Serializable)
            .start()
            .expect("secret guard submission record transaction");
        let error = submission_record
            .query_one(
                "SELECT control.task_submission_record_v1(\
                    'lattice.task-ledger.task-submission/1.0',$1::text,$2::text,\
                    'objective','project',decode(repeat('11',32),'hex'),\
                    'project-1','snapshot-1','TASK-SECRET-GUARD','1',\
                    'GENERAL_TASK_INTAKE',decode(repeat('22',32),'hex'),\
                    decode(repeat('33',32),'hex'),repeat('1',64),\
                    'GENERAL_TASK_INTAKE_V1',decode(repeat('44',32),'hex'),'1',\
                    decode(repeat('55',32),'hex'),$3::text,decode(repeat('66',32),'hex'),\
                    decode(repeat('77',32),'hex'))",
                &[&ingress_id, secret_id, &command_id],
            )
            .expect_err("submission record must reject a secret-shaped client id");
        assert_sqlstate(&error, "LCR01", "submission record secret guard");
        submission_record
            .rollback()
            .expect("rollback rejected submission record");
    }

    for (client_request_id, objective, project_display_name, project_id, snapshot_id) in [
        (
            "unicode-objective-guard",
            "\u{2003}objective",
            "project",
            "project-1",
            "snapshot-1",
        ),
        (
            "unicode-project-guard",
            "objective",
            "project\u{00a0}",
            "project-1",
            "snapshot-1",
        ),
        (
            "nfc-objective-guard",
            "objective-e\u{0301}",
            "project",
            "project-1",
            "snapshot-1",
        ),
        (
            "nfc-project-guard",
            "objective",
            "project-e\u{0301}",
            "project-1",
            "snapshot-1",
        ),
        (
            "c1-objective-guard",
            "objective\u{009f}",
            "project",
            "project-1",
            "snapshot-1",
        ),
        (
            "ascii-boundary-secret-guard",
            "界sk-do-not-use",
            "project",
            "project-1",
            "snapshot-1",
        ),
        (
            "kelvin-sensitive-key-guard",
            "Kpassword=do-not-use",
            "project",
            "project-1",
            "snapshot-1",
        ),
        (
            "kelvin-secret-prefix-guard",
            "Ksk-do-not-use",
            "project",
            "project-1",
            "snapshot-1",
        ),
        (
            "reverse-private-key-guard",
            "private key-----noise-----BEGIN ",
            "project",
            "project-1",
            "snapshot-1",
        ),
        (
            "secret-project-id-guard",
            "objective",
            "project",
            "github_pat_do_not_use",
            "snapshot-1",
        ),
        (
            "secret-snapshot-id-guard",
            "objective",
            "project",
            "project-1",
            "ASIA1234567890ABCDEF",
        ),
    ] {
        let command_id = format!("mcp-submit:{client_request_id}");
        let mut record = runtime
            .build_transaction()
            .isolation_level(postgres::IsolationLevel::Serializable)
            .start()
            .expect("Unicode trim record transaction");
        let error = record
            .query_one(
                "SELECT control.task_submission_record_v1(\
                    'lattice.task-ledger.task-submission/1.0',$1::text,$2::text,\
                    $3::text,$4::text,decode(repeat('11',32),'hex'),\
                    $5::text,$6::text,'TASK-UNICODE-GUARD','1',\
                    'GENERAL_TASK_INTAKE',decode(repeat('22',32),'hex'),\
                    decode(repeat('33',32),'hex'),repeat('1',64),\
                    'GENERAL_TASK_INTAKE_V1',decode(repeat('44',32),'hex'),'1',\
                    decode(repeat('55',32),'hex'),$7::text,decode(repeat('66',32),'hex'),\
                    decode(repeat('77',32),'hex'))",
                &[
                    &ingress_id,
                    &client_request_id,
                    &objective,
                    &project_display_name,
                    &project_id,
                    &snapshot_id,
                    &command_id,
                ],
            )
            .expect_err("submission record must reject non-canonical human text");
        assert_sqlstate(&error, "LCR01", "submission record human-text guard");
        record
            .rollback()
            .expect("rollback rejected human-text record");
    }
}

#[allow(clippy::too_many_lines, clippy::unicode_not_nfc)]
fn prove_durable_submission_secret_constraints(
    database: &str,
    ledger: &mut PostgresTaskLedger,
    task_ref: &ContentDigest,
    expected_objective: &str,
    expected_client_request_id: &str,
) {
    let mut admin = connect_superuser(database);
    for objective in [
        "secret=value",
        "credential:value",
        "Cookie=value",
        "refresh_token=value",
        "AKIA1234567890ABCDEF",
        r#"{"password":"hunter2"}"#,
        r#"{"api_key" : "hunter2"}"#,
        "password\u{2003}=hunter2",
        "api_key\u{00a0}:hunter2",
        "界sk-do-not-use",
        "Kpassword=do-not-use",
        "Ksk-do-not-use",
        "private key-----noise-----BEGIN ",
        "objective\u{009f}",
    ] {
        let error = admin
            .execute(
                "UPDATE ONLY control.task_submission_envelopes \
                 SET objective=$1 WHERE task_ref=$2",
                &[&objective, &task_ref.as_str()],
            )
            .expect_err("durable objective secret assignment must be rejected");
        assert_check_violation(&error, "durable objective secret assignment SQLSTATE");
    }
    let error = admin
        .execute(
            "UPDATE ONLY control.task_submission_envelopes \
             SET project_display_name=$1 WHERE task_ref=$2",
            &[&"api_key\u{2003}=hunter2", &task_ref.as_str()],
        )
        .expect_err("durable project-name secret assignment must be rejected");
    assert_check_violation(&error, "durable project-name secret assignment SQLSTATE");
    let error = admin
        .execute(
            "UPDATE ONLY control.task_submission_envelopes \
             SET project_id=$1 WHERE task_ref=$2",
            &[&"github_pat_do_not_use", &task_ref.as_str()],
        )
        .expect_err("durable project id secret must be rejected");
    assert_check_violation(&error, "durable project-id secret SQLSTATE");
    let error = admin
        .execute(
            "UPDATE ONLY control.task_submission_envelopes \
             SET project_snapshot_id=$1 WHERE task_ref=$2",
            &[&"ASIA1234567890ABCDEF", &task_ref.as_str()],
        )
        .expect_err("durable project snapshot secret must be rejected");
    assert_check_violation(&error, "durable project-snapshot secret SQLSTATE");
    for objective in ["\u{2003}objective", "objective\u{00a0}"] {
        let error = admin
            .execute(
                "UPDATE ONLY control.task_submission_envelopes \
                 SET objective=$1 WHERE task_ref=$2",
                &[&objective, &task_ref.as_str()],
            )
            .expect_err("durable objective Unicode outer whitespace must be rejected");
        assert_check_violation(&error, "durable objective Unicode trim SQLSTATE");
    }
    for project_display_name in ["\u{202f}project", "project\u{3000}"] {
        let error = admin
            .execute(
                "UPDATE ONLY control.task_submission_envelopes \
                 SET project_display_name=$1 WHERE task_ref=$2",
                &[&project_display_name, &task_ref.as_str()],
            )
            .expect_err("durable project-name Unicode outer whitespace must be rejected");
        assert_check_violation(&error, "durable project-name Unicode trim SQLSTATE");
    }
    let error = admin
        .execute(
            "UPDATE ONLY control.task_submission_envelopes \
             SET objective=$1 WHERE task_ref=$2",
            &[&"objective-e\u{0301}", &task_ref.as_str()],
        )
        .expect_err("durable non-NFC objective must be rejected");
    assert_check_violation(&error, "durable objective NFC SQLSTATE");
    let error = admin
        .execute(
            "UPDATE ONLY control.task_submission_envelopes \
             SET project_display_name=$1 WHERE task_ref=$2",
            &[&"project-e\u{0301}", &task_ref.as_str()],
        )
        .expect_err("durable non-NFC project name must be rejected");
    assert_check_violation(&error, "durable project-name NFC SQLSTATE");
    let retained: String = admin
        .query_one(
            "SELECT objective::text FROM ONLY control.task_submission_envelopes \
             WHERE task_ref=$1",
            &[&task_ref.as_str()],
        )
        .expect("retained objective after rejected secret writes")
        .get(0);
    assert_eq!(retained, expected_objective);

    let secret_client_ids = [
        "secret:value",
        "ghp_abcdefghijklmnopqrstuvwxyz123456",
        "prefix-sk-do-not-use",
        "AKIA1234567890ABCDEF",
    ];
    for client_request_id in secret_client_ids {
        assert_eq!(
            ledger
                .load_ingress_claim_by_request(TASK_SUBMIT_INGRESS_ID, client_request_id)
                .expect_err("neutral Store lookup must reject a secret-shaped client id")
                .kind(),
            PostgresTaskLedgerErrorKind::Malformed
        );
        assert_eq!(
            ledger
                .load_submission_by_request(TASK_SUBMIT_INGRESS_ID, client_request_id)
                .expect_err("submission Store lookup must reject a secret-shaped client id")
                .kind(),
            PostgresTaskLedgerErrorKind::Malformed
        );
        let error = admin
            .execute(
                "UPDATE ONLY control.task_submission_envelopes \
                 SET client_request_id=$1 WHERE task_ref=$2",
                &[&client_request_id, &task_ref.as_str()],
            )
            .expect_err("submission locator must reject a secret-shaped client id");
        assert_check_violation(&error, "submission client-id secret CHECK");
        let error = admin
            .execute(
                "UPDATE ONLY control.task_ingress_claims \
                 SET client_request_id=$1 \
                 WHERE ingress_id=$2 AND client_request_id=$3",
                &[
                    &client_request_id,
                    &TASK_SUBMIT_INGRESS_ID,
                    &expected_client_request_id,
                ],
            )
            .expect_err("ingress claim must reject a secret-shaped client id");
        assert_check_violation(&error, "ingress client-id secret CHECK");
    }
    let retained_client_request_id: String = admin
        .query_one(
            "SELECT client_request_id::text \
             FROM ONLY control.task_submission_envelopes WHERE task_ref=$1",
            &[&task_ref.as_str()],
        )
        .expect("retained client id after rejected secret writes")
        .get(0);
    assert_eq!(retained_client_request_id, expected_client_request_id);
    drop(admin);
    prove_runtime_functions_reject_secret_client_ids(database, &secret_client_ids);
}

fn prove_submission_lookup_linkage_corruption_fails_closed(
    database: &str,
    ledger: &mut PostgresTaskLedger,
    submission: &TaskSubmissionEnvelope,
) {
    let mut admin = connect_superuser(database);
    let stream_id = hex_bytes(submission.stream_id());
    assert_eq!(
        admin
            .execute(
                "UPDATE ONLY control.task_ledger_events \
                 SET action_id='GENERAL_TASK_INTAKE_V2' \
                 WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("corrupt submission TaskCreated action linkage"),
        1
    );
    assert_eq!(
        ledger
            .load_submission_by_task_ref(submission.task_ref())
            .expect_err("task-ref lookup must expose retained linkage corruption")
            .kind(),
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt
    );
    assert_eq!(
        ledger
            .load_submission_by_request(submission.ingress_id(), submission.client_request_id())
            .expect_err("request lookup must expose retained linkage corruption")
            .kind(),
        PostgresTaskLedgerErrorKind::RetainedRowCorrupt
    );
    assert_eq!(
        admin
            .execute(
                "UPDATE ONLY control.task_ledger_events \
                 SET action_id='GENERAL_TASK_INTAKE_V1' \
                 WHERE stream_id=$1::bytea AND sequence=1",
                &[&stream_id],
            )
            .expect("restore submission TaskCreated action linkage"),
        1
    );
    assert!(
        ledger
            .load_submission_by_task_ref(submission.task_ref())
            .expect("restored task-ref lookup")
            .is_some()
    );
}

fn assert_submission_was_rolled_back(admin: &mut Client, submission: &TaskSubmissionEnvelope) {
    let stream_id = hex_bytes(submission.stream_id());
    let counts = admin
        .query_one(
            "SELECT \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_streams \
                  WHERE stream_id=$1::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_events \
                  WHERE stream_id=$1::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_ledger_commands \
                  WHERE stream_id=$1::bytea), \
                (SELECT count(*)::bigint FROM ONLY control.task_ingress_claims \
                  WHERE ingress_id=$2 AND client_request_id=$3), \
                (SELECT count(*)::bigint FROM ONLY control.task_submission_envelopes \
                  WHERE ingress_id=$2 AND client_request_id=$3)",
            &[
                &stream_id,
                &submission.ingress_id(),
                &submission.client_request_id(),
            ],
        )
        .expect("rolled-back submission counts");
    for index in 0..5 {
        assert_eq!(counts.get::<_, i64>(index), 0);
    }
}

fn prove_submission_registry_guards(
    database: &str,
    ledger: &mut PostgresTaskLedger,
    project: &GeneralProjectFixture,
) {
    let currentness = general_submission(
        "general-submission-registry-currentness",
        "registry currentness guard",
        general_submission_identity(project, "TASK-GENERAL-REGISTRY-CURRENTNESS"),
        project,
    );
    let inactive = general_submission(
        "general-submission-registry-inactive",
        "registry lifecycle guard",
        general_submission_identity(project, "TASK-GENERAL-REGISTRY-INACTIVE"),
        project,
    );
    let mut admin = connect_superuser(database);
    assert_eq!(
        admin
            .execute(
                "UPDATE ONLY control.project_registry_projects \
                 SET drift_repository=true WHERE project_id=$1",
                &[&project.project_id.as_str()],
            )
            .expect("install Project Registry currentness drift"),
        1
    );
    let error = ledger
        .execute_submission(
            general_submission_command(&currentness),
            store_authority(),
            currentness.clone(),
        )
        .expect_err("drifted Project Registry row must reject general submission");
    assert_eq!(
        error.kind(),
        PostgresTaskLedgerErrorKind::ProjectRegistryCurrentnessConflict
    );
    assert_submission_was_rolled_back(&mut admin, &currentness);
    assert_eq!(
        admin
            .execute(
                "UPDATE ONLY control.project_registry_projects \
                 SET drift_repository=false WHERE project_id=$1",
                &[&project.project_id.as_str()],
            )
            .expect("restore Project Registry currentness"),
        1
    );

    assert_eq!(
        admin
            .execute(
                "UPDATE ONLY control.project_registry_projects \
                 SET authority_lifecycle='SUSPENDED' WHERE project_id=$1",
                &[&project.project_id.as_str()],
            )
            .expect("install inactive Project Registry lifecycle"),
        1
    );
    let error = ledger
        .execute_submission(
            general_submission_command(&inactive),
            store_authority(),
            inactive.clone(),
        )
        .expect_err("inactive Project Registry row must reject general submission");
    assert_eq!(
        error.kind(),
        PostgresTaskLedgerErrorKind::ProjectRegistryInactive
    );
    assert_submission_was_rolled_back(&mut admin, &inactive);
    assert_eq!(
        admin
            .execute(
                "UPDATE ONLY control.project_registry_projects \
                 SET authority_lifecycle='ACTIVE' WHERE project_id=$1",
                &[&project.project_id.as_str()],
            )
            .expect("restore Project Registry lifecycle"),
        1
    );
}

fn prove_submission_claim_digest_drift_fails_fresh_verifiers(
    database: &str,
    target: &MigrationTarget,
    submission: &TaskSubmissionEnvelope,
) {
    let claim = TaskIngressClaim::general_submission(submission).expect("canonical general claim");
    let original_digest = hex_bytes(claim.request_digest());
    let drift_digest = vec![0xab_u8; 32];
    assert_ne!(original_digest, drift_digest);
    let mut migrator = connect_as(database, "lattice_migrator");
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ingress_claims \
                    SET ingress_request_digest=$1::bytea \
                  WHERE ingress_id=$2 AND client_request_id=$3",
                &[
                    &drift_digest,
                    &submission.ingress_id(),
                    &submission.client_request_id(),
                ],
            )
            .expect("general claim digest drift apply"),
        1
    );
    for role in DatabaseRole::ALL {
        let mut verifier = connect_as(database, role.as_str());
        let failure = verify_postgres_schema(&mut verifier, target, role)
            .expect_err("fresh verifier must reject general claim digest drift");
        assert_eq!(
            failure.kind(),
            lattice_postgres_store::PostgresStoreSetupErrorKind::HistoryMismatch,
            "general claim digest drift role {}",
            role.as_str()
        );
    }
    assert_eq!(
        migrator
            .execute(
                "UPDATE ONLY control.task_ingress_claims \
                    SET ingress_request_digest=$1::bytea \
                  WHERE ingress_id=$2 AND client_request_id=$3",
                &[
                    &original_digest,
                    &submission.ingress_id(),
                    &submission.client_request_id(),
                ],
            )
            .expect("general claim digest drift repair"),
        1
    );
    let repaired = PostgresTaskLedger::new(connect_as(database, "lattice_runtime"), target)
        .expect("fresh runtime constructor after general claim digest repair");
    drop(repaired);
    println!("TASK_SUBMISSION_GENERAL_CLAIM_DIGEST_DRIFT_REJECTED_BY_FRESH_ROLES");
}

#[allow(clippy::too_many_lines)]
fn prove_project_registry_snapshot_width_boundary(
    database: &str,
    ledger: &mut PostgresTaskLedger,
    project: &GeneralProjectFixture,
) {
    let original_snapshot = project.project_snapshot_id.as_str();
    assert!(original_snapshot.len() < 159);
    let snapshot_159 = format!(
        "{original_snapshot}{}",
        "f".repeat(159 - original_snapshot.len())
    );
    assert_eq!(snapshot_159.len(), 159);
    let snapshot_160 = format!("{snapshot_159}x");
    assert_eq!(snapshot_160.len(), 160);

    let mut admin = connect_superuser(database);
    assert_eq!(
        admin
            .execute(
                "UPDATE ONLY control.project_registry_projects \
                 SET authority_snapshot_id=$1 WHERE project_id=$2",
                &[&snapshot_159, &project.project_id.as_str()],
            )
            .expect("install exact maximum Registry snapshot"),
        1
    );
    let maximum_project = GeneralProjectFixture {
        project_id: project.project_id.clone(),
        project_snapshot_id: ProjectSnapshotId::new(snapshot_159.clone())
            .expect("159-byte project snapshot"),
        authority_receipt_digest: project.authority_receipt_digest.clone(),
    };
    let accepted = general_submission(
        "general-submission-snapshot-159",
        "maximum Project Registry snapshot",
        general_submission_identity(&maximum_project, "TASK-GENERAL-SNAPSHOT-159"),
        &maximum_project,
    );
    let accepted_result = ledger
        .execute_submission(
            general_submission_command(&accepted),
            store_authority(),
            accepted.clone(),
        )
        .expect("159-byte snapshot must pass the formal general-intake path");
    assert!(!accepted_result.ledger_execution().is_exact_retry());

    let stream_id = hex_bytes(accepted.stream_id());
    let widths = admin
        .query_one(
            "SELECT \
                (SELECT char_length(project_snapshot_id)::bigint \
                   FROM ONLY control.task_ledger_streams WHERE stream_id=$1::bytea), \
                (SELECT char_length(project_snapshot_id)::bigint \
                   FROM ONLY control.physical_heads \
                  WHERE repository_owner='TASK_LEDGER' AND aggregate_key_digest=$1::bytea), \
                (SELECT char_length(project_snapshot_id)::bigint \
                   FROM ONLY control.terminal_transactions \
                  WHERE repository_owner='TASK_LEDGER' AND aggregate_key_digest=$1::bytea), \
                (SELECT char_length(project_snapshot_id)::bigint \
                   FROM ONLY control.task_submission_envelopes WHERE task_ref=$2)",
            &[&stream_id, &accepted.task_ref().as_str()],
        )
        .expect("159-byte retained snapshot widths");
    for index in 0..4 {
        assert_eq!(widths.get::<_, i64>(index), 159);
    }

    let global_manifest = admin
        .query_one(
            "SELECT manifest_sha256::text FROM ONLY control.schema_compatibility \
             WHERE singleton=true AND current_schema_version=7",
            &[],
        )
        .expect("current schema-v7 manifest")
        .get::<_, String>(0);
    let mut runtime = connect_as(database, "lattice_runtime");
    let error = runtime
        .query(
            "SELECT * FROM control.task_ledger_read_head_v4(\
                $1::smallint,$2::text,$3::bytea,$4::text,$5::text)",
            &[
                &7_i16,
                &global_manifest,
                &stream_id,
                &project.project_id.as_str(),
                &snapshot_160,
            ],
        )
        .expect_err("current-v7 read function must reject a 160-byte snapshot");
    assert_sqlstate(&error, "LST01", "current-v7 snapshot width guard");
    drop(runtime);

    let error = admin
        .execute(
            "UPDATE ONLY control.task_ledger_streams \
             SET project_snapshot_id=$1 WHERE stream_id=$2::bytea",
            &[&snapshot_160, &stream_id],
        )
        .expect_err("task ledger stream must reject a 160-byte snapshot");
    assert_snapshot_width_violation(&error, "task ledger stream snapshot width");
    let error = admin
        .execute(
            "UPDATE ONLY control.physical_heads SET project_snapshot_id=$1 \
             WHERE repository_owner='TASK_LEDGER' AND aggregate_key_digest=$2::bytea",
            &[&snapshot_160, &stream_id],
        )
        .expect_err("physical head must reject a 160-byte snapshot");
    assert_snapshot_width_violation(&error, "physical head snapshot width");
    let error = admin
        .execute(
            "UPDATE ONLY control.terminal_transactions SET project_snapshot_id=$1 \
             WHERE repository_owner='TASK_LEDGER' AND aggregate_key_digest=$2::bytea",
            &[&snapshot_160, &stream_id],
        )
        .expect_err("terminal transaction must reject a 160-byte snapshot");
    assert_snapshot_width_violation(&error, "terminal transaction snapshot width");
    let error = admin
        .execute(
            "UPDATE ONLY control.task_submission_envelopes SET project_snapshot_id=$1 \
             WHERE task_ref=$2",
            &[&snapshot_160, &accepted.task_ref().as_str()],
        )
        .expect_err("submission envelope must reject a 160-byte snapshot");
    assert_snapshot_width_violation(&error, "submission envelope snapshot width");

    assert_eq!(
        admin
            .execute(
                "UPDATE ONLY control.project_registry_projects \
                 SET authority_snapshot_id=$1 WHERE project_id=$2",
                &[
                    &project.project_snapshot_id.as_str(),
                    &project.project_id.as_str(),
                ],
            )
            .expect("restore Registry snapshot after width proof"),
        1
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn general_submission_is_atomic_idempotent_and_fresh_reconnectable_when_provisioned() {
    if std::env::var("LATTICE_TASK_SUBMISSION_LIVE")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!("SKIP: LATTICE_TASK_SUBMISSION_LIVE is not configured");
        return;
    }
    let database = required_environment("LATTICE_TASK_SUBMISSION_DATABASE");
    let run_id = required_environment("LATTICE_TASK_SUBMISSION_RUN_ID");
    let target = MigrationTarget::new(database.clone(), run_id.clone()).expect("submission target");
    assert_eq!(
        std::env::var("LATTICE_TASK_SUBMISSION_PROVISION_FRESH").as_deref(),
        Ok("0"),
        "general-submission acceptance requires the formal product bootstrap, not Store-owned fresh provisioning"
    );
    let composed = PostgresTaskLedger::new(connect_as(&database, "lattice_runtime"), &target)
        .expect("product-bootstrap Store-v8 runtime profile");
    drop(composed);

    let mut tamper_admin = connect_superuser(&database);
    tamper_admin
        .batch_execute(
            "CREATE TABLE control.task_submission_composition_tamper (id bigint NOT NULL)",
        )
        .expect("install Store-owned catalog tamper");
    assert!(
        PostgresTaskLedger::new(connect_as(&database, "lattice_runtime"), &target).is_err(),
        "a product-installed foreman extension must not make Store-owned catalog drift acceptable"
    );
    tamper_admin
        .batch_execute("DROP TABLE control.task_submission_composition_tamper")
        .expect("remove Store-owned catalog tamper");
    drop(tamper_admin);

    let project = provision_general_project(&database, &target);

    let atomic_identity = general_submission_identity(&project, "TASK-GENERAL-ATOMIC");
    let atomic = general_submission(
        "general-submission-atomic",
        "atomic locator rollback",
        atomic_identity.clone(),
        &project,
    );
    let atomic_command = general_submission_command(&atomic);
    let mut ledger = PostgresTaskLedger::new(connect_as(&database, "lattice_runtime"), &target)
        .expect("submission ledger");
    let mut fault_admin = connect_superuser(&database);
    fault_admin
        .batch_execute(
            "CREATE FUNCTION control.task_submission_acceptance_fault() RETURNS trigger \
             LANGUAGE plpgsql AS 'BEGIN RAISE EXCEPTION USING ERRCODE = ''P0001''; END'; \
             CREATE TRIGGER task_submission_acceptance_fault \
             BEFORE INSERT ON control.task_submission_envelopes \
             FOR EACH ROW EXECUTE FUNCTION control.task_submission_acceptance_fault()",
        )
        .expect("install submission locator fault");
    let fault = ledger
        .execute_submission(atomic_command, store_authority(), atomic.clone())
        .expect_err("locator fault must roll back Ledger and locator together");
    assert_eq!(fault.kind(), PostgresTaskLedgerErrorKind::TransactionFailed);
    fault_admin
        .batch_execute(
            "DROP TRIGGER task_submission_acceptance_fault \
                 ON control.task_submission_envelopes; \
             DROP FUNCTION control.task_submission_acceptance_fault()",
        )
        .expect("remove submission locator fault");
    assert!(
        ledger
            .load_stream(atomic_identity)
            .expect("atomic rollback stream")
            .stream()
            .events()
            .is_empty()
    );
    assert!(
        ledger
            .load_submission_by_request(atomic.ingress_id(), atomic.client_request_id())
            .expect("atomic rollback locator")
            .is_none()
    );
    let rolled_back_claims: i64 = fault_admin
        .query_one(
            "SELECT count(*)::bigint FROM ONLY control.task_ingress_claims \
             WHERE ingress_id=$1 AND client_request_id=$2",
            &[&atomic.ingress_id(), &atomic.client_request_id()],
        )
        .expect("atomic rollback claim")
        .get(0);
    assert_eq!(rolled_back_claims, 0);
    drop(fault_admin);

    let identity = general_submission_identity(&project, "TASK-GENERAL-ACCEPTANCE");
    let submission = general_submission(
        "general-submission-acceptance",
        "完成角色系統",
        identity.clone(),
        &project,
    );
    let task_ref = submission.task_ref().clone();
    let created = ledger
        .execute_submission(
            general_submission_command(&submission),
            store_authority(),
            submission.clone(),
        )
        .expect("atomic general submission");
    assert!(!created.ledger_execution().is_exact_retry());
    assert_eq!(created.submission().task_ref(), &task_ref);

    let replay = ledger
        .execute_submission(
            general_submission_command(&submission),
            store_authority(),
            submission.clone(),
        )
        .expect("same client envelope exact replay");
    assert!(replay.ledger_execution().is_exact_retry());
    assert_eq!(replay.submission().task_ref(), &task_ref);
    prove_submission_claim_digest_drift_fails_fresh_verifiers(&database, &target, &submission);

    let changed = general_submission(
        submission.client_request_id(),
        "完成不同角色系統",
        identity,
        &project,
    );
    let conflict = ledger
        .execute_submission(
            general_submission_command(&changed),
            store_authority(),
            changed,
        )
        .expect_err("changed reuse must fail closed");
    assert_eq!(
        conflict.kind(),
        PostgresTaskLedgerErrorKind::CommandSubstitution
    );
    prove_durable_submission_secret_constraints(
        &database,
        &mut ledger,
        &task_ref,
        submission.objective(),
        submission.client_request_id(),
    );
    prove_submission_registry_guards(&database, &mut ledger, &project);
    prove_project_registry_snapshot_width_boundary(&database, &mut ledger, &project);
    drop(ledger);

    let mut reconnected =
        PostgresTaskLedger::new(connect_as(&database, "lattice_runtime"), &target)
            .expect("fresh submission ledger connection");
    let by_ref = reconnected
        .load_submission_by_task_ref(&task_ref)
        .expect("fresh task-ref lookup")
        .expect("retained task-ref");
    assert_eq!(by_ref.submission(), &submission);
    assert_eq!(by_ref.ledger().stream().events().len(), 1);
    assert_eq!(
        by_ref.ledger().autonomy_state(),
        &VerifiedAutonomyReceiptState::NotApplicable
    );
    let by_request = reconnected
        .load_submission_by_request(submission.ingress_id(), submission.client_request_id())
        .expect("fresh request lookup")
        .expect("retained request");
    assert_eq!(by_request.submission().task_ref(), &task_ref);
    prove_submission_lookup_linkage_corruption_fails_closed(
        &database,
        &mut reconnected,
        &submission,
    );
    drop(reconnected);

    prove_task_ingress_claim_races(&database, &run_id, &target, &project);
}

#[test]
#[ignore = "requires an exact product-installed Store-v7 plus managed Foreman extension"]
fn exact_managed_foreman_extension_remains_store_v7_compatible_when_provisioned() {
    if std::env::var("LATTICE_STORE_FOREMAN_COMPOSITION_LIVE")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }
    let run_id = required_environment("LATTICE_TASK019_RUN_ID");
    let database = format!("lattice_task019_{}_base", &run_id[..8]);
    let target = MigrationTarget::new(database.clone(), run_id).expect("composition target");

    PostgresTaskLedger::new(connect_as(&database, "lattice_runtime"), &target)
        .expect("exact managed Foreman profile must remain Store-v7 compatible");
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
