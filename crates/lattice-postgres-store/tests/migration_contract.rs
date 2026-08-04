use lattice_postgres_store::{
    DatabaseRole, MigrationStatus, MigrationTarget, MigrationTransactionMode,
    POSTGRES_DRIVER_VERSION, POSTGRES_SCHEMA_VERSION, PostgresStoreSetupError,
    PostgresStoreSetupErrorKind, SUPPORTED_POSTGRES_MAJOR, migration_manifest,
    verify_embedded_manifest,
};

const BOOTSTRAP_SHA256: &str = "7bff021fc17f738551309c906578c8015b2dd0307d27d239c21df1697c4d09c8";
const FOUNDATION_SHA256: &str = "e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0";
const LIVE_CONTROL_STORE_SHA256: &str =
    "00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1";

#[test]
fn manifest_is_closed_ordered_and_preserves_the_superseded_bootstrap() {
    let manifest = migration_manifest();
    assert_eq!(manifest.len(), 4);

    let draft = &manifest[0];
    assert_eq!(draft.ordinal(), 1);
    assert_eq!(draft.id(), "0001_bootstrap_draft");
    assert_eq!(draft.path(), "db/migrations/0001_bootstrap.sql");
    assert_eq!(draft.byte_length(), 312);
    assert_eq!(draft.sha256(), BOOTSTRAP_SHA256);
    assert_eq!(draft.status(), MigrationStatus::Superseded);
    assert_eq!(
        draft.transaction_mode(),
        MigrationTransactionMode::NotExecuted
    );
    assert_eq!(draft.schema_version(), 0);
    assert_eq!(draft.reader_compatibility(), 0..=0);
    assert_eq!(draft.writer_compatibility(), 0..=0);

    let foundation = &manifest[1];
    assert_eq!(foundation.ordinal(), 2);
    assert_eq!(foundation.id(), "0002_control_store_foundation");
    assert_eq!(
        foundation.path(),
        "db/migrations/0002_control_store_foundation.sql"
    );
    assert!(foundation.byte_length() > 0);
    assert_eq!(foundation.byte_length(), 14_259);
    assert_eq!(foundation.sha256(), FOUNDATION_SHA256);
    assert_eq!(foundation.status(), MigrationStatus::Executable);
    assert_eq!(
        foundation.transaction_mode(),
        MigrationTransactionMode::RunnerOwned
    );
    assert_eq!(foundation.schema_version(), 1);
    assert_eq!(foundation.reader_compatibility(), 1..=1);
    assert_eq!(foundation.writer_compatibility(), 1..=1);

    let live_store = &manifest[2];
    assert_eq!(live_store.ordinal(), 3);
    assert_eq!(live_store.id(), "0003_live_control_store");
    assert_eq!(
        live_store.path(),
        "db/migrations/0003_live_control_store.sql"
    );
    assert_eq!(live_store.byte_length(), 29_518);
    assert_eq!(live_store.sha256(), LIVE_CONTROL_STORE_SHA256);
    assert_eq!(live_store.status(), MigrationStatus::Executable);
    assert_eq!(
        live_store.transaction_mode(),
        MigrationTransactionMode::RunnerOwned
    );
    assert_eq!(live_store.schema_version(), 2);
    assert_eq!(live_store.reader_compatibility(), 2..=2);
    assert_eq!(live_store.writer_compatibility(), 2..=2);

    let task_ledger = &manifest[3];
    assert_eq!(task_ledger.ordinal(), 4);
    assert_eq!(task_ledger.id(), "0004_task_ledger_repository");
    assert_eq!(
        task_ledger.path(),
        "db/migrations/0004_task_ledger_repository.sql"
    );
    assert!(task_ledger.byte_length() > 0);
    assert_eq!(task_ledger.sha256().len(), 64);
    assert_eq!(task_ledger.status(), MigrationStatus::Executable);
    assert_eq!(
        task_ledger.transaction_mode(),
        MigrationTransactionMode::RunnerOwned
    );
    assert_eq!(task_ledger.schema_version(), POSTGRES_SCHEMA_VERSION);
    assert_eq!(task_ledger.reader_compatibility(), 3..=3);
    assert_eq!(task_ledger.writer_compatibility(), 3..=3);

    let evidence = verify_embedded_manifest().expect("embedded manifest");
    assert_eq!(evidence.entry_count(), 4);
    assert_eq!(evidence.executable_count(), 3);
    assert_eq!(evidence.schema_version(), POSTGRES_SCHEMA_VERSION);
    assert_eq!(evidence.manifest_sha256().as_str().len(), 64);
}

