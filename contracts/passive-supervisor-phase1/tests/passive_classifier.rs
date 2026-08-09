use lattice_passive_supervisor_contract::{
    DispatchEvidence, EvidenceClassification, EvidenceFault, NextObservation, PassiveSnapshot,
    TaskEvidence, TaskLifecycle, TerminalOutcome, VerificationEvidence, VerificationState,
    WorkerActivity, WorkerEvidence, evaluate,
};

const TASK_REF: &str = "task-ref-001";
const ATTEMPT_REF: &str = "attempt-001";

fn task(lifecycle: TaskLifecycle, observed_at: u64) -> TaskEvidence<'static> {
    TaskEvidence {
        task_ref: TASK_REF,
        attempt_ref: ATTEMPT_REF,
        revision: 7,
        observed_at,
        lifecycle,
    }
}

fn dispatch(observed_at: u64) -> DispatchEvidence<'static> {
    DispatchEvidence {
        task_ref: TASK_REF,
        attempt_ref: ATTEMPT_REF,
        worker_session_ref: "session-001",
        observed_at,
    }
}

fn worker(activity: WorkerActivity, observed_at: u64) -> WorkerEvidence<'static> {
    WorkerEvidence {
        task_ref: TASK_REF,
        attempt_ref: ATTEMPT_REF,
        worker_session_ref: "session-001",
        observed_at,
        activity,
    }
}

fn verification(
    state: VerificationState,
    evidence_ref: Option<&'static str>,
    observed_at: u64,
) -> VerificationEvidence<'static> {
    VerificationEvidence {
        task_ref: TASK_REF,
        attempt_ref: ATTEMPT_REF,
        observed_at,
        state,
        evidence_ref,
    }
}

#[test]
fn no_task_sleeps_without_manufacturing_work() {
    let evaluation = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: None,
        dispatch: None,
        worker: None,
        verification: None,
    })
    .expect("empty authoritative view is valid");

    assert_eq!(evaluation.classification, EvidenceClassification::Sleeping);
    assert_eq!(evaluation.next, NextObservation::Sleep);
    assert_eq!(evaluation.reason_code, "SUPERVISOR_NO_TASK");
}

#[test]
fn pending_task_waits_for_dispatch_evidence() {
    let evaluation = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Pending, 98)),
        dispatch: None,
        worker: None,
        verification: None,
    })
    .expect("pending evidence");

    assert_eq!(evaluation.classification, EvidenceClassification::Pending);
    assert_eq!(evaluation.next, NextObservation::ObserveDispatch);
}

#[test]
fn stale_pending_task_is_stalled_without_becoming_a_task_transition() {
    let evaluation = evaluate(&PassiveSnapshot {
        observed_at: 111,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Pending, 100)),
        dispatch: None,
        worker: None,
        verification: None,
    })
    .expect("stale pending evidence is classifiable");

    assert_eq!(evaluation.classification, EvidenceClassification::Stalled);
    assert_eq!(evaluation.task_truth, Some(TaskLifecycle::Pending));
    assert_eq!(evaluation.next, NextObservation::ObserveDispatch);
    assert_eq!(evaluation.reason_code, "SUPERVISOR_PENDING_EVIDENCE_STALE");
}

#[test]
fn fresh_bound_activity_is_running() {
    let evaluation = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Running, 95)),
        dispatch: Some(dispatch(96)),
        worker: Some(worker(WorkerActivity::Active, 99)),
        verification: None,
    })
    .expect("fresh running evidence");

    assert_eq!(evaluation.classification, EvidenceClassification::Running);
    assert_eq!(evaluation.next, NextObservation::Wait);
}

#[test]
fn stale_activity_is_only_a_passive_stalled_classification() {
    let evaluation = evaluate(&PassiveSnapshot {
        observed_at: 111,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Running, 90)),
        dispatch: Some(dispatch(91)),
        worker: Some(worker(WorkerActivity::WaitingExternal, 100)),
        verification: None,
    })
    .expect("stale evidence is classifiable");

    assert_eq!(evaluation.classification, EvidenceClassification::Stalled);
    assert_eq!(evaluation.task_truth, Some(TaskLifecycle::Running));
    assert_eq!(evaluation.next, NextObservation::ObserveWorkerSession);
    assert_eq!(evaluation.reason_code, "SUPERVISOR_WORKER_ACTIVITY_STALE");
}

#[test]
fn terminal_task_reads_verification_before_recommending_persistence() {
    let without_verification = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(
            TaskLifecycle::Terminal(TerminalOutcome::Completed),
            99,
        )),
        dispatch: Some(dispatch(90)),
        worker: Some(worker(WorkerActivity::Terminal, 98)),
        verification: None,
    })
    .expect("terminal task without verification");
    assert_eq!(
        without_verification.classification,
        EvidenceClassification::Terminal
    );
    assert_eq!(without_verification.next, NextObservation::ReadVerification);

    let verified = evaluate(&PassiveSnapshot {
        observed_at: 101,
        stale_after: 10,
        task: Some(task(
            TaskLifecycle::Terminal(TerminalOutcome::Completed),
            99,
        )),
        dispatch: Some(dispatch(90)),
        worker: Some(worker(WorkerActivity::Terminal, 98)),
        verification: Some(verification(
            VerificationState::Passed,
            Some("verification-receipt-001"),
            100,
        )),
    })
    .expect("terminal task with verification");
    assert_eq!(verified.classification, EvidenceClassification::Terminal);
    assert_eq!(verified.next, NextObservation::RecommendDurableProjection);
}

