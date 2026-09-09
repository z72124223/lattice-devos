//! Fixed-role lifecycle adapter for a dedicated database on the managed service.
//! It never modifies the existing Store catalog or cluster roles.
use postgres::{Client, Config, GenericClient, IsolationLevel, NoTls};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const BOT_LIFECYCLE_SQL: &str = include_str!("../../../db/extensions/bot-lifecycle/v1.sql");
pub const BOT_LIFECYCLE_V2_SQL: &str = include_str!("../../../db/extensions/bot-lifecycle/v2.sql");
pub const BOT_LIFECYCLE_ARCHIVE_SQL: &str =
    include_str!("../../../db/extensions/bot-lifecycle/archive-reconcile-v2.sql");
type Result<T> = std::result::Result<T, &'static str>;

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn database(run_id: &str) -> Result<String> {
    if run_id.len() != 32
        || !run_id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("BOT_LIFECYCLE_CONFIGURATION_REJECTED");
    }
    Ok(format!("lattice_bot_lifecycle_{run_id}"))
}
fn connect(port: u16, run_id: &str, password: &str, principal: &str) -> Result<Client> {
    if port == 0 || password.is_empty() {
        return Err("BOT_LIFECYCLE_CONFIGURATION_REJECTED");
    }
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .password(password)
        .connect_timeout(Duration::from_secs(5));
    match principal {
        "bootstrap" => {
            config.user("runtime_bootstrap").dbname("postgres");
        }
        "migrator" => {
            config
                .user("lattice_migrator_login")
                .dbname(&database(run_id)?)
                .options("-c role=lattice_migrator -c search_path=pg_catalog");
        }
        "runtime" => {
            config
                .user("lattice_runtime_login")
                .dbname(&database(run_id)?)
                .options("-c role=lattice_runtime -c search_path=pg_catalog");
        }
        _ => return Err("BOT_LIFECYCLE_CONFIGURATION_REJECTED"),
    }
    config
        .connect_with_startup_timeout(NoTls, Duration::from_secs(5))
        .map_err(|_| "BOT_LIFECYCLE_DATABASE_UNAVAILABLE")?
        .ok_or("BOT_LIFECYCLE_DATABASE_UNAVAILABLE")
}

