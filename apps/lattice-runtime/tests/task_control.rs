use lattice_contracts::{
    AttemptId, ContentDigest, DaemonEpoch, GatewayChannelId, GatewayInstanceId, HolderProcessId,
    ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead,
    StoreAuthorityRevision, StoreDaemonInstanceId, SubjectBinding, TaskId, TaskIngressPeerEvidence,
    TaskLedgerStreamIdentity, WriterLeaseAuthorityHead, WriterLeaseAuthorityReceipt,
    WriterLeaseIdentity,
};
use lattice_ports::{
    HermesTaskReflectionCandidatePort, HermesTaskReflectionHistoryPort, TaskLifecycleErrorKind,
    TaskLifecyclePort, TaskReflectionEventKind, TaskReflectionHistoryQuery,
    TaskReflectionQueuePort,
};
use lattice_postgres_codebase_memory::verify_embedded_extension_manifest;
use lattice_postgres_store::{MigrationTarget, PostgresTaskLedger, PostgresTaskLedgerErrorKind};
use lattice_postgres_writer_lease::{
    ExtensionApplyOutcome, ExtensionTarget, PostgresWriterLease, apply_extension, verify_extension,
};
use lattice_runtime::delivery_ledger::DeliveryDatabaseBinding;
use lattice_runtime::task_control::PostgresTaskLifecycle;
use lattice_task_domain::{
    ReflectionCandidateKind, ReflectionFailureKind, ReflectionState, TaskState,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CommandOutcome as LedgerCommandOutcome,
    CorrelationId, LedgerEventKind, LedgerOutcome, ReasonCode,
};
use lattice_writer_lease::{
    CommandOutcome as LeaseCommandOutcome, WriterLeaseAcquireRequest, WriterLeaseReleaseRequest,
    WriterLeaseRepository, WriterLeaseRepositoryCommand,
};
use postgres::config::SslMode;
use postgres::{Client, Config, NoTls};
use std::time::{Duration, Instant};

const APPLICATION_NAME: &str = "lattice-devos-task019";

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn required_environment(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"))
}

fn connect_as(database: &str, role: &str) -> Client {
    let port = required_environment("LATTICE_TASK019_PORT")
        .parse::<u16>()
        .expect("port");
    let host = required_environment("LATTICE_TASK019_HOST");
    let login_role = format!("{role}_login");
    let password = required_environment("LATTICE_TASK019_PASSWORD");
    let mut config = Config::new();
    config
        .host(&host)
        .port(port)
        .user(&login_role)
        .password(password)
        .dbname(database)
        .application_name(APPLICATION_NAME)
        .ssl_mode(SslMode::Disable);
    let mut client = config.connect(NoTls).expect("fixed role connection");
    client
        .batch_execute(&format!("SET ROLE {role}"))
        .expect("fixed role activation");
    client
}

fn store_authority() -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new(required_environment("LATTICE_STORE_DAEMON_INSTANCE_ID"))
            .expect("daemon"),
        DaemonEpoch::new(
            required_environment("LATTICE_STORE_DAEMON_EPOCH")
                .parse::<u64>()
                .expect("epoch number"),
        )
        .expect("epoch"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(
            required_environment("LATTICE_STORE_AUTHORITY_REVISION")
                .parse::<u64>()
                .expect("revision number"),
        )
        .expect("revision"),
        ContentDigest::from_sha256(required_environment("LATTICE_STORE_OBSERVATION_DIGEST"))
            .expect("observation digest"),
        ContentDigest::from_sha256(required_environment("LATTICE_STORE_AUTHORITY_HEAD_DIGEST"))
            .expect("head digest"),
    )
    .expect("store authority")
}

fn gh9_store_authority() -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new("daemon-live-1").expect("daemon"),
        DaemonEpoch::new(7).expect("epoch"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(3).expect("revision"),
        digest('a'),
        digest('b'),
    )
    .expect("GH-9 store authority")
}

