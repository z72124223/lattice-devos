use lattice_contracts::{
    ContentDigest, DaemonEpoch, HolderProcessId, ProjectId, ProjectSnapshotId,
    RuntimeAdmissionMode, RuntimeKind, SubjectBinding, TaskId, TaskLedgerStreamIdentity,
    WriterLeaseAuthorityHead,
};
use lattice_foreman_state::{
    AttemptPacketIdentity, AttemptWatchdogObservation, ContinuationSummary, ExternalCostBudget,
    MeaningfulProgress, MeaningfulProgressKind, ModelReason, ModelSelection, ProcessObservation,
    ReasoningEffort, ReconciliationState, RetryDecision, StallReason, TurnActivityObservation,
    WorkerAttemptState, WorkerBudget, WorkerModel, WorkerTerminal,
};
use lattice_orchestrator::{
    ControlledTaskRequest, ManagedAttemptOrchestratorError, ManagedAttemptRequest,
    ManagedAttemptTarget, ManagedPrestartRestartOutcome, ManagedRestartOutcome,
    ManagedStallOutcome, ManagedWorkflowError, ManagedWorkflowRequest, claim_managed_review,
    close_managed_prestart_without_provider_effect, confirm_managed_exact_start,
    continue_managed_prestart_on_restart, finish_claimed_managed_review, finish_managed_execution,
    finish_replayed_managed_review_with_provider_guard, handle_managed_attempt_stall,
    prepare_managed_attempt as prepare_managed_attempt_with_guard, prepare_managed_review,
    reconcile_managed_attempt_on_restart,
    recover_managed_prestart_on_restart as recover_managed_prestart_on_restart_with_guard,
    run_managed_attempt as run_managed_attempt_with_guard, run_managed_workflow,
    run_managed_workflow_with_review_configuration_and_verified_hook,
    run_managed_workflow_with_verified_hook,
};
use lattice_ports::{
    AutonomyDisposition, AutonomyModel, AutonomyReason, AutonomyReceiptProjection,
    AutonomyVerification, ManagedArtifactReceipt, ManagedAttemptClaim,
    ManagedAttemptClaimDisposition, ManagedCodexWorkerPort, ManagedEvidenceInput,
    ManagedEvidenceKind, ManagedForemanRepositoryPort, ManagedModelAvailability, ManagedPortError,
    ManagedPortErrorKind, ManagedPrestartClosureDisposition, ManagedPrestartNoEffectProof,
    ManagedProviderEffectGuardPort, ManagedReviewDispatchDisposition, ManagedTerminalCandidate,
    ManagedVerificationEvidence, ManagedVerificationPort, ManagedVerificationPreparation,
    ManagedVerificationRequest, ManagedWorkerDispatchState, ManagedWorkerExecutionEvent,
    ManagedWorkerObservation, ManagedWorkerPrestartRecovery, ManagedWorkerReconciliation,
    ManagedWorkerThreadDispatchDisposition, ManagedWorkerTurnDispatchDisposition,
    TaskLifecycleAdmission, TaskLifecycleAutonomyEvidence, TaskLifecycleEvidence,
    TaskLifecyclePort, TaskLifecycleResult, VerificationOutcome, VerifiedManagedEvidence,
    VerifiedTaskExecutionBinding, VerifiedTaskVerificationRecord, VerifiedWorkerAttemptRecord,
    VerifiedWorkerObservationRecord,
};
use lattice_task_domain::TaskState;
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CorrelationId, LedgerEventKind, LedgerOutcome,
    ReasonCode, TaskExecutionBindingInput, TaskRuntimeAppendMetadata, TaskSubmissionEnvelope,
    TaskVerificationInput, VerifiedStream, WorkerAttemptInput, apply_append_plan, plan_append,
    plan_task_execution_binding, plan_task_verification_append, plan_worker_attempt_append,
    plan_worker_observation_append,
};
use lattice_writer_lease::{
    AcquireClaim, AcquireCommand, CommandOutcome, FakeWriterLease, LeaseObservation,
    ReleaseCommand, WriterLeaseCommand, WriterLeaseCommandReceipt, WriterLeaseCurrentAuthority,
    WriterLeaseRepository, WriterLeaseRepositoryCommand, WriterLeaseRepositoryError,
    WriterLeaseRepositoryErrorKind,
};
use std::cell::RefCell;
use std::rc::Rc;

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

fn app_server_identity() -> ContentDigest {
    digest('f')
}

fn content_pointer(prefix: &str, digest: &ContentDigest) -> String {
    format!("{prefix}:sha256:{}", digest.as_str())
}

fn pointer_content(pointer: &str) -> ContentDigest {
    ContentDigest::from_sha256(pointer.rsplit(':').next().expect("pointer payload"))
        .expect("pointer digest")
}

fn metadata(command: &str, second: u8) -> TaskRuntimeAppendMetadata {
    TaskRuntimeAppendMetadata::new(
        CommandId::new(command).expect("command"),
        CorrelationId::new(format!("correlation-{command}")).expect("correlation"),
        format!("2026-08-26T02:00:{second:02}Z"),
    )
    .expect("metadata")
}

fn lineage() -> (VerifiedStream, VerifiedTaskExecutionBinding) {
    let intake_identity = TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("project-1:registry:1").expect("snapshot"),
        TaskId::new("TASK-MANAGED-PORTS-001").expect("task"),
        "1",
        digest('a'),
    )
    .expect("intake identity");
    let submission = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "managed-ports-request-1",
        "完成有界的本機修改",
        "Project One",
        intake_identity,
        digest('b'),
    )
    .expect("submission");
    let intake_vacant = VerifiedStream::vacant(submission.identity().clone(), RuntimeKind::Live)
        .expect("intake vacant");
    let intake_command = AppendCommand::new_general_task_created(
        intake_vacant.head().clone(),
        CommandId::new("managed-intake-create").expect("command"),
        CorrelationId::new("managed-intake-correlation").expect("correlation"),
        "2026-08-26T01:59:58Z",
        ActorId::new("lattice-runtime").expect("actor"),
        &submission,
    )
    .expect("intake create");
    let intake_plan = plan_append(&intake_vacant, intake_command).expect("intake plan");
    let intake = apply_append_plan(&intake_vacant, &intake_plan).expect("intake apply");

    let spec_digest = digest('c');
    let spec_identity = TaskLedgerStreamIdentity::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("project-1:registry:1").expect("snapshot"),
        TaskId::new("TASK-MANAGED-PORTS-001").expect("task"),
        "1",
        spec_digest.clone(),
        "TWD",
    )
    .expect("spec identity");
    let spec_vacant =
        VerifiedStream::vacant(spec_identity, RuntimeKind::Live).expect("spec vacant");
    let created = AppendCommand::new(
        spec_vacant.head().clone(),
        CommandId::new("managed-spec-create").expect("command"),
        CorrelationId::new("managed-spec-correlation").expect("correlation"),
        "2026-08-26T01:59:59Z",
        LedgerEventKind::TaskCreated,
        ActorId::new("lattice-runtime").expect("actor"),
        ActionId::new("RECORD_MANAGED_TASK_SPEC_V1").expect("action"),
        LedgerOutcome::Recorded,
        ReasonCode::new("TASK_SPEC_CAPTURED").expect("reason"),
        spec_digest,
        None,
        None,
    )
    .expect("spec create");
    let created_plan = plan_append(&spec_vacant, created).expect("spec plan");
    let successor = apply_append_plan(&spec_vacant, &created_plan).expect("spec apply");
    let binding_plan = plan_task_execution_binding(
        &intake,
        &successor,
        &submission,
        &[],
        metadata("managed-bind", 0),
        TaskExecutionBindingInput::new(
            digest('d'),
            pointer_content(worker_budget().digest()),
            digest('f'),
        )
        .expect("binding input"),
    )
    .expect("binding plan");
    let binding = binding_plan.new_binding().expect("binding").clone();
    let stream = apply_append_plan(&successor, binding_plan.ledger_plan()).expect("bind apply");
    (stream, binding)
}

fn worker_budget() -> WorkerBudget {
    WorkerBudget::new(
        4,
        1,
        2,
        600,
        20_000,
        4,
        ExternalCostBudget::Unavailable,
        "2026-08-26T03:00:00Z",
    )
    .expect("budget")
}

fn packet(binding: &VerifiedTaskExecutionBinding, budget: &WorkerBudget) -> AttemptPacketIdentity {
    packet_number(binding, budget, 1, 10, None)
}

fn packet_with_worktree(
    binding: &VerifiedTaskExecutionBinding,
    budget: &WorkerBudget,
    worktree_digest: &ContentDigest,
) -> AttemptPacketIdentity {
    let model = ModelSelection::new(
        WorkerModel::Terra,
        ReasoningEffort::Medium,
        ModelReason::RoutineEngineering,
        None,
    )
    .expect("model");
    AttemptPacketIdentity::new(
        binding.task_ref().as_str(),
        1,
        &content_pointer("project", &digest('b')),
        &content_pointer("spec", binding.task_spec_digest()),
        &content_pointer("approval", &digest('d')),
        budget,
        &content_pointer("verification", &digest('f')),
        &content_pointer("worktree", worktree_digest),
        "5555555555555555555555555555555555555555",
        model,
        10,
        None,
        None,
    )
    .expect("packet")
}

fn packet_number(
    binding: &VerifiedTaskExecutionBinding,
    budget: &WorkerBudget,
    attempt: u8,
    writer_fence: u64,
    prior_terminal: Option<&ContentDigest>,
) -> AttemptPacketIdentity {
    let model = ModelSelection::new(
        WorkerModel::Terra,
        ReasoningEffort::Medium,
        ModelReason::RoutineEngineering,
        None,
    )
    .expect("model");
    AttemptPacketIdentity::new(
        binding.task_ref().as_str(),
        attempt,
        &content_pointer("project", &digest('b')),
        &content_pointer("spec", binding.task_spec_digest()),
        &content_pointer("approval", &digest('d')),
        budget,
        &content_pointer("verification", &digest('f')),
        &content_pointer("worktree", &digest('4')),
        "5555555555555555555555555555555555555555",
        model,
        writer_fence,
        prior_terminal
            .map(|digest| content_pointer("evidence", digest))
            .as_deref(),
        (attempt > 1).then(|| {
            ContinuationSummary::new("Resume only unverified bounded work.").expect("continuation")
        }),
    )
    .expect("packet")
}

fn with_predispatch_baseline(request: ManagedAttemptRequest) -> ManagedAttemptRequest {
    let baseline = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            ProjectId::new("project-1").expect("project"),
            request.binding().task_ref().clone(),
            request.packet().attempt(),
            ManagedEvidenceKind::GitSnapshot,
            "application/json",
            "lattice.managed-worktree-baseline/1.0",
            "managed-test-worktree",
            "1.0",
            digest('8'),
            "2026-08-26T02:00:00Z",
            br#"{"schema":"lattice.managed-worktree-baseline/1.0"}"#.to_vec(),
        )
        .expect("baseline input"),
    )
    .expect("baseline evidence");
    let packet = packet_with_worktree(
        request.binding(),
        &worker_budget(),
        baseline.content_digest(),
    );
    ManagedAttemptRequest::new(
        request.binding().clone(),
        packet,
        request.authority_digest().clone(),
    )
    .expect("baseline request")
    .with_predispatch_baseline(baseline)
    .expect("baseline-bound request")
}

fn with_wsl_execution_environment(request: ManagedAttemptRequest) -> ManagedAttemptRequest {
    let packet = request
        .packet()
        .clone()
        .with_execution_environment_ref(&format!(
            "execution-environment:sha256:{}",
            digest('e').as_str()
        ))
        .expect("WSL execution environment");
    ManagedAttemptRequest::new(
        request.binding().clone(),
        packet,
        request.authority_digest().clone(),
    )
    .expect("WSL request")
}

fn execution_preflight(request: &ManagedAttemptRequest) -> VerifiedManagedEvidence {
    let bytes = format!(
        concat!(
            "{{\"attempt\":{},",
            "\"effect_counters\":{{\"provider_effect_count\":0,\"thread_start\":0,\"turn_start\":0}},",
            "\"execution_environment_ref\":\"{}\",",
            "\"linux_cwd\":\"/home/zk/lattice/tasks/work-e\",",
            "\"provider_effect_count\":0,",
            "\"repository_head\":\"{}\",",
            "\"schema\":\"lattice.wsl2-zero-model-preflight/1.0\",",
            "\"status\":\"PASS\",",
            "\"task_ref\":\"{}\",",
            "\"worktree_ref\":\"{}\"}}"
        ),
        request.packet().attempt(),
        request.packet().execution_environment_ref(),
        request.packet().base_commit(),
        request.binding().task_ref().as_str(),
        request.packet().worktree_ref(),
    )
    .into_bytes();
    VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            ProjectId::new("project-1").expect("project"),
            request.binding().task_ref().clone(),
            request.packet().attempt(),
            ManagedEvidenceKind::WorkerLifecycle,
            "application/json",
            "lattice.wsl2-zero-model-preflight/1.0",
            "managed-test-wsl2-preflight",
            "1.0",
            digest('7'),
            "2026-08-26T02:00:00Z",
            bytes,
        )
        .expect("preflight input"),
    )
    .expect("preflight evidence")
}

