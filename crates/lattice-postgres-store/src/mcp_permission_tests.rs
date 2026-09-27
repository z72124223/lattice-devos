//! Opt-in proof against an independently bootstrapped, disposable Store v8 database.
use super::*;
use postgres::{Config, NoTls};
use serde_json::{Value, json};

struct Fixture {
    migrator: Client,
    runtime: Client,
    target: MigrationTarget,
}

fn environment(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing isolated fixture field {key}"))
}

impl Fixture {
    fn connect() -> Self {
        assert_eq!(environment("LATTICE_MCP_PERMISSION_LIVE"), "1");
        let port: u16 = environment("LATTICE_MCP_PERMISSION_PORT").parse().unwrap();
        assert!(![0, 5432, 58743].contains(&port));
        let run = environment("LATTICE_MCP_PERMISSION_RUN_ID");
        let database = environment("LATTICE_MCP_PERMISSION_DATABASE");
        let target = MigrationTarget::new(database, run).unwrap();
        assert_eq!(
            target.database_name(),
            format!("lattice_task019_{}_base", &target.run_id()[..8])
        );
        let password = environment("LATTICE_MCP_PERMISSION_PASSWORD");
        let connect = |role: DatabaseRole| {
            let mut client = Config::new()
                .host("127.0.0.1")
                .port(port)
                .user(role.login_role())
                .password(&password)
                .dbname(target.database_name())
                .application_name(REQUIRED_APPLICATION_NAME)
                .connect(NoTls)
                .expect("isolated connection");
            client
                .batch_execute(&format!("SET ROLE {}", role.as_str()))
                .unwrap();
            let marker: Option<String> = client.query_one(
                "SELECT shobj_description(oid,'pg_database') FROM pg_database WHERE datname=current_database()", &[]
            ).unwrap().get(0);
            assert_eq!(marker.as_deref(), Some(target.database_comment().as_str()));
            client
        };
        let migrator = connect(DatabaseRole::Migrator);
        let runtime = connect(DatabaseRole::Runtime);
        Self {
            migrator,
            runtime,
            target,
        }
    }
}

fn function_digest<C: GenericClient>(client: &mut C) -> String {
    managed_foreman_catalog_digest(
        client,
        &MANAGED_FOREMAN_FUNCTION_CATALOG_SQL.replace("foreman_execution", "control_product"),
        b"LATTICE_CONTROL_PRODUCT_FUNCTION_CATALOG_V1\0",
    )
    .unwrap()
}

fn data_digest(client: &mut Client) -> String {
    let tables = client.query("SELECT n.nspname,c.relname FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind='r' AND n.nspname NOT LIKE 'pg_%' AND n.nspname<>'information_schema' ORDER BY 1,2", &[]).unwrap();
    let mut digest = Sha256::new();
    for table in tables {
        let schema: String = table.get(0);
        let name: String = table.get(1);
        assert!(
            schema
                .bytes()
                .chain(name.bytes())
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        );
        digest.update(format!("{schema}.{name}\0").as_bytes());
        for row in client.query(&format!("SELECT row_to_json(t)::text FROM ONLY {schema}.{name} t ORDER BY row_to_json(t)::text COLLATE \"C\""), &[]).unwrap() {
            let bytes: String = row.get(0);
            digest.update((bytes.len() as u64).to_be_bytes());
            digest.update(bytes.as_bytes());
        }
    }
    hex_digest(&digest.finalize())
}

fn table_digest<C: GenericClient>(client: &mut C) -> String {
    managed_foreman_catalog_digest(
        client,
        &MANAGED_FOREMAN_TABLE_CATALOG_SQL.replace("foreman_execution", "control_product"),
        b"LATTICE_CONTROL_PRODUCT_TABLE_CATALOG_V1\0",
    )
    .unwrap()
}

#[test]
#[ignore = "requires a marker-owned isolated PostgreSQL 17 fixture with the known old graph catalog"]
fn measure_mcp_permission_catalog_without_installing() {
    let mut fixture = Fixture::connect();
    verify_postgres_schema(
        &mut fixture.migrator,
        &fixture.target,
        DatabaseRole::Migrator,
    )
    .unwrap();
    assert_eq!(
        function_digest(&mut fixture.migrator),
        GRAPH_USAGE_FUNCTION_CATALOG_SHA256
    );
    let before = data_digest(&mut fixture.migrator);
    let mut transaction = fixture.migrator.transaction().unwrap();
    transaction.batch_execute(MCP_PERMISSION_SQL).unwrap();
    println!(
        "MCP_PERMISSION_FUNCTION_CATALOG_SHA256={}",
        function_digest(&mut transaction)
    );
    transaction.rollback().unwrap();
    assert_eq!(
        function_digest(&mut fixture.migrator),
        GRAPH_USAGE_FUNCTION_CATALOG_SHA256
    );
    assert_eq!(data_digest(&mut fixture.migrator), before);
    println!("MCP_PERMISSION_MEASUREMENT_ROLLBACK_PASS");
}

