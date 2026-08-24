use lattice_contracts::{
    ActivityEvent, ActivityEventId, ActivityKind, AttemptId, AuthorityObservation,
    CONTRACT_VERSION, ContentDigest, DaemonEpoch, FencingToken, Freshness, GatewayTaskProjection,
    GatewayTaskState, HolderProcessId, ObservationConfidence, ObservationCursor, ObservationLevel,
    ObservationListFilter, ObservationPageSize, ObservationQuery, ObservationSource,
    ProcessBinding, ProcessEnvironment, ProcessId, ProcessState, ProjectId, ProjectSnapshotId,
    RuntimeAdmissionMode, RuntimeKind, SubjectBinding, TaskBinding, TaskId,
    WORKER_OBSERVATION_CONTRACT_VERSION, WRITER_LEASE_PRODUCER_ID, WRITER_LEASE_PRODUCER_VERSION,
    WorkSessionId, WorkSessionObservation, WorkSessionState, WorkerInstance, WorkerInstanceId,
    WorkerObservation, WorkerObservationContractError, WorkerOwnership, WorkerProvider,
    WorkerProviderId, WorkerProviderKind, WriterLeaseAuthorityReceipt, WriterLeaseIdentity,
    WriterLeaseRevision, WriterLeaseStatus,
};

fn digest(value: char) -> ContentDigest {
    ContentDigest::from_sha256(value.to_string().repeat(64)).expect("digest")
}

fn binding(task: &str) -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new("worker-observation").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new(task).expect("task"),
        "1",
        digest('1'),
    )
    .expect("binding")
}

fn task_binding() -> TaskBinding {
    TaskBinding::new(
        binding("TASK-048"),
        Some(AttemptId::new("attempt-1").expect("attempt")),
    )
    .expect("task binding")
}

fn writer_head() -> lattice_contracts::WriterLeaseAuthorityHead {
    let binding = task_binding();
    let identity = WriterLeaseIdentity::new(
        binding.binding().project_id().clone(),
        binding.binding().project_snapshot_id().clone(),
        binding.binding().task_id().clone(),
        binding.binding().task_revision(),
        binding.binding().task_spec_digest().clone(),
        binding.attempt_id().expect("attempt").clone(),
        "lease-1",
        "codex-writer-1",
        "worktree-1",
        HolderProcessId::new(4242).expect("holder process"),
        digest('2'),
        "daemon-1",
        DaemonEpoch::new(7).expect("epoch"),
        FencingToken::new(11).expect("fence"),
    )
    .expect("writer identity");
    WriterLeaseAuthorityReceipt::new(
        CONTRACT_VERSION,
        WRITER_LEASE_PRODUCER_ID,
        WRITER_LEASE_PRODUCER_VERSION,
        RuntimeKind::Live,
        identity,
        WriterLeaseStatus::Active,
        WriterLeaseRevision::new(3).expect("revision"),
        RuntimeAdmissionMode::Active,
        "2026-08-10T00:00:00Z",
        "2026-08-10T00:01:00Z",
        "2026-08-10T00:02:00Z",
        digest('3'),
        digest('4'),
        digest('5'),
        digest('6'),
    )
    .expect("writer receipt")
    .head()
}