#[derive(Debug)]
struct FakeRepository {
    calls: Vec<&'static str>,
    stream: VerifiedStream,
    attempts: Vec<VerifiedWorkerAttemptRecord>,
    observations: Vec<VerifiedWorkerObservationRecord>,
    verifications: Vec<VerifiedTaskVerificationRecord>,
    authority_digest: ContentDigest,
    authority_failure_code: Option<&'static str>,
    claim_disposition: ManagedAttemptClaimDisposition,
    thread_claim_disposition: ManagedWorkerThreadDispatchDisposition,
    turn_claim_disposition: ManagedWorkerTurnDispatchDisposition,
    review_claim_disposition: ManagedReviewDispatchDisposition,
    dispatch_state: ManagedWorkerDispatchState,
    closure_disposition: ManagedPrestartClosureDisposition,
    sequence: u8,
    execution_trace: Rc<RefCell<Vec<&'static str>>>,
}

impl FakeRepository {
    fn next_metadata(&mut self, prefix: &str) -> TaskRuntimeAppendMetadata {
        self.sequence += 1;
        metadata(&format!("{prefix}-{}", self.sequence), self.sequence)
    }
}

impl ManagedForemanRepositoryPort for FakeRepository {
    fn assert_execution_authority_current(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        authority_digest: &ContentDigest,
    ) -> Result<(), ManagedPortError> {
        self.calls.push("authority");
        if let Some(code) = self.authority_failure_code {
            return Err(ManagedPortError::new(ManagedPortErrorKind::Known, code));
        }
        if authority_digest == &self.authority_digest {
            Ok(())
        } else {
            Err(ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "AUTHORITY_NOT_CURRENT",
            ))
        }
    }

    fn claim_attempt(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        packet: &AttemptPacketIdentity,
    ) -> Result<ManagedAttemptClaim, ManagedPortError> {
        self.calls.push("claim");
        let input = WorkerAttemptInput::new(
            lattice_contracts::AttemptId::new(format!("attempt-{}", packet.attempt()))
                .expect("attempt"),
            u64::from(packet.attempt()),
            u64::from(packet.attempt()),
            packet.model_selection().model(),
            packet.model_selection().reasoning(),
            packet.model_selection().reason(),
            packet.writer_fence(),
            digest('1'),
            self.authority_digest.clone(),
            pointer_content(packet.digest()),
            pointer_content(packet.worktree_ref()),
            digest('5'),
            pointer_content(packet.model_selection().digest()),
        )
        .expect("attempt input");
        let metadata = self.next_metadata("claim");
        let plan = plan_worker_attempt_append(
            &self.stream,
            binding,
            &self.attempts,
            &self.observations,
            metadata,
            input,
        )
        .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Known, "CLAIM_REJECTED"))?;
        let record = plan.record().clone();
        self.stream = apply_append_plan(&self.stream, plan.ledger_plan())
            .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Ambiguous, "CLAIM_UNKNOWN"))?;
        if let Some(new) = plan.new_record() {
            self.attempts.push(new.clone());
        }
        Ok(ManagedAttemptClaim::new(record, self.claim_disposition))
    }

    fn record_observation(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        observation: &ManagedWorkerObservation,
    ) -> Result<VerifiedWorkerObservationRecord, ManagedPortError> {
        self.calls.push("record-observation");
        if let Some(existing) = self.observations.iter().find(|existing| {
            existing.attempt_number() == attempt.attempt_number()
                && existing.kind() == observation.kind()
                && existing.thread_id() == observation.thread_id()
                && existing.turn_id() == observation.turn_id()
                && existing.app_server_generation() == observation.app_server_generation()
                && existing.app_server_identity_digest() == observation.app_server_identity_digest()
                && existing.evidence_digest() == observation.evidence_digest()
        }) {
            return Ok(existing.clone());
        }
        if matches!(
            observation.kind(),
            lattice_ports::WorkerObservationKind::MeaningfulProgress
                | lattice_ports::WorkerObservationKind::Heartbeat
        ) {
            self.execution_trace.borrow_mut().push("durable-progress");
        }
        let metadata = self.next_metadata("observation");
        let plan = plan_worker_observation_append(
            &self.stream,
            binding,
            &self.attempts,
            &self.observations,
            metadata,
            observation.ledger_input().clone(),
        )
        .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Known, "OBSERVATION_REJECTED"))?;
        let record = plan.record().clone();
        self.stream = apply_append_plan(&self.stream, plan.ledger_plan()).map_err(|_| {
            ManagedPortError::new(ManagedPortErrorKind::Ambiguous, "OBSERVATION_UNKNOWN")
        })?;
        if let Some(new) = plan.new_record() {
            self.observations.push(new.clone());
        }
        assert_eq!(record.attempt_id(), attempt.attempt_id());
        Ok(record)
    }

    fn claim_worker_thread_dispatch(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
    ) -> Result<ManagedWorkerThreadDispatchDisposition, ManagedPortError> {
        self.calls.push("claim-thread");
        Ok(self.thread_claim_disposition)
    }

    fn claim_worker_turn_dispatch(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        thread: &VerifiedWorkerObservationRecord,
    ) -> Result<ManagedWorkerTurnDispatchDisposition, ManagedPortError> {
        self.calls.push("claim-turn");
        if thread.kind() != lattice_ports::WorkerObservationKind::ThreadAccepted {
            return Err(ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "TURN_DISPATCH_THREAD_REJECTED",
            ));
        }
        Ok(self.turn_claim_disposition)
    }

    fn load_worker_dispatch_state(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
    ) -> Result<ManagedWorkerDispatchState, ManagedPortError> {
        self.calls.push("load-dispatch-state");
        Ok(self.dispatch_state)
    }

    fn close_prestart_without_provider_effect(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        _proof: &ManagedPrestartNoEffectProof,
        _blocker_code: &'static str,
    ) -> Result<ManagedPrestartClosureDisposition, ManagedPortError> {
        self.calls.push("close-prestart");
        Ok(self.closure_disposition)
    }

    fn claim_review_dispatch(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        _terminal: &VerifiedWorkerObservationRecord,
        _request: &ManagedVerificationRequest,
    ) -> Result<ManagedReviewDispatchDisposition, ManagedPortError> {
        self.calls.push("claim-review");
        Ok(self.review_claim_disposition)
    }

    fn claim_review_turn_dispatch(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        _request: &ManagedVerificationRequest,
        _thread_lifecycle: &VerifiedManagedEvidence,
    ) -> Result<ManagedReviewDispatchDisposition, ManagedPortError> {
        self.calls.push("claim-review-turn");
        Ok(self.review_claim_disposition)
    }

    fn record_verification(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        evidence: &ManagedVerificationEvidence,
    ) -> Result<VerifiedTaskVerificationRecord, ManagedPortError> {
        self.calls.push("record-verification");
        let request = evidence.request();
        let input = TaskVerificationInput::new(
            1,
            evidence.outcome(),
            request.profile_identity().clone(),
            request.base_commit_digest().clone(),
            request.result_commit_digest().clone(),
            request.tree_digest().clone(),
            request.diff_digest().clone(),
            evidence.result_digest().clone(),
            request.evidence_artifact_digest().clone(),
            evidence.review_digest().cloned(),
        )
        .expect("verification input");
        let metadata = self.next_metadata("verification");
        let plan = plan_task_verification_append(
            &self.stream,
            binding,
            &self.attempts,
            &self.observations,
            &self.verifications,
            metadata,
            input,
        )
        .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Known, "VERIFICATION_REJECTED"))?;
        let record = plan.record().clone();
        self.stream = apply_append_plan(&self.stream, plan.ledger_plan()).map_err(|_| {
            ManagedPortError::new(ManagedPortErrorKind::Ambiguous, "VERIFICATION_UNKNOWN")
        })?;
        if let Some(new) = plan.new_record() {
            self.verifications.push(new.clone());
        }
        Ok(record)
    }

    fn record_artifact(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        evidence: &VerifiedManagedEvidence,
    ) -> Result<ManagedArtifactReceipt, ManagedPortError> {
        self.calls.push("record-artifact");
        if evidence.payload_schema() == "lattice.wsl2-provider-subtree-marker/1.0" {
            self.execution_trace
                .borrow_mut()
                .push("durable-provider-open");
        }
        ManagedArtifactReceipt::new(evidence, digest('9'))
    }
}

#[derive(Debug, Default)]
struct FakeProviderGuard {
    calls: usize,
    fail_on_call: Option<usize>,
}