fn append(
    head: lattice_contracts::TaskLedgerStreamHead,
    command_id: &str,
    subject: ContentDigest,
) -> AppendCommand {
    AppendCommand::new(
        head,
        CommandId::new(command_id).expect("command"),
        CorrelationId::new("task038-same-transaction-fence").expect("correlation"),
        "2000-01-01T00:00:00Z",
        LedgerEventKind::EvidenceRecorded,
        ActorId::new("task038-live-acceptance").expect("actor"),
        ActionId::new("FENCED_EVIDENCE").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("TASK038_SAME_TRANSACTION_FENCE").expect("reason"),
        subject,
        None,
        None,
    )
    .expect("append command")
}

fn substitute_task_identity(
    authority: &WriterLeaseAuthorityHead,
    task_id: TaskId,
    task_spec_digest: ContentDigest,
) -> WriterLeaseAuthorityHead {
    let current = authority.identity();
    let substituted = WriterLeaseIdentity::new(
        current.project_id().clone(),
        current.project_snapshot_id().clone(),
        task_id,
        current.task_revision(),
        task_spec_digest,
        current.attempt_id().clone(),
        current.lease_id(),
        current.lease_holder_id(),
        current.worktree_id(),
        current.holder_process_id(),
        current.holder_process_start_identity().clone(),
        current.daemon_instance_id(),
        current.daemon_epoch(),
        current.fencing_token(),
    )
    .expect("substituted structural identity");
    WriterLeaseAuthorityReceipt::new(
        authority.version(),
        authority.producer_id(),
        authority.producer_version(),
        authority.runtime(),
        substituted,
        authority.status(),
        authority.revision(),
        authority.runtime_admission(),
        authority.acquired_at(),
        authority.heartbeat_at(),
        authority.expires_at(),
        authority.time_observation_digest().clone(),
        authority.admission_observation_digest().clone(),
        authority.transition_digest().clone(),
        authority.receipt_digest().clone(),
    )
    .expect("substituted structural receipt")
    .head()
}

fn gh9_binding() -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new("gh9-reflection-evolution").expect("project"),
        ProjectSnapshotId::new("gh9-reflection-snapshot").expect("snapshot"),
        TaskId::new("GH-9-REFLECTION-EVOLUTION").expect("task"),
        "1",
        digest('d'),
    )
    .expect("GH-9 binding")
}

fn gh9_identity(binding: &SubjectBinding) -> TaskLedgerStreamIdentity {
    TaskLedgerStreamIdentity::new(
        binding.project_id().clone(),
        binding.project_snapshot_id().clone(),
        binding.task_id().clone(),
        binding.task_revision(),
        binding.task_spec_digest().clone(),
        "TWD",
    )
    .expect("GH-9 identity")
}

fn gh9_ingress_peer() -> TaskIngressPeerEvidence {
    TaskIngressPeerEvidence::new_local_canonical_mcp_acceptance_live(
        GatewayInstanceId::new("gh9-local-acceptance").expect("gateway"),
        "1.0.0",
        digest('3'),
        digest('4'),
        GatewayChannelId::new("stdio").expect("channel"),
        digest('5'),
        digest('6'),
    )
    .expect("GH-9 ingress peer")
}

fn gh9_delivery_binding(run_id: &str) -> DeliveryDatabaseBinding {
    DeliveryDatabaseBinding::new(
        required_environment("LATTICE_TASK019_HOST"),
        required_environment("LATTICE_TASK019_PORT")
            .parse::<u16>()
            .expect("port"),
        run_id,
    )
    .expect("GH-9 delivery binding")
}

fn gh9_lifecycle(run_id: &str, binding: &SubjectBinding) -> PostgresTaskLifecycle {
    PostgresTaskLifecycle::connect_with_ingress_peer(
        &gh9_delivery_binding(run_id),
        &required_environment("LATTICE_TASK019_PASSWORD"),
        Instant::now() + Duration::from_secs(30),
        gh9_identity(binding),
        gh9_store_authority(),
        gh9_ingress_peer(),
    )
    .expect("GH-9 lifecycle")
}

