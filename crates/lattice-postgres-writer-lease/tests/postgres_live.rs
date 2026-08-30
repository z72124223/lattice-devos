use lattice_contracts::{
    AttemptId, ContentDigest, DaemonEpoch, HolderProcessId, ProjectId, ProjectSnapshotId,
    RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead, StoreAuthorityRevision,
    StoreDaemonInstanceId, TaskId,
};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionSetupErrorKind, ExtensionTarget, PostgresWriterLease,
    V4ExtensionTarget, V5ExtensionTarget, WRITER_LEASE_EXTENSION_ID,
    WRITER_LEASE_V1_EXTENSION_PATH, apply_extension, apply_v5_extension,
    verify_embedded_v1_extension_manifest, verify_extension,
};
use lattice_writer_lease::{
    AcquireClaim, AcquireCommand, CommandOutcome, LeaseDenial, LeaseObservation,
    MarkSuspectCommand, RecoveryEvidence, ReleaseCommand, UntrustedWriterLeaseSnapshot,
    VerifiedWriterLeaseAggregate, WriterLeaseAcquireRequest, WriterLeaseCheckpoint,
    WriterLeaseCommand, WriterLeaseCommandReceipt, WriterLeaseHeartbeatRequest,
    WriterLeaseMarkSuspectRequest, WriterLeaseProcessHandoffRequest, WriterLeaseReleaseRequest,
    WriterLeaseRepository, WriterLeaseRepositoryCommand, WriterLeaseRepositoryErrorKind,
    apply_plan, plan_command, verify_snapshot_against_checkpoint,
};
use postgres::{Client, IsolationLevel, NoTls, Row, Transaction};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use time::format_description::well_known::Rfc3339;
use time::{Duration as TimeDuration, OffsetDateTime};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn digest_env(name: &str) -> ContentDigest {
    ContentDigest::from_sha256(std::env::var(name).unwrap_or_else(|_| panic!("{name} is required")))
        .unwrap_or_else(|_| panic!("{name} must be lowercase SHA-256"))
}

fn digest_bytes(value: &ContentDigest) -> Vec<u8> {
    value
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("digest pair");
            u8::from_str_radix(text, 16).expect("digest hex")
        })
        .collect()
}

fn authority(
    daemon_instance_id: String,
    daemon_epoch: u64,
    revision: u64,
    observation_digest: ContentDigest,
    head_digest: ContentDigest,
) -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new(daemon_instance_id).expect("daemon identity"),
        DaemonEpoch::new(daemon_epoch).expect("daemon epoch"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(revision).expect("authority revision"),
        observation_digest,
        head_digest,
    )
    .expect("store authority")
}

fn authority_env() -> StoreAuthorityHead {
    authority(
        std::env::var("LATTICE_WRITER_LEASE_DAEMON_INSTANCE_ID")
            .expect("daemon instance ID is required when the live test is enabled"),
        std::env::var("LATTICE_WRITER_LEASE_DAEMON_EPOCH")
            .expect("daemon epoch is required when the live test is enabled")
            .parse()
            .expect("daemon epoch must be an unsigned integer"),
        std::env::var("LATTICE_WRITER_LEASE_AUTHORITY_REVISION")
            .expect("authority revision is required when the live test is enabled")
            .parse()
            .expect("authority revision must be an unsigned integer"),
        digest_env("LATTICE_WRITER_LEASE_ADMISSION_OBSERVATION_SHA256"),
        digest_env("LATTICE_WRITER_LEASE_AUTHORITY_HEAD_SHA256"),
    )
}

fn task076_fixture_stop_admission(migrator: &mut Client, store_authority: &StoreAuthorityHead) {
    let daemon_epoch = i64::try_from(store_authority.daemon_epoch().get()).expect("daemon epoch");
    let revision = i64::try_from(store_authority.revision().get()).expect("authority revision");
    let observation_digest = digest_bytes(store_authority.observation_digest());
    let head_digest = digest_bytes(store_authority.head_digest());
    let mut transaction = migrator
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("TASK-076 fixture STOPPED transaction");
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on; SET LOCAL synchronous_commit=on;",
        )
        .expect("TASK-076 fixture admission boundary");
    assert_eq!(
        transaction
            .execute(
                "UPDATE ONLY control.runtime_admission SET admission_mode='STOPPED', \
                    daemon_instance_id=NULL,daemon_epoch=NULL,authority_revision=0, \
                    observation_digest=NULL,authority_head_digest=NULL, \
                    updated_at=pg_catalog.clock_timestamp() \
                  WHERE singleton AND admission_mode='ACTIVE' \
                    AND daemon_instance_id=$1 AND daemon_epoch=$2 \
                    AND authority_revision=$3 AND observation_digest=$4 \
                    AND authority_head_digest=$5",
                &[
                    &store_authority.daemon_instance_id().as_str(),
                    &daemon_epoch,
                    &revision,
                    &observation_digest,
                    &head_digest,
                ],
            )
            .expect("TASK-076 fixture ACTIVE to STOPPED"),
        1
    );
    let stopped: bool = transaction
        .query_one(
            "SELECT admission_mode='STOPPED' AND daemon_instance_id IS NULL \
                    AND daemon_epoch IS NULL AND authority_revision=0 \
                    AND observation_digest IS NULL AND authority_head_digest IS NULL \
               FROM ONLY control.runtime_admission WHERE singleton",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .expect("TASK-076 exact STOPPED fixture");
    assert!(stopped, "migration denial must run under exact STOPPED");
    transaction
        .commit()
        .expect("commit TASK-076 fixture STOPPED");
}

fn task076_fixture_restore_admission(migrator: &mut Client, store_authority: &StoreAuthorityHead) {
    let daemon_epoch = i64::try_from(store_authority.daemon_epoch().get()).expect("daemon epoch");
    let revision = i64::try_from(store_authority.revision().get()).expect("authority revision");
    let observation_digest = digest_bytes(store_authority.observation_digest());
    let head_digest = digest_bytes(store_authority.head_digest());
    let mut transaction = migrator
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("TASK-076 fixture ACTIVE transaction");
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on; SET LOCAL synchronous_commit=on;",
        )
        .expect("TASK-076 fixture admission boundary");
    assert_eq!(
        transaction
            .execute(
                "UPDATE ONLY control.runtime_admission SET admission_mode='ACTIVE', \
                    daemon_instance_id=$1,daemon_epoch=$2,authority_revision=$3, \
                    observation_digest=$4,authority_head_digest=$5, \
                    updated_at=pg_catalog.clock_timestamp() \
                  WHERE singleton AND admission_mode='STOPPED' \
                    AND daemon_instance_id IS NULL AND daemon_epoch IS NULL \
                    AND authority_revision=0 AND observation_digest IS NULL \
                    AND authority_head_digest IS NULL",
                &[
                    &store_authority.daemon_instance_id().as_str(),
                    &daemon_epoch,
                    &revision,
                    &observation_digest,
                    &head_digest,
                ],
            )
            .expect("TASK-076 fixture STOPPED to ACTIVE"),
        1
    );
    let row = transaction
        .query_one(
            "SELECT admission_mode::text,daemon_instance_id::text,daemon_epoch, \
                    authority_revision,observation_digest,authority_head_digest \
               FROM ONLY control.runtime_admission WHERE singleton",
            &[],
        )
        .expect("TASK-076 restored ACTIVE fixture");
    assert_eq!(row.get::<_, String>(0), "ACTIVE");
    assert_eq!(
        row.get::<_, String>(1),
        store_authority.daemon_instance_id().as_str()
    );
    assert_eq!(row.get::<_, i64>(2), daemon_epoch);
    assert_eq!(row.get::<_, i64>(3), revision);
    assert_eq!(row.get::<_, Vec<u8>>(4), observation_digest);
    assert_eq!(row.get::<_, Vec<u8>>(5), head_digest);
    transaction
        .commit()
        .expect("commit TASK-076 fixture ACTIVE");
}

fn assert_profile_rejects(migrator: &mut Client, target: &ExtensionTarget) {
    assert_eq!(
        verify_extension(migrator, target)
            .expect_err("catalog drift must fail closed")
            .kind(),
        ExtensionSetupErrorKind::PartialOrCollidingProfile
    );
}

fn assert_profile_restored(migrator: &mut Client, target: &ExtensionTarget) {
    verify_extension(migrator, target).expect("exact catalog profile restored");
}

fn assert_task076_phase_profile(migrator: &mut Client, phase: &str) {
    let row = migrator
        .query_one(
            "SELECT i.extension_schema_version, i.global_schema_version, \
                    i.required_memory_schema_version, \
                    (SELECT pg_catalog.string_agg(\
                         l.ledger_ordinal::text || ':' || l.event_kind::text, ',' \
                         ORDER BY l.ledger_ordinal) \
                       FROM ONLY writer_lease.writer_lease_extension_ledger AS l), \
                    pg_catalog.has_schema_privilege(\
                        'lattice_runtime','writer_lease','USAGE'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                     WHERE n.nspname='writer_lease' \
                       AND pg_catalog.has_function_privilege(\
                           'lattice_runtime',p.oid,'EXECUTE')) \
               FROM ONLY writer_lease.writer_lease_extension_identity AS i \
              WHERE i.singleton",
            &[],
        )
        .expect("TASK-076 exact phase profile");
    let extension: i16 = row.get(0);
    let global: i16 = row.get(1);
    let memory: i16 = row.get(2);
    let ledger: String = row.get(3);
    let runtime_usage: bool = row.get(4);
    let runtime_functions: i64 = row.get(5);
    assert_eq!(extension, 2);
    match phase {
        "bridge" => {
            assert_eq!((global, memory), (3, 2));
            assert_eq!(ledger, "1:INSTALLED,2:UPGRADED");
            assert!(!runtime_usage);
            assert_eq!(runtime_functions, 0);
        }
        "activate" | "runtime" | "restart" => {
            assert_eq!((global, memory), (5, 3));
            assert_eq!(ledger, "1:INSTALLED,2:UPGRADED,3:REBOUND");
            assert!(runtime_usage);
            assert_eq!(runtime_functions, 7);
        }
        _ => panic!("unsupported TASK-076 Writer phase"),
    }
}

fn assert_task076_fresh_current_profile(migrator: &mut Client) {
    let mut transaction = migrator
        .transaction()
        .expect("TASK-076 fresh-current profile transaction");
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on;",
        )
        .expect("TASK-076 fresh-current profile boundary");
    let row = transaction
        .query_one(
            "SELECT i.extension_schema_version, i.global_schema_version, \
                    i.required_memory_schema_version, \
                    (SELECT pg_catalog.string_agg(\
                         l.ledger_ordinal::text || ':' || l.event_kind::text, ',' \
                         ORDER BY l.ledger_ordinal) \
                       FROM ONLY writer_lease.writer_lease_extension_ledger AS l), \
                    pg_catalog.has_schema_privilege(\
                        'lattice_runtime','writer_lease','USAGE'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                     WHERE n.nspname='writer_lease' \
                       AND pg_catalog.has_function_privilege(\
                           'lattice_runtime',p.oid,'EXECUTE')) \
               FROM ONLY writer_lease.writer_lease_extension_identity AS i \
              WHERE i.singleton",
            &[],
        )
        .expect("TASK-076 exact fresh-current profile");
    assert_eq!(row.get::<_, i16>(0), 2);
    assert_eq!((row.get::<_, i16>(1), row.get::<_, i16>(2)), (5, 3));
    assert_eq!(row.get::<_, String>(3), "1:INSTALLED");
    assert!(row.get::<_, bool>(4));
    assert_eq!(row.get::<_, i64>(5), 7);
    transaction
        .commit()
        .expect("TASK-076 fresh-current profile commit");
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Task076FreshProfileEvidence {
    database_uuid: String,
    fingerprint: String,
}

fn task076_fresh_profile_evidence(
    migrator: &mut Client,
    target: &ExtensionTarget,
) -> Task076FreshProfileEvidence {
    verify_extension(migrator, target).expect("exact TASK-076 fresh-current extension");
    assert_task076_fresh_current_profile(migrator);
    let mut transaction = migrator
        .transaction()
        .expect("TASK-076 fresh profile evidence transaction");
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on;",
        )
        .expect("TASK-076 fresh profile evidence boundary");
    let row = transaction
        .query_one(
            "SELECT i.database_uuid::text, \
                    pg_catalog.jsonb_build_array(\
                      i.singleton,i.extension_id::text,i.extension_schema_version,\
                      i.extension_path::text,pg_catalog.btrim(i.extension_sql_sha256),\
                      pg_catalog.btrim(i.extension_manifest_sha256),i.database_uuid::text,\
                      pg_catalog.btrim(i.database_identity_sha256),i.global_schema_version,\
                      pg_catalog.btrim(i.global_manifest_sha256),\
                      i.required_memory_schema_version,\
                      pg_catalog.btrim(i.required_memory_manifest_sha256),\
                      (SELECT pg_catalog.jsonb_agg(pg_catalog.jsonb_build_array(\
                         l.ledger_ordinal,l.singleton,l.extension_id::text,\
                         l.extension_schema_version,pg_catalog.btrim(l.extension_sql_sha256),\
                         pg_catalog.btrim(l.extension_manifest_sha256),l.database_uuid::text,\
                         pg_catalog.btrim(l.database_identity_sha256),l.global_schema_version,\
                         pg_catalog.btrim(l.global_manifest_sha256),\
                         l.required_memory_schema_version,\
                         pg_catalog.btrim(l.required_memory_manifest_sha256),\
                         l.event_kind::text) ORDER BY l.ledger_ordinal)\
                         FROM ONLY writer_lease.writer_lease_extension_ledger AS l),\
                      (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_heads),\
                      (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_commands),\
                      (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_transitions),\
                      pg_catalog.has_schema_privilege(\
                          'lattice_runtime','writer_lease','USAGE'),\
                      (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                        JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                       WHERE n.nspname='writer_lease' \
                         AND pg_catalog.has_function_privilege(\
                             'lattice_runtime',p.oid,'EXECUTE')))::text,\
                    (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_heads),\
                    (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_commands),\
                    (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_transitions)\
               FROM ONLY writer_lease.writer_lease_extension_identity AS i \
              WHERE i.singleton",
            &[],
        )
        .expect("TASK-076 owner-only fresh profile evidence");
    let database_uuid: String = row.get(0);
    let canonical_profile: String = row.get(1);
    assert_eq!(row.get::<_, i64>(2), 0, "fresh heads must be empty");
    assert_eq!(row.get::<_, i64>(3), 0, "fresh commands must be empty");
    assert_eq!(row.get::<_, i64>(4), 0, "fresh transitions must be empty");
    let uuid = database_uuid.as_bytes();
    assert_eq!(uuid.len(), 36);
    assert_eq!(uuid[14], b'8');
    assert!(matches!(uuid[19], b'8' | b'9' | b'a' | b'b'));
    let mut framed = b"LATTICE_TASK076_WRITER_FRESH_PROFILE_V1\0".to_vec();
    append_task076_bytes(&mut framed, canonical_profile.as_bytes());
    let mut fingerprint = String::with_capacity(64);
    for byte in Sha256::digest(framed) {
        use std::fmt::Write as _;
        write!(&mut fingerprint, "{byte:02x}").expect("writing to String cannot fail");
    }
    transaction
        .commit()
        .expect("TASK-076 fresh profile evidence commit");
    Task076FreshProfileEvidence {
        database_uuid,
        fingerprint,
    }
}

