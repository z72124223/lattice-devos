use lattice_foreman_state::{
    AttemptPacketIdentity, AttemptWatchdogObservation, ContinuationSummary, ExternalCostBudget,
    MAX_ATTEMPTS, MAX_GLOBAL_ACTIVE_ATTEMPTS, MAX_REPAIR_RETRIES, MeaningfulProgress,
    MeaningfulProgressKind, ModelReason, ModelSelection, ProcessObservation, ReasoningEffort,
    ReconciliationState, RestartDecision, RetryDecision, StallClassification, StallReason,
    StartGateDecision, StartObservation, TurnActivityObservation, TurnStartedStatus,
    WorkerAttemptError, WorkerAttemptPhase, WorkerAttemptState, WorkerBudget, WorkerModel,
    WorkerTerminal, classify_attempt_stall, decide_repair_retry, restart_reconciliation_decision,
};

const EVIDENCE_A: &str =
    "evidence:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const EVIDENCE_B: &str =
    "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PROJECT: &str =
    "project:sha256:1111111111111111111111111111111111111111111111111111111111111111";
const SPEC: &str = "spec:sha256:2222222222222222222222222222222222222222222222222222222222222222";
const APPROVAL: &str =
    "approval:sha256:3333333333333333333333333333333333333333333333333333333333333333";
const VERIFICATION: &str =
    "verification:sha256:4444444444444444444444444444444444444444444444444444444444444444";
const WORKTREE: &str =
    "worktree:sha256:5555555555555555555555555555555555555555555555555555555555555555";

#[test]
fn model_reasoning_and_routing_are_closed_and_deterministic() {
    assert_eq!(
        WorkerModel::from_persisted("gpt-5.6-luna"),
        Ok(WorkerModel::Luna)
    );
    assert_eq!(
        WorkerModel::from_persisted("gpt-5.6-terra"),
        Ok(WorkerModel::Terra)
    );
    assert_eq!(
        WorkerModel::from_persisted("gpt-5.6-sol"),
        Ok(WorkerModel::Sol)
    );
    assert_eq!(
        WorkerModel::from_persisted("gpt-5.6"),
        Err(WorkerAttemptError::MalformedField)
    );
    assert_eq!(
        ReasoningEffort::from_persisted("medium"),
        Ok(ReasoningEffort::Medium)
    );
    assert_eq!(
        ReasoningEffort::from_persisted("max"),
        Ok(ReasoningEffort::Max)
    );
    assert_eq!(
        ReasoningEffort::from_persisted("ultra"),
        Ok(ReasoningEffort::Ultra)
    );
    assert_eq!(
        ReasoningEffort::from_persisted("MEDIUM"),
        Err(WorkerAttemptError::MalformedField)
    );
    for reason in [
        ModelReason::BoundedStateEvidenceDocumentation,
        ModelReason::RoutineEngineering,
        ModelReason::P0,
        ModelReason::Architecture,
        ModelReason::Security,
        ModelReason::HighRisk,
        ModelReason::TerraInsufficient,
    ] {
        assert_eq!(ModelReason::from_persisted(reason.as_str()), Ok(reason));
    }
    assert_eq!(
        ModelReason::from_persisted("routine_engineering"),
        Err(WorkerAttemptError::MalformedField)
    );

    let first = ModelSelection::new(
        WorkerModel::Terra,
        ReasoningEffort::Medium,
        ModelReason::RoutineEngineering,
        None,
    )
    .expect("default engineering route");
    let replay = ModelSelection::new(
        WorkerModel::Terra,
        ReasoningEffort::Medium,
        ModelReason::RoutineEngineering,
        None,
    )
    .expect("same route");
    assert_eq!(first, replay);
    assert_eq!(first.model(), WorkerModel::Terra);
    assert_eq!(first.reasoning(), ReasoningEffort::Medium);
    assert_eq!(first.reason(), ModelReason::RoutineEngineering);
    assert!(first.digest().starts_with("model-selection:sha256:"));
    assert_eq!(first.digest(), replay.digest());

    assert_eq!(
        ModelSelection::new(
            WorkerModel::Luna,
            ReasoningEffort::Low,
            ModelReason::RoutineEngineering,
            None,
        ),
        Err(WorkerAttemptError::InvalidModelReason)
    );
    assert_eq!(
        ModelSelection::new(
            WorkerModel::Sol,
            ReasoningEffort::High,
            ModelReason::TerraInsufficient,
            None,
        ),
        Err(WorkerAttemptError::MissingEvidence)
    );
    assert!(
        ModelSelection::new(
            WorkerModel::Sol,
            ReasoningEffort::High,
            ModelReason::TerraInsufficient,
            Some(EVIDENCE_A),
        )
        .is_ok()
    );
}