impl ManagedProviderEffectGuardPort for FakeProviderGuard {
    fn assert_provider_effect_writer_current(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
    ) -> Result<(), ManagedPortError> {
        self.calls += 1;
        if self.fail_on_call == Some(self.calls) {
            return Err(ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                "WRITER_ROTATED_BEFORE_PROVIDER_RPC",
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct FakeWorker {
    calls: Vec<&'static str>,
    exact_start: bool,
    availability: ManagedModelAvailability,
    reconciliations: std::collections::VecDeque<ManagedWorkerReconciliation>,
    prestart_recoveries: std::collections::VecDeque<ManagedWorkerPrestartRecovery>,
    terminal: WorkerTerminal,
    execution_trace: Rc<RefCell<Vec<&'static str>>>,
}

impl ManagedCodexWorkerPort for FakeWorker {
    fn model_availability(
        &mut self,
        _selection: &ModelSelection,
    ) -> Result<ManagedModelAvailability, ManagedPortError> {
        if self.calls.contains(&"provider-open") {
            self.execution_trace
                .borrow_mut()
                .push("prepared-provider-model-read");
        }
        self.calls.push("model");
        Ok(self.availability)
    }

    fn prepare_provider_dispatch(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        _packet: &AttemptPacketIdentity,
    ) -> Result<VerifiedManagedEvidence, ManagedPortError> {
        self.calls.push("provider-open");
        self.execution_trace.borrow_mut().push("provider-open");
        let attempt_number = u8::try_from(attempt.attempt_number()).expect("bounded attempt");
        VerifiedManagedEvidence::new(
            ManagedEvidenceInput::new(
                ProjectId::new("project-1").expect("project"),
                attempt.task_ref().clone(),
                attempt_number,
                ManagedEvidenceKind::WorkerLifecycle,
                "application/json",
                "lattice.wsl2-provider-subtree-marker/1.0",
                "managed-test-provider-subtree",
                "1.0",
                digest('7'),
                "2026-08-26T02:00:00Z",
                br#"{"schema":"lattice.wsl2-provider-subtree-marker/1.0","status":"OPEN"}"#
                    .to_vec(),
            )
            .expect("provider marker input"),
        )
        .map_err(|_| {
            ManagedPortError::new(ManagedPortErrorKind::Known, "TEST_PROVIDER_MARKER_REJECTED")
        })
    }

    fn start_thread(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        _packet: &AttemptPacketIdentity,
    ) -> Result<ManagedWorkerObservation, ManagedPortError> {
        self.calls.push("thread-accepted");
        ManagedWorkerObservation::thread_accepted(
            attempt.attempt_number(),
            "thread-1",
            1,
            app_server_identity(),
            digest('7'),
        )
    }

    fn start_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
    ) -> Result<ManagedWorkerObservation, ManagedPortError> {
        self.calls.push("turn-accepted");
        ManagedWorkerObservation::turn_accepted(
            attempt.attempt_number(),
            thread_id,
            "turn-1",
            1,
            app_server_identity(),
            digest('8'),
        )
    }

    fn wait_exact_started(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<ManagedWorkerObservation, ManagedPortError> {
        self.calls.push("exact-started");
        if self.exact_start {
            ManagedWorkerObservation::exact_started(
                attempt.attempt_number(),
                thread_id,
                turn_id,
                1,
                app_server_identity(),
                "2026-08-26T02:00:04Z",
                digest('9'),
            )
        } else {
            ManagedWorkerObservation::turn_accepted(
                attempt.attempt_number(),
                thread_id,
                turn_id,
                1,
                app_server_identity(),
                digest('9'),
            )
        }
    }

    fn next_execution_event(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<ManagedWorkerExecutionEvent, ManagedPortError> {
        self.execution_trace.borrow_mut().push("provider-poll");
        if !self.calls.contains(&"execution") && !self.calls.contains(&"interrupt") {
            self.calls.push("execution");
            return ManagedWorkerObservation::meaningful_progress(
                attempt.attempt_number(),
                thread_id,
                turn_id,
                1,
                app_server_identity(),
                digest('a'),
            )
            .map(ManagedWorkerExecutionEvent::Observation);
        }
        self.calls.push("terminal");
        ManagedTerminalCandidate::new(ManagedWorkerObservation::terminal(
            attempt.attempt_number(),
            thread_id,
            turn_id,
            self.terminal,
            1,
            app_server_identity(),
            digest('c'),
        )?)
        .map(ManagedWorkerExecutionEvent::Terminal)
    }

    fn recover_claimed_dispatch(
        &mut self,
        _attempt: &VerifiedWorkerAttemptRecord,
        _packet: &AttemptPacketIdentity,
    ) -> Result<ManagedWorkerPrestartRecovery, ManagedPortError> {
        self.calls.push("recover-dispatch");
        Ok(self
            .prestart_recoveries
            .pop_front()
            .unwrap_or(ManagedWorkerPrestartRecovery::ReconciliationRequired))
    }

    fn recover_prestart(
        &mut self,
        _attempt: &VerifiedWorkerAttemptRecord,
        _thread_id: &str,
        _turn_id: Option<&str>,
    ) -> Result<ManagedWorkerPrestartRecovery, ManagedPortError> {
        self.calls.push("recover-prestart");
        Ok(self
            .prestart_recoveries
            .pop_front()
            .unwrap_or(ManagedWorkerPrestartRecovery::ReconciliationRequired))
    }

    fn read_exact_thread(
        &mut self,
        _attempt: &VerifiedWorkerAttemptRecord,
        _thread_id: &str,
    ) -> Result<lattice_ports::ManagedWorkerReconciliation, ManagedPortError> {
        self.calls.push("read-thread");
        self.reconciliations.pop_front().ok_or_else(|| {
            ManagedPortError::new(ManagedPortErrorKind::ReconcileRequired, "READ_THREAD_EMPTY")
        })
    }

    fn read_exact_turn(
        &mut self,
        _attempt: &VerifiedWorkerAttemptRecord,
        _thread_id: &str,
        _turn_id: &str,
    ) -> Result<lattice_ports::ManagedWorkerReconciliation, ManagedPortError> {
        self.calls.push("read-turn");
        self.reconciliations.pop_front().ok_or_else(|| {
            ManagedPortError::new(ManagedPortErrorKind::ReconcileRequired, "READ_TURN_EMPTY")
        })
    }

    fn resume_exact_turn(
        &mut self,
        _attempt: &VerifiedWorkerAttemptRecord,
        _thread_id: &str,
        _turn_id: &str,
    ) -> Result<lattice_ports::ManagedWorkerReconciliation, ManagedPortError> {
        self.calls.push("resume-turn");
        self.reconciliations.pop_front().ok_or_else(|| {
            ManagedPortError::new(ManagedPortErrorKind::ReconcileRequired, "RESUME_EMPTY")
        })
    }

    fn reconcile_exact_turn(
        &mut self,
        _attempt: &VerifiedWorkerAttemptRecord,
        _thread_id: &str,
        _turn_id: &str,
    ) -> Result<lattice_ports::ManagedWorkerReconciliation, ManagedPortError> {
        self.calls.push("reconcile-turn");
        Ok(self
            .reconciliations
            .pop_front()
            .unwrap_or(ManagedWorkerReconciliation::Unresolved))
    }

    fn interrupt_exact_turn(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<ManagedWorkerObservation, ManagedPortError> {
        self.calls.push("interrupt");
        ManagedWorkerObservation::interrupt_requested(
            attempt.attempt_number(),
            thread_id,
            turn_id,
            1,
            app_server_identity(),
            digest('d'),
        )
    }
}

#[derive(Debug)]
struct FakeVerifier {
    calls: Vec<&'static str>,
    outcome: VerificationOutcome,
    authorize_turn: bool,
    fail_prepare_with_evidence: bool,
}

impl ManagedVerificationPort for FakeVerifier {
    fn prepare(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        _terminal: &VerifiedWorkerObservationRecord,
    ) -> Result<ManagedVerificationPreparation, ManagedPortError> {
        self.calls.push("prepare");
        if self.fail_prepare_with_evidence {
            return Err(ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "LATTICE_MANAGED_VERIFIER_GIT_REV_VERIFY_FAILED",
            ));
        }
        let artifact = VerifiedManagedEvidence::new(
            ManagedEvidenceInput::new(
                ProjectId::new("project-1").expect("project"),
                binding.task_ref().clone(),
                u8::try_from(attempt.attempt_number()).expect("attempt u8"),
                ManagedEvidenceKind::GitSnapshot,
                "application/json",
                "lattice.verification-snapshot/1.0",
                "managed-test-verifier",
                "1.0",
                digest('9'),
                "2026-08-26T02:00:20Z",
                br#"{"commit":"candidate"}"#.to_vec(),
            )
            .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Known, "EVIDENCE_INPUT"))?,
        )
        .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Known, "EVIDENCE_VERIFY"))?;
        let request = ManagedVerificationRequest::new(
            digest('f'),
            digest('1'),
            digest('5'),
            digest('2'),
            digest('3'),
            digest('4'),
            digest('a'),
            &artifact,
        )?;
        ManagedVerificationPreparation::new(binding, attempt, artifact, request)
    }

    fn preparation_failure_evidence(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        _terminal: &VerifiedWorkerObservationRecord,
        failure: &ManagedPortError,
    ) -> Result<Option<VerifiedManagedEvidence>, ManagedPortError> {
        self.calls.push("prepare-failure-evidence");
        if !self.fail_prepare_with_evidence {
            return Ok(None);
        }
        let bytes = format!(
            "{{\"schema\":\"lattice.managed-wsl2-git-transport-failure/1.0\",\"failure_code\":\"{}\",\"provider_effect_count\":0}}",
            failure.code()
        )
        .into_bytes();
        let input = ManagedEvidenceInput::new(
            ProjectId::new("project-1").expect("project"),
            binding.task_ref().clone(),
            u8::try_from(attempt.attempt_number()).expect("attempt u8"),
            ManagedEvidenceKind::VerificationResult,
            "application/json",
            "lattice.managed-wsl2-git-transport-failure/1.0",
            "managed-test-verifier",
            "1.0",
            digest('9'),
            "2026-08-26T02:00:20Z",
            bytes,
        )
        .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Known, "EVIDENCE_INPUT"))?;
        VerifiedManagedEvidence::new(input)
            .map(Some)
            .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Known, "EVIDENCE_VERIFY"))
    }

    fn review(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        _terminal: &VerifiedWorkerObservationRecord,
        _request: &ManagedVerificationRequest,
        sink: &mut dyn lattice_ports::ManagedReviewEvidenceSink,
    ) -> Result<(), ManagedPortError> {
        self.calls.push("review");
        if self.authorize_turn {
            let input = ManagedEvidenceInput::new(
                ProjectId::new("project-1").expect("project"),
                binding.task_ref().clone(),
                u8::try_from(attempt.attempt_number()).expect("attempt u8"),
                ManagedEvidenceKind::WorkerLifecycle,
                "application/json",
                "lattice.managed-review-lifecycle/1.0",
                "managed-test-reviewer",
                "1.0",
                digest('8'),
                "2026-08-26T02:00:21Z",
                br#"{"event":"thread_started"}"#.to_vec(),
            )
            .map_err(|_| ManagedPortError::new(ManagedPortErrorKind::Known, "EVIDENCE_INPUT"))?;
            let lifecycle = VerifiedManagedEvidence::new(input).map_err(|_| {
                ManagedPortError::new(ManagedPortErrorKind::Known, "EVIDENCE_VERIFY")
            })?;
            sink.record(&lifecycle)?;
            if sink.authorize_turn_start(&lifecycle)? != ManagedReviewDispatchDisposition::Claimed {
                return Err(ManagedPortError::new(
                    ManagedPortErrorKind::ReconcileRequired,
                    "REVIEW_TURN_NOT_AUTHORIZED",
                ));
            }
        }
        Ok(())
    }

    fn verify(
        &mut self,
        _binding: &VerifiedTaskExecutionBinding,
        _attempt: &VerifiedWorkerAttemptRecord,
        _terminal: &VerifiedWorkerObservationRecord,
        request: &ManagedVerificationRequest,
    ) -> Result<ManagedVerificationEvidence, ManagedPortError> {
        self.calls.push("verify");
        ManagedVerificationEvidence::new(
            request.clone(),
            self.outcome,
            digest('d'),
            Some(digest('e')),
        )
    }
}

fn run_managed_attempt(
    request: &ManagedAttemptRequest,
    repository: &mut FakeRepository,
    worker: &mut FakeWorker,
    verifier: &mut FakeVerifier,
) -> Result<lattice_orchestrator::ManagedAttemptOutcome, ManagedAttemptOrchestratorError> {
    run_managed_attempt_with_guard(
        request,
        repository,
        worker,
        verifier,
        &mut FakeProviderGuard::default(),
    )
}

fn prepare_managed_attempt(
    request: &ManagedAttemptRequest,
    repository: &mut FakeRepository,
    worker: &mut FakeWorker,
) -> Result<lattice_orchestrator::ManagedStartingAttempt, ManagedAttemptOrchestratorError> {
    prepare_managed_attempt_with_guard(
        request,
        repository,
        worker,
        &mut FakeProviderGuard::default(),
    )
}

fn recover_managed_prestart_on_restart(
    request: &ManagedAttemptRequest,
    attempt: &VerifiedWorkerAttemptRecord,
    retained_state: &WorkerAttemptState,
    repository: &mut FakeRepository,
    worker: &mut FakeWorker,
) -> Result<ManagedPrestartRestartOutcome, ManagedAttemptOrchestratorError> {
    recover_managed_prestart_on_restart_with_guard(
        request,
        attempt,
        retained_state,
        repository,
        worker,
        &mut FakeProviderGuard::default(),
    )
}

fn fixture(
    exact_start: bool,
) -> (
    ManagedAttemptRequest,
    FakeRepository,
    FakeWorker,
    FakeVerifier,
) {
    let (stream, binding) = lineage();
    let budget = worker_budget();
    let packet = packet(&binding, &budget);
    let authority_digest = digest('2');
    let request =
        ManagedAttemptRequest::new(binding, packet, authority_digest.clone()).expect("request");
    let execution_trace = Rc::new(RefCell::new(Vec::new()));
    (
        request,
        FakeRepository {
            calls: Vec::new(),
            stream,
            attempts: Vec::new(),
            observations: Vec::new(),
            verifications: Vec::new(),
            authority_digest,
            authority_failure_code: None,
            claim_disposition: ManagedAttemptClaimDisposition::Claimed,
            thread_claim_disposition: ManagedWorkerThreadDispatchDisposition::Claimed,
            turn_claim_disposition: ManagedWorkerTurnDispatchDisposition::Claimed,
            review_claim_disposition: ManagedReviewDispatchDisposition::Claimed,
            dispatch_state: ManagedWorkerDispatchState::NoWorkerThread,
            closure_disposition: ManagedPrestartClosureDisposition::Closed,
            sequence: 0,
            execution_trace: execution_trace.clone(),
        },
        FakeWorker {
            calls: Vec::new(),
            exact_start,
            availability: ManagedModelAvailability::Available,
            reconciliations: std::collections::VecDeque::new(),
            prestart_recoveries: std::collections::VecDeque::new(),
            terminal: WorkerTerminal::Completed,
            execution_trace,
        },
        FakeVerifier {
            calls: Vec::new(),
            outcome: VerificationOutcome::Passed,
            authorize_turn: false,
            fail_prepare_with_evidence: false,
        },
    )
}

#[test]
fn observation_replay_with_substituted_app_server_identity_fails_closed() {
    let (request, mut repository, _, _) = fixture(true);
    repository
        .assert_execution_authority_current(request.binding(), request.authority_digest())
        .expect("authority");
    let attempt = repository
        .claim_attempt(request.binding(), request.packet())
        .expect("claim")
        .into_attempt();
    let original = ManagedWorkerObservation::thread_accepted(
        attempt.attempt_number(),
        "thread-identity-replay",
        1,
        app_server_identity(),
        digest('7'),
    )
    .expect("original observation");
    repository
        .record_observation(request.binding(), &attempt, &original)
        .expect("record original observation");

    let substituted = ManagedWorkerObservation::thread_accepted(
        attempt.attempt_number(),
        "thread-identity-replay",
        1,
        digest('e'),
        digest('7'),
    )
    .expect("substituted observation");
    let failure = repository
        .record_observation(request.binding(), &attempt, &substituted)
        .expect_err("substituted App Server identity must not exact-replay");

    assert_eq!(failure.code(), "OBSERVATION_REJECTED");
    assert_eq!(repository.observations.len(), 1);
}