fn task076_run_id() -> String {
    let run_id = std::env::var("LATTICE_TASK019_RUN_ID")
        .expect("LATTICE_TASK019_RUN_ID is required for TASK-076 Writer phases");
    assert!(
        (8..=40).contains(&run_id.len())
            && run_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
        "TASK-076 run ID must be bounded lowercase ASCII"
    );
    run_id
}

fn task076_history_digest(migrator: &mut Client, project_id: &ProjectId) -> String {
    // Deliberately exclude only installed/recorded/updated timestamps. Every
    // owner-semantic physical column, including checkpoint commitments and the
    // caller-owned repository request bytes, is framed below.
    let head: String = migrator
        .query_one(
            "SELECT pg_catalog.jsonb_build_array(\
                h.project_id::text,h.row_version,h.snapshot_schema_version,\
                pg_catalog.encode(h.snapshot_bytes,'hex'),\
                pg_catalog.encode(h.snapshot_bytes_sha256,'hex'),\
                pg_catalog.encode(h.snapshot_digest,'hex'),h.fencing_high_water,\
                h.lease_revision,h.command_high_water,\
                CASE WHEN h.command_tail_digest IS NULL THEN NULL ELSE \
                    pg_catalog.encode(h.command_tail_digest,'hex') END,\
                h.current_status::text,\
                CASE WHEN h.current_receipt_digest IS NULL THEN NULL ELSE \
                    pg_catalog.encode(h.current_receipt_digest,'hex') END,\
                h.current_project_snapshot_id::text,h.current_task_id::text,\
                h.current_task_revision::text,\
                CASE WHEN h.current_task_spec_digest IS NULL THEN NULL ELSE \
                    pg_catalog.encode(h.current_task_spec_digest,'hex') END,\
                h.current_attempt_id::text,h.current_lease_id::text,\
                h.current_lease_holder_id::text,h.current_worktree_id::text,\
                h.current_holder_process_id,\
                CASE WHEN h.current_holder_process_start_identity IS NULL THEN NULL ELSE \
                    pg_catalog.encode(h.current_holder_process_start_identity,'hex') END,\
                h.current_daemon_instance_id::text,h.current_daemon_epoch,\
                h.current_fencing_token,h.current_expires_at)::text \
             FROM ONLY writer_lease.writer_lease_heads AS h WHERE h.project_id=$1",
            &[&project_id.as_str()],
        )
        .and_then(|row| row.try_get(0))
        .expect("TASK-076 complete physical head evidence");
    let receipts = migrator
        .query(
            "SELECT pg_catalog.jsonb_build_array(\
                c.project_id::text,c.ordinal,c.command_id::text,\
                pg_catalog.encode(c.repository_request_bytes,'hex'),\
                pg_catalog.encode(c.repository_request_sha256,'hex'),\
                pg_catalog.encode(c.request_bytes,'hex'),\
                pg_catalog.encode(c.request_digest,'hex'),\
                CASE WHEN c.previous_receipt_digest IS NULL THEN NULL ELSE \
                    pg_catalog.encode(c.previous_receipt_digest,'hex') END,\
                c.outcome::text,c.denial_reason::text,\
                CASE WHEN c.transition_digest IS NULL THEN NULL ELSE \
                    pg_catalog.encode(c.transition_digest,'hex') END,\
                pg_catalog.encode(c.receipt_bytes,'hex'),\
                pg_catalog.encode(c.receipt_digest,'hex'))::text \
             FROM ONLY writer_lease.writer_lease_commands AS c \
              WHERE c.project_id=$1 ORDER BY c.ordinal",
            &[&project_id.as_str()],
        )
        .expect("TASK-076 complete physical command evidence");
    let transitions = migrator
        .query(
            "SELECT pg_catalog.jsonb_build_array(\
                t.project_id::text,t.ordinal,t.command_id::text,t.transition_kind::text,\
                pg_catalog.encode(t.transition_bytes,'hex'),\
                pg_catalog.encode(t.transition_digest,'hex'))::text \
             FROM ONLY writer_lease.writer_lease_transitions AS t \
              WHERE t.project_id=$1 ORDER BY t.ordinal",
            &[&project_id.as_str()],
        )
        .expect("TASK-076 complete physical transition evidence");
    let mut framed = b"LATTICE_TASK076_WRITER_PHYSICAL_HISTORY_V1\0".to_vec();
    append_task076_bytes(&mut framed, head.as_bytes());
    framed.extend_from_slice(
        &u64::try_from(receipts.len())
            .expect("bounded receipt count")
            .to_be_bytes(),
    );
    for row in receipts {
        let value: String = row.get(0);
        append_task076_bytes(&mut framed, value.as_bytes());
    }
    framed.extend_from_slice(
        &u64::try_from(transitions.len())
            .expect("bounded transition count")
            .to_be_bytes(),
    );
    for row in transitions {
        let value: String = row.get(0);
        append_task076_bytes(&mut framed, value.as_bytes());
    }
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(framed) {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn append_task076_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(
        &u64::try_from(bytes.len())
            .expect("bounded canonical bytes")
            .to_be_bytes(),
    );
    output.extend_from_slice(bytes);
}

fn task076_command_id(run_id: &str, suffix: &str) -> String {
    format!("task076-{run_id}-{suffix}")
}

fn task076_acquire(
    project_id: &ProjectId,
    run_id: &str,
    suffix: &str,
) -> WriterLeaseRepositoryCommand {
    WriterLeaseRepositoryCommand::Acquire(WriterLeaseAcquireRequest {
        command_id: task076_command_id(run_id, suffix),
        expected_head: None,
        project_id: project_id.clone(),
        project_snapshot_id: ProjectSnapshotId::new("snapshot-task076").expect("snapshot"),
        task_id: TaskId::new("task076-live").expect("task"),
        task_revision: "1".to_owned(),
        task_spec_digest: digest('d'),
        attempt_id: AttemptId::new(format!("attempt-{run_id}-{suffix}")).expect("attempt"),
        lease_id: format!("lease-{run_id}-{suffix}"),
        lease_holder_id: "implementer-task076".to_owned(),
        worktree_id: "worktree-task076-live".to_owned(),
        // TASK-076 phases run in separate processes. A stable synthetic fixture
        // identity is required for an exact repository-intent retry after the
        // physical PostgreSQL restart.
        holder_process_id: HolderProcessId::new(76_076).expect("fixture process ID"),
        holder_process_start_identity: digest('e'),
    })
}

fn task076_release(
    project_id: &ProjectId,
    run_id: &str,
    suffix: &str,
    expected_head: lattice_contracts::WriterLeaseAuthorityHead,
) -> WriterLeaseRepositoryCommand {
    WriterLeaseRepositoryCommand::Release(WriterLeaseReleaseRequest {
        command_id: task076_command_id(run_id, suffix),
        project_id: project_id.clone(),
        expected_head,
    })
}

fn task076_mark_suspect(
    project_id: &ProjectId,
    run_id: &str,
    expected_head: lattice_contracts::WriterLeaseAuthorityHead,
) -> WriterLeaseRepositoryCommand {
    WriterLeaseRepositoryCommand::MarkSuspect(WriterLeaseMarkSuspectRequest {
        command_id: task076_command_id(run_id, "suspect-mark"),
        project_id: project_id.clone(),
        expected_head,
    })
}

const TASK076_GLOBAL_LOCK: i64 = 0x4c41_5454_4943_4501;
const TASK076_MEMORY_LOCK: i64 = 0x4c41_5443_4d45_4d31;
const TASK076_WRITER_LOCK: i64 = 0x4c41_5457_4c45_4131;
const TASK076_V1_BIND_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_bind_runtime_v1($1,$2,$3,$4,$5,$6,$7,$8)";
const TASK076_V1_LOAD_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_load_for_update_v1($1,$2,$3,$4,$5)";
const TASK076_V1_COMMIT_SQL: &str = "SELECT writer_lease.writer_lease_commit_plan_v1(\
    $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,\
    $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,$37,\
    $38,$39,$40,$41,$42,$43,$44,$45,$46,$47,$48)";

#[derive(Clone, Debug, Eq, PartialEq)]
struct Task076HistoryEvidence {
    digest: String,
    row_version: u64,
    fencing_high_water: u64,
    lease_revision: u64,
    command_high_water: u64,
    transition_high_water: u64,
}

fn task076_history_evidence(
    migrator: &mut Client,
    project_id: &ProjectId,
) -> Task076HistoryEvidence {
    let row = migrator
        .query_one(
            "SELECT h.row_version, h.fencing_high_water, h.lease_revision, \
                    h.command_high_water, \
                    (SELECT pg_catalog.count(*) FROM ONLY \
                        writer_lease.writer_lease_transitions AS t \
                      WHERE t.project_id=h.project_id) \
               FROM ONLY writer_lease.writer_lease_heads AS h \
              WHERE h.project_id=$1",
            &[&project_id.as_str()],
        )
        .expect("TASK-076 physical history high-water evidence");
    let nonnegative =
        |index| u64::try_from(row.get::<_, i64>(index)).expect("TASK-076 nonnegative high-water");
    Task076HistoryEvidence {
        digest: task076_history_digest(migrator, project_id),
        row_version: nonnegative(0),
        fencing_high_water: nonnegative(1),
        lease_revision: nonnegative(2),
        command_high_water: nonnegative(3),
        transition_high_water: nonnegative(4),
    }
}

fn assert_task076_v1_profile(migrator: &mut Client, target: &ExtensionTarget) {
    let manifest = verify_embedded_v1_extension_manifest().expect("frozen Writer v1 manifest");
    let row = migrator
        .query_one(
            "SELECT i.extension_id::text, i.extension_schema_version, i.extension_path::text, \
                    pg_catalog.btrim(i.extension_sql_sha256)::text, \
                    pg_catalog.btrim(i.extension_manifest_sha256)::text, \
                    pg_catalog.btrim(i.database_identity_sha256)::text, \
                    i.global_schema_version, pg_catalog.btrim(i.global_manifest_sha256)::text, \
                    i.required_memory_schema_version, \
                    pg_catalog.btrim(i.required_memory_manifest_sha256)::text, \
                    (SELECT pg_catalog.string_agg(l.ledger_ordinal::text || ':' || \
                        l.event_kind::text, ',' ORDER BY l.ledger_ordinal) \
                       FROM ONLY writer_lease.writer_lease_extension_ledger AS l), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                       JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                      WHERE n.nspname='writer_lease' AND c.relkind IN ('r','p','v','m','S','f')), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                       JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                      WHERE n.nspname='writer_lease'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                       JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                      WHERE n.nspname='writer_lease' AND \
                        pg_catalog.has_function_privilege('lattice_runtime',p.oid,'EXECUTE')), \
                    pg_catalog.has_schema_privilege('lattice_runtime','writer_lease','USAGE'), \
                    pg_catalog.obj_description('writer_lease'::regnamespace,'pg_namespace') \
               FROM ONLY writer_lease.writer_lease_extension_identity AS i \
              WHERE i.singleton",
            &[],
        )
        .expect("exact Writer v1 source profile");
    assert_eq!(row.get::<_, String>(0), WRITER_LEASE_EXTENSION_ID);
    assert_eq!(row.get::<_, i16>(1), 1);
    assert_eq!(row.get::<_, String>(2), WRITER_LEASE_V1_EXTENSION_PATH);
    assert_eq!(row.get::<_, String>(3), manifest.sql_sha256().as_str());
    assert_eq!(row.get::<_, String>(4), manifest.manifest_sha256().as_str());
    assert_eq!(
        row.get::<_, String>(5),
        target.database_identity_digest().as_str()
    );
    assert_eq!(row.get::<_, i16>(6), 3);
    assert_eq!(
        row.get::<_, String>(7),
        target.global_manifest_digest().as_str()
    );
    assert_eq!(row.get::<_, i16>(8), 2);
    assert_eq!(
        row.get::<_, String>(9),
        target.memory_manifest_digest().as_str()
    );
    assert_eq!(row.get::<_, String>(10), "1:INSTALLED");
    assert_eq!(row.get::<_, i64>(11), 5);
    assert_eq!(row.get::<_, i64>(12), 7);
    assert_eq!(row.get::<_, i64>(13), 7);
    assert!(row.get::<_, bool>(14));
    assert_eq!(row.get::<_, String>(15), "LATTICE_WRITER_LEASE_SCHEMA_V1");
}

#[allow(clippy::too_many_lines)]
fn run_task076_source_install(migrator: &mut Client, target: &ExtensionTarget) {
    println!("TASK076_WRITER_SOURCE_INSTALL_ENTER");
    println!("TASK076_WRITER_SOURCE_INSTALL_MANIFEST_ENTER");
    let manifest = verify_embedded_v1_extension_manifest().expect("frozen Writer v1 manifest");
    println!("TASK076_WRITER_SOURCE_INSTALL_MANIFEST_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_TRANSACTION_ENTER");
    let mut transaction = migrator
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("TASK-076 Writer v1 install transaction");
    println!("TASK076_WRITER_SOURCE_INSTALL_TRANSACTION_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_BOUNDARY_ENTER");
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on; SET LOCAL lock_timeout='5s'; \
             SET LOCAL statement_timeout='30s'; \
             SET LOCAL idle_in_transaction_session_timeout='30s';",
        )
        .expect("TASK-076 Writer v1 migrator boundary");
    println!("TASK076_WRITER_SOURCE_INSTALL_BOUNDARY_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_LOCKS_ENTER");
    for lock in [
        TASK076_GLOBAL_LOCK,
        TASK076_MEMORY_LOCK,
        TASK076_WRITER_LOCK,
    ] {
        transaction
            .execute("SELECT pg_catalog.pg_advisory_xact_lock($1)", &[&lock])
            .expect("TASK-076 ordered migration lock");
    }
    println!("TASK076_WRITER_SOURCE_INSTALL_LOCKS_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_FOUNDATION_QUERY_ENTER");
    let foundation = transaction
        .query_one(
            "SELECT pg_catalog.current_database()::text, d.database_uuid::text, \
                    pg_catalog.btrim(m.database_identity_sha256)::text, \
                    c.current_schema_version, pg_catalog.btrim(c.manifest_sha256)::text, \
                    m.extension_schema_version, \
                    pg_catalog.btrim(m.extension_manifest_sha256)::text, \
                    a.admission_mode::text, \
                    pg_catalog.to_regnamespace('writer_lease') IS NOT NULL \
               FROM ONLY control.database_identity AS d \
               CROSS JOIN ONLY control.schema_compatibility AS c \
               CROSS JOIN ONLY memory.codebase_memory_extension_identity AS m \
               CROSS JOIN ONLY control.runtime_admission AS a \
              WHERE m.singleton AND a.singleton AND m.database_uuid=d.database_uuid",
            &[],
        )
        .expect("TASK-076 historical G3/M2 foundation");
    println!("TASK076_WRITER_SOURCE_INSTALL_FOUNDATION_QUERY_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_FOUNDATION_PROFILE_ENTER");
    assert_eq!(foundation.get::<_, String>(0), target.database_name());
    let database_uuid: String = foundation.get(1);
    assert_eq!(
        foundation.get::<_, String>(2),
        target.database_identity_digest().as_str()
    );
    assert_eq!(foundation.get::<_, i16>(3), 3);
    assert_eq!(
        foundation.get::<_, String>(4),
        target.global_manifest_digest().as_str()
    );
    assert_eq!(foundation.get::<_, i16>(5), 2);
    assert_eq!(
        foundation.get::<_, String>(6),
        target.memory_manifest_digest().as_str()
    );
    assert_eq!(foundation.get::<_, String>(7), "STOPPED");
    assert!(!foundation.get::<_, bool>(8), "Writer schema must be fresh");
    println!("TASK076_WRITER_SOURCE_INSTALL_FOUNDATION_PROFILE_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_ADMISSION_LOCK_ENTER");
    let locked_admission: String = transaction
        .query_one(
            "SELECT a.admission_mode::text FROM ONLY control.runtime_admission AS a \
              WHERE a.singleton FOR SHARE OF a",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .expect("lock TASK-076 source admission");
    assert_eq!(locked_admission, "STOPPED");
    println!("TASK076_WRITER_SOURCE_INSTALL_ADMISSION_LOCK_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_V1_SQL_ENTER");
    transaction
        .batch_execute(std::str::from_utf8(manifest.bytes()).expect("Writer v1 SQL UTF-8"))
        .expect("install exact frozen Writer v1 SQL");
    println!("TASK076_WRITER_SOURCE_INSTALL_V1_SQL_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_IDENTITY_ENTER");
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO writer_lease.writer_lease_extension_identity (\
                    singleton,extension_id,extension_schema_version,extension_path,\
                    extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                    database_identity_sha256,global_schema_version,global_manifest_sha256,\
                    required_memory_schema_version,required_memory_manifest_sha256\
                 ) VALUES (true,$1,1,$2,$3,$4,$5::text::uuid,$6,3,$7,2,$8)",
                &[
                    &WRITER_LEASE_EXTENSION_ID,
                    &WRITER_LEASE_V1_EXTENSION_PATH,
                    &manifest.sql_sha256().as_str(),
                    &manifest.manifest_sha256().as_str(),
                    &database_uuid,
                    &target.database_identity_digest().as_str(),
                    &target.global_manifest_digest().as_str(),
                    &target.memory_manifest_digest().as_str(),
                ],
            )
            .expect("insert Writer v1 identity"),
        1
    );
    println!("TASK076_WRITER_SOURCE_INSTALL_IDENTITY_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_LEDGER_ENTER");
    assert_eq!(
        transaction
            .execute(
                "INSERT INTO writer_lease.writer_lease_extension_ledger (\
                    ledger_ordinal,singleton,extension_id,extension_schema_version,\
                    extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                    database_identity_sha256,global_schema_version,global_manifest_sha256,\
                    required_memory_schema_version,required_memory_manifest_sha256,event_kind\
                 ) SELECT 1,singleton,extension_id,extension_schema_version,\
                    extension_sql_sha256,extension_manifest_sha256,database_uuid,\
                    database_identity_sha256,global_schema_version,global_manifest_sha256,\
                    required_memory_schema_version,required_memory_manifest_sha256,'INSTALLED'\
                   FROM ONLY writer_lease.writer_lease_extension_identity WHERE singleton",
                &[],
            )
            .expect("insert Writer v1 ledger"),
        1
    );
    println!("TASK076_WRITER_SOURCE_INSTALL_LEDGER_PASS");
    transaction.commit().expect("commit exact Writer v1 source");
    println!("TASK076_WRITER_SOURCE_INSTALL_PROFILE_ENTER");
    assert_task076_v1_profile(migrator, target);
    println!("TASK076_WRITER_SOURCE_INSTALL_PROFILE_PASS");
    println!("TASK076_WRITER_SOURCE_INSTALL_PASS");
}

