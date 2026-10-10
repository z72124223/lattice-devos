//! Offline repair of a proven archived first turn; never dispatches Codex work.
use crate::control_product::ControlProductCommand;
use crate::{DatabaseRole, MigrationTarget, PostgresControlProduct, verify_postgres_schema};
use lattice_contracts::ContentDigest;
use postgres::{Client, Config, IsolationLevel, NoTls};
use serde_json::{Value, json};
use std::time::Duration;

/// Facts independently recovered from locked native Codex archive files.
pub struct ArchivedClaimProof {
    pub project_id: String,
    pub task_ref: ContentDigest,
    pub claim_id: String,
    pub thread_id: String,
    pub turn_id: String,
    pub operation_id: String,
    pub evidence_digest: String,
    pub bound_digest: String,
    pub started_at: i64,
    pub completed_at: i64,
}

/// Record the missing observations in one transaction under stopped admission.
/// Native evidence verification is the caller's responsibility; this function is
/// not exposed by MCP. Neither Registry, task completion nor catalog is changed.
///
/// # Errors
/// Rejects active admission, stale identities, any other claim history, changed
/// replay evidence, missing task scope or a failed observation contract.
pub fn reconcile_archived_claim(
    admin: &mut Client,
    target: &MigrationTarget,
    port: u16,
    password: &str,
    proof: &ArchivedClaimProof,
) -> Result<Value, &'static str> {
    crate::postgres_setup::verify_stopped_admission(admin)
        .map_err(|_| "CLAIM_RECONCILIATION_MAINTENANCE_REQUIRED")?;
    verify_postgres_schema(admin, target, DatabaseRole::Migrator)
        .map_err(|_| "CLAIM_RECONCILIATION_SCHEMA_REJECTED")?;
    let runtime = Config::new()
        .host("127.0.0.1")
        .port(port)
        .user("lattice_runtime_login")
        .password(password)
        .dbname(target.database_name())
        .application_name("lattice-devos-task019")
        .options("-c role=lattice_runtime -c search_path=pg_catalog")
        .connect_timeout(Duration::from_secs(5))
        .connect_with_startup_timeout(NoTls, Duration::from_secs(5))
        .map_err(|_| "CLAIM_RECONCILIATION_DATABASE_UNAVAILABLE")?
        .ok_or("CLAIM_RECONCILIATION_DATABASE_UNAVAILABLE")?;
    let mut guard = admin
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|_| "CLAIM_RECONCILIATION_DATABASE_UNAVAILABLE")?;
    guard.batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'; LOCK TABLE control.runtime_admission IN EXCLUSIVE MODE")
        .map_err(|_| "CLAIM_RECONCILIATION_MAINTENANCE_REQUIRED")?;
    crate::postgres_setup::verify_stopped_admission(&mut guard)
        .map_err(|_| "CLAIM_RECONCILIATION_MAINTENANCE_REQUIRED")?;
    let registry = crate::project_registry::load_registry_for_maintenance(&mut guard, target)
        .map_err(|_| "CLAIM_RECONCILIATION_REGISTRY_REJECTED")?;
    let project = lattice_contracts::ProjectId::new(&proof.project_id)
        .map_err(|_| "CLAIM_RECONCILIATION_SCOPE_REJECTED")?;
    if registry.project(&project).is_none() {
        return Err("CLAIM_RECONCILIATION_SCOPE_REJECTED");
    }
    let evidence_ref = format!("evidence:sha256:{}", proof.evidence_digest);
    let mut commands = Vec::new();
    for (offset, kind) in [
        "DISPATCH_STARTED",
        "TURN_BOUND",
        "TURN_COMPLETED",
        "ARCHIVED",
    ]
    .iter()
    .enumerate()
    {
        commands.push(ControlProductCommand::Observe {
            task_ref: proof.task_ref.clone(), claim_id: proof.claim_id.clone(),
            request_id: format!("{}:{offset}", proof.operation_id),
            expected_sequence: i64::try_from(offset).map_err(|_| "CLAIM_RECONCILIATION_INPUT_REJECTED")? + 1,
            kind: (*kind).to_owned(), thread_id: Some(proof.thread_id.clone()),
            turn_id: (*kind != "DISPATCH_STARTED").then(|| proof.turn_id.clone()),
            summary: format!("LATE_ARCHIVED_CLAIM_RECONCILIATION: {kind}; native startedAt={} completedAt={}; original controller create/thread-bound/terminal receipts and archived rollout verified. Expanded prompt is not asserted byte-identical. This records historical facts, not new dispatch, product acceptance or formal task completion.", proof.started_at, proof.completed_at),
            evidence_ref: Some(evidence_ref.clone()), approval_id: None, decision: None,
            input_id: Some(proof.claim_id.clone()), payload: None,
        });
    }
    let records =
        PostgresControlProduct::reconcile_archived_observations(runtime, target, proof, &commands)?;
    guard
        .commit()
        .map_err(|_| "CLAIM_RECONCILIATION_GUARD_RELEASE_FAILED")?;
    Ok(
        json!({"schema":"lattice.claim-reconciliation.result.v1","status":"RECONCILED",
        "projectId":proof.project_id,"taskRef":proof.task_ref.as_str(),"claimId":proof.claim_id,
        "threadId":proof.thread_id,"turnId":proof.turn_id,"evidenceRef":evidence_ref,
        "records":records,"taskCompletionChanged":false,"projectDeletionExecuted":false,
        "assurance":"LOCKED_LOCAL_ARCHIVE_HISTORY",
        "limitations":["Does not establish absence of background work in other instances or hosts; project writers must remain stopped."]}),
    )
}
