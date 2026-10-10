//! Project binding and irreversible role retirement in the dedicated Bot store.
use super::{Result, connect, digest, error};
use crate::{MigrationTarget, project_registry::load_registry_for_maintenance};
use lattice_contracts::ProjectId;
use postgres::{Client, GenericClient, IsolationLevel, Transaction};
use serde_json::{Value, json};

const SQL: &str = include_str!("../../../db/extensions/bot-lifecycle/project-ownership-v1.sql");
const SCHEMA: &str = "lattice.project-purge.bot.v1";

/// Existing services keep their old registration contract until the explicit
/// ownership extension is installed. They remain unpurgeable when unattributable.
/// # Errors
/// Rejects incompatible catalogs, invalid ownership or unavailable database operations.
pub fn bot_lifecycle_requires_registry(port: u16, run: &str, password: &str) -> Result<bool> {
    let mut client = connect(port, run, password, "runtime")?;
    super::verify(&mut client, run)?;
    verify(&mut client)
}

pub(super) fn verify(client: &mut impl GenericClient) -> Result<bool> {
    let present: bool = client
        .query_one(
            "SELECT to_regnamespace('bot_project_ownership') IS NOT NULL",
            &[],
        )
        .map_err(|e| error(&e))?
        .get(0);
    if !present {
        return Ok(false);
    }
    let schema_safe:bool=client.query_one("SELECT pg_get_userbyid(nspowner)='lattice_migrator' AND NOT has_schema_privilege('lattice_runtime',oid,'CREATE') AND NOT EXISTS(SELECT 1 FROM aclexplode(COALESCE(nspacl,acldefault('n',nspowner))) a WHERE a.grantee=0) FROM pg_namespace WHERE nspname='bot_project_ownership'",&[]).map_err(|e|error(&e))?.get(0);
    let constraints_safe:bool=client.query_one("SELECT count(*)=12 AND count(*) FILTER(WHERE contype='p')=3 AND count(*) FILTER(WHERE contype='c')=8 AND count(*) FILTER(WHERE contype='f' AND confrelid='bot_lifecycle.roles'::regclass AND conkey=ARRAY[1,2]::smallint[] AND confkey=ARRAY[1,2]::smallint[] AND condeferrable AND condeferred)=1 AND bool_and(convalidated) FROM pg_constraint WHERE connamespace='bot_project_ownership'::regnamespace",&[]).map_err(|e|error(&e))?.get(0);
    let triggers_safe:bool=client.query_one("SELECT count(*)=2 AND bool_and(tgqual IS NULL AND tgnargs=0) FROM pg_trigger WHERE tgname='project_retirement_v1' AND tgrelid IN ('bot_lifecycle.roles'::regclass,'bot_lifecycle.events'::regclass)",&[]).map_err(|e|error(&e))?.get(0);
    if !schema_safe || !constraints_safe || !triggers_safe {
        return Err("BOT_LIFECYCLE_OWNERSHIP_SCHEMA_REJECTED");
    }
    let functions = client.query("SELECT p.proname,p.prosrc,p.prosecdef,pg_get_userbyid(p.proowner),p.proconfig,has_function_privilege('lattice_runtime',p.oid,'EXECUTE'),EXISTS(SELECT 1 FROM aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a WHERE a.grantee=0) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='bot_project_ownership' ORDER BY p.proname",&[]).map_err(|e|error(&e))?;
    if functions.len() != 3 {
        return Err("BOT_LIFECYCLE_OWNERSHIP_SCHEMA_REJECTED");
    }
    for (row, name) in functions.iter().zip(["guard_v1", "pair_v1", "register_v1"]) {
        let source = SQL
            .split_once(&format!("CREATE FUNCTION bot_project_ownership.{name}("))
            .and_then(|(_, v)| v.split_once("AS $$"))
            .and_then(|(_, v)| v.split_once("$$;"))
            .map(|(v, _)| v)
            .ok_or("BOT_LIFECYCLE_OWNERSHIP_SCHEMA_REJECTED")?;
        if row.get::<_, String>(0) != name
            || row.get::<_, String>(1) != source
            || row.get::<_, bool>(2) != (name != "pair_v1")
            || row.get::<_, String>(3) != "lattice_migrator"
            || row.get::<_, Option<Vec<String>>>(4) != Some(vec!["search_path=pg_catalog".into()])
            || row.get::<_, bool>(5) != (name == "register_v1")
            || row.get::<_, bool>(6)
        {
            return Err("BOT_LIFECYCLE_OWNERSHIP_SCHEMA_REJECTED");
        }
    }
    let tables=client.query("SELECT c.relname,pg_get_userbyid(c.relowner),has_table_privilege('lattice_runtime',c.oid,'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER'),EXISTS(SELECT 1 FROM aclexplode(COALESCE(c.relacl,acldefault('r',c.relowner))) a WHERE a.grantee=0) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='bot_project_ownership' AND c.relkind='r' ORDER BY c.relname",&[]).map_err(|e|error(&e))?;
    if tables.len() != 3 {
        return Err("BOT_LIFECYCLE_OWNERSHIP_SCHEMA_REJECTED");
    }
    for (row, name) in tables.iter().zip(["bindings", "receipts", "retired"]) {
        if row.get::<_, String>(0) != name
            || row.get::<_, String>(1) != "lattice_migrator"
            || row.get::<_, bool>(2)
            || row.get::<_, bool>(3)
        {
            return Err("BOT_LIFECYCLE_OWNERSHIP_SCHEMA_REJECTED");
        }
    }
    let columns:Vec<String>=client.query("SELECT c.relname||'.'||a.attname||':'||format_type(a.atttypid,a.atttypmod)||':'||a.attnotnull::text FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='bot_project_ownership' AND c.relkind='r' AND a.attnum>0 AND NOT a.attisdropped ORDER BY c.relname,a.attnum",&[]).map_err(|e|error(&e))?.into_iter().map(|r|r.get(0)).collect();
    if columns
        != [
            "bindings.project_id:text:true",
            "bindings.role_id:text:true",
            "bindings.store_digest:text:true",
            "bindings.observation_digest:text:true",
            "receipts.operation_digest:text:true",
            "receipts.scope_digest:text:true",
            "receipts.after_digest:text:true",
            "receipts.role_count:bigint:true",
            "receipts.event_count:bigint:true",
            "retired.pair_digest:text:true",
        ]
    {
        return Err("BOT_LIFECYCLE_OWNERSHIP_SCHEMA_REJECTED");
    }
    let triggers:Vec<String>=client.query("SELECT c.relname||':'||t.tgname||':'||t.tgtype::text||':'||t.tgenabled::text||':'||t.tgfoid::regprocedure::text FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='bot_lifecycle' AND NOT t.tgisinternal ORDER BY c.relname,t.tgname",&[]).map_err(|e|error(&e))?.into_iter().map(|r|r.get(0)).collect();
    if triggers
        != [
            "events:project_retirement_v1:23:O:bot_project_ownership.guard_v1()",
            "roles:project_retirement_v1:23:O:bot_project_ownership.guard_v1()",
        ]
    {
        return Err("BOT_LIFECYCLE_OWNERSHIP_SCHEMA_REJECTED");
    }
    Ok(true)
}