#[test]
fn exact_started_orders_execution_and_pass_stops_at_merge_approval() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    let outcome = run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier)
        .expect("managed attempt");

    assert_eq!(
        outcome.target(),
        ManagedAttemptTarget::AwaitingMergeApproval
    );
    assert_eq!(
        repository.calls,
        vec![
            "authority",
            "claim",
            "claim-thread",
            "record-observation",
            "claim-turn",
            "record-observation",
            "record-observation",
            "record-observation",
            "record-observation",
            "record-artifact",
            "claim-review",
            "record-verification",
        ]
    );
    assert_eq!(
        worker.calls,
        vec![
            "model",
            "thread-accepted",
            "turn-accepted",
            "exact-started",
            "execution",
            "terminal",
        ]
    );
    assert_eq!(verifier.calls, vec!["prepare", "review", "verify"]);
    assert_eq!(
        repository.execution_trace.borrow().as_slice(),
        ["provider-poll", "durable-progress", "provider-poll"]
    );
}

#[test]
fn task_owned_baseline_is_durable_before_the_first_provider_rpc() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    let baseline = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            ProjectId::new("project-1").expect("project"),
            request.binding().task_ref().clone(),
            1,
            ManagedEvidenceKind::GitSnapshot,
            "application/json",
            "lattice.managed-worktree-baseline/1.0",
            "managed-test-worktree",
            "1.0",
            digest('8'),
            "2026-08-26T02:00:00Z",
            br#"{"schema":"lattice.managed-worktree-baseline/1.0"}"#.to_vec(),
        )
        .expect("baseline input"),
    )
    .expect("baseline evidence");
    let packet = packet_with_worktree(
        request.binding(),
        &worker_budget(),
        baseline.content_digest(),
    );
    let request = ManagedAttemptRequest::new(
        request.binding().clone(),
        packet,
        request.authority_digest().clone(),
    )
    .expect("request")
    .with_predispatch_baseline(baseline)
    .expect("baseline-bound request");

    run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier)
        .expect("managed attempt");
    assert_eq!(
        &repository.calls[..3],
        &["authority", "claim", "record-artifact"]
    );
    assert_eq!(worker.calls.first(), Some(&"model"));
    assert_eq!(
        repository
            .calls
            .iter()
            .filter(|call| **call == "record-artifact")
            .count(),
        2,
        "baseline and final candidate snapshots are separate Artifact Store objects",
    );
}

#[test]
fn wsl_attempt_requires_a_bound_zero_effect_preflight_before_model_or_claim() {
    let (request, mut repository, mut worker, _) = fixture(true);
    let request = with_wsl_execution_environment(request);
    let failure = prepare_managed_attempt(&request, &mut repository, &mut worker)
        .expect_err("WSL execution cannot start without its exact preflight receipt");

    assert_eq!(
        failure,
        ManagedAttemptOrchestratorError::ExecutionPreflightRequired
    );
    assert!(repository.calls.is_empty());
    assert!(worker.calls.is_empty());
}

#[test]
fn wsl_preflight_substitution_and_nonzero_provider_effect_fail_closed() {
    let (request, _, _, _) = fixture(true);
    let request = with_wsl_execution_environment(request);
    let exact = execution_preflight(&request);
    let substituted = String::from_utf8(exact.bytes().to_vec())
        .expect("utf8")
        .replace(
            request.packet().execution_environment_ref(),
            &format!("execution-environment:sha256:{}", digest('a').as_str()),
        );
    let substituted = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            exact.project_id().clone(),
            exact.task_ref().clone(),
            exact.attempt(),
            exact.kind(),
            exact.media_type(),
            exact.payload_schema(),
            exact.producer_id(),
            exact.producer_version(),
            exact.producer_digest().clone(),
            exact.created_at(),
            substituted.into_bytes(),
        )
        .expect("substituted input"),
    )
    .expect("substituted evidence");
    assert_eq!(
        request
            .clone()
            .with_execution_preflight(substituted)
            .expect_err("environment substitution must fail"),
        ManagedAttemptOrchestratorError::ExecutionPreflightMismatch
    );

    let nonzero = String::from_utf8(exact.bytes().to_vec())
        .expect("utf8")
        .replacen(
            "\"provider_effect_count\":0",
            "\"provider_effect_count\":1",
            1,
        );
    let nonzero = VerifiedManagedEvidence::new(
        ManagedEvidenceInput::new(
            exact.project_id().clone(),
            exact.task_ref().clone(),
            exact.attempt(),
            exact.kind(),
            exact.media_type(),
            exact.payload_schema(),
            exact.producer_id(),
            exact.producer_version(),
            exact.producer_digest().clone(),
            exact.created_at(),
            nonzero.into_bytes(),
        )
        .expect("nonzero input"),
    )
    .expect("nonzero evidence");
    assert_eq!(
        request
            .with_execution_preflight(nonzero)
            .expect_err("provider effect must fail"),
        ManagedAttemptOrchestratorError::ExecutionPreflightMismatch
    );
}

#[test]
fn exact_wsl_preflight_is_durable_before_the_first_provider_dispatch() {
    let (request, mut repository, mut worker, _) = fixture(true);
    let request = with_wsl_execution_environment(request);
    let preflight = execution_preflight(&request);
    let request = request
        .with_execution_preflight(preflight)
        .expect("exact preflight");

    prepare_managed_attempt(&request, &mut repository, &mut worker).expect("preflight-bound start");

    assert_eq!(
        &repository.calls[..4],
        &["authority", "claim", "record-artifact", "claim-thread"]
    );
    assert_eq!(
        &repository.calls[..5],
        &[
            "authority",
            "claim",
            "record-artifact",
            "claim-thread",
            "record-artifact",
        ]
    );
    assert_eq!(worker.calls[0], "provider-open");
    assert_eq!(worker.calls[1], "model");
    assert_eq!(worker.calls[2], "thread-accepted");
    assert_eq!(
        repository.execution_trace.borrow().as_slice(),
        [
            "provider-open",
            "durable-provider-open",
            "prepared-provider-model-read",
        ]
    );
}

#[test]
fn accepted_without_exact_started_suppresses_execution_terminal_and_verification() {
    let (request, mut repository, mut worker, mut verifier) = fixture(false);
    let error = run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier)
        .expect_err("exact start must gate execution");

    assert_eq!(
        error,
        ManagedAttemptOrchestratorError::ExactStartNotConfirmed
    );
    assert_eq!(
        worker.calls,
        vec!["model", "thread-accepted", "turn-accepted", "exact-started"]
    );
    assert!(verifier.calls.is_empty());
}

#[test]
fn unavailable_model_is_fail_closed_before_authority_and_atomic_claim() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    worker.availability = ManagedModelAvailability::Unavailable {
        code: "TERRA_UNAVAILABLE",
    };

    assert_eq!(
        run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier),
        Err(ManagedAttemptOrchestratorError::ModelUnavailable {
            code: "TERRA_UNAVAILABLE",
        })
    );
    assert_eq!(worker.calls, vec!["model"]);
    assert!(repository.calls.is_empty());
    assert!(verifier.calls.is_empty());
}

#[test]
fn exact_replay_claim_never_authorizes_a_second_provider_dispatch() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    repository.claim_disposition = ManagedAttemptClaimDisposition::ExactReplay;

    assert_eq!(
        run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier),
        Err(ManagedAttemptOrchestratorError::DispatchReconciliationRequired)
    );
    assert_eq!(repository.calls, vec!["authority", "claim"]);
    assert_eq!(worker.calls, vec!["model"]);
    assert!(verifier.calls.is_empty());
}

#[test]
fn exact_replay_thread_claim_never_authorizes_a_second_thread_start() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    repository.thread_claim_disposition = ManagedWorkerThreadDispatchDisposition::ExactReplay;

    assert_eq!(
        run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier),
        Err(ManagedAttemptOrchestratorError::DispatchReconciliationRequired)
    );
    assert_eq!(repository.calls, vec!["authority", "claim", "claim-thread"]);
    assert_eq!(worker.calls, vec!["model"]);
    assert!(verifier.calls.is_empty());
}

#[test]
fn exact_replay_turn_claim_never_authorizes_a_second_turn_start() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    repository.turn_claim_disposition = ManagedWorkerTurnDispatchDisposition::ExactReplay;

    assert_eq!(
        run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier),
        Err(ManagedAttemptOrchestratorError::TurnDispatchReconciliationRequired)
    );
    assert_eq!(
        repository.calls,
        vec![
            "authority",
            "claim",
            "claim-thread",
            "record-observation",
            "claim-turn"
        ]
    );
    assert_eq!(worker.calls, vec!["model", "thread-accepted"]);
    assert!(verifier.calls.is_empty());
}

fn claimed_prestart_attempt(
    request: &ManagedAttemptRequest,
    repository: &mut FakeRepository,
) -> (VerifiedWorkerAttemptRecord, WorkerAttemptState) {
    repository
        .assert_execution_authority_current(request.binding(), request.authority_digest())
        .expect("authority");
    let attempt = repository
        .claim_attempt(request.binding(), request.packet())
        .expect("claim")
        .into_attempt();
    let state = WorkerAttemptState::new(request.packet().clone()).expect("state");
    (attempt, state)
}

#[test]
fn restart_recovery_exact_empty_starts_only_one_separately_claimed_turn() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let request = with_predispatch_baseline(request);
    let (attempt, state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    worker.calls.clear();
    repository.dispatch_state = ManagedWorkerDispatchState::WorkerThreadClaimed;
    worker
        .prestart_recoveries
        .push_back(ManagedWorkerPrestartRecovery::ExactEmptyThread {
            thread: ManagedWorkerObservation::thread_accepted(
                attempt.attempt_number(),
                "thread-recovered",
                1,
                app_server_identity(),
                digest('6'),
            )
            .expect("thread recovery"),
        });

    let outcome = recover_managed_prestart_on_restart(
        &request,
        &attempt,
        &state,
        &mut repository,
        &mut worker,
    )
    .expect("restart recovery");

    let ManagedPrestartRestartOutcome::NoProviderEffect(proof) = outcome else {
        panic!("unexpected recovery: {outcome:?}");
    };
    let outcome = continue_managed_prestart_on_restart(
        &request,
        &attempt,
        &state,
        &proof,
        &mut repository,
        &mut worker,
        &mut FakeProviderGuard::default(),
    )
    .expect("guarded continuation");
    assert!(matches!(
        outcome,
        ManagedPrestartRestartOutcome::Starting(_)
    ));
    assert_eq!(
        worker.calls,
        vec!["recover-dispatch", "model", "turn-accepted"]
    );
    assert_eq!(
        worker
            .calls
            .iter()
            .filter(|call| **call == "thread-accepted")
            .count(),
        0
    );
    assert_eq!(
        worker
            .calls
            .iter()
            .filter(|call| **call == "turn-accepted")
            .count(),
        1
    );
    assert_eq!(
        repository.calls,
        vec![
            "load-dispatch-state",
            "record-observation",
            "authority",
            "record-artifact",
            "claim-turn",
            "record-observation"
        ]
    );
}

#[test]
fn restart_exact_replay_thread_claim_without_provider_candidate_requires_reconciliation() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let request = with_predispatch_baseline(request);
    let (attempt, state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    worker.calls.clear();
    repository.thread_claim_disposition = ManagedWorkerThreadDispatchDisposition::ExactReplay;
    repository.dispatch_state = ManagedWorkerDispatchState::WorkerThreadClaimed;
    worker
        .prestart_recoveries
        .push_back(ManagedWorkerPrestartRecovery::ProvenNoProviderCandidate);

    assert!(matches!(
        recover_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &mut repository,
            &mut worker,
        )
        .expect("claimed thread remains reconciliation-only at recovery"),
        ManagedPrestartRestartOutcome::ReconciliationRequired
    ));
    assert_eq!(worker.calls, vec!["recover-dispatch"]);
    assert_eq!(repository.calls, vec!["load-dispatch-state"]);
    assert!(
        !worker
            .calls
            .iter()
            .any(|call| matches!(*call, "thread-accepted" | "turn-accepted"))
    );
}

#[test]
fn restart_proven_no_candidate_with_fresh_claim_starts_one_thread_and_one_turn() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let request = with_predispatch_baseline(request);
    let (attempt, state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    worker.calls.clear();
    repository.dispatch_state = ManagedWorkerDispatchState::NoWorkerThread;
    let ManagedPrestartRestartOutcome::NoProviderEffect(proof) =
        recover_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &mut repository,
            &mut worker,
        )
        .expect("durable no-thread proof")
    else {
        panic!("expected typed no-effect proof");
    };
    assert!(matches!(
        continue_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &proof,
            &mut repository,
            &mut worker,
            &mut FakeProviderGuard::default(),
        )
        .expect("fresh restart dispatch"),
        ManagedPrestartRestartOutcome::Starting(_)
    ));
    assert_eq!(
        worker.calls,
        vec!["model", "thread-accepted", "turn-accepted"]
    );
    assert_eq!(
        worker
            .calls
            .iter()
            .filter(|call| matches!(**call, "thread-accepted" | "turn-accepted"))
            .count(),
        2
    );
}

