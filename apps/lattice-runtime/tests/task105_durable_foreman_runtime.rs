//! Marker-owned PostgreSQL acceptance for TASK-105.

use std::env;
use std::io::Write;
use std::process::{Command, Stdio};

use lattice_contracts::ContentDigest;
use lattice_postgres_codebase_memory::{
    ExtensionTarget as MemoryExtensionTarget, apply_extension as apply_memory_extension,
    verify_embedded_extension_manifest as verify_memory_manifest,
};
use lattice_postgres_store::{
    DatabaseRole, MigrationApplyOutcome, MigrationTarget, apply_migrations,
};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionTarget as WriterExtensionTarget, V3ExtensionTarget,
    apply_extension as apply_writer_extension, apply_v3_extension,
};
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls};
use serde_json::{Value, json};

const V5_MANIFEST_SHA256: &str = "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
const WRITER_V3_MANIFEST_SHA256: &str =
    "eab2812fa3d94cd3466d7c003386f805a973fd7def1f16aeb15b52f47dad78e4";

struct LiveConfig {
    host: String,
    port: u16,
    password: String,
    run_id: String,
}

impl LiveConfig {
    fn from_environment() -> Option<Self> {
        if env::var("LATTICE_TASK105_LIVE").ok().as_deref() != Some("1") {
            return None;
        }
        assert_eq!(required("LATTICE_TASK105_PHASE"), "durable_foreman_restart");
        let host = required("LATTICE_TASK019_HOST");
        let port = required("LATTICE_TASK019_PORT")
            .parse::<u16>()
            .expect("TASK105_PORT_INVALID");
        let password = required("LATTICE_TASK019_PASSWORD");
        let run_id = required("LATTICE_TASK019_RUN_ID");
        assert_eq!(host, "127.0.0.1");
        assert!(port != 0 && port != 5432 && port != 58_743 && port != 4317);
        assert_eq!(run_id.len(), 32);
        assert!(
            run_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        Some(Self {
            host,
            port,
            password,
            run_id,
        })
    }

    fn database_name(&self) -> String {
        format!("lattice_task019_{}_base", &self.run_id[..8])
    }

    fn child_database(&self, discriminator: u32) -> Self {
        assert_ne!(discriminator, 0);
        let prefix = u32::from_str_radix(&self.run_id[..8], 16).expect("TASK105_CHILD_RUN_PREFIX")
            ^ discriminator;
        Self {
            host: self.host.clone(),
            port: self.port,
            password: self.password.clone(),
            run_id: format!("{prefix:08x}{}", &self.run_id[8..]),
        }
    }

    fn bootstrap_client(&self) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user("runtime_bootstrap")
            .password(&self.password)
            .dbname(&self.database_name())
            .application_name("lattice-task105-migration-observer")
            .ssl_mode(SslMode::Disable);
        config.connect(NoTls).expect("TASK105_BOOTSTRAP_CONNECT")
    }

    fn migration_target(&self) -> MigrationTarget {
        MigrationTarget::new(self.database_name(), self.run_id.clone())
            .expect("TASK105_MIGRATION_TARGET")
    }

    fn migrator_client(&self) -> Client {
        let mut config = Config::new();
        config
            .host(&self.host)
            .port(self.port)
            .user(DatabaseRole::Migrator.login_role())
            .password(&self.password)
            .dbname(&self.database_name())
            .application_name("lattice-devos-task019")
            .ssl_mode(SslMode::Disable);
        let mut client = config.connect(NoTls).expect("TASK105_MIGRATOR_CONNECT");
        client
            .batch_execute("SET ROLE lattice_migrator")
            .expect("TASK105_MIGRATOR_ROLE");
        client
    }

    fn prepare_v5_store_only(&self) {
        let target = self.migration_target();
        let mut migrator = self.migrator_client();
        assert_eq!(
            apply_migrations(&mut migrator, &target).expect("TASK105_FIXTURE_STORE_V5"),
            MigrationApplyOutcome::Applied {
                executable_count: 5
            }
        );
    }

