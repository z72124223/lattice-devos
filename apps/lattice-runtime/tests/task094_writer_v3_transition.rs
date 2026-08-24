//! TASK-094 composition-root PostgreSQL transition proof.
//!
//! Store and Writer remain independently owned adapters. This runtime test is the
//! only place that composes their public APIs for the disposable live transition.

use std::env;

use lattice_contracts::ContentDigest;
use lattice_postgres_store::{
    DatabaseRole, MigrationApplyOutcome, MigrationTarget, apply_migrations, verify_postgres_schema,
};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionTarget, V3ExtensionTarget, apply_extension, apply_v3_extension,
    rebind_v3_extension,
};
use postgres::config::SslMode;
use postgres::error::SqlState;
use postgres::{Client, Config, IsolationLevel, NoTls};

const REQUIRED_APPLICATION_NAME: &str = "lattice-devos-task019";
const HARNESS_ROLE: &str = "task019_harness";
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

#[derive(Clone)]
struct LiveConfig {
    host: String,
    port: u16,
    password: String,
    run_id: String,
}

impl LiveConfig {
    fn from_environment() -> Option<Self> {
        if env::var("LATTICE_TASK019_LIVE").ok().as_deref() != Some("1") {
            return None;
        }
        let host = required_environment("LATTICE_TASK019_HOST");
        let port = required_environment("LATTICE_TASK019_PORT")
            .parse::<u16>()
            .unwrap_or_else(|_| panic!("TASK094_LIVE_PORT_INVALID"));
        let password = required_environment("LATTICE_TASK019_PASSWORD");
        let run_id = required_environment("LATTICE_TASK019_RUN_ID");
        assert_eq!(
            required_environment("LATTICE_TASK019_PHASE"),
            "task094_transition",
            "TASK094_LIVE_PHASE_INVALID"
        );
        assert_eq!(host, "127.0.0.1", "TASK094_LIVE_HOST_INVALID");
        assert!(
            port != 0 && port != 5432 && port != 58_743,
            "TASK094_LIVE_PORT_INVALID"
        );
        assert!(!password.is_empty(), "TASK094_LIVE_PASSWORD_MISSING");
        assert!(is_lower_hex(&run_id, 32), "TASK094_LIVE_RUN_ID_INVALID");
        Some(Self {
            host,
            port,
            password,
            run_id,
        })
    }

    fn connect(&self, database: &str, application_name: &str) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user(HARNESS_ROLE)
            .password(&self.password)
            .dbname(database)
            .application_name(application_name)
            .ssl_mode(SslMode::Disable);
        config
            .connect(NoTls)
            .unwrap_or_else(|_| panic!("TASK094_LIVE_CONNECT_FAILED"))
    }

    fn role_client(&self, database: &str, role: DatabaseRole, application_name: &str) -> Client {
        let mut client = self.connect_as(database, role.login_role(), application_name);
        client
            .batch_execute(set_role_sql(role))
            .unwrap_or_else(|_| panic!("TASK094_SET_ROLE_FAILED"));
        client
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
            .unwrap_or_else(|_| panic!("TASK094_LIVE_CONNECT_FAILED"))
    }

    fn target(&self) -> MigrationTarget {
        MigrationTarget::new(
            format!("lattice_task019_{}_transition", &self.run_id[..8]),
            self.run_id.clone(),
        )
        .unwrap_or_else(|_| panic!("TASK094_TARGET_CONSTRUCTION_FAILED"))
    }
}

