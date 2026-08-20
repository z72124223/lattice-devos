use std::cell::RefCell;
use std::rc::Rc;

use lattice_contracts::{
    AttemptId, ContentDigest, DaemonEpoch, HolderProcessId, ProjectId, ProjectSnapshotId,
    RuntimeAdmissionMode, RuntimeKind, SubjectBinding, TaskId, WriterLeaseAuthorityHead,
};
use lattice_orchestrator::{
    ControlledTaskOrchestratorError, ControlledTaskRequest, run_controlled_task,
};
use lattice_ports::{
    AutonomyDisposition, AutonomyModel, AutonomyReason, AutonomyReceiptProjection,
    AutonomyVerification, ControlledTaskExecutionError, ControlledTaskExecutionErrorKind,
    ControlledTaskExecutionPort, ControlledTaskExecutionResult, TaskLifecycleAdmission,
    TaskLifecycleAutonomyEvidence, TaskLifecycleError, TaskLifecycleErrorKind,
    TaskLifecycleEvidence, TaskLifecyclePort, TaskLifecycleResult, WriterAuthorityGuardPort,
};
use lattice_task_domain::TaskState;
use lattice_writer_lease::{
    AcquireClaim, AcquireCommand, CommandOutcome, FakeWriterLease, LeaseObservation,
    ReleaseCommand, WriterLeaseCommand, WriterLeaseCommandReceipt, WriterLeaseCurrentAuthority,
    WriterLeaseRepository, WriterLeaseRepositoryCommand, WriterLeaseRepositoryError,
    WriterLeaseRepositoryErrorKind,
};

type Calls = Rc<RefCell<Vec<String>>>;

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn binding() -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-038").expect("task"),
        "1",
        digest('a'),
    )
    .expect("binding")
}

fn request() -> ControlledTaskRequest {
    ControlledTaskRequest::new(
        binding(),
        "client-request-1",
        AttemptId::new("attempt-1").expect("attempt"),
        "lease-1",
        "codex-writer-1",
        "task038-worktree-1",
        HolderProcessId::new(42).expect("process"),
        digest('b'),
    )
    .expect("controlled request")
}

struct FakeLifecycle {
    calls: Calls,
    binding: SubjectBinding,
    state: TaskState,
    result_digest: Option<ContentDigest>,
    autonomy_receipt: Option<AutonomyReceiptProjection>,
    historical_optional: bool,
    reject_stopping_authority_as_stale: bool,
}

impl FakeLifecycle {
    fn new(calls: Calls) -> Self {
        Self {
            calls,
            binding: binding(),
            state: TaskState::Draft,
            result_digest: None,
            autonomy_receipt: None,
            historical_optional: false,
            reject_stopping_authority_as_stale: false,
        }
    }

    fn evidence(&self) -> TaskLifecycleEvidence {
        let autonomy_evidence = if self.historical_optional {
            TaskLifecycleAutonomyEvidence::HistoricalOptional(None)
        } else {
            self.autonomy_receipt.clone().map_or(
                TaskLifecycleAutonomyEvidence::Unadmitted,
                TaskLifecycleAutonomyEvidence::RequiredComplete,
            )
        };
        TaskLifecycleEvidence::new(
            self.binding.clone(),
            autonomy_evidence,
            self.state,
            digest('c'),
            self.result_digest.clone(),
        )
    }
}

impl TaskLifecyclePort for FakeLifecycle {
    fn admit(
        &mut self,
        binding: &SubjectBinding,
        client_request_id: &str,
    ) -> TaskLifecycleResult<TaskLifecycleAdmission> {
        assert_eq!(binding, &self.binding);
        assert_eq!(client_request_id, "client-request-1");
        self.calls.borrow_mut().push("task:admit".to_owned());
        if self.historical_optional || self.autonomy_receipt.is_some() {
            TaskLifecycleAdmission::existing(self.evidence())
        } else {
            Ok(TaskLifecycleAdmission::pending_required_receipt(
                self.binding.clone(),
                digest('c'),
            ))
        }
    }

