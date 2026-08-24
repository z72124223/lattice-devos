use std::cell::RefCell;
use std::rc::Rc;

use lattice_contracts::{
    AttemptId, ContentDigest, DaemonEpoch, HolderProcessId, ProjectId, ProjectSnapshotId,
    RuntimeAdmissionMode, RuntimeKind, TaskId, WriterLeaseAuthorityHead,
};
use lattice_foreman_state::{
    ForemanCheckpointIntent, ForemanServerObservation, ForemanSnapshot, ForemanState,
};
use lattice_orchestrator::{ForemanCheckpointOrchestratorError, checkpoint_foreman};
use lattice_ports::{
    ForemanAppendReceipt, ForemanCheckpointReplay, ForemanCoordinationError,
    ForemanCoordinationErrorKind, ForemanCoordinationPort, ForemanCoordinationResult,
    ForemanRuntimeStatus,
};
use lattice_writer_lease::{
    AcquireClaim, AcquireCommand, FakeWriterLease, LeaseObservation, ReleaseCommand,
    WriterLeaseAcquireRequest, WriterLeaseCommand, WriterLeaseCommandReceipt,
    WriterLeaseCurrentAuthority, WriterLeaseRepository, WriterLeaseRepositoryCommand,
    WriterLeaseRepositoryError, WriterLeaseRepositoryErrorKind,
};