#[test]
#[allow(clippy::too_many_lines)]
fn task_ledger_repository_migration_is_fixed_bounded_and_function_gated() {
    let repository = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0004_task_ledger_repository")
        .expect("Task Ledger repository migration");
    let sql = std::str::from_utf8(repository.bytes()).expect("UTF-8 SQL");
    let uppercase = sql.to_ascii_uppercase();
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();

    for forbidden in [
        "BEGIN;",
        "COMMIT;",
        "ROLLBACK;",
        "IF NOT EXISTS",
        "DO $$",
        "EXECUTE FORMAT",
        "EXECUTE IMMEDIATE",
        "CREATE EXTENSION",
        "CREATE ROLE",
        "ALTER ROLE",
        "PASSWORD",
        "CREATE DATABASE",
        "DROP TABLE",
        "DROP SCHEMA",
        "DROP FUNCTION",
    ] {
        assert!(
            !uppercase.contains(forbidden),
            "forbidden Task Ledger migration surface: {forbidden}"
        );
    }

    for table in [
        "TASK_LEDGER_STREAMS",
        "TASK_LEDGER_EVENTS",
        "TASK_LEDGER_COMMANDS",
        "TASK_LEDGER_OUTBOX",
    ] {
        assert!(normalized.contains(&format!("CREATE TABLE CONTROL.{table} (")));
        assert!(!normalized.contains(&format!("GRANT SELECT ON CONTROL.{table}")));
        assert!(!normalized.contains(&format!("GRANT INSERT ON CONTROL.{table}")));
        assert!(!normalized.contains(&format!("GRANT UPDATE ON CONTROL.{table}")));
        assert!(!normalized.contains(&format!("GRANT DELETE ON CONTROL.{table}")));
    }
    assert_eq!(
        uppercase
            .matches("CREATE TABLE CONTROL.TASK_LEDGER_")
            .count(),
        4
    );

    for function in [
        "STORE_PREPARE_V3",
        "STORE_FINALIZE_V3",
        "STORE_CURRENT_HEAD_V3",
        "TASK_LEDGER_PREPARE_V1",
        "TASK_LEDGER_READ_HEAD_V1",
        "TASK_LEDGER_READ_EVENTS_V1",
        "TASK_LEDGER_READ_COMMANDS_V1",
        "TASK_LEDGER_FINALIZE_V1",
    ] {
        assert!(normalized.contains(&format!("CREATE FUNCTION CONTROL.{function}(")));
        assert!(normalized.contains(&format!("REVOKE ALL ON FUNCTION CONTROL.{function}(")));
        assert!(normalized.contains(&format!("GRANT EXECUTE ON FUNCTION CONTROL.{function}(")));
    }
    assert_eq!(uppercase.matches("CREATE FUNCTION CONTROL.").count(), 8);
    assert_eq!(uppercase.matches("SECURITY DEFINER").count(), 8);
    assert_eq!(uppercase.matches("SET SEARCH_PATH = PG_CATALOG").count(), 8);
    assert_eq!(uppercase.matches("SET ROW_SECURITY = ON").count(), 8);
    assert_eq!(uppercase.matches("SET LOCK_TIMEOUT = '5S'").count(), 8);
    assert_eq!(
        uppercase.matches("SET STATEMENT_TIMEOUT = '30S'").count(),
        8
    );
    assert_eq!(
        sql.lines()
            .filter(|line| line.trim() == "global_schema_version smallint,")
            .count(),
        3
    );
    assert_eq!(
        sql.lines()
            .filter(|line| line.trim() == "global_manifest_sha256 text")
            .count(),
        3
    );

    for historical in [
        "STORE_PREPARE_V2",
        "STORE_FINALIZE_V2",
        "STORE_CURRENT_HEAD_V2",
    ] {
        assert!(normalized.contains(&format!("REVOKE EXECUTE ON FUNCTION CONTROL.{historical}(")));
    }

    for required in [
        "NUMERIC(20,0)",
        "18446744073709551615",
        "JSONB_PATH_EXISTS",
        "TYPE() == \"NUMBER\"",
        "LATTICE_DEVOS_CONTROL_SCHEMA_V3",
        "4582EDCE68A947998A8F4C6895BB37CEEC9E842F516471F4D9E2617A6757F129",
        "FROM ONLY CONTROL.TASK_LEDGER_STREAMS",
        "FROM ONLY CONTROL.TASK_LEDGER_EVENTS",
        "FROM ONLY CONTROL.TASK_LEDGER_COMMANDS",
        "FROM ONLY CONTROL.TASK_LEDGER_OUTBOX",
        "UNIQUE (INTENT_DIGEST)",
        "P_PROJECT_SNAPSHOT_ID !~ '^[A-Z0-9._:-]{1,128}$'",
        "V_PHYSICAL_COUNT > 1",
        "P_APPEND_EVENT AND P_EVENT_KIND = 'EFFECT_INTENT' AND P_AUDIT_OUTCOME = 'RECORDED'",
        "V_TERMINAL.EXPECTED_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC - 1",
        "V_TERMINAL.BEFORE_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC - 1",
        "V_TERMINAL.AFTER_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC",
        "TERMINAL_RECEIPT_DIGEST BYTEA, GLOBAL_SCHEMA_VERSION SMALLINT, GLOBAL_MANIFEST_SHA256 TEXT",
        "HEAD_DIGEST BYTEA, GLOBAL_SCHEMA_VERSION SMALLINT, GLOBAL_MANIFEST_SHA256 TEXT",
        "PHYSICAL_HEAD_DIGEST BYTEA, GLOBAL_SCHEMA_VERSION SMALLINT, GLOBAL_MANIFEST_SHA256 TEXT",
        "V_TERMINAL.RECEIPT_DIGEST, V_SCHEMA_VERSION, V_MANIFEST_SHA256",
        "H.HEAD_DIGEST, C.CURRENT_SCHEMA_VERSION, PG_CATALOG.BTRIM(C.MANIFEST_SHA256::TEXT)",
        "H.HEAD_DIGEST, V_GLOBAL_SCHEMA_VERSION, V_GLOBAL_MANIFEST_SHA256",
        "AND O.EVENT_DIGEST = E.EVENT_DIGEST AND O.COMMAND_ID = E.COMMAND_ID AND O.REQUEST_DIGEST = E.REQUEST_DIGEST",
        "T.XMIN = PG_CATALOG.PG_CURRENT_XACT_ID()::XID",
        "V_TERMINAL_CURRENT_XACT IS DISTINCT FROM TRUE",
        "PG_CATALOG.SHA256(",
        "LATTICE_POSTGRES_MIGRATION_MANIFEST_V1",
        "PG_CATALOG.INT8SEND(2::BIGINT)",
    ] {
        assert!(
            normalized.contains(required),
            "missing v3 invariant: {required}"
        );
    }
    assert_eq!(
        normalized
            .matches("LATTICE_POSTGRES_MIGRATION_MANIFEST_V1")
            .count(),
        4,
        "all four schema-sensitive runtime entry points must recompute the exact full manifest",
    );

    let read_head = normalized
        .split_once("CREATE FUNCTION CONTROL.TASK_LEDGER_READ_HEAD_V1(")
        .expect("Task Ledger head reader")
        .1
        .split_once("$LATTICE_TASK_LEDGER_READ_HEAD_V1$;")
        .expect("Task Ledger head reader terminator")
        .0;
    for required in [
        "P_STREAM_ID BYTEA, P_EXPECTED_PROJECT_ID TEXT, P_EXPECTED_PROJECT_SNAPSHOT_ID TEXT",
        "P_EXPECTED_PROJECT_ID !~ '^[A-Z0-9][A-Z0-9._-]{1,63}$'",
        "P_EXPECTED_PROJECT_SNAPSHOT_ID !~ '^[A-Z0-9._:-]{1,128}$'",
        "IF V_STREAM_FOUND AND ( V_PROJECT_ID IS DISTINCT FROM P_EXPECTED_PROJECT_ID OR V_PROJECT_SNAPSHOT_ID IS DISTINCT FROM P_EXPECTED_PROJECT_SNAPSHOT_ID ) THEN RAISE EXCEPTION USING ERRCODE = 'LCR01', MESSAGE = 'LEDGER STREAM SCOPE CORRUPT';",
        "SELECT PG_CATALOG.COUNT(*) INTO V_PHYSICAL_COUNT FROM ONLY CONTROL.PHYSICAL_HEADS AS H WHERE H.REPOSITORY_OWNER = 'TASK_LEDGER' AND H.AGGREGATE_KEY_DIGEST = P_STREAM_ID;",
        "V_PHYSICAL_COUNT > 1 OR (V_PHYSICAL_COUNT = 1 AND NOT EXISTS",
        "H.PROJECT_ID = P_EXPECTED_PROJECT_ID AND H.PROJECT_SNAPSHOT_ID = P_EXPECTED_PROJECT_SNAPSHOT_ID",
        "ON H.PROJECT_ID = P_EXPECTED_PROJECT_ID AND H.PROJECT_SNAPSHOT_ID = P_EXPECTED_PROJECT_SNAPSHOT_ID",
        "V_HISTORY_MANIFEST_SHA256 IS DISTINCT FROM V_GLOBAL_MANIFEST_SHA256",
    ] {
        assert!(
            read_head.contains(required),
            "missing Task Ledger head-reader invariant: {required}"
        );
    }
    assert!(
        normalized.contains(
            "REVOKE ALL ON FUNCTION CONTROL.TASK_LEDGER_READ_HEAD_V1( BYTEA, TEXT, TEXT )"
        )
    );
    assert!(normalized.contains(
        "GRANT EXECUTE ON FUNCTION CONTROL.TASK_LEDGER_READ_HEAD_V1( BYTEA, TEXT, TEXT ) TO LATTICE_RUNTIME"
    ));
    assert!(
        !normalized.contains("REVOKE ALL ON FUNCTION CONTROL.TASK_LEDGER_READ_HEAD_V1( BYTEA )")
    );

    let finalizer = normalized
        .split_once("CREATE FUNCTION CONTROL.TASK_LEDGER_FINALIZE_V1(")
        .expect("Task Ledger finalizer")
        .1
        .split_once("$LATTICE_TASK_LEDGER_FINALIZE_V1$;")
        .expect("Task Ledger finalizer terminator")
        .0;
    for required in [
        "OR (V_STREAM_FOUND AND ( V_TERMINAL.EXPECTED_STATE_DIGEST IS DISTINCT FROM P_BASE_CHECKPOINT_DIGEST OR V_TERMINAL.BEFORE_STATE_DIGEST IS DISTINCT FROM P_BASE_CHECKPOINT_DIGEST )) OR V_TERMINAL.AFTER_STATE_DIGEST IS DISTINCT FROM P_NEXT_CHECKPOINT_DIGEST",
        "OR P_NEXT_COMMAND_COUNT::NUMERIC <> 1 OR P_NEXT_EVENT_COUNT::NUMERIC <> (CASE WHEN P_APPEND_EVENT THEN 1 ELSE 0 END)",
        "V_TERMINAL.EXPECTED_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC - 1",
        "V_TERMINAL.BEFORE_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC - 1",
        "V_TERMINAL.AFTER_REVISION::NUMERIC IS DISTINCT FROM P_NEXT_COMMAND_COUNT::NUMERIC",
    ] {
        assert!(
            finalizer.contains(required),
            "missing Task Ledger finalizer invariant: {required}"
        );
    }
    assert!(
        !finalizer.contains(
            "OR V_TERMINAL.EXPECTED_STATE_DIGEST IS DISTINCT FROM P_BASE_CHECKPOINT_DIGEST"
        ),
        "fresh Ledger state must not be equated with the Store genesis domain"
    );
}

