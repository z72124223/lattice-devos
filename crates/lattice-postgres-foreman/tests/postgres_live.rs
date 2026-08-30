use std::collections::BTreeSet;
use std::env;
use std::sync::{Arc, Barrier};
use std::thread;

use lattice_approval_verifier::{
    ApprovalVerifierCheckpoint, BindExecutionApprovalCommand, ExecutionApprovalChallenge,
    ExecutionApprovalSubject, FakeApprovalVerifier, FakeNormalSigner, IssueApprovalCommand,
    SecretMaterial, VerifiedApprovalExecutionContext, VerifyApprovalCommand,
    issue_verified_approval_execution_authority, nonce_commitment,
};
use lattice_artifact_store::{
    MAX_MANAGED_EVIDENCE_BYTES, ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence,
};
use lattice_contracts::{
    ApprovalAuthority, ApprovalIdentity, ApprovalLane, ApprovalOrigin, ApprovalSubject, AttemptId,
    ContentDigest, DaemonEpoch, GitRefIdentity, HolderProcessId, ProjectClass, ProjectId,
    ProjectLifecycle, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead,
    StoreAuthorityRevision, StoreDaemonInstanceId, SubjectBinding, TaskId, TaskLedgerStreamHead,
    TaskLedgerStreamIdentity, WriterLeaseAuthorityHead,
};
use lattice_foreman_state::{ForemanCheckpointIntent, ForemanState, SoleForemanBinding};
use lattice_postgres_foreman::{
    AdapterErrorKind, AppendDisposition, ClaimDisposition, ClaimReservationDisposition,
    ExecutionEnvironmentDescriptor, ExtensionApplyOutcome, ExtensionCatalogEvidence,
    ExtensionDatabaseRole, ExtensionSetupErrorKind, ExtensionTarget, ExternalCostBudget,
    MAX_ARTIFACT_BYTES_PER_ATTEMPT, MAX_ARTIFACTS_PER_ATTEMPT, ManagedPreparationObservation,
    ManagedPreparationObservationKind, ManagedPromotionIntent, ManagedPromotionSource, ModelReason,
    NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF, PostgresForeman, ProviderDispatchKind,
    ReasoningEffort, ReplayRecordState, RestartTaskKind, WorkerBudget, WorkerModel,
    apply_extension, verify_extension,
};
use lattice_postgres_store::{
    MigrationTarget, PostgresProjectRegistry, PostgresTaskLedger, PostgresTaskLedgerErrorKind,
};
use lattice_postgres_writer_lease::{PostgresWriterLease, V5ExtensionTarget};
use lattice_project_registry::{
    CommandId as RegistryCommandId, RegistryCommand as ProjectRegistryCommand,
    RegistryCommandOutcome, RepositoryObservation,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CorrelationId, ForemanAppendMetadata,
    LedgerEventKind, LedgerOutcome, ReasonCode, TaskExecutionBindingInput,
    TaskRuntimeAppendMetadata, TaskRuntimeEventLink, TaskSubmissionEnvelope,
    VerifiedTaskExecutionBinding, VerifiedWorkerAttemptRecord, WorkerAttemptInput,
    foreman_coordination_identity, plan_approval_evidence_append, plan_artifact_reference_append,
    plan_foreman_snapshot_append, plan_task_execution_binding, plan_worker_attempt_append,
};
use lattice_writer_lease::{
    CommandOutcome as WriterLeaseCommandOutcome, WriterLeaseAcquireRequest, WriterLeaseRepository,
    WriterLeaseRepositoryCommand,
};
use postgres::error::SqlState;
use postgres::{Client, Config, GenericClient, NoTls};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

#[test]
#[ignore = "requires a fresh explicitly provisioned disposable PostgreSQL 17 Store-v7 profile without Foreman bootstrap"]
fn disposable_store_v7_fresh_extension_apply_and_reconnect() {
    if env::var("LATTICE_FOREMAN_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let migrator_url = required("LATTICE_FOREMAN_MIGRATOR_URL");
    let runtime_url = required("LATTICE_FOREMAN_RUNTIME_URL");
    let database_name = required("LATTICE_FOREMAN_DATABASE_NAME");
    let run_id = required("LATTICE_FOREMAN_RUN_ID");
    let target = ExtensionTarget::new(database_name, run_id).expect("bounded live target");

    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    let installed = apply_extension(&mut migrator, &target).expect("install extension");
    let installed_evidence = match installed {
        ExtensionApplyOutcome::Installed(evidence) => evidence,
        ExtensionApplyOutcome::AlreadyCurrent(_) => panic!("live fixture was not fresh"),
    };
    let replay = apply_extension(&mut migrator, &target).expect("exact setup replay");
    let replay_evidence = match replay {
        ExtensionApplyOutcome::AlreadyCurrent(evidence) => evidence,
        ExtensionApplyOutcome::Installed(_) => panic!("extension installed twice"),
    };
    assert_eq!(installed_evidence, replay_evidence);
    drop(migrator);

    assert_runtime_acl_and_reconnect(&runtime_url, &target, &installed_evidence);
}

#[test]
#[ignore = "requires the product-bootstrapped disposable PostgreSQL 17 Store-v7 profile"]
fn disposable_store_v7_bootstrap_owned_extension_apply_acl_and_reconnect() {
    if env::var("LATTICE_FOREMAN_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let migrator_url = required("LATTICE_FOREMAN_MIGRATOR_URL");
    let runtime_url = required("LATTICE_FOREMAN_RUNTIME_URL");
    let database_name = required("LATTICE_FOREMAN_DATABASE_NAME");
    let run_id = required("LATTICE_FOREMAN_RUN_ID");
    let target = ExtensionTarget::new(database_name, run_id).expect("bounded live target");

    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    let bootstrap_evidence = match apply_extension(&mut migrator, &target)
        .expect("product bootstrap must have installed an exact extension")
    {
        ExtensionApplyOutcome::AlreadyCurrent(evidence) => evidence,
        ExtensionApplyOutcome::Installed(_) => {
            panic!("product bootstrap omitted the required Foreman extension")
        }
    };
    let replay_evidence = match apply_extension(&mut migrator, &target)
        .expect("exact product-bootstrap extension replay")
    {
        ExtensionApplyOutcome::AlreadyCurrent(evidence) => evidence,
        ExtensionApplyOutcome::Installed(_) => {
            panic!("bootstrap extension was unexpectedly reinstalled")
        }
    };
    assert_eq!(bootstrap_evidence, replay_evidence);
    drop(migrator);

    assert_runtime_acl_and_reconnect(&runtime_url, &target, &bootstrap_evidence);
}

fn assert_runtime_acl_and_reconnect(
    runtime_url: &str,
    target: &ExtensionTarget,
    installed_evidence: &ExtensionCatalogEvidence,
) {
    let mut runtime = connect_as(runtime_url, "lattice_runtime");
    let runtime_evidence = verify_extension(&mut runtime, &target, ExtensionDatabaseRole::Runtime)
        .expect("runtime catalog verification");
    assert_eq!(
        runtime_evidence.database_uuid(),
        installed_evidence.database_uuid()
    );
    assert_eq!(
        runtime_evidence.sql_sha256(),
        installed_evidence.sql_sha256()
    );

    let direct_read = runtime.query(
        "SELECT singleton FROM ONLY foreman_execution.extension_identity",
        &[],
    );
    let error = direct_read.expect_err("runtime must not read extension tables directly");
    assert_eq!(
        error.as_db_error().map(postgres::error::DbError::code),
        Some(&SqlState::INSUFFICIENT_PRIVILEGE)
    );
    let direct_dispatch_read = runtime.query(
        "SELECT operation_kind FROM ONLY foreman_execution.provider_dispatch_claims",
        &[],
    );
    let error = direct_dispatch_read.expect_err("runtime must not read provider claims directly");
    assert_eq!(
        error.as_db_error().map(postgres::error::DbError::code),
        Some(&SqlState::INSUFFICIENT_PRIVILEGE)
    );
    let direct_stage_read = runtime.query(
        "SELECT task_ref FROM ONLY foreman_execution.staged_artifact_references",
        &[],
    );
    let error = direct_stage_read.expect_err("runtime must not read artifact stages directly");
    assert_eq!(
        error.as_db_error().map(postgres::error::DbError::code),
        Some(&SqlState::INSUFFICIENT_PRIVILEGE)
    );

    let mut adapter = PostgresForeman::new(runtime, &target).expect("verified runtime adapter");
    assert!(
        adapter
            .list_active_task_refs(256)
            .expect("bounded active-task reader")
            .is_empty()
    );
    let restart_refs = adapter
        .list_restart_task_refs(256)
        .expect("bounded restart-task reader");
    assert!(restart_refs.len() <= 256);
    let unique_restart_refs = restart_refs
        .iter()
        .map(|entry| entry.task_ref().as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(unique_restart_refs.len(), restart_refs.len());
    let task_ref = ContentDigest::from_sha256("1".repeat(64)).expect("fixture digest");
    assert!(
        adapter
            .load_staged_artifact_reference(&task_ref)
            .expect("fixed staged-artifact reader")
            .is_none()
    );
    let empty = adapter
        .read_task_replay(&task_ref)
        .expect("fixed replay reader");
    assert!(empty.records().is_empty());
    let references = adapter
        .load_reference_links(&task_ref)
        .expect("fixed reference-link reader");
    assert!(references.artifact_links().is_empty());
    assert!(references.approval_links().is_empty());
    let reconnect_digest = empty.evidence_digest().clone();
    drop(adapter);

    let runtime = connect_as(runtime_url, "lattice_runtime");
    let mut reconnected = PostgresForeman::new(runtime, target).expect("reconnected adapter");
    let replayed = reconnected
        .read_task_replay(&task_ref)
        .expect("reconnected fixed replay reader");
    assert_eq!(replayed.evidence_digest(), &reconnect_digest);
    assert_eq!(
        reconnected
            .list_restart_task_refs(256)
            .expect("reconnected bounded restart-task reader"),
        restart_refs
    );
}

fn assert_artifact_stage_rejected(
    runtime: &mut Client,
    evidence_bytes: &[u8],
    content_digest: &[u8],
    media_type: &str,
    descriptor_bytes: &[u8],
    descriptor_digest: &[u8],
    expected_message: &str,
) {
    let project_id = "artifact-ingress-probe";
    let task_ref = vec![0x11_u8; 32];
    let attempt = 1_i16;
    let evidence_kind = "WORKER_LIFECYCLE";
    let payload_schema = "lattice.artifact-ingress-probe/1.0";
    let producer_id = "lattice-postgres-foreman-live";
    let producer_version = "1";
    let producer_digest = vec![0x22_u8; 32];
    let created_at = "2026-08-28T00:00:00Z";
    let stream_id = vec![0x44_u8; 32];
    let before_sequence = "0";
    let before_last_event_digest = vec![0x45_u8; 32];
    let before_resource_revision = "0";
    let before_resource_projection_digest = vec![0x46_u8; 32];
    let before_head_digest = vec![0x47_u8; 32];
    let event_sequence = "1";
    let event_digest = vec![0x48_u8; 32];
    let command_id = "artifact-ingress-probe-command";
    let request_digest = vec![0x49_u8; 32];
    let correlation_id = "artifact-ingress-probe-correlation";
    let command_occurred_at = "2026-08-28T00:00:00Z";
    let error = runtime
        .query_one(
            "SELECT foreman_execution.stage_artifact_reference_v1( \
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15, \
                $16::text::numeric,$17,$18::text::numeric,$19,$20, \
                $21::text::numeric,$22,$23,$24,$25,$26,$27)",
            &[
                &project_id,
                &task_ref,
                &attempt,
                &evidence_kind,
                &media_type,
                &payload_schema,
                &producer_id,
                &producer_version,
                &producer_digest,
                &created_at,
                &evidence_bytes,
                &content_digest,
                &descriptor_bytes,
                &descriptor_digest,
                &stream_id,
                &before_sequence,
                &before_last_event_digest,
                &before_resource_revision,
                &before_resource_projection_digest,
                &before_head_digest,
                &event_sequence,
                &event_digest,
                &command_id,
                &request_digest,
                &descriptor_digest,
                &correlation_id,
                &command_occurred_at,
            ],
        )
        .expect_err("artifact ingress probe must fail closed");
    let database = error.as_db_error().expect("database rejection");
    assert_eq!(database.code(), &SqlState::RAISE_EXCEPTION);
    assert_eq!(database.message(), expected_message);
}

#[test]
#[ignore = "requires the installed disposable PostgreSQL 17 Store-v7 live profile"]
fn disposable_store_v7_fresh_process_restart_replay() {
    if env::var("LATTICE_FOREMAN_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let migrator_url = required("LATTICE_FOREMAN_MIGRATOR_URL");
    let runtime_url = required("LATTICE_FOREMAN_RUNTIME_URL");
    let database_name = required("LATTICE_FOREMAN_DATABASE_NAME");
    let run_id = required("LATTICE_FOREMAN_RUN_ID");
    let target = ExtensionTarget::new(database_name, run_id).expect("bounded restart target");

    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    assert!(matches!(
        apply_extension(&mut migrator, &target).expect("restart setup replay"),
        ExtensionApplyOutcome::AlreadyCurrent(_)
    ));
    drop(migrator);

    let runtime = connect_as(&runtime_url, "lattice_runtime");
    let mut adapter = PostgresForeman::new(runtime, &target).expect("restart runtime adapter");
    assert!(
        adapter
            .list_active_task_refs(256)
            .expect("restart active-task reader")
            .is_empty()
    );
    let task_ref = ContentDigest::from_sha256("1".repeat(64)).expect("fixture digest");
    let replay = adapter
        .read_task_replay(&task_ref)
        .expect("restart replay reader");
    assert!(replay.records().is_empty());
    let references = adapter
        .load_reference_links(&task_ref)
        .expect("restart reference-link reader");
    assert!(references.artifact_links().is_empty());
    assert!(references.approval_links().is_empty());
}

#[test]
#[ignore = "requires the installed disposable PostgreSQL 17 Store-v7 live profile"]
fn disposable_store_v7_artifact_ingress_guards_reject_before_insert() {
    if env::var("LATTICE_FOREMAN_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let migrator_url = required("LATTICE_FOREMAN_MIGRATOR_URL");
    let runtime_url = required("LATTICE_FOREMAN_RUNTIME_URL");
    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    let before: i64 = migrator
        .query_one(
            "SELECT pg_catalog.count(*) FROM ONLY foreman_execution.staged_artifact_references",
            &[],
        )
        .expect("count artifact stages before ingress probes")
        .get(0);
    let mut runtime = connect_as(&runtime_url, "lattice_runtime");

    let safe = br#"{"schema":"lattice.artifact-ingress-probe.v1","status":"safe"}"#;
    let wrong_digest = vec![0x55_u8; 32];
    let wrong_descriptor = vec![0x33_u8; 32];
    assert_artifact_stage_rejected(
        &mut runtime,
        safe,
        &wrong_digest,
        "application/json",
        b"{}",
        &wrong_descriptor,
        "FOREMAN_ARTIFACT_CONTENT_DIGEST_MISMATCH",
    );

    let secret = br#"{"schema":"lattice.artifact-ingress-probe.v1","remote":"https://user:secret@example.invalid/repo"}"#;
    let content_digest = Sha256::digest(secret).to_vec();
    assert_artifact_stage_rejected(
        &mut runtime,
        secret,
        &content_digest,
        "application/json",
        b"{}",
        &wrong_descriptor,
        "FOREMAN_ARTIFACT_SECRET_REJECTED",
    );

    let safe_content_digest = Sha256::digest(safe).to_vec();
    assert_artifact_stage_rejected(
        &mut runtime,
        safe,
        &safe_content_digest,
        "application/octet-stream",
        b"{}",
        &wrong_descriptor,
        "FOREMAN_ARTIFACT_MEDIA_TYPE_REJECTED",
    );
    assert_artifact_stage_rejected(
        &mut runtime,
        safe,
        &safe_content_digest,
        "application/json",
        b"{}",
        &wrong_descriptor,
        "FOREMAN_ARTIFACT_DESCRIPTOR_DIGEST_MISMATCH",
    );

    let after: i64 = migrator
        .query_one(
            "SELECT pg_catalog.count(*) FROM ONLY foreman_execution.staged_artifact_references",
            &[],
        )
        .expect("count artifact stages after ingress probes")
        .get(0);
    assert_eq!(after, before, "rejected ingress cannot retain a stage");
}

#[test]
#[ignore = "requires verified approvals retained by the ordered disposable PostgreSQL acceptance"]
fn disposable_store_v7_approval_owner_snapshot_restart_tamper_and_acl() {
    if env::var("LATTICE_FOREMAN_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let migrator_url = required("LATTICE_FOREMAN_MIGRATOR_URL");
    let runtime_url = required("LATTICE_FOREMAN_RUNTIME_URL");
    let target = ExtensionTarget::new(
        required("LATTICE_FOREMAN_DATABASE_NAME"),
        required("LATTICE_FOREMAN_RUN_ID"),
    )
    .expect("approval owner snapshot target");
    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    let retained = migrator
        .query_one(
            "SELECT pg_catalog.encode(a.task_ref,'hex'), \
                    pg_catalog.encode(a.authority_digest,'hex'), \
                    a.approval_owner_snapshot_digest, s.snapshot_bytes, \
                    s.snapshot_content_digest, s.command_high_water, \
                    s.command_tail_digest, s.nonce_bindings_digest \
               FROM ONLY foreman_execution.approval_evidence AS a \
               JOIN ONLY foreman_execution.approval_owner_snapshots AS s \
                 ON s.snapshot_digest=a.approval_owner_snapshot_digest \
              WHERE a.authority_source='VERIFIED_APPROVAL' \
              ORDER BY a.task_ref LIMIT 1",
            &[],
        )
        .expect("one retained Approval-owner snapshot");
    let task_ref =
        ContentDigest::from_sha256(retained.get::<_, String>(0)).expect("retained task_ref");
    let authority_digest = ContentDigest::from_sha256(retained.get::<_, String>(1))
        .expect("retained authority digest");
    let snapshot_digest: Vec<u8> = retained.get(2);
    let snapshot_bytes: Vec<u8> = retained.get(3);
    let snapshot_content_digest: Vec<u8> = retained.get(4);
    let command_high_water: i64 = retained.get(5);
    let command_tail_digest: Vec<u8> = retained.get(6);
    let nonce_bindings_digest: Vec<u8> = retained.get(7);

    let mut runtime_acl = connect_as(&runtime_url, "lattice_runtime");
    let acl_error = runtime_acl
        .query_one(
            "SELECT snapshot_digest FROM ONLY foreman_execution.approval_owner_snapshots LIMIT 1",
            &[],
        )
        .expect_err("runtime role cannot read Approval-owner physical rows");
    assert_eq!(
        acl_error.as_db_error().expect("ACL database error").code(),
        &SqlState::INSUFFICIENT_PRIVILEGE
    );

    let mut replay = PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
        .expect("fresh Approval-owner replay adapter");
    let loaded_authority = replay
        .load_execution_authority(&task_ref, &authority_digest)
        .expect("fresh Approval-owner snapshot replay");
    assert_eq!(loaded_authority.authority_digest(), &authority_digest);
    let checkpoint = ApprovalVerifierCheckpoint::new(
        u64::try_from(command_high_water).expect("checkpoint high water"),
        Some(content_digest_from_bytes(&command_tail_digest)),
        content_digest_from_bytes(&nonce_bindings_digest),
        content_digest_from_bytes(&snapshot_digest),
    )
    .expect("retained owner checkpoint");
    let mut retained_owner = FakeApprovalVerifier::new();
    retained_owner
        .restore_snapshot_bytes(&snapshot_bytes, &checkpoint)
        .expect("restore retained Approval owner");
    let references = replay
        .load_reference_links(&task_ref)
        .expect("load approval Task-Ledger link");
    let [approval_link] = references.approval_links() else {
        panic!("one exact approval link");
    };
    let direct_runtime = replay
        .record_verified_approval_evidence(&loaded_authority, approval_link.link(), &retained_owner)
        .expect_err("general runtime cannot self-attest even genuine Approval-owner state");
    assert_eq!(direct_runtime.kind(), AdapterErrorKind::Database);
    drop(replay);

    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_owner_snapshots \
                SET snapshot_bytes=snapshot_bytes || decode('20','hex') \
              WHERE snapshot_digest=$1",
            &[&snapshot_digest],
        )
        .expect("tamper owner snapshot bytes");
    let mut corrupted = PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
        .expect("corrupted snapshot adapter");
    assert_eq!(
        corrupted
            .load_execution_authority(&task_ref, &authority_digest)
            .expect_err("snapshot byte tamper must fail closed")
            .kind(),
        AdapterErrorKind::CorruptReplay
    );
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_owner_snapshots \
                SET snapshot_bytes=$2 WHERE snapshot_digest=$1",
            &[&snapshot_digest, &snapshot_bytes],
        )
        .expect("restore owner snapshot bytes");

    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_owner_snapshots \
                SET snapshot_content_digest=decode(repeat('ab',32),'hex') \
              WHERE snapshot_digest=$1",
            &[&snapshot_digest],
        )
        .expect("tamper owner snapshot content commitment");
    let mut corrupted = PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
        .expect("content-tampered snapshot adapter");
    assert_eq!(
        corrupted
            .load_execution_authority(&task_ref, &authority_digest)
            .expect_err("snapshot content commitment tamper must fail closed")
            .kind(),
        AdapterErrorKind::CorruptReplay
    );
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_owner_snapshots \
                SET snapshot_content_digest=$2 WHERE snapshot_digest=$1",
            &[&snapshot_digest, &snapshot_content_digest],
        )
        .expect("restore owner snapshot content commitment");

    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_owner_snapshots \
                SET command_high_water=command_high_water+1 WHERE snapshot_digest=$1",
            &[&snapshot_digest],
        )
        .expect("tamper owner checkpoint high water");
    let mut corrupted = PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
        .expect("checkpoint-tampered snapshot adapter");
    assert_eq!(
        corrupted
            .load_execution_authority(&task_ref, &authority_digest)
            .expect_err("checkpoint tamper must fail closed")
            .kind(),
        AdapterErrorKind::CorruptReplay
    );
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_owner_snapshots \
                SET command_high_water=$2 WHERE snapshot_digest=$1",
            &[&snapshot_digest, &command_high_water],
        )
        .expect("restore owner checkpoint high water");

    let foreign_snapshot: Vec<u8> = migrator
        .query_one(
            "SELECT approval_owner_snapshot_digest \
               FROM ONLY foreman_execution.approval_evidence \
              WHERE authority_source='VERIFIED_APPROVAL' \
                AND approval_owner_snapshot_digest<>$1 \
              ORDER BY task_ref LIMIT 1",
            &[&snapshot_digest],
        )
        .expect("foreign Approval-owner snapshot")
        .get(0);
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_evidence \
                SET approval_owner_snapshot_digest=$3 \
              WHERE task_ref=$1 AND authority_digest=$2",
            &[
                &digest_bytes_for_test(&task_ref),
                &digest_bytes_for_test(&authority_digest),
                &foreign_snapshot,
            ],
        )
        .expect("substitute task snapshot reference");
    let mut substituted =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
            .expect("substituted snapshot adapter");
    assert_eq!(
        substituted
            .load_execution_authority(&task_ref, &authority_digest)
            .expect_err("foreign task snapshot substitution must fail closed")
            .kind(),
        AdapterErrorKind::CorruptReplay
    );
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_evidence \
                SET approval_owner_snapshot_digest=$3 \
              WHERE task_ref=$1 AND authority_digest=$2",
            &[
                &digest_bytes_for_test(&task_ref),
                &digest_bytes_for_test(&authority_digest),
                &snapshot_digest,
            ],
        )
        .expect("restore task snapshot reference");

    let mut restored = PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
        .expect("restored Approval-owner adapter");
    assert_eq!(
        restored
            .load_execution_authority(&task_ref, &authority_digest)
            .expect("restored exact owner replay")
            .authority_digest(),
        &authority_digest
    );
}