type Calls = Rc<RefCell<Vec<String>>>;

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn intent() -> ForemanCheckpointIntent {
    ForemanCheckpointIntent::new(
        "checkpoint-1",
        1,
        "2026-08-25T00:00:01Z",
        ForemanState::Active,
        None,
        "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("intent")
}

fn observation() -> ForemanServerObservation {
    ForemanServerObservation::new(
        "sole-foreman-v1",
        "lattice-devos-sole-foreman-v1",
        "TASK-FOREMAN-COORDINATION",
        "feature/task-105-durable-foreman-runtime",
        "lattice-worktrees/task-105-durable-foreman-runtime",
        "1234567890abcdef1234567890abcdef12345678",
    )
    .expect("observation")
}

fn acquire() -> WriterLeaseAcquireRequest {
    WriterLeaseAcquireRequest {
        command_id: "foreman-acquire-checkpoint-1".to_owned(),
        expected_head: None,
        project_id: ProjectId::new("lattice-control").expect("project"),
        project_snapshot_id: ProjectSnapshotId::new("foreman-coordination-v1").expect("snapshot"),
        task_id: TaskId::new("TASK-FOREMAN-COORDINATION").expect("task"),
        task_revision: "1".to_owned(),
        task_spec_digest: digest('7'),
        attempt_id: AttemptId::new("foreman-attempt-checkpoint-1").expect("attempt"),
        lease_id: "foreman-lease-checkpoint-1".to_owned(),
        lease_holder_id: "latticed-foreman-v1".to_owned(),
        worktree_id: "task-105-durable-foreman-runtime".to_owned(),
        holder_process_id: HolderProcessId::new(42).expect("pid"),
        holder_process_start_identity: digest('8'),
    }
}

#[derive(Clone, Copy)]
enum ReleaseMode {
    Normal,
    UnknownNotCommittedOnce,
    UnknownCommittedOnce,
}

struct FakeWriter {
    calls: Calls,
    owner: FakeWriterLease,
    release_mode: ReleaseMode,
    release_attempts: usize,
}

impl FakeWriter {
    fn new(calls: Calls, release_mode: ReleaseMode) -> Self {
        Self {
            calls,
            owner: FakeWriterLease::new(),
            release_mode,
            release_attempts: 0,
        }
    }

    fn observation(at: &str) -> LeaseObservation {
        LeaseObservation {
            runtime: RuntimeKind::Fake,
            admission: RuntimeAdmissionMode::Active,
            observed_at: at.to_owned(),
            time_observation_digest: digest('d'),
            admission_observation_digest: digest('e'),
        }
    }

    fn execute_fake(
        &mut self,
        command: WriterLeaseCommand,
    ) -> Result<WriterLeaseCommandReceipt, WriterLeaseRepositoryError> {
        self.owner
            .execute(command)
            .map_err(WriterLeaseRepositoryError::from_domain)
    }
}

impl WriterLeaseRepository for FakeWriter {
    fn execute(
        &mut self,
        command: WriterLeaseRepositoryCommand,
    ) -> Result<WriterLeaseCommandReceipt, WriterLeaseRepositoryError> {
        match command {
            WriterLeaseRepositoryCommand::Acquire(request) => {
                self.calls.borrow_mut().push("writer:acquire".to_owned());
                self.execute_fake(WriterLeaseCommand::Acquire(AcquireCommand {
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
                        daemon_instance_id: "test-daemon".to_owned(),
                        daemon_epoch: DaemonEpoch::new(1).expect("epoch"),
                    },
                    observation: Self::observation("2026-08-25T00:00:01Z"),
                    expires_at: "2026-08-25T00:05:01Z".to_owned(),
                }))
            }
            WriterLeaseRepositoryCommand::Release(request) => {
                self.calls.borrow_mut().push("writer:release".to_owned());
                self.release_attempts += 1;
                let command = WriterLeaseCommand::Release(ReleaseCommand {
                    command_id: request.command_id,
                    project_id: request.project_id,
                    expected_head: request.expected_head,
                    observation: Self::observation("2026-08-25T00:01:01Z"),
                });
                if self.release_attempts == 1 {
                    match self.release_mode {
                        ReleaseMode::UnknownNotCommittedOnce => {
                            return Err(WriterLeaseRepositoryError::new(
                                WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown,
                            ));
                        }
                        ReleaseMode::UnknownCommittedOnce => {
                            self.execute_fake(command)?;
                            return Err(WriterLeaseRepositoryError::new(
                                WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown,
                            ));
                        }
                        ReleaseMode::Normal => {}
                    }
                }
                self.execute_fake(command)
            }
            _ => Err(WriterLeaseRepositoryError::new(
                WriterLeaseRepositoryErrorKind::Unavailable,
            )),
        }
    }

    fn current_authority(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<Option<WriterLeaseCurrentAuthority>, WriterLeaseRepositoryError> {
        self.calls.borrow_mut().push("writer:current".to_owned());
        match (
            self.owner.current_receipt(project_id),
            self.owner.current_head(project_id),
        ) {
            (Some(receipt), Some(head)) => {
                WriterLeaseCurrentAuthority::new(receipt, head).map(Some)
            }
            (None, None) => Ok(None),
            _ => Err(WriterLeaseRepositoryError::new(
                WriterLeaseRepositoryErrorKind::Corrupt,
            )),
        }
    }

    fn assert_current(
        &mut self,
        expected: &WriterLeaseAuthorityHead,
    ) -> Result<(), WriterLeaseRepositoryError> {
        if self
            .owner
            .current_head(expected.identity().project_id())
            .as_ref()
            == Some(expected)
        {
            Ok(())
        } else {
            Err(WriterLeaseRepositoryError::new(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ))
        }
    }
}

struct FakeCoordination {
    calls: Calls,
    replay: Option<ForemanCheckpointReplay>,
    append_unknown: bool,
    append_unknown_committed: bool,
    append_known_conflict: bool,
    replay_error: Option<ForemanCoordinationError>,
    append_count: usize,
}

impl FakeCoordination {
    fn new(calls: Calls) -> Self {
        Self {
            calls,
            replay: None,
            append_unknown: false,
            append_unknown_committed: false,
            append_known_conflict: false,
            replay_error: None,
            append_count: 0,
        }
    }
}

impl ForemanCoordinationPort for FakeCoordination {
    fn replay_checkpoint(
        &mut self,
        _intent: &ForemanCheckpointIntent,
    ) -> ForemanCoordinationResult<Option<ForemanCheckpointReplay>> {
        self.calls.borrow_mut().push("ledger:replay".to_owned());
        if let Some(error) = &self.replay_error {
            return Err(error.clone());
        }
        Ok(self.replay.clone())
    }

    fn append_snapshot(
        &mut self,
        _command_id: &str,
        _correlation_id: &str,
        _occurred_at: &str,
        snapshot: ForemanSnapshot,
        _writer: &WriterLeaseAuthorityHead,
    ) -> ForemanCoordinationResult<ForemanAppendReceipt> {
        self.calls.borrow_mut().push("ledger:append".to_owned());
        self.append_count += 1;
        if self.append_known_conflict {
            return Err(ForemanCoordinationError::new(
                ForemanCoordinationErrorKind::Conflict,
                "FOREMAN_GENERATION_INVALID",
            ));
        }
        if self.append_unknown || self.append_unknown_committed {
            if self.append_unknown_committed {
                let authority = snapshot
                    .authority()
                    .strip_prefix("authority:sha256:")
                    .and_then(|value| ContentDigest::from_sha256(value).ok())
                    .expect("authority digest");
                self.replay = Some(ForemanCheckpointReplay::new(
                    ForemanAppendReceipt::new(digest('1'), digest('3'), digest('2'), 1, true)?,
                    authority,
                ));
            }
            return Err(ForemanCoordinationError::new(
                ForemanCoordinationErrorKind::OutcomeUnknown,
                "FOREMAN_APPEND_OUTCOME_UNKNOWN",
            ));
        }
        let receipt = ForemanAppendReceipt::new(digest('1'), digest('3'), digest('2'), 1, false)?;
        let authority = snapshot
            .authority()
            .strip_prefix("authority:sha256:")
            .and_then(|value| ContentDigest::from_sha256(value).ok())
            .expect("authority digest");
        self.replay = Some(ForemanCheckpointReplay::new(
            ForemanAppendReceipt::new(digest('1'), digest('3'), digest('2'), 1, true)?,
            authority,
        ));
        Ok(receipt)
    }

    fn load_snapshots(&mut self) -> ForemanCoordinationResult<Vec<ForemanSnapshot>> {
        Ok(Vec::new())
    }

    fn load_runtime_status(&mut self) -> ForemanCoordinationResult<ForemanRuntimeStatus> {
        Ok(ForemanRuntimeStatus::new(
            digest('1'),
            digest('2'),
            0,
            0,
            0,
            0,
            "NO_DURABLE_SNAPSHOT",
        ))
    }
}

fn run(
    coordination: &mut FakeCoordination,
    writer: &mut FakeWriter,
    calls: &Calls,
) -> Result<ForemanAppendReceipt, ForemanCheckpointOrchestratorError> {
    checkpoint_foreman(coordination, writer, &intent(), acquire(), || {
        calls.borrow_mut().push("git:observe".to_owned());
        Ok(observation())
    })
}

#[test]
fn known_success_orders_replay_observe_acquire_append_release() {
    let calls = Calls::default();
    let mut coordination = FakeCoordination::new(calls.clone());
    let mut writer = FakeWriter::new(calls.clone(), ReleaseMode::Normal);
    run(&mut coordination, &mut writer, &calls).expect("checkpoint");
    assert_eq!(
        calls.borrow().as_slice(),
        [
            "ledger:replay",
            "git:observe",
            "writer:acquire",
            "writer:current",
            "ledger:append",
            "writer:release",
        ]
    );
}

#[test]
fn append_unknown_never_releases() {
    let calls = Calls::default();
    let mut coordination = FakeCoordination::new(calls.clone());
    coordination.append_unknown = true;
    let mut writer = FakeWriter::new(calls.clone(), ReleaseMode::Normal);
    assert!(matches!(
        run(&mut coordination, &mut writer, &calls),
        Err(ForemanCheckpointOrchestratorError::Append(_))
    ));
    assert!(!calls.borrow().iter().any(|call| call == "writer:release"));
}

#[test]
fn append_unknown_that_committed_retries_via_replay_without_git_or_second_append() {
    let calls = Calls::default();
    let mut coordination = FakeCoordination::new(calls.clone());
    coordination.append_unknown_committed = true;
    let mut writer = FakeWriter::new(calls.clone(), ReleaseMode::Normal);
    assert!(matches!(
        run(&mut coordination, &mut writer, &calls),
        Err(ForemanCheckpointOrchestratorError::Append(_))
    ));
    let append_count = coordination.append_count;
    calls.borrow_mut().clear();
    run(&mut coordination, &mut writer, &calls).expect("replay reconciles committed append");
    assert_eq!(coordination.append_count, append_count);
    assert_eq!(
        calls.borrow().as_slice(),
        ["ledger:replay", "writer:current", "writer:release"]
    );
}

#[test]
fn known_append_rejection_releases_writer_before_returning_error() {
    let calls = Calls::default();
    let mut coordination = FakeCoordination::new(calls.clone());
    coordination.append_known_conflict = true;
    let mut writer = FakeWriter::new(calls.clone(), ReleaseMode::Normal);
    assert!(matches!(
        run(&mut coordination, &mut writer, &calls),
        Err(ForemanCheckpointOrchestratorError::Append(_))
    ));
    assert_eq!(
        calls.borrow().as_slice(),
        [
            "ledger:replay",
            "git:observe",
            "writer:acquire",
            "writer:current",
            "ledger:append",
            "writer:release",
        ]
    );
    assert!(
        writer
            .current_authority(&acquire().project_id)
            .expect("current")
            .is_none()
    );
}

#[test]
fn preflight_generation_rejection_has_no_git_or_writer_effect() {
    let calls = Calls::default();
    let mut coordination = FakeCoordination::new(calls.clone());
    coordination.replay_error = Some(ForemanCoordinationError::new(
        ForemanCoordinationErrorKind::Conflict,
        "FOREMAN_GENERATION_INVALID",
    ));
    let mut writer = FakeWriter::new(calls.clone(), ReleaseMode::Normal);
    assert!(matches!(
        run(&mut coordination, &mut writer, &calls),
        Err(ForemanCheckpointOrchestratorError::Replay(_))
    ));
    assert_eq!(calls.borrow().as_slice(), ["ledger:replay"]);
}

#[test]
fn release_unknown_not_committed_retries_release_without_observation_or_append() {
    let calls = Calls::default();
    let mut coordination = FakeCoordination::new(calls.clone());
    let mut writer = FakeWriter::new(calls.clone(), ReleaseMode::UnknownNotCommittedOnce);
    assert!(matches!(
        run(&mut coordination, &mut writer, &calls),
        Err(ForemanCheckpointOrchestratorError::WriterRelease(_))
    ));
    let append_count = coordination.append_count;
    calls.borrow_mut().clear();
    run(&mut coordination, &mut writer, &calls).expect("reconciled release");
    assert_eq!(coordination.append_count, append_count);
    assert_eq!(
        calls.borrow().as_slice(),
        ["ledger:replay", "writer:current", "writer:release"]
    );
}

#[test]
fn release_unknown_committed_replay_proves_release_without_reappend() {
    let calls = Calls::default();
    let mut coordination = FakeCoordination::new(calls.clone());
    let mut writer = FakeWriter::new(calls.clone(), ReleaseMode::UnknownCommittedOnce);
    assert!(matches!(
        run(&mut coordination, &mut writer, &calls),
        Err(ForemanCheckpointOrchestratorError::WriterRelease(_))
    ));
    let append_count = coordination.append_count;
    calls.borrow_mut().clear();
    run(&mut coordination, &mut writer, &calls).expect("replayed completed release");
    assert_eq!(coordination.append_count, append_count);
    assert_eq!(
        calls.borrow().as_slice(),
        ["ledger:replay", "writer:current"]
    );
}