#[test]
fn task094_writer_v3_transition_composition() {
    let Some(config) = LiveConfig::from_environment() else {
        return;
    };
    let mut admin = config.connect("postgres", "lattice-devos-task094-admin");
    create_fixed_roles(&mut admin, &config.password);
    let target = provision_database(&config, &mut admin);
    drop(admin);

    println!("TASK094_STAGE_FRESH_V5_ENTER");
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("TASK094_FRESH_V5_FAILED"),
        MigrationApplyOutcome::Applied {
            executable_count: 5
        },
        "TASK094_FRESH_MUST_STOP_AT_EXACT_V5"
    );
    drop(migrator);
    println!("TASK094_STAGE_FRESH_V5_PASS");

    println!("TASK094_STAGE_MEMORY_V3_ENTER");
    install_codebase_memory_v2(&config, &target);
    upgrade_codebase_memory_v3(&config, &target);
    println!("TASK094_STAGE_MEMORY_V3_PASS");

    let database_identity = ContentDigest::from_sha256(
        target
            .expected_database_identity_sha256()
            .as_str()
            .to_owned(),
    )
    .expect("TASK094_DATABASE_IDENTITY_DIGEST");
    let writer_v2_target = ExtensionTarget::new(
        target.database_name().to_owned(),
        database_identity.clone(),
        ContentDigest::from_sha256(CURRENT_V5_MANIFEST_SHA256.to_owned())
            .expect("TASK094_V5_MANIFEST_DIGEST"),
        ContentDigest::from_sha256(CODEBASE_MEMORY_V3_MANIFEST_SHA256.to_owned())
            .expect("TASK094_MEMORY_V3_MANIFEST_DIGEST"),
    )
    .expect("TASK094_WRITER_V2_TARGET");
    let writer_v3_target =
        V3ExtensionTarget::new(target.database_name().to_owned(), database_identity)
            .expect("TASK094_WRITER_V3_TARGET");
    let mut migrator = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    println!("TASK094_STAGE_WRITER_V2_ENTER");
    assert_eq!(
        apply_extension(&mut migrator, &writer_v2_target).expect("TASK094_WRITER_V2_CURRENT"),
        ExtensionApplyOutcome::Installed
    );
    println!("TASK094_STAGE_WRITER_V2_PASS");
    println!("TASK094_STAGE_WRITER_V3_BRIDGE_ENTER");
    assert_eq!(
        apply_v3_extension(&mut migrator, &writer_v3_target).expect("TASK094_WRITER_V3_BRIDGE"),
        ExtensionApplyOutcome::Bridged
    );
    println!("TASK094_STAGE_WRITER_V3_BRIDGE_PASS");

    println!("TASK094_STAGE_REBIND_FAILURE_ATOMICITY_ENTER");
    insert_active_head_failure_fixture(&mut migrator);
    assert_active_head_rebind_sqlstate(&mut migrator);
    assert_rebind_failure_preserves_exact_v5_bridge(&mut migrator, &target, "active_head");
    migrator
        .execute(
            "DELETE FROM ONLY writer_lease.writer_lease_heads WHERE project_id='task094-failure-active'",
            &[],
        )
        .expect("TASK094_REMOVE_ACTIVE_HEAD_FAILURE_FIXTURE");
    assert_drift_failure_preserves_state(
        &mut migrator,
        &target,
        "identity",
        "UPDATE ONLY writer_lease.writer_lease_extension_identity \
         SET global_manifest_sha256 = repeat('a', 64) WHERE singleton",
        "UPDATE ONLY writer_lease.writer_lease_extension_identity \
         SET global_manifest_sha256 = $1 WHERE singleton",
        &[&CURRENT_V5_MANIFEST_SHA256],
    );
    assert_ledger_drift_preserves_state(&mut migrator, &target);
    assert_acl_drift_preserves_state(&mut migrator, &target);
    assert_exact_v5_bridge(&mut migrator);
    println!("TASK094_STAGE_REBIND_FAILURE_ATOMICITY_PASS");

    println!("TASK094_STAGE_STORE_V6_ENTER");
    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("TASK094_STORE_V6_FAILED"),
        MigrationApplyOutcome::Applied {
            executable_count: 1
        },
        "TASK094_EXACT_V5_TO_V6_OUTCOME"
    );
    let evidence = verify_postgres_schema(&mut migrator, &target, DatabaseRole::Migrator)
        .expect("TASK094_STORE_V6_SCHEMA_VERIFY");
    assert_eq!(evidence.schema_version(), 6);
    assert_eq!(
        apply_migrations(&mut migrator, &target).expect("TASK094_STORE_V6_RETRY_FAILED"),
        MigrationApplyOutcome::AlreadyCurrent
    );
    assert_eq!(
        rebind_v3_extension(&mut migrator, &writer_v3_target)
            .expect("TASK094_WRITER_V3_REBIND_RETRY"),
        ExtensionApplyOutcome::AlreadyCurrent
    );
    println!("TASK094_STAGE_STORE_V6_PASS");
    println!(
        "TASK094_TRANSITION_OK database_uuid={} manifest_sha256={}",
        evidence.database_uuid(),
        evidence.manifest_sha256().as_str()
    );
}