    fn transition(
        &mut self,
        binding: &SubjectBinding,
        from: TaskState,
        to: TaskState,
        writer_authority: Option<&WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        assert_eq!(binding, &self.binding);
        assert_eq!(self.state, from);
        let must_be_fenced = matches!(
            (from, to),
            (TaskState::Preparing, TaskState::Executing)
                | (
                    TaskState::Executing,
                    TaskState::Verifying | TaskState::Stopping
                )
                | (TaskState::Verifying, TaskState::Reviewing)
                | (TaskState::Reviewing, TaskState::AwaitingMergeApproval)
                | (TaskState::AwaitingMergeApproval, TaskState::Merging)
        );
        assert_eq!(writer_authority.is_some(), must_be_fenced);
        let fence = writer_authority.map_or_else(
            || "unfenced".to_owned(),
            |authority| {
                assert_writer_binding(authority, binding);
                format!("fence={}", authority.identity().fencing_token().get())
            },
        );
        self.calls
            .borrow_mut()
            .push(format!("task:{}->{}:{fence}", from.as_str(), to.as_str()));
        if self.reject_stopping_authority_as_stale
            && (from, to) == (TaskState::Executing, TaskState::Stopping)
        {
            return Err(TaskLifecycleError::new(
                TaskLifecycleErrorKind::Rejected,
                "LATTICE_TASK_STALE_WRITER_AUTHORITY",
            ));
        }
        self.state = to;
        Ok(self.evidence())
    }

    fn record_autonomy_receipt(
        &mut self,
        binding: &SubjectBinding,
        writer_authority: Option<&WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        assert_eq!(binding, &self.binding);
        assert_eq!(self.state, TaskState::Draft);
        let writer_authority = writer_authority.expect("controlled task requires writer");
        assert_writer_binding(writer_authority, binding);
        self.calls.borrow_mut().push(format!(
            "task:autonomy-receipt:fence={}",
            writer_authority.identity().fencing_token().get()
        ));
        self.autonomy_receipt = Some(AutonomyReceiptProjection::new(
            digest('1'),
            digest('2'),
            digest('3'),
            TaskState::Draft,
            AutonomyDisposition::Proceed,
            AutonomyReason::RoutineAuthorized,
            Some(AutonomyModel::GovernedCodexWriter),
            Some(AutonomyVerification::FocusedChecks),
        )?);
        Ok(self.evidence())
    }

    fn record_result(
        &mut self,
        binding: &SubjectBinding,
        result_digest: &ContentDigest,
        writer_authority: &WriterLeaseAuthorityHead,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        assert_eq!(binding, &self.binding);
        assert_eq!(self.state, TaskState::Merging);
        assert_writer_binding(writer_authority, binding);
        self.calls.borrow_mut().push(format!(
            "task:result:fence={}",
            writer_authority.identity().fencing_token().get()
        ));
        self.result_digest = Some(result_digest.clone());
        Ok(self.evidence())
    }

    fn load(&mut self, binding: &SubjectBinding) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        assert_eq!(binding, &self.binding);
        Ok(self.evidence())
    }
}

struct FakeLeaseRepository {
    calls: Calls,
    owner: FakeWriterLease,
}