    fn prepare_v5_writer_v2_current(&self) {
        self.prepare_v5_store_only();
        let target = self.migration_target();
        let mut migrator = self.migrator_client();
        let memory_target = MemoryExtensionTarget::new(self.database_name(), self.run_id.clone())
            .expect("TASK105_FIXTURE_MEMORY_TARGET");
        apply_memory_extension(&mut migrator, &memory_target).expect("TASK105_FIXTURE_MEMORY_V3");
        let memory = verify_memory_manifest().expect("TASK105_FIXTURE_MEMORY_MANIFEST");
        let database_identity =
            ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
                .expect("TASK105_FIXTURE_DATABASE_IDENTITY");
        let writer_v2 = WriterExtensionTarget::new(
            self.database_name(),
            database_identity.clone(),
            ContentDigest::from_sha256(V5_MANIFEST_SHA256).expect("TASK105_FIXTURE_V5_MANIFEST"),
            memory.manifest_sha256().clone(),
        )
        .expect("TASK105_FIXTURE_WRITER_V2_TARGET");
        assert_eq!(
            apply_writer_extension(&mut migrator, &writer_v2).expect("TASK105_FIXTURE_WRITER_V2"),
            ExtensionApplyOutcome::Installed
        );
    }

    fn prepare_v5_writer_v3_bridge(&self) {
        self.prepare_v5_writer_v2_current();
        let target = self.migration_target();
        let mut migrator = self.migrator_client();
        let database_identity =
            ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
                .expect("TASK105_FIXTURE_DATABASE_IDENTITY");
        let writer_v3 = V3ExtensionTarget::new(self.database_name(), database_identity)
            .expect("TASK105_FIXTURE_WRITER_V3_TARGET");
        assert_eq!(
            apply_v3_extension(&mut migrator, &writer_v3).expect("TASK105_FIXTURE_WRITER_V3"),
            ExtensionApplyOutcome::Bridged
        );
    }