#[test]
#[ignore = "destructively tampers with the installed disposable live profile"]
#[allow(clippy::too_many_lines)]
fn disposable_store_v7_catalog_tamper_fails_closed() {
    if env::var("LATTICE_FOREMAN_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let migrator_url = required("LATTICE_FOREMAN_MIGRATOR_URL");
    let database_name = required("LATTICE_FOREMAN_DATABASE_NAME");
    let run_id = required("LATTICE_FOREMAN_RUN_ID");
    let target = ExtensionTarget::new(database_name, run_id).expect("bounded tamper target");
    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    let assert_rejected = |client: &mut Client, reason: &str| {
        let failure =
            verify_extension(client, &target, ExtensionDatabaseRole::Migrator).expect_err(reason);
        assert_eq!(failure.kind(), ExtensionSetupErrorKind::CatalogMismatch);
        assert_eq!(failure.code(), "FOREMAN_EXTENSION_CATALOG_PROFILE_MISMATCH");
    };
    let assert_current = |client: &mut Client, reason: &str| {
        verify_extension(client, &target, ExtensionDatabaseRole::Migrator).expect(reason);
    };

    assert_current(&mut migrator, "baseline exact ACL profile");
    migrator
        .batch_execute("GRANT SELECT ON foreman_execution.staged_artifact_references TO PUBLIC")
        .expect("grant disposable PUBLIC staged-table read");
    assert_rejected(&mut migrator, "PUBLIC table ACL drift must fail closed");
    migrator
        .batch_execute("REVOKE SELECT ON foreman_execution.staged_artifact_references FROM PUBLIC")
        .expect("restore PUBLIC staged-table ACL");
    assert_current(&mut migrator, "PUBLIC table ACL restore");

    migrator
        .batch_execute(
            "GRANT SELECT ON foreman_execution.staged_artifact_references \
             TO lattice_runtime_login",
        )
        .expect("grant disposable extra-role staged-table read");
    assert_rejected(&mut migrator, "extra-role table ACL drift must fail closed");
    migrator
        .batch_execute(
            "REVOKE SELECT ON foreman_execution.staged_artifact_references \
             FROM lattice_runtime_login",
        )
        .expect("restore extra-role staged-table ACL");
    assert_current(&mut migrator, "extra-role table ACL restore");

    migrator
        .batch_execute(
            "GRANT SELECT (evidence_bytes) \
             ON foreman_execution.staged_artifact_references \
             TO lattice_runtime_login",
        )
        .expect("grant disposable extra-role staged-byte column read");
    assert_rejected(
        &mut migrator,
        "extra-role table-column ACL drift must fail closed",
    );
    migrator
        .batch_execute(
            "REVOKE SELECT (evidence_bytes) \
             ON foreman_execution.staged_artifact_references \
             FROM lattice_runtime_login",
        )
        .expect("restore extra-role staged-byte column ACL");
    assert_current(&mut migrator, "extra-role table-column ACL restore");

    migrator
        .batch_execute(
            "GRANT EXECUTE ON FUNCTION \
             foreman_execution.read_staged_artifact_reference_v1(bytea) \
             TO lattice_runtime_login",
        )
        .expect("grant disposable extra-role staged-reader execute");
    assert_rejected(
        &mut migrator,
        "extra-role function ACL drift must fail closed",
    );
    migrator
        .batch_execute(
            "REVOKE EXECUTE ON FUNCTION \
             foreman_execution.read_staged_artifact_reference_v1(bytea) \
             FROM lattice_runtime_login",
        )
        .expect("restore extra-role staged-reader ACL");
    assert_current(&mut migrator, "extra-role function ACL restore");

    migrator
        .batch_execute("GRANT USAGE ON SCHEMA foreman_execution TO lattice_runtime_login")
        .expect("grant disposable extra-role schema usage");
    assert_rejected(
        &mut migrator,
        "extra-role schema ACL drift must fail closed",
    );
    migrator
        .batch_execute("REVOKE USAGE ON SCHEMA foreman_execution FROM lattice_runtime_login")
        .expect("restore extra-role schema ACL");
    assert_current(&mut migrator, "extra-role schema ACL restore");

    migrator
        .batch_execute(
            "ALTER FUNCTION foreman_execution.read_task_replay_v1(bytea) \
             RENAME TO read_task_replay_tampered_v1",
        )
        .expect("apply disposable function tamper");
    assert_rejected(&mut migrator, "tampered catalog must fail closed");
}

#[test]
#[ignore = "requires a fresh explicitly provisioned disposable PostgreSQL 17 Store-v7 profile"]
#[allow(clippy::too_many_lines)]
fn disposable_store_v7_atomic_claim_capacity_and_retry_budget() {
    if env::var("LATTICE_FOREMAN_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let migrator_url = required("LATTICE_FOREMAN_MIGRATOR_URL");
    let runtime_url = required("LATTICE_FOREMAN_RUNTIME_URL");
    let database_name = required("LATTICE_FOREMAN_DATABASE_NAME");
    let run_id = required("LATTICE_FOREMAN_RUN_ID");
    let foreman_target =
        ExtensionTarget::new(database_name.clone(), run_id.clone()).expect("bounded live target");
    let store_target =
        MigrationTarget::new(database_name, run_id).expect("Store-v7 live target identity");

    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    let _ = apply_extension(&mut migrator, &foreman_target).expect("install or verify extension");
    let authority = activate_fixture_authority(&mut migrator);
    drop(migrator);

    let mut ledger = PostgresTaskLedger::new(connect_store_runtime(&runtime_url), &store_target)
        .expect("verified Task Ledger runtime");
    let foreman_writer = acquire_formal_foreman_writer(&runtime_url, &store_target, &authority);
    let foreman_checkpoint =
        append_formal_foreman_checkpoint(&mut ledger, &authority, &foreman_writer, 1);
    let budget = WorkerBudget::new(
        4,
        1,
        2,
        900,
        100_000,
        3,
        ExternalCostBudget::Unavailable,
        "2099-12-31T23:59:59Z",
    )
    .expect("closed fixture budget");

    let mut fixtures = Vec::new();
    let mut records = Vec::new();
    for task_number in 1_u8..=5 {
        let fixture = if task_number == 1 {
            build_claim_fixture_with_intake_observer(
                &mut ledger,
                &authority,
                &runtime_url,
                &foreman_target,
                &store_target,
                &budget,
                1,
                &foreman_checkpoint,
                task_number,
                |submission| {
                    let mut fresh = PostgresForeman::new(
                        connect_as(&runtime_url, "lattice_runtime"),
                        &foreman_target,
                    )
                    .expect("fresh post-intake pre-promotion adapter");
                    let discovered = fresh
                        .list_restart_task_refs_page(None, 256)
                        .expect("discover committed unpromoted DRAFT");
                    assert_eq!(
                        fresh
                            .list_restart_task_refs_page(None, 256)
                            .expect("exact DRAFT discovery replay"),
                        discovered
                    );
                    let matching = discovered
                        .iter()
                        .filter(|candidate| candidate.task_ref() == submission.task_ref())
                        .collect::<Vec<_>>();
                    assert_eq!(matching.len(), 1);
                    let draft = matching[0];
                    assert_eq!(draft.task_ref(), submission.task_ref());
                    assert_eq!(draft.restart_kind(), RestartTaskKind::DraftPendingPromotion);
                    assert_eq!(draft.restart_priority(), 6);
                    assert!(draft.attempt_number().is_none());
                    assert!(draft.attempt_id().is_none());
                    assert!(
                        fresh
                            .load_task_promotion_source(submission.task_ref())
                            .expect("pre-promotion source read")
                            .is_none()
                    );

                    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
                    let project_row = migrator
                        .query_one(
                            "SELECT project.project_id::text, project.drift_repository \
                               FROM ONLY control.task_submission_envelopes AS envelope \
                               JOIN ONLY control.project_registry_projects AS project \
                                 ON project.project_id=envelope.project_id \
                              WHERE envelope.task_ref=$1",
                            &[&submission.task_ref().as_str()],
                        )
                        .expect("load committed DRAFT Project Registry identity");
                    let project_id: String = project_row.get(0);
                    let original_drift: bool = project_row.get(1);
                    assert!(!original_drift);
                    migrator
                        .execute(
                            "UPDATE ONLY control.project_registry_projects \
                                SET drift_repository=true WHERE project_id=$1",
                            &[&project_id],
                        )
                        .expect("introduce committed DRAFT Project Registry drift");
                    let mut drifted = PostgresForeman::new(
                        connect_as(&runtime_url, "lattice_runtime"),
                        &foreman_target,
                    )
                    .expect("DRAFT drift restart adapter");
                    let candidates = drifted
                        .list_restart_task_refs(256)
                        .expect("discover committed DRAFT Project Registry drift");
                    let matching = candidates
                        .iter()
                        .filter(|candidate| candidate.task_ref() == submission.task_ref())
                        .collect::<Vec<_>>();
                    assert_eq!(
                        matching.len(),
                        1,
                        "DRAFT drift must not disappear from restart discovery"
                    );
                    let candidate = matching[0];
                    assert_eq!(
                        candidate.restart_kind(),
                        RestartTaskKind::DraftProjectReconciliationRequired
                    );
                    assert_eq!(candidate.restart_priority(), 6);
                    assert!(candidate.attempt_number().is_none());
                    assert!(candidate.attempt_id().is_none());
                    migrator
                        .execute(
                            "UPDATE ONLY control.project_registry_projects \
                                SET drift_repository=$2 WHERE project_id=$1",
                            &[&project_id, &original_drift],
                        )
                        .expect("restore committed DRAFT Project Registry drift");
                },
            )
        } else {
            build_claim_fixture(
                &mut ledger,
                &authority,
                &runtime_url,
                &foreman_target,
                &store_target,
                &budget,
                1,
                &foreman_checkpoint,
                task_number,
            )
        };
        records.push(fixture.record.clone());
        fixtures.push(fixture);
    }
    drop(ledger);
    assert_intake_link_tamper_fails_closed(
        &migrator_url,
        &runtime_url,
        &foreman_target,
        &fixtures[0],
        &fixtures[1],
    );

    let mut before_reservation =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("pre-reservation restart adapter");
    let promoted = collect_restart_pages(&mut before_reservation, 2);
    let fixture_refs = fixtures
        .iter()
        .map(|fixture| fixture.record.task_ref().as_str())
        .collect::<BTreeSet<_>>();
    let promoted_fixtures = promoted
        .iter()
        .filter(|candidate| fixture_refs.contains(candidate.task_ref().as_str()))
        .collect::<Vec<_>>();
    assert_eq!(promoted_fixtures.len(), 5);
    assert!(promoted_fixtures.iter().all(|candidate| {
        candidate.restart_kind() == RestartTaskKind::PromotedNoAttempt
            && candidate.attempt_number().is_none()
            && candidate.attempt_id().is_none()
    }));
    drop(before_reservation);

    // Reserve every independent task before entering the claim race. A
    // reservation error must surface directly instead of abandoning one
    // participant while the remaining threads wait forever at the barrier.
    let mut reservation_adapter =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("capacity reservation adapter");
    let native_index = records.len() - 1;
    let mut environments = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let descriptor = (index != native_index).then(|| {
            live_execution_environment(record.task_ref(), '8')
                .expect("exact schema-1.1 environment fixture")
        });
        let expected_ref = descriptor
            .as_ref()
            .map_or(NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF, |environment| {
                environment.environment_ref().as_str()
            });
        assert_eq!(
            if index == native_index {
                reservation_adapter
                    .reserve_worker_attempt(record, 3)
                    .expect("native capacity reservation")
            } else {
                reservation_adapter
                    .reserve_worker_attempt_with_execution_environment_ref(record, 3, expected_ref)
                    .expect("WSL capacity reservation")
            },
            ClaimReservationDisposition::Reserved
        );
        let pending = reservation_adapter
            .load_pending_worker_attempt(record.task_ref())
            .expect("pending attempt after reservation")
            .expect("retained pending attempt");
        assert_eq!(pending.execution_environment_ref(), expected_ref);
        if index == 0 {
            let mut fresh_after_reservation =
                PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
                    .expect("fresh reserve-to-environment runtime adapter");
            assert!(
                fresh_after_reservation
                    .load_execution_environments(record.task_ref())
                    .expect("fresh runtime reads pending attempt before environment record")
                    .is_empty(),
                "pending non-native reservation may precisely precede its descriptor row"
            );
            let missing = reservation_adapter
                .claim_worker_attempt_with_execution_environment_ref(record, 3, expected_ref)
                .expect_err("claim must reject the reserve-to-environment crash window");
            assert_eq!(missing.kind(), AdapterErrorKind::ClaimRejected);
            assert_eq!(missing.code(), "FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED");
            assert!(
                reservation_adapter
                    .load_pending_worker_attempt(record.task_ref())
                    .expect("pending reader after rejected environment-free claim")
                    .is_some(),
                "rejected claim must retain the exact pending reservation"
            );
        }
        let Some(descriptor) = descriptor else {
            assert!(
                reservation_adapter
                    .load_execution_environment(record.task_ref(), record.attempt_number())
                    .expect("native environment lookup")
                    .is_none(),
                "native packets retain the established environment-free lane"
            );
            environments.push(None);
            continue;
        };
        if index == 0 {
            let mut stale_sandbox_policy = live_execution_environment_json(record.task_ref(), '8');
            let retained_policy = stale_sandbox_policy["sandbox_policy"]["policy_digest"].clone();
            stale_sandbox_policy["verification_toolchain"]["home_dir"] = Value::String(format!(
                "{}/home-substituted",
                stale_sandbox_policy["verification_toolchain"]["isolation_root"]
                    .as_str()
                    .expect("isolation root")
            ));
            rehash_live_execution_environment(&mut stale_sandbox_policy);
            stale_sandbox_policy["sandbox_policy"]["policy_digest"] = retained_policy;
            rehash_live_environment_identity_only(&mut stale_sandbox_policy);
            let stale_json = serde_json::to_string(&stale_sandbox_policy)
                .expect("stale sandbox-policy descriptor JSON");
            let stale_ref = stale_sandbox_policy["identity_digest"]
                .as_str()
                .expect("stale environment ref");
            let mut raw_runtime = connect_as(&runtime_url, "lattice_runtime");
            let rejected = raw_runtime
                .query_one(
                    "SELECT foreman_execution.record_execution_environment_v1( \
                        decode($1,'hex'),$2,$3,decode($4,'hex'),$5,$6)",
                    &[
                        &record.task_ref().as_str(),
                        &i16::try_from(record.attempt_number()).expect("attempt number"),
                        &record.attempt_id().as_str(),
                        &record.packet_digest().as_str(),
                        &stale_json,
                        &stale_ref,
                    ],
                )
                .expect_err("SQL ingress must reject a stale canonical sandbox-policy digest");
            assert_eq!(
                rejected
                    .as_db_error()
                    .map(postgres::error::DbError::message),
                Some("FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH")
            );
        }
        if index == 0 {
            let mut raw_runtime = connect_as(&runtime_url, "lattice_runtime");

            let mut substituted_mapping = live_execution_environment_json(record.task_ref(), '8');
            substituted_mapping["path_mapping"]["digest"] =
                Value::String(format!("path-mapping:sha256:{}", "0".repeat(64)));
            rehash_live_environment_identity_only(&mut substituted_mapping);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &substituted_mapping,
                "coherently substituted path-mapping digest and top-level identity",
                "FOREMAN_EXECUTION_ENVIRONMENT_DIGEST_MISMATCH",
            );

            let mut nested_tree = live_execution_environment_json(record.task_ref(), '8');
            let task_root = nested_tree["verification_toolchain"]["task_root"]
                .as_str()
                .expect("task root")
                .to_owned();
            let nested_codex_root = format!("{task_root}/nested/codex");
            let nested_launcher = format!("{nested_codex_root}/bin/codex");
            nested_tree["immutable_snapshot"]["trees"]["codex"]["root"] =
                Value::String(nested_codex_root);
            nested_tree["linux"]["launcher_path"] = Value::String(nested_launcher.clone());
            nested_tree["verification_toolchain"]["sandbox"]["path"] =
                Value::String(nested_launcher);
            rehash_live_execution_environment(&mut nested_tree);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &nested_tree,
                "nested immutable codex tree root with coherent dependent paths",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let mut substituted_launcher = live_execution_environment_json(record.task_ref(), '8');
            let codex_root = substituted_launcher["immutable_snapshot"]["trees"]["codex"]["root"]
                .as_str()
                .expect("codex tree root")
                .to_owned();
            let alternate_launcher = format!("{codex_root}/codex");
            substituted_launcher["linux"]["launcher_path"] =
                Value::String(alternate_launcher.clone());
            substituted_launcher["verification_toolchain"]["sandbox"]["path"] =
                Value::String(alternate_launcher);
            rehash_live_execution_environment(&mut substituted_launcher);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &substituted_launcher,
                "non-exact launcher path within the sealed codex tree",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let mut substituted_keyring_daemon =
                live_execution_environment_json(record.task_ref(), '8');
            let keyring_root = substituted_keyring_daemon["immutable_snapshot"]["trees"]["keyring"]
                ["root"]
                .as_str()
                .expect("keyring tree root")
                .to_owned();
            substituted_keyring_daemon["linux"]["keyring_daemon_path"] = Value::String(format!(
                "{keyring_root}/root/usr/bin/gnome-keyring-daemon-substituted"
            ));
            rehash_live_execution_environment(&mut substituted_keyring_daemon);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &substituted_keyring_daemon,
                "non-exact keyring daemon path within the sealed keyring tree",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let mut substituted_keyring_library =
                live_execution_environment_json(record.task_ref(), '8');
            let keyring_root =
                substituted_keyring_library["immutable_snapshot"]["trees"]["keyring"]["root"]
                    .as_str()
                    .expect("keyring tree root")
                    .to_owned();
            substituted_keyring_library["linux"]["keyring_library_path"] =
                Value::String(format!("{keyring_root}/root/usr/lib/x86_64-linux-gnu"));
            rehash_live_execution_environment(&mut substituted_keyring_library);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &substituted_keyring_library,
                "non-exact keyring library path within the sealed keyring tree",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let mut noncanonical_systemd_run =
                live_execution_environment_json(record.task_ref(), '8');
            noncanonical_systemd_run["process_fence"]["systemd_run_path"] =
                Value::String("/usr/bin/../bin/systemd-run".to_owned());
            rehash_live_execution_environment(&mut noncanonical_systemd_run);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &noncanonical_systemd_run,
                "noncanonical systemd-run process-fence path",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let mut noncanonical_systemctl =
                live_execution_environment_json(record.task_ref(), '8');
            noncanonical_systemctl["process_fence"]["systemctl_path"] =
                Value::String("/usr//bin/systemctl".to_owned());
            rehash_live_execution_environment(&mut noncanonical_systemctl);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &noncanonical_systemctl,
                "noncanonical systemctl process-fence path",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let mut noncanonical_cargo_home =
                live_execution_environment_json(record.task_ref(), '8');
            let isolation_root =
                noncanonical_cargo_home["verification_toolchain"]["isolation_root"]
                    .as_str()
                    .expect("verification isolation root")
                    .to_owned();
            noncanonical_cargo_home["verification_toolchain"]["cargo_home"] =
                Value::String(format!("{isolation_root}/../cargo-home"));
            rehash_live_execution_environment(&mut noncanonical_cargo_home);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &noncanonical_cargo_home,
                "noncanonical durable verification cargo home",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let mut noncanonical_tree_root =
                live_execution_environment_json(record.task_ref(), '8');
            let rust_root = noncanonical_tree_root["immutable_snapshot"]["trees"]["rust"]["root"]
                .as_str()
                .expect("immutable rust tree root")
                .to_owned();
            noncanonical_tree_root["immutable_snapshot"]["trees"]["rust"]["root"] =
                Value::String(format!("{rust_root}/../rust"));
            rehash_live_execution_environment(&mut noncanonical_tree_root);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &noncanonical_tree_root,
                "noncanonical durable immutable tree root",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let mut invalid_cargo_host = live_execution_environment_json(record.task_ref(), '8');
            invalid_cargo_host["verification_toolchain"]["cargo_host"] =
                Value::String("x86_64 unknown-linux-gnu".to_owned());
            rehash_live_execution_environment(&mut invalid_cargo_host);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &invalid_cargo_host,
                "invalid cargo host triple",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            for pointer in [
                "/gateway/version",
                "/linux/launcher_version",
                "/linux/node_version",
                "/linux/git_version",
                "/process_fence/systemd_run_version",
                "/process_fence/systemctl_version",
                "/process_fence/supervisor_bootstrap_node/version",
                "/process_fence/immutable_probe_lsattr/version",
                "/process_fence/noninteractive_root_probe/version",
                "/verification_toolchain/npm/version",
                "/verification_toolchain/cargo/version",
                "/verification_toolchain/rustc/version",
                "/verification_toolchain/rustdoc/version",
                "/verification_toolchain/sandbox_helper/version",
            ] {
                for secret in ["token=fixture", "password=fixture", "secret=fixture"] {
                    let mut secret_bearing_version =
                        live_execution_environment_json(record.task_ref(), '8');
                    let version = secret_bearing_version
                        .pointer_mut(pointer)
                        .and_then(|value| value.as_str())
                        .expect("live version fixture")
                        .to_owned();
                    *secret_bearing_version
                        .pointer_mut(pointer)
                        .expect("mutable live version fixture") =
                        Value::String(format!("{version} {secret}"));
                    rehash_live_execution_environment(&mut secret_bearing_version);
                    assert_record_execution_environment_v1_rejects(
                        &mut raw_runtime,
                        record,
                        &secret_bearing_version,
                        &format!("credential-like version at {pointer}: {secret}"),
                        "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
                    );
                }
            }

            for (pointer, sentinel) in [
                (
                    "/verification_toolchain/home_dir",
                    "github_pat_phase4homesentinel",
                ),
                ("/linux/node_path", "gho_phase4toolpathsentinel"),
            ] {
                let mut secret_path = live_execution_environment_json(record.task_ref(), '8');
                let prior = secret_path
                    .pointer(pointer)
                    .and_then(Value::as_str)
                    .expect("live path fixture")
                    .to_owned();
                *secret_path
                    .pointer_mut(pointer)
                    .expect("mutable live path fixture") =
                    Value::String(format!("{prior}/{sentinel}"));
                rehash_live_execution_environment(&mut secret_path);
                assert_record_execution_environment_v1_rejects(
                    &mut raw_runtime,
                    record,
                    &secret_path,
                    &format!("credential-shaped path at {pointer}"),
                    "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
                );
            }

            let task_root_sentinel = "ghp_phase4taskrootsentinel";
            let secret_task_root = live_execution_environment_json_with_task_root(
                record.task_ref(),
                '8',
                &format!("/home/{task_root_sentinel}"),
            );
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &secret_task_root,
                "credential-shaped task root",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let cwd_sentinel = "sk-phase4repositorysentinel";
            let mut secret_cwd = live_execution_environment_json(record.task_ref(), '8');
            let task_root = secret_cwd["verification_toolchain"]["task_root"]
                .as_str()
                .expect("task root")
                .to_owned();
            let repository = format!("{task_root}/managed-worktrees/{cwd_sentinel}");
            secret_cwd["linux"]["cwd"] = Value::String(repository.clone());
            secret_cwd["path_mapping"]["linux_path"] = Value::String(repository.clone());
            secret_cwd["path_mapping"]["windows_path"] = Value::String(format!(
                r"\\wsl.localhost\Ubuntu{}",
                repository.replace('/', "\\")
            ));
            rehash_live_execution_environment(&mut secret_cwd);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &secret_cwd,
                "credential-shaped repository cwd",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            for sentinel in [
                "password=phase4-password-sentinel",
                "token=phase4-token-sentinel",
                "secret=phase4-secret-sentinel",
                "api key=phase4-api-key-sentinel",
                "Bearer phase4-bearer-sentinel",
            ] {
                let mut secret_leaf = live_execution_environment_json(record.task_ref(), '8');
                secret_leaf["distribution_identity"]["kernel_release"] =
                    Value::String(format!("{sentinel}-6.18.33.2-microsoft-standard-WSL2"));
                rehash_live_execution_environment(&mut secret_leaf);
                assert_record_execution_environment_v1_rejects(
                    &mut raw_runtime,
                    record,
                    &secret_leaf,
                    "credential-shaped arbitrary string leaf",
                    "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
                );
            }

            for (pointer, invalid) in [
                ("/gateway/version", "1234567.6.1".to_owned()),
                (
                    "/process_fence/systemd_run_version",
                    "systemd 259 ()".to_owned(),
                ),
                (
                    "/process_fence/noninteractive_root_probe/version",
                    format!("sudo-rs 0.2.13-{}", "a".repeat(65)),
                ),
            ] {
                let mut noncanonical_version =
                    live_execution_environment_json(record.task_ref(), '8');
                *noncanonical_version
                    .pointer_mut(pointer)
                    .expect("mutable live version boundary fixture") = Value::String(invalid);
                rehash_live_execution_environment(&mut noncanonical_version);
                assert_record_execution_environment_v1_rejects(
                    &mut raw_runtime,
                    record,
                    &noncanonical_version,
                    &format!("noncanonical version boundary at {pointer}"),
                    "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
                );
            }

            let mut wildcard_sibling = live_execution_environment_json_with_task_root(
                record.task_ref(),
                '8',
                "/home/lattice_root",
            );
            let sibling_isolation = format!(
                "/home/latticeXroot/verifier-state/{}",
                record.task_ref().as_str()
            );
            wildcard_sibling["verification_toolchain"]["isolation_root"] =
                Value::String(sibling_isolation.clone());
            for (key, suffix) in [
                ("home_dir", "home"),
                ("temp_dir", "tmp"),
                ("npm_cache", "npm-cache"),
                ("cargo_home", "cargo-home"),
                ("cargo_target_dir", "cargo-target"),
            ] {
                wildcard_sibling["verification_toolchain"][key] =
                    Value::String(format!("{sibling_isolation}/{suffix}"));
            }
            rehash_live_execution_environment(&mut wildcard_sibling);
            assert_record_execution_environment_v1_rejects(
                &mut raw_runtime,
                record,
                &wildcard_sibling,
                "LIKE wildcard sibling substituted for underscore task-root descendant",
                "FOREMAN_EXECUTION_ENVIRONMENT_INPUT_REJECTED",
            );

            let rejected_row_count = connect_as(&migrator_url, "lattice_migrator")
                .query_one(
                    "SELECT count(*) FROM ONLY foreman_execution.execution_environments \
                     WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                    &[
                        &record.task_ref().as_str(),
                        &i16::try_from(record.attempt_number()).expect("attempt number"),
                    ],
                )
                .expect("independent rejected-environment row count")
                .get::<_, i64>(0);
            assert_eq!(
                rejected_row_count, 0,
                "rejected descriptors must not create durable environment rows"
            );
        }
        assert_eq!(
            reservation_adapter
                .record_execution_environment(record, &descriptor)
                .expect("record pending-attempt environment"),
            AppendDisposition::Inserted
        );
        assert_eq!(
            reservation_adapter
                .record_execution_environment(record, &descriptor)
                .expect("pending-attempt environment exact replay"),
            AppendDisposition::ExactReplay
        );
        if index == 0 {
            let substituted = live_execution_environment(record.task_ref(), 'c')
                .expect("digest-substituted schema-1.1 environment");
            let rejected = reservation_adapter
                .record_execution_environment(record, &substituted)
                .expect_err("changed toolchain/digest/environment ref must fail closed");
            assert_eq!(rejected.kind(), AdapterErrorKind::ClaimRejected);
            assert_eq!(
                rejected.code(),
                "FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH"
            );
        }
        environments.push(Some(descriptor));
    }
    drop(reservation_adapter);

    let mut fresh_environment_reader =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("fresh-process environment reader");
    for (record, expected) in records.iter().zip(&environments) {
        let Some(expected) = expected else {
            assert!(
                fresh_environment_reader
                    .load_execution_environment(record.task_ref(), record.attempt_number())
                    .expect("fresh-process native environment lookup")
                    .is_none()
            );
            continue;
        };
        let retained = fresh_environment_reader
            .load_execution_environment(record.task_ref(), record.attempt_number())
            .expect("fresh-process environment reconstruction")
            .expect("retained exact environment");
        assert_eq!(retained.attempt_id(), record.attempt_id());
        assert_eq!(retained.packet_digest(), record.packet_digest());
        assert_eq!(retained.descriptor(), expected);
        assert_eq!(
            retained.descriptor().canonical_json(),
            expected.canonical_json()
        );
        assert_eq!(
            retained.descriptor().environment_ref(),
            expected.environment_ref()
        );
    }
    drop(fresh_environment_reader);

    let mut native_adapter =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("native claim adapter");
    let native_claim = native_adapter
        .claim_worker_attempt(&records[native_index], 3)
        .expect("native claim remains admitted without a descriptor row");
    assert_eq!(native_claim.disposition(), ClaimDisposition::Claimed);
    assert_eq!(native_claim.global_active(), 1);
    let native_replay = native_adapter
        .claim_worker_attempt(&records[native_index], 3)
        .expect("native exact replay remains admitted");
    assert_eq!(native_replay.disposition(), ClaimDisposition::ExactReplay);
    assert_eq!(native_replay.global_active(), 1);
    drop(native_adapter);

    let barrier = Arc::new(Barrier::new(records.len() - 1));
    let mut handles = Vec::new();
    for (index, record) in records.iter().cloned().enumerate() {
        if index == native_index {
            continue;
        }
        let execution_environment_ref = environments[index]
            .as_ref()
            .expect("WSL descriptor")
            .environment_ref()
            .as_str()
            .to_owned();
        let barrier = Arc::clone(&barrier);
        let runtime_url = runtime_url.clone();
        let target = foreman_target.clone();
        handles.push(thread::spawn(move || {
            let runtime = connect_as(&runtime_url, "lattice_runtime");
            let mut adapter = PostgresForeman::new(runtime, &target).expect("runtime adapter");
            barrier.wait();
            (
                index,
                adapter.claim_worker_attempt_with_execution_environment_ref(
                    &record,
                    3,
                    &execution_environment_ref,
                ),
            )
        }));
    }
    let mut indexed_results = handles
        .into_iter()
        .map(|handle| handle.join().expect("claim thread"))
        .collect::<Vec<_>>();
    indexed_results.push((native_index, Ok(native_claim)));
    indexed_results.sort_by_key(|(index, _)| *index);
    let results = indexed_results
        .into_iter()
        .map(|(_, result)| result)
        .collect::<Vec<_>>();
    let mut admitted = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .copied()
        .collect::<Vec<_>>();
    admitted.sort_by_key(|outcome| outcome.global_active());
    assert_eq!(admitted.len(), 4, "exactly four global claims are admitted");
    assert_eq!(
        admitted
            .iter()
            .map(|outcome| outcome.global_active())
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(admitted.iter().all(|outcome| {
        outcome.disposition() == ClaimDisposition::Claimed && outcome.task_active() == 1
    }));
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    assert_eq!(
        results
            .iter()
            .find_map(|result| result.as_ref().err())
            .map(|error| error.code()),
        Some("FOREMAN_GLOBAL_CAPACITY_EXHAUSTED")
    );

    let successful_index = results
        .iter()
        .enumerate()
        .find(|(index, result)| result.is_ok() && environments[*index].is_some())
        .map(|(index, _)| index)
        .expect("one admitted WSL fixture");
    let admitted_record = &records[successful_index];
    let admitted_environment = environments[successful_index]
        .as_ref()
        .expect("admitted WSL environment");
    let runtime = connect_as(&runtime_url, "lattice_runtime");
    let mut replay = PostgresForeman::new(runtime, &foreman_target).expect("replay adapter");
    let reconstructed = replay
        .load_execution_environment(admitted_record.task_ref(), admitted_record.attempt_number())
        .expect("fresh active-attempt environment reconstruction")
        .expect("active attempt environment");
    assert_eq!(reconstructed.descriptor(), admitted_environment);
    assert_eq!(
        replay
            .record_execution_environment(admitted_record, admitted_environment)
            .expect("active-attempt environment exact replay"),
        AppendDisposition::ExactReplay
    );
    let exact = replay
        .claim_worker_attempt_with_execution_environment_ref(
            admitted_record,
            3,
            admitted_environment.environment_ref().as_str(),
        )
        .expect("same task and attempt exact replay");
    assert_eq!(exact.disposition(), ClaimDisposition::ExactReplay);
    assert_eq!(exact.global_active(), 4, "replay cannot consume capacity");
    assert_eq!(exact.task_active(), 1, "one active attempt per task");
    assert_eq!(
        replay
            .reserve_worker_attempt_with_execution_environment_ref(
                admitted_record,
                3,
                admitted_environment.environment_ref().as_str(),
            )
            .expect("consumed reservation exact replay"),
        ClaimReservationDisposition::ExactReplay
    );
    let substituted_claim_environment = live_execution_environment(admitted_record.task_ref(), 'c')
        .expect("substituted active-claim environment");
    let substituted_claim = replay
        .claim_worker_attempt_with_execution_environment_ref(
            admitted_record,
            3,
            substituted_claim_environment.environment_ref().as_str(),
        )
        .expect_err("active exact replay with a substituted environment ref must fail closed");
    assert_eq!(substituted_claim.kind(), AdapterErrorKind::ClaimRejected);
    assert_eq!(
        substituted_claim.code(),
        "FOREMAN_EXECUTION_ENVIRONMENT_ATTEMPT_MISMATCH"
    );
    let thread_subject = digest(99_901);
    let attempt_number_sql =
        i16::try_from(admitted_record.attempt_number()).expect("attempt number");
    let mut environment_tamper = connect_as(&migrator_url, "lattice_migrator");
    let typed_projection = environment_tamper
        .query_one(
            "SELECT keyring_library_manifest_ref, \
                    encode(keyring_library_manifest_digest,'hex'), \
                    sandbox_helper_path, sandbox_helper_version, \
                    encode(sandbox_helper_digest,'hex'), \
                    immutable_snapshot_ref, encode(immutable_snapshot_digest,'hex'), \
                    sandbox_policy_ref, encode(sandbox_policy_digest,'hex'), \
                    privilege_boundary_ref, encode(privilege_boundary_digest,'hex'), \
                    immutable_probe_lsattr_path, immutable_probe_lsattr_version, \
                    encode(immutable_probe_lsattr_digest,'hex'), \
                    noninteractive_root_probe_path, noninteractive_root_probe_version, \
                    encode(noninteractive_root_probe_digest,'hex') \
               FROM ONLY foreman_execution.execution_environments \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
        )
        .expect("independently query keyring manifest and sandbox helper");
    assert_eq!(
        typed_projection.get::<_, String>(0),
        admitted_environment.keyring_library_manifest_ref()
    );
    assert_eq!(
        typed_projection.get::<_, String>(1),
        admitted_environment
            .keyring_library_manifest_digest()
            .as_str()
    );
    assert_eq!(
        typed_projection.get::<_, String>(2),
        admitted_environment.sandbox_helper().path()
    );
    assert_eq!(
        typed_projection.get::<_, String>(3),
        admitted_environment.sandbox_helper().version()
    );
    assert_eq!(
        typed_projection.get::<_, String>(4),
        admitted_environment.sandbox_helper().digest().as_str()
    );
    assert_eq!(
        typed_projection.get::<_, String>(5),
        admitted_environment.immutable_snapshot_ref()
    );
    assert_eq!(
        typed_projection.get::<_, String>(6),
        admitted_environment.immutable_snapshot_digest().as_str()
    );
    assert_eq!(
        typed_projection.get::<_, String>(7),
        admitted_environment.sandbox_policy_ref()
    );
    assert_eq!(
        typed_projection.get::<_, String>(8),
        admitted_environment.sandbox_policy_digest().as_str()
    );
    assert_eq!(
        typed_projection.get::<_, String>(9),
        admitted_environment.privilege_boundary_ref()
    );
    assert_eq!(
        typed_projection.get::<_, String>(10),
        admitted_environment.privilege_boundary_digest().as_str()
    );
    assert_eq!(
        typed_projection.get::<_, String>(11),
        admitted_environment.immutable_probe_lsattr().path()
    );
    assert_eq!(
        typed_projection.get::<_, String>(12),
        admitted_environment.immutable_probe_lsattr().version()
    );
    assert_eq!(
        typed_projection.get::<_, String>(13),
        admitted_environment
            .immutable_probe_lsattr()
            .digest()
            .as_str()
    );
    assert_eq!(
        typed_projection.get::<_, String>(14),
        admitted_environment.noninteractive_root_probe().path()
    );
    assert_eq!(
        typed_projection.get::<_, String>(15),
        admitted_environment.noninteractive_root_probe().version()
    );
    assert_eq!(
        typed_projection.get::<_, String>(16),
        admitted_environment
            .noninteractive_root_probe()
            .digest()
            .as_str()
    );
    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                SET cargo_path=cargo_path || '-substituted' \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
            )
            .expect("commit independently queryable toolchain-path substitution"),
        1
    );
    let mut tampered_reader =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("tampered fresh-process reader");
    let tampered = tampered_reader
        .load_execution_environment(admitted_record.task_ref(), admitted_record.attempt_number())
        .expect_err("typed-column substitution must fail closed");
    assert_eq!(tampered.kind(), AdapterErrorKind::CorruptReplay);
    drop(tampered_reader);
    let reserve_rejected = replay
        .reserve_worker_attempt_with_execution_environment_ref(
            admitted_record,
            3,
            admitted_environment.environment_ref().as_str(),
        )
        .expect_err("active reserve replay must reject a substituted environment");
    assert_eq!(reserve_rejected.kind(), AdapterErrorKind::ClaimRejected);
    assert_eq!(
        reserve_rejected.code(),
        "FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH"
    );
    let claim_rejected = replay
        .claim_worker_attempt_with_execution_environment_ref(
            admitted_record,
            3,
            admitted_environment.environment_ref().as_str(),
        )
        .expect_err("active claim replay must reject a substituted environment");
    assert_eq!(claim_rejected.kind(), AdapterErrorKind::ClaimRejected);
    assert_eq!(
        claim_rejected.code(),
        "FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH"
    );
    let record_rejected = replay
        .record_execution_environment(admitted_record, admitted_environment)
        .expect_err("record exact replay must reject a substituted typed column");
    assert_eq!(record_rejected.kind(), AdapterErrorKind::ClaimRejected);
    assert_eq!(
        record_rejected.code(),
        "FOREMAN_EXECUTION_ENVIRONMENT_SUBSTITUTION"
    );
    let provider_rejected = replay
        .claim_provider_dispatch(
            admitted_record,
            ProviderDispatchKind::WorkerThread,
            admitted_record.payload_digest(),
            admitted_record.packet_digest(),
            &thread_subject,
        )
        .expect_err("provider dispatch must reject a substituted execution environment");
    assert_eq!(provider_rejected.kind(), AdapterErrorKind::ClaimRejected);
    assert_eq!(
        provider_rejected.code(),
        "FOREMAN_PROVIDER_DISPATCH_REJECTED"
    );
    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET cargo_path=$3 \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[
                    &admitted_record.task_ref().as_str(),
                    &attempt_number_sql,
                    &admitted_environment.cargo().path(),
                ],
            )
            .expect("restore exact independently queryable toolchain path"),
        1
    );

    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET keyring_library_manifest_ref = \
                            'keyring-library-manifest:sha256:' || repeat('4',64), \
                        keyring_library_manifest_digest = decode(repeat('4',64),'hex') \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
            )
            .expect("commit keyring-library manifest substitution"),
        1
    );
    let mut manifest_tampered_reader =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("manifest-tampered fresh-process reader");
    assert_eq!(
        manifest_tampered_reader
            .load_execution_environment(
                admitted_record.task_ref(),
                admitted_record.attempt_number()
            )
            .expect_err("keyring-library manifest substitution must fail closed")
            .kind(),
        AdapterErrorKind::CorruptReplay
    );
    drop(manifest_tampered_reader);
    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET keyring_library_manifest_ref=$3, \
                        keyring_library_manifest_digest=decode($4,'hex') \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[
                    &admitted_record.task_ref().as_str(),
                    &attempt_number_sql,
                    &admitted_environment.keyring_library_manifest_ref(),
                    &admitted_environment
                        .keyring_library_manifest_digest()
                        .as_str(),
                ],
            )
            .expect("restore exact keyring-library manifest"),
        1
    );

    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET sandbox_helper_path=sandbox_helper_path || '-substituted' \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
            )
            .expect("commit bundled sandbox-helper substitution"),
        1
    );
    let mut helper_tampered_reader =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("helper-tampered fresh-process reader");
    assert_eq!(
        helper_tampered_reader
            .load_execution_environment(
                admitted_record.task_ref(),
                admitted_record.attempt_number()
            )
            .expect_err("sandbox-helper substitution must fail closed")
            .kind(),
        AdapterErrorKind::CorruptReplay
    );
    drop(helper_tampered_reader);
    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET sandbox_helper_path=$3 \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[
                    &admitted_record.task_ref().as_str(),
                    &attempt_number_sql,
                    &admitted_environment.sandbox_helper().path(),
                ],
            )
            .expect("restore exact bundled sandbox helper"),
        1
    );

    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET immutable_snapshot_ref = \
                            'wsl2-immutable-snapshot:sha256:' || repeat('4',64), \
                        immutable_snapshot_digest = decode(repeat('4',64),'hex'), \
                        sandbox_policy_ref = \
                            'wsl2-sandbox-policy:sha256:' || repeat('4',64), \
                        sandbox_policy_digest = decode(repeat('4',64),'hex'), \
                        privilege_boundary_ref = \
                            'wsl2-privilege-boundary:sha256:' || repeat('4',64), \
                        privilege_boundary_digest = decode(repeat('4',64),'hex') \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
            )
            .expect("commit typed security-boundary reference substitution"),
        1
    );
    let mut boundary_tampered_reader =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("boundary-tampered fresh-process reader");
    assert_eq!(
        boundary_tampered_reader
            .load_execution_environment(
                admitted_record.task_ref(),
                admitted_record.attempt_number()
            )
            .expect_err("typed security-boundary substitution must fail closed")
            .kind(),
        AdapterErrorKind::CorruptReplay
    );
    drop(boundary_tampered_reader);
    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET immutable_snapshot_ref=$3, \
                        immutable_snapshot_digest=decode($4,'hex'), \
                        sandbox_policy_ref=$5, sandbox_policy_digest=decode($6,'hex'), \
                        privilege_boundary_ref=$7, \
                        privilege_boundary_digest=decode($8,'hex') \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[
                    &admitted_record.task_ref().as_str(),
                    &attempt_number_sql,
                    &admitted_environment.immutable_snapshot_ref(),
                    &admitted_environment.immutable_snapshot_digest().as_str(),
                    &admitted_environment.sandbox_policy_ref(),
                    &admitted_environment.sandbox_policy_digest().as_str(),
                    &admitted_environment.privilege_boundary_ref(),
                    &admitted_environment.privilege_boundary_digest().as_str(),
                ],
            )
            .expect("restore exact typed security-boundary references"),
        1
    );

    let mut canonical_policy_tamper = connect_as(&migrator_url, "lattice_migrator");
    canonical_policy_tamper
        .batch_execute("BEGIN")
        .expect("begin isolated canonical sandbox-policy tamper");
    assert_eq!(
        canonical_policy_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET canonical_descriptor = foreman_execution.canonical_json_v1( \
                            jsonb_set(canonical_descriptor::jsonb, \
                                '{sandbox_policy,policy_digest}', \
                                to_jsonb('wsl2-sandbox-policy:sha256:' || repeat('4',64)))) \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
            )
            .expect("tamper canonical sandbox-policy reference inside transaction"),
        1
    );
    let rejected = canonical_policy_tamper
        .query(
            "SELECT * FROM foreman_execution.read_execution_environment_rows_v1( \
                decode($1,'hex'))",
            &[&admitted_record.task_ref().as_str()],
        )
        .expect_err("fresh reader must reject canonical sandbox-policy tamper");
    assert_eq!(
        rejected
            .as_db_error()
            .map(postgres::error::DbError::message),
        Some("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH")
    );
    canonical_policy_tamper
        .batch_execute("ROLLBACK")
        .expect("rollback isolated canonical sandbox-policy tamper");

    let mut canonical_path_tamper = connect_as(&migrator_url, "lattice_migrator");
    canonical_path_tamper
        .batch_execute("BEGIN")
        .expect("begin isolated canonical path tamper");
    assert_eq!(
        canonical_path_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET canonical_descriptor = foreman_execution.canonical_json_v1( \
                            jsonb_set(canonical_descriptor::jsonb, \
                                '{verification_toolchain,cargo_home}', \
                                to_jsonb((canonical_descriptor::jsonb #>> \
                                    '{verification_toolchain,isolation_root}') || \
                                    '/../cargo-home'))) \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
            )
            .expect("tamper canonical cargo-home path inside transaction"),
        1
    );
    let rejected = canonical_path_tamper
        .query(
            "SELECT * FROM foreman_execution.read_execution_environment_rows_v1( \
                decode($1,'hex'))",
            &[&admitted_record.task_ref().as_str()],
        )
        .expect_err("fresh reader must reject noncanonical durable path tamper");
    assert_eq!(
        rejected
            .as_db_error()
            .map(postgres::error::DbError::message),
        Some("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH")
    );
    canonical_path_tamper
        .batch_execute("ROLLBACK")
        .expect("rollback isolated canonical path tamper");

    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET immutable_probe_lsattr_digest=decode(repeat('4',64),'hex'), \
                        noninteractive_root_probe_digest=decode(repeat('4',64),'hex') \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
            )
            .expect("commit immutable and privilege probe identity substitution"),
        1
    );
    let mut probe_tampered_reader =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &foreman_target)
            .expect("probe-tampered fresh-process reader");
    assert_eq!(
        probe_tampered_reader
            .load_execution_environment(
                admitted_record.task_ref(),
                admitted_record.attempt_number()
            )
            .expect_err("probe identity substitution must fail closed")
            .kind(),
        AdapterErrorKind::CorruptReplay
    );
    drop(probe_tampered_reader);
    assert_eq!(
        environment_tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET immutable_probe_lsattr_digest=decode($3,'hex'), \
                        noninteractive_root_probe_digest=decode($4,'hex') \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[
                    &admitted_record.task_ref().as_str(),
                    &attempt_number_sql,
                    &admitted_environment
                        .immutable_probe_lsattr()
                        .digest()
                        .as_str(),
                    &admitted_environment
                        .noninteractive_root_probe()
                        .digest()
                        .as_str(),
                ],
            )
            .expect("restore exact immutable and privilege probe identities"),
        1
    );
    drop(environment_tamper);

    let mut missing_environment = connect_as(&migrator_url, "lattice_migrator");
    missing_environment
        .batch_execute("BEGIN")
        .expect("begin isolated environment deletion");
    missing_environment
        .execute(
            "DELETE FROM ONLY foreman_execution.execution_environments \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
        )
        .expect("temporarily delete active WSL environment");
    let missing = missing_environment
        .query(
            "SELECT attempt_number \
               FROM foreman_execution.read_execution_environment_rows_v1( \
                    decode($1,'hex'))",
            &[&admitted_record.task_ref().as_str()],
        )
        .expect_err("active non-native missing environment must fail closed");
    assert_eq!(
        missing.as_db_error().map(postgres::error::DbError::message),
        Some("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH")
    );
    missing_environment
        .batch_execute("ROLLBACK")
        .expect("rollback isolated environment deletion");

    let mut missing_claim = connect_as(&migrator_url, "lattice_migrator");
    missing_claim
        .batch_execute("BEGIN")
        .expect("begin isolated active-claim environment deletion");
    missing_claim
        .execute(
            "DELETE FROM ONLY foreman_execution.execution_environments \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
        )
        .expect("temporarily delete environment before active exact claim");
    let missing_claim_rejected =
        replay_active_claim_from_durable_rows(&mut missing_claim, admitted_record.task_ref())
            .expect_err("active exact claim must reject a missing environment row");
    assert_eq!(
        missing_claim_rejected
            .as_db_error()
            .map(postgres::error::DbError::message),
        Some("FOREMAN_EXECUTION_ENVIRONMENT_REQUIRED")
    );
    missing_claim
        .batch_execute("ROLLBACK")
        .expect("rollback isolated active-claim environment deletion");

    let mut missing_reserve = connect_as(&migrator_url, "lattice_migrator");
    missing_reserve
        .batch_execute("BEGIN")
        .expect("begin isolated active-reserve environment deletion");
    missing_reserve
        .execute(
            "DELETE FROM ONLY foreman_execution.execution_environments \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
        )
        .expect("temporarily delete environment before active reserve replay");
    let missing_reserve_rejected = replay_active_reservation_from_durable_rows(
        &mut missing_reserve,
        admitted_record.task_ref(),
    )
    .expect_err("active reserve replay must reject a missing environment row");
    assert_eq!(
        missing_reserve_rejected
            .as_db_error()
            .map(postgres::error::DbError::message),
        Some("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH")
    );
    missing_reserve
        .batch_execute("ROLLBACK")
        .expect("rollback isolated active-reserve environment deletion");
    assert_active_environment_closure_paths_reject_tamper(
        &migrator_url,
        admitted_record,
        admitted_environment.environment_ref().as_str(),
    );

    let mut missing_replay = connect_as(&migrator_url, "lattice_migrator");
    missing_replay
        .batch_execute("BEGIN")
        .expect("begin isolated active replay deletion");
    missing_replay
        .execute(
            "DELETE FROM ONLY foreman_execution.execution_environments \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[&admitted_record.task_ref().as_str(), &attempt_number_sql],
        )
        .expect("temporarily delete environment before active exact replay");
    let repaired = missing_replay
        .query_one(
            "SELECT foreman_execution.record_execution_environment_v1( \
                decode($1,'hex'),$2,$3,decode($4,'hex'),$5,$6)",
            &[
                &admitted_record.task_ref().as_str(),
                &attempt_number_sql,
                &admitted_record.attempt_id().as_str(),
                &admitted_record.packet_digest().as_str(),
                &admitted_environment.canonical_json(),
                &admitted_environment.environment_ref().as_str(),
            ],
        )
        .expect_err("active replay must not silently recreate a deleted environment");
    assert_eq!(
        repaired
            .as_db_error()
            .map(postgres::error::DbError::message),
        Some("FOREMAN_EXECUTION_ENVIRONMENT_SUBSTITUTION")
    );
    missing_replay
        .batch_execute("ROLLBACK")
        .expect("rollback isolated active replay deletion");
    assert_eq!(
        replay
            .load_execution_environment(
                admitted_record.task_ref(),
                admitted_record.attempt_number(),
            )
            .expect("environment after tamper rollback")
            .expect("retained environment after rollback")
            .descriptor(),
        admitted_environment
    );
    assert_eq!(
        replay
            .claim_provider_dispatch(
                admitted_record,
                ProviderDispatchKind::WorkerThread,
                admitted_record.payload_digest(),
                admitted_record.packet_digest(),
                &thread_subject,
            )
            .expect("first exact worker-thread dispatch claim"),
        ClaimDisposition::Claimed
    );
    assert_eq!(
        replay
            .claim_provider_dispatch(
                admitted_record,
                ProviderDispatchKind::WorkerThread,
                admitted_record.payload_digest(),
                admitted_record.packet_digest(),
                &thread_subject,
            )
            .expect("worker-thread exact replay"),
        ClaimDisposition::ExactReplay
    );
    let substitution = replay.claim_provider_dispatch(
        admitted_record,
        ProviderDispatchKind::WorkerThread,
        admitted_record.payload_digest(),
        admitted_record.packet_digest(),
        &digest(99_902),
    );
    assert!(
        substitution.is_err(),
        "changed dispatch subject must fail closed"
    );
    let retained = replay
        .load_provider_dispatch_claim(
            admitted_record.task_ref(),
            admitted_record.attempt_number(),
            ProviderDispatchKind::WorkerThread,
        )
        .expect("provider dispatch reader")
        .expect("retained worker-thread dispatch claim");
    assert_eq!(retained.subject_digest(), &thread_subject);
    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    let writer_row = migrator
        .query_one(
            "SELECT promotion.project_id::text, lease.current_expires_at \
               FROM ONLY foreman_execution.task_promotions AS promotion \
               JOIN ONLY writer_lease.writer_lease_heads AS lease \
                 ON lease.project_id=promotion.project_id \
              WHERE promotion.task_ref=decode($1,'hex')",
            &[&admitted_record.task_ref().as_str()],
        )
        .expect("load exact dispatch Writer");
    let admitted_project: String = writer_row.get(0);
    let admitted_expiry: String = writer_row.get(1);
    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_expires_at='2020-01-01T00:00:00Z' WHERE project_id=$1",
            &[&admitted_project],
        )
        .expect("expire exact dispatch Writer");
    assert_eq!(
        replay
            .claim_provider_dispatch(
                admitted_record,
                ProviderDispatchKind::WorkerThread,
                admitted_record.payload_digest(),
                admitted_record.packet_digest(),
                &thread_subject,
            )
            .expect("expired Writer cannot hide an existing exact dispatch"),
        ClaimDisposition::ExactReplay,
    );
    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads SET current_expires_at=$2 \
              WHERE project_id=$1",
            &[&admitted_project, &admitted_expiry],
        )
        .expect("restore exact dispatch Writer");
    let authority_row = migrator
        .query_one(
            "SELECT authority.expires_at::text \
               FROM ONLY foreman_execution.worker_attempts AS attempt \
               JOIN ONLY foreman_execution.approval_evidence AS authority \
                 ON authority.task_ref=attempt.task_ref \
                AND authority.authority_digest=attempt.approval_receipt_digest \
              WHERE attempt.task_ref=decode($1,'hex') \
                AND attempt.attempt_number=$2",
            &[
                &admitted_record.task_ref().as_str(),
                &i16::try_from(admitted_record.attempt_number()).expect("attempt number"),
            ],
        )
        .expect("load exact dispatch authority expiry");
    let admitted_authority_expiry: String = authority_row.get(0);
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_evidence \
                SET expires_at='2026-01-01T00:00:01Z' \
              WHERE task_ref=decode($1,'hex') \
                AND authority_digest=decode($2,'hex')",
            &[
                &admitted_record.task_ref().as_str(),
                &admitted_record.approval_receipt_digest().as_str(),
            ],
        )
        .expect("expire exact dispatch authority");
    assert_eq!(
        replay
            .claim_provider_dispatch(
                admitted_record,
                ProviderDispatchKind::WorkerThread,
                admitted_record.payload_digest(),
                admitted_record.packet_digest(),
                &thread_subject,
            )
            .expect("expired authority cannot hide an existing exact dispatch"),
        ClaimDisposition::ExactReplay,
    );
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_evidence SET expires_at=$3 \
              WHERE task_ref=decode($1,'hex') \
                AND authority_digest=decode($2,'hex')",
            &[
                &admitted_record.task_ref().as_str(),
                &admitted_record.approval_receipt_digest().as_str(),
                &admitted_authority_expiry,
            ],
        )
        .expect("restore exact dispatch authority expiry");
    let dispatch_replay = replay
        .read_task_replay(admitted_record.task_ref())
        .expect("provider dispatch must be represented in replay");
    assert!(dispatch_replay.records().iter().any(|record| {
        record.record_kind() == "PROVIDER_DISPATCH_WORKER_THREAD"
            && record.record_state() == ReplayRecordState::Retained
    }));

    let gated_record = records
        .iter()
        .enumerate()
        .find(|(index, _)| *index != successful_index && results[*index].is_ok())
        .map(|(_, record)| record)
        .expect("second admitted fixture for Writer/runtime dispatch gates");
    assert_provider_dispatch_rejects_stale_authority_and_registry(
        &mut replay,
        &migrator_url,
        gated_record,
    );
    assert_provider_dispatch_rejects_stale_writer_and_runtime(
        &mut replay,
        &migrator_url,
        gated_record,
    );
    let mut drift_migrator = connect_as(&migrator_url, "lattice_migrator");
    let writer_row = drift_migrator
        .query_one(
            "SELECT promotion.project_id::text, lease.current_worktree_id::text, \
                    lease.current_status::text, lease.current_expires_at \
               FROM ONLY foreman_execution.task_promotions AS promotion \
               JOIN ONLY writer_lease.writer_lease_heads AS lease \
                 ON lease.project_id=promotion.project_id \
              WHERE promotion.task_ref=decode($1,'hex')",
            &[&gated_record.task_ref().as_str()],
        )
        .expect("load active attempt Writer head");
    let writer_project_id: String = writer_row.get(0);
    let original_worktree_id: String = writer_row.get(1);
    let original_writer_status: String = writer_row.get(2);
    let original_writer_expiry: String = writer_row.get(3);
    assert_eq!(original_writer_status, "ACTIVE");
    drift_migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_worktree_id='WORK-DRIFT' WHERE project_id=$1",
            &[&writer_project_id],
        )
        .expect("introduce active attempt Writer head drift");
    assert_restart_candidate_kind(
        &runtime_url,
        &foreman_target,
        gated_record,
        RestartTaskKind::WriterReconciliationRequired,
        "active Writer head drift",
    );
    drift_migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_worktree_id=$2 WHERE project_id=$1",
            &[&writer_project_id, &original_worktree_id],
        )
        .expect("restore active attempt Writer head");
    drift_migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_status='SUSPECT' WHERE project_id=$1",
            &[&writer_project_id],
        )
        .expect("introduce active attempt Writer suspect status");
    assert_restart_candidate_kind(
        &runtime_url,
        &foreman_target,
        gated_record,
        RestartTaskKind::WriterReconciliationRequired,
        "active Writer suspect status",
    );
    drift_migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_status=$2 WHERE project_id=$1",
            &[&writer_project_id, &original_writer_status],
        )
        .expect("restore active attempt Writer status");
    drift_migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_expires_at='2020-01-01T00:00:00Z' WHERE project_id=$1",
            &[&writer_project_id],
        )
        .expect("expire active attempt Writer");
    assert_restart_candidate_kind(
        &runtime_url,
        &foreman_target,
        gated_record,
        RestartTaskKind::WriterReconciliationRequired,
        "expired active Writer",
    );
    drift_migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_expires_at=$2 WHERE project_id=$1",
            &[&writer_project_id, &original_writer_expiry],
        )
        .expect("restore active attempt Writer expiry");
    let original_admission_mode: String = drift_migrator
        .query_one(
            "SELECT admission_mode::text FROM ONLY control.runtime_admission WHERE singleton=true",
            &[],
        )
        .expect("load active runtime admission")
        .get(0);
    assert_eq!(original_admission_mode, "ACTIVE");
    drift_migrator
        .execute(
            "UPDATE ONLY control.runtime_admission SET admission_mode='DRAINING' WHERE singleton=true",
            &[],
        )
        .expect("drain active runtime admission");
    assert_restart_candidate_kind(
        &runtime_url,
        &foreman_target,
        gated_record,
        RestartTaskKind::WriterReconciliationRequired,
        "non-active runtime admission",
    );
    drift_migrator
        .execute(
            "UPDATE ONLY control.runtime_admission SET admission_mode=$1 WHERE singleton=true",
            &[&original_admission_mode],
        )
        .expect("restore active runtime admission");
    let stream_record = records
        .iter()
        .enumerate()
        .find(|(index, record)| {
            *index != successful_index
                && results[*index].is_ok()
                && record.task_ref() != gated_record.task_ref()
        })
        .map(|(_, record)| record)
        .expect("distinct admitted fixture for live provider-claim lock barriers");
    assert_provider_dispatch_serializes_authority_mutations(
        &migrator_url,
        &runtime_url,
        &foreman_target,
        gated_record,
        stream_record,
    );

    let mut foreman_ledger =
        PostgresTaskLedger::new(connect_store_runtime(&runtime_url), &store_target)
            .expect("formal Foreman checkpoint ledger");
    let newer_checkpoint =
        append_formal_foreman_checkpoint(&mut foreman_ledger, &authority, &foreman_writer, 2);
    assert_ne!(newer_checkpoint, foreman_checkpoint);
    assert_eq!(
        replay
            .claim_provider_dispatch(
                admitted_record,
                ProviderDispatchKind::WorkerThread,
                admitted_record.payload_digest(),
                admitted_record.packet_digest(),
                &thread_subject,
            )
            .expect("newer Foreman generation cannot hide an existing exact dispatch"),
        ClaimDisposition::ExactReplay,
    );
    let stale_foreman_record = records
        .iter()
        .enumerate()
        .find(|(index, record)| {
            *index != successful_index
                && results[*index].is_ok()
                && record.task_ref() != gated_record.task_ref()
                && record.task_ref() != stream_record.task_ref()
        })
        .map(|(_, record)| record)
        .expect("second admitted fixture for stale Foreman fence");
    let stale_foreman = replay
        .claim_provider_dispatch(
            stale_foreman_record,
            ProviderDispatchKind::WorkerThread,
            stale_foreman_record.payload_digest(),
            stale_foreman_record.packet_digest(),
            &digest(99_903),
        )
        .expect_err("stale formal Foreman generation/checkpoint must reject dispatch");
    assert_eq!(
        stale_foreman.kind(),
        lattice_postgres_foreman::AdapterErrorKind::ClaimRejected
    );
    assert_eq!(stale_foreman.code(), "FOREMAN_PROVIDER_DISPATCH_REJECTED");

    let changed_limit = replay.claim_worker_attempt_with_execution_environment_ref(
        admitted_record,
        2,
        admitted_environment.environment_ref().as_str(),
    );
    assert!(
        changed_limit.is_err(),
        "same task/attempt with a different p_max_attempts must fail closed"
    );
    let active = replay
        .list_active_task_refs(256)
        .expect("active claims after rejected replay");
    assert_eq!(active.len(), 4, "rejected replay cannot mutate active rows");
    let waiting_index = results
        .iter()
        .position(Result::is_err)
        .expect("one capacity waiter");
    let waiting_record = &records[waiting_index];
    let waiting_environment = environments[waiting_index]
        .as_ref()
        .expect("capacity waiter is non-native after the native attempt was claimed first");
    let pending = replay
        .load_pending_worker_attempt(waiting_record.task_ref())
        .expect("pending reader")
        .expect("capacity rejection keeps exact pending claim");
    assert_eq!(pending.max_attempts(), 3);
    assert_eq!(
        replay
            .reserve_worker_attempt_with_execution_environment_ref(
                waiting_record,
                3,
                waiting_environment.environment_ref().as_str(),
            )
            .expect("pending exact reservation replay validates its environment row"),
        ClaimReservationDisposition::ExactReplay
    );
    assert_pending_reserve_replay_rejects_wrong_environment(&migrator_url, waiting_record);
    let mut pending_drift = connect_as(&migrator_url, "lattice_migrator");
    let pending_project = pending_drift
        .query_one(
            "SELECT promotion.project_id::text, project.drift_repository \
               FROM ONLY foreman_execution.task_promotions AS promotion \
               JOIN ONLY control.project_registry_projects AS project \
                 ON project.project_id=promotion.project_id \
              WHERE promotion.task_ref=decode($1,'hex')",
            &[&waiting_record.task_ref().as_str()],
        )
        .expect("load pending-attempt Project Registry identity");
    let pending_project_id: String = pending_project.get(0);
    let pending_original_drift: bool = pending_project.get(1);
    assert!(!pending_original_drift);
    pending_drift
        .execute(
            "UPDATE ONLY control.project_registry_projects \
                SET drift_repository=true WHERE project_id=$1",
            &[&pending_project_id],
        )
        .expect("introduce pending-attempt Project Registry drift");
    assert_restart_candidate_kind(
        &runtime_url,
        &foreman_target,
        waiting_record,
        RestartTaskKind::ProjectReconciliationRequired,
        "pending-attempt Project Registry drift",
    );
    assert_no_provider_claim(&runtime_url, &foreman_target, waiting_record);
    pending_drift
        .execute(
            "UPDATE ONLY control.project_registry_projects \
                SET drift_repository=$2 WHERE project_id=$1",
            &[&pending_project_id, &pending_original_drift],
        )
        .expect("restore pending-attempt Project Registry drift");
    let restart = collect_restart_pages(&mut replay, 2);
    let restart = restart
        .iter()
        .filter(|candidate| fixture_refs.contains(candidate.task_ref().as_str()))
        .collect::<Vec<_>>();
    assert_eq!(restart.len(), 5);
    assert_eq!(
        restart
            .iter()
            .map(|candidate| candidate.restart_priority())
            .collect::<Vec<_>>(),
        vec![3, 3, 3, 3, 4],
        "active reconciliation must precede the capacity waiter across pages"
    );
    assert_eq!(
        restart
            .iter()
            .filter(|candidate| candidate.restart_kind() == RestartTaskKind::CapacityWait)
            .count(),
        1
    );
    let pending_replay = replay
        .read_task_replay(waiting_record.task_ref())
        .expect("pending replay");
    assert!(pending_replay.records().iter().any(|record| {
        record.record_kind() == "WORKER_ATTEMPT"
            && record.record_state() == ReplayRecordState::PendingClaim
    }));
    let pending_authority_blocker = persist_blocker_artifact(
        &mut replay,
        &mut foreman_ledger,
        &authority,
        &fixtures[waiting_index],
        "authority-expired",
        5,
        "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT",
        "TASK_BOUND_EXECUTION_AUTHORITY_NOT_CURRENT",
    );
    for tamper in [
        PendingEnvironmentTamper::MissingRow,
        PendingEnvironmentTamper::WrongPacket,
        PendingEnvironmentTamper::NativeRefWithRow,
    ] {
        assert_pending_close_environment_tamper_rejected(
            &migrator_url,
            waiting_record,
            "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT",
            pending_authority_blocker.descriptor_digest(),
            tamper,
        );
    }
    assert_eq!(
        replay
            .close_pending_worker_attempt(
                waiting_record.task_ref(),
                u8::try_from(waiting_record.attempt_number()).expect("bounded waiting attempt"),
                "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT",
                pending_authority_blocker.descriptor_digest(),
                waiting_record.writer_fence(),
            )
            .expect("close reserved attempt after stale authority"),
        AppendDisposition::Inserted
    );
    assert_eq!(
        replay
            .close_pending_worker_attempt(
                waiting_record.task_ref(),
                u8::try_from(waiting_record.attempt_number()).expect("bounded waiting attempt"),
                "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT",
                pending_authority_blocker.descriptor_digest(),
                waiting_record.writer_fence(),
            )
            .expect("replay stale-authority pending closure"),
        AppendDisposition::ExactReplay
    );
    let pending_closure = replay
        .load_attempt_closure(
            waiting_record.task_ref(),
            u8::try_from(waiting_record.attempt_number()).expect("bounded waiting attempt"),
        )
        .expect("load pending stale-authority closure")
        .expect("pending stale-authority closure retained");
    assert_eq!(
        pending_closure.blocker_code(),
        "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"
    );

    let staged_index = results
        .iter()
        .enumerate()
        .find(|(index, result)| *index != successful_index && result.is_ok())
        .map(|(index, _)| index)
        .expect("second admitted fixture for staged artifact crash window");
    let quota_indices = results
        .iter()
        .enumerate()
        .filter(|(index, result)| {
            *index != staged_index
                && result.is_ok()
                && records[*index].task_ref() != stale_foreman_record.task_ref()
        })
        .map(|(index, _)| index)
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(
        quota_indices.len(),
        2,
        "two admitted non-failpoint tasks exercise artifact quotas"
    );
    assert_artifact_quota_boundaries(
        &runtime_url,
        &foreman_target,
        &mut foreman_ledger,
        &authority,
        &fixtures[quota_indices[0]],
        &fixtures[quota_indices[1]],
    );
    let worker_probe_fixture = fixtures
        .iter()
        .find(|fixture| fixture.record.task_ref() == stale_foreman_record.task_ref())
        .expect("worker probe closure fixture");
    let worker_probe_blocker = persist_blocker_artifact(
        &mut replay,
        &mut foreman_ledger,
        &authority,
        worker_probe_fixture,
        "worker-probe-timeout",
        5,
        "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED",
        "WORKER_MODEL_PROBE_TIMED_OUT_EXACT_PRESTART_SUBTREE_REAPED",
    );
    assert_eq!(
        replay
            .finalize_staged_artifact_reference(
                stale_foreman_record.task_ref(),
                u8::try_from(stale_foreman_record.attempt_number())
                    .expect("bounded worker probe attempt"),
                worker_probe_blocker.descriptor_digest(),
            )
            .expect("finalize worker probe blocker"),
        AppendDisposition::Inserted
    );
    assert_eq!(
        replay
            .record_attempt_closure(
                stale_foreman_record.task_ref(),
                u8::try_from(stale_foreman_record.attempt_number())
                    .expect("bounded worker probe attempt"),
                "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED",
                worker_probe_blocker.descriptor_digest(),
                stale_foreman_record.writer_fence(),
            )
            .expect("close worker probe timeout with proven no provider effect"),
        AppendDisposition::Inserted
    );
    assert_eq!(
        replay
            .record_attempt_closure(
                stale_foreman_record.task_ref(),
                u8::try_from(stale_foreman_record.attempt_number())
                    .expect("bounded worker probe attempt"),
                "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED",
                worker_probe_blocker.descriptor_digest(),
                stale_foreman_record.writer_fence(),
            )
            .expect("replay worker probe timeout closure"),
        AppendDisposition::ExactReplay
    );
    let staged_fixture = &fixtures[staged_index];
    let staged_attempt = &staged_fixture.record;
    let staged_stream = foreman_ledger
        .load_stream(staged_fixture.successor_identity.clone())
        .expect("load staged artifact successor")
        .stream()
        .clone();
    let staged_evidence = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            staged_stream.identity().project_id().clone(),
            staged_attempt.task_ref().clone(),
            u8::try_from(staged_attempt.attempt_number()).expect("bounded staged attempt"),
            ManagedEvidenceKind::ResourceObservation,
            "application/json",
            "lattice.foreman-artifact-outbox-failpoint/1.0",
            "lattice-postgres-foreman-live",
            "1",
            staged_attempt.foreman_checkpoint_digest().clone(),
            timestamp(staged_fixture.task_number, 4),
            br#"{"schema":"lattice.foreman-artifact-outbox-failpoint.v1","status":"staged"}"#
                .to_vec(),
        )
        .expect("bounded staged artifact input"),
    )
    .expect("verified staged artifact");
    let staged_metadata = metadata("artifact", staged_fixture.task_number, 4);
    let staged_correlation = correlation_id(staged_fixture.task_number);
    let staged_plan = plan_artifact_reference_append(
        &staged_stream,
        &staged_fixture.binding,
        std::slice::from_ref(staged_attempt),
        &[],
        staged_metadata,
        staged_attempt.attempt_number(),
        staged_evidence.descriptor_digest().clone(),
    )
    .expect("plan staged artifact reference");
    assert_eq!(
        replay
            .stage_artifact_reference(
                &staged_evidence,
                staged_plan.link(),
                &staged_correlation,
                &timestamp(staged_fixture.task_number, 4),
            )
            .expect("durably stage exact artifact intent"),
        lattice_postgres_foreman::AppendDisposition::Inserted
    );
    foreman_ledger
        .execute(
            staged_plan.ledger_plan().command_record().request().clone(),
            authority.clone(),
        )
        .expect("append artifact Ledger event before failpoint");
    let retained_stage = replay
        .load_staged_artifact_reference(staged_attempt.task_ref())
        .expect("load retained staged artifact")
        .expect("stage must survive before finalization");
    assert_eq!(retained_stage.evidence(), &staged_evidence);
    assert_eq!(retained_stage.link(), staged_plan.link());
    assert!(
        replay
            .load_managed_evidence(
                staged_attempt.task_ref(),
                u8::try_from(staged_attempt.attempt_number()).expect("bounded staged attempt"),
            )
            .expect("formal evidence remains absent before finalize")
            .is_empty()
    );
    assert!(
        replay
            .load_reference_links(staged_attempt.task_ref())
            .expect("formal reference remains absent before finalize")
            .artifact_links()
            .is_empty()
    );

    let native_closure_fixture = build_claim_fixture(
        &mut foreman_ledger,
        &authority,
        &runtime_url,
        &foreman_target,
        &store_target,
        &budget,
        1,
        &newer_checkpoint,
        6,
    );
    let native_closure_record = &native_closure_fixture.record;
    assert_eq!(
        replay
            .reserve_worker_attempt(native_closure_record, 3)
            .expect("reserve native pending-closure fixture"),
        ClaimReservationDisposition::Reserved
    );
    assert!(
        replay
            .load_execution_environment(
                native_closure_record.task_ref(),
                native_closure_record.attempt_number(),
            )
            .expect("native pending-closure environment lookup")
            .is_none()
    );
    let native_closure_blocker = persist_blocker_artifact(
        &mut replay,
        &mut foreman_ledger,
        &authority,
        &native_closure_fixture,
        "native-authority-expired",
        5,
        "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT",
        "TASK_BOUND_EXECUTION_AUTHORITY_NOT_CURRENT",
    );
    assert_eq!(
        replay
            .close_pending_worker_attempt(
                native_closure_record.task_ref(),
                u8::try_from(native_closure_record.attempt_number())
                    .expect("bounded native closure attempt"),
                "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT",
                native_closure_blocker.descriptor_digest(),
                native_closure_record.writer_fence(),
            )
            .expect("native pending closure stays environment-row-free"),
        AppendDisposition::Inserted
    );
    assert!(
        replay
            .load_pending_worker_attempt(native_closure_record.task_ref())
            .expect("native pending row after closure")
            .is_none()
    );
    assert!(
        replay
            .load_execution_environment(
                native_closure_record.task_ref(),
                native_closure_record.attempt_number(),
            )
            .expect("native retained environment lookup")
            .is_none()
    );

    migrator
        .execute(
            "UPDATE ONLY foreman_execution.provider_dispatch_claims \
                SET claimed_at=claimed_at + interval '1 second' \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2 \
                AND operation_kind='WORKER_THREAD'",
            &[
                &admitted_record.task_ref().as_str(),
                &i16::try_from(admitted_record.attempt_number()).expect("attempt"),
            ],
        )
        .expect("tamper dispatch claim time");
    let tampered = replay
        .load_provider_dispatch_claim(
            admitted_record.task_ref(),
            admitted_record.attempt_number(),
            ProviderDispatchKind::WorkerThread,
        )
        .expect_err("timestamp substitution must invalidate the durable claim receipt");
    assert_eq!(
        tampered.kind(),
        lattice_postgres_foreman::AdapterErrorKind::CorruptReplay
    );
}

