//! Bounded operator-only physical erasure for a complete Registry command suffix.
//! A successful result is deliberately PG-only; the caller must separately erase
//! its local catalog/files and cannot infer success from a project being absent.
use crate::project_registry::load_registry_for_maintenance;
use crate::{DatabaseRole, MigrationTarget, verify_postgres_schema};
use lattice_contracts::ProjectId;
use lattice_project_registry::{VerifiedRegistryState, project_purge_prefix};
use postgres::{Client, Config, GenericClient, IsolationLevel, NoTls, Transaction};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::time::Duration;

pub const PROJECT_PURGE_SQL: &str = include_str!("../../../db/extensions/project-purge/v1.sql");
type Result<T> = std::result::Result<T, &'static str>;
const SCHEMA: &str = "lattice.project-purge.result.v1";
const PARAMS: &str = " AND $1::text IS NOT NULL AND $2::text[] IS NOT NULL AND $3::text[] IS NOT NULL AND $4::text[] IS NOT NULL";

fn digest(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    value
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or("PROJECT_PURGE_INPUT_REJECTED")
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}
fn db<T>(r: std::result::Result<T, postgres::Error>) -> Result<T> {
    r.map_err(|_| "PROJECT_PURGE_DATABASE_REJECTED")
}
fn quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn references(value: &Value, project: &str, canonical: &str, keys: &[&String]) -> bool {
    match value {
        Value::String(s) => {
            s.contains(project)
                || s.replace('\\', "/")
                    .to_lowercase()
                    .contains(&canonical.replace('\\', "/").to_lowercase())
                || keys.iter().any(|key| s.contains(key.as_str()))
                || serde_json::from_str::<Value>(s)
                    .is_ok_and(|nested| references(&nested, project, canonical, keys))
        }
        Value::Array(values) => values
            .iter()
            .any(|v| references(v, project, canonical, keys)),
        Value::Object(values) => values.iter().any(|(k, v)| {
            references(&Value::String(k.clone()), project, canonical, keys)
                || references(v, project, canonical, keys)
        }),
        _ => false,
    }
}

/// Fixed loopback, fixed existing migrator identity; credentials never enter JSON.
///
/// # Errors
///
/// Rejects invalid connection settings or an unavailable authenticated database.
pub fn connect_project_purge(
    port: u16,
    run_id: &str,
    password: &str,
) -> Result<(Client, MigrationTarget)> {
    let prefix = run_id
        .get(..8)
        .ok_or("PROJECT_PURGE_CONFIGURATION_REJECTED")?;
    let target = MigrationTarget::new(format!("lattice_task019_{prefix}_base"), run_id)
        .map_err(|_| "PROJECT_PURGE_CONFIGURATION_REJECTED")?;
    if port == 0 || password.is_empty() {
        return Err("PROJECT_PURGE_CONFIGURATION_REJECTED");
    }
    let mut config = Config::new();
    config
        .host("127.0.0.1")
        .port(port)
        .user("lattice_migrator_login")
        .password(password)
        .application_name("lattice-devos-task019")
        .dbname(target.database_name())
        .options("-c role=lattice_migrator -c search_path=pg_catalog")
        .connect_timeout(Duration::from_secs(5));
    let client = config
        .connect_with_startup_timeout(NoTls, Duration::from_secs(5))
        .map_err(|_| "PROJECT_PURGE_DATABASE_UNAVAILABLE")?
        .ok_or("PROJECT_PURGE_DATABASE_UNAVAILABLE")?;
    Ok((client, target))
}