/// Install only the fixed additive extension; old ownership is never inferred.
/// # Errors
/// Rejects incompatible catalogs, invalid ownership or unavailable database operations.
pub fn install_bot_project_ownership(
    port: u16,
    run: &str,
    password: &str,
    system: &str,
) -> Result<Value> {
    let observation = super::inspect_project_purge_bot_lifecycle(
        port,
        run,
        password,
        system,
        "ownership-install",
    )?;
    if observation["databasePresent"] != true {
        return Err("BOT_LIFECYCLE_DATABASE_UNAVAILABLE");
    }
    let mut client = connect(port, run, password, "migrator")?;
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|e| error(&e))?;
    tx.batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'; LOCK TABLE bot_lifecycle.roles,bot_lifecycle.events IN ACCESS EXCLUSIVE MODE").map_err(|e|error(&e))?;
    super::verify(&mut tx, run)?;
    if !verify(&mut tx)? {
        tx.batch_execute(SQL).map_err(|e| error(&e))?;
    }
    verify(&mut tx)?;
    tx.commit().map_err(|_| "BOT_LIFECYCLE_OUTCOME_UNKNOWN")?;
    Ok(
        json!({"schema":SCHEMA,"status":"OWNERSHIP_INSTALLED","sqlDigest":digest(SQL.as_bytes()),"legacyOwnership":"UNATTRIBUTABLE"}),
    )
}