    fn assert_v5_writer_v2_current(&self) {
        let row = self
            .bootstrap_client()
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*)=6 FROM ONLY control.migration_history), \
                    (SELECT current_schema_version=5 AND manifest_sha256=$1 \
                       FROM ONLY control.schema_compatibility WHERE singleton), \
                    (SELECT extension_schema_version=2 AND global_schema_version=5 \
                       AND required_memory_schema_version=3 \
                       FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    pg_catalog.has_function_privilege('lattice_runtime', \
                      'writer_lease.writer_lease_bind_runtime_v2(text,bigint,bytea,text,text,text,text,text)', \
                      'EXECUTE'), \
                    pg_catalog.to_regprocedure( \
                      'writer_lease.writer_lease_bind_runtime_v3(text,bigint,bytea,text,text,text,text,text)') \
                      IS NULL",
                &[&V5_MANIFEST_SHA256],
            )
            .expect("TASK105_V5_WRITER_V2_PROFILE");
        for index in 0..6 {
            assert!(row.get::<_, bool>(index), "TASK105_V5_WRITER_V2_{index}");
        }
    }

    fn assert_v5_writer_absent(&self) {
        let row = self
            .bootstrap_client()
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*)=6 FROM ONLY control.migration_history), \
                    (SELECT current_schema_version=5 AND manifest_sha256=$1 \
                       FROM ONLY control.schema_compatibility WHERE singleton), \
                    pg_catalog.to_regnamespace('writer_lease') IS NULL, \
                    pg_catalog.to_regnamespace('memory') IS NULL, \
                    (SELECT admission_mode='STOPPED' FROM ONLY control.runtime_admission \
                       WHERE singleton)",
                &[&V5_MANIFEST_SHA256],
            )
            .expect("TASK105_V5_WRITER_ABSENT_PROFILE");
        for index in 0..5 {
            assert!(
                row.get::<_, bool>(index),
                "TASK105_V5_WRITER_ABSENT_{index}"
            );
        }
    }

    fn durable_profile_fingerprint(&self) -> Vec<String> {
        let mut client = self.bootstrap_client();
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
            "SELECT pg_catalog.md5(pg_catalog.to_jsonb(a)::text) \
               FROM ONLY control.runtime_admission a WHERE a.singleton",
        ]
        .into_iter()
        .map(|query| {
            client
                .query_one(query, &[])
                .expect("TASK105_PROFILE_FINGERPRINT_QUERY")
                .get(0)
        })
        .collect()
    }

    fn assert_v5_writer_v3_bridge(&self) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*) FROM ONLY control.migration_history)=6, \
                    (SELECT current_schema_version=5 AND manifest_sha256=$1 \
                       FROM ONLY control.schema_compatibility WHERE singleton), \
                    (SELECT extension_schema_version=3 AND global_schema_version=5 \
                       FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    NOT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE')",
                &[&V5_MANIFEST_SHA256],
            )
            .expect("TASK105_V5_WRITER_V3_PROFILE");
        for index in 0..4 {
            assert!(row.get::<_, bool>(index), "TASK105_V5_WRITER_V3_{index}");
        }
    }

    fn assert_v6_writer_v3_current(&self) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT \
                    (SELECT pg_catalog.count(*) FROM ONLY control.migration_history)=7, \
                    (SELECT current_schema_version=6 FROM ONLY control.schema_compatibility \
                      WHERE singleton), \
                    (SELECT extension_schema_version=3 AND global_schema_version=6 \
                      FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    pg_catalog.has_function_privilege('lattice_runtime', \
                      'writer_lease.writer_lease_bind_runtime_v3(text,bigint,bytea,text,text,text,text,text)', \
                      'EXECUTE')",
                &[],
            )
            .expect("TASK105_V6_WRITER_V3_PROFILE");
        for index in 0..5 {
            assert!(row.get::<_, bool>(index), "TASK105_V6_WRITER_V3_{index}");
        }
    }

    fn make_v6_writer_bridge_pending(&self) {
        self.bootstrap_client()
            .batch_execute(
                "DELETE FROM ONLY writer_lease.writer_lease_extension_ledger \
                    WHERE ledger_ordinal=3 AND event_kind='REBOUND'; \
                 REVOKE ALL ON ALL FUNCTIONS IN SCHEMA writer_lease FROM lattice_runtime; \
                 REVOKE USAGE ON SCHEMA writer_lease FROM lattice_runtime",
            )
            .expect("TASK105_MAKE_V6_WRITER_BRIDGE_PENDING");
    }

    fn assert_v6_writer_bridge_pending(&self) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT \
                    (SELECT extension_schema_version=3 AND global_schema_version=6 \
                       AND global_manifest_sha256=(SELECT manifest_sha256 \
                         FROM ONLY control.schema_compatibility WHERE singleton) \
                       FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton), \
                    (SELECT pg_catalog.string_agg(ledger_ordinal::text || ':' || event_kind::text, \
                         ',' ORDER BY ledger_ordinal)='1:INSTALLED,2:UPGRADED' \
                       FROM ONLY writer_lease.writer_lease_extension_ledger), \
                    NOT pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    (SELECT pg_catalog.count(*) FILTER (WHERE \
                         pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE'))=0 \
                       FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n \
                         ON n.oid=p.pronamespace WHERE n.nspname='writer_lease')",
                &[],
            )
            .expect("TASK105_V6_WRITER_BRIDGE_PENDING_PROFILE");
        for index in 0..4 {
            assert!(
                row.get::<_, bool>(index),
                "TASK105_V6_WRITER_PENDING_{index}"
            );
        }
    }

    fn introduce_partial_writer_acl(&self) {
        self.bootstrap_client()
            .batch_execute(
                "REVOKE EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v3(\
                    text,bigint,bytea,text,text,text,text,text) FROM lattice_runtime",
            )
            .expect("TASK105_INTRODUCE_PARTIAL_WRITER");
    }

    fn repair_partial_writer_acl(&self) {
        self.bootstrap_client()
            .batch_execute(
                "GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_bind_runtime_v3(\
                    text,bigint,bytea,text,text,text,text,text) TO lattice_runtime",
            )
            .expect("TASK105_REPAIR_PARTIAL_WRITER");
    }

    fn introduce_corrupt_writer_identity(&self) {
        self.bootstrap_client()
            .batch_execute(
                "UPDATE ONLY writer_lease.writer_lease_extension_identity \
                 SET extension_manifest_sha256=repeat('d',64) WHERE singleton",
            )
            .expect("TASK105_INTRODUCE_CORRUPT_WRITER");
    }

    fn repair_corrupt_writer_identity(&self) {
        self.bootstrap_client()
            .batch_execute(&format!(
                "UPDATE ONLY writer_lease.writer_lease_extension_identity \
                 SET extension_manifest_sha256='{WRITER_V3_MANIFEST_SHA256}' WHERE singleton"
            ))
            .expect("TASK105_REPAIR_CORRUPT_WRITER");
    }

    fn introduce_unsupported_history(&self) {
        self.bootstrap_client()
            .batch_execute(
                "INSERT INTO control.migration_history (ordinal,migration_id,migration_path,\
                    byte_length,checksum_sha256,migration_status,transaction_mode,schema_version,\
                    min_reader,max_reader,min_writer,max_writer) VALUES (8,'0008_unsupported_fixture',\
                    'db/migrations/0008_unsupported_fixture.sql',1,repeat('d',64),'EXECUTABLE',\
                    'RUNNER_OWNED',7,7,7,7,7)",
            )
            .expect("TASK105_INTRODUCE_UNSUPPORTED_HISTORY");
    }

    fn repair_unsupported_history(&self) {
        self.bootstrap_client()
            .batch_execute(
                "DELETE FROM ONLY control.migration_history \
                 WHERE ordinal=8 AND migration_id='0008_unsupported_fixture'",
            )
            .expect("TASK105_REPAIR_UNSUPPORTED_HISTORY");
    }

    fn migration_fingerprint(&self) -> (i64, i16, String) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT (SELECT pg_catalog.count(*) FROM ONLY control.migration_history), \
                        current_schema_version, manifest_sha256::text \
                   FROM ONLY control.schema_compatibility WHERE singleton",
                &[],
            )
            .expect("TASK105_MIGRATION_FINGERPRINT");
        (row.get(0), row.get(1), row.get(2))
    }

    fn v6_absent_writer_fingerprint(&self) -> (i64, i16, String, i64, String) {
        let mut client = self.bootstrap_client();
        let row = client
            .query_one(
                "SELECT (SELECT pg_catalog.count(*) FROM ONLY control.migration_history), \
                        current_schema_version, manifest_sha256::text, \
                        (SELECT pg_catalog.count(*) FROM pg_catalog.pg_namespace \
                          WHERE nspname='writer_lease'), \
                        (SELECT pg_catalog.concat_ws(':', admission_mode::text, \
                                    COALESCE(daemon_instance_id, ''), \
                                    COALESCE(daemon_epoch::text, ''), authority_revision::text, \
                                    COALESCE(pg_catalog.encode(observation_digest, 'hex'), ''), \
                                    COALESCE(pg_catalog.encode(authority_head_digest, 'hex'), '')) \
                           FROM ONLY control.runtime_admission WHERE singleton) \
                   FROM ONLY control.schema_compatibility WHERE singleton",
                &[],
            )
            .expect("TASK105_V6_ABSENT_FINGERPRINT");
        (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4))
    }

    fn remove_disposable_writer_profile(&self) {
        self.bootstrap_client()
            .batch_execute(
                "SET ROLE lattice_migrator; DROP SCHEMA writer_lease CASCADE; RESET ROLE;",
            )
            .expect("TASK105_REMOVE_DISPOSABLE_WRITER_PROFILE");
    }
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("TASK105_ENV_MISSING:{name}"))
}