impl FakeLeaseRepository {
    fn new(calls: Calls) -> Self {
        Self {
            calls,
            owner: FakeWriterLease::new(),
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

impl WriterLeaseRepository for FakeLeaseRepository {
    fn execute(
        &mut self,
        command: WriterLeaseRepositoryCommand,
    ) -> Result<WriterLeaseCommandReceipt, WriterLeaseRepositoryError> {
        match command {
            WriterLeaseRepositoryCommand::Acquire(request) => {
                self.calls.borrow_mut().push("lease:acquire".to_owned());
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
                        daemon_instance_id: "test-daemon-1".to_owned(),
                        daemon_epoch: DaemonEpoch::new(1).expect("daemon epoch"),
                    },
                    observation: Self::observation("2026-08-09T00:00:00Z"),
                    expires_at: "2026-08-09T00:05:00Z".to_owned(),
                }))
            }
            WriterLeaseRepositoryCommand::Release(request) => {
                self.calls.borrow_mut().push("lease:release".to_owned());
                self.execute_fake(WriterLeaseCommand::Release(ReleaseCommand {
                    command_id: request.command_id,
                    project_id: request.project_id,
                    expected_head: request.expected_head,
                    observation: Self::observation("2026-08-09T00:01:00Z"),
                }))
            }
            WriterLeaseRepositoryCommand::Heartbeat(_)
            | WriterLeaseRepositoryCommand::MarkSuspect(_)
            | WriterLeaseRepositoryCommand::Revoke(_) => Err(WriterLeaseRepositoryError::new(
                WriterLeaseRepositoryErrorKind::Unavailable,
            )),
        }
    }

    fn current_authority(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<Option<WriterLeaseCurrentAuthority>, WriterLeaseRepositoryError> {
        let receipt = self.owner.current_receipt(project_id);
        let head = self.owner.current_head(project_id);
        self.calls.borrow_mut().push(match head.as_ref() {
            Some(authority) => format!(
                "lease:current:fence={}",
                authority.identity().fencing_token().get()
            ),
            None => "lease:current:none".to_owned(),
        });
        match (receipt, head) {
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
        self.calls.borrow_mut().push(format!(
            "lease:assert:fence={}",
            expected.identity().fencing_token().get()
        ));
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

struct FakeExecution {
    calls: Calls,
    failure: Option<ControlledTaskExecutionErrorKind>,
}

impl ControlledTaskExecutionPort for FakeExecution {
    fn execute(
        &mut self,
        binding: &SubjectBinding,
        writer_authority: &WriterLeaseAuthorityHead,
        writer_guard: &mut dyn WriterAuthorityGuardPort,
    ) -> ControlledTaskExecutionResult<ContentDigest> {
        assert_writer_binding(writer_authority, binding);
        writer_guard.assert_current(writer_authority)?;
        self.calls.borrow_mut().push(format!(
            "codex:execute:fence={}",
            writer_authority.identity().fencing_token().get()
        ));
        if let Some(kind) = self.failure {
            Err(ControlledTaskExecutionError::new(
                kind,
                "CODEX_CONTROLLED_TEST_FAILURE",
            ))
        } else {
            writer_guard.assert_current(writer_authority)?;
            Ok(digest('f'))
        }
    }
}

fn assert_writer_binding(authority: &WriterLeaseAuthorityHead, binding: &SubjectBinding) {
    let identity = authority.identity();
    assert_eq!(identity.project_id(), binding.project_id());
    assert_eq!(
        identity.project_snapshot_id(),
        binding.project_snapshot_id()
    );
    assert_eq!(identity.task_id(), binding.task_id());
    assert_eq!(identity.task_revision(), binding.task_revision());
    assert_eq!(identity.task_spec_digest(), binding.task_spec_digest());
}

#[test]
fn controlled_success_keeps_one_ordered_fenced_codex_lane_and_releases_before_completion() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle::new(calls.clone());
    let mut lease = FakeLeaseRepository::new(calls.clone());
    let mut execution = FakeExecution {
        calls: calls.clone(),
        failure: None,
    };

    let evidence = run_controlled_task(&request(), &mut lifecycle, &mut lease, &mut execution)
        .expect("controlled task succeeds");

    assert_eq!(evidence.state(), TaskState::Completed);
    assert_eq!(evidence.result_digest(), Some(&digest('f')));
    assert_eq!(
        calls.borrow().as_slice(),
        [
            "task:admit",
            "lease:current:none",
            "lease:acquire",
            "lease:current:fence=1",
            "lease:assert:fence=1",
            "task:autonomy-receipt:fence=1",
            "task:DRAFT->AWAITING_EXECUTION_APPROVAL:unfenced",
            "task:AWAITING_EXECUTION_APPROVAL->PREPARING:unfenced",
            "task:PREPARING->EXECUTING:fence=1",
            "lease:assert:fence=1",
            "codex:execute:fence=1",
            "lease:assert:fence=1",
            "lease:assert:fence=1",
            "task:EXECUTING->VERIFYING:fence=1",
            "task:VERIFYING->REVIEWING:fence=1",
            "task:REVIEWING->AWAITING_MERGE_APPROVAL:fence=1",
            "task:AWAITING_MERGE_APPROVAL->MERGING:fence=1",
            "task:result:fence=1",
            "lease:assert:fence=1",
            "lease:release",
            "lease:current:none",
            "task:MERGING->COMPLETED:unfenced",
        ]
    );
    assert!(
        lease
            .current_authority(binding().project_id())
            .expect("current authority")
            .is_none()
    );
}

#[test]
fn existing_ask_user_receipt_stops_before_writer_or_execution() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle::new(calls.clone());
    lifecycle.autonomy_receipt = Some(
        AutonomyReceiptProjection::new(
            digest('1'),
            digest('2'),
            digest('3'),
            TaskState::Draft,
            AutonomyDisposition::AskUser,
            AutonomyReason::NewUserDecision,
            None,
            None,
        )
        .expect("ASK_USER projection"),
    );
    let mut lease = FakeLeaseRepository::new(calls.clone());
    let mut execution = FakeExecution {
        calls: calls.clone(),
        failure: None,
    };

    let evidence = run_controlled_task(&request(), &mut lifecycle, &mut lease, &mut execution)
        .expect("ASK_USER remains a bounded Draft outcome");

    assert_eq!(evidence.state(), TaskState::Draft);
    assert_eq!(
        evidence
            .autonomy_receipt()
            .map(AutonomyReceiptProjection::disposition),
        Some(AutonomyDisposition::AskUser)
    );
    assert_eq!(calls.borrow().as_slice(), ["task:admit"]);
}

#[test]
fn existing_proceed_receipt_requires_reconciliation_before_a_new_writer() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle::new(calls.clone());
    lifecycle.autonomy_receipt = Some(
        AutonomyReceiptProjection::new(
            digest('1'),
            digest('2'),
            digest('3'),
            TaskState::Draft,
            AutonomyDisposition::Proceed,
            AutonomyReason::RoutineAuthorized,
            Some(AutonomyModel::GovernedCodexWriter),
            Some(AutonomyVerification::FocusedChecks),
        )
        .expect("PROCEED projection"),
    );
    let mut lease = FakeLeaseRepository::new(calls.clone());
    let mut execution = FakeExecution {
        calls: calls.clone(),
        failure: None,
    };

    let error = run_controlled_task(&request(), &mut lifecycle, &mut lease, &mut execution)
        .expect_err("an existing PROCEED receipt cannot authorize a replacement Writer");

    assert_eq!(
        error,
        ControlledTaskOrchestratorError::ReconciliationRequired
    );
    assert_eq!(calls.borrow().as_slice(), ["task:admit"]);
}