fn task076_content_digest(bytes: Vec<u8>) -> ContentDigest {
    assert_eq!(bytes.len(), 32, "TASK-076 digest width");
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    ContentDigest::from_sha256(output).expect("TASK-076 digest contract")
}

struct Task076V1Loaded {
    row_version: i64,
    aggregate: VerifiedWriterLeaseAggregate,
    checkpoint: WriterLeaseCheckpoint,
    observation: LeaseObservation,
    daemon_instance_id: String,
    daemon_epoch: DaemonEpoch,
}

fn task076_v1_loaded(row: &Row, project_id: &ProjectId) -> Task076V1Loaded {
    let row_version: i64 = row.get(0);
    let snapshot_bytes: Vec<u8> = row.get(1);
    assert_eq!(
        row.get::<_, Vec<u8>>(2),
        Sha256::digest(&snapshot_bytes).to_vec()
    );
    let snapshot_digest = task076_content_digest(row.get(3));
    let fencing_high_water = u64::try_from(row.get::<_, i64>(4)).expect("fencing high-water");
    let lease_revision = u64::try_from(row.get::<_, i64>(5)).expect("lease revision");
    let command_high_water = u64::try_from(row.get::<_, i64>(6)).expect("command high-water");
    let command_tail = row.get::<_, Option<Vec<u8>>>(7).map(task076_content_digest);
    let snapshot = UntrustedWriterLeaseSnapshot::from_canonical_bytes(&snapshot_bytes)
        .expect("TASK-076 v1 canonical snapshot");
    let checkpoint = WriterLeaseCheckpoint::new(
        project_id.clone(),
        command_high_water,
        command_tail,
        snapshot_digest,
    )
    .expect("TASK-076 v1 checkpoint");
    let aggregate = verify_snapshot_against_checkpoint(&snapshot, &checkpoint)
        .expect("TASK-076 v1 replay-verified aggregate");
    assert_eq!(aggregate.project_id(), project_id);
    assert_eq!(aggregate.fencing_high_water(), fencing_high_water);
    assert_eq!(aggregate.revision(), lease_revision);
    let admission = RuntimeAdmissionMode::ALL
        .into_iter()
        .find(|mode| mode.as_str() == row.get::<_, String>(26))
        .expect("TASK-076 v1 admission");
    let daemon_instance_id: String = row.get::<_, Option<String>>(27).expect("active daemon");
    let daemon_epoch = u64::try_from(row.get::<_, Option<i64>>(28).expect("active epoch"))
        .ok()
        .and_then(|value| DaemonEpoch::new(value).ok())
        .expect("TASK-076 v1 daemon epoch");
    let observation = LeaseObservation {
        runtime: RuntimeKind::Live,
        admission,
        observed_at: row.get(24),
        time_observation_digest: task076_content_digest(row.get(25)),
        admission_observation_digest: task076_content_digest(
            row.get::<_, Option<Vec<u8>>>(29)
                .expect("active observation digest"),
        ),
    };
    let repository_bytes: Option<Vec<u8>> = row.get(30);
    let repository_sha: Option<Vec<u8>> = row.get(31);
    match (&repository_bytes, &repository_sha) {
        (None, None) => {}
        (Some(bytes), Some(sha)) => assert_eq!(sha.as_slice(), Sha256::digest(bytes).as_slice()),
        _ => panic!("TASK-076 v1 repository request pair drift"),
    }
    Task076V1Loaded {
        row_version,
        aggregate,
        checkpoint,
        observation,
        daemon_instance_id,
        daemon_epoch,
    }
}

fn task076_v1_bind(
    runtime: &mut Client,
    target: &ExtensionTarget,
    store_authority: &StoreAuthorityHead,
) {
    let manifest = verify_embedded_v1_extension_manifest().expect("frozen Writer v1 manifest");
    let expected_epoch = i64::try_from(store_authority.daemon_epoch().get()).expect("daemon epoch");
    let admission_digest = digest_bytes(store_authority.observation_digest());
    let mut transaction = runtime
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .expect("TASK-076 Writer v1 bind transaction");
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_runtime; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on; SET LOCAL lock_timeout='5s'; \
             SET LOCAL statement_timeout='30s';",
        )
        .expect("TASK-076 Writer v1 reader boundary");
    let row = transaction
        .query_one(
            TASK076_V1_BIND_SQL,
            &[
                &store_authority.daemon_instance_id().as_str(),
                &expected_epoch,
                &admission_digest,
                &target.database_identity_digest().as_str(),
                &target.global_manifest_digest().as_str(),
                &target.memory_manifest_digest().as_str(),
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
            ],
        )
        .expect("bind exact Writer v1 runtime");
    assert_eq!(
        row.get::<_, String>(0),
        store_authority.daemon_instance_id().as_str()
    );
    assert_eq!(row.get::<_, i64>(1), expected_epoch);
    assert_eq!(row.get::<_, Vec<u8>>(2), admission_digest);
    transaction.commit().expect("commit Writer v1 bind read");
}

fn task076_expiry(observed_at: &str, ttl: TimeDuration) -> String {
    OffsetDateTime::parse(observed_at, &Rfc3339)
        .expect("TASK-076 PostgreSQL observation time")
        .checked_add(ttl)
        .expect("TASK-076 lease expiry")
        .format(&Rfc3339)
        .expect("TASK-076 RFC3339 expiry")
}

fn task076_i64(value: u64) -> i64 {
    i64::try_from(value).expect("TASK-076 signed PostgreSQL bound")
}

#[allow(clippy::too_many_lines)]
fn task076_v1_persist(
    transaction: &mut Transaction<'_>,
    loaded: &Task076V1Loaded,
    next: &VerifiedWriterLeaseAggregate,
    receipt: &WriterLeaseCommandReceipt,
    repository_request_bytes: &[u8],
) {
    let next_snapshot_bytes = next
        .export_canonical_bytes()
        .expect("next canonical snapshot");
    let next_snapshot_sha = Sha256::digest(&next_snapshot_bytes).to_vec();
    let next_checkpoint = next.checkpoint().expect("next checkpoint");
    let expected_snapshot_digest = digest_bytes(loaded.checkpoint.snapshot_digest());
    let expected_tail = loaded.checkpoint.command_tail_digest().map(digest_bytes);
    let next_snapshot_digest = digest_bytes(next_checkpoint.snapshot_digest());
    let next_tail = next_checkpoint.command_tail_digest().map(digest_bytes);
    let receipt_bytes = receipt.canonical_bytes().expect("canonical receipt");
    let request_bytes = receipt
        .request
        .canonical_bytes()
        .expect("canonical live request");
    assert_eq!(
        receipt
            .request
            .repository_intent_canonical_bytes()
            .expect("canonical repository intent from live command"),
        repository_request_bytes,
        "v1 source persistence must retain the exact caller-owned request"
    );
    let transition = receipt.transition_digest.as_ref().map(|expected| {
        let transition = next.transitions().last().expect("terminal transition");
        assert_eq!(transition.ordinal, receipt.ordinal);
        assert_eq!(transition.command_id, receipt.request.command_id());
        assert_eq!(&transition.transition_digest, expected);
        transition
    });
    let transition_bytes = transition.map(|value| {
        value
            .canonical_bytes()
            .expect("canonical Writer transition")
    });
    let transition_kind = transition.map(|value| value.kind.as_str());
    let (outcome, denial_reason) = match receipt.outcome {
        CommandOutcome::Applied => ("APPLIED", None),
        CommandOutcome::Denied(denial) => ("DENIED", Some(denial.as_str())),
    };
    let current = next.current_receipt();
    let current_status = current.map(|value| value.status().as_str());
    let current_receipt_digest = current.map(|value| digest_bytes(value.receipt_digest()));
    let current_project_snapshot_id =
        current.map(|value| value.identity().project_snapshot_id().as_str());
    let current_task_id = current.map(|value| value.identity().task_id().as_str());
    let current_task_revision = current.map(|value| value.identity().task_revision());
    let current_task_spec_digest =
        current.map(|value| digest_bytes(value.identity().task_spec_digest()));
    let current_attempt_id = current.map(|value| value.identity().attempt_id().as_str());
    let current_lease_id = current.map(|value| value.identity().lease_id());
    let current_lease_holder_id = current.map(|value| value.identity().lease_holder_id());
    let current_worktree_id = current.map(|value| value.identity().worktree_id());
    let current_holder_process_id =
        current.map(|value| task076_i64(value.identity().holder_process_id().get()));
    let current_holder_process_start_identity =
        current.map(|value| digest_bytes(value.identity().holder_process_start_identity()));
    let current_daemon_instance_id = current.map(|value| value.identity().daemon_instance_id());
    let current_daemon_epoch =
        current.map(|value| task076_i64(value.identity().daemon_epoch().get()));
    let current_fencing_token =
        current.map(|value| task076_i64(value.identity().fencing_token().get()));
    let current_expires_at =
        current.map(lattice_contracts::WriterLeaseAuthorityReceipt::expires_at);
    let request_digest = digest_bytes(&receipt.request_digest);
    let previous_receipt_digest = receipt.previous_receipt_digest.as_ref().map(digest_bytes);
    let transition_digest = receipt.transition_digest.as_ref().map(digest_bytes);
    let receipt_digest = digest_bytes(&receipt.receipt_digest);
    let time_digest = digest_bytes(&loaded.observation.time_observation_digest);
    let admission_digest = digest_bytes(&loaded.observation.admission_observation_digest);
    let repository_request_sha = Sha256::digest(repository_request_bytes).to_vec();
    let decision: String = transaction
        .query_one(
            TASK076_V1_COMMIT_SQL,
            &[
                &next.project_id().as_str(),
                &loaded.row_version,
                &expected_snapshot_digest,
                &task076_i64(loaded.checkpoint.command_high_water()),
                &expected_tail,
                &loaded.observation.observed_at,
                &time_digest,
                &loaded.observation.admission.as_str(),
                &loaded.daemon_instance_id,
                &task076_i64(loaded.daemon_epoch.get()),
                &admission_digest,
                &next_snapshot_bytes,
                &next_snapshot_sha,
                &next_snapshot_digest,
                &task076_i64(next.fencing_high_water()),
                &task076_i64(next.revision()),
                &task076_i64(next_checkpoint.command_high_water()),
                &next_tail,
                &current_status,
                &current_receipt_digest,
                &current_project_snapshot_id,
                &current_task_id,
                &current_task_revision,
                &current_task_spec_digest,
                &current_attempt_id,
                &current_lease_id,
                &current_lease_holder_id,
                &current_worktree_id,
                &current_holder_process_id,
                &current_holder_process_start_identity,
                &current_daemon_instance_id,
                &current_daemon_epoch,
                &current_fencing_token,
                &current_expires_at,
                &task076_i64(receipt.ordinal),
                &receipt.request.command_id(),
                &repository_request_bytes,
                &repository_request_sha,
                &request_bytes,
                &request_digest,
                &previous_receipt_digest,
                &outcome,
                &denial_reason,
                &transition_digest,
                &receipt_bytes,
                &receipt_digest,
                &transition_kind,
                &transition_bytes,
            ],
        )
        .and_then(|row| row.try_get(0))
        .expect("commit Writer v1 pure plan");
    assert_eq!(decision, "APPLIED");
}