pub(super) fn registry_lock<'a>(
    client: &'a mut Client,
    target: &MigrationTarget,
    project: &str,
) -> Result<(Transaction<'a>, String)> {
    crate::postgres_setup::verify_project_purge_inventory_schema(client, target)
        .map_err(|_| "BOT_LIFECYCLE_REGISTRY_REJECTED")?;
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .start()
        .map_err(|e| error(&e))?;
    tx.batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'; LOCK TABLE control.runtime_admission,control.project_registry_state,control.project_registry_projects,control.project_registry_observations,control.project_registry_commands,control.project_registry_identity_reservations IN SHARE MODE").map_err(|e|error(&e))?;
    let state = load_registry_for_maintenance(&mut tx, target)
        .map_err(|_| "BOT_LIFECYCLE_REGISTRY_REJECTED")?;
    let id = ProjectId::new(project).map_err(|_| "BOT_LIFECYCLE_INPUT_REJECTED")?;
    let projection = state
        .project(&id)
        .ok_or("BOT_LIFECYCLE_REGISTRY_PROJECT_MISSING")?;
    Ok((tx, projection.observation().digest().as_str().to_owned()))
}

/// Registration binds only a currently verified Registry project. The native
/// application holds Registry locks until the dedicated Bot transaction commits.
#[allow(clippy::too_many_arguments)]
/// # Errors
/// Rejects incompatible catalogs, invalid ownership or unavailable database operations.
pub fn execute_bot_lifecycle_with_registry(
    main: &mut Client,
    target: &MigrationTarget,
    port: u16,
    run: &str,
    password: &str,
    request: &Value,
) -> Result<Value> {
    if request["action"] != "register" {
        return super::execute_bot_lifecycle(port, run, password, request);
    }
    let project = request["project_id"]
        .as_str()
        .ok_or("BOT_LIFECYCLE_INPUT_REJECTED")?;
    let (_registry, observation) = registry_lock(main, target, project)?;
    let mut client = connect(port, run, password, "runtime")?;
    super::verify(&mut client, run)?;
    if !verify(&mut client)? {
        return Err("BOT_LIFECYCLE_OWNERSHIP_EXTENSION_REQUIRED");
    }
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|e| error(&e))?;
    let result = tx
        .query_one(
            "SELECT bot_project_ownership.register_v1($1,$2,$3)",
            &[
                request,
                &target.expected_database_identity_sha256().as_str(),
                &observation,
            ],
        )
        .map_err(|e| error(&e))?
        .get(0);
    tx.commit().map_err(|_| "BOT_LIFECYCLE_OUTCOME_UNKNOWN")?;
    Ok(result)
}