// Runtime can validate this catalog without access to maintenance receipt data.
// Only these exact relation OIDs become part of the principal boundary.
pub(crate) fn optional_maintenance_relations<C: GenericClient>(client: &mut C) -> Result<Vec<i64>> {
    let exists: bool =
        db(client.query_one("SELECT to_regnamespace('project_purge') IS NOT NULL", &[]))?.get(0);
    if !exists {
        return Ok(Vec::new());
    }
    let unknown: i64 = db(client.query_one("SELECT count(*)::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='project_purge' AND NOT ((c.relkind='r' AND c.relname IN ('identity','receipts')) OR (c.relkind='i' AND c.relname IN ('identity_pkey','receipts_pkey')))",&[]))?.get(0);
    if unknown != 0 {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let objects: i64 = db(client.query_one("SELECT (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='project_purge') + (SELECT count(*) FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace WHERE n.nspname='project_purge' AND NOT (t.typtype='c' AND t.typname IN ('identity','receipts') OR t.typtype='b' AND t.typname IN ('_identity','_receipts') AND t.typelem<>0))", &[]))?.get(0);
    if objects != 0 {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let boundary: bool = db(client.query_one("SELECT pg_get_userbyid(n.nspowner)='lattice_migrator' AND NOT EXISTS(SELECT 1 FROM aclexplode(COALESCE(n.nspacl,acldefault('n',n.nspowner))) acl WHERE acl.grantee<>n.nspowner) AND NOT EXISTS(SELECT 1 FROM pg_class c WHERE c.relnamespace=n.oid AND (pg_get_userbyid(c.relowner)<>'lattice_migrator' OR c.relrowsecurity OR c.relforcerowsecurity OR c.relispartition OR EXISTS(SELECT 1 FROM aclexplode(COALESCE(c.relacl,acldefault('r',c.relowner))) acl WHERE acl.grantee<>c.relowner))) AND NOT EXISTS(SELECT 1 FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid WHERE c.relnamespace=n.oid AND (a.attacl IS NOT NULL OR a.attisdropped OR a.attgenerated<>'' OR a.attidentity<>'')) AND NOT EXISTS(SELECT 1 FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid WHERE c.relnamespace=n.oid) AND NOT EXISTS(SELECT 1 FROM pg_policy p JOIN pg_class c ON c.oid=p.polrelid WHERE c.relnamespace=n.oid) AND NOT EXISTS(SELECT 1 FROM pg_rewrite r JOIN pg_class c ON c.oid=r.ev_class WHERE c.relnamespace=n.oid) AND NOT EXISTS(SELECT 1 FROM pg_type t WHERE t.typnamespace=n.oid AND (pg_get_userbyid(t.typowner)<>'lattice_migrator' OR t.typacl IS NOT NULL)) FROM pg_namespace n WHERE n.nspname='project_purge'",&[]))?.get(0);
    if !boundary {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let unsupported: i64 = db(client.query_one("SELECT (SELECT count(*) FROM pg_constraint k JOIN pg_namespace n ON n.oid=k.connamespace WHERE n.nspname='project_purge' AND (NOT k.convalidated OR k.condeferrable)) + (SELECT count(*) FROM pg_inherits i JOIN pg_class c ON c.oid=i.inhrelid OR c.oid=i.inhparent JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='project_purge')", &[]))?.get(0);
    if unsupported != 0 {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let columns: Vec<String> = db(client.query("SELECT c.relname||'.'||a.attname||':'||format_type(a.atttypid,a.atttypmod)||':'||a.attnotnull::text FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='project_purge' AND c.relkind='r' AND a.attnum>0 AND NOT a.attisdropped ORDER BY c.relname,a.attnum", &[]))?.into_iter().map(|r|r.get(0)).collect();
    if columns
        != [
            "identity.singleton:boolean:true",
            "identity.sql_sha256:text:true",
            "receipts.operation_id:text:true",
            "receipts.scope_digest:text:true",
            "receipts.request_digest:text:true",
            "receipts.result:jsonb:true",
            "receipts.completed_at:timestamp with time zone:true",
        ]
    {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let constraints: Vec<String> = db(client.query("SELECT c.relname||'.'||k.conname||':'||pg_get_constraintdef(k.oid,false) FROM pg_constraint k JOIN pg_class c ON c.oid=k.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='project_purge' AND k.convalidated AND NOT k.condeferrable ORDER BY c.relname,k.conname", &[]))?.into_iter().map(|r|r.get(0)).collect();
    if constraints
        != [
            "identity.identity_pkey:PRIMARY KEY (singleton)",
            "identity.identity_singleton_check:CHECK (singleton)",
            "identity.identity_sql_sha256_check:CHECK ((sql_sha256 ~ '^[a-f0-9]{64}$'::text))",
            "receipts.receipts_operation_id_check:CHECK ((operation_id ~ '^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$'::text))",
            "receipts.receipts_pkey:PRIMARY KEY (operation_id)",
            "receipts.receipts_request_digest_check:CHECK ((request_digest ~ '^[a-f0-9]{64}$'::text))",
            "receipts.receipts_result_check:CHECK ((jsonb_typeof(result) = 'object'::text))",
            "receipts.receipts_scope_digest_check:CHECK ((scope_digest ~ '^[a-f0-9]{64}$'::text))",
        ]
    {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let defaults: Vec<String> = db(client.query("SELECT c.relname||'.'||a.attname||':'||pg_get_expr(d.adbin,d.adrelid,false) FROM pg_attrdef d JOIN pg_class c ON c.oid=d.adrelid JOIN pg_attribute a ON a.attrelid=c.oid AND a.attnum=d.adnum JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='project_purge' ORDER BY c.relname,a.attnum", &[]))?.into_iter().map(|r|r.get(0)).collect();
    if defaults != ["receipts.completed_at:clock_timestamp()"] {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let indexes: Vec<String> = db(client.query("SELECT pg_get_indexdef(c.oid) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace JOIN pg_index i ON i.indexrelid=c.oid WHERE n.nspname='project_purge' AND i.indisvalid AND i.indisready AND i.indisprimary ORDER BY c.relname", &[]))?.into_iter().map(|r|r.get(0)).collect();
    if indexes
        != [
            "CREATE UNIQUE INDEX identity_pkey ON project_purge.identity USING btree (singleton)",
            "CREATE UNIQUE INDEX receipts_pkey ON project_purge.receipts USING btree (operation_id)",
        ]
    {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let relations: Vec<i64> = db(client.query("SELECT c.oid::bigint FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='project_purge' ORDER BY c.oid", &[]))?.into_iter().map(|r|r.get(0)).collect();
    if relations.len() != 4 {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    Ok(relations)
}

fn extension<C: GenericClient>(client: &mut C) -> Result<()> {
    if optional_maintenance_relations(client)?.len() != 4 {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    let rows = db(client.query(
        "SELECT sql_sha256 FROM ONLY project_purge.identity WHERE singleton",
        &[],
    ))?;
    if rows.len() != 1 || rows[0].get::<_, String>(0) != digest(PROJECT_PURGE_SQL.as_bytes()) {
        return Err("PROJECT_PURGE_EXTENSION_REJECTED");
    }
    Ok(())
}

struct Plan {
    public: Value,
    prefix: Option<VerifiedRegistryState>,
    streams: Vec<String>,
    tasks: Vec<String>,
    claims: Vec<String>,
    deletes: Vec<(String, String)>,
    survivor_digest: String,
}

// Fixed deletion order. Anything outside this closed list containing target
// identity is blocked, never swept by a substring deletion.
fn selectors() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "control_product.graph_usage_finishes",
            "p.usage_id IN (SELECT usage_id FROM control_product.graph_usage_starts WHERE project_id=$1)",
        ),
        ("control_product.graph_usage_starts", "p.project_id=$1"),
        (
            "control_product.conversation_observations",
            "p.claim_id=ANY($4)",
        ),
        ("control_product.conversation_claims", "p.project_id=$1"),
        ("control_product.work_metadata", "p.project_id=$1"),
        (
            "control_product.local_result_bindings",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        (
            "control_product.local_verified_result_evidence",
            "p.project_id=$1",
        ),
        (
            "control.task_external_verified_result_adoptions",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        (
            "control.external_verified_result_evidence",
            "p.project_id=$1",
        ),
        ("control.task_submission_envelopes", "p.project_id=$1"),
        (
            "control.task_ingress_historical_ambiguities",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        (
            "control.task_ingress_claims",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        (
            "control.task_ledger_foreman_snapshots",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        (
            "control.task_ledger_autonomy_receipts",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        (
            "control.task_ledger_outbox",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        (
            "control.task_ledger_events",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        (
            "control.task_ledger_commands",
            "encode(p.stream_id,'hex')=ANY($2)",
        ),
        ("control.task_ledger_streams", "p.project_id=$1"),
        ("control.terminal_transactions", "p.project_id=$1"),
        ("control.physical_heads", "p.project_id=$1"),
    ]
}

fn known_table(schema: &str, name: &str) -> bool {
    let names = match schema {
        "control" => {
            "database_identity migration_history schema_compatibility runtime_admission physical_heads terminal_transactions project_registry_state project_registry_observations project_registry_projects project_registry_commands project_registry_identity_reservations task_ledger_streams task_ledger_commands task_ledger_events task_ledger_outbox task_ledger_foreman_snapshots task_ingress_claims task_ingress_historical_ambiguities task_submission_envelopes external_verified_result_evidence task_external_verified_result_adoptions task_ledger_autonomy_receipts"
        }
        "control_product" => {
            "extension_identity work_metadata conversation_claims conversation_observations decisions decision_state local_verified_result_evidence local_result_bindings graph_usage_starts graph_usage_finishes"
        }
        "memory" => {
            "codebase_memory_extension_identity codebase_memory_extension_ledger codebase_memory_analyses codebase_memory_records codebase_memory_retrieval_audits codebase_memory_receipts codebase_memory_reflections openclaw_gateway_commands"
        }
        "writer_lease" => {
            "writer_lease_extension_identity writer_lease_extension_ledger writer_lease_heads writer_lease_commands writer_lease_transitions"
        }
        "foreman_execution" => {
            "extension_identity extension_ledger child_events preparation_observations promotion_intents task_promotions worker_attempts pending_worker_claims execution_environments worker_observations verification_records artifact_references staged_artifact_references provider_dispatch_claims attempt_closures approval_owner_snapshots approval_evidence"
        }
        "project_purge" => "identity receipts",
        _ => return false,
    };
    names.split_whitespace().any(|n| n == name)
}
fn all_tables<C: GenericClient>(client: &mut C) -> Result<Vec<(String, String)>> {
    let rows=db(client.query("SELECT n.nspname,c.relname,c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p','f','m','v') AND n.nspname NOT LIKE 'pg_%' AND n.nspname <> 'information_schema' ORDER BY n.nspname,c.relname", &[]))?;
    let mut tables = Vec::new();
    for row in rows {
        let schema: String = row.get(0);
        let name: String = row.get(1);
        let kind: String = row.get(2);
        if kind != "r" || !known_table(&schema, &name) {
            return Err("PROJECT_PURGE_UNSUPPORTED_CATALOG");
        }
        tables.push((schema, name));
    }
    Ok(tables)
}

fn snapshots<C: GenericClient>(
    client: &mut C,
    tables: &[(String, String)],
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut result = BTreeMap::new();
    let mut bytes = 0usize;
    for (schema, name) in tables {
        if schema == "project_purge" {
            continue;
        }
        let bounds=db(client.query_one(&format!("SELECT count(*)::bigint,COALESCE(sum(octet_length(value)),0)::bigint FROM (SELECT to_jsonb(p)::text value FROM ONLY {}.{} p LIMIT 100001) bounded",quoted(schema),quoted(name)),&[]))?;
        let count: i64 = bounds.get(0);
        let size: i64 = bounds.get(1);
        if count > 100_000
            || !(0..=64 * 1024 * 1024).contains(&size)
            || bytes
                .checked_add(
                    usize::try_from(size).map_err(|_| "PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?,
                )
                .is_none_or(|n| n > 64 * 1024 * 1024)
        {
            return Err("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED");
        }
        let rows = db(client.query(&format!("SELECT to_jsonb(p)::text FROM ONLY {}.{} p ORDER BY to_jsonb(p)::text COLLATE \"C\" LIMIT 100001", quoted(schema), quoted(name)), &[]))?;
        // Explicit bound; never silently truncate a deletion scope.
        if rows.len() > 100_000 {
            return Err("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED");
        }
        let values: Vec<String> = rows.into_iter().map(|r| r.get(0)).collect();
        bytes = bytes
            .checked_add(values.iter().map(String::len).sum())
            .ok_or("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?;
        if bytes > 64 * 1024 * 1024 {
            return Err("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED");
        }
        result.insert(format!("{schema}.{name}"), values);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::references;
    use serde_json::{Value, json};

    #[test]
    fn reference_scan_decodes_windows_paths_and_nested_json_text() {
        let canonical = r"C:\Fixture\Project-A";
        let encoded = serde_json::to_string(&json!({"retainedPath":canonical})).unwrap();
        let row = json!({"envelope":encoded});
        assert!(references(&row, "target-id", canonical, &[]));
        assert!(references(
            &json!({"path":"c:/fixture/project-a/file"}),
            "target-id",
            canonical,
            &[]
        ));
        assert!(!references(
            &json!({"path":"c:/fixture/project-b/file"}),
            "target-id",
            canonical,
            &[]
        ));
        let stream = "0123456789abcdef".to_owned();
        assert!(references(
            &Value::String(format!("\\x{stream}")),
            "target-id",
            canonical,
            &[&stream]
        ));
    }
}

// Keep the complete, closed ownership scan together for review.
#[allow(clippy::too_many_lines)]
fn prepare<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    project: &str,
    operation: &str,
) -> Result<Plan> {
    let id = ProjectId::new(project).map_err(|_| "PROJECT_PURGE_INPUT_REJECTED")?;
    let state = load_registry_for_maintenance(client, target)
        .map_err(|_| "PROJECT_PURGE_REGISTRY_CORRUPT")?;
    let projection = state
        .project(&id)
        .ok_or("PROJECT_PURGE_PROJECT_NOT_FOUND")?;
    let canonical = projection.observation().canonical_root();
    let mut protected_roots: Vec<String> = db(client.query("SELECT o.canonical_root FROM control.project_registry_projects p JOIN control.project_registry_observations o ON o.observation_digest=p.accepted_observation_digest WHERE p.project_id<>$1 ORDER BY p.project_id", &[&project]))?.into_iter().map(|r|r.get(0)).collect();
    let prefix = project_purge_prefix(&state, &id).ok();
    let mut blockers = Vec::<Value>::new();
    if prefix.is_none() {
        blockers.push(json!({"code":"REGISTRY_INTERLEAVED_HISTORY_REQUIRES_MIGRATION"}));
    }
    let streams: Vec<String> = db(client.query("SELECT encode(stream_id,'hex') FROM ONLY control.task_ledger_streams WHERE project_id=$1 ORDER BY stream_id", &[&project]))?.into_iter().map(|r| r.get(0)).collect();
    let tasks: Vec<String> = db(client.query("SELECT task_ref::text FROM ONLY control.task_submission_envelopes WHERE project_id=$1 ORDER BY task_ref", &[&project]))?.into_iter().map(|r| r.get(0)).collect();
    let tables = all_tables(client)?;
    let physical = snapshots(client, &tables)?;
    let mut claims = Vec::<String>::new();
    let mut roots = vec![json!({"path":canonical,"source":"POSTGRES_REGISTRY_OBSERVATION"})];
    if physical.contains_key("control_product.conversation_claims") {
        protected_roots.extend(db(client.query("SELECT worktree_path FROM ONLY control_product.conversation_claims WHERE project_id<>$1 AND worktree_path<>'' ORDER BY claim_id", &[&project]))?.into_iter().map(|row|row.get::<_,String>(0)));
        for row in db(client.query("SELECT claim_id,worktree_path FROM ONLY control_product.conversation_claims WHERE project_id=$1 ORDER BY claim_id", &[&project]))? {
            claims.push(row.get(0));
            let path: String = row.get(1);
            if !path.is_empty() { roots.push(json!({"path":path,"source":"POSTGRES_CONVERSATION_CLAIM"})); }
        }
        let active: i64 = db(client.query_one("SELECT count(*)::bigint FROM control_product.conversation_claims c CROSS JOIN LATERAL (SELECT COALESCE(max(sequence) FILTER(WHERE kind IN ('THREAD_BOUND','DISPATCH_STARTED','TURN_BOUND')),0) dispatch,COALESCE(max(sequence) FILTER(WHERE kind IN ('TURN_COMPLETED','TURN_FAILED','INTERRUPTED','VERIFICATION_FAILED','VERIFICATION_PASSED','CLAIM_FAILED')),-1) terminal,(array_agg(kind ORDER BY sequence DESC))[1] latest FROM control_product.conversation_observations o WHERE o.claim_id=c.claim_id) s WHERE c.project_id=$1 AND (s.terminal<s.dispatch OR s.latest IS NULL OR s.latest IN ('QUESTION_REQUESTED','APPROVAL_REQUESTED','INPUT_QUEUED'))", &[&project]))?.get(0);
        if active > 0 {
            blockers.push(json!({"code":"LIVE_OR_UNCERTAIN_CONVERSATION_CLAIM","count":active}));
        }
    }
    protected_roots.sort();
    protected_roots.dedup();
    for root in &roots {
        let path = text(root, "path")?
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase();
        if protected_roots.iter().any(|r| {
            let other = r.replace('\\', "/").trim_end_matches('/').to_lowercase();
            path == other
                || other.starts_with(&format!("{path}/"))
                || path.starts_with(&format!("{other}/"))
        }) {
            blockers.push(json!({"code":"SHARED_OR_OVERLAPPING_FILESYSTEM_ROOT"}));
        }
    }
    let params: [&(dyn postgres::types::ToSql + Sync); 4] = [&project, &streams, &tasks, &claims];
    let mut selected = BTreeMap::<String, Vec<String>>::new();
    let mut deletes = Vec::new();
    for (table, predicate) in selectors() {
        if !physical.contains_key(table) {
            continue;
        }
        let sql = format!(
            "SELECT to_jsonb(p)::text FROM ONLY {table} p WHERE ({predicate}){PARAMS} ORDER BY to_jsonb(p)::text COLLATE \"C\""
        );
        let rows: Vec<String> = db(client.query(&sql, &params))?
            .into_iter()
            .map(|r| r.get(0))
            .collect();
        selected.insert(table.into(), rows);
        deletes.push((table.into(), predicate.into()));
    }
    // Ignore only the Registry aggregate itself here: full replay, exact suffix
    // and post-erasure verification separately prove its survivor bytes.
    let mut survivors = physical.clone();
    for (table, rows) in &physical {
        if table.starts_with("control.project_registry_") {
            continue;
        }
        let selected_rows = selected.get(table).cloned().unwrap_or_default();
        let retained: Vec<String> = rows
            .iter()
            .filter(|r| !selected_rows.contains(r))
            .cloned()
            .collect();
        let keys: Vec<&String> = streams
            .iter()
            .chain(tasks.iter())
            .chain(claims.iter())
            .collect();
        let reference_count = retained
            .iter()
            .filter(|r| {
                serde_json::from_str(r).map_or(true, |v| references(&v, project, canonical, &keys))
            })
            .count();
        if reference_count > 0 {
            blockers.push(json!({"code":"CROSS_SCOPE_OR_UNSUPPORTED_REFERENCE","table":table,"count":reference_count}));
        }
        survivors.insert(table.clone(), retained);
    }
    survivors.retain(|table, _| !table.starts_with("control.project_registry_"));
    let mut counts: BTreeMap<String, u64> = selected
        .iter()
        .map(|(k, v)| (k.clone(), v.len() as u64))
        .collect();
    if let Some(prior) = &prefix {
        counts.insert(
            "control.project_registry_commands".into(),
            state.checkpoint().command_count() - prior.checkpoint().command_count(),
        );
        counts.insert(
            "control.project_registry_projects".into(),
            state.checkpoint().project_count() - prior.checkpoint().project_count(),
        );
        counts.insert(
            "control.project_registry_observations".into(),
            state.checkpoint().observation_count() - prior.checkpoint().observation_count(),
        );
        counts.insert(
            "control.project_registry_identity_reservations".into(),
            state.checkpoint().reservation_count() - prior.checkpoint().reservation_count(),
        );
    }
    let scope_digest = digest(&serde_json::to_vec(&json!({"schema":"lattice.project-purge.scope.v1","database":target.expected_database_identity_sha256().as_str(),"project":project,"operation":operation,"rows":physical})).map_err(|_| "PROJECT_PURGE_SERIALIZATION")?);
    Ok(Plan {
        public: json!({"schema":SCHEMA,"status":if blockers.is_empty(){"READY"}else{"BLOCKED"},"project":{"id":project,"canonicalPath":canonical},"operationId":operation,"scopeDigest":scope_digest,"registryStrategy":"VERIFIED_SUFFIX_V1","filesystemRoots":roots,"protectedRoots":protected_roots,"counts":counts,"blockers":blockers}),
        prefix,
        streams,
        tasks,
        claims,
        deletes,
        survivor_digest: digest(
            &serde_json::to_vec(&survivors).map_err(|_| "PROJECT_PURGE_SERIALIZATION")?,
        ),
    })
}

fn erase(
    transaction: &mut Transaction<'_>,
    target: &MigrationTarget,
    project: &str,
    plan: &Plan,
) -> Result<()> {
    let params: [&(dyn postgres::types::ToSql + Sync); 4] =
        [&project, &plan.streams, &plan.tasks, &plan.claims];
    for (table, predicate) in &plan.deletes {
        db(transaction.execute(
            &format!("DELETE FROM ONLY {table} p WHERE ({predicate}){PARAMS}"),
            &params,
        ))?;
    }
    let prefix = plan.prefix.as_ref().ok_or("PROJECT_PURGE_SCOPE_BLOCKED")?;
    let checkpoint = prefix.checkpoint();
    db(transaction.execute(
        "DELETE FROM ONLY control.project_registry_commands WHERE project_id=$1",
        &[&project],
    ))?;
    db(transaction.execute(
        "DELETE FROM ONLY control.project_registry_identity_reservations WHERE project_id=$1",
        &[&project],
    ))?;
    db(transaction.execute(
        "DELETE FROM ONLY control.project_registry_projects WHERE project_id=$1",
        &[&project],
    ))?;
    // A suffix introduced no observations referenced by an earlier command.
    db(transaction.execute("DELETE FROM ONLY control.project_registry_observations o WHERE NOT EXISTS(SELECT 1 FROM control.project_registry_commands c WHERE c.observation_digest=o.observation_digest) AND NOT EXISTS(SELECT 1 FROM control.project_registry_projects p WHERE p.accepted_observation_digest=o.observation_digest OR p.pending_observation_digest=o.observation_digest OR p.authority_observation_digest=o.observation_digest)",&[]))?;
    let number = |n| i64::try_from(n).map_err(|_| "PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED");
    db(transaction.execute("UPDATE ONLY control.project_registry_state SET command_ordinal=$1,observation_count=$2,project_count=$3,command_count=$4,reservation_count=$5,retained_bytes=$6,checkpoint_digest=decode($7,'hex') WHERE singleton AND stage_command_id IS NULL",&[
        &number(checkpoint.command_ordinal())?,&number(checkpoint.observation_count())?,&number(checkpoint.project_count())?,&number(checkpoint.command_count())?,&number(checkpoint.reservation_count())?,&number(checkpoint.retained_bytes())?,&checkpoint.checkpoint_digest().as_str()]))?;
    let readback = load_registry_for_maintenance(transaction, target)
        .map_err(|_| "PROJECT_PURGE_READBACK_FAILED")?;
    if &readback != prefix {
        return Err("PROJECT_PURGE_READBACK_FAILED");
    }
    let tables = all_tables(transaction)?;
    let mut survivors = snapshots(transaction, &tables)?;
    survivors.retain(|table, _| !table.starts_with("control.project_registry_"));
    if digest(&serde_json::to_vec(&survivors).map_err(|_| "PROJECT_PURGE_SERIALIZATION")?)
        != plan.survivor_digest
    {
        return Err("PROJECT_PURGE_SURVIVOR_CHANGED");
    }
    Ok(())
}

fn replay_receipt<C: GenericClient>(
    client: &mut C,
    request: &Value,
    request_digest: &str,
    result: Value,
) -> Result<Value> {
    let operation = text(request, "operationId")?;
    let project = text(request, "projectId")?;
    let scope = text(&result, "scopeDigest")?;
    if digest(format!("{operation}\n{project}\n{scope}").as_bytes()) != request_digest {
        return Err("PROJECT_PURGE_IDEMPOTENCY_CONFLICT");
    }
    let tables = all_tables(client)?;
    let current = digest(
        &serde_json::to_vec(&snapshots(client, &tables)?)
            .map_err(|_| "PROJECT_PURGE_SERIALIZATION")?,
    );
    if current != text(&result, "afterDigest")? {
        return Err("PROJECT_PURGE_READBACK_CHANGED");
    }
    Ok(result)
}

/// Preview never installs or mutates. Apply requires an exact reviewed digest.
/// The Store must be in its existing STOPPED maintenance admission, never forced
/// here; this command changes neither daemon admission nor role grants.
///
/// # Errors
///
/// Rejects invalid inputs, unsupported schema or ownership, active admission,
/// stale scope, changed readback, or any failed transactional verification.
#[allow(clippy::too_many_lines)] // Keep transaction ordering and rollback boundaries explicit.
pub fn execute_project_purge(
    client: &mut Client,
    target: &MigrationTarget,
    request: &Value,
) -> Result<Value> {
    let object = request.as_object().ok_or("PROJECT_PURGE_INPUT_REJECTED")?;
    if object.keys().any(|k| {
        ![
            "schema",
            "action",
            "projectId",
            "operationId",
            "expectedScopeDigest",
            "authorization",
        ]
        .contains(&k.as_str())
    }) || text(request, "schema")? != "lattice.project-purge.request.v1"
    {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    let action = text(request, "action")?;
    if !["install", "preview", "apply", "status"].contains(&action) {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    verify_postgres_schema(client, target, DatabaseRole::Migrator)
        .map_err(|_| "PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED")?;
    if action == "install" {
        if text(request, "authorization")? != "INSTALL_PURGE_MAINTENANCE" {
            return Err("PROJECT_PURGE_AUTHORIZATION_REQUIRED");
        }
        let mut tx = db(client.transaction())?;
        db(tx.batch_execute("LOCK TABLE control.runtime_admission IN EXCLUSIVE MODE"))?;
        crate::postgres_setup::verify_stopped_admission(&mut tx)
            .map_err(|_| "PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED")?;
        let exists: bool =
            db(tx.query_one("SELECT to_regnamespace('project_purge') IS NOT NULL", &[]))?.get(0);
        if !exists {
            db(tx.batch_execute(PROJECT_PURGE_SQL))?;
            db(tx.execute(
                "INSERT INTO project_purge.identity VALUES(true,$1)",
                &[&digest(PROJECT_PURGE_SQL.as_bytes())],
            ))?;
        }
        extension(&mut tx)?;
        db(tx.commit())?;
        return Ok(
            json!({"schema":SCHEMA,"status":"INSTALLED","registryStrategy":"VERIFIED_SUFFIX_V1"}),
        );
    }
    let operation = text(request, "operationId")?;
    if !identifier(operation) {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    let mut tx = db(client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start())?;
    db(tx.batch_execute("SET LOCAL lock_timeout='2000'; SET LOCAL statement_timeout='30000'; SET LOCAL idle_in_transaction_session_timeout='30000'"))?;
    if action == "apply" || action == "status" {
        db(tx.batch_execute("LOCK TABLE control.runtime_admission IN EXCLUSIVE MODE"))?;
    }
    crate::postgres_setup::verify_stopped_admission(&mut tx)
        .map_err(|_| "PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED")?;
    extension(&mut tx)?;
    if action == "status" {
        for (schema, name) in all_tables(&mut tx)? {
            db(tx.batch_execute(&format!(
                "LOCK TABLE {}.{} IN EXCLUSIVE MODE",
                quoted(&schema),
                quoted(&name)
            )))?;
        }
        crate::postgres_setup::verify_stopped_admission(&mut tx)
            .map_err(|_| "PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED")?;
        let row = db(tx.query_opt(
            "SELECT request_digest,result FROM ONLY project_purge.receipts WHERE operation_id=$1",
            &[&operation],
        ))?;
        return match row {
            None => {
                Ok(json!({"schema":SCHEMA,"status":"UNKNOWN_OPERATION","operationId":operation}))
            }
            Some(r) => replay_receipt(&mut tx, request, &r.get::<_, String>(0), r.get(1)),
        };
    }
    let project = text(request, "projectId")?;
    ProjectId::new(project).map_err(|_| "PROJECT_PURGE_INPUT_REJECTED")?;
    if action == "apply" && text(request, "authorization")? != "ERASE_PROJECT_DATA" {
        return Err("PROJECT_PURGE_AUTHORIZATION_REQUIRED");
    }
    // Exclude simultaneous changes even in optional modules. No lock disables
    // triggers, constraints or admission; timeout leaves the transaction intact.
    let tables = all_tables(&mut tx)?;
    if action == "apply" {
        for (schema, name) in &tables {
            db(tx.batch_execute(&format!(
                "LOCK TABLE {}.{} IN EXCLUSIVE MODE",
                quoted(schema),
                quoted(name)
            )))?;
        }
        crate::postgres_setup::verify_stopped_admission(&mut tx)
            .map_err(|_| "PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED")?;
        if let Some(row) = db(tx.query_opt(
            "SELECT request_digest,result FROM ONLY project_purge.receipts WHERE operation_id=$1",
            &[&operation],
        ))? {
            let bound = digest(
                format!(
                    "{operation}\n{project}\n{}",
                    text(request, "expectedScopeDigest")?
                )
                .as_bytes(),
            );
            if row.get::<_, String>(0) != bound {
                return Err("PROJECT_PURGE_IDEMPOTENCY_CONFLICT");
            }
            return replay_receipt(&mut tx, request, &bound, row.get(1));
        }
    }
    let plan = prepare(&mut tx, target, project, operation)?;
    if action == "preview" {
        return Ok(plan.public);
    }
    if plan.public["status"] != "READY" {
        return Ok(plan.public);
    }
    let scope = text(&plan.public, "scopeDigest")?;
    if text(request, "expectedScopeDigest")? != scope {
        return Err("PROJECT_PURGE_STALE_SCOPE");
    }
    erase(&mut tx, target, project, &plan)?;
    let tables = all_tables(&mut tx)?;
    let after_digest = digest(
        &serde_json::to_vec(&snapshots(&mut tx, &tables)?)
            .map_err(|_| "PROJECT_PURGE_SERIALIZATION")?,
    );
    let result = json!({"schema":SCHEMA,"status":"PURGED","phase":"POSTGRES_ONLY","operationId":operation,"scopeDigest":scope,"afterDigest":after_digest,"registryStrategy":"VERIFIED_SUFFIX_V1","counts":plan.public["counts"],"blockers":[]});
    let binding = digest(format!("{operation}\n{project}\n{scope}").as_bytes());
    db(tx.execute("INSERT INTO project_purge.receipts(operation_id,scope_digest,request_digest,result) VALUES($1,$2,$3,$4)",&[&operation,&scope,&binding,&result]))?;
    tx.commit()
        .map_err(|_| "PROJECT_PURGE_COMMIT_OUTCOME_UNKNOWN")?;
    Ok(result)
}
