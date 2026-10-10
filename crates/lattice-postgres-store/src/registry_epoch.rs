//! Explicit historical attestation after project erasure. The database never
//! supplies its own trust root; a host-owned external anchor pins the seal.
use crate::registry_epoch_anchor::{
    ActiveAnchor, AnchorDigest, AnchorState, PendingAnchor, RegistryEpochAnchor,
};
use lattice_cjson::{CanonicalValue, canonicalize};
use lattice_contracts::{ContentDigest, ProjectId};
use lattice_project_registry::{
    RegistryEpochBaseline, RegistryEpochPlan, RegistryRedactionAuthorization,
    VerifiedRegistryState, export_untrusted_registry_snapshot, plan_registry_epoch,
    registry_command_id_commitment, verify_registry_epoch_baseline,
};
use postgres::GenericClient;
use serde_json::Value;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::PathBuf;

pub const REGISTRY_EPOCH_SQL: &str = include_str!("../../../db/extensions/registry-epoch/v1.sql");
type Result<T> = std::result::Result<T, &'static str>;
const REJECTED: &str = "REGISTRY_EPOCH_CATALOG_REJECTED";

pub(crate) struct EpochCatalog {
    pub relation_oids: Vec<i64>,
    pub function_oids: Vec<i64>,
}

fn query<T>(result: std::result::Result<T, postgres::Error>) -> Result<T> {
    result.map_err(|_| REJECTED)
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        })
}

