//! Bounded operator-only physical erasure for a complete Registry command suffix.
//! A successful result is deliberately PG-only; the caller must separately erase
//! its local catalog/files and cannot infer success from a project being absent.
use crate::project_purge_snapshot::SnapshotHasher;
use crate::project_registry::{load_registry_for_maintenance, load_registry_for_transition};
use crate::registry_epoch::{self, EpochMigration};
use crate::{DatabaseRole, MigrationTarget, verify_postgres_schema};
use lattice_contracts::ProjectId;
use lattice_project_registry::{
    VerifiedRegistryState, preview_required_redactions, project_purge_prefix,
};
use postgres::fallible_iterator::FallibleIterator;
use postgres::{Client, Config, GenericClient, IsolationLevel, NoTls, Transaction};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::time::{Duration, Instant};

pub const PROJECT_PURGE_SQL: &str = include_str!("../../../db/extensions/project-purge/v1.sql");
type GraphSourceResolver<'a> = dyn Fn(&str) -> Result<Vec<String>> + 'a;
type Result<T> = std::result::Result<T, &'static str>;
const SCHEMA: &str = "lattice.project-purge.result.v1";
const PARAMS: &str = " AND $1::text IS NOT NULL AND $2::text[] IS NOT NULL AND $3::text[] IS NOT NULL AND $4::text[] IS NOT NULL";

/// Read the actual Graph/Memory relations without interpreting legacy shared
/// project keys as Registry ownership. The caller first verifies the Store.
///
/// # Errors
/// Rejects unknown relations, oversized snapshots and unavailable reads.
pub fn inspect_project_purge_graph(client: &mut Client) -> Result<Value> {
    let mut tx = db(client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start())?;
    db(tx.batch_execute("SET LOCAL statement_timeout='15s'; SET LOCAL lock_timeout='2s'"))?;
    let tables: Vec<_> = all_tables(&mut tx)?
        .into_iter()
        .filter(|(schema, name)| {
            schema == "memory"
                && ![
                    "codebase_memory_extension_identity",
                    "codebase_memory_extension_ledger",
                ]
                .contains(&name.as_str())
        })
        .collect();
    let rows = stream_snapshot(&mut tx, &tables, None, None, |_| false)?;
    let counts = rows.counts;
    let empty = counts.values().all(|count| *count == 0);
    let snapshot = rows.digest;
    db(tx.commit())?;
    Ok(
        json!({"schema":"lattice.project-purge.graph-inventory.v1","ownership":"LATTICE",
        "scope":"VERIFIED_MAIN_STORE_MEMORY","discovery":if empty{"VERIFIED_EMPTY"}else{"OBSERVED"},
        "counts":counts,"snapshotDigest":snapshot,"identityBinding":if empty{"NOT_APPLICABLE"}else{"LEGACY_SOURCE_BINDING_NOT_PROVEN"},
        "erasureImplemented":false}),
    )
}

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
fn operation_key(operation: &str) -> String {
    format!(
        "epoch:{}",
        digest(format!("lattice.project-purge.operation.v1\n{operation}").as_bytes())
    )
}
fn references(value: &Value, project: &str, canonical: &str, keys: &[&String]) -> bool {
    references_normalized(value, &canonical.replace('\\', "/").to_lowercase(), &|s| {
        s.contains(project) || keys.iter().any(|key| s.contains(key.as_str()))
    })
}
fn reference_text(s: &str, normalized_path: &str, matches_key: &impl Fn(&str) -> bool) -> bool {
    matches_key(s)
        || s.replace('\\', "/").to_lowercase().contains(normalized_path)
        // JSON scalars other than strings cannot contain a nested reference.
        || (s.trim_start_matches([' ', '\t', '\r', '\n']).starts_with(['{', '[', '"'])
            && serde_json::from_str::<Value>(s).is_ok_and(|nested|
                references_normalized(&nested, normalized_path, matches_key)))
}
fn references_normalized(
    value: &Value,
    normalized_path: &str,
    matches_key: &impl Fn(&str) -> bool,
) -> bool {
    match value {
        Value::String(s) => reference_text(s, normalized_path, matches_key),
        Value::Array(values) => values
            .iter()
            .any(|v| references_normalized(v, normalized_path, matches_key)),
        Value::Object(values) => values.iter().any(|(k, v)| {
            reference_text(k, normalized_path, matches_key)
                || references_normalized(v, normalized_path, matches_key)
        }),
        _ => false,
    }
}