pub(super) fn snapshot(client: &mut impl GenericClient) -> Result<Vec<(String, String)>> {
    let mut result = Vec::new();
    let mut bytes = 0usize;
    for table in [
        "bot_lifecycle.roles",
        "bot_lifecycle.events",
        "bot_project_ownership.bindings",
        "bot_project_ownership.retired",
    ] {
        let rows=client.query(&format!("SELECT to_jsonb(p)::text FROM ONLY {table} p ORDER BY to_jsonb(p)::text COLLATE \"C\" LIMIT 10001"),&[]).map_err(|e|error(&e))?;
        if rows.len() > 10000 {
            return Err("BOT_LIFECYCLE_PURGE_INVENTORY_LIMIT");
        }
        for row in rows {
            let value: String = row.get(0);
            bytes = bytes.saturating_add(value.len());
            if bytes > 32 * 1024 * 1024 {
                return Err("BOT_LIFECYCLE_PURGE_INVENTORY_LIMIT");
            }
            result.push((table.into(), value));
        }
    }
    Ok(result)
}
fn snapshot_digest(rows: &[(String, String)]) -> Result<String> {
    Ok(digest(
        &serde_json::to_vec(rows).map_err(|_| "BOT_LIFECYCLE_DATABASE_REJECTED")?,
    ))
}
fn receipt(
    client: &mut impl GenericClient,
    operation: &str,
    scope: Option<&str>,
) -> Result<Option<Value>> {
    let Some(row)=client.query_opt("SELECT scope_digest,after_digest,role_count,event_count FROM ONLY bot_project_ownership.receipts WHERE operation_digest=$1",&[&operation]).map_err(|e|error(&e))? else{return Ok(None);};
    let stored: String = row.get(0);
    let after: String = row.get(1);
    if scope.is_some_and(|v| v != stored) {
        return Err("BOT_LIFECYCLE_PURGE_SCOPE_MISMATCH");
    }
    if snapshot_digest(&snapshot(client)?)? != after {
        return Err("BOT_LIFECYCLE_PURGE_READBACK_CHANGED");
    }
    Ok(Some(
        json!({"schema":SCHEMA,"status":"PURGED","scopeDigest":stored,"afterDigest":after,"roleCount":row.get::<_,i64>(2),"eventCount":row.get::<_,i64>(3),"retirement":"PERMANENT_PROJECT_ROLE_PAIR"}),
    ))
}