#[derive(Clone, Copy, Debug)]
enum PendingEnvironmentTamper {
    MissingRow,
    WrongPacket,
    NativeRefWithRow,
}

#[derive(Clone, Copy, Debug)]
enum ActiveEnvironmentClosurePath {
    Direct,
    RetainedNoEffect,
}

#[derive(Clone, Copy, Debug)]
enum ActiveEnvironmentTamper {
    MissingRow,
    WrongPacket,
}

fn assert_active_environment_closure_paths_reject_tamper(
    migrator_url: &str,
    record: &VerifiedWorkerAttemptRecord,
    environment_ref: &str,
) {
    let attempt_number =
        i16::try_from(record.attempt_number()).expect("bounded active closure attempt");
    let writer_fence = i64::try_from(record.writer_fence()).expect("bounded writer fence");
    let blocker_digest = digest(99_904);
    let proof_digest = digest(99_905);
    for closure_path in [
        ActiveEnvironmentClosurePath::Direct,
        ActiveEnvironmentClosurePath::RetainedNoEffect,
    ] {
        for tamper_kind in [
            ActiveEnvironmentTamper::MissingRow,
            ActiveEnvironmentTamper::WrongPacket,
        ] {
            let mut tamper = connect_as(migrator_url, "lattice_migrator");
            tamper
                .batch_execute("BEGIN")
                .expect("begin active closure environment tamper");
            let changed = match tamper_kind {
                ActiveEnvironmentTamper::MissingRow => tamper.execute(
                    "DELETE FROM ONLY foreman_execution.execution_environments \
                      WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                    &[&record.task_ref().as_str(), &attempt_number],
                ),
                ActiveEnvironmentTamper::WrongPacket => tamper.execute(
                    "UPDATE ONLY foreman_execution.execution_environments \
                        SET packet_digest=CASE \
                            WHEN packet_digest=decode(repeat('ab',32),'hex') \
                            THEN decode(repeat('ac',32),'hex') \
                            ELSE decode(repeat('ab',32),'hex') END \
                      WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                    &[&record.task_ref().as_str(), &attempt_number],
                ),
            }
            .expect("tamper active closure execution environment");
            assert_eq!(changed, 1, "active closure tamper must affect one row");
            let rejected = match closure_path {
                ActiveEnvironmentClosurePath::Direct => tamper.query_one(
                    "SELECT foreman_execution.record_attempt_closure_v1( \
                        decode($1,'hex'),$2,$3,decode($4,'hex'),$5)",
                    &[
                        &record.task_ref().as_str(),
                        &attempt_number,
                        &"LATTICE_MANAGED_MODEL_UNAVAILABLE",
                        &blocker_digest.as_str(),
                        &writer_fence,
                    ],
                ),
                ActiveEnvironmentClosurePath::RetainedNoEffect => tamper.query_one(
                    "SELECT foreman_execution.close_retained_worker_without_provider_effect_v1( \
                        decode($1,'hex'),$2,$3,decode($4,'hex'),decode($5,'hex'),$6)",
                    &[
                        &record.task_ref().as_str(),
                        &attempt_number,
                        &"LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL",
                        &blocker_digest.as_str(),
                        &proof_digest.as_str(),
                        &writer_fence,
                    ],
                ),
            }
            .expect_err("active closure must reject a missing or substituted environment anchor");
            assert_eq!(
                rejected
                    .as_db_error()
                    .map(postgres::error::DbError::message),
                Some("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH"),
                "unexpected {closure_path:?}/{tamper_kind:?} rejection"
            );
            tamper
                .batch_execute("ROLLBACK")
                .expect("rollback active closure environment tamper");
            let state = tamper
                .query_one(
                    "SELECT \
                        (SELECT pg_catalog.count(*) \
                           FROM ONLY foreman_execution.worker_attempts \
                          WHERE task_ref=decode($1,'hex') AND attempt_number=$2), \
                        (SELECT pg_catalog.count(*) \
                           FROM ONLY foreman_execution.attempt_closures \
                          WHERE task_ref=decode($1,'hex') AND attempt_number=$2), \
                        (SELECT pg_catalog.count(*) \
                           FROM ONLY foreman_execution.execution_environments \
                          WHERE task_ref=decode($1,'hex') AND attempt_number=$2 \
                            AND attempt_id=$3 AND packet_digest=decode($4,'hex') \
                            AND environment_ref=$5)",
                    &[
                        &record.task_ref().as_str(),
                        &attempt_number,
                        &record.attempt_id().as_str(),
                        &record.packet_digest().as_str(),
                        &environment_ref,
                    ],
                )
                .expect("read active closure state after tamper rollback");
            assert_eq!(state.get::<_, i64>(0), 1, "active row must remain");
            assert_eq!(state.get::<_, i64>(1), 0, "closure must remain absent");
            assert_eq!(
                state.get::<_, i64>(2),
                1,
                "exact execution environment must be restored"
            );
        }
    }
}