#[allow(clippy::too_many_lines)]
fn task076_v1_execute(
    runtime: &mut Client,
    repository_command: WriterLeaseRepositoryCommand,
    store_authority: &StoreAuthorityHead,
    lease_ttl: TimeDuration,
) -> WriterLeaseCommandReceipt {
    let project_id = repository_command.project_id().clone();
    let repository_request_bytes = repository_command
        .canonical_bytes()
        .expect("TASK-076 canonical repository request");
    let vacant = VerifiedWriterLeaseAggregate::vacant(project_id.clone());
    let vacant_bytes = vacant
        .export_canonical_bytes()
        .expect("vacant canonical bytes");
    let vacant_checkpoint = vacant.checkpoint().expect("vacant checkpoint");
    let vacant_sha = Sha256::digest(&vacant_bytes).to_vec();
    let vacant_digest = digest_bytes(vacant_checkpoint.snapshot_digest());
    let mut transaction = runtime
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("TASK-076 Writer v1 write transaction");
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_runtime; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on; SET LOCAL synchronous_commit=on; \
             SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s';",
        )
        .expect("TASK-076 Writer v1 writer boundary");
    let row = transaction
        .query_one(
            TASK076_V1_LOAD_SQL,
            &[
                &project_id.as_str(),
                &vacant_bytes,
                &vacant_sha,
                &vacant_digest,
                &repository_command.command_id(),
            ],
        )
        .expect("load exact Writer v1 aggregate for update");
    let loaded = task076_v1_loaded(&row, &project_id);
    assert_eq!(loaded.observation.admission, RuntimeAdmissionMode::Active);
    assert_eq!(
        loaded.daemon_instance_id,
        store_authority.daemon_instance_id().as_str()
    );
    assert_eq!(loaded.daemon_epoch, store_authority.daemon_epoch());
    assert_eq!(
        loaded.observation.admission_observation_digest,
        store_authority.observation_digest().clone()
    );
    let expiry = task076_expiry(&loaded.observation.observed_at, lease_ttl);
    let command = match repository_command {
        WriterLeaseRepositoryCommand::Acquire(request) => {
            WriterLeaseCommand::Acquire(AcquireCommand {
                command_id: request.command_id,
                expected_head: request.expected_head,
                claim: AcquireClaim {
                    project_id: request.project_id,
                    project_snapshot_id: request.project_snapshot_id,
                    task_id: request.task_id,
                    task_revision: request.task_revision,
                    task_spec_digest: request.task_spec_digest,
                    attempt_id: request.attempt_id,
                    lease_id: request.lease_id,
                    lease_holder_id: request.lease_holder_id,
                    worktree_id: request.worktree_id,
                    holder_process_id: request.holder_process_id,
                    holder_process_start_identity: request.holder_process_start_identity,
                    daemon_instance_id: loaded.daemon_instance_id.clone(),
                    daemon_epoch: loaded.daemon_epoch,
                },
                observation: loaded.observation.clone(),
                expires_at: expiry,
            })
        }
        WriterLeaseRepositoryCommand::Release(request) => {
            WriterLeaseCommand::Release(ReleaseCommand {
                command_id: request.command_id,
                project_id: request.project_id,
                expected_head: request.expected_head,
                observation: loaded.observation.clone(),
            })
        }
        WriterLeaseRepositoryCommand::MarkSuspect(request) => {
            WriterLeaseCommand::MarkSuspect(MarkSuspectCommand {
                command_id: request.command_id,
                project_id: request.project_id,
                expected_head: request.expected_head,
                observation: loaded.observation.clone(),
            })
        }
        _ => panic!("TASK-076 v1 source accepts only acquire/mark-suspect/release"),
    };
    let plan = plan_command(&loaded.aggregate, &command).expect("TASK-076 Writer v1 pure plan");
    assert!(
        !plan.is_exact_retry(),
        "source seed must create new history"
    );
    let receipt = plan.receipt().clone();
    let next = apply_plan(&loaded.aggregate, plan).expect("TASK-076 apply Writer v1 plan");
    task076_v1_persist(
        &mut transaction,
        &loaded,
        &next,
        &receipt,
        &repository_request_bytes,
    );
    transaction
        .commit()
        .expect("commit TASK-076 Writer v1 history");
    receipt
}

#[allow(clippy::too_many_lines)]
fn run_task076_source_seed(
    migrator: &mut Client,
    runtime_url: &str,
    target: &ExtensionTarget,
    store_authority: &StoreAuthorityHead,
    run_id: &str,
) {
    assert_task076_v1_profile(migrator, target);
    let project_id =
        ProjectId::new(format!("task076-{run_id}")).expect("TASK-076 fixed project ID");
    assert_eq!(
        migrator
            .query_one(
                "SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_heads \
                  WHERE project_id=$1",
                &[&project_id.as_str()],
            )
            .and_then(|row| row.try_get::<_, i64>(0))
            .expect("TASK-076 source absence"),
        0,
        "source_seed may run only once on the marker-owned project"
    );
    let mut runtime = Client::connect(runtime_url, NoTls).expect("TASK-076 Writer v1 runtime");
    task076_v1_bind(&mut runtime, target, store_authority);
    let acquire = task076_v1_execute(
        &mut runtime,
        task076_acquire(&project_id, run_id, "source-acquire"),
        store_authority,
        TimeDuration::seconds(600),
    );
    let head = acquire
        .after
        .clone()
        .expect("TASK-076 source acquired authority");
    let active_before = task076_history_evidence(migrator, &project_id);
    task076_fixture_stop_admission(migrator, store_authority);
    let active_error = apply_extension(migrator, target)
        .expect_err("ACTIVE Writer authority must reject migration");
    assert_eq!(
        active_error.kind(),
        ExtensionSetupErrorKind::UnsupportedFoundation
    );
    assert_task076_v1_profile(migrator, target);
    assert_eq!(
        task076_history_evidence(migrator, &project_id),
        active_before,
        "ACTIVE migration denial must preserve exact W1 history"
    );
    task076_fixture_restore_admission(migrator, store_authority);
    println!("TASK076_WRITER_ACTIVE_DENIAL_PASS");
    task076_v1_execute(
        &mut runtime,
        task076_release(&project_id, run_id, "source-release", head),
        store_authority,
        TimeDuration::seconds(600),
    );

    let suspect_project_id = ProjectId::new(format!("task076-suspect-{run_id}"))
        .expect("TASK-076 legal SUSPECT marker project");
    let suspect_acquire = task076_v1_execute(
        &mut runtime,
        task076_acquire(&suspect_project_id, run_id, "suspect-acquire"),
        store_authority,
        TimeDuration::microseconds(1),
    );
    let suspect_active_head = suspect_acquire
        .after
        .expect("TASK-076 short-lived source authority");
    std::thread::sleep(Duration::from_millis(2));
    let suspect = task076_v1_execute(
        &mut runtime,
        task076_mark_suspect(&suspect_project_id, run_id, suspect_active_head),
        store_authority,
        TimeDuration::seconds(600),
    );
    assert_eq!(suspect.outcome, CommandOutcome::Applied);
    let suspect_head = suspect
        .after
        .expect("TASK-076 legally planned SUSPECT head");
    let suspect_status: String = migrator
        .query_one(
            "SELECT h.current_status::text FROM ONLY writer_lease.writer_lease_heads AS h \
              WHERE h.project_id=$1",
            &[&suspect_project_id.as_str()],
        )
        .and_then(|row| row.try_get(0))
        .expect("TASK-076 SUSPECT projection");
    assert_eq!(suspect_status, "SUSPECT");
    let suspect_before = task076_history_evidence(migrator, &suspect_project_id);
    task076_fixture_stop_admission(migrator, store_authority);
    let suspect_error = apply_extension(migrator, target)
        .expect_err("SUSPECT Writer authority must reject migration");
    assert_eq!(
        suspect_error.kind(),
        ExtensionSetupErrorKind::UnsupportedFoundation
    );
    assert_task076_v1_profile(migrator, target);
    assert_eq!(
        task076_history_evidence(migrator, &suspect_project_id),
        suspect_before,
        "SUSPECT migration denial must preserve exact W1 history"
    );
    task076_fixture_restore_admission(migrator, store_authority);
    println!("TASK076_WRITER_SUSPECT_DENIAL_PASS");
    task076_v1_execute(
        &mut runtime,
        task076_release(&suspect_project_id, run_id, "suspect-release", suspect_head),
        store_authority,
        TimeDuration::seconds(600),
    );
    drop(runtime);
    let evidence = task076_history_evidence(migrator, &project_id);
    assert_eq!(evidence.row_version, 2);
    assert_eq!(evidence.fencing_high_water, 1);
    assert_eq!(evidence.lease_revision, 2);
    assert_eq!(evidence.command_high_water, 2);
    assert_eq!(evidence.transition_high_water, 2);
    let suspect_evidence = task076_history_evidence(migrator, &suspect_project_id);
    assert_eq!(suspect_evidence.row_version, 3);
    assert_eq!(suspect_evidence.fencing_high_water, 1);
    assert_eq!(suspect_evidence.lease_revision, 3);
    assert_eq!(suspect_evidence.command_high_water, 3);
    assert_eq!(suspect_evidence.transition_high_water, 3);
    println!("TASK076_WRITER_SOURCE_SHA256={}", evidence.digest);
    println!("TASK076_WRITER_FENCING_HIGH_WATER=1");
    println!("TASK076_WRITER_COMMAND_HIGH_WATER=2");
    println!("TASK076_WRITER_TRANSITION_HIGH_WATER=2");
    println!("TASK076_WRITER_SOURCE_PASS");
}

fn task076_concurrent_apply(
    blocker_client: &mut Client,
    migrator_url: &str,
    target: &ExtensionTarget,
    trace_bridge: bool,
) -> [ExtensionApplyOutcome; 2] {
    if trace_bridge {
        println!("TASK076_WRITER_BRIDGE_BLOCKER_TX_ENTER");
    }
    let mut blocker = blocker_client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("TASK-076 setup concurrency blocker transaction");
    blocker
        .batch_execute(
            "SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on;",
        )
        .expect("TASK-076 setup concurrency blocker boundary");
    if trace_bridge {
        println!("TASK076_WRITER_BRIDGE_BLOCKER_TX_PASS");
        println!("TASK076_WRITER_BRIDGE_BLOCKER_LOCK_ENTER");
    }
    blocker
        .execute(
            "SELECT pg_catalog.pg_advisory_xact_lock($1)",
            &[&TASK076_GLOBAL_LOCK],
        )
        .expect("hold TASK-076 global advisory lock");
    if trace_bridge {
        println!("TASK076_WRITER_BRIDGE_BLOCKER_LOCK_PASS");
        println!("TASK076_WRITER_BRIDGE_RUNNERS_CONNECTED_ENTER");
    }

    let mut client_a = Client::connect(migrator_url, NoTls).expect("Writer setup runner A");
    let mut client_b = Client::connect(migrator_url, NoTls).expect("Writer setup runner B");
    if trace_bridge {
        println!("TASK076_WRITER_BRIDGE_RUNNERS_CONNECTED_PASS");
        println!("TASK076_WRITER_BRIDGE_RUNNERS_STARTED_ENTER");
    }
    let barrier = Arc::new(Barrier::new(3));
    let barrier_a = Arc::clone(&barrier);
    let target_a = target.clone();
    let runner_a = std::thread::spawn(move || {
        barrier_a.wait();
        apply_extension(&mut client_a, &target_a)
    });
    let barrier_b = Arc::clone(&barrier);
    let target_b = target.clone();
    let runner_b = std::thread::spawn(move || {
        barrier_b.wait();
        apply_extension(&mut client_b, &target_b)
    });
    barrier.wait();
    if trace_bridge {
        println!("TASK076_WRITER_BRIDGE_RUNNERS_STARTED_PASS");
        println!("TASK076_WRITER_BRIDGE_RUNNERS_BLOCKED_ENTER");
    }

    std::thread::sleep(Duration::from_millis(100));
    let runners_pending = (!runner_a.is_finished(), !runner_b.is_finished());
    if trace_bridge {
        println!("TASK076_WRITER_BRIDGE_RUNNERS_BLOCKED_PASS");
        println!("TASK076_WRITER_BRIDGE_BLOCKER_RELEASED_ENTER");
    }
    blocker
        .commit()
        .expect("release TASK-076 global advisory lock");
    if trace_bridge {
        println!("TASK076_WRITER_BRIDGE_BLOCKER_RELEASED_PASS");
        println!("TASK076_WRITER_BRIDGE_RUNNERS_JOINED_ENTER");
    }
    let outcome_a = runner_a.join().expect("Writer setup runner A thread");
    let outcome_b = runner_b.join().expect("Writer setup runner B thread");
    if trace_bridge {
        println!("TASK076_WRITER_BRIDGE_RUNNERS_JOINED_PASS");
    }
    assert!(
        runners_pending.0,
        "both Writer setup runners must remain pending while the global lock is held"
    );
    assert!(
        runners_pending.1,
        "both Writer setup runners must remain pending while the global lock is held"
    );
    [
        outcome_a.expect("Writer setup runner A outcome"),
        outcome_b.expect("Writer setup runner B outcome"),
    ]
}