#[test]
fn restart_model_unavailable_after_proven_no_candidate_does_not_create_thread_claim() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let request = with_predispatch_baseline(request);
    let (attempt, state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    worker.calls.clear();
    worker.availability = ManagedModelAvailability::Unavailable {
        code: "TERRA_UNAVAILABLE",
    };
    repository.dispatch_state = ManagedWorkerDispatchState::NoWorkerThread;
    let ManagedPrestartRestartOutcome::NoProviderEffect(proof) =
        recover_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &mut repository,
            &mut worker,
        )
        .expect("durable no-thread proof")
    else {
        panic!("expected typed no-effect proof");
    };
    assert_eq!(
        continue_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &proof,
            &mut repository,
            &mut worker,
            &mut FakeProviderGuard::default(),
        ),
        Err(ManagedAttemptOrchestratorError::ModelUnavailable {
            code: "TERRA_UNAVAILABLE",
        })
    );
    assert_eq!(worker.calls, vec!["model"]);
    assert_eq!(repository.calls, vec!["load-dispatch-state", "authority"]);
}

#[test]
fn restart_marker_turn_without_durable_exact_start_is_persisted_only_as_failed_start() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let (attempt, state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    worker.calls.clear();
    repository.thread_claim_disposition = ManagedWorkerThreadDispatchDisposition::ExactReplay;
    repository.dispatch_state = ManagedWorkerDispatchState::WorkerTurnClaimed;
    let thread = ManagedWorkerObservation::thread_accepted(
        attempt.attempt_number(),
        "thread-recovered",
        2,
        app_server_identity(),
        digest('6'),
    )
    .expect("thread recovery");
    let turn = ManagedWorkerObservation::turn_accepted(
        attempt.attempt_number(),
        "thread-recovered",
        "turn-recovered",
        2,
        app_server_identity(),
        digest('7'),
    )
    .expect("turn recovery");
    let terminal = ManagedTerminalCandidate::new(
        ManagedWorkerObservation::prestart_terminal_failed(
            attempt.attempt_number(),
            "thread-recovered",
            "turn-recovered",
            2,
            app_server_identity(),
            digest('8'),
        )
        .expect("failed-start terminal"),
    )
    .expect("terminal candidate");
    worker
        .prestart_recoveries
        .push_back(ManagedWorkerPrestartRecovery::ExactFailedStart {
            thread,
            turn: Box::new(turn),
            terminal: Box::new(terminal),
        });

    let outcome = recover_managed_prestart_on_restart(
        &request,
        &attempt,
        &state,
        &mut repository,
        &mut worker,
    )
    .expect("failed-start recovery");
    let ManagedPrestartRestartOutcome::FailedStart { terminal } = outcome else {
        panic!("unexpected restart result: {outcome:?}");
    };

    assert_eq!(
        terminal.kind(),
        lattice_ports::WorkerObservationKind::PrestartTerminalFailed
    );
    assert!(
        !worker
            .calls
            .iter()
            .any(|call| matches!(*call, "thread-accepted" | "turn-accepted" | "exact-started"))
    );
    assert_eq!(
        repository
            .observations
            .iter()
            .filter(|observation| observation.kind()
                == lattice_ports::WorkerObservationKind::TurnStarted)
            .count(),
        0
    );
}

#[test]
fn retained_thread_double_proves_exact_empty_before_continuing_replayed_turn_claim() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let request = with_predispatch_baseline(request);
    let (attempt, mut state) = claimed_prestart_attempt(&request, &mut repository);
    state.begin_dispatch().expect("dispatch");
    let retained_thread = ManagedWorkerObservation::thread_accepted(
        attempt.attempt_number(),
        "thread-retained",
        1,
        app_server_identity(),
        digest('6'),
    )
    .expect("thread");
    state
        .apply_start(retained_thread.start_observation().expect("thread start"))
        .expect("apply thread");
    repository
        .record_observation(request.binding(), &attempt, &retained_thread)
        .expect("persist thread");
    repository.calls.clear();
    worker.calls.clear();
    repository.dispatch_state = ManagedWorkerDispatchState::WorkerTurnClaimed;
    repository.turn_claim_disposition = ManagedWorkerTurnDispatchDisposition::ExactReplay;
    worker
        .prestart_recoveries
        .push_back(ManagedWorkerPrestartRecovery::ExactEmptyThread {
            thread: retained_thread,
        });

    let ManagedPrestartRestartOutcome::NoProviderEffect(proof) =
        recover_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &mut repository,
            &mut worker,
        )
        .expect("exact empty proof")
    else {
        panic!("expected typed exact-empty proof");
    };
    assert!(matches!(
        continue_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &proof,
            &mut repository,
            &mut worker,
            &mut FakeProviderGuard::default(),
        )
        .expect("turn replay result"),
        ManagedPrestartRestartOutcome::Starting(_)
    ));
    assert_eq!(
        worker.calls,
        vec!["recover-prestart", "model", "turn-accepted"]
    );
    assert_eq!(
        repository
            .calls
            .iter()
            .filter(|call| **call == "claim-turn")
            .count(),
        1
    );
    assert_eq!(
        worker
            .calls
            .iter()
            .filter(|call| **call == "turn-accepted")
            .count(),
        1
    );
}

#[test]
fn stale_authority_still_closes_an_exact_replayed_prestart_turn_but_cannot_start_work() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let (attempt, state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    worker.calls.clear();
    repository.dispatch_state = ManagedWorkerDispatchState::WorkerTurnClaimed;
    repository.authority_failure_code = Some("AUTHORITY_EXPIRED");
    let thread = ManagedWorkerObservation::thread_accepted(
        attempt.attempt_number(),
        "thread-recovered",
        2,
        app_server_identity(),
        digest('6'),
    )
    .expect("thread recovery");
    let turn = ManagedWorkerObservation::turn_accepted(
        attempt.attempt_number(),
        "thread-recovered",
        "turn-recovered",
        2,
        app_server_identity(),
        digest('7'),
    )
    .expect("turn recovery");
    let terminal = ManagedTerminalCandidate::new(
        ManagedWorkerObservation::prestart_terminal_failed(
            attempt.attempt_number(),
            "thread-recovered",
            "turn-recovered",
            2,
            app_server_identity(),
            digest('8'),
        )
        .expect("failed-start terminal"),
    )
    .expect("terminal candidate");
    worker
        .prestart_recoveries
        .push_back(ManagedWorkerPrestartRecovery::ExactFailedStart {
            thread,
            turn: Box::new(turn),
            terminal: Box::new(terminal),
        });

    assert!(matches!(
        recover_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &mut repository,
            &mut worker,
        )
        .expect("stale authority may close exact provider work"),
        ManagedPrestartRestartOutcome::FailedStart { .. }
    ));
    assert_eq!(worker.calls, vec!["recover-dispatch"]);
    assert!(
        !repository
            .calls
            .iter()
            .any(|call| matches!(*call, "claim-thread" | "claim-turn"))
    );
    assert!(
        !worker
            .calls
            .iter()
            .any(|call| matches!(*call, "thread-accepted" | "turn-accepted" | "exact-started"))
    );
}

#[test]
fn stale_authority_without_a_provider_candidate_never_claims_or_starts_work() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let request = with_predispatch_baseline(request);
    let (attempt, state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    worker.calls.clear();
    repository.dispatch_state = ManagedWorkerDispatchState::WorkerThreadClaimed;
    repository.authority_failure_code = Some("AUTHORITY_EXPIRED");
    worker
        .prestart_recoveries
        .push_back(ManagedWorkerPrestartRecovery::ProvenNoProviderCandidate);

    assert!(matches!(
        recover_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &mut repository,
            &mut worker,
        )
        .expect("claimed thread remains reconciliation-only despite stale authority"),
        ManagedPrestartRestartOutcome::ReconciliationRequired
    ));
    assert_eq!(worker.calls, vec!["recover-dispatch"]);
    assert_eq!(repository.calls, vec!["load-dispatch-state"]);
}

#[test]
fn no_baseline_crash_recovers_a_typed_no_provider_effect_before_any_new_start() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let (attempt, state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    worker.calls.clear();
    repository.dispatch_state = ManagedWorkerDispatchState::NoWorkerThread;

    let outcome = recover_managed_prestart_on_restart(
        &request,
        &attempt,
        &state,
        &mut repository,
        &mut worker,
    )
    .expect("durable no-thread proof");
    let ManagedPrestartRestartOutcome::NoProviderEffect(proof) = outcome else {
        panic!("unexpected restart outcome: {outcome:?}");
    };
    assert_eq!(
        proof,
        ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
            worker_thread_claimed: false,
        }
    );
    assert_eq!(repository.calls, vec!["load-dispatch-state"]);
    assert!(worker.calls.is_empty());

    let mut guard = FakeProviderGuard::default();
    assert_eq!(
        continue_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &proof,
            &mut repository,
            &mut worker,
            &mut guard,
        ),
        Err(ManagedAttemptOrchestratorError::PredispatchBaselineRequired)
    );
    assert_eq!(guard.calls, 0);
    assert!(worker.calls.is_empty());
}

#[test]
fn exact_empty_stale_authority_is_typed_no_effect_and_never_starts_a_turn() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let request = with_predispatch_baseline(request);
    let (attempt, mut state) = claimed_prestart_attempt(&request, &mut repository);
    state.begin_dispatch().expect("dispatch");
    repository.dispatch_state = ManagedWorkerDispatchState::WorkerThreadClaimed;
    let thread = ManagedWorkerObservation::thread_accepted(
        attempt.attempt_number(),
        "thread-exact-empty",
        2,
        app_server_identity(),
        digest('6'),
    )
    .expect("thread");
    worker
        .prestart_recoveries
        .push_back(ManagedWorkerPrestartRecovery::ExactEmptyThread {
            thread: thread.clone(),
        });
    repository.calls.clear();
    worker.calls.clear();

    let outcome = recover_managed_prestart_on_restart(
        &request,
        &attempt,
        &state,
        &mut repository,
        &mut worker,
    )
    .expect("exact empty proof");
    let ManagedPrestartRestartOutcome::NoProviderEffect(proof) = outcome else {
        panic!("unexpected restart outcome: {outcome:?}");
    };
    assert!(matches!(
        proof,
        ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn {
            worker_turn_claimed: false,
            ..
        }
    ));

    repository.authority_failure_code = Some("AUTHORITY_EXPIRED");
    let mut guard = FakeProviderGuard::default();
    assert!(matches!(
        continue_managed_prestart_on_restart(
            &request,
            &attempt,
            &state,
            &proof,
            &mut repository,
            &mut worker,
            &mut guard,
        ),
        Err(ManagedAttemptOrchestratorError::Repository(_))
    ));
    assert_eq!(guard.calls, 0);
    assert!(!worker.calls.iter().any(|call| *call == "turn-accepted"));
}

#[test]
fn pending_stale_authority_has_a_durable_no_effect_closure_before_release() {
    let (request, mut repository, _worker, _verifier) = fixture(true);
    let (attempt, _state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    let proof = ManagedPrestartNoEffectProof::PendingReservation;

    assert_eq!(
        close_managed_prestart_without_provider_effect(
            request.binding(),
            &attempt,
            &proof,
            "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT",
            &mut repository,
        )
        .expect("pending closure"),
        ManagedPrestartClosureDisposition::Closed
    );
    assert_eq!(repository.calls, vec!["close-prestart"]);
}

#[test]
fn claimed_thread_no_candidate_proof_cannot_close_or_release_capacity() {
    let (request, mut repository, _worker, _verifier) = fixture(true);
    let (attempt, _state) = claimed_prestart_attempt(&request, &mut repository);
    repository.calls.clear();
    let proof = ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
        worker_thread_claimed: true,
    };

    assert_eq!(
        close_managed_prestart_without_provider_effect(
            request.binding(),
            &attempt,
            &proof,
            "LATTICE_MANAGED_THREAD_START_RPC_REJECTED",
            &mut repository,
        ),
        Err(ManagedAttemptOrchestratorError::DispatchReconciliationRequired)
    );
    assert!(repository.calls.is_empty());
}