#[test]
fn executable_migration_has_runner_owned_transaction_and_no_discovery_escape() {
    let executable = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0002_control_store_foundation")
        .expect("foundation migration");
    let sql = std::str::from_utf8(executable.bytes()).expect("UTF-8 SQL");
    let uppercase = sql.to_ascii_uppercase();

    for forbidden in [
        "BEGIN;",
        "COMMIT;",
        "IF NOT EXISTS",
        "DO $$",
        "EXECUTE ",
        "CREATE EXTENSION",
        "CREATE ROLE",
        "ALTER ROLE",
        "PASSWORD",
        "CREATE DATABASE",
        "DROP TABLE",
        "DROP SCHEMA",
        "DROP FUNCTION",
    ] {
        assert!(
            !uppercase.contains(forbidden),
            "forbidden migration surface: {forbidden}"
        );
    }

    for required in [
        "CREATE SCHEMA CONTROL",
        "CREATE SCHEMA MEMORY",
        "CREATE SCHEMA READMODEL",
        "CREATE TABLE CONTROL.DATABASE_IDENTITY",
        "CREATE TABLE CONTROL.MIGRATION_HISTORY",
        "CREATE TABLE CONTROL.SCHEMA_COMPATIBILITY",
        "CREATE TABLE CONTROL.RUNTIME_ADMISSION",
        "CREATE TABLE CONTROL.PHYSICAL_HEADS",
        "CREATE TABLE CONTROL.TERMINAL_TRANSACTIONS",
        "CONSTRAINT DATABASE_IDENTITY_UUID_V8 CHECK",
        "REVOKE ALL ON SCHEMA CONTROL FROM PUBLIC",
        "ALTER DEFAULT PRIVILEGES",
    ] {
        assert!(
            uppercase.contains(required),
            "missing schema invariant: {required}"
        );
    }
    assert!(!uppercase.contains("DATABASE_IDENTITY_UUID_V5"));
}

