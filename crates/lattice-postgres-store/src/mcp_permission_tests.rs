// These end-to-end fixtures keep their ordered assertions and owned JSON inputs together.
#![allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
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
#[ignore = "requires a marker-owned isolated PostgreSQL 17 fixture with the current control catalog"]
fn typed_runtime_reads_preserve_results_and_fresh_rejection() {
    let mut f = Fixture::connect();
    let baseline = verify_runtime_store_schema(&mut f.runtime, &f.target).unwrap();
    let before = data_digest(&mut f.migrator);
    // Compare the old and new wire paths, including ordered multi-row catalogs and NULL.
    for sql in [
        RELATION_SIGNATURE_SQL,
        COLUMN_SIGNATURE_SQL,
        CONSTRAINT_SIGNATURE_SQL,
        INDEX_SIGNATURE_SQL,
        FUNCTION_SIGNATURE_SQL,
        TYPE_CATALOG_SIGNATURE_SQL,
        TABLE_ACL_SIGNATURE_SQL,
        FUNCTION_ACL_SIGNATURE_SQL,
        SCHEMA_ACL_SIGNATURE_SQL,
        MANAGED_FOREMAN_FUNCTION_CATALOG_SQL,
        MANAGED_FOREMAN_TABLE_CATALOG_SQL,
        "SELECT v FROM (VALUES (1,NULL::text),(2,'catalog'::text)) t(i,v) ORDER BY i",
    ] {
        let sql = sql.replace("foreman_execution", "control_product");
        let sql = sql.as_str();
        let old: Vec<Option<String>> = f
            .runtime
            .query(sql, &[])
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        let typed: Vec<Option<String>> = f
            .runtime
            .query_typed(sql, &[])
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        assert!(
            !old.is_empty(),
            "catalog equivalence requires populated rows"
        );
        assert_eq!(old, typed);
    }
    for sql in ["SELECT 1 WHERE false", "SELECT generate_series(1,2)"] {
        assert_eq!(
            f.runtime.query_one(sql, &[]).unwrap_err().to_string(),
            f.runtime.query_typed_one(sql, &[]).unwrap_err().to_string()
        );
    }
    assert_eq!(
        f.runtime
            .query_one("SELECT 7::int4", &[])
            .unwrap()
            .get::<_, i32>(0),
        f.runtime
            .query_typed_one("SELECT 7::int4", &[])
            .unwrap()
            .get::<_, i32>(0)
    );
    assert_eq!(
        f.runtime.query("SELECT 1/0", &[]).unwrap_err().code(),
        Some(&SqlState::DIVISION_BY_ZERO)
    );
    assert_eq!(
        f.runtime.query_typed("SELECT 1/0", &[]).unwrap_err().code(),
        Some(&SqlState::DIVISION_BY_ZERO)
    );
    println!("TYPED_READ_ORDER_NULL_CARDINALITY_SQLSTATE_EQUIVALENCE_PASS");

    // Reuse the same runtime connection: every call must see newly committed drift.
    let identity = baseline.database_uuid();
    for (tamper, restore) in [
        ("ALTER FUNCTION control_product.snapshot_v1(text,text[]) VOLATILE".to_owned(),
         "ALTER FUNCTION control_product.snapshot_v1(text,text[]) STABLE".to_owned()),
        ("GRANT SELECT ON control_product.conversation_observations TO lattice_runtime".to_owned(),
         "REVOKE SELECT ON control_product.conversation_observations FROM lattice_runtime".to_owned()),
        ("UPDATE control.database_identity SET database_uuid='11111111-1111-8111-8111-111111111111'::uuid".to_owned(),
         format!("UPDATE control.database_identity SET database_uuid='{identity}'::uuid")),
        ("UPDATE control.schema_compatibility SET current_schema_version=9,min_reader=9,max_reader=9,min_writer=9,max_writer=9".to_owned(),
         "UPDATE control.schema_compatibility SET current_schema_version=8,min_reader=8,max_reader=8,min_writer=8,max_writer=8".to_owned()),
    ] {
        f.migrator.batch_execute(&tamper).unwrap();
        let rejected = verify_runtime_store_schema(&mut f.runtime, &f.target);
        f.migrator.batch_execute(&restore).unwrap();
        assert!(rejected.is_err(), "fresh validation accepted committed drift");
        assert_eq!(verify_runtime_store_schema(&mut f.runtime, &f.target).unwrap(), baseline);
    }
    assert_eq!(data_digest(&mut f.migrator), before);
    println!("TYPED_READ_FRESH_FUNCTION_ACL_IDENTITY_FUTURE_REJECTION_RESTORED_PASS");

    let id = format!("typed-read-{}", std::process::id());
    let task = hex_digest(&Sha256::digest(id.as_bytes()));
    claim(&mut f.migrator, &id, &task);
    dispatch(&mut f.migrator, &id, "typed-turn", "typed-input");
    assert_denied(&mut f.runtime, &task, false);
    question(
        &mut f.migrator,
        &id,
        "QUESTION_REQUESTED",
        "typed-turn",
        "typed-input",
        "typed-question",
        "",
    );
    assert_eq!(
        verify_runtime_store_schema(&mut f.runtime, &f.target).unwrap(),
        baseline
    );
    assert_eq!(
        snapshot(&mut f.runtime, &task)["claims"][0]["pending_questions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    question(
        &mut f.migrator,
        &id,
        "QUESTION_RESOLVED",
        "typed-turn",
        "typed-input",
        "typed-question",
        "decline",
    );
    assert_eq!(
        verify_runtime_store_schema(&mut f.runtime, &f.target).unwrap(),
        baseline
    );
    assert_denied(&mut f.runtime, &task, true);
    assert!(
        snapshot(&mut f.runtime, &task)["claims"][0]["pending_questions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    println!("TYPED_READ_FRESH_PENDING_AND_DURABLE_DENIAL_PASS");
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

#[test]
#[ignore = "requires the marker-owned isolated PostgreSQL 17 fixture with populated control catalogs"]
fn catalog_batch_preserves_digest_order_and_first_error() {
    let mut f = Fixture::connect();
    let kind = PostgresStoreSetupErrorKind::CorruptCatalog;
    let function_sql =
        MANAGED_FOREMAN_FUNCTION_CATALOG_SQL.replace("foreman_execution", "control_product");
    let table_sql =
        MANAGED_FOREMAN_TABLE_CATALOG_SQL.replace("foreman_execution", "control_product");
    let queries = [
        RELATION_SIGNATURE_SQL,
        COLUMN_SIGNATURE_SQL,
        CONSTRAINT_SIGNATURE_SQL,
        INDEX_SIGNATURE_SQL,
        FUNCTION_SIGNATURE_SQL,
        TYPE_CATALOG_SIGNATURE_SQL,
        TABLE_ACL_SIGNATURE_SQL,
        FUNCTION_ACL_SIGNATURE_SQL,
        SCHEMA_ACL_SIGNATURE_SQL,
        function_sql.as_str(),
        table_sql.as_str(),
    ];
    // Independent copy of the pre-batch byte contract, including row count.
    let legacy = |values: &[String]| {
        let mut hash = Sha256::new();
        hash.update(CATALOG_SIGNATURE_DOMAIN);
        hash.update((values.len() as u64).to_be_bytes());
        for value in values {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        hex_digest(&hash.finalize())
    };
    let mut tx = f
        .runtime
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .unwrap();
    harden_transaction(&mut tx).unwrap();
    let batched = tx
        .query_typed_one(&catalog_signature_batch_sql(&queries), &[])
        .unwrap();
    let mut expected = Vec::new();
    for (index, query) in queries.iter().enumerate() {
        let old: Vec<String> = tx
            .query(*query, &[])
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        assert!(!old.is_empty());
        assert_eq!(old, batched.get::<_, Vec<String>>(index));
        let digest = legacy(&old);
        assert_eq!(catalog_signature(&mut tx, query, kind).unwrap(), digest);
        expected.push(digest);
    }
    verify_catalog_signature_batch(
        &mut tx,
        &queries,
        &expected.iter().map(String::as_str).collect::<Vec<_>>(),
        kind,
    )
    .unwrap();
    tx.rollback().unwrap();
    println!("CATALOG_BATCH_POPULATED_ORDER_AND_LEGACY_DIGEST_EQUIVALENCE_PASS");

    for sql in [
        "SELECT ''::text WHERE false",
        "SELECT ''::text",
        "SELECT v FROM (VALUES(2,'繁體中文'::text),(1,''::text)) t(i,v) ORDER BY i",
        "SELECT NULL::text",
    ] {
        let mut tx = f
            .runtime
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .unwrap();
        let old = catalog_signature(&mut tx, sql, kind);
        let expected = old.as_ref().map_or("irrelevant-null", String::as_str);
        let batch = verify_catalog_signature_batch(&mut tx, &[sql], &[expected], kind);
        assert_eq!(old.is_ok(), batch.is_ok());
        if let Err(error) = old {
            assert_eq!(error.kind(), batch.unwrap_err().kind());
        }
        tx.rollback().unwrap();
    }
    println!("CATALOG_BATCH_NULL_EMPTY_ZERO_ONE_MULTI_EQUIVALENCE_PASS");

    // The later denied table read errors in the batch; the old first guard must win.
    let denied = "SELECT claim_id::text FROM control_product.conversation_observations";
    let actual = legacy(&["actual".to_owned()]);
    for (first, expected, expected_kind) in [
        (
            "SELECT 'actual'::text",
            "wrong",
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "SELECT (1/0)::text",
            "unused",
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "SELECT NULL::text",
            "unused",
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ),
        (
            "SELECT 'actual'::text",
            actual.as_str(),
            PostgresStoreSetupErrorKind::PermissionDenied,
        ),
    ] {
        let mut tx = f
            .runtime
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .unwrap();
        let batch_error = tx
            .query_typed_one(&catalog_signature_batch_sql(&[first, denied]), &[])
            .unwrap_err();
        assert_eq!(
            batch_error.code(),
            Some(if first == "SELECT (1/0)::text" {
                &SqlState::DIVISION_BY_ZERO
            } else {
                &SqlState::INSUFFICIENT_PRIVILEGE
            })
        );
        assert!(catalog_batch_may_replay(&batch_error));
        tx.rollback().unwrap();
        let mut tx = f
            .runtime
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .unwrap();
        let observed =
            verify_catalog_signature_batch(&mut tx, &[first, denied], &[expected, "unused"], kind)
                .unwrap_err();
        assert_eq!(observed.kind(), expected_kind);
        if first != "SELECT (1/0)::text" && expected_kind == kind {
            let state = tx.query_one("SELECT current_user::text,current_setting('transaction_isolation'),current_setting('transaction_read_only')", &[]).unwrap();
            assert_eq!(state.get::<_, String>(0), "lattice_runtime");
            assert_eq!(state.get::<_, String>(1), "repeatable read");
            assert_eq!(state.get::<_, String>(2), "on");
        }
        tx.rollback().unwrap();
    }
    println!("CATALOG_BATCH_FIRST_MISMATCH_AND_SQL_ERROR_PRIORITY_PASS");

    let mut tx = f
        .runtime
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .unwrap();
    tx.batch_execute("SET LOCAL statement_timeout='30ms'")
        .unwrap();
    let slow = "SELECT 'slow'::text FROM pg_sleep(0.2)";
    assert!(
        verify_catalog_signature_batch(
            &mut tx,
            &["SELECT 'first'::text", slow],
            &["wrong", "unused"],
            kind
        )
        .is_err()
    );
    // A terminal timeout must leave the transaction failed, never recover/replay.
    assert_eq!(
        tx.query_one("SELECT 1", &[]).unwrap_err().code(),
        Some(&SqlState::IN_FAILED_SQL_TRANSACTION)
    );
    tx.rollback().unwrap();
    let mut tx = f
        .runtime
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .unwrap();
    tx.batch_execute("SET LOCAL statement_timeout='30ms'")
        .unwrap();
    let timeout = tx.query_typed_one(slow, &[]).unwrap_err();
    assert_eq!(timeout.code(), Some(&SqlState::QUERY_CANCELED));
    assert!(!catalog_batch_may_replay(&timeout));
    tx.rollback().unwrap();
    println!("CATALOG_BATCH_TIMEOUT_TERMINATES_WITHOUT_REPLAY_PASS");

    let mut lock = f.migrator.transaction().unwrap();
    lock.batch_execute("LOCK TABLE control.database_identity IN ACCESS EXCLUSIVE MODE")
        .unwrap();
    let mut tx = f
        .runtime
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .unwrap();
    tx.batch_execute("SET LOCAL lock_timeout='30ms'").unwrap();
    assert!(
        verify_catalog_signature_batch(
            &mut tx,
            &[
                "SELECT 'first'::text",
                "SELECT database_uuid::text FROM control.database_identity"
            ],
            &["wrong", "unused"],
            kind
        )
        .is_err()
    );
    assert_eq!(
        tx.query_one("SELECT 1", &[]).unwrap_err().code(),
        Some(&SqlState::IN_FAILED_SQL_TRANSACTION)
    );
    tx.rollback().unwrap();
    let mut tx = f
        .runtime
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .unwrap();
    tx.batch_execute("SET LOCAL lock_timeout='30ms'").unwrap();
    let timeout = tx
        .query_typed_one(
            "SELECT database_uuid::text FROM control.database_identity",
            &[],
        )
        .unwrap_err();
    assert_eq!(timeout.code(), Some(&SqlState::LOCK_NOT_AVAILABLE));
    assert!(!catalog_batch_may_replay(&timeout));
    tx.rollback().unwrap();
    lock.rollback().unwrap();
    println!("CATALOG_BATCH_LOCK_TIMEOUT_TERMINATES_WITHOUT_REPLAY_PASS");
}