#[test]
fn provider_instance_session_activity_task_and_process_models_are_neutral_and_bounded() {
    let providers = [
        WorkerProvider::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            WorkerProviderId::new("codex-app-server").expect("provider"),
            WorkerProviderKind::AiAgent,
        ),
        WorkerProvider::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            WorkerProviderId::new("powershell").expect("provider"),
            WorkerProviderKind::Terminal,
        ),
        WorkerProvider::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            WorkerProviderId::new("verification-runner").expect("provider"),
            WorkerProviderKind::Verification,
        ),
    ];
    assert!(providers.iter().all(Result::is_ok));

    let provider = providers[0].clone().expect("provider");
    let instance = WorkerInstance::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkerInstanceId::new("worker-1").expect("instance"),
        provider.id().clone(),
        WorkerOwnership::LatticeManaged,
    )
    .expect("instance");
    let process = ProcessBinding::new(
        ProcessEnvironment::Windows,
        ProcessId::new(4242).expect("pid"),
        Some(digest('2')),
        Some(ProcessId::new(4000).expect("parent")),
        ProcessState::Running,
        Freshness::Current,
        ObservationSource::ManagedProcessSupervisor,
        ObservationConfidence::VerifiedStructured,
        "2026-08-10T00:01:00Z",
        digest('c'),
    )
    .expect("process");
    let activity = ActivityEvent::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        ActivityEventId::new("event-1").expect("event"),
        WorkSessionId::new("session-1").expect("session"),
        1,
        ActivityKind::Heartbeat,
        ObservationSource::LatticeActivityEvent,
        ObservationConfidence::VerifiedStructured,
        "2026-08-10T00:01:00Z",
        Some(WorkSessionState::Idle),
        Some(ProcessState::Running),
        digest('7'),
    )
    .expect("activity");
    assert_eq!(activity.sequence(), 1);
    assert_eq!(
        activity.confidence(),
        ObservationConfidence::VerifiedStructured
    );
    assert_eq!(activity.session_state_after(), Some(WorkSessionState::Idle));
    assert_eq!(activity.process_state_after(), Some(ProcessState::Running));

    let task = task_binding();
    let task_projection = GatewayTaskProjection::new(
        task.binding().clone(),
        GatewayTaskState::Executing,
        digest('8'),
        digest('9'),
    )
    .expect("task projection");
    let session = WorkSessionObservation::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkSessionId::new("session-1").expect("session"),
        instance.id().clone(),
        ObservationLevel::LatticeManaged,
        WorkSessionState::Idle,
        Freshness::Current,
        ObservationSource::LatticeActivityEvent,
        ObservationConfidence::VerifiedStructured,
        "2026-08-10T00:01:00Z",
        Some(process),
        Some(task),
        Some(task_projection),
        AuthorityObservation::WriterLease(Box::new(writer_head())),
        Some(activity.id().clone()),
        digest('a'),
    )
    .expect("session");
    let observation = WorkerObservation::new(provider, instance, session).expect("observation");

    assert_eq!(observation.provider().kind(), WorkerProviderKind::AiAgent);
    assert_eq!(
        observation.instance().ownership(),
        WorkerOwnership::LatticeManaged
    );
    assert_eq!(observation.session().state(), WorkSessionState::Idle);
    assert_eq!(
        observation.session().confidence(),
        ObservationConfidence::VerifiedStructured
    );
}

#[test]
fn worker_observation_identifiers_are_bounded_and_not_path_or_secret_fragments() {
    assert!(WorkerProviderId::new("a".repeat(128)).is_ok());
    assert_eq!(
        WorkerProviderId::new("a".repeat(129)),
        Err(WorkerObservationContractError::InvalidValue {
            field: "worker_provider_id"
        })
    );
    assert!(WorkerProviderId::new("provider/path").is_err());
    assert!(WorkerProviderId::new("provider secret=value").is_err());
}

#[test]
fn process_supervision_cannot_claim_worker_session_state() {
    let process = ProcessBinding::new(
        ProcessEnvironment::Windows,
        ProcessId::new(4242).expect("pid"),
        Some(digest('2')),
        None,
        ProcessState::Running,
        Freshness::Current,
        ObservationSource::ManagedProcessSupervisor,
        ObservationConfidence::VerifiedStructured,
        "2026-08-10T00:01:00Z",
        digest('c'),
    )
    .expect("process");

    assert!(matches!(
        WorkSessionObservation::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            WorkSessionId::new("session-process-only-source").expect("session"),
            WorkerInstanceId::new("worker-1").expect("instance"),
            ObservationLevel::LatticeManaged,
            WorkSessionState::Running,
            Freshness::Current,
            ObservationSource::ManagedProcessSupervisor,
            ObservationConfidence::VerifiedStructured,
            "2026-08-10T00:01:00Z",
            Some(process),
            None,
            None,
            AuthorityObservation::NotObserved,
            None,
            digest('d'),
        ),
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "process_source_session_state"
        })
    ));
}