#[test]
#[allow(clippy::too_many_lines)]
fn reflection_core_and_journal_replay_across_postgres_restart_when_provisioned() {
    if std::env::var("LATTICE_GH9_REFLECTION_LIVE").ok().as_deref() != Some("1") {
        eprintln!("SKIP: LATTICE_GH9_REFLECTION_LIVE is not configured");
        return;
    }
    assert_eq!(required_environment("LATTICE_TASK019_LIVE"), "1");
    let phase = required_environment("LATTICE_TASK019_PHASE");
    assert!(matches!(phase.as_str(), "initial" | "restart"));
    let run_id = required_environment("LATTICE_TASK019_RUN_ID");
    assert_eq!(run_id.len(), 32);
    let database = format!("lattice_task019_{}_base", &run_id[..8]);
    let migration_target = MigrationTarget::new(database.clone(), run_id.clone()).expect("target");
    let binding = gh9_binding();
    let identity = gh9_identity(&binding);
    let result_digest = digest('2');
    let candidate_digest = digest('c');
    let failure_digest = digest('f');

    if phase == "initial" {
        let runtime = connect_as(&database, "lattice_runtime");
        let mut ledger = PostgresTaskLedger::new(runtime, &migration_target).expect("ledger");
        let vacant = ledger.load_stream(identity.clone()).expect("vacant stream");
        assert!(vacant.stream().events().is_empty());
        let memory_manifest = verify_embedded_extension_manifest().expect("memory manifest");
        let extension_target = ExtensionTarget::new(
            database.clone(),
            vacant.persistence().database_identity_digest().clone(),
            vacant.persistence().manifest_digest().clone(),
            memory_manifest.manifest_sha256().clone(),
        )
        .expect("writer extension target");
        let mut migrator = connect_as(&database, "lattice_migrator");
        assert!(matches!(
            apply_extension(&mut migrator, &extension_target).expect("apply writer extension"),
            ExtensionApplyOutcome::Installed | ExtensionApplyOutcome::AlreadyCurrent
        ));
        verify_extension(&mut migrator, &extension_target).expect("writer extension profile");

        let mut lifecycle = gh9_lifecycle(&run_id, &binding);
        lifecycle
            .admit(&binding, "gh9-reflection-submit")
            .expect("admit GH-9 Task");
        lifecycle
            .transition(
                &binding,
                TaskState::Draft,
                TaskState::AwaitingExecutionApproval,
                None,
            )
            .expect("execution approval");
        lifecycle
            .transition(
                &binding,
                TaskState::AwaitingExecutionApproval,
                TaskState::Preparing,
                None,
            )
            .expect("prepare Task");

        let writer_runtime = connect_as(&database, "lattice_runtime");
        let mut writer = PostgresWriterLease::new(
            writer_runtime,
            extension_target,
            &gh9_store_authority(),
            600,
        )
        .expect("writer repository");
        let acquired = writer
            .execute(WriterLeaseRepositoryCommand::Acquire(
                WriterLeaseAcquireRequest {
                    command_id: "gh9-writer-acquire".to_owned(),
                    expected_head: None,
                    project_id: binding.project_id().clone(),
                    project_snapshot_id: binding.project_snapshot_id().clone(),
                    task_id: binding.task_id().clone(),
                    task_revision: binding.task_revision().to_owned(),
                    task_spec_digest: binding.task_spec_digest().clone(),
                    attempt_id: AttemptId::new("gh9-attempt-1").expect("attempt"),
                    lease_id: "gh9-lease-1".to_owned(),
                    lease_holder_id: "gh9-controlled-writer".to_owned(),
                    worktree_id: "gh9-reflection-evolution".to_owned(),
                    holder_process_id: HolderProcessId::new(std::process::id().into())
                        .expect("pid"),
                    holder_process_start_identity: digest('e'),
                },
            ))
            .expect("acquire writer");
        let writer_authority = acquired.after.expect("writer authority");
        for (from, to) in [
            (TaskState::Preparing, TaskState::Executing),
            (TaskState::Executing, TaskState::Verifying),
            (TaskState::Verifying, TaskState::Reviewing),
            (TaskState::Reviewing, TaskState::AwaitingMergeApproval),
            (TaskState::AwaitingMergeApproval, TaskState::Merging),
        ] {
            lifecycle
                .transition(&binding, from, to, Some(&writer_authority))
                .expect("writer-owned transition");
        }
        lifecycle
            .record_result(&binding, &result_digest, &writer_authority)
            .expect("record core result");
        writer
            .execute(WriterLeaseRepositoryCommand::Release(
                WriterLeaseReleaseRequest {
                    command_id: "gh9-writer-release".to_owned(),
                    project_id: binding.project_id().clone(),
                    expected_head: writer_authority,
                },
            ))
            .expect("release writer");
        let completed = lifecycle
            .transition(&binding, TaskState::Merging, TaskState::Completed, None)
            .expect("complete core Task");
        let completed_core_head = completed.core_head_digest().clone();

        lifecycle
            .ensure_pending(&binding)
            .expect("queue Reflection");
        lifecycle
            .claim_pending(&binding, "gh9-reflection-claim:0")
            .expect("claim Reflection");
        let history_query = TaskReflectionHistoryQuery::latest(1).expect("history query");
        let history = lifecycle
            .read_authorized_history(&binding, history_query)
            .expect("authorized history");
        lifecycle
            .append_candidate(
                &binding,
                "gh9-reflection-candidate:0",
                ReflectionCandidateKind::Observation,
                history_query,
                history.history_digest(),
                &candidate_digest,
            )
            .expect("append digest-only candidate");
        lifecycle
            .record_failure(
                &binding,
                "gh9-reflection-failure:0",
                ReflectionFailureKind::HermesFailure,
                &failure_digest,
            )
            .expect("record Hermes failure");
        let exact_retry = lifecycle
            .append_candidate(
                &binding,
                "gh9-reflection-candidate:0",
                ReflectionCandidateKind::Observation,
                history_query,
                history.history_digest(),
                &candidate_digest,
            )
            .expect("candidate exact retry after later state");
        assert_eq!(exact_retry.state(), ReflectionState::Failed);

        let replayed_core = lifecycle.load(&binding).expect("replay core");
        let replayed_reflection = lifecycle
            .load_reflection(&binding)
            .expect("replay Reflection");
        let replayed_history = lifecycle
            .read_authorized_history(
                &binding,
                TaskReflectionHistoryQuery::latest(
                    lattice_ports::MAX_TASK_REFLECTION_HISTORY_EVENTS,
                )
                .expect("full history query"),
            )
            .expect("replay history");
        assert_eq!(replayed_core.state(), TaskState::Completed);
        assert_eq!(replayed_core.result_digest(), Some(&result_digest));
        assert_eq!(replayed_core.core_head_digest(), &completed_core_head);
        assert_eq!(replayed_reflection.state(), ReflectionState::Failed);
        assert_eq!(replayed_reflection.core_head_digest(), &completed_core_head);
        assert_eq!(replayed_history.events().len(), 4);
        assert_eq!(
            replayed_history.events().last().map(|event| event.kind()),
            Some(TaskReflectionEventKind::Failure(
                ReflectionFailureKind::HermesFailure,
            ))
        );
        println!("GH9_REFLECTION_INITIAL_OK core=COMPLETED reflection=REFLECTION_FAILED events=4");
        return;
    }

    let mut lifecycle = gh9_lifecycle(&run_id, &binding);
    let before_core = lifecycle.load(&binding).expect("fresh-process core replay");
    let before_reflection = lifecycle
        .load_reflection(&binding)
        .expect("fresh-process Reflection replay");
    let before_history = lifecycle
        .read_authorized_history(
            &binding,
            TaskReflectionHistoryQuery::latest(lattice_ports::MAX_TASK_REFLECTION_HISTORY_EVENTS)
                .expect("history query"),
        )
        .expect("fresh-process history replay");
    let stable_journal_head = before_core.ledger_head_digest().clone();
    assert_eq!(before_core.state(), TaskState::Completed);
    assert_eq!(before_core.result_digest(), Some(&result_digest));
    assert_eq!(before_reflection.state(), ReflectionState::Failed);
    assert_eq!(before_reflection.generation(), 0);
    assert_eq!(before_history.events().len(), 4);
    assert_eq!(
        before_history.core_head_digest(),
        before_core.core_head_digest()
    );
    assert_eq!(before_history.journal_head_digest(), &stable_journal_head);
    assert!(before_history.events().iter().any(|event| {
        event.kind() == TaskReflectionEventKind::Failure(ReflectionFailureKind::HermesFailure)
    }));

    let after_core = lifecycle.load(&binding).expect("repeat core replay");
    let after_reflection = lifecycle
        .load_reflection(&binding)
        .expect("repeat Reflection replay");
    assert_eq!(after_core.ledger_head_digest(), &stable_journal_head);
    assert_eq!(after_reflection.journal_head_digest(), &stable_journal_head);
    println!("GH9_REFLECTION_RESTART_OK core=COMPLETED reflection=REFLECTION_FAILED events=4");
}