fn assert_task076_login_apply_boundary(migrator_url: &str, target: &ExtensionTarget) {
    let mut caller = Client::connect(migrator_url, NoTls).expect("Writer setup login caller");
    let login_role = caller
        .query_one("SELECT current_user::text", &[])
        .and_then(|row| row.try_get::<_, String>(0))
        .expect("Writer setup login identity");
    assert_eq!(login_role, "lattice_migrator_login");
    assert_eq!(
        apply_extension(&mut caller, target).expect("normal login Writer reapply"),
        ExtensionApplyOutcome::AlreadyCurrent
    );
    let restored_role = caller
        .query_one("SELECT current_user::text", &[])
        .and_then(|row| row.try_get::<_, String>(0))
        .expect("Writer setup restored login identity");
    assert_eq!(restored_role, "lattice_migrator_login");

    let wrong_identity = if target.database_identity_digest().as_str() == digest('f').as_str() {
        digest('e')
    } else {
        digest('f')
    };
    let wrong_target = ExtensionTarget::new(
        target.database_name().to_owned(),
        wrong_identity,
        target.global_manifest_digest().clone(),
        target.memory_manifest_digest().clone(),
    )
    .expect("bounded wrong Writer target");
    let error = apply_extension(&mut caller, &wrong_target)
        .expect_err("wrong target must fail after taking the setup gate");
    assert_eq!(error.kind(), ExtensionSetupErrorKind::UnsupportedFoundation);
    let error_restored_role = caller
        .query_one("SELECT current_user::text", &[])
        .and_then(|row| row.try_get::<_, String>(0))
        .expect("Writer setup error restored login identity");
    assert_eq!(error_restored_role, "lattice_migrator_login");

    let mut probe = Client::connect(migrator_url, NoTls).expect("Writer setup gate probe");
    let mut transaction = probe
        .transaction()
        .expect("Writer setup gate probe transaction");
    transaction
        .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog;")
        .expect("Writer setup gate probe role");
    let acquired = transaction
        .query_one(
            "SELECT pg_catalog.pg_try_advisory_lock($1)",
            &[&TASK076_GLOBAL_LOCK],
        )
        .and_then(|row| row.try_get::<_, bool>(0))
        .expect("Writer setup gate probe acquisition");
    assert!(
        acquired,
        "Writer setup gate must not remain held after error"
    );
    let unlocked = transaction
        .query_one(
            "SELECT pg_catalog.pg_advisory_unlock($1)",
            &[&TASK076_GLOBAL_LOCK],
        )
        .and_then(|row| row.try_get::<_, bool>(0))
        .expect("Writer setup gate probe release");
    assert!(
        unlocked,
        "Writer setup gate probe must release its own lock"
    );
    transaction
        .commit()
        .expect("Writer setup gate probe commit");
}