#[test]
fn writer_rotation_immediately_before_each_provider_rpc_fails_closed() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let mut before_thread = FakeProviderGuard {
        calls: 0,
        fail_on_call: Some(1),
    };
    assert!(matches!(
        prepare_managed_attempt_with_guard(
            &request,
            &mut repository,
            &mut worker,
            &mut before_thread,
        ),
        Err(ManagedAttemptOrchestratorError::ProviderEffectGuard(_))
    ));
    assert_eq!(before_thread.calls, 1);
    assert!(!worker.calls.iter().any(|call| *call == "thread-accepted"));

    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let mut before_turn = FakeProviderGuard {
        calls: 0,
        fail_on_call: Some(2),
    };
    assert!(matches!(
        prepare_managed_attempt_with_guard(
            &request,
            &mut repository,
            &mut worker,
            &mut before_turn,
        ),
        Err(ManagedAttemptOrchestratorError::ProviderEffectGuard(_))
    ));
    assert_eq!(before_turn.calls, 2);
    assert_eq!(
        worker
            .calls
            .iter()
            .filter(|call| **call == "thread-accepted")
            .count(),
        1
    );
    assert!(!worker.calls.iter().any(|call| *call == "turn-accepted"));
}

#[test]
fn exact_replay_review_claim_never_authorizes_a_second_reviewer_dispatch() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    repository.review_claim_disposition = ManagedReviewDispatchDisposition::ExactReplay;

    assert_eq!(
        run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier),
        Err(ManagedAttemptOrchestratorError::ReviewDispatchReconciliationRequired)
    );
    assert_eq!(repository.calls.last(), Some(&"claim-review"));
    assert_eq!(verifier.calls, vec!["prepare"]);
}

#[test]
fn exact_replay_review_continuation_requires_current_guard_without_a_second_claim() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    repository.review_claim_disposition = ManagedReviewDispatchDisposition::ExactReplay;
    let starting =
        prepare_managed_attempt(&request, &mut repository, &mut worker).expect("starting");
    let executing =
        confirm_managed_exact_start(starting, &mut repository, &mut worker).expect("exact started");
    let terminal = finish_managed_execution(executing, &mut repository, &mut worker)
        .expect("terminal completed");
    let review_ready = prepare_managed_review(terminal, &mut repository, &mut verifier)
        .expect("mechanical review preparation");
    let claimed = claim_managed_review(review_ready, &mut repository).expect("replayed claim");
    assert_eq!(
        claimed.disposition(),
        ManagedReviewDispatchDisposition::ExactReplay
    );
    let mut guard = FakeProviderGuard::default();

    finish_replayed_managed_review_with_provider_guard(
        claimed,
        &mut repository,
        &mut verifier,
        &mut guard,
    )
    .expect("exact retained review continuation");

    assert_eq!(guard.calls, 1);
    assert_eq!(
        repository
            .calls
            .iter()
            .filter(|call| **call == "claim-review")
            .count(),
        1
    );
    assert_eq!(verifier.calls, vec!["prepare", "review", "verify"]);
}

#[test]
fn reviewer_effect_requires_the_post_mechanical_claimed_type_state() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    let starting =
        prepare_managed_attempt(&request, &mut repository, &mut worker).expect("starting");
    let executing =
        confirm_managed_exact_start(starting, &mut repository, &mut worker).expect("exact started");
    let terminal = finish_managed_execution(executing, &mut repository, &mut worker)
        .expect("terminal completed");
    let review_ready = prepare_managed_review(terminal, &mut repository, &mut verifier)
        .expect("mechanical review preparation");
    assert_eq!(verifier.calls, vec!["prepare"]);

    let claimed = claim_managed_review(review_ready, &mut repository).expect("review claim");
    assert_eq!(
        claimed.disposition(),
        ManagedReviewDispatchDisposition::Claimed
    );
    assert_eq!(repository.calls.last(), Some(&"claim-review"));
    assert_eq!(verifier.calls, vec!["prepare"]);

    finish_claimed_managed_review(
        claimed,
        &mut repository,
        &mut verifier,
        &mut FakeProviderGuard::default(),
    )
    .expect("fresh claimed review");
    assert_eq!(verifier.calls, vec!["prepare", "review", "verify"]);
}

#[test]
fn failed_prepare_persists_typed_transport_evidence_before_returning_the_failure() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    verifier.fail_prepare_with_evidence = true;
    let starting =
        prepare_managed_attempt(&request, &mut repository, &mut worker).expect("starting");
    let executing =
        confirm_managed_exact_start(starting, &mut repository, &mut worker).expect("exact started");
    let terminal = finish_managed_execution(executing, &mut repository, &mut worker)
        .expect("terminal completed");
    let artifacts_before = repository
        .calls
        .iter()
        .filter(|call| **call == "record-artifact")
        .count();

    let failure = prepare_managed_review(terminal, &mut repository, &mut verifier)
        .expect_err("transport-bound prepare must remain failed");

    assert!(matches!(
        failure,
        ManagedAttemptOrchestratorError::Verification(ref port)
            if port.code() == "LATTICE_MANAGED_VERIFIER_GIT_REV_VERIFY_FAILED"
    ));
    assert_eq!(verifier.calls, vec!["prepare", "prepare-failure-evidence"]);
    assert_eq!(
        repository
            .calls
            .iter()
            .filter(|call| **call == "record-artifact")
            .count(),
        artifacts_before + 1,
        "the typed failure receipt must become durable before the error escapes"
    );
    assert_eq!(repository.calls.last(), Some(&"record-artifact"));
    assert!(
        !repository
            .calls
            .iter()
            .any(|call| *call == "record-verification")
    );
}

#[test]
fn rotated_writer_after_review_thread_claim_suppresses_reviewer_process_effect() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    let starting =
        prepare_managed_attempt(&request, &mut repository, &mut worker).expect("starting");
    let executing =
        confirm_managed_exact_start(starting, &mut repository, &mut worker).expect("exact started");
    let terminal = finish_managed_execution(executing, &mut repository, &mut worker)
        .expect("terminal completed");
    let review_ready = prepare_managed_review(terminal, &mut repository, &mut verifier)
        .expect("mechanical review preparation");
    let claimed = claim_managed_review(review_ready, &mut repository).expect("review claim");
    let mut guard = FakeProviderGuard {
        calls: 0,
        fail_on_call: Some(1),
    };

    let failure =
        finish_claimed_managed_review(claimed, &mut repository, &mut verifier, &mut guard)
            .expect_err("rotated writer must suppress reviewer process creation");

    assert!(matches!(
        failure,
        ManagedAttemptOrchestratorError::ProviderEffectGuard(_)
    ));
    assert_eq!(guard.calls, 1);
    assert_eq!(verifier.calls, vec!["prepare"]);
    assert!(
        !repository
            .calls
            .iter()
            .any(|call| *call == "claim-review-turn")
    );
}

#[test]
fn rotated_writer_after_review_turn_claim_suppresses_turn_authorization() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    verifier.authorize_turn = true;
    let starting =
        prepare_managed_attempt(&request, &mut repository, &mut worker).expect("starting");
    let executing =
        confirm_managed_exact_start(starting, &mut repository, &mut worker).expect("exact started");
    let terminal = finish_managed_execution(executing, &mut repository, &mut worker)
        .expect("terminal completed");
    let review_ready = prepare_managed_review(terminal, &mut repository, &mut verifier)
        .expect("mechanical review preparation");
    let claimed = claim_managed_review(review_ready, &mut repository).expect("review claim");
    let mut guard = FakeProviderGuard {
        calls: 0,
        fail_on_call: Some(2),
    };

    let failure =
        finish_claimed_managed_review(claimed, &mut repository, &mut verifier, &mut guard)
            .expect_err("rotated writer must suppress reviewer turn authorization");

    assert!(matches!(
        failure,
        ManagedAttemptOrchestratorError::Verification(_)
    ));
    assert_eq!(guard.calls, 2);
    assert_eq!(verifier.calls, vec!["prepare", "review"]);
    assert_eq!(repository.calls.last(), Some(&"claim-review-turn"));
    assert!(
        !repository
            .calls
            .iter()
            .any(|call| *call == "record-verification")
    );
}

#[test]
fn failed_independent_verification_is_durable_and_never_returns_merge_approval() {
    let (request, mut repository, mut worker, mut verifier) = fixture(true);
    verifier.outcome = VerificationOutcome::Failed;

    let error = run_managed_attempt(&request, &mut repository, &mut worker, &mut verifier)
        .expect_err("failed verification must not produce a merge target");
    let ManagedAttemptOrchestratorError::VerificationFailed(record) = error else {
        panic!("unexpected failure: {error:?}");
    };
    assert_eq!(record.outcome(), VerificationOutcome::Failed);
    assert_eq!(
        &repository.calls[repository.calls.len() - 2..],
        &["claim-review", "record-verification"]
    );
    assert_eq!(verifier.calls, vec!["prepare", "review", "verify"]);
}

fn claimed_running_attempt(
    request: &ManagedAttemptRequest,
    repository: &mut FakeRepository,
) -> (VerifiedWorkerAttemptRecord, WorkerAttemptState) {
    repository
        .assert_execution_authority_current(request.binding(), request.authority_digest())
        .expect("authority");
    let attempt = repository
        .claim_attempt(request.binding(), request.packet())
        .expect("claim")
        .into_attempt();
    let mut state = WorkerAttemptState::new(request.packet().clone()).expect("state");
    state.begin_dispatch().expect("dispatch");
    let number = request.packet().attempt();
    let thread_id = format!("thread-{number}");
    let turn_id = format!("turn-{number}");
    for observation in [
        ManagedWorkerObservation::thread_accepted(
            u64::from(number),
            &thread_id,
            1,
            app_server_identity(),
            digest('7'),
        )
        .expect("thread"),
        ManagedWorkerObservation::turn_accepted(
            u64::from(number),
            &thread_id,
            &turn_id,
            1,
            app_server_identity(),
            digest('8'),
        )
        .expect("turn"),
        ManagedWorkerObservation::exact_started(
            u64::from(number),
            &thread_id,
            &turn_id,
            1,
            app_server_identity(),
            "2026-08-26T02:00:04Z",
            digest('9'),
        )
        .expect("started"),
    ] {
        state
            .apply_start(observation.start_observation().expect("start"))
            .expect("apply start");
        repository
            .record_observation(request.binding(), &attempt, &observation)
            .expect("record observation");
    }
    (attempt, state)
}

fn claim_and_terminalize(
    request: &ManagedAttemptRequest,
    repository: &mut FakeRepository,
    evidence: char,
) -> VerifiedWorkerAttemptRecord {
    repository
        .assert_execution_authority_current(request.binding(), request.authority_digest())
        .expect("authority");
    let attempt = repository
        .claim_attempt(request.binding(), request.packet())
        .expect("claim")
        .into_attempt();
    let number = request.packet().attempt();
    let thread_id = format!("thread-{number}");
    let turn_id = format!("turn-{number}");
    for observation in [
        ManagedWorkerObservation::thread_accepted(
            u64::from(number),
            &thread_id,
            1,
            app_server_identity(),
            digest('7'),
        )
        .expect("thread"),
        ManagedWorkerObservation::turn_accepted(
            u64::from(number),
            &thread_id,
            &turn_id,
            1,
            app_server_identity(),
            digest('8'),
        )
        .expect("turn"),
        ManagedWorkerObservation::exact_started(
            u64::from(number),
            &thread_id,
            &turn_id,
            1,
            app_server_identity(),
            "2026-08-26T02:00:04Z",
            digest('9'),
        )
        .expect("started"),
        ManagedWorkerObservation::terminal(
            u64::from(number),
            &thread_id,
            &turn_id,
            WorkerTerminal::Interrupted,
            1,
            app_server_identity(),
            digest(evidence),
        )
        .expect("terminal"),
    ] {
        repository
            .record_observation(request.binding(), &attempt, &observation)
            .expect("record observation");
    }
    attempt
}

#[test]
fn restart_reads_thread_then_turn_then_resumes_and_reconciles_without_starting() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let (attempt, state) = claimed_running_attempt(&request, &mut repository);
    for evidence in ['1', '2', '3', '4'] {
        worker
            .reconciliations
            .push_back(ManagedWorkerReconciliation::ExactActive(
                ManagedWorkerObservation::reconciled(
                    1,
                    "thread-1",
                    "turn-1",
                    1,
                    app_server_identity(),
                    digest(evidence),
                )
                .expect("reconciled"),
            ));
    }
    worker.calls.clear();

    assert_eq!(
        reconcile_managed_attempt_on_restart(
            request.binding(),
            &attempt,
            &state,
            &mut repository,
            &mut worker,
        )
        .expect("restart reconciliation"),
        ManagedRestartOutcome::ExactActive
    );
    assert_eq!(
        worker.calls,
        vec!["read-thread", "read-turn", "resume-turn", "reconcile-turn"]
    );
    assert!(!worker.calls.iter().any(|call| call.contains("accepted")));
}