#[test]
#[allow(clippy::too_many_lines)]
fn stale_writer_cannot_append_after_reacquire_in_the_same_transaction_when_provisioned() {
    if std::env::var("LATTICE_TASK038_LIVE").ok().as_deref() != Some("1") {
        eprintln!("SKIP: LATTICE_TASK038_LIVE is not configured");
        return;
    }
    assert_eq!(required_environment("LATTICE_TASK019_LIVE"), "1");
    assert_eq!(required_environment("LATTICE_TASK019_PHASE"), "restart");
    let run_id = required_environment("LATTICE_TASK019_RUN_ID");
    assert_eq!(run_id.len(), 32);
    let database = format!("lattice_task019_{}_base", &run_id[..8]);
    let migration_target = MigrationTarget::new(database.clone(), run_id.clone()).expect("target");

    let project_id = ProjectId::new("task038-same-tx-fence").expect("project");
    let snapshot_id = ProjectSnapshotId::new("task038-snapshot").expect("snapshot");
    let task_id = TaskId::new("TASK-038-FENCE").expect("task");
    let task_spec_digest = digest('d');
    let binding = SubjectBinding::new(
        project_id.clone(),
        snapshot_id.clone(),
        task_id.clone(),
        "1",
        task_spec_digest.clone(),
    )
    .expect("binding");
    let identity = TaskLedgerStreamIdentity::new(
        project_id.clone(),
        snapshot_id.clone(),
        task_id.clone(),
        binding.task_revision(),
        task_spec_digest.clone(),
        "TWD",
    )
    .expect("identity");

    let runtime = connect_as(&database, "lattice_runtime");
    let mut ledger = PostgresTaskLedger::new(runtime, &migration_target).expect("ledger");
    let vacant = ledger.load_stream(identity.clone()).expect("vacant stream");
    let memory_manifest = verify_embedded_extension_manifest().expect("memory manifest");
    let extension_target = ExtensionTarget::new(
        database.clone(),
        vacant.persistence().database_identity_digest().clone(),
        vacant.persistence().manifest_digest().clone(),
        memory_manifest.manifest_sha256().clone(),
    )
    .expect("writer extension target");
    let mut migrator = connect_as(&database, "lattice_migrator");
    assert!(matches!(
        apply_extension(&mut migrator, &extension_target).expect("apply writer extension"),
        ExtensionApplyOutcome::Installed | ExtensionApplyOutcome::AlreadyCurrent
    ));
    verify_extension(&mut migrator, &extension_target).expect("writer extension profile");

    // The concrete PostgreSQL lifecycle boundary must reject a missing writer
    // head before it can append the first lease-owned state transition. This
    // protects against a future internal caller bypassing the sole orchestrator.
    let lifecycle_binding = SubjectBinding::new(
        ProjectId::new("task038-transition-policy").expect("project"),
        ProjectSnapshotId::new("task038-transition-policy-snapshot").expect("snapshot"),
        TaskId::new("TASK-038-TRANSITION-POLICY").expect("task"),
        "1",
        digest('7'),
    )
    .expect("lifecycle binding");
    let lifecycle_identity = TaskLedgerStreamIdentity::new(
        lifecycle_binding.project_id().clone(),
        lifecycle_binding.project_snapshot_id().clone(),
        lifecycle_binding.task_id().clone(),
        lifecycle_binding.task_revision(),
        lifecycle_binding.task_spec_digest().clone(),
        "TWD",
    )
    .expect("lifecycle identity");
    let ingress_peer = TaskIngressPeerEvidence::new_local_canonical_mcp_acceptance_live(
        GatewayInstanceId::new("lattice-mcp-local-acceptance").expect("gateway"),
        "1.0.0",
        digest('8'),
        digest('9'),
        GatewayChannelId::new("stdio").expect("channel"),
        digest('a'),
        digest('b'),
    )
    .expect("ingress peer");
    let delivery_binding = DeliveryDatabaseBinding::new(
        required_environment("LATTICE_TASK019_HOST"),
        required_environment("LATTICE_TASK019_PORT")
            .parse::<u16>()
            .expect("port"),
        run_id,
    )
    .expect("delivery binding");
    let mut lifecycle = PostgresTaskLifecycle::connect_with_ingress_peer(
        &delivery_binding,
        &required_environment("LATTICE_TASK019_PASSWORD"),
        Instant::now() + Duration::from_secs(30),
        lifecycle_identity,
        store_authority(),
        ingress_peer,
    )
    .expect("lifecycle");
    lifecycle
        .admit(&lifecycle_binding, "task038-transition-policy-submit")
        .expect("admit lifecycle");
    lifecycle
        .transition(
            &lifecycle_binding,
            TaskState::Draft,
            TaskState::AwaitingExecutionApproval,
            None,
        )
        .expect("await execution approval");
    lifecycle
        .transition(
            &lifecycle_binding,
            TaskState::AwaitingExecutionApproval,
            TaskState::Preparing,
            None,
        )
        .expect("prepare lifecycle");
    let before_missing_authority = lifecycle.load(&lifecycle_binding).expect("before denial");
    let missing_authority = lifecycle
        .transition(
            &lifecycle_binding,
            TaskState::Preparing,
            TaskState::Executing,
            None,
        )
        .expect_err("missing authority must fail before append");
    assert_eq!(missing_authority.kind(), TaskLifecycleErrorKind::Rejected);
    assert_eq!(
        missing_authority.code(),
        "LATTICE_TASK_TRANSITION_WRITER_AUTHORITY_REQUIRED"
    );
    let after_missing_authority = lifecycle.load(&lifecycle_binding).expect("after denial");
    assert_eq!(after_missing_authority.state(), TaskState::Preparing);
    assert_eq!(
        after_missing_authority.ledger_head_digest(),
        before_missing_authority.ledger_head_digest()
    );

    let writer_runtime = connect_as(&database, "lattice_runtime");
    let mut writer =
        PostgresWriterLease::new(writer_runtime, extension_target, &store_authority(), 600)
            .expect("writer repository");
    let acquire = |command_id: &str, attempt_id: &str, lease_id: &str| {
        WriterLeaseRepositoryCommand::Acquire(WriterLeaseAcquireRequest {
            command_id: command_id.to_owned(),
            expected_head: None,
            project_id: project_id.clone(),
            project_snapshot_id: snapshot_id.clone(),
            task_id: task_id.clone(),
            task_revision: "1".to_owned(),
            task_spec_digest: task_spec_digest.clone(),
            attempt_id: AttemptId::new(attempt_id).expect("attempt"),
            lease_id: lease_id.to_owned(),
            lease_holder_id: "codex-writer".to_owned(),
            worktree_id: "task038-controlled-worktree".to_owned(),
            holder_process_id: HolderProcessId::new(std::process::id().into()).expect("pid"),
            holder_process_start_identity: digest('e'),
        })
    };
    let first = writer
        .execute(acquire(
            "task038-same-tx-acquire-1",
            "task038-same-tx-attempt-1",
            "task038-same-tx-lease-1",
        ))
        .expect("first acquire");
    assert_eq!(first.outcome, LeaseCommandOutcome::Applied);
    let old_authority = first.after.expect("first authority");

    // A caller cannot copy the genuine receipt digest/fence into a different
    // TaskSpec-bound stream. The Rust preflight accepts the forged shape for
    // that stream, then the same PostgreSQL transaction rejects it against the
    // complete current Writer Lease projection before appending any row.
    let substituted_task_id = TaskId::new("TASK-038-FENCE-SUBSTITUTED").expect("task");
    let substituted_spec_digest = digest('f');
    let substituted_identity = TaskLedgerStreamIdentity::new(
        project_id.clone(),
        snapshot_id.clone(),
        substituted_task_id.clone(),
        "1",
        substituted_spec_digest.clone(),
        "TWD",
    )
    .expect("substituted ledger identity");
    let substituted_stream = ledger
        .load_stream(substituted_identity.clone())
        .expect("substituted vacant stream");
    let forged_authority =
        substitute_task_identity(&old_authority, substituted_task_id, substituted_spec_digest);
    let cross_binding_error = ledger
        .execute_fenced(
            append(
                substituted_stream.stream().head().clone(),
                "task038-cross-bound-append",
                digest('0'),
            ),
            store_authority(),
            forged_authority,
        )
        .expect_err("copied receipt digest cannot authorize another task stream");
    assert_eq!(
        cross_binding_error.kind(),
        PostgresTaskLedgerErrorKind::AuthorityMismatch
    );
    assert!(
        ledger
            .load_stream(substituted_identity)
            .expect("substituted stream after denial")
            .stream()
            .events()
            .is_empty()
    );

    let first_append = ledger
        .execute_fenced(
            append(
                vacant.stream().head().clone(),
                "task038-fenced-append-1",
                digest('1'),
            ),
            store_authority(),
            old_authority.clone(),
        )
        .expect("first fenced append");
    assert_eq!(
        first_append.receipt().outcome(),
        &LedgerCommandOutcome::Appended
    );
    writer
        .execute(WriterLeaseRepositoryCommand::Release(
            WriterLeaseReleaseRequest {
                command_id: "task038-same-tx-release-1".to_owned(),
                project_id: project_id.clone(),
                expected_head: old_authority.clone(),
            },
        ))
        .expect("release first authority");
    let second = writer
        .execute(acquire(
            "task038-same-tx-acquire-2",
            "task038-same-tx-attempt-2",
            "task038-same-tx-lease-2",
        ))
        .expect("second acquire");
    let current_authority = second.after.expect("second authority");
    assert_eq!(current_authority.identity().fencing_token().get(), 2);

    let current_stream = ledger
        .load_stream(identity.clone())
        .expect("current stream");
    let stale = append(
        current_stream.stream().head().clone(),
        "task038-fenced-append-2",
        digest('2'),
    );
    let error = ledger
        .execute_fenced(stale.clone(), store_authority(), old_authority.clone())
        .expect_err("old fence must fail inside ledger transaction");
    assert_eq!(error.kind(), PostgresTaskLedgerErrorKind::AuthorityMismatch);
    let after_denial = ledger
        .load_stream(identity.clone())
        .expect("stream after denial");
    assert_eq!(after_denial.stream().events().len(), 1);
    ledger
        .execute_fenced(stale, store_authority(), current_authority.clone())
        .expect("current fence append");
    let final_stream = ledger.load_stream(identity).expect("final stream");
    assert_eq!(final_stream.stream().events().len(), 2);
    writer
        .execute(WriterLeaseRepositoryCommand::Release(
            WriterLeaseReleaseRequest {
                command_id: "task038-same-tx-release-2".to_owned(),
                project_id,
                expected_head: current_authority,
            },
        ))
        .expect("release current authority");

    println!("TASK038_SAME_TRANSACTION_FENCE_OK fencing_token=2 events=2");
}