fn run_latticed_admin(config: &LiveConfig, argument: &str, expected_success: bool) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_latticed"))
        .arg(argument)
        .env("LATTICE_TASK019_RUN_ID", &config.run_id)
        .output()
        .expect("TASK105_LATTICED_ADMIN_START");
    assert_eq!(
        output.status.success(),
        expected_success,
        "TASK105_LATTICED_ADMIN_STATUS:{argument}:{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).expect("TASK105_LATTICED_ADMIN_STDERR");
    if expected_success {
        let expected = if argument == "--postgres-initialize" {
            "LATTICE_POSTGRES_INITIALIZE_READY\n"
        } else {
            "LATTICE_POSTGRES_BOOTSTRAP_READY\n"
        };
        assert_eq!(stderr.replace("\r\n", "\n"), expected);
    } else {
        assert!(!stderr.contains("READY"));
        assert!(stderr.len() <= 128);
    }
    stderr
}

fn run_latticed(requests: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_latticed"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("TASK105_LATTICED_START");
    let input = requests
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    child
        .stdin
        .take()
        .expect("TASK105_LATTICED_STDIN")
        .write_all(input.as_bytes())
        .expect("TASK105_LATTICED_WRITE");
    let output = child.wait_with_output().expect("TASK105_LATTICED_WAIT");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("TASK105_LATTICED_FAILED:{stderr}");
    }
    String::from_utf8(output.stdout)
        .expect("TASK105_LATTICED_UTF8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("TASK105_LATTICED_JSON"))
        .collect()
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc":"2.0", "id":1, "method":"initialize",
        "params":{
            "protocolVersion":"2025-11-25", "capabilities":{},
            "clientInfo":{"name":"task105-live","version":"1"}
        }
    })
}