fn assert_active_head_rebind_sqlstate(client: &mut Client) {
    let failure = client
        .batch_execute("CALL writer_lease.writer_lease_rebind_v3()")
        .expect_err("TASK094_ACTIVE_HEAD_MUST_REJECT_WRITER_REBIND");
    assert_eq!(
        failure
            .as_db_error()
            .expect("TASK094_ACTIVE_HEAD_DATABASE_ERROR")
            .code(),
        &SqlState::OBJECT_NOT_IN_PREREQUISITE_STATE,
        "TASK094_ACTIVE_HEAD_SQLSTATE_MUST_BE_55000"
    );
}

fn assert_rebind_failure_preserves_exact_v5_bridge(
    client: &mut Client,
    target: &MigrationTarget,
    label: &str,
) {
    let before = v5_bridge_fingerprint(client);
    let failure = apply_migrations(client, target).expect_err("TASK094_REBIND_FAILURE_MUST_FAIL");
    assert_eq!(
        failure.kind().code(),
        "STORE_MIGRATION_TRANSACTION_FAILED",
        "TASK094_{label}_FAILURE_KIND"
    );
    assert_eq!(
        v5_bridge_fingerprint(client),
        before,
        "TASK094_{label}_MUST_ROLL_BACK_HISTORY_COMPATIBILITY_IDENTITY_LEDGER_AND_RUNTIME_ACL"
    );
}

fn assert_drift_failure_preserves_state(
    client: &mut Client,
    target: &MigrationTarget,
    label: &str,
    introduce_sql: &str,
    repair_sql: &str,
    repair_parameters: &[&(dyn postgres::types::ToSql + Sync)],
) {
    let introduced = client
        .execute(introduce_sql, &[])
        .unwrap_or_else(|_| panic!("TASK094_{label}_DRIFT_INTRODUCTION_FAILED"));
    assert_eq!(introduced, 1, "TASK094_{label}_DRIFT_MUST_CHANGE_ONE_ROW");
    let before = v5_bridge_fingerprint(client);
    let failure = apply_migrations(client, target).expect_err("TASK094_DRIFT_MUST_FAIL_CLOSED");
    assert_eq!(
        failure.kind().code(),
        "STORE_MIGRATION_TRANSACTION_FAILED",
        "TASK094_{label}_DRIFT_FAILURE_KIND"
    );
    assert_eq!(
        v5_bridge_fingerprint(client),
        before,
        "TASK094_{label}_DRIFT_MUST_NOT_PARTIALLY_APPLY"
    );
    client
        .execute(repair_sql, repair_parameters)
        .unwrap_or_else(|_| panic!("TASK094_{label}_DRIFT_REPAIR_FAILED"));
}

fn assert_acl_drift_preserves_state(client: &mut Client, target: &MigrationTarget) {
    client
        .batch_execute("GRANT USAGE ON SCHEMA writer_lease TO lattice_runtime")
        .expect("TASK094_ACL_DRIFT_INTRODUCTION_FAILED");
    let before = v5_bridge_fingerprint(client);
    let failure = apply_migrations(client, target).expect_err("TASK094_ACL_DRIFT_MUST_FAIL_CLOSED");
    assert_eq!(
        failure.kind().code(),
        "STORE_POSTGRES_CATALOG_CORRUPT",
        "TASK094_ACL_DRIFT_FAILURE_KIND"
    );
    assert_eq!(
        v5_bridge_fingerprint(client),
        before,
        "TASK094_ACL_DRIFT_MUST_NOT_PARTIALLY_APPLY"
    );
    client
        .batch_execute("REVOKE USAGE ON SCHEMA writer_lease FROM lattice_runtime")
        .expect("TASK094_ACL_DRIFT_REPAIR_FAILED");
}