/// Whitelist only a complete exact extension. An unexpected trigger, privilege,
/// function body, type or relation is an error, not an absent capability.
#[allow(clippy::too_many_lines)] // Exact catalog validation is kept together for auditability.
pub(crate) fn optional_catalog<C: GenericClient>(client: &mut C) -> Result<Option<EpochCatalog>> {
    let exists: bool =
        query(client.query_one("SELECT to_regnamespace('registry_epoch') IS NOT NULL", &[]))?
            .get(0);
    if !exists {
        return Ok(None);
    }
    let relations = query(client.query("SELECT c.oid::bigint,c.relname,c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='registry_epoch' ORDER BY c.relname", &[]))?;
    let actual: Vec<(String, String)> = relations.iter().map(|r| (r.get(1), r.get(2))).collect();
    let expected: Vec<(String, String)> = [
        ("current_seal", "r"),
        ("current_seal_pkey", "i"),
        ("identity", "r"),
        ("identity_pkey", "i"),
        ("used_commands", "r"),
        ("used_commands_pkey", "i"),
    ]
    .into_iter()
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .collect();
    if actual != expected {
        return Err(REJECTED);
    }
    let safe: bool = query(client.query_one("SELECT pg_get_userbyid(n.nspowner)='lattice_migrator'
        AND NOT EXISTS(SELECT 1 FROM aclexplode(COALESCE(n.nspacl,acldefault('n',n.nspowner))) a WHERE a.grantee<>n.nspowner AND NOT(a.grantee=(SELECT oid FROM pg_roles WHERE rolname='lattice_runtime') AND a.privilege_type='USAGE' AND NOT a.is_grantable))
        AND has_schema_privilege('lattice_runtime',n.oid,'USAGE')
        AND NOT EXISTS(SELECT 1 FROM pg_class c WHERE c.relnamespace=n.oid AND (pg_get_userbyid(c.relowner)<>'lattice_migrator' OR c.relpersistence<>'p' OR c.relrowsecurity OR c.relforcerowsecurity OR c.relispartition OR c.relhassubclass OR EXISTS(SELECT 1 FROM aclexplode(COALESCE(c.relacl,acldefault('r',c.relowner))) a WHERE a.grantee<>c.relowner)))
        AND NOT EXISTS(SELECT 1 FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_type t ON t.oid=a.atttypid WHERE c.relnamespace=n.oid AND (a.attacl IS NOT NULL OR a.attisdropped OR a.attgenerated<>'' OR a.attidentity<>'' OR a.attcollation<>t.typcollation))
        AND NOT EXISTS(SELECT 1 FROM pg_attrdef a JOIN pg_class c ON c.oid=a.adrelid WHERE c.relnamespace=n.oid)
        AND NOT EXISTS(SELECT 1 FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid WHERE c.relnamespace=n.oid)
        AND NOT EXISTS(SELECT 1 FROM pg_policy p JOIN pg_class c ON c.oid=p.polrelid WHERE c.relnamespace=n.oid)
        AND NOT EXISTS(SELECT 1 FROM pg_rewrite r JOIN pg_class c ON c.oid=r.ev_class WHERE c.relnamespace=n.oid)
        AND NOT EXISTS(SELECT 1 FROM pg_inherits i JOIN pg_class c ON c.oid=i.inhrelid OR c.oid=i.inhparent WHERE c.relnamespace=n.oid)
        AND NOT EXISTS(SELECT 1 FROM pg_type t WHERE t.typnamespace=n.oid AND (pg_get_userbyid(t.typowner)<>'lattice_migrator' OR t.typacl IS NOT NULL OR NOT t.typisdefined OR NOT(t.typtype='c' AND t.typname IN('current_seal','identity','used_commands') OR t.typtype='b' AND t.typname IN('_current_seal','_identity','_used_commands') AND t.typelem<>0)))
        FROM pg_namespace n WHERE n.nspname='registry_epoch'", &[]))?.get(0);
    if !safe {
        return Err(REJECTED);
    }
    let columns: Vec<String> = query(client.query("SELECT c.relname||'.'||a.attname||':'||format_type(a.atttypid,a.atttypmod)||':'||a.attnotnull::text FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='registry_epoch' AND c.relkind='r' AND a.attnum>0 ORDER BY c.relname,a.attnum", &[]))?.iter().map(|r|r.get(0)).collect();
    if columns
        != [
            "current_seal.singleton:boolean:true",
            "current_seal.epoch:bigint:true",
            "current_seal.seal_digest:text:true",
            "current_seal.payload:jsonb:true",
            "identity.singleton:boolean:true",
            "identity.sql_sha256:text:true",
            "used_commands.command_commitment:text:true",
            "used_commands.disposition:text:true",
        ]
    {
        return Err(REJECTED);
    }
    let constraints: Vec<String> = query(client.query("SELECT c.relname||'.'||k.conname||':'||pg_get_constraintdef(k.oid,false) FROM pg_constraint k JOIN pg_class c ON c.oid=k.conrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='registry_epoch' ORDER BY c.relname,k.conname", &[]))?.iter().map(|r|r.get(0)).collect();
    if constraints
        != [
            "current_seal.current_seal_epoch_check:CHECK ((epoch > 0))",
            "current_seal.current_seal_payload_check:CHECK ((jsonb_typeof(payload) = 'object'::text))",
            "current_seal.current_seal_pkey:PRIMARY KEY (singleton)",
            "current_seal.current_seal_seal_digest_check:CHECK ((seal_digest ~ '^[a-f0-9]{64}$'::text))",
            "current_seal.current_seal_singleton_check:CHECK (singleton)",
            "identity.identity_pkey:PRIMARY KEY (singleton)",
            "identity.identity_singleton_check:CHECK (singleton)",
            "identity.identity_sql_sha256_check:CHECK ((sql_sha256 ~ '^[a-f0-9]{64}$'::text))",
            "used_commands.used_commands_command_commitment_check:CHECK ((command_commitment ~ '^[a-f0-9]{64}$'::text))",
            "used_commands.used_commands_disposition_check:CHECK ((disposition = ANY (ARRAY['ARCHIVED'::text, 'REDACTED'::text])))",
            "used_commands.used_commands_pkey:PRIMARY KEY (command_commitment)",
        ]
    {
        return Err(REJECTED);
    }
    let valid: bool = query(client.query_one("SELECT NOT EXISTS(SELECT 1 FROM pg_constraint k JOIN pg_namespace n ON n.oid=k.connamespace WHERE n.nspname='registry_epoch' AND (NOT k.convalidated OR k.condeferrable)) AND (SELECT count(*)=3 AND bool_and(i.indisvalid AND i.indisready AND i.indisprimary AND i.indisunique AND i.indexprs IS NULL AND i.indpred IS NULL) FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='registry_epoch')", &[]))?.get(0);
    if !valid {
        return Err(REJECTED);
    }
    let indexes:Vec<String>=query(client.query("SELECT pg_get_indexdef(c.oid) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='registry_epoch' AND c.relkind='i' ORDER BY c.relname",&[]))?.iter().map(|r|r.get(0)).collect();
    if indexes
        != [
            "CREATE UNIQUE INDEX current_seal_pkey ON registry_epoch.current_seal USING btree (singleton)",
            "CREATE UNIQUE INDEX identity_pkey ON registry_epoch.identity USING btree (singleton)",
            "CREATE UNIQUE INDEX used_commands_pkey ON registry_epoch.used_commands USING btree (command_commitment)",
        ]
    {
        return Err(REJECTED);
    }
    let functions = query(client.query("SELECT p.oid::bigint,p.proname,p.prosrc,p.prosecdef,p.proisstrict,p.provolatile::text,pg_get_function_identity_arguments(p.oid),pg_get_function_result(p.oid),p.proconfig,pg_get_userbyid(p.proowner),l.lanname,p.proleakproof,p.proretset,p.prokind::text,p.probin,
        EXISTS(SELECT 1 FROM aclexplode(COALESCE(p.proacl,acldefault('f',p.proowner))) a WHERE a.grantee<>p.proowner AND NOT(p.proname='read_v1' AND a.grantee=(SELECT oid FROM pg_roles WHERE rolname='lattice_runtime') AND a.privilege_type='EXECUTE' AND NOT a.is_grantable)), has_function_privilege('lattice_runtime',p.oid,'EXECUTE')
        FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace JOIN pg_language l ON l.oid=p.prolang WHERE n.nspname='registry_epoch' ORDER BY p.proname", &[]))?;
    if functions.len() != 3 {
        return Err(REJECTED);
    }
    for (row, (name, args, returns, language, definer, strict, volatility)) in
        functions.iter().zip([
            (
                "command_key_v1",
                "p_command_id text",
                "text",
                "sql",
                false,
                true,
                "i",
            ),
            (
                "guard_used_command_v1",
                "",
                "trigger",
                "plpgsql",
                true,
                false,
                "v",
            ),
            ("read_v1", "", "jsonb", "sql", true, false, "s"),
        ])
    {
        let marker = format!("CREATE FUNCTION registry_epoch.{name}(");
        let source = REGISTRY_EPOCH_SQL
            .split_once(&marker)
            .ok_or(REJECTED)?
            .1
            .split_once("AS $$")
            .ok_or(REJECTED)?
            .1
            .split_once("$$;")
            .ok_or(REJECTED)?
            .0;
        let options: Vec<String> = row.get::<_, Option<Vec<String>>>(8).unwrap_or_default();
        let expected_options = if definer {
            vec![
                "search_path=pg_catalog".to_owned(),
                "row_security=on".to_owned(),
            ]
        } else {
            vec!["search_path=pg_catalog".to_owned()]
        };
        if row.get::<_, String>(1) != name
            || row.get::<_, String>(2) != source
            || row.get::<_, bool>(3) != definer
            || row.get::<_, bool>(4) != strict
            || row.get::<_, String>(5) != volatility
            || row.get::<_, String>(6) != args
            || row.get::<_, String>(7) != returns
            || options != expected_options
            || row.get::<_, String>(9) != "lattice_migrator"
            || row.get::<_, String>(10) != language
            || row.get::<_, bool>(11)
            || row.get::<_, bool>(12)
            || row.get::<_, String>(13) != "f"
            || row.get::<_, Option<String>>(14).is_some()
            || row.get::<_, bool>(15)
            || row.get::<_, bool>(16) != (name == "read_v1")
        {
            return Err(REJECTED);
        }
    }
    let triggers=query(client.query("SELECT pg_get_triggerdef(t.oid,false),t.tgenabled::text,t.tgisinternal,t.tgconstraint::bigint FROM pg_trigger t JOIN pg_proc p ON p.oid=t.tgfoid JOIN pg_namespace n ON n.oid=p.pronamespace WHERE n.nspname='registry_epoch'", &[]))?;
    if triggers.len() != 1
        || triggers[0].get::<_, String>(0)
            != "CREATE TRIGGER registry_epoch_used_command_guard BEFORE INSERT ON control.project_registry_commands FOR EACH ROW EXECUTE FUNCTION registry_epoch.guard_used_command_v1()"
        || triggers[0].get::<_, String>(1) != "O"
        || triggers[0].get::<_, bool>(2)
        || triggers[0].get::<_, i64>(3) != 0
    {
        return Err(REJECTED);
    }
    let identity: Value = query(client.query_one("SELECT registry_epoch.read_v1()", &[]))?.get(0);
    if identity["sql_sha256"].as_str() != Some(digest(REGISTRY_EPOCH_SQL.as_bytes()).as_str()) {
        return Err(REJECTED);
    }
    Ok(Some(EpochCatalog {
        relation_oids: relations.iter().map(|r| r.get(0)).collect(),
        function_oids: functions.iter().map(|r| r.get(0)).collect(),
    }))
}

const SEAL_SCHEMA: &str = "lattice.registry-epoch.seal.v1";
const TRUST: &str = "REGISTRY_EPOCH_TRUST_REJECTED";
const MAX_SEAL_BYTES: usize = 64 * 1024 * 1024;

/// Host configuration is never read from an untrusted database or purge plan.
pub(crate) fn anchor_root(database: &str) -> Result<PathBuf> {
    AnchorDigest::new(database).map_err(|_| TRUST)?;
    let root = if let Some(root) = std::env::var_os("LATTICE_REGISTRY_ANCHOR_ROOT") {
        PathBuf::from(root)
    } else {
        let state_root = if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        } else {
            std::env::var_os("XDG_STATE_HOME")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
                })
        }
        .ok_or("REGISTRY_ANCHOR_HOST_CONFIGURATION_REQUIRED")?;
        state_root.join("LATTICE").join("registry-epochs")
    };
    if !root.is_absolute() {
        return Err(TRUST);
    }
    Ok(root.join(database))
}
fn anchor(database: &str) -> Result<RegistryEpochAnchor> {
    RegistryEpochAnchor::new(anchor_root(database)?, database).map_err(|_| TRUST)
}
fn read_anchor(database: &str) -> Result<AnchorState> {
    anchor(database)?.read().map_err(|_| TRUST)
}
fn content_digest(value: &str) -> Result<ContentDigest> {
    ContentDigest::from_sha256(value).map_err(|_| TRUST)
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name].as_str().ok_or(TRUST)
}
fn closed(value: &Value, names: &[&str]) -> Result<()> {
    let map = value.as_object().ok_or(TRUST)?;
    if map.len() != names.len() || names.iter().any(|name| !map.contains_key(*name)) {
        return Err(TRUST);
    }
    Ok(())
}
fn canonical(value: &Value, depth: usize) -> Result<CanonicalValue> {
    if depth > 64 {
        return Err(TRUST);
    }
    Ok(match value {
        Value::Null => CanonicalValue::Null,
        Value::Bool(b) => CanonicalValue::Bool(*b),
        Value::String(s) => CanonicalValue::String(s.clone()),
        Value::Array(items) => CanonicalValue::Array(
            items
                .iter()
                .map(|v| canonical(v, depth + 1))
                .collect::<Result<_>>()?,
        ),
        Value::Object(items) => CanonicalValue::Object(
            items
                .iter()
                .map(|(k, v)| Ok((k.clone(), canonical(v, depth + 1)?)))
                .collect::<Result<_>>()?,
        ),
        Value::Number(_) => return Err(TRUST),
    })
}
fn encode_baseline(plan: &RegistryEpochPlan) -> Result<Value> {
    let bytes = canonicalize(&plan.baseline().to_canonical_value()).map_err(|_| TRUST)?;
    serde_json::from_slice(bytes.as_slice()).map_err(|_| TRUST)
}
fn seal_digest(payload: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(payload).map_err(|_| TRUST)?;
    if bytes.len() > MAX_SEAL_BYTES {
        return Err("REGISTRY_EPOCH_CAPACITY_EXCEEDED");
    }
    Ok(digest(&bytes))
}
fn used_commands(envelope: &RegistryEpochBaseline) -> Result<BTreeMap<String, String>> {
    let mut used = BTreeMap::new();
    for id in envelope.normal_history().keys() {
        used.insert(
            registry_command_id_commitment(id)
                .map_err(|_| TRUST)?
                .as_str()
                .to_owned(),
            "ARCHIVED".to_owned(),
        );
    }
    for id in envelope.redacted_command_ids() {
        if used.insert(id.clone(), "REDACTED".to_owned()).is_some() {
            return Err(TRUST);
        }
    }
    Ok(used)
}