fn run_task076_fresh_install(migrator: &mut Client, migrator_url: &str, target: &ExtensionTarget) {
    let namespace_absent: bool = migrator
        .query_one(
            "SELECT pg_catalog.to_regnamespace('writer_lease') IS NULL",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .expect("TASK-076 fresh namespace precondition");
    assert!(namespace_absent, "fresh Writer namespace must be absent");
    let outcomes = task076_concurrent_apply(migrator, migrator_url, target, false);
    assert_eq!(
        outcomes
            .into_iter()
            .filter(|outcome| *outcome == ExtensionApplyOutcome::Installed)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .into_iter()
            .filter(|outcome| *outcome == ExtensionApplyOutcome::AlreadyCurrent)
            .count(),
        1
    );
    println!("TASK076_WRITER_FRESH_INSTALLED_PASS");
    println!("TASK076_WRITER_FRESH_CONCURRENT_PASS");
    assert_task076_login_apply_boundary(migrator_url, target);
    println!("TASK076_WRITER_LOGIN_APPLY_BOUNDARY_PASS");
    let installed = task076_fresh_profile_evidence(migrator, target);
    assert_eq!(
        apply_extension(migrator, target).expect("reapply exact fresh Writer v2 current"),
        ExtensionApplyOutcome::AlreadyCurrent
    );
    let reapplied = task076_fresh_profile_evidence(migrator, target);
    assert_eq!(
        reapplied, installed,
        "fresh no-op must preserve exact profile"
    );
    println!("TASK076_WRITER_FRESH_ALREADY_CURRENT_PASS");
    println!(
        "TASK076_WRITER_FRESH_DATABASE_UUID={}",
        installed.database_uuid
    );
    println!(
        "TASK076_WRITER_FRESH_PROFILE_SHA256={}",
        installed.fingerprint
    );
    println!("TASK076_WRITER_FRESH_INSTALL_PASS");
}

fn run_task076_fresh_restart(migrator: &mut Client, target: &ExtensionTarget) {
    let restarted = task076_fresh_profile_evidence(migrator, target);
    assert_eq!(
        apply_extension(migrator, target).expect("reapply restarted fresh Writer v2 current"),
        ExtensionApplyOutcome::AlreadyCurrent
    );
    let reapplied = task076_fresh_profile_evidence(migrator, target);
    assert_eq!(
        reapplied, restarted,
        "fresh restart no-op must preserve exact profile"
    );
    println!("TASK076_WRITER_FRESH_RESTART_ALREADY_CURRENT_PASS");
    println!(
        "TASK076_WRITER_FRESH_RESTART_DATABASE_UUID={}",
        restarted.database_uuid
    );
    println!(
        "TASK076_WRITER_FRESH_RESTART_PROFILE_SHA256={}",
        restarted.fingerprint
    );
    println!("TASK076_WRITER_FRESH_RESTART_PASS");
}

fn run_task076_bridge(
    migrator: &mut Client,
    migrator_url: &str,
    target: &ExtensionTarget,
    run_id: &str,
) {
    println!("TASK076_WRITER_BRIDGE_PRE_PROFILE_ENTER");
    assert_task076_v1_profile(migrator, target);
    println!("TASK076_WRITER_BRIDGE_PRE_PROFILE_PASS");
    println!("TASK076_WRITER_BRIDGE_HISTORY_LOAD_ENTER");
    let project_id =
        ProjectId::new(format!("task076-{run_id}")).expect("TASK-076 fixed project ID");
    let suspect_project_id = ProjectId::new(format!("task076-suspect-{run_id}"))
        .expect("TASK-076 legal SUSPECT marker project");
    let before = task076_history_evidence(migrator, &project_id);
    let suspect_before = task076_history_evidence(migrator, &suspect_project_id);
    assert_eq!(
        (before.fencing_high_water, before.command_high_water),
        (1, 2)
    );
    assert_eq!(
        (
            suspect_before.fencing_high_water,
            suspect_before.command_high_water,
            suspect_before.transition_high_water
        ),
        (1, 3, 3)
    );
    println!("TASK076_WRITER_BRIDGE_HISTORY_LOAD_PASS");
    let outcomes = task076_concurrent_apply(migrator, migrator_url, target, true);
    println!("TASK076_WRITER_BRIDGE_OUTCOMES_ENTER");
    assert_eq!(outcomes, [ExtensionApplyOutcome::Bridged; 2]);
    println!("TASK076_WRITER_BRIDGE_OUTCOMES_PASS");
    println!("TASK076_WRITER_BRIDGE_POST_PROFILE_ENTER");
    assert_task076_phase_profile(migrator, "bridge");
    println!("TASK076_WRITER_BRIDGE_POST_PROFILE_PASS");
    println!("TASK076_WRITER_BRIDGE_HISTORY_COMPARE_ENTER");
    let after = task076_history_evidence(migrator, &project_id);
    let suspect_after = task076_history_evidence(migrator, &suspect_project_id);
    assert_eq!(
        after, before,
        "Writer bridge must preserve exact v1 history"
    );
    assert_eq!(
        suspect_after, suspect_before,
        "Writer bridge must preserve exact legal SUSPECT/release history"
    );
    println!("TASK076_WRITER_BRIDGE_HISTORY_COMPARE_PASS");
    println!("TASK076_WRITER_BRIDGE_CONCURRENT_PASS");
    println!("TASK076_WRITER_BRIDGE_SEQUENTIAL_NOOP_ENTER");
    assert_eq!(
        apply_extension(migrator, target).expect("reapply exact Writer v2 bridge"),
        ExtensionApplyOutcome::Bridged
    );
    assert_task076_phase_profile(migrator, "bridge");
    let replay = task076_history_evidence(migrator, &project_id);
    let suspect_replay = task076_history_evidence(migrator, &suspect_project_id);
    assert_eq!(
        replay, after,
        "bridge replay must preserve exact released history"
    );
    assert_eq!(
        suspect_replay, suspect_after,
        "bridge replay must preserve exact legal SUSPECT/release history"
    );
    println!("TASK076_WRITER_BRIDGE_SEQUENTIAL_NOOP_PASS");
    println!("TASK076_WRITER_BRIDGE_REPLAY_PASS");
    println!("TASK076_WRITER_BRIDGE_SHA256={}", after.digest);
    println!("TASK076_WRITER_BRIDGE_PASS");
}

fn run_task076_activate(
    migrator: &mut Client,
    migrator_url: &str,
    target: &ExtensionTarget,
    run_id: &str,
) {
    let project_id =
        ProjectId::new(format!("task076-{run_id}")).expect("TASK-076 fixed project ID");
    let suspect_project_id = ProjectId::new(format!("task076-suspect-{run_id}"))
        .expect("TASK-076 legal SUSPECT marker project");
    let before = task076_history_evidence(migrator, &project_id);
    let suspect_before = task076_history_evidence(migrator, &suspect_project_id);
    assert_task076_phase_profile(migrator, "bridge");
    let outcomes = task076_concurrent_apply(migrator, migrator_url, target, false);
    assert_eq!(
        outcomes
            .into_iter()
            .filter(|outcome| *outcome == ExtensionApplyOutcome::Activated)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .into_iter()
            .filter(|outcome| *outcome == ExtensionApplyOutcome::AlreadyCurrent)
            .count(),
        1
    );
    verify_extension(migrator, target).expect("exact Writer v2 current profile");
    assert_task076_phase_profile(migrator, "activate");
    let after = task076_history_evidence(migrator, &project_id);
    let suspect_after = task076_history_evidence(migrator, &suspect_project_id);
    assert_eq!(
        after, before,
        "Writer activation must preserve exact released history"
    );
    assert_eq!(
        suspect_after, suspect_before,
        "Writer activation must preserve exact legal SUSPECT/release history"
    );
    println!("TASK076_WRITER_ACTIVATE_CONCURRENT_PASS");
    assert_eq!(
        apply_extension(migrator, target).expect("reapply exact Writer v2 current"),
        ExtensionApplyOutcome::AlreadyCurrent
    );
    verify_extension(migrator, target).expect("exact replayed Writer v2 current profile");
    assert_task076_phase_profile(migrator, "activate");
    let replay = task076_history_evidence(migrator, &project_id);
    let suspect_replay = task076_history_evidence(migrator, &suspect_project_id);
    assert_eq!(
        replay, after,
        "activation replay must preserve exact released history"
    );
    assert_eq!(
        suspect_replay, suspect_after,
        "activation replay must preserve exact legal SUSPECT/release history"
    );
    println!("TASK076_WRITER_ACTIVATE_REPLAY_PASS");
    println!("TASK076_WRITER_ACTIVATE_PASS");
}

fn run_task076_runtime(
    migrator: &mut Client,
    runtime_url: &str,
    target: ExtensionTarget,
    store_authority: &StoreAuthorityHead,
    run_id: &str,
) {
    let project_id = ProjectId::new(format!("task076-{run_id}"))
        .expect("TASK-076 fixed marker-owned project ID");
    let baseline = task076_history_evidence(migrator, &project_id);
    assert_eq!(baseline.fencing_high_water, 1);
    assert_eq!(baseline.command_high_water, 2);
    assert_eq!(baseline.transition_high_water, 2);
    let runtime = Client::connect(runtime_url, NoTls).expect("TASK-076 current runtime");
    let mut repository = PostgresWriterLease::new(runtime, target, store_authority, 600)
        .expect("TASK-076 current adapter");
    let replayed = repository
        .inspect_project(&project_id)
        .expect("TASK-076 v2 replay of v1 history")
        .expect("TASK-076 retained v1 source history");
    assert!(replayed.current_authority().is_none());
    assert_eq!(replayed.fencing_high_water(), 1);
    assert_eq!(replayed.transition_high_water(), 2);
    assert_eq!(replayed.command_high_water(), 2);

    let acquire_retry = repository
        .execute(task076_acquire(&project_id, run_id, "source-acquire"))
        .expect("TASK-076 cross-version exact acquire retry");
    let retry_head = acquire_retry
        .after
        .clone()
        .expect("TASK-076 retained source acquire authority");
    repository
        .execute(task076_release(
            &project_id,
            run_id,
            "source-release",
            retry_head,
        ))
        .expect("TASK-076 cross-version exact release retry");
    let retry = repository
        .inspect_project(&project_id)
        .expect("TASK-076 exact retry replay")
        .expect("TASK-076 exact retry history");
    assert_eq!(retry, replayed);
    assert_eq!(task076_history_evidence(migrator, &project_id), baseline);

    let next = repository
        .execute(task076_acquire(&project_id, run_id, "runtime-acquire"))
        .expect("TASK-076 v2 runtime reacquire");
    let next_head = next.after.clone().expect("TASK-076 v2 runtime authority");
    assert_eq!(next_head.identity().fencing_token().get(), 2);
    repository
        .execute(task076_release(
            &project_id,
            run_id,
            "runtime-release",
            next_head,
        ))
        .expect("TASK-076 v2 runtime release");
    let final_evidence = repository
        .inspect_project(&project_id)
        .expect("TASK-076 v2 final replay")
        .expect("TASK-076 v2 final history");
    assert!(final_evidence.current_authority().is_none());
    assert_eq!(final_evidence.fencing_high_water(), 2);
    assert_eq!(final_evidence.transition_high_water(), 4);
    assert_eq!(final_evidence.command_high_water(), 4);
    drop(repository);
    let final_history = task076_history_evidence(migrator, &project_id);
    assert_eq!(final_history.row_version, 4);
    assert_eq!(final_history.fencing_high_water, 2);
    assert_eq!(final_history.lease_revision, 4);
    assert_eq!(final_history.command_high_water, 4);
    assert_eq!(final_history.transition_high_water, 4);
    println!("TASK076_WRITER_SOURCE_SHA256={}", baseline.digest);
    println!("TASK076_WRITER_RUNTIME_SHA256={}", final_history.digest);
    println!("TASK076_WRITER_FENCING_HIGH_WATER=2");
    println!("TASK076_WRITER_COMMAND_HIGH_WATER=4");
    println!("TASK076_WRITER_TRANSITION_HIGH_WATER=4");
    println!("TASK076_WRITER_RUNTIME_PASS");
}

fn run_task076_restart(
    migrator: &mut Client,
    runtime_url: &str,
    target: ExtensionTarget,
    store_authority: &StoreAuthorityHead,
    run_id: &str,
) {
    let project_id = ProjectId::new(format!("task076-{run_id}"))
        .expect("TASK-076 fixed marker-owned project ID");
    let before = task076_history_evidence(migrator, &project_id);
    assert_eq!(before.fencing_high_water, 2);
    assert_eq!(before.command_high_water, 4);
    assert_eq!(before.transition_high_water, 4);
    let runtime = Client::connect(runtime_url, NoTls).expect("TASK-076 restarted runtime");
    let mut repository = PostgresWriterLease::new(runtime, target, store_authority, 600)
        .expect("TASK-076 restarted adapter");
    let replayed = repository
        .inspect_project(&project_id)
        .expect("TASK-076 physical restart replay")
        .expect("TASK-076 retained final history");
    assert!(replayed.current_authority().is_none());
    assert_eq!(replayed.fencing_high_water(), 2);
    assert_eq!(replayed.command_high_water(), 4);
    assert_eq!(replayed.transition_high_water(), 4);
    let acquire_retry = repository
        .execute(task076_acquire(&project_id, run_id, "runtime-acquire"))
        .expect("TASK-076 post-restart exact acquire retry");
    let retry_head = acquire_retry.after.expect("retained runtime authority");
    repository
        .execute(task076_release(
            &project_id,
            run_id,
            "runtime-release",
            retry_head,
        ))
        .expect("TASK-076 post-restart exact release retry");
    drop(repository);
    let after = task076_history_evidence(migrator, &project_id);
    assert_eq!(after, before, "physical restart retry must be byte-exact");
    println!("TASK076_WRITER_RESTART_SHA256={}", after.digest);
    println!("TASK076_WRITER_FENCING_HIGH_WATER=2");
    println!("TASK076_WRITER_COMMAND_HIGH_WATER=4");
    println!("TASK076_WRITER_TRANSITION_HIGH_WATER=4");
    println!("TASK076_WRITER_RESTART_PASS");
}

fn acquire_intent(
    project_id: &ProjectId,
    command_id: &str,
    lease_id: &str,
    attempt_id: &str,
) -> WriterLeaseRepositoryCommand {
    WriterLeaseRepositoryCommand::Acquire(WriterLeaseAcquireRequest {
        command_id: command_id.to_owned(),
        expected_head: None,
        project_id: project_id.clone(),
        project_snapshot_id: ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        task_id: TaskId::new("task038-live").expect("task"),
        task_revision: "1".to_owned(),
        task_spec_digest: digest('d'),
        attempt_id: AttemptId::new(attempt_id).expect("attempt"),
        lease_id: lease_id.to_owned(),
        lease_holder_id: "implementer-task038".to_owned(),
        worktree_id: "worktree-task038-live".to_owned(),
        holder_process_id: HolderProcessId::new(std::process::id().into()).expect("pid"),
        holder_process_start_identity: digest('e'),
    })
}

#[test]
#[allow(clippy::too_many_lines)]
fn live_postgres_acquire_restarts_and_replays_authority_when_provisioned() {
    let Ok(migrator_url) = std::env::var("LATTICE_WRITER_LEASE_MIGRATOR_URL") else {
        eprintln!("SKIP: LATTICE_WRITER_LEASE_MIGRATOR_URL is not configured");
        return;
    };
    let runtime_url = std::env::var("LATTICE_WRITER_LEASE_RUNTIME_URL")
        .expect("runtime URL is required when the live test is enabled");
    let database_name = std::env::var("LATTICE_WRITER_LEASE_DATABASE_NAME")
        .expect("database name is required when the live test is enabled");
    let target = ExtensionTarget::new(
        database_name,
        digest_env("LATTICE_WRITER_LEASE_DATABASE_IDENTITY_SHA256"),
        digest_env("LATTICE_WRITER_LEASE_GLOBAL_MANIFEST_SHA256"),
        digest_env("LATTICE_WRITER_LEASE_MEMORY_MANIFEST_SHA256"),
    )
    .expect("target");
    let mut migrator = Client::connect(&migrator_url, NoTls).expect("migrator connection");
    let task076_phase = std::env::var("LATTICE_TASK076_WRITER_PHASE").ok();
    if task076_phase.is_some() {
        migrator
            .batch_execute(
                "SET ROLE lattice_migrator; SET search_path=pg_catalog; SET row_security=on;",
            )
            .expect("TASK-076 phased owner session role");
    }
    match task076_phase.as_deref() {
        Some("fresh_install") => {
            run_task076_fresh_install(&mut migrator, &migrator_url, &target);
            return;
        }
        Some("fresh_restart") => {
            run_task076_fresh_restart(&mut migrator, &target);
            return;
        }
        Some("source_install") => {
            run_task076_source_install(&mut migrator, &target);
            return;
        }
        Some("source_seed") => {
            let store_authority = authority_env();
            let run_id = task076_run_id();
            run_task076_source_seed(
                &mut migrator,
                &runtime_url,
                &target,
                &store_authority,
                &run_id,
            );
            return;
        }
        Some("bridge") => {
            let run_id = task076_run_id();
            run_task076_bridge(&mut migrator, &migrator_url, &target, &run_id);
            return;
        }
        Some("activate") => {
            let run_id = task076_run_id();
            run_task076_activate(&mut migrator, &migrator_url, &target, &run_id);
            return;
        }
        Some("runtime") => {
            verify_extension(&mut migrator, &target).expect("exact Writer v2 runtime profile");
            assert_task076_phase_profile(&mut migrator, "runtime");
            let store_authority = authority_env();
            let run_id = task076_run_id();
            run_task076_runtime(
                &mut migrator,
                &runtime_url,
                target,
                &store_authority,
                &run_id,
            );
            return;
        }
        Some("restart") => {
            verify_extension(&mut migrator, &target).expect("exact restarted Writer v2 profile");
            assert_task076_phase_profile(&mut migrator, "restart");
            let store_authority = authority_env();
            let run_id = task076_run_id();
            run_task076_restart(
                &mut migrator,
                &runtime_url,
                target,
                &store_authority,
                &run_id,
            );
            return;
        }
        None => {}
        Some(_) => panic!(
            "LATTICE_TASK076_WRITER_PHASE must be fresh_install, fresh_restart, \
             source_install, source_seed, bridge, activate, runtime, or restart"
        ),
    }
    let store_authority = authority_env();

    assert!(matches!(
        apply_extension(&mut migrator, &target).expect("apply extension"),
        ExtensionApplyOutcome::Installed
            | ExtensionApplyOutcome::Activated
            | ExtensionApplyOutcome::AlreadyCurrent
    ));
    verify_extension(&mut migrator, &target).expect("exact extension profile");

    let runtime = Client::connect(&runtime_url, NoTls).expect("runtime connection");
    let mut repository = PostgresWriterLease::new(runtime, target.clone(), &store_authority, 600)
        .expect("live repository");
    let run_suffix = format!(
        "{:x}-{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    );
    let project_id =
        ProjectId::new(format!("task038-writer-lease-{run_suffix}")).expect("unique project");
    assert_eq!(
        repository
            .inspect_project(&project_id)
            .expect("absent project inspection"),
        None
    );
    assert_eq!(
        repository
            .inspect_project(&project_id)
            .expect("repeated absent project inspection must remain read-only"),
        None
    );
    let first_acquire = WriterLeaseRepositoryCommand::Acquire(WriterLeaseAcquireRequest {
        command_id: "task038-live-acquire".to_owned(),
        expected_head: None,
        project_id: project_id.clone(),
        project_snapshot_id: ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        task_id: TaskId::new("task038-live").expect("task"),
        task_revision: "1".to_owned(),
        task_spec_digest: digest('d'),
        attempt_id: AttemptId::new("attempt-1").expect("attempt"),
        lease_id: "lease-task038-live".to_owned(),
        lease_holder_id: "implementer-task038".to_owned(),
        worktree_id: "worktree-task038-live".to_owned(),
        holder_process_id: HolderProcessId::new(std::process::id().into()).expect("pid"),
        holder_process_start_identity: digest('e'),
    });
    let receipt = repository
        .execute(first_acquire.clone())
        .expect("acquire terminal receipt");
    assert_eq!(receipt.ordinal, 1);
    assert_eq!(receipt.outcome, CommandOutcome::Applied);
    assert_eq!(
        repository.execute(first_acquire).expect("exact replay"),
        receipt
    );
    let old_authority = repository
        .current_authority(&project_id)
        .expect("current")
        .expect("active");
    let old_head = old_authority.independent_head().clone();
    repository
        .execute(WriterLeaseRepositoryCommand::Release(
            WriterLeaseReleaseRequest {
                command_id: "task038-live-release".to_owned(),
                project_id: project_id.clone(),
                expected_head: old_head.clone(),
            },
        ))
        .expect("release");
    let released = repository
        .inspect_project(&project_id)
        .expect("released project replay")
        .expect("released aggregate remains durable");
    assert_eq!(released.project_id(), &project_id);
    assert!(released.current_authority().is_none());
    assert_eq!(released.fencing_high_water(), 1);
    assert_eq!(released.transition_high_water(), 2);
    assert_eq!(released.command_high_water(), 2);
    assert_eq!(
        repository
            .inspect_project(&project_id)
            .expect("repeated released project replay")
            .expect("released aggregate remains durable"),
        released
    );
    let second = repository
        .execute(WriterLeaseRepositoryCommand::Acquire(
            WriterLeaseAcquireRequest {
                command_id: "task038-live-reacquire".to_owned(),
                expected_head: None,
                project_id: project_id.clone(),
                project_snapshot_id: ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
                task_id: TaskId::new("task038-live").expect("task"),
                task_revision: "1".to_owned(),
                task_spec_digest: digest('d'),
                attempt_id: AttemptId::new("attempt-2").expect("attempt"),
                lease_id: "lease-task038-live-2".to_owned(),
                lease_holder_id: "implementer-task038".to_owned(),
                worktree_id: "worktree-task038-live".to_owned(),
                holder_process_id: HolderProcessId::new(std::process::id().into()).expect("pid"),
                holder_process_start_identity: digest('e'),
            },
        ))
        .expect("reacquire");
    assert_eq!(
        second
            .after
            .expect("new head")
            .identity()
            .fencing_token()
            .get(),
        2
    );
    assert_eq!(
        repository
            .assert_current(&old_head)
            .expect_err("stale fence")
            .kind(),
        WriterLeaseRepositoryErrorKind::AuthorityMismatch
    );
    drop(repository);

    let runtime = Client::connect(&runtime_url, NoTls).expect("fresh runtime connection");
    let mut restarted = PostgresWriterLease::new(runtime, target.clone(), &store_authority, 600)
        .expect("restarted repository");
    let current = restarted
        .current_authority(&project_id)
        .expect("durable current lookup")
        .expect("current authority");
    restarted
        .assert_current(current.independent_head())
        .expect("fresh process exact current assertion");
    assert_eq!(current.receipt().identity().fencing_token().get(), 2);
    let identity = current.independent_head().identity();
    let mut substitution_client =
        Client::connect(&runtime_url, NoTls).expect("substitution regression connection");
    let mut substitution = substitution_client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("substitution transaction");
    substitution
        .batch_execute(
            "SET LOCAL ROLE lattice_runtime; SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; SET LOCAL synchronous_commit = on;",
        )
        .expect("runtime transaction boundary");
    let task_spec_digest = digest_bytes(identity.task_spec_digest());
    let process_start_digest = digest_bytes(identity.holder_process_start_identity());
    let receipt_digest = digest_bytes(current.independent_head().receipt_digest());
    let error = substitution
        .query_one(
            "SELECT writer_lease.writer_lease_assert_current_v1(\
                $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
            &[
                &project_id.as_str(),
                &identity.project_snapshot_id().as_str(),
                &"task-substitution",
                &identity.task_revision(),
                &task_spec_digest,
                &identity.attempt_id().as_str(),
                &identity.lease_id(),
                &identity.lease_holder_id(),
                &identity.worktree_id(),
                &i64::try_from(identity.holder_process_id().get()).expect("process ID"),
                &process_start_digest,
                &identity.daemon_instance_id(),
                &i64::try_from(identity.daemon_epoch().get()).expect("daemon epoch"),
                &i64::try_from(identity.fencing_token().get()).expect("fence"),
                &receipt_digest,
            ],
        )
        .expect_err("same digest and fence cannot substitute task binding");
    assert_eq!(
        error.code().map(postgres::error::SqlState::code),
        Some("LWL05")
    );
    drop(substitution);
    drop(substitution_client);

    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             ALTER FUNCTION writer_lease.writer_lease_assert_current_v1(\
               text,text,text,text,bytea,text,text,text,text,bigint,bytea,text,bigint,bigint,bytea) \
               SET statement_timeout = '31s'; RESET ROLE;",
        )
        .expect("commit function proconfig drift");
    assert_profile_rejects(&mut migrator, &target);
    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             ALTER FUNCTION writer_lease.writer_lease_assert_current_v1(\
               text,text,text,text,bytea,text,text,text,text,bigint,bytea,text,bigint,bigint,bytea) \
               SET statement_timeout = '30s'; RESET ROLE;",
        )
        .expect("restore function proconfig");
    assert_profile_restored(&mut migrator, &target);

    let original_assert_definition: String = migrator
        .query_one(
            "SELECT pg_catalog.pg_get_functiondef(p.oid) FROM pg_catalog.pg_proc p \
             JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
             WHERE n.nspname='writer_lease' AND p.proname='writer_lease_assert_current_v1'",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .expect("original assertion definition");
    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             CREATE OR REPLACE FUNCTION writer_lease.writer_lease_assert_current_v1(\
               p_project_id text,p_project_snapshot_id text,p_task_id text,p_task_revision text,\
               p_task_spec_digest bytea,p_attempt_id text,p_lease_id text,p_lease_holder_id text,\
               p_worktree_id text,p_holder_process_id bigint,p_holder_process_start_identity bytea,\
               p_daemon_instance_id text,p_daemon_epoch bigint,p_fencing_token bigint,\
               p_receipt_digest bytea) \
             RETURNS boolean LANGUAGE plpgsql STABLE PARALLEL SAFE SECURITY DEFINER \
             SET search_path=pg_catalog SET row_security=on SET lock_timeout='5s' \
             SET statement_timeout='30s' AS $catalog_drift$ BEGIN RETURN true; END; \
             $catalog_drift$; RESET ROLE;",
        )
        .expect("commit function body drift");
    assert_profile_rejects(&mut migrator, &target);
    migrator
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; {original_assert_definition} RESET ROLE;"
        ))
        .expect("restore assertion body");
    assert_profile_restored(&mut migrator, &target);

    for (drift, restore) in [
        (
            "GRANT USAGE ON SCHEMA writer_lease TO lattice_readonly",
            "REVOKE USAGE ON SCHEMA writer_lease FROM lattice_readonly",
        ),
        (
            "GRANT SELECT ON writer_lease.writer_lease_heads TO lattice_readonly",
            "REVOKE SELECT ON writer_lease.writer_lease_heads FROM lattice_readonly",
        ),
        (
            "GRANT SELECT (project_id) ON writer_lease.writer_lease_heads TO lattice_readonly",
            "REVOKE SELECT (project_id) ON writer_lease.writer_lease_heads FROM lattice_readonly",
        ),
        (
            "GRANT EXECUTE ON FUNCTION writer_lease.writer_lease_load_current_v1(text) TO lattice_readonly",
            "REVOKE EXECUTE ON FUNCTION writer_lease.writer_lease_load_current_v1(text) FROM lattice_readonly",
        ),
    ] {
        migrator
            .batch_execute(&format!("SET ROLE lattice_migrator; {drift}; RESET ROLE;"))
            .expect("commit ACL drift");
        assert_profile_rejects(&mut migrator, &target);
        migrator
            .batch_execute(&format!(
                "SET ROLE lattice_migrator; {restore}; RESET ROLE;"
            ))
            .expect("restore ACL");
        assert_profile_restored(&mut migrator, &target);
    }

    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             ALTER TABLE writer_lease.writer_lease_heads ALTER COLUMN updated_at \
             SET DEFAULT pg_catalog.transaction_timestamp(); RESET ROLE;",
        )
        .expect("commit column default drift");
    assert_profile_rejects(&mut migrator, &target);
    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             ALTER TABLE writer_lease.writer_lease_heads ALTER COLUMN updated_at \
             SET DEFAULT pg_catalog.clock_timestamp(); RESET ROLE;",
        )
        .expect("restore column default");
    assert_profile_restored(&mut migrator, &target);

    let original_versions_constraint: String = migrator
        .query_one(
            "SELECT pg_catalog.pg_get_constraintdef(con.oid,false) \
             FROM pg_catalog.pg_constraint con \
             JOIN pg_catalog.pg_namespace n ON n.oid=con.connamespace \
             WHERE n.nspname='writer_lease' AND con.conname='writer_lease_heads_versions'",
            &[],
        )
        .and_then(|row| row.try_get(0))
        .expect("original versions constraint");
    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             ALTER TABLE writer_lease.writer_lease_heads \
             DROP CONSTRAINT writer_lease_heads_versions; \
             ALTER TABLE writer_lease.writer_lease_heads \
             ADD CONSTRAINT writer_lease_heads_versions CHECK (row_version >= 0); \
             RESET ROLE;",
        )
        .expect("commit same-name weak constraint");
    assert_profile_rejects(&mut migrator, &target);
    migrator
        .batch_execute(&format!(
            "SET ROLE lattice_migrator; ALTER TABLE writer_lease.writer_lease_heads \
             DROP CONSTRAINT writer_lease_heads_versions; \
             ALTER TABLE writer_lease.writer_lease_heads ADD CONSTRAINT \
             writer_lease_heads_versions {original_versions_constraint}; RESET ROLE;"
        ))
        .expect("restore exact constraint");
    assert_profile_restored(&mut migrator, &target);

    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             ALTER INDEX writer_lease.writer_lease_heads_pkey SET (fillfactor=70); RESET ROLE;",
        )
        .expect("commit index option drift");
    assert_profile_rejects(&mut migrator, &target);
    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             ALTER INDEX writer_lease.writer_lease_heads_pkey RESET (fillfactor); RESET ROLE;",
        )
        .expect("restore index option");
    assert_profile_restored(&mut migrator, &target);

    let concurrent_project =
        ProjectId::new(format!("task038-concurrent-{run_suffix}")).expect("concurrent project");
    let vacant = VerifiedWriterLeaseAggregate::vacant(concurrent_project.clone());
    let vacant_bytes = vacant
        .export_canonical_bytes()
        .expect("vacant canonical bytes");
    let vacant_sha256 = Sha256::digest(&vacant_bytes).to_vec();
    let vacant_snapshot_digest = digest_bytes(
        vacant
            .checkpoint()
            .expect("vacant checkpoint")
            .snapshot_digest(),
    );
    let mut seeder = Client::connect(&runtime_url, NoTls).expect("vacant seeder connection");
    let mut seed_transaction = seeder
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("vacant seed transaction");
    seed_transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_runtime; SET LOCAL search_path=pg_catalog; \
             SET LOCAL row_security=on; SET LOCAL synchronous_commit=on;",
        )
        .expect("vacant runtime boundary");
    seed_transaction
        .query_one(
            "SELECT * FROM writer_lease.writer_lease_load_for_update_v2($1,$2,$3,$4,$5)",
            &[
                &concurrent_project.as_str(),
                &vacant_bytes,
                &vacant_sha256,
                &vacant_snapshot_digest,
                &"concurrent-seed",
            ],
        )
        .expect("seed vacant aggregate through fixed boundary");
    seed_transaction.commit().expect("vacant seed commit");
    drop(seeder);

    let mut blocker = migrator
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .expect("concurrency blocker transaction");
    blocker
        .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog;")
        .expect("blocker boundary");
    blocker
        .query_one(
            "SELECT h.project_id::text FROM ONLY writer_lease.writer_lease_heads h \
             WHERE h.project_id=$1 FOR UPDATE OF h",
            &[&concurrent_project.as_str()],
        )
        .expect("hold exact project row lock");

    let mut contender_client_a =
        Client::connect(&runtime_url, NoTls).expect("contender A connection");
    let contender_pid_a: i32 = contender_client_a
        .query_one("SELECT pg_catalog.pg_backend_pid()", &[])
        .and_then(|row| row.try_get(0))
        .expect("contender A pid");
    let contender_a =
        PostgresWriterLease::new(contender_client_a, target.clone(), &store_authority, 600)
            .expect("contender A repository");
    let mut contender_client_b =
        Client::connect(&runtime_url, NoTls).expect("contender B connection");
    let contender_pid_b: i32 = contender_client_b
        .query_one("SELECT pg_catalog.pg_backend_pid()", &[])
        .and_then(|row| row.try_get(0))
        .expect("contender B pid");
    let contender_b =
        PostgresWriterLease::new(contender_client_b, target.clone(), &store_authority, 600)
            .expect("contender B repository");
    let command_a = acquire_intent(
        &concurrent_project,
        "concurrent-acquire-a",
        "concurrent-lease-a",
        "concurrent-attempt-a",
    );
    let command_b = acquire_intent(
        &concurrent_project,
        "concurrent-acquire-b",
        "concurrent-lease-b",
        "concurrent-attempt-b",
    );
    let barrier = Arc::new(Barrier::new(3));
    let barrier_a = Arc::clone(&barrier);
    let first_thread_command = command_a.clone();
    let contender_thread_a = std::thread::spawn(move || {
        let mut repository = contender_a;
        barrier_a.wait();
        repository.execute(first_thread_command)
    });
    let barrier_b = Arc::clone(&barrier);
    let second_thread_command = command_b.clone();
    let contender_thread_b = std::thread::spawn(move || {
        let mut repository = contender_b;
        barrier_b.wait();
        repository.execute(second_thread_command)
    });
    barrier.wait();

    let mut supervisor =
        Client::connect(&migrator_url, NoTls).expect("concurrency supervisor connection");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let blocked_a: bool = supervisor
            .query_one(
                "SELECT pg_catalog.cardinality(pg_catalog.pg_blocking_pids($1)) > 0",
                &[&contender_pid_a],
            )
            .and_then(|row| row.try_get(0))
            .expect("observe contender A blocking");
        let blocked_b: bool = supervisor
            .query_one(
                "SELECT pg_catalog.cardinality(pg_catalog.pg_blocking_pids($1)) > 0",
                &[&contender_pid_b],
            )
            .and_then(|row| row.try_get(0))
            .expect("observe contender B blocking");
        if blocked_a && blocked_b {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "both contenders must reach the same locked project row"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    blocker.commit().expect("release concurrency blocker");
    let receipt_a = contender_thread_a
        .join()
        .expect("contender A thread")
        .expect("contender A terminal receipt");
    let receipt_b = contender_thread_b
        .join()
        .expect("contender B thread")
        .expect("contender B terminal receipt");
    assert_eq!(
        [receipt_a.outcome, receipt_b.outcome]
            .into_iter()
            .filter(|outcome| *outcome == CommandOutcome::Applied)
            .count(),
        1
    );
    assert_eq!(
        [receipt_a.outcome, receipt_b.outcome]
            .into_iter()
            .filter(|outcome| {
                *outcome == CommandOutcome::Denied(LeaseDenial::WriterAlreadyHeld)
            })
            .count(),
        1
    );
    let mut concurrent_replay = PostgresWriterLease::new(
        Client::connect(&runtime_url, NoTls).expect("concurrent replay connection"),
        target.clone(),
        &store_authority,
        600,
    )
    .expect("concurrent replay repository");
    assert_eq!(
        concurrent_replay.execute(command_a).expect("retry A"),
        receipt_a
    );
    assert_eq!(
        concurrent_replay.execute(command_b).expect("retry B"),
        receipt_b
    );
    let concurrent_current = concurrent_replay
        .current_authority(&concurrent_project)
        .expect("concurrent physical replay")
        .expect("one current writer");
    assert_eq!(
        concurrent_current
            .receipt()
            .identity()
            .fencing_token()
            .get(),
        1
    );
    drop(concurrent_replay);
    let physical_counts: (i64, i64) = {
        let mut evidence = supervisor
            .transaction()
            .expect("concurrent evidence transaction");
        evidence
            .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog;")
            .expect("concurrent evidence boundary");
        let row = evidence
            .query_one(
                "SELECT \
                   (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_commands \
                     WHERE project_id=$1), \
                   (SELECT pg_catalog.count(*) FROM ONLY writer_lease.writer_lease_transitions \
                     WHERE project_id=$1)",
                &[&concurrent_project.as_str()],
            )
            .expect("concurrent physical counts");
        let counts = (
            row.try_get(0).expect("command count"),
            row.try_get(1).expect("transition count"),
        );
        evidence.commit().expect("concurrent evidence commit");
        counts
    };
    assert_eq!(physical_counts, (2, 1));

    let (original_receipt_bytes, original_transition_bytes) = {
        let mut evidence = migrator
            .transaction()
            .expect("physical evidence transaction");
        evidence
            .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog;")
            .expect("migrator evidence boundary");
        let receipt_bytes: Vec<u8> = evidence
            .query_one(
                "SELECT receipt_bytes FROM ONLY writer_lease.writer_lease_commands \
                 WHERE project_id=$1 AND ordinal=1",
                &[&project_id.as_str()],
            )
            .and_then(|row| row.try_get(0))
            .expect("physical receipt bytes");
        let transition_bytes: Vec<u8> = evidence
            .query_one(
                "SELECT transition_bytes FROM ONLY writer_lease.writer_lease_transitions \
                 WHERE project_id=$1 AND ordinal=1",
                &[&project_id.as_str()],
            )
            .and_then(|row| row.try_get(0))
            .expect("physical transition bytes");
        evidence.commit().expect("physical evidence commit");
        (receipt_bytes, transition_bytes)
    };
    {
        let mut corruption = migrator
            .transaction()
            .expect("receipt corruption transaction");
        corruption
            .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog;")
            .expect("migrator corruption boundary");
        corruption
            .execute(
                "UPDATE ONLY writer_lease.writer_lease_commands SET receipt_bytes=$2 \
                 WHERE project_id=$1 AND ordinal=1",
                &[&project_id.as_str(), &b"{}".as_slice()],
            )
            .expect("commit receipt corruption");
        corruption.commit().expect("receipt corruption commit");
    }
    assert_eq!(
        restarted
            .inspect_project(&project_id)
            .expect_err("snapshot replay must reject physical receipt divergence")
            .kind(),
        WriterLeaseRepositoryErrorKind::Corrupt
    );
    {
        let mut restore = migrator.transaction().expect("receipt restore transaction");
        restore
            .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog;")
            .expect("migrator restore boundary");
        restore
            .execute(
                "UPDATE ONLY writer_lease.writer_lease_commands SET receipt_bytes=$2 \
                 WHERE project_id=$1 AND ordinal=1",
                &[&project_id.as_str(), &original_receipt_bytes],
            )
            .expect("restore receipt bytes");
        restore.commit().expect("receipt restore commit");
    }
    restarted
        .current_authority(&project_id)
        .expect("receipt physical replay restored")
        .expect("current after receipt restore");
    {
        let mut corruption = migrator
            .transaction()
            .expect("transition corruption transaction");
        corruption
            .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog;")
            .expect("migrator corruption boundary");
        corruption
            .execute(
                "UPDATE ONLY writer_lease.writer_lease_transitions SET transition_bytes=$2 \
                 WHERE project_id=$1 AND ordinal=1",
                &[&project_id.as_str(), &b"{}".as_slice()],
            )
            .expect("commit transition corruption");
        corruption.commit().expect("transition corruption commit");
    }
    assert_eq!(
        restarted
            .inspect_project(&project_id)
            .expect_err("snapshot replay must reject physical transition divergence")
            .kind(),
        WriterLeaseRepositoryErrorKind::Corrupt
    );
    {
        let mut restore = migrator
            .transaction()
            .expect("transition restore transaction");
        restore
            .batch_execute("SET LOCAL ROLE lattice_migrator; SET LOCAL search_path=pg_catalog;")
            .expect("migrator restore boundary");
        restore
            .execute(
                "UPDATE ONLY writer_lease.writer_lease_transitions SET transition_bytes=$2 \
                 WHERE project_id=$1 AND ordinal=1",
                &[&project_id.as_str(), &original_transition_bytes],
            )
            .expect("restore transition bytes");
        restore.commit().expect("transition restore commit");
    }
    restarted
        .current_authority(&project_id)
        .expect("transition physical replay restored")
        .expect("current after transition restore");

    if let Ok(admin_url) = std::env::var("LATTICE_WRITER_LEASE_ADMIN_URL") {
        let fault_project =
            ProjectId::new(format!("task038-commit-fault-{run_suffix}")).expect("fault project");
        let fault_command = acquire_intent(
            &fault_project,
            "commit-unknown-acquire",
            "commit-unknown-lease",
            "commit-unknown-attempt",
        );
        migrator
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 CREATE FUNCTION writer_lease.writer_lease_test_commit_sleep_v1() \
                 RETURNS trigger LANGUAGE plpgsql VOLATILE PARALLEL UNSAFE SECURITY DEFINER \
                 SET search_path=pg_catalog SET row_security=on AS $commit_fault$ \
                 BEGIN PERFORM pg_catalog.pg_sleep(10); RETURN NEW; END; $commit_fault$; \
                 REVOKE ALL ON FUNCTION writer_lease.writer_lease_test_commit_sleep_v1() FROM PUBLIC; \
                 CREATE CONSTRAINT TRIGGER writer_lease_test_commit_sleep_v1 \
                 AFTER INSERT ON writer_lease.writer_lease_commands \
                 DEFERRABLE INITIALLY DEFERRED FOR EACH ROW \
                 EXECUTE FUNCTION writer_lease.writer_lease_test_commit_sleep_v1(); \
                 RESET ROLE;",
            )
            .expect("install disposable commit fault");
        assert_profile_rejects(&mut migrator, &target);
        let application_name = format!("writer_lease_commit_fault_{run_suffix}");
        let separator = if runtime_url.contains('?') { '&' } else { '?' };
        let fault_runtime_url =
            format!("{runtime_url}{separator}application_name={application_name}");
        let fault_client =
            Client::connect(&fault_runtime_url, NoTls).expect("commit fault runtime connection");
        let mut fault_repository =
            PostgresWriterLease::new(fault_client, target.clone(), &store_authority, 600)
                .expect("commit fault repository");
        let command_for_thread = fault_command.clone();
        let fault_thread = std::thread::spawn(move || fault_repository.execute(command_for_thread));
        let mut admin = Client::connect(&admin_url, NoTls).expect("commit fault admin connection");
        let fault_deadline = Instant::now() + Duration::from_secs(5);
        let fault_pid = loop {
            let row = admin
                .query_opt(
                    "SELECT pid FROM pg_catalog.pg_stat_activity \
                     WHERE application_name=$1 AND wait_event='PgSleep'",
                    &[&application_name],
                )
                .expect("observe commit fault backend");
            if let Some(row) = row {
                break row.try_get::<_, i32>(0).expect("fault backend pid");
            }
            assert!(
                Instant::now() < fault_deadline,
                "runtime commit must reach the deferred PostgreSQL trigger"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        let terminated: bool = admin
            .query_one("SELECT pg_catalog.pg_terminate_backend($1)", &[&fault_pid])
            .and_then(|row| row.try_get(0))
            .expect("terminate commit response path");
        assert!(terminated);
        let fault_error = fault_thread
            .join()
            .expect("commit fault thread")
            .expect_err("lost commit response must not be reported as success");
        assert_eq!(
            fault_error.kind(),
            WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown
        );
        migrator
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 DROP TRIGGER writer_lease_test_commit_sleep_v1 \
                   ON writer_lease.writer_lease_commands; \
                 DROP FUNCTION writer_lease.writer_lease_test_commit_sleep_v1(); \
                 RESET ROLE;",
            )
            .expect("remove disposable commit fault");
        assert_profile_restored(&mut migrator, &target);
        let mut reconciler = PostgresWriterLease::new(
            Client::connect(&runtime_url, NoTls).expect("commit reconciliation connection"),
            target.clone(),
            &store_authority,
            600,
        )
        .expect("commit reconciliation repository");
        let reconciliation_receipt = reconciler
            .execute(fault_command)
            .expect("safe exact-intent reconciliation after unknown commit outcome");
        assert_eq!(reconciliation_receipt.ordinal, 1);
        assert_eq!(reconciliation_receipt.outcome, CommandOutcome::Applied);
        assert_eq!(
            reconciler
                .current_authority(&fault_project)
                .expect("commit fault physical replay")
                .expect("reconciled authority")
                .receipt()
                .identity()
                .fencing_token()
                .get(),
            1
        );
    } else {
        eprintln!("SKIP: LATTICE_WRITER_LEASE_ADMIN_URL is not configured");
    }

    let current_head = current.independent_head().clone();
    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             UPDATE ONLY control.runtime_admission \
                SET daemon_instance_id = 'daemon-task038-replacement', \
                    daemon_epoch = 2, \
                    observation_digest = pg_catalog.decode(pg_catalog.repeat('ee', 32), 'hex') \
              WHERE singleton; \
             RESET ROLE;",
        )
        .expect("replace live daemon leadership");
    assert_eq!(
        restarted
            .execute(WriterLeaseRepositoryCommand::Heartbeat(
                WriterLeaseHeartbeatRequest {
                    command_id: format!("stale-heartbeat-{run_suffix}"),
                    project_id: project_id.clone(),
                    expected_head: current_head.clone(),
                },
            ))
            .expect_err("old adapter must not borrow replacement leadership")
            .kind(),
        WriterLeaseRepositoryErrorKind::AuthorityMismatch
    );

    let borrowed_runtime = Client::connect(&runtime_url, NoTls).expect("borrow attempt runtime");
    let Err(borrow_error) =
        PostgresWriterLease::new(borrowed_runtime, target.clone(), &store_authority, 600)
    else {
        panic!("old process-start authority cannot bind replacement daemon");
    };
    assert_eq!(
        borrow_error.kind(),
        WriterLeaseRepositoryErrorKind::AuthorityMismatch
    );

    let replacement_authority = authority(
        "daemon-task038-replacement".to_owned(),
        2,
        2,
        digest('e'),
        digest('f'),
    );
    let replacement_runtime = Client::connect(&runtime_url, NoTls).expect("replacement runtime");
    let mut replacement = PostgresWriterLease::new(
        replacement_runtime,
        target.clone(),
        &replacement_authority,
        600,
    )
    .expect("replacement daemon binding");
    assert!(
        replacement
            .current_authority(&project_id)
            .expect("lookup")
            .is_some()
    );
    assert_eq!(
        replacement
            .assert_current(&current_head)
            .expect_err("replacement cannot assert prior-daemon authority")
            .kind(),
        WriterLeaseRepositoryErrorKind::AuthorityMismatch
    );
    drop(replacement);
    drop(restarted);

    migrator
        .batch_execute(
            "SET ROLE lattice_migrator; \
             ALTER TABLE writer_lease.writer_lease_heads ADD COLUMN catalog_tombstone integer; \
             ALTER TABLE writer_lease.writer_lease_heads DROP COLUMN catalog_tombstone; \
             RESET ROLE;",
        )
        .expect("commit dropped-column tombstone drift");
    assert_profile_rejects(&mut migrator, &target);
    migrator
        .batch_execute("SET ROLE lattice_migrator; DROP SCHEMA writer_lease CASCADE; RESET ROLE;")
        .expect("discard tombstoned disposable extension");
    let fresh_outcomes = task076_concurrent_apply(&mut migrator, &migrator_url, &target, false);
    assert_eq!(
        fresh_outcomes
            .into_iter()
            .filter(|outcome| *outcome == ExtensionApplyOutcome::Installed)
            .count(),
        1
    );
    assert_eq!(
        fresh_outcomes
            .into_iter()
            .filter(|outcome| *outcome == ExtensionApplyOutcome::AlreadyCurrent)
            .count(),
        1
    );
    assert_profile_restored(&mut migrator, &target);
    assert_task076_fresh_current_profile(&mut migrator);
    println!("TASK076_WRITER_FRESH_CONCURRENT_PASS");
    assert_eq!(
        apply_extension(&mut migrator, &target).expect("reapply fresh Writer v2 current"),
        ExtensionApplyOutcome::AlreadyCurrent
    );
    assert_profile_restored(&mut migrator, &target);
    assert_task076_fresh_current_profile(&mut migrator);
    println!("TASK076_WRITER_FRESH_NOOP_PASS");
}