fn assert_ledger_drift_preserves_state(client: &mut Client, target: &MigrationTarget) {
    client
        .batch_execute(
            "ALTER TABLE writer_lease.writer_lease_extension_ledger \
             DROP CONSTRAINT writer_lease_extension_ledger_profile_v3; \
             UPDATE ONLY writer_lease.writer_lease_extension_ledger \
             SET ledger_ordinal = 3 WHERE ledger_ordinal = 2",
        )
        .expect("TASK094_LEDGER_DRIFT_INTRODUCTION_FAILED");
    let before = v5_bridge_fingerprint(client);
    let failure =
        apply_migrations(client, target).expect_err("TASK094_LEDGER_DRIFT_MUST_FAIL_CLOSED");
    assert_eq!(
        failure.kind().code(),
        "STORE_POSTGRES_CATALOG_CORRUPT",
        "TASK094_LEDGER_DRIFT_FAILURE_KIND"
    );
    assert_eq!(
        v5_bridge_fingerprint(client),
        before,
        "TASK094_LEDGER_DRIFT_MUST_NOT_PARTIALLY_APPLY"
    );
    client
        .batch_execute(
            "UPDATE ONLY writer_lease.writer_lease_extension_ledger \
             SET ledger_ordinal = 2 WHERE ledger_ordinal = 3; \
             ALTER TABLE writer_lease.writer_lease_extension_ledger \
             ADD CONSTRAINT writer_lease_extension_ledger_profile_v3 CHECK (\
                extension_id = 'lattice-writer-lease' AND (\
                    (ledger_ordinal = 1 AND extension_schema_version = 1 \
                     AND global_schema_version = 3 AND required_memory_schema_version = 2 \
                     AND event_kind = 'INSTALLED') OR \
                    (ledger_ordinal = 1 AND extension_schema_version = 2 \
                     AND global_schema_version = 5 AND required_memory_schema_version = 3 \
                     AND event_kind = 'INSTALLED') OR \
                    (ledger_ordinal = 1 AND extension_schema_version = 3 \
                     AND global_schema_version = 6 AND required_memory_schema_version = 3 \
                     AND event_kind = 'INSTALLED') OR \
                    (ledger_ordinal = 2 AND extension_schema_version = 2 \
                     AND global_schema_version = 3 AND required_memory_schema_version = 2 \
                     AND event_kind = 'UPGRADED') OR \
                    (ledger_ordinal = 2 AND extension_schema_version = 3 \
                     AND global_schema_version = 5 AND required_memory_schema_version = 3 \
                     AND event_kind = 'UPGRADED') OR \
                    (ledger_ordinal = 3 AND extension_schema_version = 2 \
                     AND global_schema_version = 5 AND required_memory_schema_version = 3 \
                     AND event_kind = 'REBOUND') OR \
                    (ledger_ordinal = 3 AND extension_schema_version = 3 \
                     AND global_schema_version = 6 AND required_memory_schema_version = 3 \
                     AND event_kind = 'REBOUND') OR \
                    (ledger_ordinal = 4 AND extension_schema_version = 3 \
                     AND global_schema_version = 5 AND required_memory_schema_version = 3 \
                     AND event_kind = 'UPGRADED') OR \
                    (ledger_ordinal = 5 AND extension_schema_version = 3 \
                     AND global_schema_version = 6 AND required_memory_schema_version = 3 \
                     AND event_kind = 'REBOUND')))",
        )
        .expect("TASK094_LEDGER_DRIFT_REPAIR_FAILED");
}

fn insert_active_head_failure_fixture(client: &mut Client) {
    client
        .execute(
            "INSERT INTO writer_lease.writer_lease_heads (\
                 project_id,row_version,snapshot_schema_version,snapshot_bytes,\
                 snapshot_bytes_sha256,snapshot_digest,fencing_high_water,lease_revision,\
                 command_high_water,command_tail_digest,current_status,current_receipt_digest,\
                 current_project_snapshot_id,current_task_id,current_task_revision,\
                 current_task_spec_digest,current_attempt_id,current_lease_id,\
                 current_lease_holder_id,current_worktree_id,current_holder_process_id,\
                 current_holder_process_start_identity,current_daemon_instance_id,\
                 current_daemon_epoch,current_fencing_token,current_expires_at) VALUES (\
                 'task094-failure-active',0,1,decode('01','hex'),\
                 pg_catalog.sha256(decode('01','hex')),decode(repeat('11',32),'hex'),\
                 1,1,0,NULL,'ACTIVE',decode(repeat('12',32),'hex'),'snapshot-094',\
                 'task-094','1',decode(repeat('13',32),'hex'),'attempt-094','lease-094',\
                 'holder-094','worktree-094',1,decode(repeat('14',32),'hex'),'daemon-094',\
                 1,1,'2026-08-24T00:00:00Z')",
            &[],
        )
        .expect("TASK094_INSERT_ACTIVE_HEAD_FAILURE_FIXTURE");
}