pub(crate) struct LoadedEpoch {
    pub baseline: VerifiedRegistryState,
    pub envelope: RegistryEpochBaseline,
    pub command_rows: Vec<Value>,
}

fn load_against<C: GenericClient>(
    client: &mut C,
    database: &str,
    expected: Option<&ActiveAnchor>,
) -> Result<Option<LoadedEpoch>> {
    if optional_catalog(client)?.is_none() {
        return if expected.is_none() {
            Ok(None)
        } else {
            Err(TRUST)
        };
    }
    let read: Value = query(client.query_one("SELECT registry_epoch.read_v1()", &[]))?.get(0);
    if read["seal"].is_null() {
        return if expected.is_none() && read["used_commands"] == json!([]) {
            Ok(None)
        } else {
            Err(TRUST)
        };
    }
    let expected = expected.ok_or("REGISTRY_EPOCH_EXTERNAL_ANCHOR_REQUIRED")?;
    let seal = &read["seal"];
    closed(seal, &["epoch", "seal_digest", "payload"])?;
    if seal["epoch"].as_u64() != Some(expected.epoch)
        || field(seal, "seal_digest")? != expected.seal_digest.as_str()
    {
        return Err(TRUST);
    }
    let payload = &seal["payload"];
    closed(
        payload,
        &[
            "schema",
            "database",
            "epoch",
            "previousSealDigest",
            "operationDigest",
            "baselineDigest",
            "baseline",
            "commandRows",
        ],
    )?;
    if seal_digest(payload)? != expected.seal_digest.as_str()
        || field(payload, "schema")? != SEAL_SCHEMA
        || field(payload, "database")? != database
        || payload["epoch"].as_u64() != Some(expected.epoch)
    {
        return Err(TRUST);
    }
    AnchorDigest::new(field(payload, "operationDigest")?).map_err(|_| TRUST)?;
    if expected.epoch == 1 {
        if !payload["previousSealDigest"].is_null() {
            return Err(TRUST);
        }
    } else {
        AnchorDigest::new(field(payload, "previousSealDigest")?).map_err(|_| TRUST)?;
    }
    let envelope =
        RegistryEpochBaseline::from_canonical_value(&canonical(&payload["baseline"], 0)?)
            .map_err(|_| TRUST)?;
    let baseline = verify_registry_epoch_baseline(
        &envelope,
        expected.epoch,
        &content_digest(field(payload, "baselineDigest")?)?,
    )
    .map_err(|_| TRUST)?;
    let command_rows = payload["commandRows"].as_array().ok_or(TRUST)?.clone();
    if command_rows.len() != envelope.normal_history().len() {
        return Err(TRUST);
    }
    let mut ids = std::collections::BTreeSet::new();
    for row in &command_rows {
        let id = field(row, "command_id")?;
        if !ids.insert(id)
            || !envelope
                .normal_history()
                .keys()
                .any(|key| key.as_str() == id)
        {
            return Err(TRUST);
        }
    }
    let expected_used = used_commands(&envelope)?
        .into_iter()
        .map(|(id, disposition)| json!({"commitment":id,"disposition":disposition}))
        .collect::<Vec<_>>();
    if read["used_commands"] != json!(expected_used) {
        return Err(TRUST);
    }
    Ok(Some(LoadedEpoch {
        baseline,
        envelope,
        command_rows,
    }))
}