fn worker_budget() -> WorkerBudget {
    WorkerBudget::new(
        MAX_GLOBAL_ACTIVE_ATTEMPTS,
        1,
        MAX_REPAIR_RETRIES,
        900,
        100_000,
        3,
        ExternalCostBudget::Unavailable,
        "2026-08-26T12:30:00Z",
    )
    .expect("bounded worker budget")
}

fn terra() -> ModelSelection {
    ModelSelection::new(
        WorkerModel::Terra,
        ReasoningEffort::Medium,
        ModelReason::RoutineEngineering,
        None,
    )
    .expect("terra route")
}

fn packet(attempt: u8, writer_fence: u64) -> AttemptPacketIdentity {
    let prior = (attempt > 1).then_some(EVIDENCE_B);
    let continuation = (attempt > 1).then(|| {
        ContinuationSummary::new("Preserve verified work; repair the closed failure.").unwrap()
    });
    AttemptPacketIdentity::new(
        "taskref-phase4-001",
        attempt,
        PROJECT,
        SPEC,
        APPROVAL,
        &worker_budget(),
        VERIFICATION,
        WORKTREE,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        terra(),
        writer_fence,
        prior,
        continuation,
    )
    .expect("attempt packet")
}

#[test]
#[allow(clippy::too_many_lines)]
fn budget_and_attempt_packet_bind_exact_secret_safe_identity() {
    let budget = worker_budget();
    assert_eq!(budget.max_attempts(), MAX_ATTEMPTS);
    assert_eq!(budget.repair_retry_limit(), MAX_REPAIR_RETRIES);
    assert!(budget.allows_attempt(1));
    assert!(budget.allows_attempt(3));
    assert!(!budget.allows_attempt(4));
    assert!(budget.digest().starts_with("budget:sha256:"));
    assert_eq!(budget.digest(), worker_budget().digest());
    assert_eq!(
        WorkerBudget::new(
            5,
            1,
            2,
            900,
            100_000,
            3,
            ExternalCostBudget::Unavailable,
            "2026-08-26T12:30:00Z",
        ),
        Err(WorkerAttemptError::InvalidBudget)
    );
    assert_eq!(
        WorkerBudget::new(
            4,
            1,
            3,
            900,
            100_000,
            3,
            ExternalCostBudget::Unavailable,
            "2026-08-26T12:30:00Z",
        ),
        Err(WorkerAttemptError::InvalidBudget)
    );

    let first = AttemptPacketIdentity::new(
        "taskref-phase4-001",
        1,
        PROJECT,
        SPEC,
        APPROVAL,
        &budget,
        VERIFICATION,
        WORKTREE,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        terra(),
        41,
        None,
        None,
    )
    .expect("initial packet");
    let replay = AttemptPacketIdentity::new(
        "taskref-phase4-001",
        1,
        PROJECT,
        SPEC,
        APPROVAL,
        &budget,
        VERIFICATION,
        WORKTREE,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        terra(),
        41,
        None,
        None,
    )
    .expect("same packet");
    assert_eq!(first, replay);
    assert_eq!(first.attempt(), 1);
    assert_eq!(first.writer_fence(), 41);
    assert_eq!(first.model_selection().model(), WorkerModel::Terra);
    assert_eq!(first.budget_digest(), budget.digest());
    assert_eq!(first.remaining_total_tokens(), budget.max_total_tokens());
    assert_eq!(first.remaining_model_calls(), budget.max_model_calls());
    assert_eq!(
        first.execution_environment_ref(),
        "execution-environment:sha256:0000000000000000000000000000000000000000000000000000000000000001"
    );
    assert!(first.is_native_windows_execution_environment());
    assert!(first.digest().starts_with("attempt-packet:sha256:"));
    assert_eq!(first.digest(), replay.digest());
    let narrowed = first
        .clone()
        .with_remaining_budget(20_000, 1)
        .expect("replay-derived remaining budget");
    assert_eq!(narrowed.remaining_total_tokens(), 20_000);
    assert_eq!(narrowed.remaining_model_calls(), 1);
    assert_ne!(narrowed.digest(), first.digest());
    let wsl = first
        .clone()
        .with_execution_environment_ref(
            "execution-environment:sha256:6666666666666666666666666666666666666666666666666666666666666666",
        )
        .expect("typed WSL2 execution environment");
    assert_ne!(wsl.digest(), first.digest());
    assert_eq!(
        wsl.execution_environment_ref(),
        "execution-environment:sha256:6666666666666666666666666666666666666666666666666666666666666666"
    );
    assert!(!wsl.is_native_windows_execution_environment());
    assert_eq!(
        first.clone().with_remaining_budget(0, 1),
        Err(WorkerAttemptError::InvalidBudget)
    );
    assert_eq!(
        first.clone().with_remaining_budget(100_001, 1),
        Err(WorkerAttemptError::InvalidBudget)
    );

    let continuation = ContinuationSummary::new(
        "Focused tests passed; preserve the verified parser and repair only retry handling.",
    )
    .expect("bounded summary");
    assert!(continuation.digest().starts_with("continuation:sha256:"));
    let retry = AttemptPacketIdentity::new(
        "taskref-phase4-001",
        2,
        PROJECT,
        SPEC,
        APPROVAL,
        &budget,
        VERIFICATION,
        WORKTREE,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        terra(),
        42,
        Some(EVIDENCE_B),
        Some(continuation),
    )
    .expect("repair packet");
    let mut previous_state = WorkerAttemptState::new(first.clone()).unwrap();
    start_exact_turn(&mut previous_state);
    previous_state
        .record_terminal(
            "thread-phase4-001",
            "turn-phase4-001",
            WorkerTerminal::Failed,
            EVIDENCE_B,
        )
        .unwrap();
    retry
        .validate_repair_successor(&previous_state)
        .expect("same task and incremented attempt/fence");
    retry
        .validate_closed_prestart_repair_successor(&first, EVIDENCE_B)
        .expect("durable prestart closure is an exact repair predecessor");
    let cross_domain_retry = retry
        .clone()
        .with_execution_environment_ref(
            "execution-environment:sha256:7777777777777777777777777777777777777777777777777777777777777777",
        )
        .expect("different execution domain");
    assert_eq!(
        cross_domain_retry.validate_repair_successor(&previous_state),
        Err(WorkerAttemptError::InvalidAttempt)
    );
    assert_eq!(
        retry.validate_closed_prestart_repair_successor(&first, EVIDENCE_A),
        Err(WorkerAttemptError::InvalidAttempt)
    );
    assert_ne!(retry.digest(), first.digest());

    assert_eq!(
        ContinuationSummary::new("Bearer top-secret"),
        Err(WorkerAttemptError::ForbiddenContent)
    );
    assert_eq!(
        ContinuationSummary::new("x".repeat(513)),
        Err(WorkerAttemptError::MalformedField)
    );
    assert_eq!(
        AttemptPacketIdentity::new(
            "taskref-phase4-001",
            4,
            PROJECT,
            SPEC,
            APPROVAL,
            &budget,
            VERIFICATION,
            WORKTREE,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            terra(),
            43,
            Some(EVIDENCE_A),
            Some(ContinuationSummary::new("bounded repair").unwrap()),
        ),
        Err(WorkerAttemptError::InvalidAttempt)
    );
}

