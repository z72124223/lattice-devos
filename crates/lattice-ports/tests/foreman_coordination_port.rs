use lattice_contracts::WriterLeaseAuthorityHead;
use lattice_foreman_state::{ForemanCheckpointIntent, ForemanSnapshot, ForemanState, reconstruct};
use lattice_ports::{
    ForemanAppendReceipt, ForemanCheckpointReplay, ForemanCoordinationError,
    ForemanCoordinationErrorKind, ForemanCoordinationPort, ForemanCoordinationResult,
    ForemanRuntimeStatus,
};

#[derive(Default)]
struct RestartedReader {
    retained: Vec<ForemanSnapshot>,
}

impl ForemanCoordinationPort for RestartedReader {
    fn replay_checkpoint(
        &mut self,
        _intent: &ForemanCheckpointIntent,
    ) -> ForemanCoordinationResult<Option<ForemanCheckpointReplay>> {
        Ok(None)
    }

    fn append_snapshot(
        &mut self,
        _command_id: &str,
        _correlation_id: &str,
        _occurred_at: &str,
        _snapshot: ForemanSnapshot,
        _writer: &WriterLeaseAuthorityHead,
    ) -> ForemanCoordinationResult<ForemanAppendReceipt> {
        Err(ForemanCoordinationError::new(
            ForemanCoordinationErrorKind::Unavailable,
            "FOREMAN_FAKE_APPEND_DISABLED",
        ))
    }

    fn load_snapshots(&mut self) -> ForemanCoordinationResult<Vec<ForemanSnapshot>> {
        Ok(self.retained.clone())
    }

    fn load_runtime_status(&mut self) -> ForemanCoordinationResult<ForemanRuntimeStatus> {
        let projection = reconstruct(self.retained.clone()).expect("projection");
        Ok(ForemanRuntimeStatus::new(
            lattice_contracts::ContentDigest::from_sha256("1".repeat(64)).expect("digest"),
            lattice_contracts::ContentDigest::from_sha256("2".repeat(64)).expect("digest"),
            projection.latest_generation(),
            projection.active().len(),
            projection.blocked().len(),
            projection.completed().len(),
            projection.runtime_next_action(),
            projection.dependency().cloned(),
        ))
    }
}

#[test]
fn fresh_reader_uses_only_typed_verified_snapshots() {
    let blocked = ForemanSnapshot::new(
        "worker-1",
        "thread-worker-1",
        "TASK-079",
        "feature/task-079-durable-foreman-state",
        "lattice-worktrees/task-079-durable-foreman-state",
        "1234567890abcdef1234567890abcdef12345678",
        ForemanState::Blocked,
        Some("dependency:TASK-087".to_owned()),
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "authority:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        1,
    )
    .expect("snapshot");
    let mut reader = RestartedReader {
        retained: vec![blocked],
    };
    let snapshots = reader.load_snapshots().expect("load");
    let projection = reconstruct(snapshots).expect("projection");
    assert_eq!(projection.blocked().len(), 1);
    assert!(!projection.blocked()[0].archive_ready());
}
