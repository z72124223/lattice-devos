use lattice_foreman_state::{
    DashboardIndex, ForemanSnapshot, ForemanState, LiveWorktree, SnapshotError, WatchdogFinding,
    reconstruct, watchdog,
};

fn snapshot(
    worker: &str,
    generation: u64,
    state: ForemanState,
    head: &str,
    dependency: Option<&str>,
) -> ForemanSnapshot {
    ForemanSnapshot::new(
        worker,
        "thread-079",
        "TASK-079",
        "feature/task-079-durable-foreman-state",
        "worktree-task-079",
        head,
        state,
        dependency.map(str::to_owned),
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        generation,
    )
    .unwrap()
}

#[test]
fn fresh_reader_reconstructs_active_blocked_and_next_action() {
    let active = snapshot(
        "worker-a",
        2,
        ForemanState::Active,
        "a".repeat(40).as_str(),
        None,
    );
    let blocked = snapshot(
        "worker-b",
        3,
        ForemanState::Blocked,
        "b".repeat(40).as_str(),
        Some("TASK-078-delivery-evidence"),
    );

    let projection = reconstruct([active, blocked]).unwrap();
    assert_eq!(projection.active().len(), 1);
    assert_eq!(projection.blocked().len(), 1);
    assert!(!projection.blocked()[0].archive_ready());
    assert_eq!(
        projection.next_action(),
        "unblock worker-b: TASK-078-delivery-evidence"
    );
}

#[test]
fn replay_rejects_duplicate_identity_and_generation_rollback() {
    let latest = snapshot(
        "worker-a",
        2,
        ForemanState::Active,
        "a".repeat(40).as_str(),
        None,
    );
    let stale = snapshot(
        "worker-a",
        1,
        ForemanState::Active,
        "a".repeat(40).as_str(),
        None,
    );
    assert_eq!(
        reconstruct([latest, stale]),
        Err(SnapshotError::GenerationRollback)
    );

    let other_thread = ForemanSnapshot::new(
        "worker-a",
        "thread-other",
        "TASK-079",
        "feature/task-079-durable-foreman-state",
        "worktree-task-079",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ForemanState::Active,
        None,
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    )
    .unwrap();
    assert_eq!(
        reconstruct([
            snapshot(
                "worker-a",
                2,
                ForemanState::Active,
                "a".repeat(40).as_str(),
                None
            ),
            other_thread
        ]),
        Err(SnapshotError::DuplicateWorkerIdentity)
    );
}

#[test]
fn transcript_and_secret_like_fields_are_rejected_without_rejecting_task_identifiers() {
    let allowed = ForemanSnapshot::new(
        "worker-a",
        "thread-079",
        "TASK-079",
        "feature/task-079-durable-foreman-state",
        "lattice-worktrees/task-079-durable-foreman-state",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ForemanState::Active,
        None,
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        1,
    );
    assert!(allowed.is_ok());
    assert_eq!(
        ForemanSnapshot::new(
            "worker-a",
            "thread-079",
            "TASK-079",
            "feature/task-079-durable-foreman-state",
            "worktree-task-079",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ForemanState::Active,
            None,
            "full chat: sk-live-secret",
            "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
        ),
        Err(SnapshotError::ForbiddenContent),
    );
    for token in ["sk-live-secret", "SK-live-secret"] {
        assert_eq!(
            ForemanSnapshot::new(
                "worker-a",
                "thread-079",
                "TASK-079",
                "feature/task-079-durable-foreman-state",
                "worktree-task-079",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ForemanState::Active,
                None,
                token,
                "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                1,
            ),
            Err(SnapshotError::ForbiddenContent),
        );
    }
    assert_eq!(
        ForemanSnapshot::new(
            "worker-a",
            "thread-079",
            "TASK-079",
            "feature/task-079-durable-foreman-state",
            "worktree-task-079",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ForemanState::Active,
            None,
            "s𝕜-live-secret",
            "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
        ),
        Err(SnapshotError::MalformedReference),
    );
}

#[test]
fn watchdog_detects_all_missed_heartbeats_old_head_and_dashboard_drift() {
    let item = snapshot(
        "worker-a",
        1,
        ForemanState::Active,
        "a".repeat(40).as_str(),
        None,
    );
    let findings = watchdog(
        &[item],
        &DashboardIndex::new(
            "2026-08-20T00:00:00Z",
            "feature/task-079-durable-foreman-state",
            "cccccccccccccccccccccccccccccccccccccccc",
            "ACTIVE",
        )
        .unwrap(),
        &[LiveWorktree::new(
            "worker-a",
            "feature/task-079-durable-foreman-state",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            false,
        )
        .unwrap()],
    )
    .unwrap();
    assert!(findings.contains(&WatchdogFinding::AllWorkersMissedHeartbeat));
    assert!(findings.contains(&WatchdogFinding::OldHead {
        worker: "worker-a".into()
    }));
    assert!(findings.contains(&WatchdogFinding::DashboardDrift));
}