fn start_exact_turn(state: &mut WorkerAttemptState) {
    state.begin_dispatch().expect("durable dispatch intent");
    assert_eq!(
        state
            .apply_start(StartObservation::ThreadStartAccepted {
                thread_id: "thread-phase4-001".into(),
            })
            .unwrap(),
        StartGateDecision::Applied(WorkerAttemptPhase::Accepted)
    );
    assert_eq!(
        state
            .apply_start(StartObservation::ThreadStarted {
                thread_id: "thread-phase4-001".into(),
            })
            .unwrap(),
        StartGateDecision::Applied(WorkerAttemptPhase::Starting)
    );
    assert_eq!(
        state
            .apply_start(StartObservation::TurnStartAccepted {
                thread_id: "thread-phase4-001".into(),
                turn_id: "turn-phase4-001".into(),
            })
            .unwrap(),
        StartGateDecision::Applied(WorkerAttemptPhase::Starting)
    );
    assert_eq!(
        state
            .apply_start(StartObservation::TurnStarted {
                thread_id: "thread-phase4-001".into(),
                turn_id: "turn-phase4-001".into(),
                status: TurnStartedStatus::InProgress,
                observed_at: "2026-08-26T12:00:00Z".into(),
            })
            .unwrap(),
        StartGateDecision::Applied(WorkerAttemptPhase::Executing)
    );
}