#[test]
fn review_regression_sql_nulls_grants_receipt_relations_and_defaults_fail_closed() {
    let executable = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0002_control_store_foundation")
        .expect("foundation migration");
    let sql = std::str::from_utf8(executable.bytes()).expect("UTF-8 SQL");
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();

    assert!(normalized.contains("DAEMON_INSTANCE_ID IS NOT NULL"));
    assert!(normalized.contains("DAEMON_EPOCH IS NOT NULL"));
    assert!(normalized.contains("OBSERVATION_DIGEST IS NOT NULL"));
    assert!(normalized.contains("AUTHORITY_HEAD_DIGEST IS NOT NULL"));

    assert!(!normalized.contains("GRANT SELECT ON ALL TABLES IN SCHEMA CONTROL"));
    assert!(normalized.contains(
        "GRANT SELECT ON CONTROL.DATABASE_IDENTITY, CONTROL.MIGRATION_HISTORY, CONTROL.SCHEMA_COMPATIBILITY, CONTROL.RUNTIME_ADMISSION"
    ));

    assert!(normalized.contains("BEFORE_STATE_DIGEST = EXPECTED_STATE_DIGEST"));
    assert!(normalized.contains("BEFORE_HEAD_DIGEST = EXPECTED_HEAD_DIGEST"));
    assert!(normalized.contains("AFTER_STATE_DIGEST = NEXT_STATE_DIGEST"));
    assert!(normalized.contains("AFTER_STATE_DIGEST = BEFORE_STATE_DIGEST"));
    assert!(normalized.contains("AFTER_HEAD_DIGEST = BEFORE_HEAD_DIGEST"));
    assert!(normalized.contains("AFTER_REVISION - BEFORE_REVISION = 1"));

    for class in ["TABLES", "SEQUENCES", "FUNCTIONS", "TYPES"] {
        assert!(normalized.contains(&format!(
            "ALTER DEFAULT PRIVILEGES FOR ROLE LATTICE_MIGRATOR REVOKE ALL ON {class} FROM PUBLIC"
        )));
    }
    assert!(!normalized.contains("DEFAULT PRIVILEGES FOR ROLE LATTICE_MIGRATOR IN SCHEMA"));

    assert!(normalized.contains("TERMINAL_TRANSACTIONS_DAEMON_INSTANCE_ID CHECK"));
    assert!(normalized.contains("DAEMON_INSTANCE_ID ~ '^[A-Z0-9][A-Z0-9._:-]{0,127}$'"));
}

