//! One-time, explicit attribution of existing Bot history to a current Registry.
//! No lifecycle transition, dispatch or claim of native quiescence is performed.
use super::{Result, connect, digest, error, project_purge};
use crate::MigrationTarget;
use postgres::{Client, IsolationLevel};
use serde_json::{Value, json};

/// Verify complete stored histories and add only current ownership bindings.
/// # Errors
/// Rejects missing Registry identity, event gaps, drift, or non-offline operation.
#[allow(clippy::too_many_arguments)]
pub fn adopt_bot_project_ownership(
    main: &mut Client,
    target: &MigrationTarget,
    port: u16,
    run: &str,
    password: &str,
    system: &str,
    request: &Value,
) -> Result<Value> {
    let (apply, project) = validate_request(request)?;
    let inventory =
        super::inspect_project_purge_bot_lifecycle(port, run, password, system, project)?;
    if inventory["databasePresent"] != true {
        return Err("BOT_LIFECYCLE_DATABASE_UNAVAILABLE");
    }
    let (mut registry, observation) = project_purge::registry_lock(main, target, project)?;
    let stopped: bool = registry
        .query_one(
            "SELECT admission_mode='STOPPED' FROM ONLY control.runtime_admission WHERE singleton",
            &[],
        )
        .map_err(|e| error(&e))?
        .get(0);
    if !stopped {
        return Err("BOT_OWNERSHIP_MAINTENANCE_OFFLINE_REQUIRED");
    }
    let mut client = connect(port, run, password, "migrator")?;
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|e| error(&e))?;
    tx.batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'; LOCK TABLE bot_lifecycle.roles,bot_lifecycle.events IN ACCESS EXCLUSIVE MODE").map_err(|e|error(&e))?;
    super::verify(&mut tx, run)?;
    if !project_purge::verify(&mut tx)? {
        return Err("BOT_LIFECYCLE_OWNERSHIP_EXTENSION_REQUIRED");
    }
    tx.batch_execute("LOCK TABLE bot_project_ownership.bindings,bot_project_ownership.retired,bot_project_ownership.receipts IN ACCESS EXCLUSIVE MODE").map_err(|e|error(&e))?;
    let before = project_purge::snapshot(&mut tx)?;
    let original: Vec<_> = before
        .iter()
        .filter(|(t, _)| t.starts_with("bot_lifecycle."))
        .cloned()
        .collect();
    let store = target.expected_database_identity_sha256().as_str();
    let snapshot_digest = digest(
        &serde_json::to_vec(&json!([system, store, observation, original]))
            .map_err(|_| "BOT_OWNERSHIP_INPUT_REJECTED")?,
    );
    if apply && request["expectedSnapshotDigest"] != snapshot_digest {
        return Err("BOT_OWNERSHIP_SNAPSHOT_CHANGED");
    }
    let histories = tx.query("SELECT to_jsonb(r),COALESCE((SELECT jsonb_agg(to_jsonb(e) ORDER BY (e.receipt->'state'->>'revision')::bigint) FROM ONLY bot_lifecycle.events e WHERE e.project_id=r.project_id AND e.role_id=r.role_id),'[]'::jsonb) FROM ONLY bot_lifecycle.roles r WHERE project_id=$1 ORDER BY role_id COLLATE \"C\"",&[&project]).map_err(|e|error(&e))?;
    let malformed: bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM ONLY bot_lifecycle.events e WHERE e.project_id=$1 AND (NOT EXISTS(SELECT 1 FROM ONLY bot_lifecycle.roles r WHERE r.project_id=e.project_id AND r.role_id=e.role_id) OR e.request_digest IS DISTINCT FROM encode(sha256(convert_to(e.request::text,'UTF8')),'hex')))",&[&project]).map_err(|e|error(&e))?.get(0);
    if malformed || histories.is_empty() {
        return Err("BOT_OWNERSHIP_HISTORY_REJECTED");
    }
    let mut roles = Vec::new();
    let mut events = 0usize;
    for row in histories {
        let role: Value = row.get(0);
        let history: Value = row.get(1);
        verify_history(&role, &history)?;
        events += history
            .as_array()
            .ok_or("BOT_OWNERSHIP_HISTORY_REJECTED")?
            .len();
        let role_id = role["role_id"]
            .as_str()
            .ok_or("BOT_OWNERSHIP_HISTORY_REJECTED")?
            .to_owned();
        let retired: bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM ONLY bot_project_ownership.retired WHERE pair_digest=bot_project_ownership.pair_v1($1,$2))",&[&project,&role_id]).map_err(|e|error(&e))?.get(0);
        if retired {
            return Err("BOT_LIFECYCLE_PROJECT_RETIRED");
        }
        if let Some(binding) = tx.query_opt("SELECT store_digest FROM ONLY bot_project_ownership.bindings WHERE project_id=$1 AND role_id=$2",&[&project,&role_id]).map_err(|e|error(&e))? {
            if binding.get::<_,String>(0) == store { roles.push(role_id); continue; }
            return Err("BOT_OWNERSHIP_BINDING_CONFLICT");
        }
        roles.push(role_id);
    }
    if apply {
        for role in &roles {
            tx.execute("INSERT INTO bot_project_ownership.bindings VALUES($1,$2,$3,$4) ON CONFLICT(project_id,role_id) DO NOTHING",&[&project,role,&store,&observation]).map_err(|e|error(&e))?;
        }
        let after = project_purge::snapshot(&mut tx)?;
        let retained = |rows: &[(String, String)]| {
            rows.iter()
                .filter(|(t, _)| t != "bot_project_ownership.bindings")
                .cloned()
                .collect::<Vec<_>>()
        };
        if retained(&before) != retained(&after) {
            return Err("BOT_OWNERSHIP_SURVIVORS_CHANGED");
        }
        tx.commit().map_err(|_| "BOT_LIFECYCLE_OUTCOME_UNKNOWN")?;
    }
    Ok(
        json!({"schema":"lattice.bot-ownership-adoption.v1","status":if apply {"ADOPTED"} else {"READY"},"projectId":project,"snapshotDigest":snapshot_digest,"roleCount":roles.len(),"eventCount":events,"historyUnchanged":true,"assurance":"STORED_HISTORY_CURRENT_REGISTRY_BINDING","nativeActivityVerified":false}),
    )
}