/// Preview or erase proven Bot rows before the main Registry is erased. Status
/// uses the exact committed receipt and therefore works after Registry erasure.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
/// # Errors
/// Rejects incompatible catalogs, invalid ownership or unavailable database operations.
pub fn execute_bot_project_purge(
    main: &mut Client,
    target: &MigrationTarget,
    port: u16,
    run: &str,
    password: &str,
    system: &str,
    request: &Value,
) -> Result<Value> {
    let action = request["action"]
        .as_str()
        .ok_or("BOT_LIFECYCLE_INPUT_REJECTED")?;
    if !["preview-bot", "apply-bot", "status-bot"].contains(&action) {
        return Err("BOT_LIFECYCLE_INPUT_REJECTED");
    }
    let object = request.as_object().ok_or("BOT_LIFECYCLE_INPUT_REJECTED")?;
    if request["schema"] != "lattice.project-purge.request.v1"
        || object.keys().any(|k| {
            ![
                "schema",
                "action",
                "projectId",
                "operationId",
                "expectedBotScopeDigest",
                "authorization",
                "botBoundaries",
            ]
            .contains(&k.as_str())
        })
        || (action != "apply-bot"
            && (object.contains_key("authorization") || object.contains_key("botBoundaries")))
    {
        return Err("BOT_LIFECYCLE_INPUT_REJECTED");
    }
    let project = request["projectId"]
        .as_str()
        .ok_or("BOT_LIFECYCLE_INPUT_REJECTED")?;
    let operation = request["operationId"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 128)
        .ok_or("BOT_LIFECYCLE_INPUT_REJECTED")?;
    let inventory =
        super::inspect_project_purge_bot_lifecycle(port, run, password, system, project)?;
    if inventory["databasePresent"] != true {
        return Ok(json!({"schema":SCHEMA,"status":"VERIFIED_ABSENT","inventory":inventory}));
    }
    let mut client = connect(port, run, password, "migrator")?;
    if !verify(&mut client)? {
        return Ok(
            json!({"schema":SCHEMA,"status":"BLOCKED","blockers":["BOT_LIFECYCLE_OWNERSHIP_EXTENSION_REQUIRED"]}),
        );
    }
    let store = target.expected_database_identity_sha256().as_str();
    let op = digest(
        format!("lattice.bot-project-purge.operation.v1\n{store}\n{project}\n{operation}")
            .as_bytes(),
    );
    let expected = request["expectedBotScopeDigest"].as_str();
    // Use the same cross-database lock order as registration. A committed receipt
    // is checked again under Bot locks; it can be replayed after Registry removal.
    let prior: bool = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM bot_project_ownership.receipts WHERE operation_digest=$1)",
            &[&op],
        )
        .map_err(|e| error(&e))?
        .get(0);
    let mut registry = if action != "status-bot" && !prior {
        Some(registry_lock(main, target, project)?)
    } else {
        None
    };
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|e| error(&e))?;
    tx.batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'; LOCK TABLE bot_lifecycle.roles,bot_lifecycle.events,bot_project_ownership.bindings,bot_project_ownership.retired,bot_project_ownership.receipts IN ACCESS EXCLUSIVE MODE").map_err(|e|error(&e))?;
    super::verify(&mut tx, run)?;
    if let Some(value) = receipt(&mut tx, &op, expected)? {
        return Ok(value);
    }
    if action == "status-bot" {
        return Ok(json!({"schema":SCHEMA,"status":"NOT_FOUND"}));
    }
    if registry.is_none() {
        return Err("BOT_LIFECYCLE_PURGE_SCOPE_MISMATCH");
    }
    let before = snapshot(&mut tx)?;
    let bindings: std::collections::BTreeMap<(String, String), Value> = before
        .iter()
        .filter(|(table, _)| table == "bot_project_ownership.bindings")
        .map(|(_, row)| {
            let v: Value =
                serde_json::from_str(row).map_err(|_| "BOT_LIFECYCLE_DATABASE_REJECTED")?;
            Ok((
                (
                    v["project_id"].as_str().unwrap_or_default().to_owned(),
                    v["role_id"].as_str().unwrap_or_default().to_owned(),
                ),
                v,
            ))
        })
        .collect::<Result<_>>()?;
    let mut blockers = Vec::<&str>::new();
    if let Some((registry_tx, _)) = &mut registry {
        let stopped:bool=registry_tx.query_one("SELECT admission_mode='STOPPED' FROM ONLY control.runtime_admission WHERE singleton",&[]).map_err(|e|error(&e))?.get(0);
        if !stopped {
            blockers.push("BOT_LIFECYCLE_MAINTENANCE_OFFLINE_REQUIRED");
        }
    }
    let mut survivors = Vec::new();
    let mut owners = Vec::new();
    let mut event_count = 0i64;
    for (table, row) in &before {
        let value: Value =
            serde_json::from_str(row).map_err(|_| "BOT_LIFECYCLE_DATABASE_REJECTED")?;
        if table == "bot_project_ownership.retired" {
            survivors.push((table.clone(), row.clone()));
            continue;
        }
        let pair = (
            value["project_id"].as_str().unwrap_or_default().to_owned(),
            value["role_id"].as_str().unwrap_or_default().to_owned(),
        );
        let Some(binding) = bindings.get(&pair) else {
            blockers.push("BOT_LIFECYCLE_LEGACY_OWNERSHIP_UNATTRIBUTABLE");
            survivors.push((table.clone(), row.clone()));
            continue;
        };
        if binding["store_digest"] == store && pair.0 == project {
            if table == "bot_lifecycle.roles" {
                let state = &value["state"];
                if state["phase"] != "ACTIVE"
                    || !state["handoff"].is_null()
                    || state["steps"] != json!({})
                    || state["manifest"].as_object().is_some_and(|m| {
                        ["pending_input_ids", "in_flight_ids"]
                            .iter()
                            .any(|k| m.get(*k) != Some(&json!([])))
                    })
                {
                    blockers.push("BOT_LIFECYCLE_OWNER_NOT_QUIESCENT");
                }
                owners.push(json!({"roleId":pair.1,"ownerThreadId":state["owner_thread_id"],"ownerHostId":state["owner_host_id"],"revision":state["revision"],"generation":state["generation"]}));
            }
            if table == "bot_lifecycle.events" {
                event_count += 1;
            }
        } else {
            survivors.push((table.clone(), row.clone()));
        }
    }
    // Retained event bodies may mention another project even with a valid owner.
    for (table, row) in &survivors {
        if table != "bot_project_ownership.retired"
            && contains_text(
                &serde_json::from_str(row).map_err(|_| "BOT_LIFECYCLE_DATABASE_REJECTED")?,
                project,
            )
        {
            blockers.push("BOT_LIFECYCLE_SURVIVOR_REFERENCE");
        }
    }
    if owners.len() > 64 {
        blockers.push("BOT_LIFECYCLE_PURGE_INVENTORY_LIMIT");
    }
    blockers.sort_unstable();
    blockers.dedup();
    let scope=digest(&serde_json::to_vec(&json!({"schema":SCHEMA,"store":store,"botSystem":system,"botRun":run,"project":project,"operation":op,"snapshot":snapshot_digest(&before)?})).map_err(|_|"BOT_LIFECYCLE_DATABASE_REJECTED")?);
    let plan = json!({"schema":SCHEMA,"status":if blockers.is_empty(){"READY"}else{"BLOCKED"},"scopeDigest":scope,"owners":owners,"roleCount":owners.len(),"eventCount":event_count,"blockers":blockers,"retirement":"PERMANENT_PROJECT_ROLE_PAIR","binding":"APPLICATION_VERIFIED_REGISTRY"});
    if action == "preview-bot" {
        return Ok(plan);
    }
    if !blockers.is_empty() {
        return Err("BOT_LIFECYCLE_PURGE_BLOCKED");
    }
    if expected != Some(&scope) || request["authorization"] != "ERASE_PROJECT_DATA" {
        return Err("BOT_LIFECYCLE_PURGE_SCOPE_MISMATCH");
    }
    let boundaries = request["botBoundaries"]
        .as_array()
        .ok_or("BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED")?;
    if boundaries.len() != owners.len() {
        return Err("BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED");
    }
    for owner in &owners {
        let matched: Vec<_> = boundaries
            .iter()
            .filter(|b| b["roleId"] == owner["roleId"])
            .collect();
        if matched.len() != 1
            || matched[0]["revision"] != owner["revision"]
            || matched[0]["generation"] != owner["generation"]
        {
            return Err("BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED");
        }
        validate_boundary(&mut tx, &matched[0]["boundary"], owner)?;
    }
    for owner in &owners {
        tx.execute(
            "INSERT INTO bot_project_ownership.retired SELECT bot_project_ownership.pair_v1($1,$2)",
            &[
                &project,
                &owner["roleId"]
                    .as_str()
                    .ok_or("BOT_LIFECYCLE_DATABASE_REJECTED")?,
            ],
        )
        .map_err(|e| error(&e))?;
    }
    for table in [
        "bot_lifecycle.events",
        "bot_project_ownership.bindings",
        "bot_lifecycle.roles",
    ] {
        for owner in &owners {
            tx.execute(
                &format!("DELETE FROM ONLY {table} WHERE project_id=$1 AND role_id=$2"),
                &[
                    &project,
                    &owner["roleId"]
                        .as_str()
                        .ok_or("BOT_LIFECYCLE_DATABASE_REJECTED")?,
                ],
            )
            .map_err(|e| error(&e))?;
        }
    }
    let after = snapshot(&mut tx)?;
    let retained: Vec<_> = after
        .iter()
        .filter(|(table, _)| table != "bot_project_ownership.retired")
        .cloned()
        .collect();
    let expected_retained: Vec<_> = survivors
        .iter()
        .filter(|(table, _)| table != "bot_project_ownership.retired")
        .cloned()
        .collect();
    let mut expected_retired: std::collections::BTreeSet<String> = survivors
        .iter()
        .filter(|(t, _)| t == "bot_project_ownership.retired")
        .map(|(_, v)| {
            serde_json::from_str::<Value>(v)
                .map_err(|_| "BOT_LIFECYCLE_DATABASE_REJECTED")?["pair_digest"]
                .as_str()
                .map(str::to_owned)
                .ok_or("BOT_LIFECYCLE_DATABASE_REJECTED")
        })
        .collect::<Result<_>>()?;
    for owner in &owners {
        expected_retired.insert(digest(
            format!(
                "lattice.bot-project-retirement.v1\n{project}\n{}",
                owner["roleId"]
                    .as_str()
                    .ok_or("BOT_LIFECYCLE_DATABASE_REJECTED")?
            )
            .as_bytes(),
        ));
    }
    let actual_retired: std::collections::BTreeSet<String> = after
        .iter()
        .filter(|(t, _)| t == "bot_project_ownership.retired")
        .map(|(_, v)| {
            serde_json::from_str::<Value>(v)
                .map_err(|_| "BOT_LIFECYCLE_DATABASE_REJECTED")?["pair_digest"]
                .as_str()
                .map(str::to_owned)
                .ok_or("BOT_LIFECYCLE_DATABASE_REJECTED")
        })
        .collect::<Result<_>>()?;
    if retained != expected_retained
        || expected_retired != actual_retired
        || !survivors
            .iter()
            .filter(|(t, _)| t == "bot_project_ownership.retired")
            .all(|r| after.contains(r))
    {
        return Err("BOT_LIFECYCLE_PURGE_SURVIVOR_CHANGED");
    }
    let after_digest = snapshot_digest(&after)?;
    tx.execute(
        "INSERT INTO bot_project_ownership.receipts VALUES($1,$2,$3,$4,$5)",
        &[
            &op,
            &scope,
            &after_digest,
            &i64::try_from(owners.len()).map_err(|_| "BOT_LIFECYCLE_DATABASE_REJECTED")?,
            &event_count,
        ],
    )
    .map_err(|e| error(&e))?;
    let result =
        receipt(&mut tx, &op, Some(&scope))?.ok_or("BOT_LIFECYCLE_PURGE_READBACK_CHANGED")?;
    tx.commit().map_err(|_| "BOT_LIFECYCLE_OUTCOME_UNKNOWN")?;
    Ok(result)
}