/// Runtime never accepts a pending transition or a self-asserted DB seal.
pub(crate) fn load_epoch<C: GenericClient>(
    client: &mut C,
    database: &str,
) -> Result<Option<LoadedEpoch>> {
    match read_anchor(database)? {
        AnchorState::Absent => load_against(client, database, None),
        AnchorState::Active(active) => load_against(client, database, Some(&active)),
        AnchorState::Pending(_) => Err("REGISTRY_EPOCH_PENDING_MAINTENANCE"),
    }
}
/// An explicit same-operation maintenance resume may validate the old database
/// against the externally pinned previous seal before repeating its transaction.
pub(crate) fn load_epoch_for_resume<C: GenericClient>(
    client: &mut C,
    database: &str,
    binding: &str,
) -> Result<Option<LoadedEpoch>> {
    match read_anchor(database)? {
        AnchorState::Pending(pending) if pending.operation_digest.as_str() == binding => {
            load_against(client, database, pending.previous.as_ref())
        }
        AnchorState::Pending(_) => Err(TRUST),
        _ => load_epoch(client, database),
    }
}
pub(crate) fn load_epoch_for_transition<C: GenericClient>(
    client: &mut C,
    database: &str,
    next: &ActiveAnchor,
) -> Result<Option<LoadedEpoch>> {
    match read_anchor(database)? {
        AnchorState::Pending(pending) if pending.next == *next => {
            load_against(client, database, Some(next))
        }
        _ => Err(TRUST),
    }
}

