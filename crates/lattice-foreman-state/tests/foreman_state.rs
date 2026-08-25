use lattice_foreman_state::{
    Confidence, DashboardIndex, DependencyBinding, DependencyContinuationState,
    EpistemicReferences, ForemanCheckpointIntent, ForemanSnapshot, ForemanState, LiveWorktree,
    RefreshTrigger, SnapshotError, SoleForemanBinding, WatchdogFinding, is_exact_next_generation,
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
        "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        generation,
    )
    .unwrap()
}

fn task106_dependency_snapshot(
    binding: &DependencyBinding,
    state: ForemanState,
    head: &str,
    generation: u64,
) -> ForemanSnapshot {
    let (blocker, evidence) = if state == ForemanState::Blocked {
        (
            Some(binding.as_blocker_ref().to_owned()),
            binding.evidence_ref().to_owned(),
        )
    } else {
        (
            None,
            format!(
                "evidence:sha256:{}",
                if generation == 2 { "b" } else { "c" }.repeat(64)
            ),
        )
    };
    ForemanSnapshot::new(
        SoleForemanBinding::WORKER,
        SoleForemanBinding::THREAD,
        SoleForemanBinding::TASK,
        "feature/task-106-dependency-continuation",
        "lattice-worktrees/task-106-dependency-continuation",
        head,
        state,
        blocker,
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        evidence,
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
fn sole_foreman_binding_constructs_and_verifies_only_the_fixed_identity() {
    let observation = SoleForemanBinding::observe_git(
        "feature/task-105-durable-foreman-runtime",
        "lattice-worktrees/task-105-durable-foreman-runtime",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .expect("observation");
    let intent = ForemanCheckpointIntent::new(
        "checkpoint-fixed",
        1,
        "2026-08-25T00:00:01Z",
        ForemanState::Active,
        None,
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("intent");
    let fixed = observation
        .bind(
            &intent,
            "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        )
        .expect("snapshot");
    assert!(SoleForemanBinding::matches(&fixed));
    assert!(!SoleForemanBinding::matches(&snapshot(
        "worker-1",
        1,
        ForemanState::Active,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
    )));
}

#[test]
fn checkpoint_intent_is_closed_lowercase_and_state_compatible() {
    let valid = ForemanCheckpointIntent::new(
        "checkpoint-1",
        1,
        "2026-08-25T00:00:01Z",
        ForemanState::Blocked,
        Some("TASK-094".to_owned()),
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("valid checkpoint");
    assert_eq!(valid.checkpoint_id(), "checkpoint-1");
    assert_eq!(valid.generation(), 1);
    assert_eq!(valid.blocker_ref(), Some("TASK-094"));

    let long_id = "a".repeat(65);
    for invalid_id in ["", "-leading", "contains space", long_id.as_str()] {
        assert_eq!(
            ForemanCheckpointIntent::new(
                invalid_id,
                1,
                "2026-08-25T00:00:01Z",
                ForemanState::Active,
                None,
                "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            Err(SnapshotError::MalformedReference)
        );
    }
    assert_eq!(
        ForemanCheckpointIntent::new(
            "checkpoint-2",
            2,
            "2026-08-25T00:00:02Z",
            ForemanState::Active,
            None,
            "heartbeat:sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
        Err(SnapshotError::MalformedReference)
    );
    assert_eq!(
        ForemanCheckpointIntent::new(
            "checkpoint-3",
            3,
            "2026-08-25T00:00:03Z",
            ForemanState::Blocked,
            None,
            "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
        Err(SnapshotError::MissingBlocker)
    );
    for state in [ForemanState::Active, ForemanState::Completed] {
        assert_eq!(
            ForemanCheckpointIntent::new(
                "checkpoint-blocker",
                1,
                "2026-08-25T00:00:01Z",
                state,
                Some("TASK-094".to_owned()),
                "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            Err(SnapshotError::UnexpectedBlocker)
        );
    }
    for invalid_time in [
        "2026-99-99T00:00:00Z",
        "2026-08-25T24:00:00Z",
        "2026-08-25T00:00:01+00:00",
        "2026-08-25T00:00:01.000Z",
        "2026-08-25T00:00:01z",
    ] {
        assert_eq!(
            ForemanCheckpointIntent::new(
                "checkpoint-time",
                1,
                invalid_time,
                ForemanState::Active,
                None,
                "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            Err(SnapshotError::MalformedReference)
        );
    }
}

#[test]
fn runtime_projection_reports_closed_counts_and_next_action() {
    let active = reconstruct([snapshot(
        "sole-foreman-v1",
        1,
        ForemanState::Active,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
    )])
    .expect("active");
    assert_eq!(active.active().len(), 1);
    assert_eq!(active.completed().len(), 0);
    assert_eq!(active.latest_generation(), 1);
    assert_eq!(active.runtime_next_action(), "CONTINUE");

    let blocked = reconstruct([snapshot(
        "sole-foreman-v1",
        1,
        ForemanState::Blocked,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        Some("TASK-094"),
    )])
    .expect("blocked");
    assert_eq!(blocked.runtime_next_action(), "RESOLVE_BLOCKERS");

    let completed = reconstruct([snapshot(
        "sole-foreman-v1",
        1,
        ForemanState::Completed,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        None,
    )])
    .expect("completed");
    assert_eq!(completed.completed().len(), 1);
    assert_eq!(completed.runtime_next_action(), "ALL_COMPLETED");

    let empty = reconstruct([]).expect("empty");
    assert_eq!(empty.latest_generation(), 0);
    assert_eq!(empty.runtime_next_action(), "NO_DURABLE_SNAPSHOT");
}

#[test]
fn dependency_binding_is_canonical_bounded_and_derives_closed_fields() {
    let binding = DependencyBinding::new(
        "TASK-106",
        "TASK-107",
        "TASK-107-WORKTREE",
        "lattice/task-107",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "COMPLETE_DEPENDENCY",
    )
    .expect("dependency binding");
    assert_eq!(
        binding.as_blocker_ref(),
        "dependency:v1:TASK-106:TASK-107:TASK-107-WORKTREE:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(binding.parent_task_id(), "TASK-106");
    assert_eq!(binding.dependency_task_id(), "TASK-107");
    assert_eq!(binding.dependency_worktree_id(), "TASK-107-WORKTREE");
    assert_eq!(binding.dependency_branch(), "lattice/task-107");
    assert_eq!(
        binding.base_sha(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(binding.next_action(), "COMPLETE_DEPENDENCY");
    assert!(binding.evidence_ref().starts_with("evidence:sha256:"));
    assert_eq!(
        DependencyBinding::from_blocker_ref(binding.as_blocker_ref()),
        Ok(Some(binding.clone()))
    );
    assert!(
        ForemanCheckpointIntent::new(
            "dependency-checkpoint",
            1,
            "2026-08-25T00:00:00Z",
            ForemanState::Blocked,
            Some(binding.as_blocker_ref().to_owned()),
            format!("heartbeat:sha256:{}", "a".repeat(64)),
            binding.evidence_ref(),
        )
        .is_ok()
    );
    assert_eq!(
        ForemanCheckpointIntent::new(
            "dependency-checkpoint-wrong-evidence",
            1,
            "2026-08-25T00:00:00Z",
            ForemanState::Blocked,
            Some(binding.as_blocker_ref().to_owned()),
            format!("heartbeat:sha256:{}", "a".repeat(64)),
            format!("evidence:sha256:{}", "b".repeat(64)),
        ),
        Err(SnapshotError::MalformedReference)
    );
    assert_eq!(DependencyBinding::from_blocker_ref("TASK-094"), Ok(None));
    assert_eq!(
        DependencyBinding::from_blocker_ref("dependency:vpn"),
        Ok(None)
    );
    assert_eq!(
        DependencyBinding::from_blocker_ref(
            "dependency:v2:TASK-106:TASK-107:TASK-107-WORKTREE:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ),
        Ok(None)
    );
    assert_eq!(
        DependencyBinding::from_blocker_ref("dependency:v1:waiting-on-docker"),
        Ok(None)
    );
}

#[test]
fn dependency_binding_rejects_invalid_identity_base_branch_and_action() {
    for invalid in [
        (
            "task-106",
            "TASK-107",
            "TASK-107-WORKTREE",
            "lattice/task-107",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "COMPLETE_DEPENDENCY",
        ),
        (
            "TASK-_06",
            "TASK-107",
            "TASK-107-WORKTREE",
            "lattice/task-107",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "COMPLETE_DEPENDENCY",
        ),
        (
            "TASK-106",
            "TASK-_07",
            "TASK-107-WORKTREE",
            "lattice/task-_07",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "COMPLETE_DEPENDENCY",
        ),
        (
            "TASK-106",
            "TASK-106",
            "TASK-106-WORKTREE",
            "lattice/task-106",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "COMPLETE_DEPENDENCY",
        ),
        (
            "TASK-106",
            "TASK-107",
            "_TASK-107-WORKTREE",
            "lattice/task-107",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "COMPLETE_DEPENDENCY",
        ),
        (
            "TASK-106",
            "TASK-107",
            "../escape",
            "lattice/task-107",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "COMPLETE_DEPENDENCY",
        ),
        (
            "TASK-106",
            "TASK-107",
            "TASK-107-WORKTREE",
            "feature/task-107",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "COMPLETE_DEPENDENCY",
        ),
        (
            "TASK-106",
            "TASK-107",
            "TASK-107-WORKTREE",
            "lattice/task-107",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "COMPLETE_DEPENDENCY",
        ),
        (
            "TASK-106",
            "TASK-107",
            "TASK-107-WORKTREE",
            "lattice/task-107",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "CONTINUE_PARENT",
        ),
    ] {
        assert_eq!(
            DependencyBinding::new(
                invalid.0, invalid.1, invalid.2, invalid.3, invalid.4, invalid.5
            ),
            Err(SnapshotError::MalformedReference)
        );
    }
}

#[test]
fn structured_blocker_base_must_equal_the_observed_parent_head() {
    let binding = DependencyBinding::new(
        "TASK-106",
        "TASK-107",
        "TASK-107-WORKTREE",
        "lattice/task-107",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "COMPLETE_DEPENDENCY",
    )
    .unwrap();
    let snapshot = ForemanSnapshot::new(
        SoleForemanBinding::WORKER,
        SoleForemanBinding::THREAD,
        SoleForemanBinding::TASK,
        "feature/task-106-dependency-continuation",
        "lattice-worktrees/task-106-dependency-continuation",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ForemanState::Blocked,
        Some(binding.as_blocker_ref().to_owned()),
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        binding.evidence_ref(),
        1,
    )
    .expect("typed snapshot bytes");
    assert_eq!(
        reconstruct([snapshot]),
        Err(SnapshotError::MalformedReference)
    );
}

#[test]
fn fresh_replay_restores_blocked_then_resumed_dependency_next_action() {
    let binding = DependencyBinding::new(
        "TASK-106",
        "TASK-107",
        "TASK-107-WORKTREE",
        "lattice/task-107",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "COMPLETE_DEPENDENCY",
    )
    .unwrap();
    let blocked = task106_dependency_snapshot(
        &binding,
        ForemanState::Blocked,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        1,
    );
    let blocked_projection = reconstruct([blocked.clone()]).unwrap();
    let dependency = blocked_projection.dependency().expect("dependency");
    assert_eq!(dependency.state(), DependencyContinuationState::Blocked);
    assert_eq!(dependency.parent_task_id(), "TASK-106");
    assert_eq!(
        dependency.parent_branch(),
        "feature/task-106-dependency-continuation"
    );
    assert_eq!(
        dependency.parent_worktree(),
        "lattice-worktrees/task-106-dependency-continuation"
    );
    assert_eq!(dependency.dependency_task_id(), "TASK-107");
    assert_eq!(dependency.dependency_branch(), "lattice/task-107");
    assert_eq!(dependency.dependency_worktree_id(), "TASK-107-WORKTREE");
    assert_eq!(
        dependency.base_sha(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(dependency.next_action(), "COMPLETE_DEPENDENCY");

    let completed_without_resume = task106_dependency_snapshot(
        &binding,
        ForemanState::Completed,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );
    assert_eq!(
        reconstruct([blocked.clone(), completed_without_resume]),
        Err(SnapshotError::MalformedReference)
    );

    let resumed = task106_dependency_snapshot(
        &binding,
        ForemanState::Active,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        2,
    );
    let resumed_projection = reconstruct([blocked.clone(), resumed.clone()]).unwrap();
    let dependency = resumed_projection.dependency().expect("dependency");
    assert_eq!(dependency.state(), DependencyContinuationState::Resumed);
    assert_eq!(dependency.next_action(), "CONTINUE_PARENT");

    let completed_after_resume = task106_dependency_snapshot(
        &binding,
        ForemanState::Completed,
        "cccccccccccccccccccccccccccccccccccccccc",
        3,
    );
    let completed_projection = reconstruct([blocked, resumed, completed_after_resume]).unwrap();
    assert_eq!(completed_projection.completed().len(), 1);
    assert_eq!(
        completed_projection
            .dependency()
            .expect("dependency")
            .state(),
        DependencyContinuationState::Resumed
    );
}

#[test]
fn numeric_legacy_dependency_string_replays_without_a_typed_projection() {
    for blocker in [
        "dependency:v2:TASK-106:TASK-107:TASK-107-WORKTREE:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "dependency:v1:waiting-on-docker",
    ] {
        let legacy = snapshot(
            SoleForemanBinding::WORKER,
            1,
            ForemanState::Blocked,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some(blocker),
        );
        let projection = reconstruct([legacy]).expect("legacy replay");
        assert!(projection.dependency().is_none());
        assert_eq!(projection.blocked().len(), 1);
    }
    let collision = DependencyBinding::new(
        "TASK-106",
        "TASK-107",
        "TASK-107-WORKTREE",
        "lattice/task-107",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "COMPLETE_DEPENDENCY",
    )
    .unwrap();
    let legacy_collision = snapshot(
        SoleForemanBinding::WORKER,
        1,
        ForemanState::Blocked,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        Some(collision.as_blocker_ref()),
    );
    let projection = reconstruct([legacy_collision]).expect("canonical legacy collision replay");
    assert!(projection.dependency().is_none());
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