#[test]
#[allow(clippy::too_many_lines)]
fn live_store_migration_is_fixed_function_gated_and_transaction_control_free() {
    let live = migration_manifest()
        .iter()
        .find(|entry| entry.id() == "0003_live_control_store")
        .expect("live Store migration");
    let sql = std::str::from_utf8(live.bytes()).expect("UTF-8 SQL");
    let uppercase = sql.to_ascii_uppercase();
    let normalized = sql
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();

    for forbidden in [
        "BEGIN;",
        "COMMIT;",
        "ROLLBACK;",
        "IF NOT EXISTS",
        "DO $$",
        "EXECUTE FORMAT",
        "EXECUTE IMMEDIATE",
        "CREATE EXTENSION",
        "CREATE ROLE",
        "ALTER ROLE",
        "PASSWORD",
        "CREATE DATABASE",
        "DROP TABLE",
        "DROP SCHEMA",
        "DROP FUNCTION",
    ] {
        assert!(
            !uppercase.contains(forbidden),
            "forbidden live migration surface: {forbidden}"
        );
    }

    assert_eq!(
        uppercase.matches("CREATE FUNCTION CONTROL.STORE_").count(),
        3
    );
    assert_eq!(uppercase.matches("SECURITY DEFINER").count(), 3);
    assert_eq!(uppercase.matches("SET SEARCH_PATH = PG_CATALOG").count(), 3);
    assert_eq!(uppercase.matches("SET ROW_SECURITY = ON").count(), 3);
    for function in [
        "STORE_PREPARE_V2",
        "STORE_FINALIZE_V2",
        "STORE_CURRENT_HEAD_V2",
    ] {
        assert!(normalized.contains(&format!("CREATE FUNCTION CONTROL.{function}(")));
        assert!(normalized.contains(&format!("GRANT EXECUTE ON FUNCTION CONTROL.{function}(")));
        assert!(normalized.contains(&format!("REVOKE ALL ON FUNCTION CONTROL.{function}(")));
    }
    for required in [
        "ADD COLUMN STORE_CONTRACT_VERSION SMALLINT NOT NULL",
        "ADD COLUMN DATABASE_IDENTITY_DIGEST BYTEA NOT NULL",
        "TERMINAL_TRANSACTIONS_STORE_CONTRACT_V2",
        "TERMINAL_TRANSACTIONS_DATABASE_IDENTITY_DIGEST",
        "SESSION_USER <> 'LATTICE_RUNTIME_LOGIN'",
        "CURRENT_SETTING('TRANSACTION_ISOLATION') <> 'SERIALIZABLE'",
        "FROM ONLY CONTROL.TERMINAL_TRANSACTIONS",
        "FROM ONLY CONTROL.PHYSICAL_HEADS",
        "FOR SHARE OF A",
        "FOR UPDATE OF H",
        "LTX01",
        "LAD01",
        "LAU01",
        "LRV01",
        "IS DISTINCT FROM",
        "V_ADMISSION_MODE IS DISTINCT FROM 'ACTIVE'",
        "V_DAEMON_INSTANCE_ID IS DISTINCT FROM P_DAEMON_INSTANCE_ID",
        "V_AUTHORITY_OBSERVATION_DIGEST IS DISTINCT FROM P_AUTHORITY_OBSERVATION_DIGEST",
        "V_TERMINAL.PRODUCER_ID IS DISTINCT FROM 'LATTICE-POSTGRES-STORE'",
        "V_TERMINAL.DATABASE_UUID IS DISTINCT FROM V_DATABASE_UUID",
        "V_TERMINAL.SCHEMA_VERSION IS DISTINCT FROM V_SCHEMA_VERSION",
        "V_PREPARE.PREPARE_STATUS IS DISTINCT FROM 'PREPARED'",
    ] {
        assert!(
            normalized.contains(required),
            "missing live invariant: {required}"
        );
    }
    assert_eq!(
        normalized
            .matches("DROP CONSTRAINT TERMINAL_TRANSACTIONS_SCOPE_HEAD_FK")
            .count(),
        1,
        "v2 must remove the v1 FK so a stale first use can retain a terminal receipt without materializing genesis",
    );
    assert_eq!(
        normalized
            .matches("INSERT INTO CONTROL.PHYSICAL_HEADS")
            .count(),
        1,
        "only an applied transition may materialize or advance a physical head",
    );
    let terminal_lookup = normalized
        .find("FROM ONLY CONTROL.TERMINAL_TRANSACTIONS")
        .expect("terminal replay lookup");
    let new_work_admission = normalized
        .find("IF P_ADMISSION_MODE IS DISTINCT FROM 'ACTIVE'")
        .expect("new-work admission check");
    assert!(
        terminal_lookup < new_work_admission,
        "replay and changed-ID classification must precede mutable admission"
    );
    assert!(!normalized.contains("OR P_ADMISSION_MODE <> 'ACTIVE'"));
    assert!(!normalized.contains("GRANT SELECT ON CONTROL.PHYSICAL_HEADS"));
    assert!(!normalized.contains("GRANT SELECT ON CONTROL.TERMINAL_TRANSACTIONS"));
    assert!(!normalized.contains("GRANT INSERT ON CONTROL.PHYSICAL_HEADS"));
    assert!(!normalized.contains("GRANT UPDATE ON CONTROL.PHYSICAL_HEADS"));
}