fn v5_bridge_fingerprint(client: &mut Client) -> Vec<String> {
    [
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(t)::text,E'\\n' ORDER BY t.ordinal),'')) \
           FROM ONLY control.migration_history t",
        "SELECT pg_catalog.md5(pg_catalog.to_jsonb(c)::text) \
           FROM ONLY control.schema_compatibility c WHERE c.singleton",
        "SELECT pg_catalog.md5(pg_catalog.to_jsonb(w)::text) \
           FROM ONLY writer_lease.writer_lease_extension_identity w WHERE w.singleton",
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             pg_catalog.to_jsonb(l)::text,E'\\n' ORDER BY l.ledger_ordinal),'')) \
           FROM ONLY writer_lease.writer_lease_extension_ledger l",
        "SELECT pg_catalog.md5(COALESCE(pg_catalog.string_agg(\
             p.proname::text || ':' || \
             pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')::text, \
             E'\\n' ORDER BY p.proname,pg_catalog.pg_get_function_identity_arguments(p.oid)),'')) \
           FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
          WHERE n.nspname='writer_lease'",
    ]
    .into_iter()
    .map(|query| {
        client
            .query_one(query, &[])
            .unwrap_or_else(|_| panic!("TASK094_V5_BRIDGE_FINGERPRINT_QUERY_FAILED"))
            .get(0)
    })
    .collect()
}

fn assert_exact_v5_bridge(client: &mut Client) {
    let row = client
        .query_one(
            "SELECT \
               (SELECT pg_catalog.count(*) FROM ONLY control.migration_history) = 6, \
               NOT EXISTS (SELECT 1 FROM ONLY control.migration_history WHERE ordinal=7), \
               (SELECT manifest_sha256=$1 AND current_schema_version=5 \
                         AND min_reader=5 AND max_reader=5 AND min_writer=5 AND max_writer=5 \
                  FROM ONLY control.schema_compatibility WHERE singleton), \
               (SELECT extension_schema_version=3 AND global_schema_version=5 \
                         AND global_manifest_sha256=$1 \
                  FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
               (SELECT pg_catalog.string_agg(ledger_ordinal::text || ':' || event_kind::text || \
                   ':' || extension_schema_version::text || ':' || global_schema_version::text, \
                   ',' ORDER BY ledger_ordinal) \
                  FROM ONLY writer_lease.writer_lease_extension_ledger) = '1:INSTALLED:2:5,2:UPGRADED:3:5', \
               NOT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
               (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc p \
                 JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 WHERE n.nspname='writer_lease' \
                   AND pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')) = 0",
            &[&CURRENT_V5_MANIFEST_SHA256],
        )
        .expect("TASK094_EXACT_V5_BRIDGE_QUERY");
    for index in 0..7 {
        assert!(
            row.get::<_, bool>(index),
            "TASK094_EXACT_V5_BRIDGE_ASSERTION_{index}"
        );
    }
}

fn create_fixed_roles(admin: &mut Client, password: &str) {
    let quoted_password: String = admin
        .query_one("SELECT quote_literal($1::text)", &[&password])
        .expect("TASK094_PASSWORD_QUOTE_FAILED")
        .get(0);
    admin
        .batch_execute(&format!(
            "CREATE ROLE lattice_migrator NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_runtime NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_guardian NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_readonly NOLOGIN NOSUPERUSER INHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1; \
             CREATE ROLE lattice_migrator_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_runtime_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_guardian_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             CREATE ROLE lattice_readonly_login LOGIN NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1 PASSWORD {quoted_password}; \
             GRANT lattice_migrator TO lattice_migrator_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_runtime TO lattice_runtime_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_guardian TO lattice_guardian_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             GRANT lattice_readonly TO lattice_readonly_login WITH ADMIN FALSE, INHERIT FALSE, SET TRUE; \
             REVOKE ALL ON DATABASE postgres FROM PUBLIC; \
             REVOKE ALL ON DATABASE template0 FROM PUBLIC; \
             REVOKE ALL ON DATABASE template1 FROM PUBLIC"
        ))
        .expect("TASK094_ROLE_PROVISION_FAILED");
}

fn provision_database(config: &LiveConfig, admin: &mut Client) -> MigrationTarget {
    let target = config.target();
    let quoted_name = quoted_database_name(target.database_name());
    admin
        .batch_execute(&format!(
            "CREATE DATABASE {quoted_name} OWNER lattice_migrator"
        ))
        .expect("TASK094_DATABASE_CREATE_FAILED");
    set_exact_database_access(admin, target.database_name());
    set_exact_pre_role_function_access(config, target.database_name());
    admin
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; COMMENT ON DATABASE {quoted_name} IS '{}'; RESET ROLE",
            target.database_comment()
        ))
        .expect("TASK094_DATABASE_BOUNDARY_PROVISION_FAILED");
    target
}