fn checkpoint(
    id: i64,
    checkpoint_id: &str,
    generation: u64,
    state: &str,
    blocker: Value,
    evidence: char,
) -> Value {
    json!({
        "jsonrpc":"2.0", "id":id, "method":"tools/call",
        "params":{
            "name":"lattice_foreman_checkpoint",
            "arguments":{
                "checkpoint_id":checkpoint_id,
                "generation":generation,
                "occurred_at":if generation == 1 { "2026-08-25T00:00:01Z" } else { "2026-08-25T00:00:02Z" },
                "state":state,
                "blocker_ref":blocker,
                "heartbeat_ref":format!("heartbeat:sha256:{}", "a".repeat(64)),
                "evidence_ref":format!("evidence:sha256:{}", evidence.to_string().repeat(64))
            }
        }
    })
}

fn response<'a>(responses: &'a [Value], id: i64) -> &'a Value {
    responses
        .iter()
        .find(|value| value["id"] == id)
        .unwrap_or_else(|| panic!("TASK105_RESPONSE_MISSING:{id}"))
}

#[test]
fn task105_checkpoint_survives_a_fresh_latticed_process_without_migration() {
    let Some(config) = LiveConfig::from_environment() else {
        return;
    };
    println!("TASK105_STAGE_INITIALIZE_ENTER");
    run_latticed_admin(&config, "--postgres-initialize", true);
    config.prepare_v5_writer_v3_bridge();
    config.assert_v5_writer_v3_bridge();
    run_latticed_admin(&config, "--postgres-bootstrap", true);
    config.assert_v6_writer_v3_current();
    let migration_before = config.migration_fingerprint();
    assert_eq!(migration_before.1, 6);
    println!("TASK105_STAGE_V5_WRITER_V3_BOOTSTRAP_PASS");

    config.make_v6_writer_bridge_pending();
    config.assert_v6_writer_bridge_pending();
    let pending = config.durable_profile_fingerprint();
    run_latticed_admin(&config, "--postgres-bootstrap", true);
    config.assert_v6_writer_v3_current();
    assert_ne!(config.durable_profile_fingerprint(), pending);
    println!("TASK105_STAGE_V6_BRIDGE_PENDING_PASS");

    let current = config.durable_profile_fingerprint();
    run_latticed_admin(&config, "--postgres-bootstrap", true);
    assert_eq!(config.durable_profile_fingerprint(), current);
    assert_eq!(config.migration_fingerprint(), migration_before);
    println!("TASK105_STAGE_V6_CURRENT_NOOP_PASS");

    let parent_run_id = required("LATTICE_TASK019_RUN_ID");
    let writer_v2 = config.child_database(0x1000_0000);
    assert_ne!(writer_v2.run_id, config.run_id);
    run_latticed_admin(&writer_v2, "--postgres-initialize", true);
    writer_v2.prepare_v5_writer_v2_current();
    writer_v2.assert_v5_writer_v2_current();
    run_latticed_admin(&writer_v2, "--postgres-bootstrap", true);
    writer_v2.assert_v6_writer_v3_current();
    println!("TASK105_STAGE_V5_WRITER_V2_EXECUTABLE_PASS");

    let writer_absent = config.child_database(0x2000_0000);
    assert_ne!(writer_absent.run_id, config.run_id);
    assert_ne!(writer_absent.run_id, writer_v2.run_id);
    run_latticed_admin(&writer_absent, "--postgres-initialize", true);
    writer_absent.prepare_v5_store_only();
    writer_absent.assert_v5_writer_absent();
    run_latticed_admin(&writer_absent, "--postgres-bootstrap", true);
    writer_absent.assert_v6_writer_v3_current();
    assert_eq!(required("LATTICE_TASK019_RUN_ID"), parent_run_id);
    println!("TASK105_STAGE_V5_WRITER_ABSENT_EXECUTABLE_PASS");

    config.introduce_partial_writer_acl();
    let partial = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), partial);
    config.repair_partial_writer_acl();
    config.assert_v6_writer_v3_current();
    println!("TASK105_STAGE_PARTIAL_FAIL_CLOSED_PASS");

    config.introduce_corrupt_writer_identity();
    let corrupt = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), corrupt);
    config.repair_corrupt_writer_identity();
    config.assert_v6_writer_v3_current();
    println!("TASK105_STAGE_CORRUPT_FAIL_CLOSED_PASS");

    config.introduce_unsupported_history();
    let unsupported = config.durable_profile_fingerprint();
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false).trim(),
        "LATTICED_RUNTIME_POSTGRES_VERIFICATION_REJECTED"
    );
    assert_eq!(config.durable_profile_fingerprint(), unsupported);
    config.repair_unsupported_history();
    config.assert_v6_writer_v3_current();
    println!("TASK105_STAGE_UNSUPPORTED_FAIL_CLOSED_PASS");
    println!("TASK105_STAGE_INITIALIZE_PASS");

    let process_a = run_latticed(&[
        initialize_request(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        checkpoint(2, "task105-checkpoint-1", 1, "ACTIVE", Value::Null, 'b'),
        checkpoint(3, "task105-checkpoint-1", 1, "ACTIVE", Value::Null, 'b'),
        checkpoint(4, "task105-checkpoint-1", 1, "ACTIVE", Value::Null, 'c'),
        checkpoint(5, "task105-checkpoint-gap", 3, "ACTIVE", Value::Null, 'd'),
        checkpoint(
            6,
            "task105-checkpoint-2",
            2,
            "BLOCKED",
            json!("TASK-094"),
            'e',
        ),
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"lattice_runtime_status"}}),
    ]);
    let first = &response(&process_a, 2)["result"]["structuredContent"];
    assert_eq!(response(&process_a, 2)["result"]["isError"], false);
    assert_eq!(first["status"], "RECORDED");
    assert_eq!(
        response(&process_a, 3)["result"]["structuredContent"]["status"],
        "REPLAYED"
    );
    assert_eq!(
        response(&process_a, 3)["result"]["structuredContent"]["ledger_digest"],
        first["ledger_digest"]
    );
    assert_eq!(
        response(&process_a, 4)["result"]["structuredContent"]["code"],
        "FOREMAN_CHECKPOINT_ID_REUSE"
    );
    assert_eq!(
        response(&process_a, 5)["result"]["structuredContent"]["code"],
        "FOREMAN_GENERATION_INVALID"
    );
    assert_eq!(response(&process_a, 6)["result"]["isError"], false);
    let status_a = &response(&process_a, 7)["result"]["structuredContent"]["foreman"];
    let second = &response(&process_a, 6)["result"]["structuredContent"];
    assert_eq!(status_a["ledger_digest"], second["ledger_digest"]);
    assert_eq!(status_a["checkpoint_digest"], second["checkpoint_digest"]);
    assert_eq!(status_a["latest_generation"], 2);
    assert_eq!(status_a["active_count"], 0);
    assert_eq!(status_a["blocked_count"], 1);
    assert_eq!(status_a["completed_count"], 0);
    assert_eq!(status_a["next_action"], "RESOLVE_BLOCKERS");
    assert_eq!(status_a["degraded_code"], Value::Null);
    println!("TASK105_STAGE_PROCESS_A_PASS");

    let process_b = run_latticed(&[
        initialize_request(),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"lattice_runtime_status"}}),
    ]);
    let status_b = &response(&process_b, 2)["result"]["structuredContent"]["foreman"];
    assert_eq!(status_b, status_a);
    assert_eq!(config.migration_fingerprint(), migration_before);
    println!("TASK105_STAGE_FRESH_PROCESS_REPLAY_PASS");

    config.remove_disposable_writer_profile();
    let absent_before = config.v6_absent_writer_fingerprint();
    assert_eq!(absent_before.1, 6);
    assert_eq!(absent_before.3, 0);
    assert_eq!(
        run_latticed_admin(&config, "--postgres-bootstrap", false)
            .replace("\r\n", "\n")
            .trim(),
        "LATTICE_WRITER_LEASE_REJECTED"
    );
    assert_eq!(config.v6_absent_writer_fingerprint(), absent_before);
    println!("TASK105_STAGE_V6_ABSENT_FAIL_CLOSED_PASS");
}