#[test]
fn existing_historical_optional_draft_continues_without_synthesizing_a_receipt() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle::new(calls.clone());
    lifecycle.historical_optional = true;
    let mut lease = FakeLeaseRepository::new(calls.clone());
    let mut execution = FakeExecution {
        calls: calls.clone(),
        failure: None,
    };

    let evidence = run_controlled_task(&request(), &mut lifecycle, &mut lease, &mut execution)
        .expect("historical optional task remains runnable without a synthetic receipt");

    assert_eq!(evidence.state(), TaskState::Completed);
    assert!(matches!(
        evidence.autonomy_evidence(),
        TaskLifecycleAutonomyEvidence::HistoricalOptional(None)
    ));
    assert!(
        calls
            .borrow()
            .iter()
            .all(|call| !call.starts_with("task:autonomy-receipt"))
    );
}

#[test]
fn controlled_codex_failure_fails_closed_releases_writer_and_never_records_a_result() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle::new(calls.clone());
    let mut lease = FakeLeaseRepository::new(calls.clone());
    let mut execution = FakeExecution {
        calls: calls.clone(),
        failure: Some(ControlledTaskExecutionErrorKind::Known),
    };

    let error = run_controlled_task(&request(), &mut lifecycle, &mut lease, &mut execution)
        .expect_err("Codex failure must fail the controlled task");

    let ControlledTaskOrchestratorError::Execution(error) = error else {
        panic!("expected controlled execution failure");
    };
    assert_eq!(error.kind(), ControlledTaskExecutionErrorKind::Known);
    assert_eq!(error.code(), "CODEX_CONTROLLED_TEST_FAILURE");
    assert_eq!(lifecycle.state, TaskState::Failed);
    assert!(lifecycle.result_digest.is_none());
    assert_eq!(
        calls.borrow().as_slice(),
        [
            "task:admit",
            "lease:current:none",
            "lease:acquire",
            "lease:current:fence=1",
            "lease:assert:fence=1",
            "task:autonomy-receipt:fence=1",
            "task:DRAFT->AWAITING_EXECUTION_APPROVAL:unfenced",
            "task:AWAITING_EXECUTION_APPROVAL->PREPARING:unfenced",
            "task:PREPARING->EXECUTING:fence=1",
            "lease:assert:fence=1",
            "codex:execute:fence=1",
            "task:EXECUTING->STOPPING:fence=1",
            "lease:assert:fence=1",
            "lease:release",
            "lease:current:none",
            "task:STOPPING->FAILED:unfenced",
        ]
    );
    assert!(
        lease
            .current_authority(binding().project_id())
            .expect("current authority")
            .is_none()
    );
    assert!(
        calls
            .borrow()
            .iter()
            .all(|call| !call.starts_with("task:result"))
    );
}