fn contains_text(value: &Value, text: &str) -> bool {
    match value {
        Value::String(v) => v.contains(text),
        Value::Array(v) => v.iter().any(|v| contains_text(v, text)),
        Value::Object(v) => v
            .iter()
            .any(|(k, v)| k.contains(text) || contains_text(v, text)),
        _ => false,
    }
}
fn validate_boundary(
    client: &mut impl GenericClient,
    boundary: &Value,
    owner: &Value,
) -> Result<()> {
    let object = boundary
        .as_object()
        .ok_or("BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED")?;
    let keys = [
        "source",
        "observed_at",
        "thread_id",
        "host_id",
        "latest_turn_id",
        "thread_updated_at",
        "status",
        "latest_turn_status",
        "pending_input_count",
        "in_flight_count",
        "readback_digest",
        "evidence_ref",
    ];
    let hex = |v: &Value| {
        v.as_str().is_some_and(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
    };
    if object.len() != keys.len()
        || keys.iter().any(|k| !object.contains_key(*k))
        || boundary["source"] != "codex.read_thread"
        || boundary["thread_id"] != owner["ownerThreadId"]
        || boundary["host_id"] != owner["ownerHostId"]
        || !matches!(boundary["status"].as_str(), Some("idle" | "notLoaded"))
        || boundary["latest_turn_status"] != "completed"
        || boundary["pending_input_count"] != 0
        || boundary["in_flight_count"] != 0
        || !hex(&boundary["readback_digest"])
        || !boundary["latest_turn_id"].as_str().is_some_and(|s| {
            s.len() == 36
                && s.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit() && !b.is_ascii_uppercase()
                    }
                })
        })
        || boundary["thread_updated_at"].as_u64().is_none()
        || !boundary["evidence_ref"].as_str().is_some_and(|s| {
            !s.is_empty()
                && s.len() <= 512
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:/#-".contains(&b))
        })
    {
        return Err("BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED");
    }
    let observed = boundary["observed_at"]
        .as_str()
        .ok_or("BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED")?;
    let valid:bool=client.query_one("SELECT $1::text::timestamptz BETWEEN clock_timestamp()-interval '300 seconds' AND clock_timestamp()+interval '5 seconds'",&[&observed]).map_err(|_|"BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED")?.get(0);
    if !valid {
        return Err("BOT_LIFECYCLE_NATIVE_BOUNDARY_REJECTED");
    }
    Ok(())
}