#[test]
fn runner_has_closed_fresh_v1_v2_prefix_and_v3_states() {
    let source = include_str!("../src/postgres_setup.rs");
    for required in [
        "enum InstalledManifestState",
        "Fresh",
        "ExactV1Prefix",
        "ExactV2Prefix",
        "ExactV3Full",
        "classify_installed_manifest_state",
        "verify_v1_upgrade_source",
        "verify_v2_upgrade_source",
        "apply_missing_entries",
        "advance_compatibility_from_v1",
        "advance_compatibility_from_v2",
        "LOCK TABLE control.physical_heads IN ACCESS EXCLUSIVE MODE",
        "LOCK TABLE control.terminal_transactions IN ACCESS EXCLUSIVE MODE",
        "LOCK TABLE control.runtime_admission IN ACCESS EXCLUSIVE MODE",
        "UPDATE ONLY control.schema_compatibility",
        "t.tgisinternal AND t.tgenabled = 'O'",
    ] {
        assert!(
            source.contains(required),
            "missing runner invariant: {required}"
        );
    }
    assert!(!source.contains("apply_manifest_in_transaction"));
}

#[test]
fn migration_target_rejects_default_or_ambiguous_database_identity() {
    let run_id = "0123456789abcdef0123456789abcdef";
    for database in [
        "postgres",
        "template0",
        "template1",
        "",
        "UPPERCASE",
        "has-dash",
        "has.dot",
        " leading",
        "trailing ",
    ] {
        assert!(
            MigrationTarget::new(database, run_id).is_err(),
            "unsafe database accepted: {database:?}"
        );
    }

    for bad_run_id in [
        "",
        "abc",
        "0123456789ABCDEF0123456789ABCDEF",
        "0123456789abcdef0123456789abcdeg",
        "0123456789abcdef0123456789abcdef0",
    ] {
        assert!(
            MigrationTarget::new("lattice_task019_a", bad_run_id).is_err(),
            "unsafe run id accepted: {bad_run_id:?}"
        );
    }

    let target = MigrationTarget::new("lattice_task019_a", run_id).expect("safe target");
    assert_eq!(target.database_name(), "lattice_task019_a");
    assert_eq!(target.run_id(), run_id);
    assert_eq!(
        target.database_comment(),
        "LATTICE_DEVOS_DISPOSABLE_V1:0123456789abcdef0123456789abcdef"
    );
    let expected_uuid = target.expected_database_uuid();
    assert_eq!(expected_uuid.len(), 36);
    assert_eq!(expected_uuid.as_bytes()[14], b'8');
    assert!(matches!(
        expected_uuid.as_bytes()[19],
        b'8' | b'9' | b'a' | b'b'
    ));
    assert_ne!(expected_uuid, "00000000-0000-0000-0000-000000000000");
    let expected_identity = target.expected_database_identity_sha256();
    assert_eq!(expected_identity.as_str().len(), 64);
    assert!(
        expected_identity
            .as_str()
            .bytes()
            .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) })
    );
    assert_ne!(expected_identity.as_str(), &"0".repeat(64));
    assert_eq!(
        expected_uuid,
        MigrationTarget::new("lattice_task019_a", run_id)
            .expect("same safe target")
            .expected_database_uuid()
    );
    assert_eq!(
        expected_identity,
        MigrationTarget::new("lattice_task019_a", run_id)
            .expect("same safe target")
            .expected_database_identity_sha256()
    );
    assert_ne!(
        expected_uuid,
        MigrationTarget::new("lattice_task019_b", run_id)
            .expect("different safe target")
            .expected_database_uuid()
    );
    assert_ne!(
        expected_identity,
        MigrationTarget::new("lattice_task019_b", run_id)
            .expect("different safe target")
            .expected_database_identity_sha256()
    );
    assert!(!format!("{target:?}").contains("password"));
}