#[test]
fn only_exact_in_progress_turn_started_enters_executing() {
    let mut state = WorkerAttemptState::new(packet(1, 41)).expect("claimed attempt");
    assert_eq!(state.phase(), WorkerAttemptPhase::Claimed);
    assert!(!state.is_real_running());
    state.begin_dispatch().expect("durable dispatch intent");
    assert_eq!(state.phase(), WorkerAttemptPhase::Dispatching);

    state
        .apply_start(StartObservation::ThreadStartAccepted {
            thread_id: "thread-phase4-001".into(),
        })
        .unwrap();
    assert_eq!(state.phase(), WorkerAttemptPhase::Accepted);
    assert!(!state.is_real_running());
    state
        .apply_start(StartObservation::ThreadStarted {
            thread_id: "thread-phase4-001".into(),
        })
        .unwrap();
    state
        .apply_start(StartObservation::TurnStartAccepted {
            thread_id: "thread-phase4-001".into(),
            turn_id: "turn-phase4-001".into(),
        })
        .unwrap();

    assert_eq!(
        state
            .apply_start(StartObservation::TurnStarted {
                thread_id: "thread-phase4-001".into(),
                turn_id: "turn-wrong".into(),
                status: TurnStartedStatus::InProgress,
                observed_at: "2026-08-26T12:00:00Z".into(),
            })
            .unwrap(),
        StartGateDecision::Ignored
    );
    assert_eq!(
        state
            .apply_start(StartObservation::TurnStarted {
                thread_id: "thread-phase4-001".into(),
                turn_id: "turn-phase4-001".into(),
                status: TurnStartedStatus::NotInProgress,
                observed_at: "2026-08-26T12:00:00Z".into(),
            })
            .unwrap(),
        StartGateDecision::Ignored
    );
    assert_eq!(state.phase(), WorkerAttemptPhase::Starting);
    assert!(!state.is_real_running());

    assert_eq!(
        state
            .apply_start(StartObservation::TurnStarted {
                thread_id: "thread-phase4-001".into(),
                turn_id: "turn-phase4-001".into(),
                status: TurnStartedStatus::InProgress,
                observed_at: "2026-08-26T12:00:00Z".into(),
            })
            .unwrap(),
        StartGateDecision::Applied(WorkerAttemptPhase::Executing)
    );
    assert!(state.is_real_running());
    assert!(state.digest().starts_with("attempt-state:sha256:"));

    let mut replay = WorkerAttemptState::new(packet(1, 41)).unwrap();
    start_exact_turn(&mut replay);
    assert_eq!(state, replay);
    assert_eq!(state.digest(), replay.digest());
    assert_eq!(replay.attempt_started_at(), Some("2026-08-26T12:00:00Z"));
    assert_eq!(replay.attempt_deadline_at(), Some("2026-08-26T12:15:00Z"));

    assert_eq!(
        {
            let mut unsafe_id = WorkerAttemptState::new(packet(1, 41)).unwrap();
            unsafe_id.begin_dispatch().unwrap();
            unsafe_id.apply_start(StartObservation::ThreadStartAccepted {
                thread_id: "Bearer-leaked-token".into(),
            })
        },
        Err(WorkerAttemptError::ForbiddenContent)
    );
}