#[test]
#[allow(clippy::too_many_lines)]
fn live_v5_process_handoff_preserves_active_lineage_when_provisioned() {
    let Ok(migrator_url) = std::env::var("LATTICE_WRITER_LEASE_V5_MIGRATOR_URL") else {
        eprintln!("SKIP: LATTICE_WRITER_LEASE_V5_MIGRATOR_URL is not configured");
        return;
    };
    let runtime_url = std::env::var("LATTICE_WRITER_LEASE_V5_RUNTIME_URL")
        .expect("v5 runtime URL is required when the live test is enabled");
    let database_name = std::env::var("LATTICE_WRITER_LEASE_V5_DATABASE_NAME")
        .expect("v5 database name is required when the live test is enabled");
    let database_identity = digest_env("LATTICE_WRITER_LEASE_V5_DATABASE_IDENTITY_SHA256");
    let v4_target = V4ExtensionTarget::new(database_name.clone(), database_identity.clone())
        .expect("v4 predecessor target");
    let v5_target =
        V5ExtensionTarget::new(database_name, database_identity).expect("v5 successor target");
    let store_authority = authority_env();
    let mut migrator = Client::connect(&migrator_url, NoTls).expect("v5 migrator connection");
    let run_suffix = format!(
        "{:x}-{:x}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    );
    let project_id = ProjectId::new(format!("writer-v5-{run_suffix}")).expect("unique v5 project");
    let predecessor_process_id = HolderProcessId::new(75_001).expect("predecessor pid");
    let predecessor_process_start = digest('e');

    let runtime = Client::connect(&runtime_url, NoTls).expect("v4 runtime connection");
    let mut v4 = PostgresWriterLease::new_v4_v7(runtime, &v4_target, &store_authority, 600)
        .expect("v4 predecessor adapter");
    let acquire = WriterLeaseRepositoryCommand::Acquire(WriterLeaseAcquireRequest {
        command_id: format!("v5-{run_suffix}-acquire"),
        expected_head: None,
        project_id: project_id.clone(),
        project_snapshot_id: ProjectSnapshotId::new("snapshot-v5").expect("snapshot"),
        task_id: TaskId::new("task-v5-handoff").expect("task"),
        task_revision: "1".to_owned(),
        task_spec_digest: digest('d'),
        attempt_id: AttemptId::new("attempt-v5-retained").expect("attempt"),
        lease_id: "lease-v5-retained".to_owned(),
        lease_holder_id: "foreman-v5".to_owned(),
        worktree_id: "worktree-v5-retained".to_owned(),
        holder_process_id: predecessor_process_id,
        holder_process_start_identity: predecessor_process_start.clone(),
    });
    let acquired = v4.execute(acquire).expect("v4 acquire");
    assert_eq!(acquired.outcome, CommandOutcome::Applied);
    let predecessor_head = acquired.after.clone().expect("predecessor head");
    let predecessor_fence = predecessor_head.identity().fencing_token();
    drop(v4);

    task076_fixture_stop_admission(&mut migrator, &store_authority);
    assert_eq!(
        apply_v5_extension(&mut migrator, &v5_target).expect("v4 to v5 upgrade"),
        ExtensionApplyOutcome::Activated
    );
    assert_eq!(
        apply_v5_extension(&mut migrator, &v5_target).expect("exact v5 setup retry"),
        ExtensionApplyOutcome::AlreadyCurrent
    );
    task076_fixture_restore_admission(&mut migrator, &store_authority);

    let runtime = Client::connect(&runtime_url, NoTls).expect("v5 runtime connection");
    let mut v5 = PostgresWriterLease::new_v5_v7(runtime, &v5_target, &store_authority, 600)
        .expect("v5 adapter");
    assert_eq!(
        v5.current_authority(&project_id)
            .expect("replayed predecessor")
            .expect("retained authority")
            .independent_head(),
        &predecessor_head
    );
    let handoff_request =
        WriterLeaseRepositoryCommand::ProcessHandoff(WriterLeaseProcessHandoffRequest {
            command_id: format!("v5-{run_suffix}-handoff"),
            project_id: project_id.clone(),
            expected_head: predecessor_head.clone(),
            successor_holder_process_id: predecessor_process_id,
            successor_holder_process_start_identity: digest('f'),
            evidence: RecoveryEvidence::ProcessDeath {
                holder_process_id: predecessor_process_id,
                holder_process_start_identity: predecessor_process_start,
                holder_daemon_instance_id: store_authority.daemon_instance_id().as_str().to_owned(),
                evidence_digest: digest('a'),
            },
        });
    let handed_off = v5
        .execute(handoff_request.clone())
        .expect("v5 process handoff");
    assert_eq!(handed_off.outcome, CommandOutcome::Applied);
    let successor_head = handed_off.after.clone().expect("successor head");
    assert_eq!(
        successor_head.identity().attempt_id(),
        predecessor_head.identity().attempt_id()
    );
    assert_eq!(
        successor_head.identity().lease_id(),
        predecessor_head.identity().lease_id()
    );
    assert_eq!(successor_head.identity().fencing_token(), predecessor_fence);
    assert_eq!(
        successor_head.identity().holder_process_id(),
        predecessor_process_id
    );
    assert_eq!(
        v5.execute(handoff_request.clone())
            .expect("exact handoff replay"),
        handed_off
    );
    let mut substituted = handoff_request;
    let WriterLeaseRepositoryCommand::ProcessHandoff(request) = &mut substituted else {
        unreachable!();
    };
    request.successor_holder_process_start_identity = digest('9');
    let substitution = v5
        .execute(substituted)
        .expect_err("same command id cannot substitute successor");
    assert_eq!(substitution.kind(), WriterLeaseRepositoryErrorKind::Domain);
    assert_eq!(
        substitution.domain(),
        Some(lattice_writer_lease::WriterLeaseError::CommandIdReuse)
    );

    let release = WriterLeaseRepositoryCommand::Release(WriterLeaseReleaseRequest {
        command_id: format!("v5-{run_suffix}-release"),
        project_id: project_id.clone(),
        expected_head: successor_head.clone(),
    });
    assert_eq!(
        v5.execute(release).expect("release successor").outcome,
        CommandOutcome::Applied
    );
    let historical = v5
        .inspect_historical_authority(&project_id, predecessor_head.receipt_digest())
        .expect("released historical lookup")
        .expect("historical predecessor receipt");
    assert_eq!(historical.head(), predecessor_head);
    drop(v5);

    let runtime = Client::connect(&runtime_url, NoTls).expect("fresh v5 runtime connection");
    let mut restarted = PostgresWriterLease::new_v5_v7(runtime, &v5_target, &store_authority, 600)
        .expect("fresh v5 adapter");
    assert_eq!(
        restarted
            .inspect_historical_authority(&project_id, successor_head.receipt_digest())
            .expect("fresh historical lookup")
            .expect("historical successor")
            .head(),
        successor_head
    );
}