#[test]
fn cross_task_and_future_evidence_fail_closed() {
    let wrong_task_worker = WorkerEvidence {
        task_ref: "other-task",
        ..worker(WorkerActivity::Active, 99)
    };
    let cross_task = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Running, 95)),
        dispatch: Some(dispatch(96)),
        worker: Some(wrong_task_worker),
        verification: None,
    });
    assert_eq!(cross_task, Err(EvidenceFault::CrossTaskEvidence));

    let future = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Running, 101)),
        dispatch: None,
        worker: None,
        verification: None,
    });
    assert_eq!(future, Err(EvidenceFault::FutureObservation));
}

#[test]
fn terminal_worker_before_terminal_task_is_stalled_not_completed() {
    let evaluation = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Running, 95)),
        dispatch: Some(dispatch(96)),
        worker: Some(worker(WorkerActivity::Terminal, 99)),
        verification: None,
    })
    .expect("worker can finish before task truth catches up");

    assert_eq!(evaluation.classification, EvidenceClassification::Stalled);
    assert_eq!(evaluation.next, NextObservation::ObserveTask);
    assert_eq!(
        evaluation.reason_code,
        "SUPERVISOR_TASK_TERMINAL_NOT_OBSERVED"
    );
}

#[test]
fn identical_inputs_produce_identical_outputs() {
    let snapshot = PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Running, 95)),
        dispatch: Some(dispatch(96)),
        worker: Some(worker(WorkerActivity::Verifying, 99)),
        verification: Some(verification(VerificationState::Pending, None, 99)),
    };

    assert_eq!(evaluate(&snapshot), evaluate(&snapshot));
}

#[test]
fn missing_truth_impossible_order_and_terminal_conflict_fail_closed() {
    let missing_truth = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: None,
        dispatch: Some(dispatch(96)),
        worker: None,
        verification: None,
    });
    assert_eq!(missing_truth, Err(EvidenceFault::MissingTaskTruth));

    let impossible_order = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(TaskLifecycle::Running, 90)),
        dispatch: Some(dispatch(97)),
        worker: Some(worker(WorkerActivity::Active, 96)),
        verification: None,
    });
    assert_eq!(
        impossible_order,
        Err(EvidenceFault::ImpossibleObservationOrder)
    );

    let terminal_conflict = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(
            TaskLifecycle::Terminal(TerminalOutcome::Completed),
            98,
        )),
        dispatch: Some(dispatch(90)),
        worker: Some(worker(WorkerActivity::Active, 99)),
        verification: None,
    });
    assert_eq!(
        terminal_conflict,
        Err(EvidenceFault::TerminalWorkerConflict)
    );

    let missing_verification_ref = evaluate(&PassiveSnapshot {
        observed_at: 100,
        stale_after: 10,
        task: Some(task(
            TaskLifecycle::Terminal(TerminalOutcome::Completed),
            98,
        )),
        dispatch: Some(dispatch(90)),
        worker: Some(worker(WorkerActivity::Terminal, 99)),
        verification: Some(verification(VerificationState::Passed, None, 100)),
    });
    assert_eq!(
        missing_verification_ref,
        Err(EvidenceFault::MissingVerificationReference)
    );
}

#[test]
fn next_observation_surface_is_passive_only() {
    assert_eq!(
        NextObservation::ALL,
        [
            NextObservation::Sleep,
            NextObservation::ObserveDispatch,
            NextObservation::ObserveTask,
            NextObservation::ObserveWorkerSession,
            NextObservation::ReadVerification,
            NextObservation::RecommendDurableProjection,
            NextObservation::Wait,
        ]
    );
    assert_eq!(NextObservation::Sleep.as_str(), "SLEEP");
    assert_eq!(
        NextObservation::ObserveDispatch.as_str(),
        "OBSERVE_DISPATCH"
    );
    assert_eq!(NextObservation::ObserveTask.as_str(), "OBSERVE_TASK");
    assert_eq!(
        NextObservation::ObserveWorkerSession.as_str(),
        "OBSERVE_WORKER_SESSION"
    );
    assert_eq!(
        NextObservation::ReadVerification.as_str(),
        "READ_VERIFICATION"
    );
    assert_eq!(
        NextObservation::RecommendDurableProjection.as_str(),
        "RECOMMEND_DURABLE_PROJECTION"
    );
    assert_eq!(NextObservation::Wait.as_str(), "WAIT");
}