fn set_exact_database_access(admin: &mut Client, target_database: &str) {
    let database_names = admin
        .query(
            "SELECT datname::text FROM pg_database ORDER BY datname",
            &[],
        )
        .expect("TASK094_DATABASE_INVENTORY_FAILED");
    for row in &database_names {
        let database: String = row.get(0);
        let quoted = quoted_database_name(&database);
        admin
            .batch_execute(&format!(
                "REVOKE ALL ON DATABASE {quoted} FROM PUBLIC; \
                 REVOKE ALL ON DATABASE {quoted} FROM lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login"
            ))
            .expect("TASK094_DATABASE_ACCESS_REVOKE_FAILED");
    }
    let quoted_target = quoted_database_name(target_database);
    admin
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; \
             GRANT CONNECT ON DATABASE {quoted_target} TO lattice_migrator, lattice_runtime, \
                 lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; RESET ROLE"
        ))
        .expect("TASK094_DATABASE_ACCESS_GRANT_FAILED");
}

fn set_exact_pre_role_function_access(config: &LiveConfig, database: &str) {
    let mut admin = config.connect(database, "task094-function-boundary-provision");
    admin
        .batch_execute(
            "REVOKE ALL PRIVILEGES ON FUNCTION \
                 pg_catalog.lo_creat(integer), pg_catalog.lo_create(oid), \
                 pg_catalog.lo_from_bytea(oid, bytea), pg_catalog.lo_import(text), \
                 pg_catalog.lo_import(text, oid), \
                 pg_catalog.pg_logical_emit_message(boolean, text, text, boolean), \
                 pg_catalog.pg_logical_emit_message(boolean, text, bytea, boolean), \
                 pg_catalog.pg_advisory_lock(bigint), pg_catalog.pg_advisory_lock(integer, integer), \
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
                 pg_catalog.txid_current() FROM PUBLIC, lattice_migrator, lattice_runtime, \
                 lattice_guardian, lattice_readonly, lattice_migrator_login, lattice_runtime_login, \
                 lattice_guardian_login, lattice_readonly_login; \
             GRANT EXECUTE ON FUNCTION pg_catalog.pg_try_advisory_lock(bigint), \
                 pg_catalog.pg_advisory_xact_lock(bigint), pg_catalog.pg_current_xact_id() \
                 TO lattice_migrator",
        )
        .expect("TASK094_FUNCTION_BOUNDARY_PROVISION_FAILED");
}