fn assert_pending_reserve_replay_rejects_wrong_environment(
    migrator_url: &str,
    record: &VerifiedWorkerAttemptRecord,
) {
    let mut tamper = connect_as(migrator_url, "lattice_migrator");
    tamper
        .batch_execute("BEGIN")
        .expect("begin pending reserve environment tamper");
    assert_eq!(
        tamper
            .execute(
                "UPDATE ONLY foreman_execution.execution_environments \
                    SET packet_digest=CASE \
                        WHEN packet_digest=decode(repeat('ab',32),'hex') \
                        THEN decode(repeat('ac',32),'hex') \
                        ELSE decode(repeat('ab',32),'hex') END \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
                &[
                    &record.task_ref().as_str(),
                    &i16::try_from(record.attempt_number()).expect("attempt number"),
                ],
            )
            .expect("tamper pending reserve environment packet"),
        1
    );
    let rejected = tamper
        .query_one(
            "SELECT foreman_execution.reserve_worker_attempt_v1( \
                pending.task_ref,pending.successor_stream_id,pending.task_spec_digest, \
                pending.binding_digest,pending.budget_digest,pending.attempt_id, \
                pending.attempt_number,pending.foreman_generation,pending.model, \
                pending.reasoning,pending.writer_fence,pending.foreman_checkpoint_digest, \
                pending.approval_receipt_digest,pending.packet_digest, \
                pending.execution_environment_ref,pending.worktree_digest, \
                pending.base_commit_digest,pending.model_reason,pending.model_reason_digest, \
                pending.claimed_at,pending.payload_digest,pending.max_attempts, \
                child.ledger_stream_id,child.ledger_event_sequence, \
                child.ledger_event_digest,child.ledger_command_id, \
                child.ledger_request_digest) \
               FROM ONLY foreman_execution.pending_worker_claims AS pending \
               JOIN ONLY foreman_execution.child_events AS child \
                 ON child.ledger_event_digest=pending.ledger_event_digest \
              WHERE pending.task_ref=decode($1,'hex')",
            &[&record.task_ref().as_str()],
        )
        .expect_err("pending exact reservation replay must reject a wrong environment row");
    assert_eq!(
        rejected
            .as_db_error()
            .map(postgres::error::DbError::message),
        Some("FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH")
    );
    tamper
        .batch_execute("ROLLBACK")
        .expect("rollback pending reserve environment tamper");
    assert_pending_transition_state(&mut tamper, record, 0);
}