fn error(e: postgres::Error) -> &'static str {
    if let Some(d) = e.as_db_error() {
        if matches!(d.code().code(), "40001" | "40P01") {
            return "BOT_LIFECYCLE_REVISION_CONFLICT";
        }
        match d.message() {
            "BOT_LIFECYCLE_INPUT_REJECTED" => "BOT_LIFECYCLE_INPUT_REJECTED",
            "BOT_LIFECYCLE_NATIVE_EVIDENCE_REJECTED" => "BOT_LIFECYCLE_NATIVE_EVIDENCE_REJECTED",
            "BOT_LIFECYCLE_NATIVE_TARGET_MISMATCH" => "BOT_LIFECYCLE_NATIVE_TARGET_MISMATCH",
            "BOT_LIFECYCLE_WORK_SET_MISMATCH" => "BOT_LIFECYCLE_WORK_SET_MISMATCH",
            "BOT_LIFECYCLE_ROLE_REJECTED" => "BOT_LIFECYCLE_ROLE_REJECTED",
            "BOT_LIFECYCLE_IDEMPOTENCY_CONFLICT" => "BOT_LIFECYCLE_IDEMPOTENCY_CONFLICT",
            "BOT_LIFECYCLE_REVISION_CONFLICT" => "BOT_LIFECYCLE_REVISION_CONFLICT",
            "BOT_LIFECYCLE_ROLE_MISSING" => "BOT_LIFECYCLE_ROLE_MISSING",
            "BOT_LIFECYCLE_STALE_OWNER" => "BOT_LIFECYCLE_STALE_OWNER",
            "BOT_LIFECYCLE_PHASE_REJECTED" => "BOT_LIFECYCLE_PHASE_REJECTED",
            "BOT_LIFECYCLE_STEP_ALREADY_RESERVED" => "BOT_LIFECYCLE_STEP_ALREADY_RESERVED",
            "BOT_LIFECYCLE_STEP_NOT_RESERVED" => "BOT_LIFECYCLE_STEP_NOT_RESERVED",
            "BOT_LIFECYCLE_ARCHIVE_NOT_READY" => "BOT_LIFECYCLE_ARCHIVE_NOT_READY",
            "BOT_LIFECYCLE_ACK_REJECTED" => "BOT_LIFECYCLE_ACK_REJECTED",
            "BOT_LIFECYCLE_UNVERIFIED_SWITCH" => "BOT_LIFECYCLE_UNVERIFIED_SWITCH",
            "BOT_LIFECYCLE_MIGRATION_INCOMPLETE" => "BOT_LIFECYCLE_MIGRATION_INCOMPLETE",
            "BOT_LIFECYCLE_ABORT_UNSAFE" => "BOT_LIFECYCLE_ABORT_UNSAFE",
            "BOT_LIFECYCLE_RULES_MISMATCH" => "BOT_LIFECYCLE_RULES_MISMATCH",
            "BOT_LIFECYCLE_LIMIT" => "BOT_LIFECYCLE_LIMIT",
            "BOT_LIFECYCLE_CONTRACT_MIGRATION_REJECTED" => {
                "BOT_LIFECYCLE_CONTRACT_MIGRATION_REJECTED"
            }
            "BOT_LIFECYCLE_ACTOR_REJECTED" => "BOT_LIFECYCLE_ACTOR_REJECTED",
            "BOT_LIFECYCLE_EXECUTOR_GRANT_REJECTED" => "BOT_LIFECYCLE_EXECUTOR_GRANT_REJECTED",
            "BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED" => "BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED",
            "BOT_LIFECYCLE_ARCHIVE_RECONCILE_REJECTED" => {
                "BOT_LIFECYCLE_ARCHIVE_RECONCILE_REJECTED"
            }
            _ => "BOT_LIFECYCLE_DATABASE_REJECTED",
        }
    } else {
        "BOT_LIFECYCLE_DATABASE_UNAVAILABLE"
    }
}