#[test]
fn process_session_task_and_authority_states_remain_independent() {
    let provider = WorkerProvider::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkerProviderId::new("codex-app-server").expect("provider"),
        WorkerProviderKind::AiAgent,
    )
    .expect("provider");
    let instance = WorkerInstance::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkerInstanceId::new("worker-1").expect("instance"),
        provider.id().clone(),
        WorkerOwnership::LatticeManaged,
    )
    .expect("instance");
    let process = ProcessBinding::new(
        ProcessEnvironment::Windows,
        ProcessId::new(4242).expect("pid"),
        Some(digest('2')),
        None,
        ProcessState::Running,
        Freshness::Current,
        ObservationSource::ManagedProcessSupervisor,
        ObservationConfidence::VerifiedStructured,
        "2026-08-10T00:01:00Z",
        digest('c'),
    )
    .expect("process");
    let task = task_binding();
    let projection = GatewayTaskProjection::new(
        task.binding().clone(),
        GatewayTaskState::Verifying,
        digest('8'),
        digest('9'),
    )
    .expect("task projection");
    let session = WorkSessionObservation::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkSessionId::new("session-1").expect("session"),
        instance.id().clone(),
        ObservationLevel::LatticeManaged,
        WorkSessionState::Idle,
        Freshness::Stale,
        ObservationSource::LatticeActivityEvent,
        ObservationConfidence::VerifiedStructured,
        "2026-08-10T00:01:00Z",
        Some(process),
        Some(task),
        Some(projection),
        AuthorityObservation::WriterLease(Box::new(writer_head())),
        None,
        digest('a'),
    )
    .expect("session");
    let observation = WorkerObservation::new(provider, instance, session).expect("observation");

    assert_eq!(
        observation.session().process().expect("process").state(),
        ProcessState::Running
    );
    assert_eq!(
        observation.session().process().expect("process").source(),
        ObservationSource::ManagedProcessSupervisor
    );
    assert_eq!(observation.session().state(), WorkSessionState::Idle);
    assert_eq!(observation.session().freshness(), Freshness::Stale);
    assert_eq!(
        observation
            .session()
            .task_projection()
            .expect("task projection")
            .state(),
        GatewayTaskState::Verifying
    );
    assert!(matches!(
        observation.session().authority(),
        AuthorityObservation::WriterLease(head)
            if head.status() == WriterLeaseStatus::Active
                && head.runtime_admission() == RuntimeAdmissionMode::Active
    ));
}

#[test]
fn process_only_and_unobservable_sessions_degrade_without_task_or_progress_claims() {
    let instance_id = WorkerInstanceId::new("worker-1").expect("instance");
    let process = || {
        ProcessBinding::new(
            ProcessEnvironment::PowerShell,
            ProcessId::new(4242).expect("pid"),
            None,
            None,
            ProcessState::Running,
            Freshness::Current,
            ObservationSource::ProcessDiscovery,
            ObservationConfidence::PresenceOnly,
            "2026-08-10T00:01:00Z",
            digest('c'),
        )
        .expect("process")
    };

    let discovered = WorkSessionObservation::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkSessionId::new("session-1").expect("session"),
        instance_id.clone(),
        ObservationLevel::ProcessPresenceOnly,
        WorkSessionState::Unknown,
        Freshness::Current,
        ObservationSource::ProcessDiscovery,
        ObservationConfidence::PresenceOnly,
        "2026-08-10T00:01:00Z",
        Some(process()),
        None,
        None,
        AuthorityObservation::NotObserved,
        None,
        digest('a'),
    )
    .expect("discovered session");
    assert!(discovered.task_binding().is_none());
    assert!(discovered.task_projection().is_none());

    assert!(matches!(
        WorkSessionObservation::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            WorkSessionId::new("session-2").expect("session"),
            instance_id.clone(),
            ObservationLevel::ProcessPresenceOnly,
            WorkSessionState::Running,
            Freshness::Current,
            ObservationSource::ProcessDiscovery,
            ObservationConfidence::PresenceOnly,
            "2026-08-10T00:01:00Z",
            Some(process()),
            None,
            None,
            AuthorityObservation::NotObserved,
            None,
            digest('a'),
        ),
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "process_only_session_state"
        })
    ));
    assert!(matches!(
        WorkSessionObservation::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            WorkSessionId::new("session-3").expect("session"),
            instance_id.clone(),
            ObservationLevel::ProcessPresenceOnly,
            WorkSessionState::Unknown,
            Freshness::Current,
            ObservationSource::ProcessDiscovery,
            ObservationConfidence::PresenceOnly,
            "2026-08-10T00:01:00Z",
            Some(process()),
            Some(task_binding()),
            None,
            AuthorityObservation::NotObserved,
            None,
            digest('a'),
        ),
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "process_only_task_binding"
        })
    ));

    let unobservable = WorkSessionObservation::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkSessionId::new("session-4").expect("session"),
        instance_id,
        ObservationLevel::Unobservable,
        WorkSessionState::Unobservable,
        Freshness::Unknown,
        ObservationSource::DeclaredUnobservable,
        ObservationConfidence::Unknown,
        "2026-08-10T00:01:00Z",
        None,
        None,
        None,
        AuthorityObservation::NotObserved,
        None,
        digest('b'),
    )
    .expect("unobservable session");
    assert!(unobservable.process().is_none());
}