fn assert_pending_close_environment_tamper_rejected(
    migrator_url: &str,
    record: &VerifiedWorkerAttemptRecord,
    blocker_code: &str,
    blocker_descriptor_digest: &ContentDigest,
    tamper_kind: PendingEnvironmentTamper,
) {
    let mut tamper = connect_as(migrator_url, "lattice_migrator");
    tamper
        .batch_execute("BEGIN")
        .expect("begin pending closure environment tamper");
    let changed = match tamper_kind {
        PendingEnvironmentTamper::MissingRow => tamper.execute(
            "DELETE FROM ONLY foreman_execution.execution_environments \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
            ],
        ),
        PendingEnvironmentTamper::WrongPacket => tamper.execute(
            "UPDATE ONLY foreman_execution.execution_environments \
                SET packet_digest=CASE \
                    WHEN packet_digest=decode(repeat('ab',32),'hex') \
                    THEN decode(repeat('ac',32),'hex') \
                    ELSE decode(repeat('ab',32),'hex') END \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
            ],
        ),
        PendingEnvironmentTamper::NativeRefWithRow => tamper.execute(
            "UPDATE ONLY foreman_execution.pending_worker_claims \
                SET execution_environment_ref=$3 \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
                &NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
            ],
        ),
    }
    .expect("apply pending closure environment tamper");
    assert_eq!(changed, 1, "tamper fixture must change exactly one row");

    let rejected = tamper
        .query_one(
            "SELECT foreman_execution.close_pending_worker_attempt_v1( \
                decode($1,'hex'),$2,$3,decode($4,'hex'),$5)",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
                &blocker_code,
                &blocker_descriptor_digest.as_str(),
                &i64::try_from(record.writer_fence()).expect("writer fence"),
            ],
        )
        .expect_err("pending closure must reject a missing or substituted environment anchor");
    let expected = match tamper_kind {
        PendingEnvironmentTamper::MissingRow => {
            "FOREMAN_PENDING_CLOSURE_EXECUTION_ENVIRONMENT_REQUIRED"
        }
        PendingEnvironmentTamper::WrongPacket | PendingEnvironmentTamper::NativeRefWithRow => {
            "FOREMAN_EXECUTION_ENVIRONMENT_ANCHOR_MISMATCH"
        }
    };
    assert_eq!(
        rejected
            .as_db_error()
            .map(postgres::error::DbError::message),
        Some(expected)
    );
    tamper
        .batch_execute("ROLLBACK")
        .expect("rollback pending closure environment tamper");
    assert_pending_transition_state(&mut tamper, record, 1);
}

fn assert_pending_transition_state(
    migrator: &mut Client,
    record: &VerifiedWorkerAttemptRecord,
    expected_stage_count: i64,
) {
    let row = migrator
        .query_one(
            "SELECT \
                (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.pending_worker_claims \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2), \
                (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_attempts \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2), \
                (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.attempt_closures \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2), \
                (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.staged_artifact_references \
                  WHERE task_ref=decode($1,'hex') AND attempt_number=$2), \
                (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.execution_environments AS environment \
                  JOIN ONLY foreman_execution.pending_worker_claims AS pending \
                    ON pending.task_ref=environment.task_ref \
                   AND pending.attempt_number=environment.attempt_number \
                   AND pending.attempt_id=environment.attempt_id \
                   AND pending.packet_digest=environment.packet_digest \
                   AND pending.execution_environment_ref=environment.environment_ref \
                 WHERE environment.task_ref=decode($1,'hex') \
                   AND environment.attempt_number=$2)",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
            ],
        )
        .expect("read pending transition state after rollback");
    assert_eq!(row.get::<_, i64>(0), 1, "pending row must remain");
    assert_eq!(row.get::<_, i64>(1), 0, "active row must remain absent");
    assert_eq!(row.get::<_, i64>(2), 0, "closure row must remain absent");
    assert_eq!(row.get::<_, i64>(3), expected_stage_count);
    assert_eq!(
        row.get::<_, i64>(4),
        1,
        "exact environment row must be restored"
    );
}

#[test]
#[ignore = "requires the staged-artifact failpoint left by the ordered disposable PostgreSQL 17 acceptance"]
#[allow(clippy::too_many_lines)]
fn disposable_store_v7_fresh_process_staged_artifact_finalize_replay() {
    if env::var("LATTICE_FOREMAN_LIVE").ok().as_deref() != Some("1") {
        return;
    }
    let migrator_url = required("LATTICE_FOREMAN_MIGRATOR_URL");
    let runtime_url = required("LATTICE_FOREMAN_RUNTIME_URL");
    let database_name = required("LATTICE_FOREMAN_DATABASE_NAME");
    let run_id = required("LATTICE_FOREMAN_RUN_ID");
    let target = ExtensionTarget::new(database_name, run_id).expect("bounded outbox target");

    let mut migrator = connect_as(&migrator_url, "lattice_migrator");
    assert!(matches!(
        apply_extension(&mut migrator, &target).expect("post-restart extension replay"),
        ExtensionApplyOutcome::AlreadyCurrent(_)
    ));
    let staged_rows = migrator
        .query(
            "SELECT pg_catalog.encode(task_ref,'hex'), \
                    pg_catalog.encode(descriptor_digest,'hex') \
               FROM ONLY foreman_execution.staged_artifact_references \
              ORDER BY task_ref",
            &[],
        )
        .expect("read disposable staged failpoint row");
    assert_eq!(
        staged_rows.len(),
        1,
        "one task may retain one bounded stage"
    );
    let task_ref =
        ContentDigest::from_sha256(staged_rows[0].get::<_, String>(0)).expect("staged task ref");
    let descriptor =
        ContentDigest::from_sha256(staged_rows[0].get::<_, String>(1)).expect("staged descriptor");
    drop(migrator);

    let mut adapter = PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
        .expect("fresh-process staged adapter");
    let staged = adapter
        .load_staged_artifact_reference(&task_ref)
        .expect("load staged artifact after PostgreSQL restart")
        .expect("staged artifact survived PostgreSQL restart");
    assert_eq!(staged.evidence().descriptor_digest(), &descriptor);
    assert_eq!(staged.link().payload_digest(), &descriptor);
    assert!(
        adapter
            .load_reference_links(&task_ref)
            .expect("pre-finalize formal links")
            .artifact_links()
            .is_empty()
    );

    let attempt_substitution = adapter
        .finalize_staged_artifact_reference(&task_ref, 2, &descriptor)
        .expect_err("changed attempt cannot consume retained stage");
    assert_eq!(
        attempt_substitution.code(),
        "FOREMAN_ARTIFACT_STAGE_SUBSTITUTION"
    );
    assert!(
        adapter
            .load_staged_artifact_reference(&task_ref)
            .expect("stage remains after rejected attempt substitution")
            .is_some()
    );

    let substitution = adapter
        .finalize_staged_artifact_reference(&task_ref, staged.evidence().attempt(), &digest(99_999))
        .expect_err("changed descriptor cannot consume retained stage");
    assert_eq!(substitution.code(), "FOREMAN_ARTIFACT_STAGE_SUBSTITUTION");
    assert!(
        adapter
            .load_staged_artifact_reference(&task_ref)
            .expect("stage remains after rejected substitution")
            .is_some()
    );

    assert_eq!(
        adapter
            .finalize_staged_artifact_reference(
                &task_ref,
                staged.evidence().attempt(),
                &descriptor,
            )
            .expect("atomically finalize staged artifact"),
        lattice_postgres_foreman::AppendDisposition::Inserted
    );
    assert!(
        adapter
            .load_staged_artifact_reference(&task_ref)
            .expect("stage reader after finalization")
            .is_none()
    );
    assert_eq!(
        adapter
            .finalize_staged_artifact_reference(
                &task_ref,
                staged.evidence().attempt(),
                &descriptor,
            )
            .expect("commit-unknown exact finalize replay"),
        lattice_postgres_foreman::AppendDisposition::ExactReplay
    );
    assert_eq!(
        adapter
            .stage_artifact_reference(
                staged.evidence(),
                staged.link(),
                staged.correlation_id(),
                staged.command_occurred_at(),
            )
            .expect("fully bound finalized stage replay"),
        lattice_postgres_foreman::AppendDisposition::ExactReplay
    );
    let changed_correlation = CorrelationId::new("foreman-artifact-substituted-correlation")
        .expect("changed correlation");
    assert_eq!(
        adapter
            .stage_artifact_reference(
                staged.evidence(),
                staged.link(),
                &changed_correlation,
                staged.command_occurred_at(),
            )
            .expect_err("finalized replay cannot substitute correlation")
            .code(),
        "FOREMAN_ARTIFACT_STAGE_SUBSTITUTION"
    );
    assert_eq!(
        adapter
            .stage_artifact_reference(
                staged.evidence(),
                staged.link(),
                staged.correlation_id(),
                "2026-08-27T23:59:59Z",
            )
            .expect_err("finalized replay cannot substitute occurred-at")
            .code(),
        "FOREMAN_ARTIFACT_STAGE_SUBSTITUTION"
    );
    let exact_head = staged.link().expected_head();
    let changed_head = TaskLedgerStreamHead::new(
        exact_head.version(),
        exact_head.producer_id(),
        exact_head.producer_version(),
        exact_head.runtime(),
        exact_head.identity().clone(),
        exact_head.stream_id().clone(),
        exact_head.sequence(),
        exact_head.last_event_digest().clone(),
        exact_head.resource_revision(),
        exact_head.resource_projection_digest().clone(),
        digest(99_998),
    )
    .expect("self-consistent changed replay head");
    let changed_link = TaskRuntimeEventLink::new(
        changed_head,
        staged.link().stream_id().clone(),
        staged.link().event_sequence(),
        staged.link().event_digest().clone(),
        staged.link().command_id().clone(),
        staged.link().request_digest().clone(),
        staged.link().payload_digest().clone(),
    );
    assert_eq!(
        adapter
            .stage_artifact_reference(
                staged.evidence(),
                &changed_link,
                staged.correlation_id(),
                staged.command_occurred_at(),
            )
            .expect_err("finalized replay cannot substitute expected head")
            .code(),
        "FOREMAN_ARTIFACT_STAGE_SUBSTITUTION"
    );
    let evidence = adapter
        .load_managed_evidence(&task_ref, staged.evidence().attempt())
        .expect("load finalized Artifact Store evidence");
    assert_eq!(evidence, vec![staged.evidence().clone()]);
    let links = adapter
        .load_reference_links(&task_ref)
        .expect("load finalized artifact link");
    assert_eq!(links.artifact_links().len(), 1);
    assert_eq!(links.artifact_links()[0].descriptor_digest(), &descriptor);
    assert_eq!(links.artifact_links()[0].link(), staged.link());
    let replay = adapter
        .read_task_replay(&task_ref)
        .expect("load finalized task replay");
    assert!(replay.records().iter().any(|record| {
        record.record_kind() == "ARTIFACT_REFERENCE"
            && record.record_state() == ReplayRecordState::Retained
            && record.record_digest() == &descriptor
    }));
    let replay_digest = replay.evidence_digest().clone();
    drop(adapter);

    let mut reconnected =
        PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
            .expect("post-finalize reconnect adapter");
    assert_eq!(
        reconnected
            .read_task_replay(&task_ref)
            .expect("post-finalize reconnect replay")
            .evidence_digest(),
        &replay_digest
    );
    assert!(
        reconnected
            .load_staged_artifact_reference(&task_ref)
            .expect("post-finalize reconnect stage reader")
            .is_none()
    );
}

fn assert_provider_dispatch_rejects_stale_authority_and_registry(
    adapter: &mut PostgresForeman,
    migrator_url: &str,
    record: &VerifiedWorkerAttemptRecord,
) {
    let mut migrator = connect_as(migrator_url, "lattice_migrator");
    let row = migrator
        .query_one(
            "SELECT promotion.project_id::text, authority.expires_at::text, \
                    pg_catalog.encode(attempt.approval_receipt_digest,'hex'), \
                    project.drift_repository, \
                    pg_catalog.encode(project.authority_receipt_digest,'hex') \
               FROM ONLY foreman_execution.worker_attempts AS attempt \
               JOIN ONLY foreman_execution.task_promotions AS promotion \
                 ON promotion.task_ref=attempt.task_ref \
               JOIN ONLY foreman_execution.approval_evidence AS authority \
                 ON authority.task_ref=attempt.task_ref \
                AND authority.authority_digest=attempt.approval_receipt_digest \
               JOIN ONLY control.project_registry_projects AS project \
                 ON project.project_id=promotion.project_id \
              WHERE attempt.task_ref=decode($1,'hex') \
                AND attempt.attempt_number=$2",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
            ],
        )
        .expect("load exact authority and Registry gate fixture");
    let project_id: String = row.get(0);
    let original_expires_at: String = row.get(1);
    let original_authority_digest: String = row.get(2);
    let original_drift: bool = row.get(3);
    let original_project_receipt: String = row.get(4);
    assert!(!original_drift);
    assert_eq!(
        original_authority_digest,
        record.approval_receipt_digest().as_str()
    );

    let reject = |adapter: &mut PostgresForeman, label: &str| {
        let error = match adapter.claim_provider_dispatch(
            record,
            ProviderDispatchKind::WorkerThread,
            record.payload_digest(),
            record.packet_digest(),
            &digest(99_909),
        ) {
            Err(error) => error,
            Ok(_) => panic!("{label} must reject provider dispatch"),
        };
        assert_eq!(error.code(), "FOREMAN_PROVIDER_DISPATCH_REJECTED");
        assert!(
            adapter
                .load_provider_dispatch_claim(
                    record.task_ref(),
                    record.attempt_number(),
                    ProviderDispatchKind::WorkerThread,
                )
                .expect("load rejected authority dispatch")
                .is_none(),
            "{label} cannot retain a provider claim"
        );
    };

    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_evidence \
                SET expires_at='2026-01-01T00:00:01Z' \
              WHERE task_ref=decode($1,'hex') \
                AND authority_digest=decode($2,'hex')",
            &[&record.task_ref().as_str(), &original_authority_digest],
        )
        .expect("expire exact execution authority");
    reject(adapter, "expired execution authority");
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.approval_evidence SET expires_at=$3 \
              WHERE task_ref=decode($1,'hex') \
                AND authority_digest=decode($2,'hex')",
            &[
                &record.task_ref().as_str(),
                &original_authority_digest,
                &original_expires_at,
            ],
        )
        .expect("restore execution authority expiry");

    migrator
        .execute(
            "UPDATE ONLY foreman_execution.worker_attempts \
                SET approval_receipt_digest=decode(repeat('ab',32),'hex') \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
            ],
        )
        .expect("substitute attempt execution authority");
    reject(adapter, "substituted execution authority");
    migrator
        .execute(
            "UPDATE ONLY foreman_execution.worker_attempts \
                SET approval_receipt_digest=decode($3,'hex') \
              WHERE task_ref=decode($1,'hex') AND attempt_number=$2",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
                &original_authority_digest,
            ],
        )
        .expect("restore attempt execution authority");

    migrator
        .execute(
            "UPDATE ONLY control.project_registry_projects \
                SET drift_repository=true WHERE project_id=$1",
            &[&project_id],
        )
        .expect("introduce Registry repository drift");
    assert_restart_candidate_kind_on_adapter(
        adapter,
        record,
        RestartTaskKind::ProjectReconciliationRequired,
        "active-attempt Project Registry drift",
    );
    reject(adapter, "drifted Project Registry identity");
    migrator
        .execute(
            "UPDATE ONLY control.project_registry_projects \
                SET drift_repository=$2 WHERE project_id=$1",
            &[&project_id, &original_drift],
        )
        .expect("restore Registry drift");

    migrator
        .execute(
            "UPDATE ONLY control.project_registry_projects \
                SET authority_receipt_digest=decode(repeat('ac',32),'hex') \
              WHERE project_id=$1",
            &[&project_id],
        )
        .expect("substitute Registry authority receipt");
    reject(adapter, "substituted Project Registry receipt");
    migrator
        .execute(
            "UPDATE ONLY control.project_registry_projects \
                SET authority_receipt_digest=decode($2,'hex') WHERE project_id=$1",
            &[&project_id, &original_project_receipt],
        )
        .expect("restore Registry authority receipt");
}

fn assert_provider_dispatch_rejects_stale_writer_and_runtime(
    adapter: &mut PostgresForeman,
    migrator_url: &str,
    record: &VerifiedWorkerAttemptRecord,
) {
    let mut migrator = connect_as(migrator_url, "lattice_migrator");
    let row = migrator
        .query_one(
            "SELECT promotion.project_id::text, lease.current_status::text, \
                    lease.current_expires_at, lease.current_daemon_instance_id::text, \
                    lease.current_daemon_epoch \
               FROM ONLY foreman_execution.task_promotions AS promotion \
               JOIN ONLY writer_lease.writer_lease_heads AS lease \
                 ON lease.project_id=promotion.project_id \
              WHERE promotion.task_ref=decode($1,'hex')",
            &[&record.task_ref().as_str()],
        )
        .expect("load exact Writer gate fixture");
    let project_id: String = row.get(0);
    let original_status: String = row.get(1);
    let original_expires_at: String = row.get(2);
    let original_daemon: String = row.get(3);
    let original_epoch: i64 = row.get(4);
    assert_eq!(original_status, "ACTIVE");

    let reject = |adapter: &mut PostgresForeman, label: &str| {
        let error = match adapter.claim_provider_dispatch(
            record,
            ProviderDispatchKind::WorkerThread,
            record.payload_digest(),
            record.packet_digest(),
            &digest(99_910),
        ) {
            Err(error) => error,
            Ok(_) => panic!("{label} must reject provider dispatch"),
        };
        assert_eq!(error.code(), "FOREMAN_PROVIDER_DISPATCH_REJECTED");
        assert!(
            adapter
                .load_provider_dispatch_claim(
                    record.task_ref(),
                    record.attempt_number(),
                    ProviderDispatchKind::WorkerThread,
                )
                .expect("load rejected dispatch")
                .is_none(),
            "{label} cannot retain a provider claim"
        );
    };

    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads SET current_status='SUSPECT' \
              WHERE project_id=$1",
            &[&project_id],
        )
        .expect("make Writer suspect");
    reject(adapter, "suspect Writer");
    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads SET current_status=$2 \
              WHERE project_id=$1",
            &[&project_id, &original_status],
        )
        .expect("restore Writer status");

    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_expires_at='2020-01-01T00:00:00Z' WHERE project_id=$1",
            &[&project_id],
        )
        .expect("expire Writer");
    reject(adapter, "expired Writer");
    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads SET current_expires_at=$2 \
              WHERE project_id=$1",
            &[&project_id, &original_expires_at],
        )
        .expect("restore Writer expiry");

    migrator
        .execute(
            "UPDATE ONLY control.runtime_admission SET admission_mode='DRAINING' \
              WHERE singleton=true",
            &[],
        )
        .expect("drain runtime admission");
    reject(adapter, "draining runtime");
    migrator
        .execute(
            "UPDATE ONLY control.runtime_admission SET admission_mode='ACTIVE' \
              WHERE singleton=true",
            &[],
        )
        .expect("restore runtime admission");

    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_daemon_instance_id='substituted-daemon' WHERE project_id=$1",
            &[&project_id],
        )
        .expect("substitute Writer daemon");
    reject(adapter, "mismatched Writer daemon");
    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads \
                SET current_daemon_instance_id=$2, current_daemon_epoch=$3 WHERE project_id=$1",
            &[&project_id, &original_daemon, &original_epoch],
        )
        .expect("restore Writer daemon binding");
}

fn spawn_provider_claim(
    runtime_url: &str,
    target: &ExtensionTarget,
    record: &VerifiedWorkerAttemptRecord,
    subject: ContentDigest,
) -> thread::JoinHandle<Result<ClaimDisposition, lattice_postgres_foreman::AdapterError>> {
    let runtime_url = runtime_url.to_owned();
    let target = target.clone();
    let record = record.clone();
    thread::spawn(move || {
        let mut runtime = connect_as(&runtime_url, "lattice_runtime");
        runtime
            .batch_execute("SET statement_timeout='5s'; SET lock_timeout='750ms'")
            .expect("bound provider-claim barrier session");
        let mut adapter = PostgresForeman::new(runtime, &target).expect("runtime claim adapter");
        adapter.claim_provider_dispatch(
            &record,
            ProviderDispatchKind::WorkerThread,
            record.payload_digest(),
            record.packet_digest(),
            &subject,
        )
    })
}

fn assert_no_provider_claim(
    runtime_url: &str,
    target: &ExtensionTarget,
    record: &VerifiedWorkerAttemptRecord,
) {
    let mut adapter = PostgresForeman::new(connect_as(runtime_url, "lattice_runtime"), target)
        .expect("claim absence adapter");
    assert!(
        adapter
            .load_provider_dispatch_claim(
                record.task_ref(),
                record.attempt_number(),
                ProviderDispatchKind::WorkerThread,
            )
            .expect("load provider claim after rejected barrier")
            .is_none()
    );
}

fn assert_restart_candidate_kind(
    runtime_url: &str,
    target: &ExtensionTarget,
    record: &VerifiedWorkerAttemptRecord,
    expected_kind: RestartTaskKind,
    label: &str,
) {
    let mut runtime = connect_as(runtime_url, "lattice_runtime");
    runtime
        .batch_execute("SET statement_timeout='5s'")
        .expect("bound restart drift reader");
    let mut adapter = PostgresForeman::new(runtime, target).expect("restart drift adapter");
    assert_restart_candidate_kind_on_adapter(&mut adapter, record, expected_kind, label);
}

fn assert_restart_candidate_kind_on_adapter(
    adapter: &mut PostgresForeman,
    record: &VerifiedWorkerAttemptRecord,
    expected_kind: RestartTaskKind,
    label: &str,
) {
    let candidates = adapter
        .list_restart_task_refs(256)
        .expect("discover restart drift candidate");
    let matching = candidates
        .iter()
        .filter(|candidate| candidate.task_ref() == record.task_ref())
        .collect::<Vec<_>>();
    assert_eq!(
        matching.len(),
        1,
        "{label} must not disappear from restart discovery"
    );
    let candidate = matching[0];
    assert_eq!(candidate.restart_kind(), expected_kind, "{label}");
    assert_eq!(candidate.restart_priority(), 3, "{label}");
    assert_eq!(
        candidate.attempt_number(),
        Some(u8::try_from(record.attempt_number()).expect("bounded attempt number")),
        "{label}"
    );
    assert_eq!(candidate.attempt_id(), Some(record.attempt_id()), "{label}");
}