fn claim(client: &mut Client, id: &str, task: &str) {
    client.execute("INSERT INTO control_product.conversation_claims(claim_id,task_ref,project_id,phase,request_digest,prompt,model,worktree_path) VALUES($1,$2,'isolated-permission','EXECUTION',$2,'isolated SQL fixture','fixture','fixture')", &[&id,&task]).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn observation(
    client: &mut Client,
    claim: &str,
    kind: &str,
    thread: &str,
    turn: &str,
    input: &str,
    approval: &str,
    payload: Value,
) {
    client.execute("INSERT INTO control_product.conversation_observations(claim_id,sequence,request_id,request_digest,kind,thread_id,turn_id,input_id,approval_id,summary,payload,execution_sequence) SELECT $1::text,COALESCE(max(sequence),0)+1,$1::text||':'||(COALESCE(max(sequence),0)+1)::text,repeat('a',64),$2,$3,$4,$5,$6,'isolated fixture',$7,1 FROM control_product.conversation_observations WHERE claim_id=$1", &[&claim,&kind,&thread,&turn,&input,&approval,&payload]).unwrap();
}

fn dispatch(client: &mut Client, claim: &str, turn: &str, input: &str) {
    observation(
        client,
        claim,
        "DISPATCH_STARTED",
        "thread",
        turn,
        input,
        "",
        json!({}),
    );
    observation(
        client,
        claim,
        "TURN_BOUND",
        "thread",
        turn,
        input,
        "",
        json!({}),
    );
}

fn question(
    client: &mut Client,
    claim: &str,
    kind: &str,
    turn: &str,
    input: &str,
    id: &str,
    action: &str,
) {
    // Intentionally false payload binding: the real relational columns alone bind the denial.
    observation(
        client,
        claim,
        kind,
        "thread",
        turn,
        input,
        id,
        json!({"method":"mcpServer/elicitation/request", "binding":{"claimId":"wrong-claim","turnId":"wrong-turn","inputId":"wrong-input"}, "response":{"action":action}}),
    );
}

fn snapshot(client: &mut Client, task: &str) -> Value {
    client
        .query_one(
            "SELECT control_product.snapshot_v1('isolated-permission',ARRAY[$1::text])",
            &[&task],
        )
        .unwrap()
        .get(0)
}

fn assert_denied(client: &mut Client, task: &str, denied: bool) {
    let value = snapshot(client, task);
    assert_eq!(
        value["claims"][0]["mcp_permission"],
        json!({"version":1,"denied":denied})
    );
}

#[test]
#[ignore = "requires a fresh marker-owned isolated PostgreSQL 17 fixture with the known old graph catalog"]
fn durable_mcp_permission_upgrade_and_relational_projection() {
    let mut f = Fixture::connect();
    let task = "a".repeat(64);
    let other_task = "b".repeat(64);
    assert_eq!(
        function_digest(&mut f.migrator),
        GRAPH_USAGE_FUNCTION_CATALOG_SHA256
    );
    verify_postgres_schema(&mut f.migrator, &f.target, DatabaseRole::Migrator).unwrap();
    claim(&mut f.migrator, "permission-claim", &task);
    claim(&mut f.migrator, "other-claim", &other_task);
    dispatch(&mut f.migrator, "permission-claim", "turn-1", "input-1");
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_REQUESTED",
        "turn-1",
        "input-1",
        "denied-1",
        "",
    );
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_RESOLVED",
        "turn-1",
        "input-1",
        "denied-1",
        "decline",
    );
    for _ in 0..120 {
        observation(
            &mut f.migrator,
            "permission-claim",
            "PROGRESS",
            "thread",
            "turn-1",
            "input-1",
            "",
            json!({}),
        );
    }
    let before = data_digest(&mut f.migrator);
    let old_profile = verify_optional_control_product_extension(&mut f.migrator)
        .unwrap()
        .unwrap();
    assert!(old_profile.graph_usage && !old_profile.mcp_permission);
    assert!(
        snapshot(&mut f.runtime, &task)["claims"][0]
            .get("mcp_permission")
            .is_none()
    );
    apply_control_product_extension(&mut f.migrator, &f.target).unwrap();
    let profile = verify_optional_control_product_extension(&mut f.migrator)
        .unwrap()
        .unwrap();
    assert!(profile.mcp_permission);
    assert_eq!(profile.function_oids, old_profile.function_oids);
    assert_eq!(profile.relation_oids, old_profile.relation_oids);
    assert_eq!(data_digest(&mut f.migrator), before);
    let value = snapshot(&mut f.runtime, &task);
    assert_eq!(value["observations"].as_array().unwrap().len(), 100);
    assert!(
        value["observations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["kind"] == "PROGRESS")
    );
    assert_denied(&mut f.runtime, &task, true);
    assert_denied(&mut f.runtime, &other_task, false);
    apply_control_product_extension(&mut f.migrator, &f.target).unwrap();
    assert_eq!(
        function_digest(&mut f.migrator),
        MCP_PERMISSION_FUNCTION_CATALOG_SHA256
    );
    assert_eq!(
        table_digest(&mut f.migrator),
        GRAPH_USAGE_TABLE_CATALOG_SHA256
    );
    let repeated = verify_optional_control_product_extension(&mut f.migrator)
        .unwrap()
        .unwrap();
    assert_eq!(repeated.function_oids, profile.function_oids);
    assert_eq!(repeated.relation_oids, profile.relation_oids);
    assert_eq!(data_digest(&mut f.migrator), before);
    println!("MCP_PERMISSION_UPGRADE_DATA_OID_ACL_IDENTITY_IDEMPOTENCY_PASS");

    dispatch(&mut f.migrator, "permission-claim", "turn-2", "input-2");
    assert_denied(&mut f.runtime, &task, false);
    // Old turn/input, mismatched resolution columns, and a different claim cannot poison this turn.
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_REQUESTED",
        "turn-1",
        "input-1",
        "stale",
        "",
    );
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_RESOLVED",
        "turn-1",
        "input-1",
        "stale",
        "cancel",
    );
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_REQUESTED",
        "turn-2",
        "input-2",
        "mismatch",
        "",
    );
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_RESOLVED",
        "turn-2",
        "wrong-input",
        "mismatch",
        "decline",
    );
    question(
        &mut f.migrator,
        "other-claim",
        "QUESTION_RESOLVED",
        "turn-2",
        "input-2",
        "mismatch",
        "decline",
    );
    assert_denied(&mut f.runtime, &task, false);
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_REQUESTED",
        "turn-2",
        "input-2",
        "accepted",
        "",
    );
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_RESOLVED",
        "turn-2",
        "input-2",
        "accepted",
        "accept",
    );
    assert_denied(&mut f.runtime, &task, false);
    question(
        &mut f.migrator,
        "permission-claim",
        "QUESTION_RESOLVED",
        "turn-2",
        "input-2",
        "mismatch",
        "cancel",
    );
    assert_denied(&mut f.runtime, &task, true);
    // Even when the turn id is reused, a new dispatch starts a new input scope.
    dispatch(&mut f.migrator, "permission-claim", "turn-2", "input-3");
    assert_denied(&mut f.runtime, &task, false);
    println!("MCP_PERMISSION_FULL_HISTORY_RELATIONAL_CURRENT_SCOPE_PASS");

    let identity: String = f
        .migrator
        .query_one(
            "SELECT sql_sha256::text FROM control_product.extension_identity",
            &[],
        )
        .unwrap()
        .get(0);
    for (tamper, restore) in [
        (
            "ALTER FUNCTION control_product.snapshot_v1(text,text[]) VOLATILE".to_owned(),
            "ALTER FUNCTION control_product.snapshot_v1(text,text[]) STABLE".to_owned(),
        ),
        (
            "GRANT SELECT ON control_product.conversation_observations TO lattice_runtime"
                .to_owned(),
            "REVOKE SELECT ON control_product.conversation_observations FROM lattice_runtime"
                .to_owned(),
        ),
        (
            "UPDATE control_product.extension_identity SET sql_sha256=repeat('0',64)".to_owned(),
            format!("UPDATE control_product.extension_identity SET sql_sha256='{identity}'"),
        ),
    ] {
        f.migrator.batch_execute(&tamper).unwrap();
        let tampered_functions = function_digest(&mut f.migrator);
        let tampered_tables = table_digest(&mut f.migrator);
        let tampered_data = data_digest(&mut f.migrator);
        assert!(apply_control_product_extension(&mut f.migrator, &f.target).is_err());
        assert_eq!(function_digest(&mut f.migrator), tampered_functions);
        assert_eq!(table_digest(&mut f.migrator), tampered_tables);
        assert_eq!(data_digest(&mut f.migrator), tampered_data);
        f.migrator.batch_execute(&restore).unwrap();
        verify_postgres_schema(&mut f.migrator, &f.target, DatabaseRole::Migrator).unwrap();
    }
    println!("MCP_PERMISSION_UNKNOWN_FUNCTION_ACL_IDENTITY_REJECTED_PASS");
    println!(
        "MCP_PERMISSION_FINAL_FUNCTION_CATALOG_SHA256={}",
        function_digest(&mut f.migrator)
    );
    println!(
        "MCP_PERMISSION_FINAL_DATA_SHA256={}",
        data_digest(&mut f.migrator)
    );
}