#[test]
fn process_discovery_cannot_claim_session_activity() {
    assert_eq!(
        ObservationConfidence::PresenceOnly.as_str(),
        "PRESENCE_ONLY"
    );
    assert!(matches!(
        ActivityEvent::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            ActivityEventId::new("event-process-1").expect("event"),
            WorkSessionId::new("session-5").expect("session"),
            1,
            ActivityKind::Progress,
            ObservationSource::ProcessDiscovery,
            ObservationConfidence::PresenceOnly,
            "2026-08-10T00:01:00Z",
            Some(WorkSessionState::Running),
            Some(ProcessState::Running),
            digest('d'),
        ),
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "process_discovery_activity"
        })
    ));

    let process_presence = ActivityEvent::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        ActivityEventId::new("event-process-2").expect("event"),
        WorkSessionId::new("session-5").expect("session"),
        2,
        ActivityKind::ProcessDiscovered,
        ObservationSource::ProcessDiscovery,
        ObservationConfidence::PresenceOnly,
        "2026-08-10T00:01:01Z",
        None,
        Some(ProcessState::Running),
        digest('e'),
    )
    .expect("process lifecycle evidence");
    assert_eq!(
        process_presence.confidence(),
        ObservationConfidence::PresenceOnly
    );
    assert_eq!(process_presence.session_state_after(), None);
    assert_eq!(
        process_presence.process_state_after(),
        Some(ProcessState::Running)
    );

    assert!(
        ActivityEvent::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            ActivityEventId::new("event-process-3").expect("event"),
            WorkSessionId::new("session-5").expect("session"),
            3,
            ActivityKind::ProcessExited,
            ObservationSource::ProcessDiscovery,
            ObservationConfidence::PresenceOnly,
            "2026-08-10T00:01:02Z",
            None,
            Some(ProcessState::Running),
            digest('f'),
        )
        .is_err()
    );

    assert!(matches!(
        ActivityEvent::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            ActivityEventId::new("event-process-4").expect("event"),
            WorkSessionId::new("session-5").expect("session"),
            4,
            ActivityKind::ProcessDiscovered,
            ObservationSource::ProcessDiscovery,
            ObservationConfidence::VerifiedStructured,
            "2026-08-10T00:01:03Z",
            None,
            Some(ProcessState::Running),
            digest('1'),
        ),
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "observation_source_confidence"
        })
    ));
}