fn install_codebase_memory_v2(config: &LiveConfig, target: &MigrationTarget) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .expect("TASK094_MEMORY_V2_FIXTURE_TRANSACTION_FAILED");
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .expect("TASK094_MEMORY_V2_FIXTURE_HARDEN_FAILED");
    transaction
        .batch_execute(CODEBASE_MEMORY_V2_SQL)
        .expect("TASK094_MEMORY_V2_FIXTURE_SQL_FAILED");
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_identity (singleton, extension_id, \
             extension_schema_version, extension_path, extension_sql_sha256, extension_manifest_sha256, \
             database_uuid, database_identity_sha256, global_schema_version, global_manifest_sha256) \
             VALUES (true, 'lattice-codebase-memory', 2, $1, $2, $3, $4::text::uuid, $5, 3, $6)",
            &[&CODEBASE_MEMORY_V2_PATH, &CODEBASE_MEMORY_V2_SQL_SHA256, &CODEBASE_MEMORY_V2_MANIFEST_SHA256,
                &target.expected_database_uuid(), &target.expected_database_identity_sha256().as_str(),
                &"09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407"],
        )
        .expect("TASK094_MEMORY_V2_FIXTURE_IDENTITY_FAILED");
    transaction
        .execute(
            "INSERT INTO memory.codebase_memory_extension_ledger (ledger_ordinal, singleton, extension_id, \
             extension_schema_version, extension_sql_sha256, extension_manifest_sha256, database_uuid, \
             database_identity_sha256, global_schema_version, global_manifest_sha256, event_kind) \
             VALUES (1, true, 'lattice-codebase-memory', 2, $1, $2, $3::text::uuid, $4, 3, $5, 'INSTALLED')",
            &[&CODEBASE_MEMORY_V2_SQL_SHA256, &CODEBASE_MEMORY_V2_MANIFEST_SHA256,
                &target.expected_database_uuid(), &target.expected_database_identity_sha256().as_str(),
                &"09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407"],
        )
        .expect("TASK094_MEMORY_V2_FIXTURE_LEDGER_FAILED");
    transaction
        .commit()
        .expect("TASK094_MEMORY_V2_FIXTURE_COMMIT_FAILED");
}

fn upgrade_codebase_memory_v3(config: &LiveConfig, target: &MigrationTarget) {
    let mut client = config.role_client(
        target.database_name(),
        DatabaseRole::Migrator,
        REQUIRED_APPLICATION_NAME,
    );
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .expect("TASK094_MEMORY_V3_FIXTURE_TRANSACTION_FAILED");
    transaction
        .batch_execute("SET LOCAL search_path = pg_catalog; SET LOCAL row_security = on")
        .expect("TASK094_MEMORY_V3_FIXTURE_HARDEN_FAILED");
    transaction
        .batch_execute(CODEBASE_MEMORY_V3_SQL)
        .expect("TASK094_MEMORY_V3_FIXTURE_SQL_FAILED");
    assert_eq!(
        transaction.execute(
            "UPDATE ONLY memory.codebase_memory_extension_identity SET extension_schema_version = 3, \
             extension_path = $1, extension_sql_sha256 = $2, extension_manifest_sha256 = $3, \
             global_schema_version = 5, global_manifest_sha256 = $4 WHERE singleton \
             AND extension_schema_version = 2 AND extension_path = $5 AND global_schema_version = 3 \
             AND global_manifest_sha256 = $6",
            &[&CODEBASE_MEMORY_V3_PATH, &CODEBASE_MEMORY_V3_SQL_SHA256, &CODEBASE_MEMORY_V3_MANIFEST_SHA256,
                &CURRENT_V5_MANIFEST_SHA256, &CODEBASE_MEMORY_V2_PATH,
                &"09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407"],
        ).expect("TASK094_MEMORY_V3_FIXTURE_IDENTITY_FAILED"),
        1,
        "TASK094_MEMORY_V3_FIXTURE_IDENTITY_MISSING"
    );
    transaction.execute(
        "INSERT INTO memory.codebase_memory_extension_ledger (ledger_ordinal, singleton, extension_id, \
         extension_schema_version, extension_sql_sha256, extension_manifest_sha256, database_uuid, \
         database_identity_sha256, global_schema_version, global_manifest_sha256, event_kind) \
         VALUES (2, true, 'lattice-codebase-memory', 3, $1, $2, $3::text::uuid, $4, 5, $5, 'UPGRADED')",
        &[&CODEBASE_MEMORY_V3_SQL_SHA256, &CODEBASE_MEMORY_V3_MANIFEST_SHA256,
            &target.expected_database_uuid(), &target.expected_database_identity_sha256().as_str(),
            &CURRENT_V5_MANIFEST_SHA256],
    ).expect("TASK094_MEMORY_V3_FIXTURE_LEDGER_FAILED");
    transaction
        .commit()
        .expect("TASK094_MEMORY_V3_FIXTURE_COMMIT_FAILED");
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
        "TASK094_DATABASE_IDENTIFIER_INVALID"
    );
    format!("\"{value}\"")
}

fn required_environment(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("TASK094_REQUIRED_ENVIRONMENT_MISSING"))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
