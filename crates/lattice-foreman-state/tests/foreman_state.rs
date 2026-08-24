use lattice_foreman_state::{
    Confidence, DashboardIndex, EpistemicReferences, ForemanSnapshot, ForemanState, LiveWorktree,
    RefreshTrigger, SnapshotError, WatchdogFinding, is_exact_next_generation, reconstruct,
    watchdog,
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
        "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        generation,
    )
    .unwrap()
}

#[test]
fn fresh_reader_reconstructs_active_blocked_and_next_action() {
    let active = snapshot(
        "worker-a",
        1,
        ForemanState::Active,
        "a".repeat(40).as_str(),
        None,
    );
    let blocked = snapshot(
        "worker-b",
        1,
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
        "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    )
    .unwrap();
    assert_eq!(
        reconstruct([
            snapshot(
                "worker-a",
                1,
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
fn replay_rejects_generation_gap() {
    let first = snapshot(
        "worker-1",
        1,
        ForemanState::Active,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
    );
    let skipped = snapshot(
        "worker-1",
        3,
        ForemanState::Completed,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
    );

    assert_eq!(
        reconstruct([first, skipped]),
        Err(SnapshotError::GenerationRollback)
    );
}

#[test]
fn replay_rejects_generation_other_than_one_for_new_identity() {
    assert_eq!(
        reconstruct([snapshot(
            "worker-1",
            2,
            ForemanState::Active,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            None,
        )]),
        Err(SnapshotError::GenerationRollback)
    );
}

#[test]
fn exact_generation_never_wraps_after_u64_max() {
    assert!(!is_exact_next_generation(Some(u64::MAX), 1));
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
        "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
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
            "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
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
                "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
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
            "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            1,
        ),
        Err(SnapshotError::MalformedReference),
    );
}

#[test]
fn epistemic_references_are_expiring_pointers_not_authoritative_hypotheses() {
    let references = EpistemicReferences::new(
        vec!["fact:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()],
        vec!["hypothesis:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()],
        Confidence::Unknown,
        vec!["unknown:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into()],
        vec!["evidence:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into()],
        vec!["counterevidence:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into()],
        "2026-08-21T00:00:00Z",
        "2026-08-22T00:00:00Z",
        RefreshTrigger::Expiry,
        "decision:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "probe:sha256:1111111111111111111111111111111111111111111111111111111111111111",
        "falsifier:sha256:2222222222222222222222222222222222222222222222222222222222222222",
    )
    .unwrap();
    let snapshot = snapshot(
        "worker-a",
        1,
        ForemanState::Active,
        "a".repeat(40).as_str(),
        None,
    )
    .with_epistemic(references)
    .unwrap();
    assert_eq!(snapshot.state(), ForemanState::Active);
    assert_eq!(
        snapshot.epistemic().unwrap().schema(),
        "lattice.foreman-epistemic/1.0"
    );
    assert_eq!(
        snapshot.epistemic().unwrap().confidence(),
        Confidence::Unknown
    );
    assert_eq!(snapshot.epistemic().unwrap().observed_facts().len(), 1);
    assert_eq!(snapshot.epistemic().unwrap().hypotheses().len(), 1);
    assert_eq!(snapshot.epistemic().unwrap().unknowns().len(), 1);
    assert_eq!(snapshot.epistemic().unwrap().evidence().len(), 1);
    assert_eq!(snapshot.epistemic().unwrap().counterevidence().len(), 1);
    assert_eq!(
        snapshot.epistemic().unwrap().checked_at(),
        "2026-08-21T00:00:00Z"
    );
    assert_eq!(
        snapshot.epistemic().unwrap().expires_at(),
        "2026-08-22T00:00:00Z"
    );
    assert_eq!(
        snapshot.epistemic().unwrap().refresh_trigger(),
        RefreshTrigger::Expiry
    );
    assert!(
        snapshot
            .epistemic()
            .unwrap()
            .decision()
            .starts_with("decision:sha256:")
    );
    assert!(
        snapshot
            .epistemic()
            .unwrap()
            .probe()
            .starts_with("probe:sha256:")
    );
    assert!(
        snapshot
            .epistemic()
            .unwrap()
            .falsifier()
            .starts_with("falsifier:sha256:")
    );
    assert_eq!(
        EpistemicReferences::new(
            vec![],
            vec!["hypothesis:the worker is done".into()],
            Confidence::High,
            vec![],
            vec![],
            vec![],
            "2026-08-21T00:00:00Z",
            "2026-08-22T00:00:00Z",
            RefreshTrigger::NewEvidence,
            "decision:sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            "probe:sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "falsifier:sha256:2222222222222222222222222222222222222222222222222222222222222222",
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