fn assert_provider_dispatch_serializes_authority_mutations(
    migrator_url: &str,
    runtime_url: &str,
    target: &ExtensionTarget,
    rejected_record: &VerifiedWorkerAttemptRecord,
    admitted_record: &VerifiedWorkerAttemptRecord,
) {
    let mut migrator = connect_as(migrator_url, "lattice_migrator");
    let project_id: String = migrator
        .query_one(
            "SELECT project_id::text FROM ONLY foreman_execution.task_promotions \
              WHERE task_ref=decode($1,'hex')",
            &[&rejected_record.task_ref().as_str()],
        )
        .expect("load barrier project")
        .get(0);

    migrator
        .batch_execute("BEGIN")
        .expect("begin admission barrier");
    migrator
        .execute(
            "UPDATE ONLY control.runtime_admission SET admission_mode='DRAINING' WHERE singleton=true",
            &[],
        )
        .expect("hold uncommitted admission mutation");
    let claim = spawn_provider_claim(runtime_url, target, rejected_record, digest(99_920));
    let lock_timeout = claim
        .join()
        .expect("join admission lock-timeout claim")
        .expect_err("admission mutation must lock the provider claim");
    assert_eq!(
        lock_timeout.kind(),
        lattice_postgres_foreman::AdapterErrorKind::Database
    );
    assert_no_provider_claim(runtime_url, target, rejected_record);
    migrator
        .batch_execute("COMMIT")
        .expect("commit admission mutation");
    let mut rejected = PostgresForeman::new(connect_as(runtime_url, "lattice_runtime"), target)
        .expect("draining admission adapter");
    let error = rejected
        .claim_provider_dispatch(
            rejected_record,
            ProviderDispatchKind::WorkerThread,
            rejected_record.payload_digest(),
            rejected_record.packet_digest(),
            &digest(99_920),
        )
        .expect_err("committed draining admission must reject claim");
    assert_eq!(error.code(), "FOREMAN_PROVIDER_DISPATCH_REJECTED");
    assert_no_provider_claim(runtime_url, target, rejected_record);
    migrator
        .execute(
            "UPDATE ONLY control.runtime_admission SET admission_mode='ACTIVE' WHERE singleton=true",
            &[],
        )
        .expect("restore admission after barrier");

    migrator
        .batch_execute("BEGIN")
        .expect("begin Writer barrier");
    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads SET current_status='SUSPECT' WHERE project_id=$1",
            &[&project_id],
        )
        .expect("hold uncommitted Writer mutation");
    let claim = spawn_provider_claim(runtime_url, target, rejected_record, digest(99_921));
    let lock_timeout = claim
        .join()
        .expect("join Writer lock-timeout claim")
        .expect_err("Writer mutation must lock the provider claim");
    assert_eq!(
        lock_timeout.kind(),
        lattice_postgres_foreman::AdapterErrorKind::Database
    );
    assert_no_provider_claim(runtime_url, target, rejected_record);
    migrator
        .batch_execute("COMMIT")
        .expect("commit Writer mutation");
    let mut rejected = PostgresForeman::new(connect_as(runtime_url, "lattice_runtime"), target)
        .expect("suspect Writer adapter");
    let error = rejected
        .claim_provider_dispatch(
            rejected_record,
            ProviderDispatchKind::WorkerThread,
            rejected_record.payload_digest(),
            rejected_record.packet_digest(),
            &digest(99_921),
        )
        .expect_err("committed suspect Writer must reject claim");
    assert_eq!(error.code(), "FOREMAN_PROVIDER_DISPATCH_REJECTED");
    assert_no_provider_claim(runtime_url, target, rejected_record);
    migrator
        .execute(
            "UPDATE ONLY writer_lease.writer_lease_heads SET current_status='ACTIVE' WHERE project_id=$1",
            &[&project_id],
        )
        .expect("restore Writer after barrier");

    migrator
        .batch_execute("BEGIN")
        .expect("begin Foreman stream barrier");
    migrator
        .query_one(
            "SELECT stream.stream_id \
               FROM ONLY control.task_ledger_streams AS stream \
              WHERE stream.project_id='lattice-control' \
                AND stream.project_snapshot_id='foreman-coordination-v1' \
                AND stream.task_id='TASK-FOREMAN-COORDINATION' \
              FOR UPDATE OF stream",
            &[],
        )
        .expect("hold exact Foreman stream mutation barrier");
    let claim = spawn_provider_claim(runtime_url, target, admitted_record, digest(99_922));
    let lock_timeout = claim
        .join()
        .expect("join Foreman stream lock-timeout claim")
        .expect_err("Foreman stream mutation must lock the provider claim");
    assert_eq!(
        lock_timeout.kind(),
        lattice_postgres_foreman::AdapterErrorKind::Database
    );
    assert_no_provider_claim(runtime_url, target, admitted_record);
    migrator
        .batch_execute("ROLLBACK")
        .expect("release Foreman stream barrier");
    let mut admitted = PostgresForeman::new(connect_as(runtime_url, "lattice_runtime"), target)
        .expect("released Foreman stream adapter");
    assert_eq!(
        admitted
            .claim_provider_dispatch(
                admitted_record,
                ProviderDispatchKind::WorkerThread,
                admitted_record.payload_digest(),
                admitted_record.packet_digest(),
                &digest(99_922),
            )
            .expect("unchanged Foreman stream admits exact claim"),
        ClaimDisposition::Claimed
    );
}

fn collect_restart_pages(
    adapter: &mut PostgresForeman,
    page_limit: u16,
) -> Vec<lattice_postgres_foreman::RestartTaskRef> {
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    loop {
        let page = adapter
            .list_restart_task_refs_page(cursor.as_ref(), page_limit)
            .expect("keyset restart-discovery page");
        assert!(page.len() <= usize::from(page_limit));
        if page.is_empty() {
            break;
        }
        let page_len = page.len();
        for candidate in page {
            let next = candidate.cursor();
            assert!(cursor.as_ref().is_none_or(|previous| next > *previous));
            assert!(seen.insert((next.restart_priority(), next.task_ref().as_str().to_owned(),)));
            cursor = Some(next);
            candidates.push(candidate);
        }
        if page_len < usize::from(page_limit) {
            break;
        }
    }
    candidates
}

#[derive(Clone)]
struct ArtifactCandidate {
    evidence: VerifiedManagedEvidence,
    link: TaskRuntimeEventLink,
    correlation_id: CorrelationId,
    occurred_at: String,
    command: AppendCommand,
}

fn assert_artifact_quota_boundaries(
    runtime_url: &str,
    target: &ExtensionTarget,
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    count_fixture: &ClaimFixture,
    byte_fixture: &ClaimFixture,
) {
    assert_artifact_count_boundary_and_concurrency(
        runtime_url,
        target,
        ledger,
        authority,
        count_fixture,
    );
    assert_artifact_byte_boundary(runtime_url, target, ledger, authority, byte_fixture);
}

#[allow(clippy::too_many_lines)]
fn assert_artifact_count_boundary_and_concurrency(
    runtime_url: &str,
    target: &ExtensionTarget,
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    fixture: &ClaimFixture,
) {
    let mut adapter = PostgresForeman::new(connect_as(runtime_url, "lattice_runtime"), target)
        .expect("count-quota adapter");
    for ordinal in 1..MAX_ARTIFACTS_PER_ATTEMPT {
        let candidate =
            plan_artifact_candidate(ledger, &mut adapter, fixture, "count", ordinal, vec![b'c']);
        persist_artifact_candidate(&mut adapter, ledger, authority, &candidate);
    }
    let retained = adapter
        .load_managed_evidence(
            fixture.record.task_ref(),
            u8::try_from(fixture.record.attempt_number()).expect("bounded count attempt"),
        )
        .expect("load count-boundary evidence");
    assert_eq!(
        retained.len(),
        usize::from(MAX_ARTIFACTS_PER_ATTEMPT - 1),
        "one count slot remains before the concurrent race"
    );

    let race_candidates = [
        plan_artifact_candidate(
            ledger,
            &mut adapter,
            fixture,
            "count",
            MAX_ARTIFACTS_PER_ATTEMPT,
            vec![b'x'],
        ),
        plan_artifact_candidate(
            ledger,
            &mut adapter,
            fixture,
            "count",
            MAX_ARTIFACTS_PER_ATTEMPT + 1,
            vec![b'y'],
        ),
    ];
    let replay_before_race = adapter
        .read_task_replay(fixture.record.task_ref())
        .expect("count replay before concurrent stage")
        .evidence_digest()
        .clone();
    let head_before_race = ledger
        .load_stream(fixture.successor_identity.clone())
        .expect("count stream before concurrent stage")
        .stream()
        .head()
        .clone();
    let barrier = Arc::new(Barrier::new(race_candidates.len()));
    let handles = race_candidates
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, candidate)| {
            let barrier = Arc::clone(&barrier);
            let runtime_url = runtime_url.to_owned();
            let target = target.clone();
            thread::spawn(move || {
                let mut contender =
                    PostgresForeman::new(connect_as(&runtime_url, "lattice_runtime"), &target)
                        .expect("count-quota contender");
                barrier.wait();
                (
                    index,
                    contender.stage_artifact_reference(
                        &candidate.evidence,
                        &candidate.link,
                        &candidate.correlation_id,
                        &candidate.occurred_at,
                    ),
                )
            })
        })
        .collect::<Vec<_>>();
    let race_results = handles
        .into_iter()
        .map(|handle| handle.join().expect("count-quota contender thread"))
        .collect::<Vec<_>>();
    let winners = race_results
        .iter()
        .filter_map(|(index, result)| {
            matches!(result, Ok(AppendDisposition::Inserted)).then_some(*index)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        winners.len(),
        1,
        "only one contender may claim the last slot"
    );
    let winner_index = winners[0];
    let loser_error = race_results
        .iter()
        .find_map(|(index, result)| (*index != winner_index).then_some(result.as_ref().err()))
        .flatten()
        .expect("the other last-slot contender must fail closed");
    assert_eq!(
        loser_error.code(),
        "FOREMAN_ARTIFACT_STAGE_SUBSTITUTION",
        "the one-pending-stage invariant serializes same-task contenders"
    );
    assert_eq!(
        adapter
            .read_task_replay(fixture.record.task_ref())
            .expect("count replay while winner remains staged")
            .evidence_digest(),
        &replay_before_race,
        "staging cannot project a Task Ledger artifact"
    );
    assert_eq!(
        ledger
            .load_stream(fixture.successor_identity.clone())
            .expect("count stream while winner remains staged")
            .stream()
            .head(),
        &head_before_race,
        "the losing stage cannot append either contender's Ledger command"
    );
    let staged = adapter
        .load_staged_artifact_reference(fixture.record.task_ref())
        .expect("load winning concurrent stage")
        .expect("one concurrent stage is retained");
    assert_eq!(staged.evidence(), &race_candidates[winner_index].evidence);

    let winner = &race_candidates[winner_index];
    ledger
        .execute(winner.command.clone(), authority.clone())
        .expect("append last count-slot Ledger event");
    assert_eq!(
        adapter
            .finalize_staged_artifact_reference(
                fixture.record.task_ref(),
                winner.evidence.attempt(),
                winner.evidence.descriptor_digest(),
            )
            .expect("finalize last count slot"),
        AppendDisposition::Inserted
    );
    assert_eq!(
        adapter
            .stage_artifact_reference(
                &winner.evidence,
                &winner.link,
                &winner.correlation_id,
                &winner.occurred_at,
            )
            .expect("exact count-boundary stage replay"),
        AppendDisposition::ExactReplay,
        "exact replay is checked before quota accounting"
    );
    assert_eq!(
        adapter
            .finalize_staged_artifact_reference(
                fixture.record.task_ref(),
                winner.evidence.attempt(),
                winner.evidence.descriptor_digest(),
            )
            .expect("exact count-boundary finalize replay"),
        AppendDisposition::ExactReplay
    );
    assert_eq!(
        adapter
            .load_managed_evidence(fixture.record.task_ref(), winner.evidence.attempt(),)
            .expect("load full count quota")
            .len(),
        usize::from(MAX_ARTIFACTS_PER_ATTEMPT)
    );

    let over_limit = plan_artifact_candidate(
        ledger,
        &mut adapter,
        fixture,
        "count",
        MAX_ARTIFACTS_PER_ATTEMPT + 2,
        vec![b'z'],
    );
    assert_quota_rejection_leaves_ledger_clean(
        &mut adapter,
        ledger,
        fixture,
        &over_limit,
        "FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED",
    );
}

fn assert_artifact_byte_boundary(
    runtime_url: &str,
    target: &ExtensionTarget,
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    fixture: &ClaimFixture,
) {
    let max_bytes = usize::try_from(MAX_ARTIFACT_BYTES_PER_ATTEMPT)
        .expect("artifact byte quota fits this process");
    assert_eq!(
        max_bytes % MAX_MANAGED_EVIDENCE_BYTES,
        0,
        "the database quota must be expressible through bounded evidence objects"
    );
    let chunks = max_bytes / MAX_MANAGED_EVIDENCE_BYTES;
    assert!(chunks < usize::from(MAX_ARTIFACTS_PER_ATTEMPT));
    let mut adapter = PostgresForeman::new(connect_as(runtime_url, "lattice_runtime"), target)
        .expect("byte-quota adapter");
    let mut last = None;
    for ordinal in 1..=u16::try_from(chunks).expect("bounded byte chunks") {
        let candidate = plan_artifact_candidate(
            ledger,
            &mut adapter,
            fixture,
            "bytes",
            ordinal,
            vec![b'b'; MAX_MANAGED_EVIDENCE_BYTES],
        );
        persist_artifact_candidate(&mut adapter, ledger, authority, &candidate);
        last = Some(candidate);
    }
    let retained = adapter
        .load_managed_evidence(
            fixture.record.task_ref(),
            u8::try_from(fixture.record.attempt_number()).expect("bounded byte attempt"),
        )
        .expect("load byte-boundary evidence");
    assert_eq!(retained.len(), chunks);
    assert_eq!(
        retained
            .iter()
            .map(|evidence| evidence.bytes().len())
            .sum::<usize>(),
        max_bytes,
        "the exact byte ceiling is retained"
    );
    let last = last.expect("at least one full-size evidence chunk");
    assert_eq!(
        adapter
            .stage_artifact_reference(
                &last.evidence,
                &last.link,
                &last.correlation_id,
                &last.occurred_at,
            )
            .expect("exact byte-boundary replay"),
        AppendDisposition::ExactReplay,
        "exact replay does not consume bytes twice"
    );

    let over_limit = plan_artifact_candidate(
        ledger,
        &mut adapter,
        fixture,
        "bytes",
        u16::try_from(chunks + 1).expect("bounded over-limit ordinal"),
        vec![b'o'],
    );
    assert_quota_rejection_leaves_ledger_clean(
        &mut adapter,
        ledger,
        fixture,
        &over_limit,
        "FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED",
    );
}

fn plan_artifact_candidate(
    ledger: &mut PostgresTaskLedger,
    adapter: &mut PostgresForeman,
    fixture: &ClaimFixture,
    quota_kind: &str,
    ordinal: u16,
    bytes: Vec<u8>,
) -> ArtifactCandidate {
    let bytes = quota_json_bytes(bytes.len(), bytes.first().copied().unwrap_or(b'x'));
    let stream = ledger
        .load_stream(fixture.successor_identity.clone())
        .expect("load quota artifact stream")
        .stream()
        .clone();
    let reference_links = adapter
        .load_reference_links(fixture.record.task_ref())
        .expect("load quota artifact links")
        .artifact_links()
        .iter()
        .map(|reference| reference.link().clone())
        .collect::<Vec<_>>();
    let occurred_at = quota_timestamp(fixture.task_number, ordinal);
    let evidence = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            stream.identity().project_id().clone(),
            fixture.record.task_ref().clone(),
            u8::try_from(fixture.record.attempt_number()).expect("bounded quota attempt"),
            ManagedEvidenceKind::ResourceObservation,
            "application/json",
            format!("lattice.foreman-artifact-quota-{quota_kind}/1.0"),
            "lattice-postgres-foreman-live",
            "1",
            digest(80_000 + u64::from(fixture.task_number) * 1_000 + u64::from(ordinal)),
            occurred_at.clone(),
            bytes,
        )
        .expect("bounded quota evidence input"),
    )
    .expect("verified quota evidence");
    let correlation_id = CorrelationId::new(format!(
        "foreman-artifact-{quota_kind}-correlation-{}-{ordinal}",
        fixture.task_number
    ))
    .expect("quota artifact correlation");
    let metadata = TaskRuntimeAppendMetadata::new(
        CommandId::new(format!(
            "foreman-artifact-{quota_kind}-{}-{ordinal}",
            fixture.task_number
        ))
        .expect("quota artifact command"),
        correlation_id.clone(),
        occurred_at.clone(),
    )
    .expect("quota artifact metadata");
    let plan = plan_artifact_reference_append(
        &stream,
        &fixture.binding,
        std::slice::from_ref(&fixture.record),
        &reference_links,
        metadata,
        fixture.record.attempt_number(),
        evidence.descriptor_digest().clone(),
    )
    .expect("plan quota artifact reference");
    ArtifactCandidate {
        evidence,
        link: plan.link().clone(),
        correlation_id,
        occurred_at,
        command: plan.ledger_plan().command_record().request().clone(),
    }
}

fn quota_json_bytes(length: usize, fill: u8) -> Vec<u8> {
    match length {
        0 => Vec::new(),
        1 => vec![b'0'],
        _ => {
            let fill = if fill.is_ascii_alphanumeric() {
                fill
            } else {
                b'x'
            };
            let mut bytes = vec![fill; length];
            bytes[0] = b'"';
            bytes[length - 1] = b'"';
            bytes
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_blocker_artifact(
    adapter: &mut PostgresForeman,
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    fixture: &ClaimFixture,
    command_kind: &str,
    second_offset: u8,
    blocker_code: &str,
    blocker_reason: &str,
) -> VerifiedManagedEvidence {
    let stream = ledger
        .load_stream(fixture.successor_identity.clone())
        .expect("load blocker successor stream")
        .stream()
        .clone();
    let links = adapter
        .load_reference_links(fixture.record.task_ref())
        .expect("load blocker artifact links")
        .artifact_links()
        .iter()
        .map(|reference| reference.link().clone())
        .collect::<Vec<_>>();
    let occurred_at = timestamp(fixture.task_number, second_offset);
    let bytes = format!(
        "{{\"schema\":\"lattice.managed-blocker.v1\",\"attempt\":{},\"code\":\"{}\",\"reason\":\"{}\",\"retryable\":false}}",
        fixture.record.attempt_number(), blocker_code, blocker_reason
    )
    .into_bytes();
    let evidence = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            stream.identity().project_id().clone(),
            fixture.record.task_ref().clone(),
            u8::try_from(fixture.record.attempt_number()).expect("bounded blocker attempt"),
            ManagedEvidenceKind::WorkerLifecycle,
            "application/json",
            "lattice.managed-blocker.v1",
            "lattice-foreman",
            "1",
            fixture.record.foreman_checkpoint_digest().clone(),
            occurred_at.clone(),
            bytes,
        )
        .expect("bounded blocker evidence input"),
    )
    .expect("verified blocker evidence");
    let append_metadata = metadata(command_kind, fixture.task_number, second_offset);
    let plan = plan_artifact_reference_append(
        &stream,
        &fixture.binding,
        std::slice::from_ref(&fixture.record),
        &links,
        append_metadata,
        fixture.record.attempt_number(),
        evidence.descriptor_digest().clone(),
    )
    .expect("plan blocker artifact reference");
    assert_eq!(
        adapter
            .stage_artifact_reference(
                &evidence,
                plan.link(),
                &correlation_id(fixture.task_number),
                &occurred_at,
            )
            .expect("stage blocker artifact"),
        AppendDisposition::Inserted
    );
    ledger
        .execute(
            plan.ledger_plan().command_record().request().clone(),
            authority.clone(),
        )
        .expect("append blocker artifact Ledger event");
    evidence
}

fn persist_artifact_candidate(
    adapter: &mut PostgresForeman,
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    candidate: &ArtifactCandidate,
) {
    assert_eq!(
        adapter
            .stage_artifact_reference(
                &candidate.evidence,
                &candidate.link,
                &candidate.correlation_id,
                &candidate.occurred_at,
            )
            .expect("stage quota artifact"),
        AppendDisposition::Inserted
    );
    ledger
        .execute(candidate.command.clone(), authority.clone())
        .expect("append quota artifact Ledger event");
    assert_eq!(
        adapter
            .finalize_staged_artifact_reference(
                candidate.evidence.task_ref(),
                candidate.evidence.attempt(),
                candidate.evidence.descriptor_digest(),
            )
            .expect("finalize quota artifact"),
        AppendDisposition::Inserted
    );
}

fn assert_quota_rejection_leaves_ledger_clean(
    adapter: &mut PostgresForeman,
    ledger: &mut PostgresTaskLedger,
    fixture: &ClaimFixture,
    candidate: &ArtifactCandidate,
    expected_code: &str,
) {
    let replay_before = adapter
        .read_task_replay(fixture.record.task_ref())
        .expect("quota replay before rejection")
        .evidence_digest()
        .clone();
    let stream_before = ledger
        .load_stream(fixture.successor_identity.clone())
        .expect("quota stream before rejection")
        .stream()
        .clone();
    let rejection = adapter
        .stage_artifact_reference(
            &candidate.evidence,
            &candidate.link,
            &candidate.correlation_id,
            &candidate.occurred_at,
        )
        .expect_err("over-quota artifact must fail closed before Ledger append");
    assert_eq!(rejection.code(), expected_code);
    assert!(
        adapter
            .load_staged_artifact_reference(fixture.record.task_ref())
            .expect("quota stage reader after rejection")
            .is_none()
    );
    assert_eq!(
        adapter
            .read_task_replay(fixture.record.task_ref())
            .expect("quota replay after rejection")
            .evidence_digest(),
        &replay_before,
        "quota rejection cannot change Foreman replay"
    );
    let stream_after = ledger
        .load_stream(fixture.successor_identity.clone())
        .expect("quota stream after rejection")
        .stream()
        .clone();
    assert_eq!(
        stream_after.head(),
        stream_before.head(),
        "quota rejection cannot change the formal Task Ledger head"
    );
    assert!(
        stream_after
            .commands()
            .iter()
            .all(|record| { record.request().command_id() != candidate.command.command_id() })
    );
}

fn quota_timestamp(task_number: u8, ordinal: u16) -> String {
    let hour = 13_u16 + u16::from(task_number);
    let minute = ordinal / 60;
    let second = ordinal % 60;
    format!("2026-08-27T{hour:02}:{minute:02}:{second:02}Z")
}

#[derive(Clone)]
struct ClaimFixture {
    task_number: u8,
    binding: VerifiedTaskExecutionBinding,
    successor_identity: TaskLedgerStreamIdentity,
    preparation: ManagedPreparationObservation,
    intent: ManagedPromotionIntent,
    record: VerifiedWorkerAttemptRecord,
}

fn assert_intake_link_tamper_fails_closed(
    migrator_url: &str,
    runtime_url: &str,
    target: &ExtensionTarget,
    first: &ClaimFixture,
    second: &ClaimFixture,
) {
    let task_ref = first.binding.task_ref().as_str();
    let second_task_ref = second.binding.task_ref().as_str();
    let foreign_intake_event = second.binding.intake_event_digest().as_str();
    let second_replacement_event = second
        .binding
        .successor_task_created_event_digest()
        .as_str();
    let mut migrator = connect_as(migrator_url, "lattice_migrator");
    let before = intake_link_mutation_snapshot(&mut migrator);
    assert_eq!(before.2, 0, "fixture opens no provider dispatch");

    {
        let mut transaction = migrator.transaction().expect("tamper transaction");
        assert_eq!(
            transaction
                .execute(
                    "UPDATE ONLY foreman_execution.preparation_observations \
                        SET intake_event_digest = pg_catalog.decode($2, 'hex') \
                      WHERE task_ref = pg_catalog.decode($1, 'hex')",
                    &[&second_task_ref, &second_replacement_event],
                )
                .expect("temporarily free the second intake event key"),
            1
        );
        assert_eq!(
            transaction
                .execute(
                    "UPDATE ONLY foreman_execution.preparation_observations \
                        SET intake_event_digest = pg_catalog.decode($2, 'hex') \
                      WHERE task_ref = pg_catalog.decode($1, 'hex')",
                    &[&task_ref, &foreign_intake_event],
                )
                .expect("split foreign keys permit A-stream plus B-intake-event tamper"),
            1
        );
        assert_eq!(intake_link_mutation_snapshot(&mut transaction), before);

        transaction
            .batch_execute("SAVEPOINT preparation_read")
            .expect("preparation read savepoint");
        let failure = transaction
            .query(
                "SELECT * FROM foreman_execution.read_preparation_observation_v1( \
                    pg_catalog.decode($1, 'hex'))",
                &[&task_ref],
            )
            .expect_err("mixed preparation lineage read must fail closed");
        assert_lineage_error(&failure, "FOREMAN_PREPARATION_OBSERVATION_LINEAGE_MISMATCH");
        transaction
            .batch_execute(
                "ROLLBACK TO SAVEPOINT preparation_read; RELEASE SAVEPOINT preparation_read",
            )
            .expect("recover after rejected preparation read");

        transaction
            .batch_execute("SAVEPOINT preparation_record")
            .expect("preparation record savepoint");
        let failure = transaction
            .query(
                "SELECT foreman_execution.record_preparation_observation_v1( \
                    observation.task_ref, observation.project_id::text, \
                    observation.project_snapshot_id::text, \
                    observation.project_authority_receipt_digest, \
                    observation.observation_kind::text, observation.subject_digest, \
                    observation.observed_at::text, observation.observation_digest) \
                   FROM ONLY foreman_execution.preparation_observations AS observation \
                  WHERE observation.task_ref = pg_catalog.decode($1, 'hex')",
                &[&task_ref],
            )
            .expect_err("mixed preparation lineage exact record must fail closed");
        assert_lineage_error(&failure, "FOREMAN_PREPARATION_OBSERVATION_SUBSTITUTION");
        transaction
            .batch_execute(
                "ROLLBACK TO SAVEPOINT preparation_record; RELEASE SAVEPOINT preparation_record",
            )
            .expect("recover after rejected preparation record");
        transaction.rollback().expect("rollback preparation tamper");
    }

    {
        let mut transaction = migrator.transaction().expect("intent tamper transaction");
        assert_eq!(
            transaction
                .execute(
                    "UPDATE ONLY foreman_execution.promotion_intents \
                        SET intake_event_digest = pg_catalog.decode($2, 'hex') \
                      WHERE task_ref = pg_catalog.decode($1, 'hex')",
                    &[&second_task_ref, &second_replacement_event],
                )
                .expect("temporarily free the second intent intake event key"),
            1
        );
        assert_eq!(
            transaction
                .execute(
                    "UPDATE ONLY foreman_execution.promotion_intents \
                        SET intake_event_digest = pg_catalog.decode($2, 'hex') \
                      WHERE task_ref = pg_catalog.decode($1, 'hex')",
                    &[&task_ref, &foreign_intake_event],
                )
                .expect("split foreign keys permit A-intent plus B-intake-event tamper"),
            1
        );
        assert_eq!(intake_link_mutation_snapshot(&mut transaction), before);

        transaction
            .batch_execute("SAVEPOINT intent_read")
            .expect("intent read savepoint");
        let failure = transaction
            .query(
                "SELECT * FROM foreman_execution.read_promotion_intent_v1( \
                    pg_catalog.decode($1, 'hex'))",
                &[&task_ref],
            )
            .expect_err("mixed promotion-intent lineage read must fail closed");
        assert_lineage_error(&failure, "FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH");
        transaction
            .batch_execute("ROLLBACK TO SAVEPOINT intent_read; RELEASE SAVEPOINT intent_read")
            .expect("recover after rejected intent read");

        transaction
            .batch_execute("SAVEPOINT intent_record")
            .expect("intent record savepoint");
        let failure = transaction
            .query(
                "SELECT foreman_execution.record_promotion_intent_v1( \
                    intent.task_ref, intent.project_id::text, \
                    intent.project_snapshot_id::text, \
                    intent.project_authority_receipt_digest, intent.successor_stream_id, \
                    intent.task_spec_digest, intent.approval_subject_digest, \
                    intent.budget_digest, intent.global_active_limit, \
                    intent.per_task_active_limit, intent.repair_retry_limit, \
                    intent.max_duration_seconds, intent.max_total_tokens, \
                    intent.max_model_calls, intent.external_cost_status::text, \
                    intent.external_cost_limit_micros, intent.issued_at::text, \
                    intent.deadline_at::text, intent.budget_pointer::text, \
                    intent.verification_policy_digest, intent.base_ref::text, \
                    intent.base_commit::text, intent.source_clean, intent.intent_digest) \
                   FROM ONLY foreman_execution.promotion_intents AS intent \
                  WHERE intent.task_ref = pg_catalog.decode($1, 'hex')",
                &[&task_ref],
            )
            .expect_err("mixed promotion-intent exact record must fail closed");
        assert_lineage_error(&failure, "FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH");
        transaction
            .batch_execute("ROLLBACK TO SAVEPOINT intent_record; RELEASE SAVEPOINT intent_record")
            .expect("recover after rejected intent record");

        transaction
            .batch_execute("SAVEPOINT promotion_record")
            .expect("promotion record savepoint");
        let failure = transaction
            .query(
                "SELECT foreman_execution.record_task_promotion_v1( \
                    promotion.task_ref, promotion.project_id::text, \
                    promotion.project_snapshot_id::text, promotion.intake_stream_id, \
                    promotion.intake_event_digest, \
                    promotion.project_authority_receipt_digest, \
                    promotion.successor_stream_id, \
                    promotion.successor_task_created_event_digest, \
                    promotion.task_spec_digest, promotion.approval_subject_digest, \
                    promotion.budget_digest, promotion.global_active_limit, \
                    promotion.per_task_active_limit, promotion.repair_retry_limit, \
                    promotion.max_duration_seconds, promotion.max_total_tokens, \
                    promotion.max_model_calls, promotion.external_cost_status::text, \
                    promotion.external_cost_limit_micros, promotion.deadline_at::text, \
                    promotion.budget_pointer::text, promotion.verification_policy_digest, \
                    promotion.binding_digest, promotion.base_ref::text, \
                    promotion.base_commit::text, child.ledger_stream_id, \
                    child.ledger_event_sequence, child.ledger_event_digest, \
                    child.ledger_command_id::text, child.ledger_request_digest, \
                    child.ledger_payload_digest) \
                   FROM ONLY foreman_execution.task_promotions AS promotion \
                   JOIN ONLY foreman_execution.child_events AS child \
                     ON child.ledger_event_digest = promotion.ledger_event_digest \
                  WHERE promotion.task_ref = pg_catalog.decode($1, 'hex')",
                &[&task_ref],
            )
            .expect_err("mixed intent lineage must block exact successor replay");
        assert_lineage_error(&failure, "FOREMAN_PROMOTION_INTENT_LINEAGE_MISMATCH");
        transaction
            .batch_execute(
                "ROLLBACK TO SAVEPOINT promotion_record; RELEASE SAVEPOINT promotion_record",
            )
            .expect("recover after rejected promotion record");
        transaction.rollback().expect("rollback intent tamper");
    }

    assert_eq!(intake_link_mutation_snapshot(&mut migrator), before);
    drop(migrator);
    let mut replay = PostgresForeman::new(connect_as(runtime_url, "lattice_runtime"), target)
        .expect("fresh exact lineage replay adapter");
    assert_eq!(
        replay
            .load_preparation_observation(first.binding.task_ref())
            .expect("restored preparation replay"),
        Some(first.preparation.clone())
    );
    assert_eq!(
        replay
            .load_promotion_intent(first.binding.task_ref())
            .expect("restored promotion intent replay"),
        Some(first.intent.clone())
    );
}

fn intake_link_mutation_snapshot(client: &mut impl GenericClient) -> (i64, i64, i64, i64, i64) {
    let row = client
        .query_one(
            "SELECT \
                (SELECT pg_catalog.count(*) FROM ONLY control.task_ledger_events), \
                (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.child_events), \
                (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.provider_dispatch_claims), \
                (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.task_promotions), \
                (SELECT COALESCE(pg_catalog.sum(observation_generation), 0)::bigint \
                   FROM ONLY foreman_execution.preparation_observations)",
            &[],
        )
        .expect("intake-link zero-mutation snapshot");
    (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4))
}