#[test]
fn fresh_restart_after_durable_progress_preserves_projection_without_a_new_agent() {
    let (request, mut repository, _old_worker, _verifier) = fixture(true);
    let (attempt, state) = claimed_running_attempt(&request, &mut repository);
    let progress = ManagedWorkerObservation::meaningful_progress(
        attempt.attempt_number(),
        "thread-1",
        "turn-1",
        1,
        app_server_identity(),
        digest('a'),
    )
    .expect("progress");
    repository
        .record_observation(request.binding(), &attempt, &progress)
        .expect("durable progress");
    let progress_before = repository
        .observations
        .iter()
        .rev()
        .find(|observation| {
            matches!(
                observation.kind(),
                lattice_ports::WorkerObservationKind::TurnStarted
                    | lattice_ports::WorkerObservationKind::MeaningfulProgress
            )
        })
        .expect("progress projection")
        .observed_at()
        .to_owned();
    let attempt_count_before = repository.attempts.len();

    // A fresh worker adapter owns no prior in-memory session. It may only
    // reconcile the exact retained IDs supplied by durable replay.
    let (_, _, mut fresh_worker, _) = fixture(true);
    for evidence in ['1', '2', '3', '4'] {
        fresh_worker
            .reconciliations
            .push_back(ManagedWorkerReconciliation::ExactActive(
                ManagedWorkerObservation::reconciled(
                    attempt.attempt_number(),
                    "thread-1",
                    "turn-1",
                    1,
                    app_server_identity(),
                    digest(evidence),
                )
                .expect("reconciled"),
            ));
    }

    assert_eq!(
        reconcile_managed_attempt_on_restart(
            request.binding(),
            &attempt,
            &state,
            &mut repository,
            &mut fresh_worker,
        )
        .expect("fresh restart reconciliation"),
        ManagedRestartOutcome::ExactActive
    );
    let progress_after = repository
        .observations
        .iter()
        .rev()
        .find(|observation| {
            matches!(
                observation.kind(),
                lattice_ports::WorkerObservationKind::TurnStarted
                    | lattice_ports::WorkerObservationKind::MeaningfulProgress
            )
        })
        .expect("replayed progress projection")
        .observed_at();
    assert_eq!(progress_after, progress_before);
    assert_eq!(repository.attempts.len(), attempt_count_before);
    assert_eq!(
        fresh_worker.calls,
        vec!["read-thread", "read-turn", "resume-turn", "reconcile-turn"]
    );
    assert!(
        !fresh_worker
            .calls
            .iter()
            .any(|call| matches!(*call, "thread-accepted" | "turn-accepted" | "exact-started"))
    );
}

#[test]
fn restart_short_circuits_on_exact_terminal_without_resume_or_start() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let (attempt, state) = claimed_running_attempt(&request, &mut repository);
    worker
        .reconciliations
        .push_back(ManagedWorkerReconciliation::ExactTerminal(
            ManagedTerminalCandidate::new(
                ManagedWorkerObservation::terminal(
                    1,
                    "thread-1",
                    "turn-1",
                    WorkerTerminal::Interrupted,
                    1,
                    app_server_identity(),
                    digest('6'),
                )
                .expect("terminal"),
            )
            .expect("candidate"),
        ));
    worker.calls.clear();

    let outcome = reconcile_managed_attempt_on_restart(
        request.binding(),
        &attempt,
        &state,
        &mut repository,
        &mut worker,
    )
    .expect("restart terminal");
    assert!(matches!(
        outcome,
        ManagedRestartOutcome::ExactTerminal {
            terminal: WorkerTerminal::Interrupted,
            ..
        }
    ));
    assert_eq!(worker.calls, vec!["read-thread"]);
}

#[test]
fn closed_heartbeat_stall_reconciles_then_interrupts_waits_terminal_and_allows_one_retry() {
    let (request, mut repository, mut worker, _verifier) = fixture(true);
    let (attempt, mut state) = claimed_running_attempt(&request, &mut repository);
    worker.terminal = WorkerTerminal::Interrupted;
    worker
        .reconciliations
        .push_back(ManagedWorkerReconciliation::Unresolved);
    worker.calls.clear();
    let budget = worker_budget();
    let progress = MeaningfulProgress::new(
        &state,
        MeaningfulProgressKind::ExactLifecycleNotification,
        "2026-08-26T02:00:10Z",
        &content_pointer("evidence", &digest('e')),
    )
    .expect("progress");
    let watchdog = AttemptWatchdogObservation::new(
        "2026-08-26T02:02:00Z",
        60,
        ProcessObservation::Alive,
        TurnActivityObservation::ExactInProgress {
            thread_id: "thread-1".to_owned(),
            turn_id: "turn-1".to_owned(),
        },
        ReconciliationState::NotAttempted,
    )
    .expect("watchdog");

    assert_eq!(
        handle_managed_attempt_stall(
            request.binding(),
            &attempt,
            &mut state,
            &budget,
            &progress,
            &watchdog,
            true,
            &mut repository,
            &mut worker,
        )
        .expect("stall recovery"),
        ManagedStallOutcome::Retry {
            reason: StallReason::HeartbeatTimeoutActiveTurn,
            decision: RetryDecision::Retry { next_attempt: 2 },
        }
    );
    assert_eq!(
        worker.calls,
        vec!["reconcile-turn", "interrupt", "terminal"]
    );
}

#[test]
fn exact_interrupt_terminal_never_exceeds_two_repair_retries() {
    let (stream, binding) = lineage();
    let authority_digest = digest('2');
    let budget = worker_budget();
    let mut repository = FakeRepository {
        calls: Vec::new(),
        stream,
        attempts: Vec::new(),
        observations: Vec::new(),
        verifications: Vec::new(),
        authority_digest: authority_digest.clone(),
        authority_failure_code: None,
        claim_disposition: ManagedAttemptClaimDisposition::Claimed,
        thread_claim_disposition: ManagedWorkerThreadDispatchDisposition::Claimed,
        turn_claim_disposition: ManagedWorkerTurnDispatchDisposition::Claimed,
        review_claim_disposition: ManagedReviewDispatchDisposition::Claimed,
        dispatch_state: ManagedWorkerDispatchState::NoWorkerThread,
        closure_disposition: ManagedPrestartClosureDisposition::Closed,
        sequence: 0,
        execution_trace: Rc::new(RefCell::new(Vec::new())),
    };
    let request_one = ManagedAttemptRequest::new(
        binding.clone(),
        packet_number(&binding, &budget, 1, 10, None),
        authority_digest.clone(),
    )
    .expect("attempt one");
    claim_and_terminalize(&request_one, &mut repository, 'a');
    let prior_one = digest('a');
    let request_two = ManagedAttemptRequest::new(
        binding.clone(),
        packet_number(&binding, &budget, 2, 20, Some(&prior_one)),
        authority_digest.clone(),
    )
    .expect("attempt two");
    claim_and_terminalize(&request_two, &mut repository, 'b');
    let prior_two = digest('b');
    let request_three = ManagedAttemptRequest::new(
        binding,
        packet_number(request_two.binding(), &budget, 3, 30, Some(&prior_two)),
        authority_digest,
    )
    .expect("attempt three");
    let (attempt, mut state) = claimed_running_attempt(&request_three, &mut repository);
    let mut worker = FakeWorker {
        calls: Vec::new(),
        exact_start: true,
        availability: ManagedModelAvailability::Available,
        reconciliations: std::collections::VecDeque::from([
            ManagedWorkerReconciliation::Unresolved,
        ]),
        prestart_recoveries: std::collections::VecDeque::new(),
        terminal: WorkerTerminal::Interrupted,
        execution_trace: Rc::new(RefCell::new(Vec::new())),
    };
    let progress = MeaningfulProgress::new(
        &state,
        MeaningfulProgressKind::ExactLifecycleNotification,
        "2026-08-26T02:00:10Z",
        &content_pointer("evidence", &digest('e')),
    )
    .expect("progress");
    let watchdog = AttemptWatchdogObservation::new(
        "2026-08-26T02:02:00Z",
        60,
        ProcessObservation::Alive,
        TurnActivityObservation::ExactInProgress {
            thread_id: "thread-3".to_owned(),
            turn_id: "turn-3".to_owned(),
        },
        ReconciliationState::NotAttempted,
    )
    .expect("watchdog");

    assert_eq!(
        handle_managed_attempt_stall(
            request_three.binding(),
            &attempt,
            &mut state,
            &budget,
            &progress,
            &watchdog,
            true,
            &mut repository,
            &mut worker,
        )
        .expect("bounded recovery"),
        ManagedStallOutcome::Retry {
            reason: StallReason::HeartbeatTimeoutActiveTurn,
            decision: RetryDecision::BlockedRetryBudgetExhausted,
        }
    );
    assert_eq!(
        worker.calls,
        vec!["reconcile-turn", "interrupt", "terminal"]
    );
}

fn workflow_subject_binding() -> SubjectBinding {
    SubjectBinding::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("project-1:registry:1").expect("snapshot"),
        TaskId::new("TASK-MANAGED-PORTS-001").expect("task"),
        "1",
        digest('c'),
    )
    .expect("subject binding")
}

#[derive(Debug)]
struct WorkflowLifecycle {
    binding: SubjectBinding,
    state: TaskState,
    autonomy: Option<AutonomyReceiptProjection>,
    transitions: Vec<(TaskState, TaskState)>,
}

impl WorkflowLifecycle {
    fn evidence(&self) -> TaskLifecycleEvidence {
        TaskLifecycleEvidence::new(
            self.binding.clone(),
            self.autonomy.clone().map_or(
                TaskLifecycleAutonomyEvidence::HistoricalOptional(None),
                TaskLifecycleAutonomyEvidence::RequiredComplete,
            ),
            self.state,
            digest('6'),
            None,
        )
    }
}

impl TaskLifecyclePort for WorkflowLifecycle {
    fn admit(
        &mut self,
        binding: &SubjectBinding,
        client_request_id: &str,
    ) -> TaskLifecycleResult<TaskLifecycleAdmission> {
        assert_eq!(binding, &self.binding);
        assert_eq!(client_request_id, "managed-workflow-request");
        TaskLifecycleAdmission::existing(self.evidence())
    }