fn verify_history(role: &Value, history: &Value) -> Result<()> {
    let events = history
        .as_array()
        .filter(|v| !v.is_empty())
        .ok_or("BOT_OWNERSHIP_HISTORY_REJECTED")?;
    for (index, event) in events.iter().enumerate() {
        let request = &event["request"];
        let receipt = &event["receipt"];
        let state = &receipt["state"];
        let revision = u64::try_from(index).map_err(|_| "BOT_OWNERSHIP_HISTORY_REJECTED")?;
        if [event, request, state]
            .iter()
            .any(|v| v["project_id"] != role["project_id"] || v["role_id"] != role["role_id"])
            || event["request_id"] != request["request_id"]
            || event["request_id"] != receipt["request_id"]
            || event["request_digest"] != receipt["request_digest"]
            || request["action"] != receipt["action"]
            || state["revision"] != revision + 1
            || request["expected_revision"] != revision
            || (index == 0) != (request["action"] == "register")
        {
            return Err("BOT_OWNERSHIP_HISTORY_REJECTED");
        }
    }
    let first = &events[0]["request"];
    let proof = &first["body"]["binding_receipt"];
    if proof["tool"] != "read_thread"
        || proof["success"] != true
        || proof["readback_verified"] != true
        || proof["target_thread_id"] != first["owner_thread_id"]
        || proof["target_host_id"] != first["owner_host_id"]
        || events.last().unwrap()["receipt"]["state"] != role["state"]
    {
        return Err("BOT_OWNERSHIP_HISTORY_REJECTED");
    }
    Ok(())
}

fn validate_request(request: &Value) -> Result<(bool, &str)> {
    let apply = request["action"] == "adopt-bot-ownership";
    let object = request.as_object().ok_or("BOT_OWNERSHIP_INPUT_REJECTED")?;
    if request["schema"] != "lattice.project-purge.request.v1"
        || (!apply && request["action"] != "preview-bot-adoption")
        || object.len() != if apply { 5 } else { 3 }
        || object.keys().any(|k| {
            ![
                "schema",
                "action",
                "projectId",
                "authorization",
                "expectedSnapshotDigest",
            ]
            .contains(&k.as_str())
        })
        || (apply && request["authorization"] != "ADOPT_EXISTING_BOT_OWNERSHIP")
    {
        return Err("BOT_OWNERSHIP_INPUT_REJECTED");
    }
    let project = request["projectId"]
        .as_str()
        .ok_or("BOT_OWNERSHIP_INPUT_REJECTED")?;
    Ok((apply, project))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Value, Value) {
        let state = json!({"project_id":"project-a","role_id":"role","revision":1});
        let event = json!({"project_id":"project-a","role_id":"role","request_id":"r1","request_digest":"digest",
            "request":{"project_id":"project-a","role_id":"role","request_id":"r1","action":"register","expected_revision":0,"owner_thread_id":"thread","owner_host_id":"local",
                "body":{"binding_receipt":{"tool":"read_thread","success":true,"readback_verified":true,"target_thread_id":"thread","target_host_id":"local"}}},
            "receipt":{"request_id":"r1","request_digest":"digest","action":"register","state":state}});
        (
            json!({"project_id":"project-a","role_id":"role","state":state}),
            json!([event]),
        )
    }
    #[test]
    fn full_history_rejects_gaps_ownership_drift_and_receipt_mismatch() {
        let (role, history) = fixture();
        assert!(verify_history(&role, &history).is_ok());
        for mutation in 0..6 {
            let mut bad = history.clone();
            match mutation {
                0 => bad = json!([]),
                1 => bad[0]["receipt"]["state"]["revision"] = json!(2),
                2 => bad[0]["request"]["project_id"] = json!("other"),
                3 => bad[0]["receipt"]["request_digest"] = json!("other"),
                4 => bad[0]["request"]["body"]["binding_receipt"]["success"] = json!(false),
                _ => bad[0]["request"]["expected_revision"] = json!(1),
            }
            assert!(verify_history(&role, &bad).is_err(), "mutation {mutation}");
        }
    }
}