fn assert_lineage_error(error: &postgres::Error, expected_message: &str) {
    let database = error.as_db_error().expect("fixed database rejection");
    assert_eq!(database.code(), &SqlState::RAISE_EXCEPTION);
    assert_eq!(database.message(), expected_message);
}

#[allow(clippy::too_many_lines)]
fn build_claim_fixture(
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    runtime_url: &str,
    target: &ExtensionTarget,
    store_target: &MigrationTarget,
    budget: &WorkerBudget,
    foreman_generation: u64,
    foreman_checkpoint_digest: &ContentDigest,
    task_number: u8,
) -> ClaimFixture {
    build_claim_fixture_with_intake_observer(
        ledger,
        authority,
        runtime_url,
        target,
        store_target,
        budget,
        foreman_generation,
        foreman_checkpoint_digest,
        task_number,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn build_claim_fixture_with_intake_observer<F>(
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    runtime_url: &str,
    target: &ExtensionTarget,
    store_target: &MigrationTarget,
    budget: &WorkerBudget,
    foreman_generation: u64,
    foreman_checkpoint_digest: &ContentDigest,
    task_number: u8,
    observe_committed_draft: F,
) -> ClaimFixture
where
    F: FnOnce(&TaskSubmissionEnvelope),
{
    let (project_id, snapshot_id, authority_receipt_digest) =
        provision_capacity_project(runtime_url, store_target, authority, task_number);
    let task_id =
        TaskId::new(format!("TASK-FOREMAN-CAPACITY-{task_number}")).expect("fixture task");
    let intake_identity = TaskLedgerStreamIdentity::new_general_task_intake(
        project_id.clone(),
        snapshot_id.clone(),
        task_id.clone(),
        "1",
        digest(u64::from(task_number) * 100 + 1),
    )
    .expect("intake identity");
    let submission = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        format!("foreman-capacity-request-{task_number}"),
        "bounded disposable capacity fixture",
        format!("Foreman Capacity {task_number}"),
        intake_identity.clone(),
        authority_receipt_digest,
    )
    .expect("submission");
    if task_number == 1 {
        assert_capacity_submission_rejects_unverified_registry(
            ledger,
            authority,
            &project_id,
            &snapshot_id,
            task_number,
        );
    }
    let intake_vacant = ledger
        .load_stream(intake_identity.clone())
        .expect("load intake")
        .stream()
        .clone();
    let intake_command = AppendCommand::new_general_task_created(
        intake_vacant.head().clone(),
        CommandId::new(format!("mcp-submit:foreman-capacity-request-{task_number}"))
            .expect("general submission command"),
        correlation_id(task_number),
        timestamp(task_number, 0),
        ActorId::new("lattice-runtime").expect("actor"),
        &submission,
    )
    .expect("intake command");
    ledger
        .execute_submission(intake_command, authority.clone(), submission.clone())
        .expect("persist intake event");
    let intake = ledger
        .load_stream(intake_identity)
        .expect("reload intake")
        .stream()
        .clone();
    observe_committed_draft(&submission);

    let task_spec_digest = digest(u64::from(task_number) * 100 + 3);
    let successor_identity = TaskLedgerStreamIdentity::new(
        project_id.clone(),
        snapshot_id.clone(),
        task_id.clone(),
        "1",
        task_spec_digest.clone(),
        "TWD",
    )
    .expect("successor identity");
    let successor_vacant = ledger
        .load_stream(successor_identity.clone())
        .expect("load successor")
        .stream()
        .clone();
    let source = ManagedPromotionSource::new(
        format!("refs/heads/foreman-capacity-{task_number}"),
        format!("{:040x}", u64::from(task_number) + 10_000),
    )
    .expect("promotion source");
    let subject_binding = SubjectBinding::new(
        project_id.clone(),
        snapshot_id.clone(),
        task_id.clone(),
        "1",
        task_spec_digest.clone(),
    )
    .expect("approval subject binding");
    let approval_id = format!("approval-foreman-capacity-{task_number}");
    let approval_identity = ApprovalIdentity::new(
        approval_id.clone(),
        format!("challenge-foreman-capacity-{task_number}"),
        subject_binding.clone(),
        ApprovalSubject::Execution {
            task_spec_hash: subject_binding.task_spec_digest().clone(),
            external_cost: None,
        },
        "lattice-runtime",
        "responsible-user",
        ApprovalAuthority::ResponsibleUser,
        ApprovalOrigin::OsAuthenticatedUser,
        ApprovalLane::Normal,
        "local-approval-channel",
        format!("local-approval-session-{task_number}"),
    )
    .expect("approval identity");
    let signer = FakeNormalSigner::new(
        "responsible-user",
        "os-authenticator",
        "local-key",
        SecretMaterial::new(format!("phase4-capacity-signing-key-{task_number}").into_bytes())
            .expect("signer secret"),
    )
    .expect("normal signer");
    let mut approval_verifier = FakeApprovalVerifier::new();
    let issue = approval_verifier
        .issue(IssueApprovalCommand {
            command_id: format!("issue-foreman-capacity-{task_number}"),
            expected_head: None,
            runtime: RuntimeKind::Fake,
            identity: approval_identity,
            nonce_id: format!("nonce-foreman-capacity-{task_number}"),
            nonce_commitment: nonce_commitment(
                &SecretMaterial::new(format!("phase4-capacity-nonce-{task_number}").into_bytes())
                    .expect("nonce secret"),
            )
            .expect("nonce commitment"),
            issued_at: "2026-01-01T00:00:00Z".to_owned(),
            expires_at: "2099-12-31T23:59:59Z".to_owned(),
            authenticator_id: signer.authenticator_id().to_owned(),
            key_id: signer.key_id().to_owned(),
            verification_key_commitment: signer.verification_key_commitment().clone(),
            evidence_digest: signer.evidence_digest().clone(),
            review_set_digest: None,
        })
        .expect("issue approval challenge");
    let base_challenge = issue.challenge.expect("base approval challenge");
    let approval_subject_digest = base_challenge.subject_digest().clone();
    let mut foreman = PostgresForeman::new(connect_as(runtime_url, "lattice_runtime"), target)
        .expect("promotion adapter");
    let preparation = ManagedPreparationObservation::new(
        submission.task_ref().clone(),
        project_id.clone(),
        snapshot_id.clone(),
        submission.project_authority_receipt_digest().clone(),
        ManagedPreparationObservationKind::Cleared,
        digest(u64::from(task_number) * 100 + 2),
        timestamp(task_number, 0),
    )
    .expect("preparation observation");
    foreman
        .record_preparation_observation(&preparation)
        .expect("persist preparation observation");
    let intent = ManagedPromotionIntent::new(
        submission.task_ref().clone(),
        project_id.clone(),
        snapshot_id.clone(),
        submission.project_authority_receipt_digest().clone(),
        successor_vacant.head().stream_id().clone(),
        task_spec_digest.clone(),
        approval_subject_digest.clone(),
        budget.clone(),
        digest(u64::from(task_number) * 100 + 5),
        source.clone(),
        true,
        timestamp(task_number, 0),
    )
    .expect("promotion intent");
    foreman
        .record_promotion_intent(&intent)
        .expect("persist promotion intent before successor");
    let successor_command = AppendCommand::new(
        successor_vacant.head().clone(),
        command_id("spec", task_number),
        correlation_id(task_number),
        timestamp(task_number, 1),
        LedgerEventKind::TaskCreated,
        ActorId::new("lattice-runtime").expect("actor"),
        ActionId::new("RECORD_MANAGED_TASK_SPEC_V1").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("TASK_SPEC_CAPTURED").expect("reason"),
        task_spec_digest,
        None,
        None,
    )
    .expect("successor command");
    ledger
        .execute(successor_command, authority.clone())
        .expect("persist successor event");
    let successor = ledger
        .load_stream(successor_identity.clone())
        .expect("reload successor")
        .stream()
        .clone();

    let budget_digest = ContentDigest::from_sha256(
        budget
            .digest()
            .strip_prefix("budget:sha256:")
            .expect("budget pointer"),
    )
    .expect("budget digest");
    let binding_plan = plan_task_execution_binding(
        &intake,
        &successor,
        &submission,
        &[],
        metadata("binding", task_number, 2),
        TaskExecutionBindingInput::new(
            approval_subject_digest,
            budget_digest,
            digest(u64::from(task_number) * 100 + 5),
        )
        .expect("binding input"),
    )
    .expect("binding plan");
    let binding = binding_plan.binding().clone();
    ledger
        .execute(
            binding_plan
                .ledger_plan()
                .command_record()
                .request()
                .clone(),
            authority.clone(),
        )
        .expect("persist binding event");
    foreman
        .record_task_promotion(&binding, budget, &source)
        .expect("persist promotion");
    assert_eq!(
        foreman
            .load_task_promotion_source(binding.task_ref())
            .expect("read promotion source"),
        Some(source)
    );

    let execution_subject = ExecutionApprovalSubject::new(
        binding.task_ref().clone(),
        binding.successor_stream_id().clone(),
        subject_binding,
        binding.approval_subject_digest().clone(),
        binding.budget_digest().clone(),
    )
    .expect("execution approval subject");
    let execution_challenge = ExecutionApprovalChallenge::new(base_challenge, execution_subject)
        .expect("execution approval challenge");
    let execution_proof = signer
        .sign_execution(&execution_challenge)
        .expect("execution approval proof");
    let verify = approval_verifier
        .verify(VerifyApprovalCommand {
            command_id: format!("verify-foreman-capacity-{task_number}"),
            approval_id: approval_id.clone(),
            expected_head: approval_verifier
                .state_head(&approval_id)
                .expect("challenged approval head"),
            observed_at: "2026-08-27T12:00:00Z".to_owned(),
            proof: execution_proof.base_proof().clone(),
        })
        .expect("verify base approval");
    let approval_receipt = verify.authority_receipt.expect("authority receipt");
    let approval_head = approval_verifier
        .current_head_at(&approval_id, "2026-08-27T12:00:00Z")
        .expect("approval head lookup")
        .expect("current approval head");
    let bound = approval_verifier
        .bind_execution(BindExecutionApprovalCommand {
            command_id: format!("bind-execution-foreman-capacity-{task_number}"),
            approval_id: approval_id.clone(),
            expected_head: approval_verifier
                .state_head(&approval_id)
                .expect("verified approval head"),
            observed_at: "2026-08-27T12:00:00Z".to_owned(),
            execution_challenge,
            execution_proof,
        })
        .expect("bind execution approval");
    let execution_binding_receipt = bound
        .execution_binding_receipt
        .expect("execution binding receipt terminal");
    assert_eq!(
        approval_verifier.execution_binding_receipt(&approval_id),
        Some(&execution_binding_receipt),
        "approval owner must replay the exact bound execution receipt"
    );
    let approval_context = VerifiedApprovalExecutionContext::new_with_binding_receipt(
        execution_binding_receipt,
        approval_receipt,
        approval_head,
    )
    .expect("current owner-issued authority context");
    let execution_authority =
        issue_verified_approval_execution_authority(&approval_context, "2026-08-27T12:00:00Z")
            .expect("task-bound execution authority");
    let successor = ledger
        .load_stream(successor_identity.clone())
        .expect("reload successor for approval evidence")
        .stream()
        .clone();
    let approval_plan = plan_approval_evidence_append(
        &successor,
        &binding,
        &[],
        metadata("approval", task_number, 3),
        execution_authority.authority_digest().clone(),
    )
    .expect("approval evidence plan");
    ledger
        .execute(
            approval_plan
                .ledger_plan()
                .command_record()
                .request()
                .clone(),
            authority.clone(),
        )
        .expect("persist approval evidence event");
    let mut approval_owner_store = PostgresForeman::new_with_role(
        connect_as(
            &required("LATTICE_FOREMAN_MIGRATOR_URL"),
            "lattice_migrator",
        ),
        target,
        ExtensionDatabaseRole::Migrator,
    )
    .expect("Approval-owner persistence adapter");
    approval_owner_store
        .record_verified_approval_evidence(
            &execution_authority,
            approval_plan.link(),
            &approval_verifier,
        )
        .expect("persist approval evidence");

    let attempt_id =
        AttemptId::new(format!("foreman-capacity-attempt-{task_number}")).expect("attempt id");
    let writer_target = V5ExtensionTarget::new(
        store_target.database_name().to_owned(),
        ContentDigest::from_sha256(
            store_target
                .expected_database_identity_sha256()
                .as_str()
                .to_owned(),
        )
        .expect("writer target identity"),
    )
    .expect("writer target");
    let mut writer = PostgresWriterLease::new_v5_v7(
        connect_store_runtime(runtime_url),
        &writer_target,
        authority,
        600,
    )
    .expect("Writer Lease runtime");
    let task_ref = binding.task_ref().as_str();
    let acquired = writer
        .execute(WriterLeaseRepositoryCommand::Acquire(
            WriterLeaseAcquireRequest {
                command_id: format!("foreman-capacity-acquire-{task_number}"),
                expected_head: None,
                project_id: project_id.clone(),
                project_snapshot_id: snapshot_id.clone(),
                task_id: task_id.clone(),
                task_revision: "1".to_owned(),
                task_spec_digest: binding.task_spec_digest().clone(),
                attempt_id: attempt_id.clone(),
                lease_id: format!("managed-lease-{task_ref}-1"),
                lease_holder_id: "lattice-foreman".to_owned(),
                worktree_id: format!("WORK-{}", task_ref[..59].to_ascii_uppercase()),
                holder_process_id: lattice_contracts::HolderProcessId::new(u64::from(
                    std::process::id(),
                ))
                .expect("holder process"),
                holder_process_start_identity: digest(u64::from(task_number) * 100 + 12),
            },
        ))
        .expect("acquire canonical Writer Lease");
    assert_eq!(acquired.outcome, WriterLeaseCommandOutcome::Applied);
    let writer_fence = acquired
        .after
        .expect("current Writer Lease authority")
        .identity()
        .fencing_token()
        .get();

    let successor = ledger
        .load_stream(successor_identity.clone())
        .expect("reload bound successor")
        .stream()
        .clone();
    let attempt_plan = plan_worker_attempt_append(
        &successor,
        &binding,
        &[],
        &[],
        metadata("attempt", task_number, 4),
        WorkerAttemptInput::new(
            attempt_id,
            1,
            foreman_generation,
            WorkerModel::Terra,
            ReasoningEffort::Medium,
            ModelReason::RoutineEngineering,
            writer_fence,
            foreman_checkpoint_digest.clone(),
            execution_authority.authority_digest().clone(),
            digest(u64::from(task_number) * 100 + 8),
            digest(u64::from(task_number) * 100 + 9),
            digest(u64::from(task_number) * 100 + 10),
            digest(u64::from(task_number) * 100 + 11),
        )
        .expect("attempt input"),
    )
    .expect("attempt plan");
    let record = attempt_plan.record().clone();
    ledger
        .execute(
            attempt_plan
                .ledger_plan()
                .command_record()
                .request()
                .clone(),
            authority.clone(),
        )
        .expect("persist attempt event");
    ClaimFixture {
        task_number,
        binding,
        successor_identity,
        preparation,
        intent,
        record,
    }
}

fn acquire_formal_foreman_writer(
    runtime_url: &str,
    store_target: &MigrationTarget,
    authority: &StoreAuthorityHead,
) -> WriterLeaseAuthorityHead {
    let identity = foreman_coordination_identity().expect("fixed Foreman coordination identity");
    let writer_target = V5ExtensionTarget::new(
        store_target.database_name().to_owned(),
        ContentDigest::from_sha256(
            store_target
                .expected_database_identity_sha256()
                .as_str()
                .to_owned(),
        )
        .expect("formal Foreman Writer target identity"),
    )
    .expect("formal Foreman Writer target");
    let mut writer = PostgresWriterLease::new_v5_v7(
        connect_store_runtime(runtime_url),
        &writer_target,
        authority,
        600,
    )
    .expect("formal Foreman Writer runtime");
    let acquired = writer
        .execute(WriterLeaseRepositoryCommand::Acquire(
            WriterLeaseAcquireRequest {
                command_id: "phase4-formal-foreman-acquire".to_owned(),
                expected_head: None,
                project_id: identity.project_id().clone(),
                project_snapshot_id: identity.project_snapshot_id().clone(),
                task_id: identity.task_id().clone(),
                task_revision: identity.task_revision().to_owned(),
                task_spec_digest: identity
                    .task_spec_digest()
                    .cloned()
                    .expect("fixed Foreman task spec"),
                attempt_id: AttemptId::new("phase4-formal-foreman-attempt")
                    .expect("formal Foreman attempt"),
                lease_id: "phase4-formal-foreman-lease".to_owned(),
                lease_holder_id: "latticed-foreman-v1".to_owned(),
                worktree_id: "phase4-formal-foreman-worktree".to_owned(),
                holder_process_id: HolderProcessId::new(u64::from(std::process::id()))
                    .expect("formal Foreman holder process"),
                holder_process_start_identity: digest(99_950),
            },
        ))
        .expect("acquire formal Foreman Writer");
    assert_eq!(acquired.outcome, WriterLeaseCommandOutcome::Applied);
    acquired.after.expect("formal Foreman Writer authority")
}

fn append_formal_foreman_checkpoint(
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    writer: &WriterLeaseAuthorityHead,
    generation: u64,
) -> ContentDigest {
    let replay = ledger
        .load_foreman_replay()
        .expect("load formal Foreman replay");
    let checkpoint_id = format!("phase4-formal-foreman-{generation}");
    let occurred_at = format!("2026-08-27T12:59:{generation:02}Z");
    let intent = ForemanCheckpointIntent::new(
        checkpoint_id.clone(),
        generation,
        occurred_at.clone(),
        ForemanState::Active,
        None,
        format!("heartbeat:sha256:{}", digest(99_960 + generation).as_str()),
        format!("evidence:sha256:{}", digest(99_970 + generation).as_str()),
    )
    .expect("formal Foreman checkpoint intent");
    let snapshot = SoleForemanBinding::observe_git(
        "refs/heads/phase4-postgres-live",
        "worktree:phase4-postgres-live",
        format!("{generation:040x}"),
    )
    .expect("formal SoleForeman observation")
    .bind(
        &intent,
        format!("authority:sha256:{}", writer.receipt_digest().as_str()),
    )
    .expect("bind formal Foreman checkpoint");
    let plan = plan_foreman_snapshot_append(
        replay.ledger().stream(),
        replay.records(),
        ForemanAppendMetadata::new(
            CommandId::new(checkpoint_id).expect("formal Foreman command"),
            CorrelationId::new(format!("phase4-formal-foreman-correlation-{generation}"))
                .expect("formal Foreman correlation"),
            occurred_at,
        )
        .expect("formal Foreman append metadata"),
        snapshot,
    )
    .expect("plan formal Foreman checkpoint");
    ledger
        .execute_foreman(&plan, authority, writer)
        .expect("persist formal Foreman checkpoint")
        .result_checkpoint()
        .checkpoint_digest()
        .clone()
}

fn provision_capacity_project(
    runtime_url: &str,
    target: &MigrationTarget,
    authority: &StoreAuthorityHead,
    task_number: u8,
) -> (ProjectId, ProjectSnapshotId, ContentDigest) {
    let project_id =
        ProjectId::new(format!("foreman-capacity-{task_number}")).expect("fixture project");
    let observation = RepositoryObservation::new(
        format!("C:/lattice/foreman-capacity-{task_number}"),
        digest(u64::from(task_number) * 100 + 71),
        digest(u64::from(task_number) * 100 + 72),
        digest(u64::from(task_number) * 100 + 73),
        GitRefIdentity::new("refs/heads/main", digest(u64::from(task_number) * 100 + 74))
            .expect("fixture project ref"),
    )
    .expect("fixture project observation");
    let mut registry = PostgresProjectRegistry::new(connect_store_runtime(runtime_url), target)
        .expect("verified Project Registry runtime");
    let vacant = registry.load().expect("vacant Project Registry");
    assert!(vacant.state().project(&project_id).is_none());
    let registered = registry
        .execute(
            ProjectRegistryCommand::register(
                RegistryCommandId::new(format!("foreman-capacity-register-{task_number}"))
                    .expect("registry register command"),
                project_id.clone(),
                ProjectClass::UserProject,
                observation.clone(),
            ),
            authority.clone(),
        )
        .expect("register capacity project");
    assert!(matches!(
        registered.semantic_receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    let registered_authority = registered
        .semantic_receipt()
        .authority()
        .expect("registered project authority")
        .clone();
    let observed = registry
        .execute(
            ProjectRegistryCommand::observe(
                RegistryCommandId::new(format!("foreman-capacity-observe-{task_number}"))
                    .expect("registry observe command"),
                project_id.clone(),
                registered_authority.head(),
                observation.clone(),
            ),
            authority.clone(),
        )
        .expect("observe capacity project");
    assert!(matches!(
        observed.semantic_receipt().outcome(),
        RegistryCommandOutcome::Applied
    ));
    drop(registry);

    let mut reloaded = PostgresProjectRegistry::new(connect_store_runtime(runtime_url), target)
        .expect("reconnect Project Registry runtime");
    let loaded = reloaded.load().expect("reload capacity project");
    let projection = loaded
        .state()
        .project(&project_id)
        .expect("current capacity project projection");
    assert_eq!(projection.observation(), &observation);
    assert!(projection.pending_observation().is_none());
    assert!(projection.drift().is_empty());
    assert_eq!(projection.authority().lifecycle(), ProjectLifecycle::Active);
    assert_eq!(projection.authority().runtime(), RuntimeKind::Live);
    (
        project_id,
        projection.authority().project_snapshot_id().clone(),
        projection.authority().receipt_digest().clone(),
    )
}

fn assert_capacity_submission_rejects_unverified_registry(
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    project_id: &ProjectId,
    snapshot_id: &ProjectSnapshotId,
    task_number: u8,
) {
    let wrong_receipt_identity = TaskLedgerStreamIdentity::new_general_task_intake(
        project_id.clone(),
        snapshot_id.clone(),
        TaskId::new(format!("TASK-FOREMAN-CAPACITY-WRONG-{task_number}"))
            .expect("wrong-receipt task"),
        "1",
        digest(u64::from(task_number) * 100 + 81),
    )
    .expect("wrong-receipt identity");
    let wrong_receipt = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        format!("foreman-capacity-wrong-receipt-{task_number}"),
        "reject a substituted Project Registry receipt",
        "Foreman Capacity",
        wrong_receipt_identity,
        digest(u64::from(task_number) * 100 + 82),
    )
    .expect("wrong-receipt submission");
    assert_registry_receipt_rejected(ledger, authority, &wrong_receipt, task_number, "wrong");

    let missing_project =
        ProjectId::new(format!("foreman-capacity-missing-{task_number}")).expect("missing project");
    let missing_identity = TaskLedgerStreamIdentity::new_general_task_intake(
        missing_project,
        ProjectSnapshotId::new(format!("foreman-capacity-missing-{task_number}:registry:1"))
            .expect("missing snapshot"),
        TaskId::new(format!("TASK-FOREMAN-CAPACITY-MISSING-{task_number}"))
            .expect("missing-receipt task"),
        "1",
        digest(u64::from(task_number) * 100 + 83),
    )
    .expect("missing-receipt identity");
    let missing_receipt = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        format!("foreman-capacity-missing-receipt-{task_number}"),
        "reject a missing Project Registry receipt",
        "Foreman Capacity",
        missing_identity,
        digest(u64::from(task_number) * 100 + 84),
    )
    .expect("missing-receipt submission");
    assert_registry_receipt_rejected(ledger, authority, &missing_receipt, task_number, "missing");
}

fn assert_registry_receipt_rejected(
    ledger: &mut PostgresTaskLedger,
    authority: &StoreAuthorityHead,
    submission: &TaskSubmissionEnvelope,
    task_number: u8,
    kind: &str,
) {
    let vacant = lattice_task_ledger::VerifiedStream::vacant(
        submission.identity().clone(),
        RuntimeKind::Live,
    )
    .expect("vacant rejected submission stream");
    let command = AppendCommand::new_general_task_created(
        vacant.head().clone(),
        CommandId::new(format!("mcp-submit:{}", submission.client_request_id()))
            .expect("rejected submission command"),
        CorrelationId::new(format!("foreman-capacity-{kind}-receipt-{task_number}"))
            .expect("rejected submission correlation"),
        timestamp(task_number, 9),
        ActorId::new("lattice-runtime").expect("actor"),
        submission,
    )
    .expect("rejected submission command");
    let error = ledger
        .execute_submission(command, authority.clone(), submission.clone())
        .expect_err("unverified Project Registry receipt must reject submission");
    assert_eq!(
        error.kind(),
        PostgresTaskLedgerErrorKind::ProjectRegistryCurrentnessConflict
    );
}

fn activate_fixture_authority(migrator: &mut Client) -> StoreAuthorityHead {
    let authority = StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new("foreman-capacity-harness").expect("daemon"),
        DaemonEpoch::new(1).expect("epoch"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(1).expect("revision"),
        digest(9_001),
        digest(9_002),
    )
    .expect("authority");
    migrator
        .execute(
            "UPDATE ONLY control.runtime_admission \
                SET admission_mode='ACTIVE', daemon_instance_id=$1, daemon_epoch=$2, \
                    authority_revision=$3, observation_digest=decode($4,'hex'), \
                    authority_head_digest=decode($5,'hex'), updated_at=clock_timestamp() \
              WHERE singleton=true",
            &[
                &authority.daemon_instance_id().as_str(),
                &i64::try_from(authority.daemon_epoch().get()).expect("epoch i64"),
                &i64::try_from(authority.revision().get()).expect("revision i64"),
                &authority.observation_digest().as_str(),
                &authority.head_digest().as_str(),
            ],
        )
        .expect("activate disposable runtime");
    authority
}

fn connect_store_runtime(url: &str) -> Client {
    let mut config = url.parse::<Config>().expect("runtime URL");
    config.application_name("lattice-devos-task019");
    let mut client = config.connect(NoTls).expect("connect Store runtime");
    client
        .batch_execute("SET ROLE lattice_runtime")
        .expect("set Store runtime role");
    client
}

fn command_id(kind: &str, task_number: u8) -> CommandId {
    CommandId::new(format!("foreman-capacity-{kind}-{task_number}")).expect("command")
}

fn correlation_id(task_number: u8) -> CorrelationId {
    CorrelationId::new(format!("foreman-capacity-correlation-{task_number}")).expect("correlation")
}

fn metadata(kind: &str, task_number: u8, second_offset: u8) -> TaskRuntimeAppendMetadata {
    TaskRuntimeAppendMetadata::new(
        command_id(kind, task_number),
        correlation_id(task_number),
        timestamp(task_number, second_offset),
    )
    .expect("append metadata")
}

fn timestamp(task_number: u8, second_offset: u8) -> String {
    format!("2026-08-26T12:{task_number:02}:{second_offset:02}Z")
}

fn replay_active_claim_from_durable_rows(
    client: &mut Client,
    task_ref: &ContentDigest,
) -> Result<(), postgres::Error> {
    client
        .query(
            "SELECT replay.* \
               FROM ONLY foreman_execution.worker_attempts AS attempt \
               JOIN ONLY foreman_execution.task_promotions AS promotion \
                 ON promotion.task_ref=attempt.task_ref \
               JOIN ONLY foreman_execution.child_events AS child \
                 ON child.ledger_event_digest=attempt.ledger_event_digest \
               CROSS JOIN LATERAL foreman_execution.claim_worker_attempt_v1( \
                    attempt.task_ref,attempt.successor_stream_id, \
                    attempt.task_spec_digest,attempt.binding_digest,attempt.budget_digest, \
                    attempt.attempt_id::text,attempt.attempt_number, \
                    attempt.foreman_generation,attempt.model::text,attempt.reasoning::text, \
                    attempt.writer_fence,attempt.foreman_checkpoint_digest, \
                    attempt.approval_receipt_digest,attempt.packet_digest, \
                    attempt.execution_environment_ref::text,attempt.worktree_digest, \
                    attempt.base_commit_digest,attempt.model_reason::text, \
                    attempt.model_reason_digest,attempt.claimed_at::text, \
                    attempt.payload_digest,(promotion.repair_retry_limit+1)::smallint, \
                    child.ledger_stream_id,child.ledger_event_sequence, \
                    child.ledger_event_digest,child.ledger_command_id::text, \
                    child.ledger_request_digest \
               ) AS replay \
              WHERE attempt.task_ref=decode($1,'hex')",
            &[&task_ref.as_str()],
        )
        .map(|_| ())
}

fn replay_active_reservation_from_durable_rows(
    client: &mut Client,
    task_ref: &ContentDigest,
) -> Result<(), postgres::Error> {
    client
        .query(
            "SELECT foreman_execution.reserve_worker_attempt_v1( \
                    attempt.task_ref,attempt.successor_stream_id, \
                    attempt.task_spec_digest,attempt.binding_digest,attempt.budget_digest, \
                    attempt.attempt_id::text,attempt.attempt_number, \
                    attempt.foreman_generation,attempt.model::text,attempt.reasoning::text, \
                    attempt.writer_fence,attempt.foreman_checkpoint_digest, \
                    attempt.approval_receipt_digest,attempt.packet_digest, \
                    attempt.execution_environment_ref::text,attempt.worktree_digest, \
                    attempt.base_commit_digest,attempt.model_reason::text, \
                    attempt.model_reason_digest,attempt.claimed_at::text, \
                    attempt.payload_digest,(promotion.repair_retry_limit+1)::smallint, \
                    child.ledger_stream_id,child.ledger_event_sequence, \
                    child.ledger_event_digest,child.ledger_command_id::text, \
                    child.ledger_request_digest \
               ) \
               FROM ONLY foreman_execution.worker_attempts AS attempt \
               JOIN ONLY foreman_execution.task_promotions AS promotion \
                 ON promotion.task_ref=attempt.task_ref \
               JOIN ONLY foreman_execution.child_events AS child \
                 ON child.ledger_event_digest=attempt.ledger_event_digest \
              WHERE attempt.task_ref=decode($1,'hex')",
            &[&task_ref.as_str()],
        )
        .map(|_| ())
}

fn assert_record_execution_environment_v1_rejects(
    runtime: &mut Client,
    record: &VerifiedWorkerAttemptRecord,
    descriptor: &Value,
    case_name: &str,
    expected_message: &str,
) {
    let descriptor_json =
        serde_json::to_string(descriptor).expect("negative execution-environment descriptor JSON");
    let environment_ref = descriptor["identity_digest"]
        .as_str()
        .expect("negative execution-environment identity")
        .to_owned();
    let rejected = runtime
        .query_one(
            "SELECT foreman_execution.record_execution_environment_v1( \
                decode($1,'hex'),$2,$3,decode($4,'hex'),$5,$6)",
            &[
                &record.task_ref().as_str(),
                &i16::try_from(record.attempt_number()).expect("attempt number"),
                &record.attempt_id().as_str(),
                &record.packet_digest().as_str(),
                &descriptor_json,
                &environment_ref,
            ],
        )
        .expect_err(case_name);
    assert_eq!(
        rejected
            .as_db_error()
            .map(postgres::error::DbError::message),
        Some(expected_message),
        "unexpected fail-closed SQL ingress error for {case_name}"
    );
}

fn live_execution_environment(
    task_ref: &ContentDigest,
    cargo_digest_byte: char,
) -> Result<ExecutionEnvironmentDescriptor, lattice_postgres_foreman::AdapterError> {
    let descriptor = live_execution_environment_json(task_ref, cargo_digest_byte);
    ExecutionEnvironmentDescriptor::from_json(
        &serde_json::to_string(&descriptor).expect("live execution-environment JSON"),
    )
}

fn live_execution_environment_json(task_ref: &ContentDigest, cargo_digest_byte: char) -> Value {
    live_execution_environment_json_with_task_root(task_ref, cargo_digest_byte, "/home/lattice")
}

fn live_execution_environment_json_with_task_root(
    task_ref: &ContentDigest,
    cargo_digest_byte: char,
    task_root: &str,
) -> Value {
    let task_ref = task_ref.as_str();
    let isolation_root = format!("{task_root}/verifier-state/{task_ref}");
    let repository = format!("{task_root}/managed-worktrees/{task_ref}");
    let launcher = format!("{task_root}/codex/bin/codex");
    let mut descriptor = json!({
        "schema": "lattice.execution-environment.wsl2-linux/1.1",
        "kind": "WSL2_LINUX",
        "distribution": "Ubuntu",
        "distribution_identity": {
            "os_id": "ubuntu",
            "os_version_id": "26.04",
            "os_version_codename": "resolute",
            "os_release_sha256": "1".repeat(64),
            "kernel_release": "6.18.33.2-microsoft-standard-WSL2",
            "identity_digest": Value::Null
        },
        "gateway": {
            "windows_path": r"C:\Windows\System32\wsl.exe",
            "version": "2.6.1",
            "sha256": "2".repeat(64)
        },
        "linux": {
            "launcher_path": launcher.clone(),
            "launcher_version": "codex-cli 0.146.0",
            "launcher_sha256": "3".repeat(64),
            "node_path": format!("{task_root}/toolchain-node-24.15.0/root/bin/node"),
            "node_version": "v24.15.0",
            "node_sha256": "4".repeat(64),
            "git_path": "/usr/bin/git",
            "git_version": "git version 2.53.0",
            "git_sha256": "5".repeat(64),
            "supervisor_path": format!("{task_root}/runtime-v1/wsl2-codex-supervisor.mjs"),
            "supervisor_sha256": "6".repeat(64),
            "codex_home": format!("{task_root}/codex-home"),
            "config_digest": format!("codex-config:sha256:{}", "7".repeat(64)),
            "cwd": repository,
            "repository_head": "0123456789abcdef0123456789abcdef01234567",
            "repository_identity": format!("repository:sha256:{}", "8".repeat(64)),
            "dbus_run_session_path": "/usr/bin/dbus-run-session",
            "dbus_run_session_sha256": "9".repeat(64),
            "setsid_path": "/usr/bin/setsid",
            "setsid_sha256": "a".repeat(64),
            "keyring_daemon_path": format!(
                "{task_root}/keyring-static-v1/root/usr/bin/gnome-keyring-daemon"
            ),
            "keyring_daemon_sha256": "b".repeat(64),
            "keyring_library_path": format!("{task_root}/keyring-static-v1/packages"),
            "keyring_library_manifest_digest": format!(
                "keyring-library-manifest:sha256:{}", "f".repeat(64)
            ),
            "xdg_runtime_dir": "/run/user/1000"
        },
        "credential_authority": {
            "kind": "LINUX_KEYRING",
            "authority_digest": Value::Null
        },
        "process_fence": {
            "schema": "lattice.wsl2-cgroup-v2-fence/1.0",
            "kind": "SYSTEMD_USER_SERVICE_CGROUP_V2",
            "systemd_run_path": "/usr/bin/systemd-run",
            "systemd_run_version": "systemd 259",
            "systemd_run_sha256": "c".repeat(64),
            "systemctl_path": "/usr/bin/systemctl",
            "systemctl_version": "systemd 259",
            "systemctl_sha256": "d".repeat(64),
            "cgroup_mount": "/sys/fs/cgroup",
            "user_runtime_dir": "/run/user/1000",
            "unit_prefix": format!("lattice-wsl2-{}", &task_ref[..16]),
            "supervisor_bootstrap_node": {
                "path": "/usr/bin/node",
                "version": "v22.22.1",
                "sha256": "8".repeat(64)
            },
            "immutable_probe_lsattr": {
                "path": "/usr/bin/lsattr",
                "version": "lsattr 1.47.2 (1-Jan-2025)",
                "sha256": "9".repeat(64)
            },
            "noninteractive_root_probe": {
                "path": "/usr/bin/sudo",
                "version": "Sudo version 1.9.16p2",
                "sha256": "a".repeat(64)
            },
            "identity_digest": Value::Null
        },
        "verification_toolchain": {
            "schema": "lattice.wsl2-verification-toolchain/1.0",
            "task_ref": task_ref,
            "task_root": task_root,
            "isolation_root": isolation_root,
            "owner_uid": 1000,
            "home_dir": format!("{isolation_root}/home"),
            "temp_dir": format!("{isolation_root}/tmp"),
            "npm_cache": format!("{isolation_root}/npm-cache"),
            "cargo_home": format!("{isolation_root}/cargo-home"),
            "cargo_target_dir": format!("{isolation_root}/cargo-target"),
            "cargo_host": "x86_64-unknown-linux-gnu",
            "npm": {
                "path": format!("{task_root}/toolchain-node-24.15.0/root/lib/node_modules/npm/bin/npm-cli.js"),
                "version": "11.12.1",
                "sha256": "e".repeat(64)
            },
            "cargo": {
                "path": format!("{task_root}/toolchain-rust-1.97.1/bin/cargo"),
                "version": "cargo 1.97.1 (c980f4866 2026-06-30)",
                "sha256": cargo_digest_byte.to_string().repeat(64)
            },
            "rustc": {
                "path": format!("{task_root}/toolchain-rust-1.97.1/bin/rustc"),
                "version": "rustc 1.97.1 (8bab26f4f 2026-07-14)",
                "sha256": "1".repeat(64)
            },
            "rustdoc": {
                "path": format!("{task_root}/toolchain-rust-1.97.1/bin/rustdoc"),
                "version": "rustdoc 1.97.1 (8bab26f4f 2026-07-14)",
                "sha256": "2".repeat(64)
            },
            "sandbox": {
                "path": launcher,
                "version": "codex-cli 0.146.0",
                "sha256": "3".repeat(64)
            },
            "sandbox_helper": {
                "path": "/usr/bin/bwrap",
                "version": "bubblewrap 0.11.1",
                "sha256": "6".repeat(64)
            },
            "identity_digest": Value::Null
        },
        "immutable_snapshot": {
            "schema": "lattice.wsl2-immutable-snapshot/1.0",
            "task_root_path": task_root,
            "task_root_device": "24",
            "task_root_inode": "8675309",
            "task_root_owner_uid": 0,
            "task_root_owner_gid": 0,
            "task_root_mode": "0555",
            "task_root_immutable": true,
            "trees": {
                "codex": {
                    "root": format!("{task_root}/codex"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "1".repeat(64))
                },
                "supervisor_runtime": {
                    "root": format!("{task_root}/runtime-v1"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "2".repeat(64))
                },
                "node": {
                    "root": format!("{task_root}/toolchain-node-24.15.0"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "3".repeat(64))
                },
                "rust": {
                    "root": format!("{task_root}/toolchain-rust-1.97.1"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "4".repeat(64))
                },
                "keyring": {
                    "root": format!("{task_root}/keyring-static-v1"),
                    "manifest_digest": format!("immutable-tree-manifest:sha256:{}", "5".repeat(64))
                }
            },
            "snapshot_digest": Value::Null
        },
        "sandbox_policy": {
            "schema": "lattice.wsl2-sandbox-policy/1.0",
            "policy_digest": format!("wsl2-sandbox-policy:sha256:{}", "6".repeat(64))
        },
        "privilege_boundary": {
            "schema": "lattice.wsl2-privilege-boundary/1.0",
            "effective_uid": 1000,
            "effective_gid": 1000,
            "effective_capabilities_digest": format!(
                "linux-capabilities:sha256:{}", "7".repeat(64)
            ),
            "noninteractive_root_unavailable": true,
            "boundary_digest": Value::Null
        },
        "path_mapping": {
            "windows_path": format!(
                r"\\wsl.localhost\Ubuntu{}",
                repository.replace('/', "\\")
            ),
            "linux_path": repository,
            "digest": format!("path-mapping:sha256:{}", "f".repeat(64))
        },
        "identity_digest": Value::Null
    });
    rehash_live_execution_environment(&mut descriptor);
    descriptor
}

fn rehash_live_execution_environment(descriptor: &mut Value) {
    let mut distribution = descriptor["distribution_identity"].clone();
    distribution
        .as_object_mut()
        .expect("distribution identity")
        .remove("identity_digest");
    distribution["distribution"] = descriptor["distribution"].clone();
    descriptor["distribution_identity"]["identity_digest"] =
        Value::String(typed_live_json_digest("wsl2-distribution", &distribution));
    let credential = json!({
        "kind": descriptor["credential_authority"]["kind"],
        "distribution_identity_ref": descriptor["distribution_identity"]["identity_digest"],
        "codex_home": descriptor["linux"]["codex_home"],
        "config_digest": descriptor["linux"]["config_digest"],
        "keyring_daemon_path": descriptor["linux"]["keyring_daemon_path"],
        "keyring_daemon_sha256": descriptor["linux"]["keyring_daemon_sha256"],
        "keyring_library_path": descriptor["linux"]["keyring_library_path"],
        "keyring_library_manifest_digest": descriptor["linux"]["keyring_library_manifest_digest"],
        "xdg_runtime_dir": descriptor["linux"]["xdg_runtime_dir"]
    });
    descriptor["credential_authority"]["authority_digest"] = Value::String(typed_live_json_digest(
        "wsl2-credential-authority",
        &credential,
    ));
    let mut fence = descriptor["process_fence"].clone();
    fence
        .as_object_mut()
        .expect("process fence")
        .remove("identity_digest");
    fence["distribution_identity_ref"] =
        descriptor["distribution_identity"]["identity_digest"].clone();
    descriptor["process_fence"]["identity_digest"] = Value::String(typed_live_json_digest(
        "wsl2-process-fence-authority",
        &fence,
    ));
    let mut toolchain = descriptor["verification_toolchain"].clone();
    toolchain
        .as_object_mut()
        .expect("verification toolchain")
        .remove("identity_digest");
    descriptor["verification_toolchain"]["identity_digest"] = Value::String(
        typed_live_json_digest("wsl2-verification-toolchain", &toolchain),
    );
    let mut immutable_snapshot = descriptor["immutable_snapshot"].clone();
    immutable_snapshot
        .as_object_mut()
        .expect("immutable snapshot")
        .remove("snapshot_digest");
    descriptor["immutable_snapshot"]["snapshot_digest"] = Value::String(typed_live_json_digest(
        "wsl2-immutable-snapshot",
        &immutable_snapshot,
    ));
    descriptor["sandbox_policy"]["policy_digest"] = Value::String(typed_live_json_digest(
        "wsl2-sandbox-policy",
        &live_sandbox_policy_template(descriptor),
    ));
    let mut privilege_boundary = descriptor["privilege_boundary"].clone();
    privilege_boundary
        .as_object_mut()
        .expect("privilege boundary")
        .remove("boundary_digest");
    descriptor["privilege_boundary"]["boundary_digest"] = Value::String(typed_live_json_digest(
        "wsl2-privilege-boundary",
        &privilege_boundary,
    ));
    let path_mapping = json!({
        "distribution": descriptor["distribution"],
        "windows_path": descriptor["path_mapping"]["windows_path"],
        "linux_path": descriptor["path_mapping"]["linux_path"],
        "repository_identity": descriptor["linux"]["repository_identity"],
        "repository_head": descriptor["linux"]["repository_head"],
    });
    descriptor["path_mapping"]["digest"] =
        Value::String(typed_live_json_digest("path-mapping", &path_mapping));
    rehash_live_environment_identity_only(descriptor);
}

fn rehash_live_environment_identity_only(descriptor: &mut Value) {
    let mut subject = descriptor.clone();
    subject
        .as_object_mut()
        .expect("execution environment")
        .remove("identity_digest");
    descriptor["identity_digest"] =
        Value::String(typed_live_json_digest("execution-environment", &subject));
}

fn live_sandbox_policy_template(descriptor: &Value) -> Value {
    let linux = &descriptor["linux"];
    let toolchain = &descriptor["verification_toolchain"];
    let task_root = toolchain["task_root"].as_str().expect("task root");
    let linux_home = task_root.split('/').take(3).collect::<Vec<_>>().join("/");
    json!({
        "schema": "lattice.wsl2-sandbox-template/1.0",
        "permission_profile_type": "managed",
        "filesystem_type": "restricted",
        "network": "restricted",
        "base_entries": [
            { "path": { "type": "special", "value": { "kind": "minimal" } }, "access": "read" },
            { "path": { "type": "path", "path": task_root }, "access": "read" }
        ],
        "role_writes": {
            "PREFLIGHT": [
                linux["cwd"], toolchain["home_dir"], toolchain["temp_dir"],
                toolchain["npm_cache"], toolchain["cargo_home"], toolchain["cargo_target_dir"]
            ],
            "NODE": [toolchain["home_dir"], toolchain["temp_dir"], toolchain["npm_cache"]],
            "CARGO": [
                toolchain["home_dir"], toolchain["temp_dir"],
                toolchain["cargo_home"], toolchain["cargo_target_dir"]
            ],
            "GIT": {
                "bootstrap": ["$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR"],
                "guarded_object_write": [
                    "$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR", "$GIT_COMMON_DIR/objects"
                ],
                "guarded_index_write": [
                    "$GIT_CONTROL_HOME", "$GIT_CONTROL_TMPDIR",
                    "$GIT_CONTROL_ROOT/candidate-index"
                ]
            }
        },
        "deny_entries": [
            { "path": linux["codex_home"], "missing_path_behavior": "skip" },
            { "path": format!("{linux_home}/.codex"), "missing_path_behavior": "skip" },
            { "path": "/mnt", "missing_path_behavior": "skip" },
            { "path": linux["xdg_runtime_dir"], "missing_path_behavior": "skip" }
        ],
        "codex_linux_sandbox_exe": Value::Null,
        "sandbox_cwd": format!("file://{}", linux["cwd"].as_str().expect("Linux cwd")),
        "use_legacy_landlock": false
    })
}

fn typed_live_json_digest(domain: &str, value: &Value) -> String {
    let encoded = serde_json::to_vec(&canonical_live_json(value)).expect("canonical JSON");
    let digest = Sha256::digest(encoded)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{domain}:sha256:{digest}")
}

fn canonical_live_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_live_json).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical_live_json(&object[key])))
                    .collect::<Map<_, _>>(),
            )
        }
        _ => value.clone(),
    }
}

fn digest(value: u64) -> ContentDigest {
    ContentDigest::from_sha256(format!("{value:064x}")).expect("fixture digest")
}

fn digest_bytes_for_test(value: &ContentDigest) -> Vec<u8> {
    value
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("digest hex pair");
            u8::from_str_radix(text, 16).expect("digest hex byte")
        })
        .collect()
}

fn content_digest_from_bytes(bytes: &[u8]) -> ContentDigest {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("digest hex formatting");
    }
    ContentDigest::from_sha256(value).expect("digest bytes")
}

fn connect_as(url: &str, role: &str) -> Client {
    let mut client = Client::connect(url, NoTls).expect("connect disposable PostgreSQL");
    client
        .batch_execute(match role {
            "lattice_migrator" => "SET ROLE lattice_migrator",
            "lattice_runtime" => "SET ROLE lattice_runtime",
            _ => panic!("closed live role"),
        })
        .expect("set closed role");
    client
}

fn required(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("missing live environment: {name}"))
}