    fn record_autonomy_receipt(
        &mut self,
        binding: &SubjectBinding,
        writer_authority: Option<&WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        assert_eq!(binding, &self.binding);
        assert!(writer_authority.is_none());
        self.autonomy = Some(AutonomyReceiptProjection::new(
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

    fn transition(
        &mut self,
        binding: &SubjectBinding,
        from: TaskState,
        to: TaskState,
        writer_authority: Option<&WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        assert_eq!(binding, &self.binding);
        assert_eq!(self.state, from);
        assert_eq!(
            writer_authority.is_some(),
            matches!(
                (from, to),
                (TaskState::Preparing, TaskState::Executing)
                    | (TaskState::Executing, TaskState::Verifying)
                    | (TaskState::Verifying, TaskState::Reviewing)
                    | (TaskState::Reviewing, TaskState::AwaitingMergeApproval)
            )
        );
        self.transitions.push((from, to));
        self.state = to;
        Ok(self.evidence())
    }

    fn record_result(
        &mut self,
        _binding: &SubjectBinding,
        _result_digest: &ContentDigest,
        _writer_authority: &WriterLeaseAuthorityHead,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        panic!("managed workflow must not use merging result/completion path")
    }

    fn load(&mut self, binding: &SubjectBinding) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        assert_eq!(binding, &self.binding);
        Ok(self.evidence())
    }
}

#[derive(Debug)]
struct WorkflowLease {
    owner: FakeWriterLease,
    released: bool,
}

impl WorkflowLease {
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

impl WriterLeaseRepository for WorkflowLease {
    fn execute(
        &mut self,
        command: WriterLeaseRepositoryCommand,
    ) -> Result<WriterLeaseCommandReceipt, WriterLeaseRepositoryError> {
        match command {
            WriterLeaseRepositoryCommand::Acquire(request) => {
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
                        daemon_instance_id: "managed-test-daemon".to_owned(),
                        daemon_epoch: DaemonEpoch::new(1).expect("epoch"),
                    },
                    observation: Self::observation("2026-08-26T02:00:00Z"),
                    expires_at: "2026-08-26T03:00:00Z".to_owned(),
                }))
            }
            WriterLeaseRepositoryCommand::Release(request) => {
                let result = self.execute_fake(WriterLeaseCommand::Release(ReleaseCommand {
                    command_id: request.command_id,
                    project_id: request.project_id,
                    expected_head: request.expected_head,
                    observation: Self::observation("2026-08-26T02:01:00Z"),
                }));
                if result
                    .as_ref()
                    .is_ok_and(|receipt| receipt.outcome == CommandOutcome::Applied)
                {
                    self.released = true;
                }
                result
            }
            WriterLeaseRepositoryCommand::Heartbeat(_)
            | WriterLeaseRepositoryCommand::MarkSuspect(_)
            | WriterLeaseRepositoryCommand::ProcessHandoff(_)
            | WriterLeaseRepositoryCommand::Revoke(_) => Err(WriterLeaseRepositoryError::new(
                WriterLeaseRepositoryErrorKind::Unavailable,
            )),
        }
    }

    fn current_authority(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<Option<WriterLeaseCurrentAuthority>, WriterLeaseRepositoryError> {
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

fn workflow_fixture(
    verification_outcome: VerificationOutcome,
) -> (
    ManagedWorkflowRequest,
    WorkflowLifecycle,
    WorkflowLease,
    FakeRepository,
    FakeWorker,
    FakeVerifier,
) {
    let (stream, execution_binding) = lineage();
    let budget = worker_budget();
    let authority_digest = digest('2');
    let packet = packet_number(&execution_binding, &budget, 1, 1, None);
    let attempt_request =
        ManagedAttemptRequest::new(execution_binding, packet, authority_digest.clone())
            .expect("attempt request");
    let control = ControlledTaskRequest::new(
        workflow_subject_binding(),
        "managed-workflow-request",
        lattice_contracts::AttemptId::new("attempt-1").expect("attempt"),
        "managed-lease-1",
        "sole-foreman-v1",
        "managed-worktree-1",
        HolderProcessId::new(77).expect("process"),
        digest('b'),
    )
    .expect("control request");
    let workflow = ManagedWorkflowRequest::new(control, attempt_request).expect("workflow");
    let lifecycle = WorkflowLifecycle {
        binding: workflow_subject_binding(),
        state: TaskState::Draft,
        autonomy: None,
        transitions: Vec::new(),
    };
    let lease = WorkflowLease {
        owner: FakeWriterLease::new(),
        released: false,
    };
    let repository = FakeRepository {
        calls: Vec::new(),
        stream,
        attempts: Vec::new(),
        observations: Vec::new(),
        verifications: Vec::new(),
        authority_digest,
        authority_failure_code: None,
        claim_disposition: ManagedAttemptClaimDisposition::Claimed,
        thread_claim_disposition: ManagedWorkerThreadDispatchDisposition::Claimed,
        turn_claim_disposition: ManagedWorkerTurnDispatchDisposition::Claimed,
        review_claim_disposition: ManagedReviewDispatchDisposition::Claimed,
        dispatch_state: ManagedWorkerDispatchState::NoWorkerThread,
        closure_disposition: ManagedPrestartClosureDisposition::Closed,
        sequence: 0,
        execution_trace: Rc::new(RefCell::new(Vec::new())),
    };
    let worker = FakeWorker {
        calls: Vec::new(),
        exact_start: true,
        availability: ManagedModelAvailability::Available,
        reconciliations: std::collections::VecDeque::new(),
        prestart_recoveries: std::collections::VecDeque::new(),
        terminal: WorkerTerminal::Completed,
        execution_trace: Rc::new(RefCell::new(Vec::new())),
    };
    let verifier = FakeVerifier {
        calls: Vec::new(),
        outcome: verification_outcome,
        authorize_turn: false,
        fail_prepare_with_evidence: false,
    };
    (workflow, lifecycle, lease, repository, worker, verifier)
}

#[test]
fn high_level_workflow_owns_task_and_writer_order_and_stops_before_merge() {
    let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
        workflow_fixture(VerificationOutcome::Passed);

    let outcome = run_managed_workflow(
        &workflow,
        &mut lifecycle,
        &mut lease,
        &mut repository,
        &mut worker,
        &mut verifier,
    )
    .expect("managed workflow");
    assert_eq!(
        outcome.lifecycle().state(),
        TaskState::AwaitingMergeApproval
    );
    assert_eq!(
        lifecycle.transitions,
        vec![
            (TaskState::Draft, TaskState::AwaitingExecutionApproval),
            (TaskState::AwaitingExecutionApproval, TaskState::Preparing),
            (TaskState::Preparing, TaskState::Executing),
            (TaskState::Executing, TaskState::Verifying),
            (TaskState::Verifying, TaskState::Reviewing),
            (TaskState::Reviewing, TaskState::AwaitingMergeApproval),
        ]
    );
    assert!(lease.released);
    assert!(
        lease
            .current_authority(workflow_subject_binding().project_id())
            .expect("current")
            .is_none()
    );
}

#[test]
fn high_level_reviewer_is_configured_only_after_the_exact_durable_claim() {
    let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
        workflow_fixture(VerificationOutcome::Passed);

    run_managed_workflow_with_review_configuration_and_verified_hook(
        &workflow,
        &mut lifecycle,
        &mut lease,
        &mut repository,
        &mut worker,
        &mut verifier,
        |_| Ok(()),
        |claimed, repository, verifier| {
            assert_eq!(
                claimed.disposition(),
                ManagedReviewDispatchDisposition::Claimed
            );
            assert_eq!(repository.calls.last(), Some(&"claim-review"));
            assert_eq!(verifier.calls, vec!["prepare"]);
            verifier.calls.push("configure-review");
            Ok(())
        },
        |_, _, _| Ok(()),
    )
    .expect("post-claim configured workflow");

    assert_eq!(
        verifier.calls,
        vec!["prepare", "configure-review", "review", "verify"]
    );
}

#[test]
fn high_level_writer_validation_fails_before_any_provider_or_attempt_claim() {
    let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
        workflow_fixture(VerificationOutcome::Passed);

    let failure = run_managed_workflow_with_review_configuration_and_verified_hook(
        &workflow,
        &mut lifecycle,
        &mut lease,
        &mut repository,
        &mut worker,
        &mut verifier,
        |_| {
            Err(ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                "LATTICE_MANAGED_WRITER_PROCESS_TAKEOVER_REQUIRED",
            ))
        },
        |_, _, _| Ok(()),
        |_, _, _| Ok(()),
    )
    .expect_err("foreign process writer must fail closed");

    let ManagedWorkflowError::Attempt(failure) = failure else {
        panic!("writer validation must remain an attempt failure");
    };
    let ManagedAttemptOrchestratorError::ProviderEffectGuard(failure) = *failure else {
        panic!("writer validation must remain a provider guard failure");
    };
    assert_eq!(
        failure.code(),
        "LATTICE_MANAGED_WRITER_PROCESS_TAKEOVER_REQUIRED"
    );
    assert!(repository.attempts.is_empty());
    assert!(worker.calls.is_empty());
    assert!(!lease.released);
    assert_eq!(lifecycle.state, TaskState::AwaitingExecutionApproval);
}

#[test]
fn verified_hook_failure_precedes_awaiting_merge_and_writer_release() {
    let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
        workflow_fixture(VerificationOutcome::Passed);

    let error = run_managed_workflow_with_verified_hook(
        &workflow,
        &mut lifecycle,
        &mut lease,
        &mut repository,
        &mut worker,
        &mut verifier,
        |attempt, _repository, writer| {
            assert_eq!(
                attempt.verification().outcome(),
                VerificationOutcome::Passed
            );
            assert!(attempt.verification().review_digest().is_some());
            assert!(
                writer
                    .current_authority(workflow_subject_binding().project_id())
                    .expect("current writer in verified hook")
                    .is_some(),
                "verified mutation hook must receive the still-current Writer owner"
            );
            Err(ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "LATTICE_MANAGED_PROTECTED_REF_REJECTED",
            ))
        },
    )
    .expect_err("protected ref failure must stop before lifecycle advance or release");

    let lattice_orchestrator::ManagedWorkflowError::Attempt(attempt_error) = error else {
        panic!("unexpected workflow error: {error:?}");
    };
    assert!(matches!(
        *attempt_error,
        ManagedAttemptOrchestratorError::Repository(ref failure)
            if failure.code() == "LATTICE_MANAGED_PROTECTED_REF_REJECTED"
    ));
    assert_eq!(lifecycle.state, TaskState::Reviewing);
    assert!(!lease.released);
    assert!(
        lease
            .current_authority(workflow_subject_binding().project_id())
            .expect("current writer")
            .is_some()
    );
}

#[test]
fn retained_awaiting_zero_attempt_reenters_once_after_fresh_authority_checks() {
    let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
        workflow_fixture(VerificationOutcome::Passed);
    lifecycle.state = TaskState::AwaitingExecutionApproval;

    let outcome = run_managed_workflow(
        &workflow,
        &mut lifecycle,
        &mut lease,
        &mut repository,
        &mut worker,
        &mut verifier,
    )
    .expect("retained pre-authorized task dispatches");

    assert_eq!(
        outcome.lifecycle().state(),
        TaskState::AwaitingMergeApproval
    );
    assert_eq!(
        lifecycle.transitions.first(),
        Some(&(TaskState::AwaitingExecutionApproval, TaskState::Preparing,))
    );
    assert_eq!(
        repository
            .calls
            .iter()
            .filter(|call| **call == "authority")
            .count(),
        2,
        "authority is checked before writer and again immediately before claim"
    );
    assert_eq!(
        repository
            .calls
            .iter()
            .filter(|call| **call == "claim")
            .count(),
        1
    );
    assert_eq!(
        worker
            .calls
            .iter()
            .filter(|call| **call == "thread-accepted")
            .count(),
        1
    );
    assert_eq!(
        worker
            .calls
            .iter()
            .filter(|call| **call == "turn-accepted")
            .count(),
        1
    );
}

#[test]
fn missing_execution_authority_stays_awaiting_without_writer_or_codex_effect() {
    let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
        workflow_fixture(VerificationOutcome::Passed);
    repository.authority_digest = digest('9');

    let error = run_managed_workflow(
        &workflow,
        &mut lifecycle,
        &mut lease,
        &mut repository,
        &mut worker,
        &mut verifier,
    )
    .expect_err("missing exact authority must fail before writer acquisition");
    assert!(matches!(
        error,
        lattice_orchestrator::ManagedWorkflowError::Attempt(_)
    ));
    assert_eq!(lifecycle.state, TaskState::AwaitingExecutionApproval);
    assert!(
        lease
            .current_authority(workflow_subject_binding().project_id())
            .expect("current writer")
            .is_none()
    );
    assert_eq!(repository.calls, vec!["authority"]);
    assert!(worker.calls.is_empty());
    assert!(verifier.calls.is_empty());
}

#[test]
fn non_preapproving_intake_receipt_proceeds_only_after_formal_execution_authority_verifies() {
    let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
        workflow_fixture(VerificationOutcome::Passed);
    lifecycle.autonomy = Some(
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
        .expect("non-preapproving intake receipt"),
    );

    let outcome = run_managed_workflow(
        &workflow,
        &mut lifecycle,
        &mut lease,
        &mut repository,
        &mut worker,
        &mut verifier,
    )
    .expect("formal task-bound authority satisfies the later execution gate");

    assert_eq!(
        outcome.lifecycle().state(),
        TaskState::AwaitingMergeApproval
    );
    assert_eq!(repository.calls.first(), Some(&"authority"));
    assert!(lease.released);
}

#[test]
fn rejected_tampered_or_expired_authority_never_reaches_writer_claim_or_codex() {
    for failure_code in [
        "EXECUTION_AUTHORITY_MISSING",
        "EXECUTION_AUTHORITY_POLICY_EVIDENCE_MISMATCH",
        "EXECUTION_AUTHORITY_EXPIRED",
    ] {
        let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
            workflow_fixture(VerificationOutcome::Passed);
        repository.authority_failure_code = Some(failure_code);

        run_managed_workflow(
            &workflow,
            &mut lifecycle,
            &mut lease,
            &mut repository,
            &mut worker,
            &mut verifier,
        )
        .expect_err("invalid authority must stop before any writer or Codex effect");

        assert_eq!(lifecycle.state, TaskState::AwaitingExecutionApproval);
        assert!(
            lease
                .current_authority(workflow_subject_binding().project_id())
                .expect("current writer")
                .is_none(),
            "{failure_code} acquired a writer lease"
        );
        assert_eq!(repository.calls, vec!["authority"]);
        assert!(repository.attempts.is_empty());
        assert!(worker.calls.is_empty(), "{failure_code} reached Codex");
        assert!(verifier.calls.is_empty());
    }
}

#[test]
fn review_failure_retains_exact_writer_and_reviewing_state_for_repair() {
    let (workflow, mut lifecycle, mut lease, mut repository, mut worker, mut verifier) =
        workflow_fixture(VerificationOutcome::Failed);

    let error = run_managed_workflow(
        &workflow,
        &mut lifecycle,
        &mut lease,
        &mut repository,
        &mut worker,
        &mut verifier,
    )
    .expect_err("failed verification retains writer for repair");
    let lattice_orchestrator::ManagedWorkflowError::Attempt(attempt_error) = error else {
        panic!("unexpected workflow error: {error:?}");
    };
    assert!(matches!(
        *attempt_error,
        ManagedAttemptOrchestratorError::VerificationFailed(_)
    ));
    assert_eq!(lifecycle.state, TaskState::Reviewing);
    assert!(!lease.released);
    let current = lease
        .current_authority(workflow_subject_binding().project_id())
        .expect("current")
        .expect("retained authority");
    assert_eq!(
        current.independent_head().identity().fencing_token().get(),
        1
    );
}