// Compare exact stored function bodies to the embedded installer. Runtime users
// have EXECUTE on only three functions and no direct relation privileges.
fn verify(client: &mut impl GenericClient, run_id: &str) -> Result<u8> {
    let identity: Value = client
        .query_one("SELECT bot_lifecycle.identity_read_v1()", &[])
        .map_err(error)?
        .get(0);
    if identity
        != json!({"database":database(run_id)?,"run_id":run_id,"sql_sha256":digest(BOT_LIFECYCLE_SQL.as_bytes())})
    {
        return Err("BOT_LIFECYCLE_SCHEMA_REJECTED");
    }
    let rows = client.query("SELECT p.proname,p.prosrc,p.prosecdef,pg_get_userbyid(p.proowner),p.proconfig, \
        has_function_privilege('lattice_runtime',p.oid,'EXECUTE'), \
        EXISTS(SELECT 1 FROM aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a WHERE a.grantee=0 AND a.privilege_type='EXECUTE') \
        FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='bot_lifecycle'", &[]).map_err(error)?;
    let version = if rows.iter().any(|r| r.get::<_, String>(0) == "apply_v2") {
        2
    } else {
        1
    };
    let archive_extension = rows
        .iter()
        .any(|r| r.get::<_, String>(0) == "reconcile_archive_v2");
    if rows.len()
        != if version == 2 {
            11 + usize::from(archive_extension)
        } else {
            7
        }
    {
        return Err("BOT_LIFECYCLE_SCHEMA_REJECTED");
    }
    for row in rows {
        let name: String = row.get(0);
        let source: String = row.get(1);
        let marker = format!("CREATE FUNCTION bot_lifecycle.{name}(");
        let sql = if name == "reconcile_archive_v2" {
            BOT_LIFECYCLE_ARCHIVE_SQL
        } else if name.ends_with("_v2") {
            BOT_LIFECYCLE_V2_SQL
        } else {
            BOT_LIFECYCLE_SQL
        };
        let section = sql
            .split_once(&marker)
            .ok_or("BOT_LIFECYCLE_SCHEMA_REJECTED")?
            .1;
        let expected = section
            .split_once("AS $$")
            .ok_or("BOT_LIFECYCLE_SCHEMA_REJECTED")?
            .1
            .split_once("$$;")
            .ok_or("BOT_LIFECYCLE_SCHEMA_REJECTED")?
            .0;
        let definer = matches!(
            name.as_str(),
            "identity_read_v1"
                | "read_v1"
                | "apply_v1"
                | "apply_v2"
                | "migrate_v2"
                | "reconcile_archive_v2"
        );
        let public_api = matches!(name.as_str(), "identity_read_v1" | "read_v1" | "apply_v2")
            || (version == 1 && name == "apply_v1");
        let options: Vec<String> = row.get::<_, Option<Vec<String>>>(4).unwrap_or_default();
        if source != expected
            || row.get::<_, String>(3) != "lattice_migrator"
            || row.get::<_, bool>(2) != definer
            || row.get::<_, bool>(5) != public_api
            || row.get::<_, bool>(6)
            || !options.iter().any(|o| o == "search_path=pg_catalog")
        {
            return Err("BOT_LIFECYCLE_SCHEMA_REJECTED");
        }
    }
    let relations=client.query("SELECT c.relname,pg_get_userbyid(c.relowner), \
        has_table_privilege('lattice_runtime',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER'), \
        EXISTS(SELECT 1 FROM aclexplode(COALESCE(c.relacl,acldefault('r',c.relowner))) a WHERE a.grantee=0) \
        FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='bot_lifecycle' AND c.relkind='r' ORDER BY c.relname",&[]).map_err(error)?;
    if relations.len() != 3 {
        return Err("BOT_LIFECYCLE_SCHEMA_REJECTED");
    }
    for (row, expected) in relations.iter().zip(["events", "identity", "roles"]) {
        if row.get::<_, String>(0) != expected
            || row.get::<_, String>(1) != "lattice_migrator"
            || row.get::<_, bool>(2)
            || row.get::<_, bool>(3)
        {
            return Err("BOT_LIFECYCLE_SCHEMA_REJECTED");
        }
    }
    Ok(version)
}

/// Explicit versioned migration; schema extension and control enrollment share
/// one serializable transaction. Never called by runtime reads or installation.
pub fn migrate_bot_lifecycle(
    port: u16,
    run_id: &str,
    password: &str,
    request: &Value,
) -> Result<Value> {
    let mut client = connect(port, run_id, password, "migrator")?;
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(error)?;
    let version = verify(&mut tx, run_id)?;
    if version == 1 {
        tx.batch_execute(BOT_LIFECYCLE_V2_SQL).map_err(error)?;
    }
    verify(&mut tx, run_id)?;
    let mut value: Value = tx
        .query_one("SELECT bot_lifecycle.migrate_v2($1)", &[request])
        .map_err(error)?
        .get(0);
    tx.commit().map_err(|_| "BOT_LIFECYCLE_OUTCOME_UNKNOWN")?;
    value["v1_sql_sha256"] = json!(digest(BOT_LIFECYCLE_SQL.as_bytes()));
    value["v2_sql_sha256"] = json!(digest(BOT_LIFECYCLE_V2_SQL.as_bytes()));
    Ok(value)
}