#[test]
fn exact_cross_bindings_reject_provider_instance_task_and_lease_substitution() {
    let codex = WorkerProvider::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkerProviderId::new("codex-app-server").expect("provider"),
        WorkerProviderKind::AiAgent,
    )
    .expect("provider");
    let powershell = WorkerProvider::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkerProviderId::new("powershell").expect("provider"),
        WorkerProviderKind::Terminal,
    )
    .expect("provider");
    let instance = WorkerInstance::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkerInstanceId::new("worker-1").expect("instance"),
        codex.id().clone(),
        WorkerOwnership::LatticeManaged,
    )
    .expect("instance");
    let task = task_binding();
    let wrong_projection = GatewayTaskProjection::new(
        binding("TASK-999"),
        GatewayTaskState::Executing,
        digest('8'),
        digest('9'),
    )
    .expect("projection");

    assert!(matches!(
        WorkSessionObservation::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            WorkSessionId::new("session-1").expect("session"),
            instance.id().clone(),
            ObservationLevel::LatticeManaged,
            WorkSessionState::Running,
            Freshness::Current,
            ObservationSource::LatticeActivityEvent,
            ObservationConfidence::VerifiedStructured,
            "2026-08-10T00:01:00Z",
            None,
            Some(task),
            Some(wrong_projection),
            AuthorityObservation::NotObserved,
            None,
            digest('a'),
        ),
        Err(WorkerObservationContractError::CrossBinding {
            field: "task_projection"
        })
    ));

    let session = WorkSessionObservation::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkSessionId::new("session-2").expect("session"),
        instance.id().clone(),
        ObservationLevel::LatticeManaged,
        WorkSessionState::Running,
        Freshness::Current,
        ObservationSource::LatticeActivityEvent,
        ObservationConfidence::VerifiedStructured,
        "2026-08-10T00:01:00Z",
        None,
        Some(task_binding()),
        None,
        AuthorityObservation::NotObserved,
        None,
        digest('a'),
    )
    .expect("session");
    assert_eq!(
        WorkerObservation::new(powershell, instance.clone(), session.clone()),
        Err(WorkerObservationContractError::CrossBinding {
            field: "worker_provider"
        })
    );

    let other_instance = WorkerInstance::new(
        WORKER_OBSERVATION_CONTRACT_VERSION,
        WorkerInstanceId::new("worker-2").expect("instance"),
        codex.id().clone(),
        WorkerOwnership::LatticeManaged,
    )
    .expect("instance");
    assert_eq!(
        WorkerObservation::new(codex, other_instance, session),
        Err(WorkerObservationContractError::CrossBinding {
            field: "worker_instance"
        })
    );
}

#[test]
fn writer_lease_observation_requires_exact_process_binding() {
    let instance_id = WorkerInstanceId::new("worker-1").expect("instance");
    assert!(matches!(
        WorkSessionObservation::new(
            WORKER_OBSERVATION_CONTRACT_VERSION,
            WorkSessionId::new("session-writer-without-process").expect("session"),
            instance_id,
            ObservationLevel::LatticeManaged,
            WorkSessionState::Running,
            Freshness::Current,
            ObservationSource::LatticeActivityEvent,
            ObservationConfidence::VerifiedStructured,
            "2026-08-10T00:01:00Z",
            None,
            Some(task_binding()),
            None,
            AuthorityObservation::WriterLease(Box::new(writer_head())),
            None,
            digest('a'),
        ),
        Err(WorkerObservationContractError::InconsistentObservation {
            field: "writer_lease_without_process_binding"
        })
    ));
}

#[test]
fn query_contract_is_closed_to_read_only_worker_and_session_list_or_status() {
    let filter = ObservationListFilter::new(
        Some(WorkerProviderId::new("codex-app-server").expect("provider")),
        Some(ProjectId::new("worker-observation").expect("project")),
        Some(TaskId::new("TASK-048").expect("task")),
        Some(WorkSessionState::Running),
        ObservationPageSize::new(100).expect("page"),
        Some(ObservationCursor::new("cursor-1").expect("cursor")),
    );
    let queries = [
        ObservationQuery::WorkerList(filter.clone()),
        ObservationQuery::WorkerStatus(WorkerInstanceId::new("worker-1").expect("worker")),
        ObservationQuery::SessionList(filter),
        ObservationQuery::SessionStatus(WorkSessionId::new("session-1").expect("session")),
    ];
    assert_eq!(
        queries.map(ObservationQuery::kind),
        [
            "WORKER_LIST",
            "WORKER_STATUS",
            "SESSION_LIST",
            "SESSION_STATUS",
        ]
    );
    assert!(ObservationPageSize::new(0).is_err());
    assert!(ObservationPageSize::new(101).is_err());
    assert!(ObservationCursor::new("a".repeat(512)).is_ok());
    assert!(ObservationCursor::new("a".repeat(513)).is_err());
    assert!(ObservationCursor::new("../task-control").is_err());
}
