use lattice_contracts::{
    AttemptId, ContentDigest, DaemonEpoch, HolderProcessId, ProjectId, ProjectSnapshotId,
    RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead, StoreAuthorityRevision,
    StoreDaemonInstanceId, TaskId,
};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionSetupErrorKind, ExtensionTarget, PostgresWriterLease,
    apply_extension, verify_extension,
};
use lattice_writer_lease::{
    CommandOutcome, LeaseDenial, VerifiedWriterLeaseAggregate, WriterLeaseAcquireRequest,
    WriterLeaseHeartbeatRequest, WriterLeaseReleaseRequest, WriterLeaseRepository,
    WriterLeaseRepositoryCommand, WriterLeaseRepositoryErrorKind,
};
use postgres::{Client, IsolationLevel, NoTls};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    let store_authority = authority_env();

    let mut migrator = Client::connect(&migrator_url, NoTls).expect("migrator connection");
    assert!(matches!(
        apply_extension(&mut migrator, &target).expect("apply extension"),
        ExtensionApplyOutcome::Installed | ExtensionApplyOutcome::AlreadyCurrent
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
            "SELECT * FROM writer_lease.writer_lease_load_for_update_v1($1,$2,$3,$4,$5)",
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
    assert_eq!(
        apply_extension(&mut migrator, &target).expect("fresh extension after tombstone"),
        ExtensionApplyOutcome::Installed
    );
    assert_profile_restored(&mut migrator, &target);
}