/// Install the additive recovery function and reconcile one exact archived
/// boundary atomically. Failure rolls back installation as well as state.
pub fn reconcile_bot_lifecycle_archive(
    port: u16,
    run_id: &str,
    password: &str,
    request: &Value,
) -> Result<Value> {
    let mut client = connect(port, run_id, password, "migrator")?;
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(error)?;
    if verify(&mut tx, run_id)? != 2 {
        return Err("BOT_LIFECYCLE_ARCHIVE_RECONCILE_REJECTED");
    }
    // Serialize catalog installation independently of the per-role state lock.
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended('bot_lifecycle/archive-reconcile-v2',0))",
        &[],
    )
    .map_err(error)?;
    let present: bool = tx
        .query_one(
            "SELECT to_regprocedure('bot_lifecycle.reconcile_archive_v2(jsonb)') IS NOT NULL",
            &[],
        )
        .map_err(error)?
        .get(0);
    if !present {
        tx.batch_execute(BOT_LIFECYCLE_ARCHIVE_SQL).map_err(error)?;
    }
    verify(&mut tx, run_id)?;
    let mut value: Value = tx
        .query_one("SELECT bot_lifecycle.reconcile_archive_v2($1)", &[request])
        .map_err(error)?
        .get(0);
    tx.commit().map_err(|_| "BOT_LIFECYCLE_OUTCOME_UNKNOWN")?;
    value["archive_sql_sha256"] = json!(digest(BOT_LIFECYCLE_ARCHIVE_SQL.as_bytes()));
    Ok(value)
}

/// Explicit installer. Only creates the exact new database; existing data is
/// verified and retained. No original Store objects, roles or passwords change.
pub fn install_bot_lifecycle(port: u16, run_id: &str, password: &str) -> Result<Value> {
    let name = database(run_id)?;
    let mut bootstrap = connect(port, run_id, password, "bootstrap")?;
    let shared_store: bool = bootstrap
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname LIKE 'lattice_task019_%')",
            &[],
        )
        .map_err(error)?
        .get(0);
    if shared_store {
        return Err("BOT_LIFECYCLE_ISOLATED_CLUSTER_REQUIRED");
    }
    let roles=bootstrap.query("SELECT rolname,rolsuper,rolcreatedb,rolcreaterole,rolbypassrls FROM pg_roles WHERE rolname IN('lattice_runtime','lattice_runtime_login','lattice_migrator','lattice_migrator_login')",&[]).map_err(error)?;
    if roles.len() != 4 || roles.iter().any(|r| (1..=4).any(|i| r.get::<_, bool>(i))) {
        return Err("BOT_LIFECYCLE_ROLE_REJECTED");
    }
    let found = bootstrap
        .query(
            "SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname=$1",
            &[&name],
        )
        .map_err(error)?;
    let created = found.is_empty();
    if created {
        // name is derived solely from a validated 32-byte lowercase hex run ID.
        bootstrap
            .batch_execute(&format!(
                "CREATE DATABASE {name} OWNER lattice_migrator TEMPLATE template0;"
            ))
            .map_err(error)?;
        bootstrap.batch_execute(&format!("REVOKE ALL ON DATABASE {name} FROM PUBLIC; GRANT CONNECT ON DATABASE {name} TO lattice_runtime,lattice_runtime_login,lattice_migrator,lattice_migrator_login;")).map_err(error)?;
    } else if found[0].get::<_, String>(0) != "lattice_migrator" {
        return Err("BOT_LIFECYCLE_DATABASE_IDENTITY_REJECTED");
    }
    let mut migrator = connect(port, run_id, password, "migrator")?;
    let present: bool = migrator
        .query_one("SELECT to_regnamespace('bot_lifecycle') IS NOT NULL", &[])
        .map_err(error)?
        .get(0);
    if !present {
        if !created {
            return Err("BOT_LIFECYCLE_PARTIAL_INSTALL_REVIEW_REQUIRED");
        }
        let mut tx = migrator.transaction().map_err(error)?;
        tx.batch_execute(BOT_LIFECYCLE_SQL).map_err(error)?;
        tx.execute(
            "INSERT INTO bot_lifecycle.identity VALUES(true,$1,$2)",
            &[&run_id, &digest(BOT_LIFECYCLE_SQL.as_bytes())],
        )
        .map_err(error)?;
        tx.commit().map_err(|_| "BOT_LIFECYCLE_OUTCOME_UNKNOWN")?;
    }
    verify(&mut migrator, run_id)?;
    let mut runtime = connect(port, run_id, password, "runtime")?;
    let version = verify(&mut runtime, run_id)?;
    Ok(
        json!({"schemaVersion":version,"schema_version":format!("lattice.bot-lifecycle.v{version}"),"status":if created {"INSTALLED"}else{"VERIFIED"},"database":name,"host":"127.0.0.1","port":port,"runId":run_id}),
    )
}

