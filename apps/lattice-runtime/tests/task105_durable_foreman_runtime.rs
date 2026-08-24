//! Marker-owned PostgreSQL acceptance for TASK-105.

use std::env;
use std::io::Write;
use std::process::{Command, Stdio};

use lattice_runtime::composition::{
    bootstrap_postgres_extensions_from_environment, initialize_runtime_postgres_from_environment,
};
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls};
use serde_json::{Value, json};

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
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("TASK105_ENV_MISSING:{name}"))
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
    initialize_runtime_postgres_from_environment().expect("TASK105_INITIALIZE");
    bootstrap_postgres_extensions_from_environment().expect("TASK105_BOOTSTRAP");
    let migration_before = config.migration_fingerprint();
    assert_eq!(migration_before.1, 6);
    bootstrap_postgres_extensions_from_environment().expect("TASK105_V6_CURRENT_RETRY");
    assert_eq!(config.migration_fingerprint(), migration_before);
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
        "FOREMAN_GENERATION_GAP"
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
}