#[test]
fn database_roles_are_closed_and_never_a_login_or_caller_value() {
    assert_eq!(
        DatabaseRole::ALL,
        [
            DatabaseRole::Migrator,
            DatabaseRole::Runtime,
            DatabaseRole::Guardian,
            DatabaseRole::ReadOnly,
        ]
    );
    assert_eq!(DatabaseRole::Migrator.as_str(), "lattice_migrator");
    assert_eq!(DatabaseRole::Runtime.as_str(), "lattice_runtime");
    assert_eq!(DatabaseRole::Guardian.as_str(), "lattice_guardian");
    assert_eq!(DatabaseRole::ReadOnly.as_str(), "lattice_readonly");
    assert_eq!(
        DatabaseRole::Migrator.login_role(),
        "lattice_migrator_login"
    );
    assert_eq!(DatabaseRole::Runtime.login_role(), "lattice_runtime_login");
    assert_eq!(
        DatabaseRole::Guardian.login_role(),
        "lattice_guardian_login"
    );
    assert_eq!(
        DatabaseRole::ReadOnly.login_role(),
        "lattice_readonly_login"
    );
}

#[test]
fn setup_errors_are_closed_static_bounded_and_redacted() {
    assert_eq!(PostgresStoreSetupErrorKind::ALL.len(), 15);
    assert!(
        PostgresStoreSetupErrorKind::ALL
            .contains(&PostgresStoreSetupErrorKind::PostApplyVerificationFailed)
    );
    assert_eq!(
        PostgresStoreSetupErrorKind::PostApplyVerificationFailed.code(),
        "STORE_MIGRATION_COMMITTED_UNVERIFIED"
    );
    for kind in PostgresStoreSetupErrorKind::ALL {
        let error = PostgresStoreSetupError::new(kind);
        assert!(!error.code().is_empty());
        assert!(error.code().len() <= 64);
        assert!(
            error
                .code()
                .bytes()
                .all(|byte| { byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_' })
        );
        let display = error.to_string();
        let debug = format!("{error:?}");
        for forbidden in [
            "password",
            "postgres://",
            "127.0.0.1",
            "SELECT ",
            "C:\\",
            "DATABASE_URL",
        ] {
            assert!(!display.contains(forbidden));
            assert!(!debug.contains(forbidden));
        }
    }
}

#[test]
fn driver_and_schema_support_are_exact_for_this_foundation() {
    assert_eq!(POSTGRES_DRIVER_VERSION, "0.19.14");
    assert_eq!(SUPPORTED_POSTGRES_MAJOR, 17);
    assert_eq!(POSTGRES_SCHEMA_VERSION, 3);
}

#[test]
fn review_regression_verifier_uses_one_exact_catalog_snapshot_and_fixed_tables() {
    let source = include_str!("../src/postgres_setup.rs");

    assert!(source.contains(".isolation_level(IsolationLevel::ReadCommitted)"));
    assert!(source.contains(".isolation_level(IsolationLevel::RepeatableRead)"));
    assert!(source.contains("current_setting('transaction_isolation')"));
    assert!(source.contains("current_setting('transaction_read_only')"));
    assert!(source.contains("pg_inherits"));
    assert!(source.contains("c.relhassubclass"));
    assert!(source.contains("c.relispartition"));
    assert!(!source.contains("AND NOT a.attisdropped"));
    assert!(source.contains("COALESCE(array_to_string(p.proconfig, ','), '<NULL>')"));
    assert!(
        source.contains(
            "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
        )
    );
    assert!(source.contains("('pg_catalog.pg_current_xact_id()', 'lattice_migrator'::text)"));

    for table in [
        "control.database_identity",
        "control.migration_history",
        "control.schema_compatibility",
        "control.runtime_admission",
    ] {
        assert!(
            source.contains(&format!("FROM ONLY {table}")),
            "authoritative read does not use ONLY: {table}"
        );
    }
}

#[test]
fn review_regression_requires_real_login_to_capability_role_mapping() {
    let source = include_str!("../src/postgres_setup.rs");
    let live = include_str!("postgres_live.rs");

    for login in [
        "lattice_migrator_login",
        "lattice_runtime_login",
        "lattice_guardian_login",
        "lattice_readonly_login",
    ] {
        assert!(
            source.contains(login),
            "missing fixed login principal: {login}"
        );
    }
    assert!(source.contains("m.inherit_option"));
    assert!(source.contains("m.set_option"));
    assert!(source.contains("m.admin_option"));
    assert!(source.contains("has_schema_privilege($1, n.oid, 'CREATE')"));
    assert!(live.contains("WITH ADMIN FALSE, INHERIT FALSE, SET TRUE"));
    assert!(!live.contains("WITH ADMIN FALSE, INHERIT TRUE, SET TRUE"));
    assert!(live.contains("prove_login_requires_set_role"));
    assert!(live.contains("lattice_readonly_login;"));
    assert!(source.contains("pg_parameter_acl"));
    for acl_catalog in [
        "pg_attribute",
        "pg_language",
        "pg_foreign_data_wrapper",
        "pg_foreign_server",
        "pg_tablespace",
        "pg_largeobject_metadata",
    ] {
        assert!(
            source.contains(acl_catalog),
            "missing ACL closure: {acl_catalog}"
        );
    }
    assert!(source.contains("FROM pg_database d"));
    assert!(source.contains("WHERE acl.grantee = 0"));
    assert!(live.contains("prove_cross_database_acl_drift"));
    assert!(live.contains("prove_parameter_acl_drift"));
    assert!(live.contains("prove_external_column_acl_drift"));
    assert!(source.contains("verify_external_relation_principal_closure"));
    assert!(source.contains("verify_external_function_principal_closure"));
    assert!(source.contains("verify_pre_role_system_function_boundary"));
    assert!(source.contains("verify_large_object_boundary"));
    assert!(source.contains("max_prepared_transactions"));
    assert!(source.contains("FROM pg_shdepend d"));
    assert!(source.contains("a.attacl"));
    assert!(source.contains("c.relacl"));
    assert!(live.contains("prove_external_capability_acl_drift"));
    assert!(live.contains("prove_external_public_acl_drift"));
    assert!(live.contains("prove_external_function_acl_drift"));
    assert!(live.contains("prove_external_function_fixed_acl_drift"));
    assert!(live.contains("prove_non_migrator_default_acl_drift"));
    assert!(live.contains("prove_large_object_acl_drift"));
    assert!(live.contains("prove_login_owner_dependency_drift"));
    assert!(live.contains("PREPARE TRANSACTION 'task019_pre_set_role_forbidden'"));
    assert!(live.contains("pg_cancel_backend"));
    assert!(live.contains("pg_terminate_backend"));
    assert!(live.contains("pg_export_snapshot"));
    assert!(live.contains("pg_current_xact_id"));
    assert!(live.contains("txid_current"));
    assert!(live.contains("lo_import(text, oid)"));
    assert!(live.contains("prove_notifications_are_non_authoritative"));
    assert!(live.contains("NOTIFY lattice_task019, 'ignored'"));
    assert!(live.contains("SqlState::INSUFFICIENT_PRIVILEGE"));
    assert!(live.contains("SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE"));
    assert!(live.contains("pg_logical_emit_message"));
    assert!(live.contains("pg_try_advisory_lock"));
    assert!(!source.contains("pg_catalog.pg_notify(text,text)"));
}

#[test]
fn review_regression_owned_type_and_post_commit_phase_are_closed() {
    let source = include_str!("../src/postgres_setup.rs");
    let live = include_str!("postgres_live.rs");

    assert!(source.contains("TYPE_SIGNATURE_SQL"));
    assert!(source.contains("PostApplyVerificationFailed"));
    assert!(live.contains("CREATE TYPE control.task019_shell"));
    assert!(live.contains("prove_post_apply_verification_failure"));
}

#[test]
fn review_regression_commit_unknown_is_a_real_transport_boundary() {
    let source = include_str!("postgres_live.rs");

    assert!(source.contains("CommitResponseDropProxy"));
    assert!(source.contains("relay_backend_until_commit_ack"));
    assert!(source.contains("frame[0] == b'C'"));
    assert!(source.contains("frame[5..].starts_with(b\"COMMIT\\0\")"));
    assert!(!source.contains("fn inject_commit_response_loss"));
}

#[test]
fn review_regression_harness_cleanup_is_fail_closed_and_preflighted() {
    let source = include_str!("../../../scripts/run-task019-postgres.ps1");
    let pass_position = source
        .rfind("TASK019_POSTGRES_HARNESS=PASS")
        .expect("PASS marker");
    let finalizer_position = source.rfind("finally {").expect("outer finalizer");

    assert!(source.contains("return ($statusExitCode -eq 3)"));
    assert!(source.contains("Assert-NoReparseAncestor"));
    assert!(source.contains("TASK019_HARNESS_SELF_TEST=PASS"));
    assert!(source.contains("TASK019_SERVER_LOG_SANITIZE_FAILED"));
    assert!(source.contains("$safeTokens"));
    assert!(source.contains(".native-stdout.log"));
    assert!(
        pass_position > finalizer_position,
        "PASS marker must follow cleanup and installed-service verification"
    );
}