#[test]
fn capacity_wait_and_retries_cannot_extend_the_immutable_task_deadline() {
    let budget = worker_budget();
    let mut state = WorkerAttemptState::new(packet(1, 41)).expect("claimed attempt");
    state.begin_dispatch().expect("dispatch");
    state
        .apply_start(StartObservation::ThreadStartAccepted {
            thread_id: "thread-phase4-001".into(),
        })
        .expect("thread accepted");
    state
        .apply_start(StartObservation::TurnStartAccepted {
            thread_id: "thread-phase4-001".into(),
            turn_id: "turn-phase4-001".into(),
        })
        .expect("turn accepted");
    state
        .apply_start(StartObservation::TurnStarted {
            thread_id: "thread-phase4-001".into(),
            turn_id: "turn-phase4-001".into(),
            status: TurnStartedStatus::InProgress,
            observed_at: "2026-08-26T12:20:00Z".into(),
        })
        .expect("exact start");

    assert_eq!(state.attempt_started_at(), Some("2026-08-26T12:20:00Z"));
    assert_eq!(state.attempt_deadline_at(), Some("2026-08-26T12:30:00Z"));
    assert_eq!(budget.deadline_at(), "2026-08-26T12:30:00Z");

    let progress = MeaningfulProgress::new(
        &state,
        MeaningfulProgressKind::VerifiedWorkChange,
        "2026-08-26T12:29:20Z",
        EVIDENCE_A,
    )
    .expect("progress before the immutable task deadline");
    assert_eq!(
        classify_attempt_stall(
            &state,
            &budget,
            &progress,
            &AttemptWatchdogObservation::new(
                "2026-08-26T12:29:30Z",
                60,
                ProcessObservation::Alive,
                TurnActivityObservation::ExactInProgress {
                    thread_id: "thread-phase4-001".into(),
                    turn_id: "turn-phase4-001".into(),
                },
                ReconciliationState::NotAttempted,
            )
            .expect("watchdog"),
        ),
        Ok(StallClassification::Healthy)
    );
    assert_eq!(
        classify_attempt_stall(
            &state,
            &budget,
            &progress,
            &AttemptWatchdogObservation::new(
                "2026-08-26T12:30:00Z",
                60,
                ProcessObservation::Alive,
                TurnActivityObservation::ExactInProgress {
                    thread_id: "thread-phase4-001".into(),
                    turn_id: "turn-phase4-001".into(),
                },
                ReconciliationState::NotAttempted,
            )
            .expect("watchdog"),
        ),
        Ok(StallClassification::ReconcileFirst(
            StallReason::DeadlineExceeded
        ))
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn meaningful_progress_and_stalls_are_closed_and_reconcile_first() {
    let budget = worker_budget();
    let mut starting = WorkerAttemptState::new(packet(1, 41)).unwrap();
    starting.begin_dispatch().unwrap();
    starting
        .apply_start(StartObservation::ThreadStartAccepted {
            thread_id: "thread-phase4-001".into(),
        })
        .unwrap();
    let starting_progress = MeaningfulProgress::new(
        &starting,
        MeaningfulProgressKind::ExactLifecycleNotification,
        "2026-08-26T12:00:00Z",
        EVIDENCE_A,
    )
    .unwrap();
    assert_eq!(
        classify_attempt_stall(
            &starting,
            &budget,
            &starting_progress,
            &AttemptWatchdogObservation::new(
                "2026-08-26T12:20:00Z",
                60,
                ProcessObservation::Alive,
                TurnActivityObservation::Unknown,
                ReconciliationState::NotAttempted,
            )
            .unwrap(),
        ),
        Ok(StallClassification::Healthy)
    );

    let mut executing = WorkerAttemptState::new(packet(1, 41)).unwrap();
    start_exact_turn(&mut executing);
    let progress = MeaningfulProgress::new(
        &executing,
        MeaningfulProgressKind::VerifiedWorkChange,
        "2026-08-26T12:00:00Z",
        EVIDENCE_A,
    )
    .unwrap();
    let same_progress = MeaningfulProgress::new(
        &executing,
        MeaningfulProgressKind::VerifiedWorkChange,
        "2026-08-26T12:00:00Z",
        EVIDENCE_A,
    )
    .unwrap();
    assert_eq!(progress.digest(), same_progress.digest());
    assert_eq!(
        classify_attempt_stall(
            &executing,
            &budget,
            &progress,
            &AttemptWatchdogObservation::new(
                "2026-08-26T12:02:00Z",
                60,
                ProcessObservation::Alive,
                TurnActivityObservation::ExactInProgress {
                    thread_id: "thread-phase4-001".into(),
                    turn_id: "turn-phase4-001".into(),
                },
                ReconciliationState::NotAttempted,
            )
            .unwrap(),
        ),
        Ok(StallClassification::ReconcileFirst(
            StallReason::HeartbeatTimeoutActiveTurn
        ))
    );
    assert_eq!(
        classify_attempt_stall(
            &executing,
            &budget,
            &progress,
            &AttemptWatchdogObservation::new(
                "2026-08-26T12:02:00Z",
                60,
                ProcessObservation::Alive,
                TurnActivityObservation::ExactInProgress {
                    thread_id: "thread-phase4-001".into(),
                    turn_id: "turn-wrong".into(),
                },
                ReconciliationState::NotAttempted,
            )
            .unwrap(),
        ),
        Err(WorkerAttemptError::InvalidPhase)
    );
    assert_eq!(
        classify_attempt_stall(
            &executing,
            &budget,
            &progress,
            &AttemptWatchdogObservation::new(
                "2026-08-26T12:00:30Z",
                60,
                ProcessObservation::Exited,
                TurnActivityObservation::ExactInProgress {
                    thread_id: "thread-phase4-001".into(),
                    turn_id: "turn-phase4-001".into(),
                },
                ReconciliationState::NotAttempted,
            )
            .unwrap(),
        ),
        Ok(StallClassification::ReconcileFirst(
            StallReason::ProcessExitWithoutTerminal
        ))
    );
    assert_eq!(
        classify_attempt_stall(
            &executing,
            &budget,
            &progress,
            &AttemptWatchdogObservation::new(
                "2026-08-26T12:31:00Z",
                60,
                ProcessObservation::Alive,
                TurnActivityObservation::Unknown,
                ReconciliationState::NotAttempted,
            )
            .unwrap(),
        ),
        Ok(StallClassification::ReconcileFirst(
            StallReason::DeadlineExceeded
        ))
    );
    assert_eq!(
        classify_attempt_stall(
            &executing,
            &budget,
            &progress,
            &AttemptWatchdogObservation::new(
                "2026-08-26T12:00:30Z",
                60,
                ProcessObservation::Alive,
                TurnActivityObservation::ExactInProgress {
                    thread_id: "thread-phase4-001".into(),
                    turn_id: "turn-phase4-001".into(),
                },
                ReconciliationState::Exhausted,
            )
            .unwrap(),
        ),
        Ok(StallClassification::Stalled(
            StallReason::ReconciliationExhausted
        ))
    );
}

#[test]
fn restart_reconciles_exact_ids_and_retry_never_exceeds_two_repairs() {
    let budget = worker_budget();
    let claimed = WorkerAttemptState::new(packet(1, 41)).unwrap();
    assert_eq!(
        restart_reconciliation_decision(&claimed),
        Ok(RestartDecision::DispatchUnsentAttempt)
    );

    let mut uncertain = claimed.clone();
    uncertain.begin_dispatch().unwrap();
    assert_eq!(
        restart_reconciliation_decision(&uncertain),
        Ok(RestartDecision::BlockUncertainDispatch)
    );

    let mut executing = claimed;
    start_exact_turn(&mut executing);
    assert_eq!(
        restart_reconciliation_decision(&executing),
        Ok(RestartDecision::ReadResumeExactTurn {
            thread_id: "thread-phase4-001".into(),
            turn_id: "turn-phase4-001".into(),
        })
    );
    assert_eq!(
        decide_repair_retry(&executing, &budget, true),
        Ok(RetryDecision::ReconcileExactTurn)
    );

    executing
        .record_terminal(
            "thread-phase4-001",
            "turn-phase4-001",
            WorkerTerminal::Failed,
            EVIDENCE_A,
        )
        .unwrap();
    assert_eq!(
        restart_reconciliation_decision(&executing),
        Ok(RestartDecision::PreserveTerminal)
    );
    assert_eq!(
        decide_repair_retry(&executing, &budget, true),
        Ok(RetryDecision::Retry { next_attempt: 2 })
    );
    assert_eq!(
        decide_repair_retry(&executing, &budget, false),
        Ok(RetryDecision::BlockedNonRepairable)
    );

    let mut second = WorkerAttemptState::new(packet(2, 42)).unwrap();
    start_exact_turn(&mut second);
    second
        .record_terminal(
            "thread-phase4-001",
            "turn-phase4-001",
            WorkerTerminal::Interrupted,
            EVIDENCE_A,
        )
        .unwrap();
    assert_eq!(
        decide_repair_retry(&second, &budget, true),
        Ok(RetryDecision::Retry { next_attempt: 3 })
    );

    let mut third = WorkerAttemptState::new(packet(3, 43)).unwrap();
    start_exact_turn(&mut third);
    third
        .record_terminal(
            "thread-phase4-001",
            "turn-phase4-001",
            WorkerTerminal::Failed,
            EVIDENCE_A,
        )
        .unwrap();
    assert_eq!(
        decide_repair_retry(&third, &budget, true),
        Ok(RetryDecision::BlockedRetryBudgetExhausted)
    );
}