pub(crate) struct EpochMigration {
    pub baseline: VerifiedRegistryState,
    pub payload: Value,
    pub next: ActiveAnchor,
    pub previous: Option<ActiveAnchor>,
    pub operation_digest: AnchorDigest,
    used: BTreeMap<String, String>,
}
pub(crate) fn plan_migration<C: GenericClient>(
    client: &mut C,
    database: &str,
    state: &VerifiedRegistryState,
    project: &ProjectId,
    binding: &str,
    redactions: &[RegistryRedactionAuthorization],
) -> Result<EpochMigration> {
    let previous = match read_anchor(database)? {
        AnchorState::Absent => None,
        AnchorState::Active(active) => Some(active),
        AnchorState::Pending(pending) if pending.operation_digest.as_str() == binding => {
            pending.previous
        }
        AnchorState::Pending(_) => return Err(TRUST),
    };
    let prior = load_against(client, database, previous.as_ref())?;
    let pure = plan_registry_epoch(
        state,
        project,
        state.epoch().checked_add(1).ok_or(TRUST)?,
        redactions,
    )
    .map_err(|error| match error {
        lattice_project_registry::RegistryError::EpochSurvivorReference => {
            "REGISTRY_CURRENT_SURVIVOR_REFERENCE"
        }
        lattice_project_registry::RegistryError::EpochRedactionRequired => {
            "REGISTRY_HISTORICAL_REDACTION_REQUIRED"
        }
        _ => "REGISTRY_EPOCH_PLAN_REJECTED",
    })?;
    let mut rows: BTreeMap<String, Value> = BTreeMap::new();
    if let Some(prior) = prior {
        for row in prior.command_rows {
            rows.insert(field(&row, "command_id")?.to_owned(), row);
        }
    }
    for row in query(client.query(
        "SELECT to_jsonb(c) FROM ONLY control.project_registry_commands c ORDER BY ordinal",
        &[],
    ))? {
        let raw: Value = row.get(0);
        if rows
            .insert(field(&raw, "command_id")?.to_owned(), raw)
            .is_some()
        {
            return Err(TRUST);
        }
    }
    let command_rows = pure
        .baseline()
        .normal_history()
        .keys()
        .map(|id| rows.remove(id.as_str()).ok_or(TRUST))
        .collect::<Result<Vec<_>>>()?;
    let payload = json!({"schema":SEAL_SCHEMA,"database":database,"epoch":pure.baseline().epoch(),"previousSealDigest":previous.as_ref().map(|active|active.seal_digest.as_str()),"operationDigest":binding,"baselineDigest":pure.baseline_digest().as_str(),"baseline":encode_baseline(&pure)?,"commandRows":command_rows});
    let next = ActiveAnchor::new(
        pure.baseline().epoch(),
        AnchorDigest::new(&seal_digest(&payload)?).map_err(|_| TRUST)?,
    )
    .map_err(|_| TRUST)?;
    let baseline =
        verify_registry_epoch_baseline(pure.baseline(), next.epoch, pure.baseline_digest())
            .map_err(|_| TRUST)?;
    Ok(EpochMigration {
        baseline,
        payload,
        next,
        previous,
        operation_digest: AnchorDigest::new(binding).map_err(|_| TRUST)?,
        used: used_commands(pure.baseline())?,
    })
}
pub(crate) fn prepare_anchor(database: &str, plan: &EpochMigration) -> Result<PendingAnchor> {
    anchor(database)?
        .prepare(plan.previous.as_ref(), &plan.next, &plan.operation_digest)
        .map_err(|_| "REGISTRY_EPOCH_ANCHOR_PREPARE_FAILED")
}
pub(crate) fn activate_anchor<C: GenericClient>(
    client: &mut C,
    database: &str,
    binding: &str,
) -> Result<()> {
    let pending = match read_anchor(database)? {
        AnchorState::Pending(pending) if pending.operation_digest.as_str() == binding => pending,
        AnchorState::Active(active) => {
            load_against(client, database, Some(&active))?;
            return Ok(());
        }
        _ => return Err(TRUST),
    };
    load_against(client, database, Some(&pending.next))?.ok_or(TRUST)?;
    // A committed receipt, independently bound to this operation, is required.
    let matches:i64=query(client.query_one("SELECT count(*)::bigint FROM ONLY project_purge.receipts WHERE request_digest=$1 AND result->>'registrySealDigest'=$2",&[&binding,&pending.next.seal_digest.as_str()]))?.get(0);
    if matches != 1 {
        return Err(TRUST);
    }
    anchor(database)?
        .activate(&pending)
        .map_err(|_| "REGISTRY_EPOCH_ANCHOR_ACTIVATE_FAILED")?;
    Ok(())
}
pub(crate) fn write_migration<C: GenericClient>(
    client: &mut C,
    project: &str,
    plan: &EpochMigration,
) -> Result<()> {
    query(client.execute("DELETE FROM ONLY control.project_registry_commands", &[]))?;
    query(client.execute(
        "DELETE FROM ONLY control.project_registry_identity_reservations WHERE project_id=$1",
        &[&project],
    ))?;
    query(client.execute(
        "DELETE FROM ONLY control.project_registry_projects WHERE project_id=$1",
        &[&project],
    ))?;
    let snapshot = export_untrusted_registry_snapshot(&plan.baseline);
    let observations: Vec<String> = snapshot
        .observations()
        .iter()
        .map(|o| o.digest().as_str().to_owned())
        .collect();
    query(client.execute("DELETE FROM ONLY control.project_registry_observations WHERE NOT (encode(observation_digest,'hex')=ANY($1))",&[&observations]))?;
    query(client.execute("DELETE FROM ONLY registry_epoch.used_commands", &[]))?;
    for (id, disposition) in &plan.used {
        query(client.execute(
            "INSERT INTO registry_epoch.used_commands VALUES($1,$2)",
            &[id, disposition],
        ))?;
    }
    let epoch = i64::try_from(plan.next.epoch).map_err(|_| TRUST)?;
    query(client.execute("INSERT INTO registry_epoch.current_seal VALUES(true,$1,$2,$3) ON CONFLICT(singleton) DO UPDATE SET epoch=excluded.epoch,seal_digest=excluded.seal_digest,payload=excluded.payload",&[&epoch,&plan.next.seal_digest.as_str(),&plan.payload]))?;
    Ok(())
}