pub fn execute_bot_lifecycle(
    port: u16,
    run_id: &str,
    password: &str,
    request: &Value,
) -> Result<Value> {
    let object = request.as_object().ok_or("BOT_LIFECYCLE_INPUT_REJECTED")?;
    let text = |k: &str| -> Result<&str> {
        object
            .get(k)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or("BOT_LIFECYCLE_INPUT_REJECTED")
    };
    let action = text("action")?;
    for key in ["project_id", "role_id"] {
        let v = text(key)?;
        if v.len() > 64
            || !v
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("BOT_LIFECYCLE_INPUT_REJECTED");
        }
    }
    let mut client = connect(port, run_id, password, "runtime")?;
    let version = verify(&mut client, run_id)?;
    if matches!(action, "read" | "assert-owner" | "assert-handoff-owner") {
        let mut expected = if action == "read" {
            vec!["action", "project_id", "role_id"]
        } else {
            vec![
                "action",
                "project_id",
                "role_id",
                "expected_generation",
                "owner_thread_id",
                "owner_host_id",
            ]
        };
        if action == "assert-handoff-owner" {
            expected.push("handoff_id");
        }
        if object.len() != expected.len() || expected.iter().any(|k| !object.contains_key(*k)) {
            return Err("BOT_LIFECYCLE_INPUT_REJECTED");
        }
        let mut tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(error)?;
        let mut value: Value = tx
            .query_one(
                "SELECT bot_lifecycle.read_v1($1,$2)",
                &[&text("project_id")?, &text("role_id")?],
            )
            .map_err(error)?
            .get(0);
        if action != "read" {
            let state = &value["current"];
            if request["expected_generation"].as_u64().is_none()
                || state["generation"] != request["expected_generation"]
                || state["owner_thread_id"] != request["owner_thread_id"]
                || state["owner_host_id"] != request["owner_host_id"]
            {
                return Err("BOT_LIFECYCLE_STALE_OWNER");
            }
            if action == "assert-handoff-owner" {
                if version != 2
                    || request["role_id"] != "control"
                    || state["contract_version"] != 2
                    || state["phase"] != "MIGRATING"
                    || request["handoff_id"].as_str().is_none()
                    || state["handoff"]["handoff_id"] != request["handoff_id"]
                    || state["handoff"]["successor_thread_id"] != state["owner_thread_id"]
                    || state["ack"]["manifest_digest"].is_null()
                    || state["ack"]["manifest_digest"] != state["manifest_digest"]
                {
                    return Err("BOT_LIFECYCLE_ADMISSION_PAUSED");
                }
                value["status"] = json!("HANDOFF_OWNER_CURRENT");
            } else if state["phase"] != "ACTIVE" {
                return Err("BOT_LIFECYCLE_ADMISSION_PAUSED");
            } else {
                value["status"] = json!("OWNER_CURRENT");
            }
        }
        if value["current"]["contract_version"] == 2 {
            value["schema_version"] = json!("lattice.bot-lifecycle.v2");
        }
        tx.commit().map_err(error)?;
        return Ok(value);
    }
    for key in ["request_id", "owner_thread_id", "owner_host_id"] {
        text(key)?;
    }
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(error)?;
    let value: Value = tx
        .query_one(
            if version == 2 {
                "SELECT bot_lifecycle.apply_v2($1)"
            } else {
                "SELECT bot_lifecycle.apply_v1($1)"
            },
            &[request],
        )
        .map_err(error)?
        .get(0);
    tx.commit().map_err(|_| "BOT_LIFECYCLE_OUTCOME_UNKNOWN")?;
    Ok(value)
}