#[test]
fn ambiguous_execution_keeps_the_fence_and_enters_reconciliation() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle::new(calls.clone());
    let mut lease = FakeLeaseRepository::new(calls.clone());
    let mut execution = FakeExecution {
        calls: calls.clone(),
        failure: Some(ControlledTaskExecutionErrorKind::Ambiguous),
    };

    let error = run_controlled_task(&request(), &mut lifecycle, &mut lease, &mut execution)
        .expect_err("ambiguous execution must require reconciliation");

    let ControlledTaskOrchestratorError::Execution(error) = error else {
        panic!("expected controlled execution failure");
    };
    assert_eq!(error.kind(), ControlledTaskExecutionErrorKind::Ambiguous);
    assert_eq!(lifecycle.state, TaskState::Stopping);
    assert!(lifecycle.result_digest.is_none());
    assert!(
        lease
            .current_authority(binding().project_id())
            .expect("current authority")
            .is_some()
    );
    assert!(calls.borrow().iter().all(|call| call != "lease:release"));
    assert!(
        calls
            .borrow()
            .iter()
            .any(|call| call == "task:EXECUTING->STOPPING:fence=1")
    );
}

#[test]
fn stale_writer_authority_cannot_record_stopping_or_release_the_current_lease() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle::new(calls.clone());
    lifecycle.reject_stopping_authority_as_stale = true;
    let mut lease = FakeLeaseRepository::new(calls.clone());
    let mut execution = FakeExecution {
        calls: calls.clone(),
        failure: Some(ControlledTaskExecutionErrorKind::Known),
    };

    let error = run_controlled_task(&request(), &mut lifecycle, &mut lease, &mut execution)
        .expect_err("a stale writer must not persist the stopping transition");

    let ControlledTaskOrchestratorError::Lifecycle(error) = error else {
        panic!("expected the fenced lifecycle rejection");
    };
    assert_eq!(error.kind(), TaskLifecycleErrorKind::Rejected);
    assert_eq!(error.code(), "LATTICE_TASK_STALE_WRITER_AUTHORITY");
    assert_eq!(lifecycle.state, TaskState::Executing);
    assert!(
        lease
            .current_authority(binding().project_id())
            .expect("current authority")
            .is_some()
    );
    assert!(calls.borrow().iter().all(|call| call != "lease:release"));
    assert!(
        calls
            .borrow()
            .iter()
            .any(|call| call == "task:EXECUTING->STOPPING:fence=1")
    );
}

#[test]
fn fake_repository_returns_only_applied_acquire_and_release_receipts() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut lifecycle = FakeLifecycle::new(calls.clone());
    let mut lease = FakeLeaseRepository::new(calls.clone());
    let mut execution = FakeExecution {
        calls,
        failure: None,
    };

    run_controlled_task(&request(), &mut lifecycle, &mut lease, &mut execution)
        .expect("controlled task succeeds");

    let aggregate = lease
        .owner
        .export_snapshot(binding().project_id())
        .expect("project aggregate exists");
    let verified = lattice_writer_lease::verify_snapshot(&aggregate).expect("verified aggregate");
    assert_eq!(verified.command_receipts().len(), 2);
    assert!(
        verified
            .command_receipts()
            .iter()
            .all(|receipt| receipt.outcome == CommandOutcome::Applied)
    );
}