// Registry observations have no project_id. Only observations referenced by a
// retained command/project belong to this survivor scan; the global checkpoint
// and target-only observations are not another project's retained content.
fn registry_survivor_reference_blockers(
    physical: &BTreeMap<String, Vec<String>>,
    project: &str,
    canonical: &str,
    keys: &[&String],
) -> Result<Vec<Value>> {
    let mut blockers = Vec::new();
    let mut observation_digests = BTreeSet::new();
    for (table, observation_fields) in [
        (
            "control.project_registry_commands",
            &["observation_digest", "before_observation_digest"][..],
        ),
        (
            "control.project_registry_projects",
            &[
                "accepted_observation_digest",
                "pending_observation_digest",
                "authority_observation_digest",
            ][..],
        ),
        ("control.project_registry_identity_reservations", &[][..]),
    ] {
        let mut count = 0;
        for raw in physical
            .get(table)
            .ok_or("PROJECT_PURGE_REGISTRY_CORRUPT")?
        {
            let row: Value =
                serde_json::from_str(raw).map_err(|_| "PROJECT_PURGE_REGISTRY_CORRUPT")?;
            let owner = row["project_id"]
                .as_str()
                .ok_or("PROJECT_PURGE_REGISTRY_CORRUPT")?;
            if owner == project {
                continue;
            }
            for field in observation_fields {
                if let Some(value) = row[*field].as_str() {
                    observation_digests.insert(value.to_owned());
                }
            }
            if references(&row, project, canonical, keys) {
                count += 1;
            }
        }
        if count > 0 {
            blockers
                .push(json!({"code":"REGISTRY_SURVIVOR_REFERENCE","table":table,"count":count}));
        }
    }
    let table = "control.project_registry_observations";
    let mut count = 0;
    for raw in physical
        .get(table)
        .ok_or("PROJECT_PURGE_REGISTRY_CORRUPT")?
    {
        let row: Value = serde_json::from_str(raw).map_err(|_| "PROJECT_PURGE_REGISTRY_CORRUPT")?;
        let observation_digest = row["observation_digest"]
            .as_str()
            .ok_or("PROJECT_PURGE_REGISTRY_CORRUPT")?;
        if observation_digests.contains(observation_digest)
            && references(&row, project, canonical, keys)
        {
            count += 1;
        }
    }
    if count > 0 {
        blockers.push(json!({"code":"REGISTRY_SURVIVOR_REFERENCE","table":table,"count":count}));
    }
    Ok(blockers)
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
    epoch: Option<EpochMigration>,
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
        "registry_epoch" => "identity current_seal used_commands",
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
    // The old BTreeMap serialized table keys in Rust byte order, independent
    // of the database locale. Preserve that order before streaming the map.
    tables.sort_by_cached_key(|(schema, name)| format!("{schema}.{name}"));
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

struct SnapshotSelection<'a> {
    predicates: &'a [(String, String)],
    params: &'a [&'a (dyn postgres::types::ToSql + Sync)],
}
struct StreamedSnapshot {
    digest: String,
    scope_digest: Option<String>,
    survivor_digest: String,
    counts: BTreeMap<String, u64>,
    selected_counts: BTreeMap<String, u64>,
    blockers: Vec<Value>,
}

fn snapshot_statement_budget(started: Instant, original_ms: i64) -> Result<String> {
    let remaining = Duration::from_mins(1)
        .checked_sub(started.elapsed())
        .ok_or("PROJECT_PURGE_SNAPSHOT_DEADLINE_EXCEEDED")?;
    let millis = i64::try_from(remaining.as_millis())
        .map_err(|_| "PROJECT_PURGE_SNAPSHOT_DEADLINE_EXCEEDED")?;
    if millis == 0 {
        return Err("PROJECT_PURGE_SNAPSHOT_DEADLINE_EXCEEDED");
    }
    Ok(if original_ms > 0 {
        millis.min(original_ms)
    } else {
        millis
    }
    .to_string())
}

fn set_snapshot_budget<C: GenericClient>(
    client: &mut C,
    started: Instant,
    original_ms: i64,
) -> Result<()> {
    let budget = snapshot_statement_budget(started, original_ms)?;
    db(client.query_one("SELECT set_config('statement_timeout',$1,true)", &[&budget]))?;
    Ok(())
}

// Stream sorted row strings through the exact old JSON serialization. Only a
// single row payload is interpreted at a time; cardinality and total bytes no
// longer bound unrelated surviving projects. An oversized individual row and
// PostgreSQL's existing statement/lock deadlines still fail closed.
#[allow(clippy::too_many_lines)]
fn stream_snapshot<C: GenericClient>(
    client: &mut C,
    tables: &[(String, String)],
    scope: Option<&Value>,
    selection: Option<&SnapshotSelection<'_>>,
    mut has_reference: impl FnMut(&str) -> bool,
) -> Result<StreamedSnapshot> {
    let started = Instant::now();
    let original_timeout: i64 = db(client.query_one(
        "SELECT setting::bigint FROM pg_settings WHERE name='statement_timeout'",
        &[],
    ))?
    .get(0);
    let mut physical = SnapshotHasher::new(scope)?;
    let mut survivors = SnapshotHasher::new(None)?;
    let mut counts = BTreeMap::new();
    let mut selected_counts = BTreeMap::new();
    let mut blockers = Vec::new();
    for (schema, name) in tables {
        if schema == "project_purge" {
            continue;
        }
        let table = format!("{schema}.{name}");
        let retained_table = selection.is_some()
            && !table.starts_with("control.project_registry_")
            && schema != "registry_epoch";
        let predicate =
            selection.and_then(|value| value.predicates.iter().find(|(key, _)| key == &table));
        let (condition, parameters) =
            predicate.map_or(("false", ""), |(_, value)| (value.as_str(), PARAMS));
        let params = if predicate.is_some() {
            selection.map_or(&[][..], |value| value.params)
        } else {
            &[][..]
        };
        // OFFSET 0 is a planner barrier: serialize each row once, rather than
        // expanding to_jsonb separately for the size guard, output and sort.
        let query = format!(
            "SELECT CASE WHEN octet_length(source.value)<=67108864 THEN source.value ELSE NULL END,source.selected FROM (SELECT to_jsonb(p)::text value,({condition}) IS TRUE selected FROM ONLY {}.{} p WHERE true{parameters} OFFSET 0) source ORDER BY source.value COLLATE \"C\"",
            quoted(schema),
            quoted(name)
        );
        set_snapshot_budget(client, started, original_timeout)?;
        db(client.execute(
            &format!("DECLARE lattice_purge_rows NO SCROLL CURSOR FOR {query}"),
            params,
        ))?;
        physical.table(&table)?;
        if retained_table {
            survivors.table(&table)?;
        }
        let mut count = 0u64;
        let mut selected = 0u64;
        let mut references = 0u64;
        loop {
            set_snapshot_budget(client, started, original_timeout)?;
            let mut rows = db(client.query_raw(
                "FETCH FORWARD 512 FROM lattice_purge_rows",
                std::iter::empty::<&(dyn postgres::types::ToSql + Sync)>(),
            ))?;
            let mut fetched = 0usize;
            while let Some(row) = db(rows.next())? {
                if started.elapsed() > Duration::from_mins(1) {
                    return Err("PROJECT_PURGE_SNAPSHOT_DEADLINE_EXCEEDED");
                }
                fetched += 1;
                let raw = row
                    .get::<_, Option<String>>(0)
                    .ok_or("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?;
                let is_selected: bool = row.get(1);
                physical.row(&raw)?;
                count = count
                    .checked_add(1)
                    .ok_or("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?;
                if is_selected {
                    selected = selected
                        .checked_add(1)
                        .ok_or("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?;
                } else if retained_table {
                    survivors.row(&raw)?;
                    if has_reference(&raw) {
                        references = references
                            .checked_add(1)
                            .ok_or("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?;
                    }
                }
            }
            if fetched < 512 {
                break;
            }
        }
        set_snapshot_budget(client, started, original_timeout)?;
        db(client.batch_execute("CLOSE lattice_purge_rows"))?;
        physical.end_table();
        if retained_table {
            survivors.end_table();
        }
        counts.insert(table.clone(), count);
        if predicate.is_some() {
            selected_counts.insert(table.clone(), selected);
        }
        if references > 0 {
            blockers.push(json!({"code":"CROSS_SCOPE_OR_UNSUPPORTED_REFERENCE","table":table,"count":references}));
        }
    }
    snapshot_statement_budget(started, original_timeout)?;
    db(client.query_one(
        "SELECT set_config('statement_timeout',$1,true)",
        &[&original_timeout.to_string()],
    ))?;
    let (digest, scope_digest) = physical.finish();
    Ok(StreamedSnapshot {
        digest,
        scope_digest,
        survivor_digest: survivors.finish().0,
        counts,
        selected_counts,
        blockers,
    })
}

fn snapshot_digest<C: GenericClient>(
    client: &mut C,
    tables: &[(String, String)],
) -> Result<String> {
    Ok(stream_snapshot(client, tables, None, None, |_| false)?.digest)
}

#[allow(clippy::too_many_lines)]
fn prepare<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    project: &str,
    operation: &str,
    epoch_policy: bool,
    resume_scope: Option<&str>,
    graph_source: Option<&GraphSourceResolver<'_>>,
) -> Result<Plan> {
    let id = ProjectId::new(project).map_err(|_| "PROJECT_PURGE_INPUT_REJECTED")?;
    let database = target.expected_database_identity_sha256().as_str();
    let state = if let Some(scope) = resume_scope {
        let binding = digest(format!("{operation}\n{project}\n{scope}").as_bytes());
        let epoch = registry_epoch::load_epoch_for_resume(client, database, &binding)?;
        load_registry_for_transition(client, target, epoch)
    } else {
        load_registry_for_maintenance(client, target)
    }
    .map_err(|_| "PROJECT_PURGE_REGISTRY_CORRUPT")?;
    let projection = state
        .project(&id)
        .ok_or("PROJECT_PURGE_PROJECT_NOT_FOUND")?;
    let canonical = projection.observation().canonical_root();
    let mut protected_roots: Vec<String> = db(client.query("SELECT o.canonical_root FROM control.project_registry_projects p JOIN control.project_registry_observations o ON o.observation_digest=p.accepted_observation_digest WHERE p.project_id<>$1 ORDER BY p.project_id", &[&project]))?.into_iter().map(|r|r.get(0)).collect();
    let prefix = if !epoch_policy && state.epoch() == 0 {
        project_purge_prefix(&state, &id).ok()
    } else {
        None
    };
    let mut blockers = Vec::<Value>::new();
    if prefix.is_none() && !epoch_policy {
        blockers.push(
            json!({"code":"REGISTRY_INTERLEAVED_HISTORY_REQUIRES_MIGRATION",
            "detail":"REGISTRY_COMPACTION_TRUST_ROOT_REQUIRED"}),
        );
    }
    let streams: Vec<String> = db(client.query("SELECT encode(stream_id,'hex') FROM ONLY control.task_ledger_streams WHERE project_id=$1 ORDER BY stream_id", &[&project]))?.into_iter().map(|r| r.get(0)).collect();
    let tasks: Vec<String> = db(client.query("SELECT task_ref::text FROM ONLY control.task_submission_envelopes WHERE project_id=$1 ORDER BY task_ref", &[&project]))?.into_iter().map(|r| r.get(0)).collect();
    let tables = all_tables(client)?;
    let table_names: BTreeSet<String> = tables
        .iter()
        .map(|(schema, name)| format!("{schema}.{name}"))
        .collect();
    let mut graph_groups = BTreeMap::<String, u64>::new();
    if table_names.contains("memory.codebase_memory_analyses") {
        let mut rows = db(client.query_raw("SELECT encode(configuration_digest,'hex'),count(*)::bigint FROM ONLY memory.codebase_memory_analyses GROUP BY configuration_digest", std::iter::empty::<&(dyn postgres::types::ToSql + Sync)>()))?;
        while let Some(row) = db(rows.next())? {
            graph_groups.insert(
                row.get(0),
                u64::try_from(row.get::<_, i64>(1))
                    .map_err(|_| "PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?,
            );
            if graph_groups.len() > 100_000 {
                return Err("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED");
            }
        }
    }
    let graph_rows = graph_groups
        .values()
        .try_fold(0u64, |sum, count| sum.checked_add(*count))
        .ok_or("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?;
    let mut graph_configurations = Vec::new();
    let mut survivor_graph_configurations = Vec::new();
    let mut graph_proof = json!(null);
    if graph_rows > 0 {
        if let Some(resolve) = graph_source {
            match resolve(canonical) {
                Ok(mut values)
                    if !values.is_empty()
                        && values.len() <= 16
                        && values.iter().all(|value| {
                            value.len() == 64
                                && value
                                    .bytes()
                                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        }) =>
                {
                    values.sort();
                    values.dedup();
                    // Recompute every surviving source too: two lexical paths
                    // can canonicalize to one directory on the actual platform.
                    for other in &protected_roots {
                        match resolve(other) {
                            Ok(other_values)
                                if other_values.iter().all(|value| !values.contains(value)) =>
                            {
                                survivor_graph_configurations.extend(other_values);
                            }
                            Ok(_) => {
                                blockers.push(json!({"code":"GRAPH_SOURCE_SHARED_WITH_SURVIVOR"}));
                            }
                            Err(_) => blockers.push(
                                json!({"code":"GRAPH_SURVIVOR_SOURCE_UNIQUENESS_NOT_PROVEN"}),
                            ),
                        }
                    }
                    graph_configurations = values;
                }
                _ => blockers.push(json!({"code":"GRAPH_SOURCE_CONFIGURATION_NOT_PROVEN"})),
            }
        }
        let mut target_count = 0u64;
        let mut survivor_count = 0u64;
        for (configuration, count) in &graph_groups {
            if graph_configurations.contains(configuration) {
                target_count += count;
            } else if survivor_graph_configurations.contains(configuration) {
                survivor_count += count;
            }
        }
        let gateway_count: i64 = if table_names.contains("memory.openclaw_gateway_commands") {
            db(client.query_one(
                "SELECT count(*)::bigint FROM ONLY memory.openclaw_gateway_commands",
                &[],
            ))?
            .get(0)
        } else {
            0
        };
        graph_proof = json!({"binding":if graph_configurations.is_empty(){"NOT_PROVEN"}else{"RECOMPUTED_RUNTIME_SOURCE_CONFIGURATION"},
            "configurations":graph_configurations,"targetAnalyses":target_count,"survivorAnalyses":survivor_count,
            "unattributableAnalyses":graph_rows-target_count-survivor_count,
            "unclassifiedGatewayCommands":gateway_count,
            "unmatchedDisposition":"UNATTRIBUTABLE_NOT_PROVEN_UNRELATED"});
        if graph_rows > target_count + survivor_count {
            blockers.push(json!({"code":"GRAPH_ANALYSIS_OWNERSHIP_UNATTRIBUTABLE","count":graph_rows-target_count-survivor_count}));
        }
    }
    let mut claims = Vec::<String>::new();
    let mut roots = vec![json!({"path":canonical,"source":"POSTGRES_REGISTRY_OBSERVATION"})];
    if table_names.contains("control_product.conversation_claims") {
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
    let keys: Vec<&String> = streams
        .iter()
        .chain(tasks.iter())
        .chain(claims.iter())
        .collect();
    if !epoch_policy {
        let registry_tables: Vec<_> = tables
            .iter()
            .filter(|(schema, name)| schema == "control" && name.starts_with("project_registry_"))
            .cloned()
            .collect();
        let registry_rows = snapshots(client, &registry_tables)?;
        blockers.extend(registry_survivor_reference_blockers(
            &registry_rows,
            project,
            canonical,
            &keys,
        )?);
    }
    let mut deletes = Vec::new();
    let mut predicates: Vec<(String, String)> = selectors()
        .into_iter()
        .map(|(table, predicate)| (table.into(), predicate.into()))
        .collect();
    if !graph_configurations.is_empty() {
        let values = graph_configurations
            .iter()
            .map(|value| format!("'{value}'"))
            .collect::<Vec<_>>()
            .join(",");
        let analyses = format!(
            "SELECT analysis_digest FROM memory.codebase_memory_analyses WHERE encode(configuration_digest,'hex') IN ({values})"
        );
        predicates.extend([
            ("memory.codebase_memory_reflections".into(),format!("p.graph_receipt_digest IN (SELECT r.receipt_digest FROM memory.codebase_memory_receipts r JOIN memory.codebase_memory_analyses a ON a.analysis_digest=r.analysis_digest WHERE a.analysis_digest IN ({analyses}) AND (p.project_id=a.project_id OR p.project_id=$1))")),
            ("memory.codebase_memory_receipts".into(),format!("p.analysis_digest IN ({analyses})")),
            ("memory.codebase_memory_retrieval_audits".into(),format!("p.analysis_digest IN ({analyses})")),
            ("memory.codebase_memory_records".into(),format!("p.analysis_digest IN ({analyses})")),
            ("memory.codebase_memory_analyses".into(),format!("p.analysis_digest IN ({analyses})")),
        ]);
    }
    let mut graph_reference_keys = BTreeSet::<String>::new();
    for (table, predicate) in predicates {
        if !table_names.contains(&table) {
            continue;
        }
        if table.starts_with("memory.") {
            // Only parent identities are needed before scanning surviving rows.
            // DISTINCT avoids retaining the same analysis key for every record.
            let query = format!(
                "SELECT DISTINCT reference FROM ONLY {table} p CROSS JOIN LATERAL (VALUES(to_jsonb(p)->>'analysis_digest'),(to_jsonb(p)->>'receipt_digest'),(to_jsonb(p)->>'retrieval_digest'),(to_jsonb(p)->>'reflection_receipt_digest')) keys(reference) WHERE ({predicate}){PARAMS} AND reference IS NOT NULL"
            );
            let mut rows = db(client.query_raw(&query, params.iter().copied()))?;
            while let Some(row) = db(rows.next())? {
                let value: String = row.get(0);
                if let Some(value) = value.strip_prefix("\\x") {
                    graph_reference_keys.insert(value.to_owned());
                }
                if graph_reference_keys.len() > 100_000 {
                    return Err("PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED");
                }
            }
        }
        deletes.push((table, predicate));
    }
    let mut keys = keys;
    keys.extend(graph_reference_keys.iter());
    // Preserve v2/v3/v4 scope bytes, including their full legacy rows map.
    // Serialization streams those bytes into SHA-256 without storing the map.
    let maintenance_installed = table_names.contains("project_purge.identity");
    let mut scope_value = json!({"schema":"lattice.project-purge.scope.v2","database":database,"project":project,"operation":operation,"maintenanceExtensionInstalled":maintenance_installed,"rows":null});
    if epoch_policy {
        scope_value["schema"] = json!("lattice.project-purge.scope.v3");
        scope_value["registryPolicy"] = json!("MINIMAL_ATTESTATION");
    }
    if !graph_configurations.is_empty() {
        scope_value["schema"] = json!("lattice.project-purge.scope.v4");
        scope_value["graphSourceProof"] = graph_proof.clone();
    }
    let reference_keys = aho_corasick::AhoCorasick::new(
        std::iter::once(project).chain(keys.iter().map(|key| key.as_str())),
    )
    .map_err(|_| "PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED")?;
    let normalized_path = canonical.replace('\\', "/").to_lowercase();
    let snapshot = stream_snapshot(
        client,
        &tables,
        Some(&scope_value),
        Some(&SnapshotSelection {
            predicates: &deletes,
            params: &params,
        }),
        |raw| {
            serde_json::from_str(raw).map_or(true, |value| {
                references_normalized(&value, &normalized_path, &|text| {
                    reference_keys.is_match(text)
                })
            })
        },
    )?;
    blockers.extend(snapshot.blockers);
    let scope_digest = snapshot.scope_digest.ok_or("PROJECT_PURGE_SERIALIZATION")?;
    let mut counts = snapshot.selected_counts;
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
    let mut epoch = None;
    let mut history = json!(null);
    if epoch_policy {
        if registry_epoch::optional_catalog(client)?.is_none() {
            blockers.push(json!({"code":"REGISTRY_EPOCH_EXTENSION_REQUIRED"}));
        }
        let permissions = preview_required_redactions(&state, &id)
            .map_err(|_| "REGISTRY_CURRENT_SURVIVOR_REFERENCE");
        match permissions {
            Err(code) => blockers.push(json!({"code":code})),
            Ok(permissions) => {
                let binding = digest(format!("{operation}\n{project}\n{scope_digest}").as_bytes());
                match registry_epoch::plan_migration(
                    client,
                    database,
                    &state,
                    &id,
                    &binding,
                    &permissions,
                ) {
                    Err(code) => blockers.push(json!({"code":code})),
                    Ok(migration) => {
                        let checkpoint = migration.baseline.checkpoint();
                        for (table, count) in [
                            (
                                "control.project_registry_commands",
                                state.checkpoint().command_count(),
                            ),
                            ("control.project_registry_projects", 1),
                            (
                                "control.project_registry_observations",
                                state.checkpoint().observation_count()
                                    - checkpoint.observation_count(),
                            ),
                            (
                                "control.project_registry_identity_reservations",
                                state.checkpoint().reservation_count()
                                    - checkpoint.reservation_count(),
                            ),
                        ] {
                            counts.insert(table.into(), count);
                        }
                        if references(&migration.payload, project, canonical, &keys) {
                            blockers
                                .push(json!({"code":"REGISTRY_UNCLASSIFIED_SURVIVOR_REFERENCE"}));
                        }
                        history = json!({"assurance":"ATTESTED_FROM_SEAL","newTail":"FULL_REPLAY_FROM_BASELINE","epoch":migration.next.epoch,"sealDigest":migration.next.seal_digest.as_str(),"unchangedArchivedCommands":migration.payload["commandRows"].as_array().map(Vec::len),
                            "redactedSurvivorCommands":permissions.iter().map(|permission|json!({"commandCommitment":permission.command_id_digest().as_str(),"recordSetDigest":permission.record_set_digest().as_str()})).collect::<Vec<_>>(),
                            "identifiersAreAnonymous":false,"operationIdentifierRetention":"SHA256_COMMITMENT_GUESSABLE_IF_LOW_ENTROPY","rollbackProtection":"DATABASE_ONLY_CROSS_EPOCH","sameEpochTailRollbackProtected":false,"powerLossRecoveryGuaranteed":false});
                        epoch = Some(migration);
                    }
                }
            }
        }
    }
    if epoch_policy && maintenance_installed {
        let legacy:i64=db(client.query_one("SELECT count(*)::bigint FROM ONLY project_purge.receipts WHERE result->>'operationCommitment' IS NULL",&[]))?.get(0);
        if history.is_object() {
            history["legacyMaintenanceReceiptsRequiringReview"] = json!(legacy);
        }
    }
    if let Ok(root) = registry_epoch::anchor_root(database) {
        protected_roots.push(root.to_string_lossy().into_owned());
    }
    Ok(Plan {
        public: json!({"schema":SCHEMA,"status":if blockers.is_empty(){"READY"}else{"BLOCKED"},"project":{"id":project,"canonicalPath":canonical},"operationId":operation,"scopeDigest":scope_digest,"registryStrategy":if epoch_policy{"ATTESTED_EPOCH_V1"}else{"VERIFIED_SUFFIX_V1"},"history":history,"graphSourceProof":graph_proof,"filesystemRoots":roots,"protectedRoots":protected_roots,"counts":counts,"blockers":blockers}),
        prefix,
        epoch,
        streams,
        tasks,
        claims,
        deletes,
        survivor_digest: snapshot.survivor_digest,
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
    let prefix = plan
        .epoch
        .as_ref()
        .map(|epoch| &epoch.baseline)
        .or(plan.prefix.as_ref())
        .ok_or("PROJECT_PURGE_SCOPE_BLOCKED")?;
    let checkpoint = prefix.checkpoint();
    if let Some(epoch) = &plan.epoch {
        registry_epoch::write_migration(transaction, project, epoch)?;
    } else {
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
    }
    let number = |n| i64::try_from(n).map_err(|_| "PROJECT_PURGE_SCOPE_CAPACITY_EXCEEDED");
    db(transaction.execute("UPDATE ONLY control.project_registry_state SET command_ordinal=$1,observation_count=$2,project_count=$3,command_count=$4,reservation_count=$5,retained_bytes=$6,checkpoint_digest=decode($7,'hex') WHERE singleton AND stage_command_id IS NULL",&[
        &number(checkpoint.command_ordinal())?,&number(checkpoint.observation_count())?,&number(checkpoint.project_count())?,&number(checkpoint.command_count())?,&number(checkpoint.reservation_count())?,&number(checkpoint.retained_bytes())?,&checkpoint.checkpoint_digest().as_str()]))?;
    let readback = if let Some(epoch) = &plan.epoch {
        let loaded = registry_epoch::load_epoch_for_transition(
            transaction,
            target.expected_database_identity_sha256().as_str(),
            &epoch.next,
        )?;
        load_registry_for_transition(transaction, target, loaded)
    } else {
        load_registry_for_maintenance(transaction, target)
    }
    .map_err(|_| "PROJECT_PURGE_READBACK_FAILED")?;
    if &readback != prefix {
        return Err("PROJECT_PURGE_READBACK_FAILED");
    }
    let tables: Vec<_> = all_tables(transaction)?
        .into_iter()
        .filter(|(schema, name)| {
            !format!("{schema}.{name}").starts_with("control.project_registry_")
                && schema != "registry_epoch"
        })
        .collect();
    if snapshot_digest(transaction, &tables)? != plan.survivor_digest {
        return Err("PROJECT_PURGE_SURVIVOR_CHANGED");
    }
    Ok(())
}

fn replay_receipt<C: GenericClient>(
    client: &mut C,
    target: &MigrationTarget,
    request: &Value,
    request_digest: &str,
    mut result: Value,
) -> Result<Value> {
    let operation = text(request, "operationId")?;
    let project = text(request, "projectId")?;
    if result.get("operationCommitment").is_some()
        && result["operationCommitment"] != operation_key(operation)
    {
        return Err("PROJECT_PURGE_IDEMPOTENCY_CONFLICT");
    }
    let scope = text(&result, "scopeDigest")?;
    if digest(format!("{operation}\n{project}\n{scope}").as_bytes()) != request_digest {
        return Err("PROJECT_PURGE_IDEMPOTENCY_CONFLICT");
    }
    let tables = all_tables(client)?;
    let current = snapshot_digest(client, &tables)?;
    if current != text(&result, "afterDigest")? {
        return Err("PROJECT_PURGE_READBACK_CHANGED");
    }
    if result["registryStrategy"] == "ATTESTED_EPOCH_V1" {
        registry_epoch::activate_anchor(
            client,
            target.expected_database_identity_sha256().as_str(),
            request_digest,
        )?;
        load_registry_for_maintenance(client, target)
            .map_err(|_| "PROJECT_PURGE_READBACK_FAILED")?;
    }
    result["operationId"] = json!(operation);
    Ok(result)
}

/// Preview uses a read-only snapshot even before maintenance installation or
/// while the Store is active, reporting unmet maintenance conditions as blockers.
/// Apply still requires the exact reviewed digest and existing STOPPED admission;
/// this command changes neither daemon admission nor role grants.
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
    execute_project_purge_with_graph_source(client, target, request, None)
}

/// Same maintenance transaction with a trusted native Runtime source resolver.
/// Untrusted request JSON cannot provide its own configuration digests.
///
/// # Errors
/// Has the same failure boundary as `execute_project_purge`; source drift blocks.
#[allow(clippy::too_many_lines)]
pub fn execute_project_purge_with_graph_source(
    client: &mut Client,
    target: &MigrationTarget,
    request: &Value,
    graph_source: Option<&GraphSourceResolver<'_>>,
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
            "registryPolicy",
        ]
        .contains(&k.as_str())
    }) || text(request, "schema")? != "lattice.project-purge.request.v1"
    {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    let action = text(request, "action")?;
    if !["install", "install-epoch", "preview", "apply", "status"].contains(&action) {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    let epoch_policy = match request.get("registryPolicy").and_then(Value::as_str) {
        None => false,
        Some("MINIMAL_ATTESTATION") => true,
        Some(_) => return Err("PROJECT_PURGE_INPUT_REJECTED"),
    };
    if request
        .get("registryPolicy")
        .is_some_and(|value| !value.is_string())
    {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    if action == "preview" {
        crate::postgres_setup::verify_project_purge_inventory_schema(client, target)
            .map_err(|_| "PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED")?;
    } else {
        verify_postgres_schema(client, target, DatabaseRole::Migrator)
            .map_err(|_| "PROJECT_PURGE_MAINTENANCE_PROFILE_REQUIRED")?;
    }
    if action == "install" || action == "install-epoch" {
        let authorization = if action == "install" {
            "INSTALL_PURGE_MAINTENANCE"
        } else {
            "INSTALL_REGISTRY_EPOCH_MAINTENANCE"
        };
        if text(request, "authorization")? != authorization {
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
        if action == "install-epoch" {
            let installed: bool =
                db(tx.query_one("SELECT to_regnamespace('registry_epoch') IS NOT NULL", &[]))?
                    .get(0);
            if !installed {
                db(tx.batch_execute(registry_epoch::REGISTRY_EPOCH_SQL))?;
                db(tx.execute(
                    "INSERT INTO registry_epoch.identity VALUES(true,$1)",
                    &[&registry_epoch::digest(
                        registry_epoch::REGISTRY_EPOCH_SQL.as_bytes(),
                    )],
                ))?;
            }
            registry_epoch::optional_catalog(&mut tx)?
                .ok_or("REGISTRY_EPOCH_EXTENSION_REQUIRED")?;
        }
        db(tx.commit())?;
        return Ok(
            json!({"schema":SCHEMA,"status":"INSTALLED","registryStrategy":if action=="install-epoch"{"ATTESTED_EPOCH_V1"}else{"VERIFIED_SUFFIX_V1"}}),
        );
    }
    let operation = text(request, "operationId")?;
    if !identifier(operation) {
        return Err("PROJECT_PURGE_INPUT_REJECTED");
    }
    let mut tx = db(client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .read_only(action == "preview")
        .deferrable(action == "preview")
        .start())?;
    db(tx.batch_execute("SET LOCAL lock_timeout='2000'; SET LOCAL statement_timeout='30000'; SET LOCAL idle_in_transaction_session_timeout='30000'"))?;
    if action == "preview" {
        // Absence is allowed only for inventory. An installed extension must
        // still pass its exact catalog and identity checks, never be ignored.
        let installed: bool =
            db(tx.query_one("SELECT to_regnamespace('project_purge') IS NOT NULL", &[]))?.get(0);
        if installed {
            extension(&mut tx)?;
        }
        let stopped: bool = db(tx.query_one("SELECT admission_mode='STOPPED' AND daemon_instance_id IS NULL AND daemon_epoch IS NULL AND authority_revision=0 AND observation_digest IS NULL AND authority_head_digest IS NULL FROM ONLY control.runtime_admission WHERE singleton", &[]))?.get(0);
        let mut plan = prepare(
            &mut tx,
            target,
            text(request, "projectId")?,
            operation,
            epoch_policy,
            request.get("expectedScopeDigest").and_then(Value::as_str),
            graph_source,
        )?;
        let blockers = plan.public["blockers"]
            .as_array_mut()
            .ok_or("PROJECT_PURGE_SERIALIZATION")?;
        if !installed {
            blockers.push(json!({"code":"MAINTENANCE_EXTENSION_REQUIRED"}));
        }
        if !stopped {
            blockers.push(json!({"code":"MAINTENANCE_OFFLINE_REQUIRED"}));
        }
        if !blockers.is_empty() {
            plan.public["status"] = json!("BLOCKED");
        }
        plan.public["inventoryMode"] = json!("READ_ONLY_SNAPSHOT");
        plan.public["maintenanceExtensionInstalled"] = json!(installed);
        plan.public["maintenanceStopped"] = json!(stopped);
        return Ok(plan.public);
    }
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
            "SELECT request_digest,result FROM ONLY project_purge.receipts WHERE operation_id=$1 OR operation_id=$2",
            &[&operation,&operation_key(operation)],
        ))?;
        return match row {
            None => {
                Ok(json!({"schema":SCHEMA,"status":"UNKNOWN_OPERATION","operationId":operation}))
            }
            Some(r) => replay_receipt(&mut tx, target, request, &r.get::<_, String>(0), r.get(1)),
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
            "SELECT request_digest,result FROM ONLY project_purge.receipts WHERE operation_id=$1 OR operation_id=$2",
            &[&operation,&operation_key(operation)],
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
            return replay_receipt(&mut tx, target, request, &bound, row.get(1));
        }
    }
    let plan = prepare(
        &mut tx,
        target,
        project,
        operation,
        epoch_policy,
        Some(text(request, "expectedScopeDigest")?),
        graph_source,
    )?;
    if plan.public["status"] != "READY" {
        return Ok(plan.public);
    }
    let scope = text(&plan.public, "scopeDigest")?;
    if text(request, "expectedScopeDigest")? != scope {
        return Err("PROJECT_PURGE_STALE_SCOPE");
    }
    if let Some(epoch) = &plan.epoch {
        registry_epoch::prepare_anchor(target.expected_database_identity_sha256().as_str(), epoch)?;
    }
    erase(&mut tx, target, project, &plan)?;
    let tables = all_tables(&mut tx)?;
    let after_digest = snapshot_digest(&mut tx, &tables)?;
    let mut result = json!({"schema":SCHEMA,"status":"PURGED","phase":"POSTGRES_ONLY","operationId":operation,"scopeDigest":scope,"afterDigest":after_digest,"registryStrategy":plan.public["registryStrategy"],"counts":plan.public["counts"],"blockers":[]});
    result["graphSourceProof"] = plan.public["graphSourceProof"].clone();
    if let Some(epoch) = &plan.epoch {
        result["registrySealDigest"] = json!(epoch.next.seal_digest.as_str());
        result["history"] = plan.public["history"].clone();
        result["operationCommitment"] = json!(operation_key(operation));
    }
    let binding = digest(format!("{operation}\n{project}\n{scope}").as_bytes());
    let retained_key = if plan.epoch.is_some() {
        operation_key(operation)
    } else {
        operation.to_owned()
    };
    let mut retained_result = result.clone();
    retained_result["operationId"] = json!(retained_key);
    db(tx.execute("INSERT INTO project_purge.receipts(operation_id,scope_digest,request_digest,result) VALUES($1,$2,$3,$4)",&[&retained_key,&scope,&binding,&retained_result]))?;
    tx.commit()
        .map_err(|_| "PROJECT_PURGE_COMMIT_OUTCOME_UNKNOWN")?;
    if plan.epoch.is_some() {
        registry_epoch::activate_anchor(
            client,
            target.expected_database_identity_sha256().as_str(),
            &binding,
        )?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{references, registry_survivor_reference_blockers};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    #[test]
    fn snapshot_budget_never_disables_or_extends_the_statement_deadline() {
        let now = std::time::Instant::now();
        assert_eq!(
            super::snapshot_statement_budget(now, 15_000).unwrap(),
            "15000"
        );
        let nearly_expired = now.checked_sub(std::time::Duration::from_secs(59)).unwrap();
        let remaining: u64 = super::snapshot_statement_budget(nearly_expired, 30_000)
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=1_000).contains(&remaining));
        assert!(
            super::snapshot_statement_budget(
                now.checked_sub(std::time::Duration::from_mins(1)).unwrap(),
                30_000
            )
            .is_err()
        );
        let bounded: u64 = super::snapshot_statement_budget(now, 0)
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=60_000).contains(&bounded));
    }

    #[test]
    fn compiled_reference_keys_preserve_contains_existence_semantics() {
        for patterns in [
            vec![],
            vec![""],
            vec!["abc", "abc", "bc", "繁體"],
            vec!["target-ID", "AbC"],
        ] {
            let matcher = aho_corasick::AhoCorasick::new(&patterns).unwrap();
            for text in [
                "",
                "abc",
                "ABC",
                "zabc",
                "target-id",
                "target-ID",
                "繁體字",
                "\\u0061bc",
            ] {
                assert_eq!(
                    matcher.is_match(text),
                    patterns.iter().any(|pattern| text.contains(pattern))
                );
            }
        }
    }

    #[test]
    fn normalized_reference_scan_preserves_legacy_nested_json_semantics() {
        fn legacy(value: &Value, project: &str, canonical: &str, keys: &[&String]) -> bool {
            match value {
                Value::String(s) => {
                    s.contains(project)
                        || s.replace('\\', "/")
                            .to_lowercase()
                            .contains(&canonical.replace('\\', "/").to_lowercase())
                        || keys.iter().any(|key| s.contains(key.as_str()))
                        || serde_json::from_str::<Value>(s)
                            .is_ok_and(|nested| legacy(&nested, project, canonical, keys))
                }
                Value::Array(values) => values
                    .iter()
                    .any(|value| legacy(value, project, canonical, keys)),
                Value::Object(values) => values.iter().any(|(key, value)| {
                    legacy(&Value::String(key.clone()), project, canonical, keys)
                        || legacy(value, project, canonical, keys)
                }),
                _ => false,
            }
        }
        let key = "f".repeat(64);
        let matcher = aho_corasick::AhoCorasick::new(["target-id", key.as_str()]).unwrap();
        for value in [
            json!(null),
            json!(true),
            json!(18_446_744_073_709_551_615_u64),
            json!(0.125),
            json!("ordinary field"),
            json!("123"),
            json!("false"),
            json!(r#"  {"value":"\u0074arget-id"}"#),
            json!({"nested":["C:/PROJECT/İ/src.rs", {"value":key}]}),
            json!({"target-id":"unrelated"}),
            json!("[malformed"),
        ] {
            let mut nested = value;
            for _ in 0..4 {
                assert_eq!(
                    references(&nested, "target-id", "C:\\Project\\İ", &[&key]),
                    legacy(&nested, "target-id", "C:\\Project\\İ", &[&key])
                );
                assert_eq!(
                    super::references_normalized(
                        &nested,
                        &"C:\\Project\\İ".replace('\\', "/").to_lowercase(),
                        &|s| matcher.is_match(s)
                    ),
                    legacy(&nested, "target-id", "C:\\Project\\İ", &[&key])
                );
                nested = Value::String(nested.to_string());
            }
        }
    }

    fn registry_rows() -> BTreeMap<String, Vec<String>> {
        BTreeMap::from([
            ("control.project_registry_commands".into(), vec![
                json!({"project_id":"target-id","observation_digest":"target-observation"}).to_string(),
                json!({"project_id":"survivor-id","observation_digest":"survivor-observation"}).to_string(),
            ]),
            ("control.project_registry_projects".into(), vec![
                json!({"project_id":"target-id","accepted_observation_digest":"target-observation"}).to_string(),
                json!({"project_id":"survivor-id","accepted_observation_digest":"survivor-observation"}).to_string(),
            ]),
            ("control.project_registry_identity_reservations".into(), vec![
                json!({"project_id":"target-id"}).to_string(),
                json!({"project_id":"survivor-id"}).to_string(),
            ]),
            ("control.project_registry_observations".into(), vec![
                json!({"observation_digest":"target-observation","canonical_root":"C:/fixture/target"}).to_string(),
                json!({"observation_digest":"survivor-observation","canonical_root":"C:/fixture/survivor"}).to_string(),
            ]),
            ("control.project_registry_state".into(), vec![
                json!({"aggregate":"target-id"}).to_string(),
            ]),
        ])
    }

    #[test]
    fn registry_scan_excludes_target_owned_rows_and_the_global_checkpoint() {
        let physical = registry_rows();
        assert!(
            registry_survivor_reference_blockers(&physical, "target-id", "C:/fixture/target", &[])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn registry_scan_counts_survivor_denial_and_shared_observation_without_content() {
        let mut physical = registry_rows();
        physical
            .get_mut("control.project_registry_commands")
            .unwrap()
            .push(
                json!({"project_id":"survivor-id","denial_existing_project_id":"target-id",
                "observation_digest":"target-observation"})
                .to_string(),
            );
        let original = physical.clone();
        assert_eq!(
            registry_survivor_reference_blockers(&physical, "target-id", "C:/fixture/target", &[])
                .unwrap(),
            vec![
                json!({"code":"REGISTRY_SURVIVOR_REFERENCE","table":"control.project_registry_commands","count":1}),
                json!({"code":"REGISTRY_SURVIVOR_REFERENCE","table":"control.project_registry_observations","count":1}),
            ]
        );
        assert_eq!(physical, original);
    }

    #[test]
    fn registry_scan_follows_each_survivor_observation_reference() {
        for (table, field) in [
            (
                "control.project_registry_commands",
                "before_observation_digest",
            ),
            (
                "control.project_registry_projects",
                "accepted_observation_digest",
            ),
            (
                "control.project_registry_projects",
                "pending_observation_digest",
            ),
            (
                "control.project_registry_projects",
                "authority_observation_digest",
            ),
        ] {
            let mut physical = registry_rows();
            let row = json!({"project_id":"survivor-id",field:"target-observation"});
            physical.get_mut(table).unwrap().push(row.to_string());
            assert_eq!(
                registry_survivor_reference_blockers(
                    &physical,
                    "target-id",
                    "C:/fixture/target",
                    &[]
                )
                .unwrap(),
                vec![
                    json!({"code":"REGISTRY_SURVIVOR_REFERENCE","table":"control.project_registry_observations","count":1}),
                ]
            );
        }
    }

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
