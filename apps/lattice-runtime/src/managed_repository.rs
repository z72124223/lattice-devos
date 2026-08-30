//! Same-database repository adapter for one formally promoted managed task.
//!
//! Task Ledger owns every semantic child event. The foreman extension stores
//! only the corresponding bounded payloads and evidence in the same formal
//! `PostgreSQL` profile. Every public operation reloads and owner-verifies both
//! sides before returning a usable record.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use lattice_approval_verifier::{
    ClosedPolicyExecutionContext, ExecutionAuthoritySource, ExecutionCapability,
    VerifiedExecutionAuthority, reverify_closed_policy_execution_authority,
};
use lattice_artifact_store::{ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence};
use lattice_contracts::{
    AttemptId, ContentDigest, ProjectAuthorityHead, ProjectAuthorityReceipt, ProjectClass,
    ProjectLifecycle, RuntimeAdmissionMode, StoreAuthorityHead, SubjectBinding,
    TaskLedgerStreamIdentity, TaskSpecSubmission,
};
use lattice_foreman_state::{AttemptPacketIdentity, WorkerBudget, WorkerModel};
use lattice_policy::{
    Boundary, ExecutionGate, ExecutionGateDecisionEvidence, ManagedExecutionBindingFact,
    ProjectAuthorityFact, RuntimeAdmission, TaskContext,
    evaluate_managed_execution_gate_with_evidence,
};
use lattice_ports::{
    ManagedArtifactReceipt, ManagedAttemptClaim, ManagedAttemptClaimDisposition,
    ManagedForemanRepositoryPort, ManagedPortError, ManagedPortErrorKind, ManagedPortResult,
    ManagedPrestartClosureDisposition, ManagedPrestartNoEffectProof,
    ManagedReviewDispatchDisposition, ManagedVerificationEvidence, ManagedVerificationRequest,
    ManagedWorkerDispatchState, ManagedWorkerObservation, ManagedWorkerThreadDispatchDisposition,
    ManagedWorkerTurnDispatchDisposition,
};
use lattice_postgres_foreman::{
    AdapterError, AdapterErrorKind, AppendDisposition, AttemptClosure, ClaimDisposition,
    ClaimReservationDisposition, ExecutionEnvironmentDescriptor, MAX_ARTIFACT_BYTES_PER_ATTEMPT,
    MAX_ARTIFACT_BYTES_PER_TASK, MAX_ARTIFACTS_PER_ATTEMPT, MAX_ARTIFACTS_PER_TASK,
    ManagedPromotionSource, NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF, PendingWorkerAttempt,
    PersistedExecutionEnvironment, PersistedReferenceLinks, PersistedTaskRuntimeRows,
    PostgresForeman, ProviderDispatchClaim, ProviderDispatchKind, ReplayRecord, ReplayRecordState,
    StagedArtifactReference, TaskReplay,
};
use lattice_postgres_store::{
    MigrationTarget as StoreTarget, PostgresProjectRegistry, PostgresTaskLedger,
    PostgresTaskLedgerError, PostgresTaskLedgerErrorKind,
};
use lattice_task_domain::{TaskSpec, TaskState};
use lattice_task_ledger::{
    CommandId, CorrelationId, NO_PROVIDER_EFFECT_CLOSURE_OWNER, TaskCreatedProfile,
    TaskExecutionBindingInput, TaskRuntimeAppendMetadata, TaskSubmissionEnvelope,
    TaskVerificationInput, UntrustedWorkerAttemptRow, VerifiedNoProviderEffectPredecessor,
    VerifiedStream, VerifiedTaskExecutionBinding, VerifiedTaskRuntimeRecords,
    VerifiedTaskVerificationRecord, VerifiedWorkerAttemptRecord, VerifiedWorkerObservationRecord,
    WorkerAttemptAppendPlan, WorkerAttemptInput, classify_task_created_profile,
    plan_approval_evidence_append, plan_artifact_reference_append, plan_task_execution_binding,
    plan_task_verification_append, plan_worker_attempt_append,
    plan_worker_attempt_append_with_no_provider_effect_predecessor, plan_worker_observation_append,
    recover_task_verification_record, recover_worker_attempt_record,
    recover_worker_observation_record, task_execution_binding_is_recorded,
    verify_approval_evidence_links, verify_artifact_reference_links,
    verify_untrusted_task_execution_binding, verify_untrusted_task_runtime_records,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::delivery_ledger::{DeliveryDatabaseBinding, connect_fixed_runtime_client};
use crate::managed_task_spec::rebuild_managed_task_spec_from_submission;

const CORRELATION_ID: &str = "managed-foreman-runtime-v1";
const CROSS_OWNER_SNAPSHOT_RETRY_LIMIT: usize = 4;
const CROSS_OWNER_SNAPSHOT_RETRY_BASE_DELAY: Duration = Duration::from_millis(5);
const PROVIDER_DISPATCH_KINDS: [ProviderDispatchKind; 4] = [
    ProviderDispatchKind::WorkerThread,
    ProviderDispatchKind::WorkerTurn,
    ProviderDispatchKind::ReviewThread,
    ProviderDispatchKind::ReviewTurn,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttemptClaimPersistenceStep {
    RecordExecutionEnvironment,
    ClaimCapacity,
}

const NATIVE_ATTEMPT_CLAIM_PERSISTENCE_STEPS: &[AttemptClaimPersistenceStep] =
    &[AttemptClaimPersistenceStep::ClaimCapacity];
const TYPED_ATTEMPT_CLAIM_PERSISTENCE_STEPS: &[AttemptClaimPersistenceStep] = &[
    AttemptClaimPersistenceStep::RecordExecutionEnvironment,
    AttemptClaimPersistenceStep::ClaimCapacity,
];

fn attempt_claim_persistence_steps(
    packet: &AttemptPacketIdentity,
    descriptor_environment_ref: Option<&str>,
) -> ManagedPortResult<&'static [AttemptClaimPersistenceStep]> {
    match (
        packet.is_native_windows_execution_environment(),
        descriptor_environment_ref,
    ) {
        (true, None) => Ok(NATIVE_ATTEMPT_CLAIM_PERSISTENCE_STEPS),
        (false, Some(descriptor_ref)) if descriptor_ref == packet.execution_environment_ref() => {
            Ok(TYPED_ATTEMPT_CLAIM_PERSISTENCE_STEPS)
        }
        _ => Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION")),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttemptReservationReplaySource {
    Pending,
    Active,
}

fn attempt_reservation_replay_source(
    disposition: ClaimReservationDisposition,
    pending_matches: Option<bool>,
    active_matches: usize,
) -> ManagedPortResult<AttemptReservationReplaySource> {
    match (disposition, pending_matches, active_matches) {
        (ClaimReservationDisposition::Reserved, Some(true), 0)
        | (ClaimReservationDisposition::ExactReplay, Some(true), 0) => {
            Ok(AttemptReservationReplaySource::Pending)
        }
        (ClaimReservationDisposition::ExactReplay, None, 1) => {
            Ok(AttemptReservationReplaySource::Active)
        }
        _ => Err(reconcile(
            "LATTICE_MANAGED_ATTEMPT_RESERVATION_RECONCILE_REQUIRED",
        )),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingExecutionEnvironmentPersistence {
    NativeWindows,
    DurableExact,
    RecordConfigured,
}

fn pending_execution_environment_persistence(
    expected_ref: &str,
    durable_ref: Option<&str>,
    configured_ref: Option<&str>,
) -> ManagedPortResult<PendingExecutionEnvironmentPersistence> {
    if expected_ref == NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF {
        return match (durable_ref, configured_ref) {
            (None, None) => Ok(PendingExecutionEnvironmentPersistence::NativeWindows),
            _ => Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION")),
        };
    }
    match (durable_ref, configured_ref) {
        (Some(durable), None) if durable == expected_ref => {
            Ok(PendingExecutionEnvironmentPersistence::DurableExact)
        }
        (Some(durable), Some(configured))
            if durable == expected_ref && configured == expected_ref =>
        {
            Ok(PendingExecutionEnvironmentPersistence::DurableExact)
        }
        (None, Some(configured)) if configured == expected_ref => {
            Ok(PendingExecutionEnvironmentPersistence::RecordConfigured)
        }
        _ => Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION")),
    }
}

/// Process-owned source for fresh Policy V2 owner facts. It contains no MCP
/// argument and opens a new fixed-role read on every authority assertion, so a
/// restart or Registry/runtime change cannot reuse a stale in-memory verdict.
#[derive(Clone)]
pub(crate) struct ManagedPolicyAuthoritySource {
    database: DeliveryDatabaseBinding,
    password: String,
    timeout: Duration,
    status_request_deadline: Option<Instant>,
    store_authority: StoreAuthorityHead,
}

impl ManagedPolicyAuthoritySource {
    pub(crate) fn new(
        database: DeliveryDatabaseBinding,
        password: String,
        timeout: Duration,
        store_authority: StoreAuthorityHead,
    ) -> ManagedPortResult<Self> {
        if password.is_empty() || timeout.is_zero() || timeout > Duration::from_secs(3_600) {
            return Err(known("LATTICE_MANAGED_POLICY_SOURCE_REJECTED"));
        }
        Ok(Self {
            database,
            password,
            timeout,
            status_request_deadline: None,
            store_authority,
        })
    }

    pub(crate) fn with_status_request_deadline(mut self, deadline: Option<Instant>) -> Self {
        self.status_request_deadline = deadline;
        self
    }

    fn deadline(&self) -> ManagedPortResult<Instant> {
        if let Some(deadline) = self.status_request_deadline {
            return (Instant::now() < deadline)
                .then_some(deadline)
                .ok_or_else(|| known("LATTICE_MANAGED_STATUS_TIMEOUT"));
        }
        Instant::now()
            .checked_add(self.timeout)
            .ok_or_else(|| known("LATTICE_MANAGED_POLICY_SOURCE_REJECTED"))
    }

    fn reconcile_read(&self, fallback_code: &'static str) -> ManagedPortError {
        if self
            .status_request_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            known("LATTICE_MANAGED_STATUS_TIMEOUT")
        } else {
            reconcile(fallback_code)
        }
    }

    pub(crate) fn current_project_authority(
        &self,
        submission: &TaskSubmissionEnvelope,
    ) -> ManagedPortResult<(ProjectAuthorityReceipt, ProjectAuthorityHead)> {
        let target = StoreTarget::new(self.database.database_name(), self.database.run_id())
            .map_err(|_| known("LATTICE_MANAGED_PROJECT_AUTHORITY_NOT_CURRENT"))?;
        let client = connect_fixed_runtime_client(&self.database, &self.password, self.deadline()?)
            .map_err(|_| self.reconcile_read("LATTICE_MANAGED_PROJECT_AUTHORITY_NOT_CURRENT"))?;
        let mut registry = PostgresProjectRegistry::new(client, &target)
            .map_err(|_| self.reconcile_read("LATTICE_MANAGED_PROJECT_AUTHORITY_NOT_CURRENT"))?;
        let loaded = registry
            .load()
            .map_err(|_| self.reconcile_read("LATTICE_MANAGED_PROJECT_AUTHORITY_NOT_CURRENT"))?;
        let current = loaded
            .state()
            .project(submission.identity().project_id())
            .ok_or_else(|| known("LATTICE_MANAGED_PROJECT_AUTHORITY_NOT_CURRENT"))?;
        let receipt = current.authority();
        if current.project_class() != ProjectClass::UserProject
            || receipt.lifecycle() != ProjectLifecycle::Active
            || current.pending_observation().is_some()
            || !current.drift().is_empty()
            || receipt.project_snapshot_id() != submission.identity().project_snapshot_id()
            || receipt.receipt_digest() != submission.project_authority_receipt_digest()
        {
            return Err(known("LATTICE_MANAGED_PROJECT_AUTHORITY_NOT_CURRENT"));
        }
        Ok((receipt.clone(), receipt.head()))
    }

    pub(crate) fn assert_runtime_active(&self) -> ManagedPortResult<()> {
        if self.store_authority.admission() != RuntimeAdmissionMode::Active {
            return Err(known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"));
        }
        let mut client =
            connect_fixed_runtime_client(&self.database, &self.password, self.deadline()?)
                .map_err(|_| self.reconcile_read("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?;
        let row = client
            .query_one(
                "SELECT admission_mode::text, daemon_instance_id, daemon_epoch, authority_revision, \
                        pg_catalog.encode(observation_digest,'hex'), \
                        pg_catalog.encode(authority_head_digest,'hex') \
                   FROM ONLY control.runtime_admission WHERE singleton",
                &[],
            )
            .map_err(|_| self.reconcile_read("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?;
        let mode: String = row
            .try_get(0)
            .map_err(|_| known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?;
        let daemon_instance_id: Option<String> = row
            .try_get(1)
            .map_err(|_| known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?;
        let daemon_epoch: Option<i64> = row
            .try_get(2)
            .map_err(|_| known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?;
        let authority_revision: i64 = row
            .try_get(3)
            .map_err(|_| known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?;
        let observation_digest: Option<String> = row
            .try_get(4)
            .map_err(|_| known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?;
        let authority_head_digest: Option<String> = row
            .try_get(5)
            .map_err(|_| known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?;
        if mode != "ACTIVE"
            || daemon_instance_id.as_deref()
                != Some(self.store_authority.daemon_instance_id().as_str())
            || daemon_epoch != i64::try_from(self.store_authority.daemon_epoch().get()).ok()
            || authority_revision
                != i64::try_from(self.store_authority.revision().get())
                    .map_err(|_| known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"))?
            || observation_digest.as_deref()
                != Some(self.store_authority.observation_digest().as_str())
            || authority_head_digest.as_deref() != Some(self.store_authority.head_digest().as_str())
        {
            return Err(known("LATTICE_MANAGED_RUNTIME_ADMISSION_NOT_ACTIVE"));
        }
        Ok(())
    }

    /// Loads the current owner facts and evaluates the exact immutable
    /// TaskSpec through Policy. The returned `PolicyDecision` is opaque and
    /// cannot be caller-constructed; Approval Verifier consumes it only as
    /// bounded decision evidence and never calls Policy itself.
    pub(crate) fn evaluate_execution_gate(
        &self,
        submission: &TaskSubmissionEnvelope,
        task_spec: &TaskSpec,
        binding: &SubjectBinding,
        execution_binding: &VerifiedTaskExecutionBinding,
    ) -> ManagedPortResult<(
        ProjectAuthorityReceipt,
        ProjectAuthorityHead,
        ExecutionGateDecisionEvidence,
    )> {
        self.assert_runtime_active()?;
        let (project_receipt, current_project_head) = self.current_project_authority(submission)?;
        let decision = evaluate_managed_execution_gate_with_evidence(
            ExecutionGate {
                context: TaskContext {
                    task_spec: Some(task_spec),
                    project: Some(ProjectAuthorityFact {
                        binding: binding.clone(),
                        receipt: project_receipt.clone(),
                        current_head: current_project_head.clone(),
                    }),
                    state: Boundary::Known(TaskState::AwaitingExecutionApproval),
                    runtime_admission: Boundary::Known(RuntimeAdmission::Active),
                },
                approval: None,
            },
            ManagedExecutionBindingFact {
                task_ref: execution_binding.task_ref().clone(),
                successor_stream_id: execution_binding.successor_stream_id().clone(),
                task_spec_digest: execution_binding.task_spec_digest().clone(),
                approval_subject_digest: execution_binding.approval_subject_digest().clone(),
                budget_digest: execution_binding.budget_digest().clone(),
            },
        );
        Ok((project_receipt, current_project_head, decision))
    }

    fn reverify(
        &self,
        submission: &TaskSubmissionEnvelope,
        managed_submission: &TaskSpecSubmission,
        loaded: &LoadedRuntime,
        authority: &VerifiedExecutionAuthority,
    ) -> ManagedPortResult<()> {
        self.reverify_status_projection(
            submission,
            managed_submission,
            &loaded.binding,
            &loaded.source,
            authority,
        )
    }

    fn reverify_status_projection(
        &self,
        submission: &TaskSubmissionEnvelope,
        managed_submission: &TaskSpecSubmission,
        binding: &VerifiedTaskExecutionBinding,
        source: &ManagedPromotionSource,
        authority: &VerifiedExecutionAuthority,
    ) -> ManagedPortResult<()> {
        if authority.source() == ExecutionAuthoritySource::VerifiedApproval {
            // `PostgresForeman::load_execution_authority` has already restored
            // the exact Approval-owner snapshot against its independently
            // retained checkpoint and reverified the BIND_EXECUTION receipt.
            // Policy owns only the closed-policy lane; applying that evaluator
            // to a verified approval would discard the Approval-owner proof.
            if authority.capability() != ExecutionCapability::LocalReversibleTaskExecution
                || !authority_matches_binding(authority, binding)
                || !authority_is_current(authority)?
            {
                return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"));
            }
            return Ok(());
        }
        let managed = rebuild_managed_task_spec_from_submission(
            submission,
            source.base_ref(),
            source.base_commit(),
            managed_submission,
        )
        .map_err(|_| known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"))?;
        let (project_receipt, current_project_head, decision) = self.evaluate_execution_gate(
            submission,
            managed.task_spec(),
            managed.submission().binding(),
            binding,
        )?;
        let context = ClosedPolicyExecutionContext::new(
            binding.task_ref().clone(),
            binding.successor_stream_id().clone(),
            managed.submission().binding().clone(),
            binding.approval_subject_digest().clone(),
            binding.budget_digest().clone(),
            project_receipt,
            current_project_head,
            authority.issued_at(),
            authority.expires_at(),
        )
        .map_err(|_| known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"))?;
        let observed_at = now_utc()?;
        reverify_closed_policy_execution_authority(authority, &context, &decision, &observed_at)
            .map_err(|_| known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"))
    }
}

struct LoadedRuntime {
    stream: VerifiedStream,
    binding: VerifiedTaskExecutionBinding,
    rows: PersistedTaskRuntimeRows,
    references: PersistedReferenceLinks,
    budget: WorkerBudget,
    source: ManagedPromotionSource,
    pending_attempt: Option<PendingWorkerAttempt>,
    execution_environments: Vec<PersistedExecutionEnvironment>,
    staged_artifact: Option<StagedArtifactReference>,
    task_replay: TaskReplay,
}

struct FreshRuntime {
    binding: VerifiedTaskExecutionBinding,
    authority: Option<VerifiedExecutionAuthority>,
    records: VerifiedTaskRuntimeRecords,
    references: PersistedReferenceLinks,
    evidence: Vec<VerifiedManagedEvidence>,
    budget: WorkerBudget,
    source: ManagedPromotionSource,
    pending_attempt: Option<VerifiedWorkerAttemptRecord>,
    task_replay: TaskReplay,
}

/// Fresh, owner-verified read projection used by status and restart
/// reconciliation. It exposes no client, SQL, prompt, environment, or secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedTaskReplayProjection {
    binding: VerifiedTaskExecutionBinding,
    authority: Option<VerifiedExecutionAuthority>,
    records: VerifiedTaskRuntimeRecords,
    references: PersistedReferenceLinks,
    evidence: Vec<VerifiedManagedEvidence>,
    budget: WorkerBudget,
    source: ManagedPromotionSource,
    pending_attempt: Option<VerifiedWorkerAttemptRecord>,
    task_replay: TaskReplay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RestartWriterBlockerRecordDisposition {
    Persisted,
    ExactReplay,
    DurableEvidenceReady,
}

impl ManagedTaskReplayProjection {
    #[must_use]
    pub const fn binding(&self) -> &VerifiedTaskExecutionBinding {
        &self.binding
    }

    #[must_use]
    pub const fn authority(&self) -> Option<&VerifiedExecutionAuthority> {
        self.authority.as_ref()
    }

    #[must_use]
    pub const fn records(&self) -> &VerifiedTaskRuntimeRecords {
        &self.records
    }

    #[must_use]
    pub const fn references(&self) -> &PersistedReferenceLinks {
        &self.references
    }

    /// Returns Artifact Store owner-verified evidence in exact reference order.
    #[must_use]
    pub fn evidence(&self) -> &[VerifiedManagedEvidence] {
        &self.evidence
    }

    #[must_use]
    pub const fn budget(&self) -> &WorkerBudget {
        &self.budget
    }

    #[must_use]
    pub const fn source(&self) -> &ManagedPromotionSource {
        &self.source
    }

    /// Returns the exact Ledger-owned attempt waiting for an atomic capacity
    /// claim. A pending attempt has not launched a Codex thread or turn and
    /// must never be treated as active worker progress.
    #[must_use]
    pub const fn pending_attempt(&self) -> Option<&VerifiedWorkerAttemptRecord> {
        self.pending_attempt.as_ref()
    }

    #[must_use]
    pub const fn task_replay(&self) -> &TaskReplay {
        &self.task_replay
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        self.task_replay.evidence_digest()
    }
}

/// Owner-verified result of the one promotion plus retained execution-authority
/// evidence. It does not itself grant execution; active dispatch must still
/// assert the exact authority digest and current validity window.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedPromotionBootstrap {
    binding: VerifiedTaskExecutionBinding,
    authority: VerifiedExecutionAuthority,
    source: ManagedPromotionSource,
}

/// Durable intake-to-TaskSpec binding retained before execution authority.
///
/// This is not permission to dispatch. It only binds the formal successor,
/// immutable source observation, budget and verification policy so a process
/// crash or a later Git HEAD change cannot create a second successor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedPromotionBinding {
    binding: VerifiedTaskExecutionBinding,
    source: ManagedPromotionSource,
}

impl ManagedPromotionBinding {
    #[must_use]
    pub const fn binding(&self) -> &VerifiedTaskExecutionBinding {
        &self.binding
    }

    #[must_use]
    pub const fn source(&self) -> &ManagedPromotionSource {
        &self.source
    }
}

impl ManagedPromotionBootstrap {
    #[must_use]
    pub const fn binding(&self) -> &VerifiedTaskExecutionBinding {
        &self.binding
    }

    #[must_use]
    pub const fn authority(&self) -> &VerifiedExecutionAuthority {
        &self.authority
    }

    #[must_use]
    pub const fn source(&self) -> &ManagedPromotionSource {
        &self.source
    }
}

/// Reconstructs the one immutable promotion binding without treating it as
/// execution authority. This is the owner read used while a task is durably
/// awaiting a separately verified approval receipt.
pub fn load_existing_managed_promotion_binding(
    ledger: &mut PostgresTaskLedger,
    foreman: &mut PostgresForeman,
    intake: &TaskSubmissionEnvelope,
    managed_submission: &TaskSpecSubmission,
    successor_identity: &TaskLedgerStreamIdentity,
) -> ManagedPortResult<Option<ManagedPromotionBinding>> {
    let intake_stream = ledger
        .load_stream(intake.identity().clone())
        .map_err(map_ledger_read)?;
    let successor = ledger
        .load_stream(successor_identity.clone())
        .map_err(map_ledger_read)?;
    require_managed_successor(successor.stream())?;
    let binding_recorded =
        task_execution_binding_is_recorded(intake_stream.stream(), successor.stream(), intake)
            .map_err(map_domain)?;
    let replay = foreman
        .read_task_replay(intake.task_ref())
        .map_err(map_foreman_read)?;
    if !binding_recorded {
        return if replay.records().is_empty() {
            Ok(None)
        } else {
            Err(known("LATTICE_MANAGED_PROMOTION_REPLAY_REJECTED"))
        };
    }
    if replay.task_ref() != intake.task_ref()
        || is_zero(replay.evidence_digest())
        || replay
            .records()
            .iter()
            .filter(|record| record.record_kind() == "TASK_PROMOTION")
            .count()
            != 1
        || replay.records().first().is_none_or(|record| {
            record.record_kind() != "TASK_PROMOTION"
                || record.record_state() != ReplayRecordState::Retained
        })
    {
        return Err(reconcile("LATTICE_MANAGED_PROMOTION_RECONCILE_REQUIRED"));
    }
    let rows = foreman
        .load_task_runtime_rows(intake.task_ref())
        .map_err(map_foreman_read)?;
    let binding = verify_untrusted_task_execution_binding(
        intake_stream.stream(),
        successor.stream(),
        intake,
        rows.binding(),
    )
    .map_err(map_domain)?;
    let source = foreman
        .load_task_promotion_source(binding.task_ref())
        .map_err(map_foreman_read)?
        .ok_or_else(|| reconcile("LATTICE_MANAGED_PROMOTION_SOURCE_RECONCILE_REQUIRED"))?;
    verify_promotion_source(
        intake,
        managed_submission,
        successor_identity,
        &binding,
        &source,
    )?;
    Ok(Some(ManagedPromotionBinding { binding, source }))
}

/// Reconstructs the one existing managed promotion and its retained execution
/// authority from `PostgreSQL` without accepting caller-supplied authority
/// fields.
///
/// `require_current` must be `true` before active dispatch. Status and restart
/// projections for already terminal work may pass `false` to retain an expired
/// authority as historical evidence; this function never refreshes or grants
/// that authority. An ambiguous authority set is rejected in either mode.
///
/// # Errors
///
/// Cross-stream lineage, a Ledger/extension half-write, duplicate promotion or
/// authority rows, unowned approval links, a changed authority binding, and a
/// non-current authority when `require_current` is true all fail closed.
pub fn load_existing_managed_bootstrap(
    ledger: &mut PostgresTaskLedger,
    foreman: &mut PostgresForeman,
    intake: &TaskSubmissionEnvelope,
    managed_submission: &TaskSpecSubmission,
    successor_identity: &TaskLedgerStreamIdentity,
    require_current: bool,
) -> ManagedPortResult<Option<ManagedPromotionBootstrap>> {
    let mut last_skew = None;
    for pass in 0..CROSS_OWNER_SNAPSHOT_RETRY_LIMIT {
        match load_existing_managed_bootstrap_once(
            ledger,
            foreman,
            intake,
            managed_submission,
            successor_identity,
            require_current,
        ) {
            Ok(bootstrap) => return Ok(bootstrap),
            Err(error) if cross_owner_snapshot_retry_allowed(pass, &error) => {
                last_skew = Some(error);
                std::thread::sleep(cross_owner_snapshot_retry_delay(pass));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_skew.unwrap_or_else(|| known("LEDGER_INVALID_TASK_RUNTIME_RECORD")))
}

fn load_existing_managed_bootstrap_once(
    ledger: &mut PostgresTaskLedger,
    foreman: &mut PostgresForeman,
    intake: &TaskSubmissionEnvelope,
    managed_submission: &TaskSpecSubmission,
    successor_identity: &TaskLedgerStreamIdentity,
    require_current: bool,
) -> ManagedPortResult<Option<ManagedPromotionBootstrap>> {
    let Some(promotion) = load_existing_managed_promotion_binding(
        ledger,
        foreman,
        intake,
        managed_submission,
        successor_identity,
    )?
    else {
        return Ok(None);
    };
    let successor = ledger
        .load_stream(successor_identity.clone())
        .map_err(map_ledger_read)?;
    let binding = promotion.binding;
    let source = promotion.source;
    let references = foreman
        .load_reference_links(binding.task_ref())
        .map_err(map_foreman_read)?;
    let approval_links = references
        .approval_links()
        .iter()
        .map(|reference| reference.link().clone())
        .collect::<Vec<_>>();
    verify_approval_evidence_links(successor.stream(), &binding, &approval_links)
        .map_err(map_domain)?;
    let authorities = foreman
        .load_execution_authorities(binding.task_ref())
        .map_err(map_foreman_read)?;
    if authorities.is_empty() && references.approval_links().is_empty() {
        return Ok(None);
    }
    let [authority] = authorities.as_slice() else {
        return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_AMBIGUOUS"));
    };
    let [approval_reference] = references.approval_links() else {
        return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_AMBIGUOUS"));
    };
    if !authority_matches_binding(authority, &binding)
        || authority.capability() != ExecutionCapability::LocalReversibleTaskExecution
        || approval_reference.authority_digest() != authority.authority_digest()
        || approval_reference.link().payload_digest() != authority.authority_digest()
    {
        return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_REJECTED"));
    }
    if require_current && !authority_is_current(authority)? {
        return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"));
    }
    Ok(Some(ManagedPromotionBootstrap {
        binding,
        authority: authority.clone(),
        source,
    }))
}

/// Appends and extension-persists one exact intake-to-TaskSpec promotion and
/// its independently verified local execution authority.
///
/// The managed `TaskSpec` stream must already have been admitted through the
/// existing lifecycle/autonomy path. This helper owns only the subsequent
/// promotion and authority child events. Both dual writes are restart-safe:
/// if Task Ledger committed before the extension row, the same metadata and
/// input recover the exact verified row and finish the extension write.
///
/// # Errors
///
/// Changed metadata/input, a non-matching `TaskSpec` submission, missing
/// admission, cross-bound authority, persistence ambiguity, and owner-replay
/// disagreement all fail closed. An expired but otherwise verified authority
/// may be retained as historical crash-recovery evidence; this helper never
/// grants execution and active dispatch must separately assert currentness.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn record_managed_promotion_binding(
    ledger: &mut PostgresTaskLedger,
    foreman: &mut PostgresForeman,
    store_authority: &StoreAuthorityHead,
    intake: &TaskSubmissionEnvelope,
    managed_submission: &TaskSpecSubmission,
    successor_identity: &TaskLedgerStreamIdentity,
    promotion_source: &ManagedPromotionSource,
    approval_subject_digest: ContentDigest,
    budget: &WorkerBudget,
    verification_policy_digest: ContentDigest,
    promotion_metadata: TaskRuntimeAppendMetadata,
) -> ManagedPortResult<ManagedPromotionBinding> {
    if !managed_submission_matches_identity(managed_submission, successor_identity) {
        return Err(known("LATTICE_MANAGED_PROMOTION_INPUT_REJECTED"));
    }
    let rebuilt = rebuild_managed_task_spec_from_submission(
        intake,
        promotion_source.base_ref(),
        promotion_source.base_commit(),
        managed_submission,
    )
    .map_err(|_| known("LATTICE_MANAGED_PROMOTION_SOURCE_REJECTED"))?;
    if rebuilt.submission() != managed_submission
        || rebuilt.approval_subject_digest() != &approval_subject_digest
        || rebuilt.verification_policy_digest() != &verification_policy_digest
    {
        return Err(known("LATTICE_MANAGED_PROMOTION_SOURCE_REJECTED"));
    }
    let budget_digest = pointer_content(budget.digest(), "budget")?;
    let intake_stream = ledger
        .load_stream(intake.identity().clone())
        .map_err(map_ledger_read)?;
    let successor = ledger
        .load_stream(successor_identity.clone())
        .map_err(map_ledger_read)?;
    require_managed_successor(successor.stream())?;
    if lattice_gateway_ipc::task_spec_document_digest(managed_submission.canonical_document())
        .map_err(|_| known("LATTICE_MANAGED_TASK_SPEC_DOCUMENT_REJECTED"))?
        != *managed_submission.claimed_spec_digest()
    {
        return Err(known("LATTICE_MANAGED_TASK_SPEC_ADMISSION_REJECTED"));
    }

    let replay = foreman
        .read_task_replay(intake.task_ref())
        .map_err(map_foreman_read)?;
    let existing = if replay.records().is_empty() {
        Vec::new()
    } else {
        if replay
            .records()
            .first()
            .is_none_or(|record| record.record_kind() != "TASK_PROMOTION")
        {
            return Err(known("LATTICE_MANAGED_PROMOTION_REPLAY_REJECTED"));
        }
        let rows = foreman
            .load_task_runtime_rows(intake.task_ref())
            .map_err(map_foreman_read)?;
        vec![
            verify_untrusted_task_execution_binding(
                intake_stream.stream(),
                successor.stream(),
                intake,
                rows.binding(),
            )
            .map_err(map_domain)?,
        ]
    };
    let plan = plan_task_execution_binding(
        intake_stream.stream(),
        successor.stream(),
        intake,
        &existing,
        promotion_metadata,
        TaskExecutionBindingInput::new(
            approval_subject_digest,
            budget_digest,
            verification_policy_digest,
        )
        .map_err(map_domain)?,
    )
    .map_err(map_domain)?;
    ledger
        .execute(
            plan.ledger_plan().command_record().request().clone(),
            store_authority.clone(),
        )
        .map_err(map_ledger_write)?;
    foreman
        .record_task_promotion(plan.binding(), budget, promotion_source)
        .map_err(map_foreman_write)?;

    let successor = ledger
        .load_stream(successor_identity.clone())
        .map_err(map_ledger_read)?;
    let rows = foreman
        .load_task_runtime_rows(intake.task_ref())
        .map_err(map_foreman_read)?;
    let binding = verify_untrusted_task_execution_binding(
        intake_stream.stream(),
        successor.stream(),
        intake,
        rows.binding(),
    )
    .map_err(map_domain)?;
    if binding != *plan.binding() {
        return Err(known("LATTICE_MANAGED_PROMOTION_REPLAY_REJECTED"));
    }

    let retained = load_existing_managed_promotion_binding(
        ledger,
        foreman,
        intake,
        managed_submission,
        successor_identity,
    )?
    .ok_or_else(|| reconcile("LATTICE_MANAGED_PROMOTION_RECONCILE_REQUIRED"))?;
    if retained.binding() != &binding || retained.source() != promotion_source {
        return Err(reconcile("LATTICE_MANAGED_PROMOTION_RECONCILE_REQUIRED"));
    }
    Ok(retained)
}

/// Appends the independently verified execution authority after the immutable
/// promotion binding is already durable. A denial or process crash therefore
/// leaves an exact AWAITING_EXECUTION_APPROVAL replay, never a new TaskSpec.
pub fn append_managed_execution_authority(
    ledger: &mut PostgresTaskLedger,
    foreman: &mut PostgresForeman,
    store_authority: &StoreAuthorityHead,
    intake: &TaskSubmissionEnvelope,
    managed_submission: &TaskSpecSubmission,
    successor_identity: &TaskLedgerStreamIdentity,
    promotion: &ManagedPromotionBinding,
    authority: &VerifiedExecutionAuthority,
    approval_metadata: TaskRuntimeAppendMetadata,
) -> ManagedPortResult<ManagedPromotionBootstrap> {
    if !authority_is_bootstrap_evidence(authority, promotion.binding().approval_subject_digest())
        || !authority_matches_binding(authority, promotion.binding())
    {
        return Err(known("LATTICE_MANAGED_PROMOTION_INPUT_REJECTED"));
    }
    let successor = ledger
        .load_stream(successor_identity.clone())
        .map_err(map_ledger_read)?;
    require_managed_successor(successor.stream())?;
    let binding = promotion.binding().clone();

    let references = foreman
        .load_reference_links(binding.task_ref())
        .map_err(map_foreman_read)?;
    let existing_approval_links = references
        .approval_links()
        .iter()
        .map(|reference| reference.link().clone())
        .collect::<Vec<_>>();
    let approval_plan = plan_approval_evidence_append(
        successor.stream(),
        &binding,
        &existing_approval_links,
        approval_metadata,
        authority.authority_digest().clone(),
    )
    .map_err(map_domain)?;
    ledger
        .execute(
            approval_plan
                .ledger_plan()
                .command_record()
                .request()
                .clone(),
            store_authority.clone(),
        )
        .map_err(map_ledger_write)?;
    foreman
        .record_approval_evidence(authority, approval_plan.link())
        .map_err(map_foreman_write)?;

    let retained = load_existing_managed_bootstrap(
        ledger,
        foreman,
        intake,
        managed_submission,
        successor_identity,
        false,
    )?
    .ok_or_else(|| reconcile("LATTICE_MANAGED_PROMOTION_RECONCILE_REQUIRED"))?;
    if retained.binding() != &binding
        || retained.authority() != authority
        || retained.source() != promotion.source()
    {
        return Err(reconcile("LATTICE_MANAGED_PROMOTION_RECONCILE_REQUIRED"));
    }
    Ok(retained)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn bootstrap_managed_promotion(
    ledger: &mut PostgresTaskLedger,
    foreman: &mut PostgresForeman,
    store_authority: &StoreAuthorityHead,
    intake: &TaskSubmissionEnvelope,
    managed_submission: &TaskSpecSubmission,
    successor_identity: &TaskLedgerStreamIdentity,
    promotion_source: &ManagedPromotionSource,
    approval_subject_digest: ContentDigest,
    budget: &WorkerBudget,
    verification_policy_digest: ContentDigest,
    authority: &VerifiedExecutionAuthority,
    promotion_metadata: TaskRuntimeAppendMetadata,
    approval_metadata: TaskRuntimeAppendMetadata,
) -> ManagedPortResult<ManagedPromotionBootstrap> {
    let promotion = record_managed_promotion_binding(
        ledger,
        foreman,
        store_authority,
        intake,
        managed_submission,
        successor_identity,
        promotion_source,
        approval_subject_digest,
        budget,
        verification_policy_digest,
        promotion_metadata,
    )?;
    append_managed_execution_authority(
        ledger,
        foreman,
        store_authority,
        intake,
        managed_submission,
        successor_identity,
        &promotion,
        authority,
        approval_metadata,
    )
}

/// Concrete managed-foreman repository over the existing Task Ledger plus the
/// Store-v7-bound `foreman-execution/v1` extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VerifiedReviewThreadDispatch {
    claimed_at: String,
}

impl VerifiedReviewThreadDispatch {
    pub(crate) fn claimed_at(&self) -> &str {
        &self.claimed_at
    }
}

pub struct PostgresManagedForemanRepository {
    ledger: PostgresTaskLedger,
    foreman: PostgresForeman,
    store_authority: StoreAuthorityHead,
    submission: TaskSubmissionEnvelope,
    managed_submission: TaskSpecSubmission,
    successor_identity: TaskLedgerStreamIdentity,
    expected_binding: Option<VerifiedTaskExecutionBinding>,
    foreman_generation: u64,
    foreman_checkpoint_digest: ContentDigest,
    asserted_authority_digest: Option<ContentDigest>,
    policy_authority: ManagedPolicyAuthoritySource,
    execution_environment: Option<ExecutionEnvironmentDescriptor>,
    recover_staged_artifact_on_environment_install: bool,
}

impl PostgresManagedForemanRepository {
    /// Retains already-open, exact-profile adapters and proves the supplied
    /// promotion lineage plus approval references. Each effect operation then
    /// performs an intent-aware full child-row replay before writing or launch.
    ///
    /// # Errors
    ///
    /// Any missing promotion, changed lineage or approval reference, or
    /// malformed formal foreman identity fails closed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        ledger: PostgresTaskLedger,
        foreman: PostgresForeman,
        store_authority: StoreAuthorityHead,
        submission: TaskSubmissionEnvelope,
        managed_submission: TaskSpecSubmission,
        successor_identity: TaskLedgerStreamIdentity,
        expected_binding: VerifiedTaskExecutionBinding,
        foreman_generation: u64,
        foreman_checkpoint_digest: ContentDigest,
        policy_authority: ManagedPolicyAuthoritySource,
    ) -> ManagedPortResult<Self> {
        Self::new_with_recovery(
            ledger,
            foreman,
            store_authority,
            submission,
            managed_submission,
            successor_identity,
            Some(expected_binding),
            foreman_generation,
            foreman_checkpoint_digest,
            policy_authority,
            true,
        )
    }

    /// Builds a replay-only repository for status projection. It validates
    /// any staged Artifact Store outbox row in memory but never appends its
    /// Ledger event or finalizes the subordinate reference.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_read_only(
        ledger: PostgresTaskLedger,
        foreman: PostgresForeman,
        store_authority: StoreAuthorityHead,
        submission: TaskSubmissionEnvelope,
        managed_submission: TaskSpecSubmission,
        successor_identity: TaskLedgerStreamIdentity,
        expected_binding: VerifiedTaskExecutionBinding,
        foreman_generation: u64,
        foreman_checkpoint_digest: ContentDigest,
        policy_authority: ManagedPolicyAuthoritySource,
    ) -> ManagedPortResult<Self> {
        Self::new_with_recovery(
            ledger,
            foreman,
            store_authority,
            submission,
            managed_submission,
            successor_identity,
            Some(expected_binding),
            foreman_generation,
            foreman_checkpoint_digest,
            policy_authority,
            false,
        )
    }

    /// Builds one status-only repository whose exact binding is derived from
    /// the owner-verified Ledger and foreman rows in its single replay pass.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_status_read_only_unbound(
        ledger: PostgresTaskLedger,
        foreman: PostgresForeman,
        store_authority: StoreAuthorityHead,
        submission: TaskSubmissionEnvelope,
        managed_submission: TaskSpecSubmission,
        successor_identity: TaskLedgerStreamIdentity,
        foreman_generation: u64,
        foreman_checkpoint_digest: ContentDigest,
        policy_authority: ManagedPolicyAuthoritySource,
    ) -> ManagedPortResult<Self> {
        Self::new_with_recovery(
            ledger,
            foreman,
            store_authority,
            submission,
            managed_submission,
            successor_identity,
            None,
            foreman_generation,
            foreman_checkpoint_digest,
            policy_authority,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_recovery(
        ledger: PostgresTaskLedger,
        foreman: PostgresForeman,
        store_authority: StoreAuthorityHead,
        submission: TaskSubmissionEnvelope,
        managed_submission: TaskSpecSubmission,
        successor_identity: TaskLedgerStreamIdentity,
        expected_binding: Option<VerifiedTaskExecutionBinding>,
        foreman_generation: u64,
        foreman_checkpoint_digest: ContentDigest,
        policy_authority: ManagedPolicyAuthoritySource,
        recover_staged_artifact: bool,
    ) -> ManagedPortResult<Self> {
        if foreman_generation == 0 || is_zero(&foreman_checkpoint_digest) {
            return Err(known("LATTICE_MANAGED_FOREMAN_IDENTITY_REJECTED"));
        }
        let mut repository = Self {
            ledger,
            foreman,
            store_authority,
            submission,
            managed_submission,
            successor_identity,
            expected_binding,
            foreman_generation,
            foreman_checkpoint_digest,
            asserted_authority_digest: None,
            policy_authority,
            execution_environment: None,
            recover_staged_artifact_on_environment_install: recover_staged_artifact,
        };
        // Construction cannot recover a staged pending-attempt blocker before
        // its exact WSL2 descriptor has been installed. Keep the outbox row
        // untouched here; `with_execution_environment` performs the first
        // mutating recovery only after binding the configured descriptor.
        if recover_staged_artifact {
            let loaded = repository.load_runtime_unreconciled()?;
            repository.verify_approval_references(
                &loaded.stream,
                &loaded.binding,
                &loaded.references,
            )?;
        }
        Ok(repository)
    }

    /// Installs the already live-preflighted, typed execution descriptor used
    /// by every claim or exact replay through this repository instance.
    pub(crate) fn with_execution_environment(
        mut self,
        descriptor: Option<ExecutionEnvironmentDescriptor>,
    ) -> ManagedPortResult<Self> {
        if self.execution_environment.is_some() {
            return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION"));
        }
        self.execution_environment = descriptor;
        if self.recover_staged_artifact_on_environment_install {
            let loaded = self.load_runtime()?;
            self.verify_approval_references(&loaded.stream, &loaded.binding, &loaded.references)?;
            self.recover_staged_artifact_on_environment_install = false;
        }
        Ok(self)
    }

    /// Reloads `PostgreSQL` and returns only fully owner-verified managed-task
    /// state plus the extension's bounded replay evidence commitment.
    ///
    /// # Errors
    ///
    /// Any stale authority binding, missing child row/link, tampered evidence,
    /// or unavailable persistence fails closed.
    pub fn load_replay_projection(&mut self) -> ManagedPortResult<ManagedTaskReplayProjection> {
        let fresh = self.fresh_runtime()?;
        Ok(ManagedTaskReplayProjection {
            binding: fresh.binding,
            authority: fresh.authority,
            records: fresh.records,
            references: fresh.references,
            evidence: fresh.evidence,
            budget: fresh.budget,
            source: fresh.source,
            pending_attempt: fresh.pending_attempt,
            task_replay: fresh.task_replay,
        })
    }

    /// Returns the same durable projection without performing outbox repair.
    /// A staged row is owner-validated against its exact pure Ledger plan, but
    /// remains staged for the supervisor-owned recovery path.
    pub(crate) fn load_replay_projection_read_only(
        &mut self,
    ) -> ManagedPortResult<ManagedTaskReplayProjection> {
        self.load_status_projection_read_only()
    }

    /// Returns one coherent, fully owner-verified status snapshot. Unlike
    /// effect-capable construction, opening this replay-only repository does
    /// not pre-read the complete runtime and then repeat the same snapshot.
    pub(crate) fn load_status_projection_read_only(
        &mut self,
    ) -> ManagedPortResult<ManagedTaskReplayProjection> {
        let fresh = self.fresh_runtime_read_only()?;
        if self.expected_binding.is_none() {
            self.expected_binding = Some(fresh.binding.clone());
        }
        Ok(ManagedTaskReplayProjection {
            binding: fresh.binding,
            authority: fresh.authority,
            records: fresh.records,
            references: fresh.references,
            evidence: fresh.evidence,
            budget: fresh.budget,
            source: fresh.source,
            pending_attempt: fresh.pending_attempt,
            task_replay: fresh.task_replay,
        })
    }

    /// Revalidates the exact authority against an already verified status
    /// snapshot without replaying every managed runtime row a second time.
    pub(crate) fn assert_status_execution_authority_current(
        &mut self,
        projection: &ManagedTaskReplayProjection,
        binding: &VerifiedTaskExecutionBinding,
        authority_digest: &ContentDigest,
    ) -> ManagedPortResult<()> {
        if projection.binding() != binding {
            return Err(known("LATTICE_MANAGED_BINDING_NOT_CURRENT"));
        }
        let authority = projection
            .authority()
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"))?;
        if authority.authority_digest() != authority_digest
            || !authority_matches_binding(&authority, binding)
            || authority.capability() != ExecutionCapability::LocalReversibleTaskExecution
            || !projection
                .references()
                .approval_links()
                .iter()
                .any(|reference| {
                    reference.authority_digest() == authority_digest
                        && reference.link().payload_digest() == authority_digest
                })
            || !authority_is_current(authority)?
        {
            return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"));
        }
        self.policy_authority.reverify_status_projection(
            &self.submission,
            &self.managed_submission,
            binding,
            projection.source(),
            authority,
        )?;
        self.asserted_authority_digest = Some(authority_digest.clone());
        Ok(())
    }

    /// Fresh-loads and fully binds the exact durable `REVIEW_THREAD` claim.
    /// The returned timestamp is the sole reviewer discovery boundary; Git
    /// artifact times are not provider-effect authority.
    pub(crate) fn load_review_thread_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
        request: &ManagedVerificationRequest,
    ) -> ManagedPortResult<VerifiedReviewThreadDispatch> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        self.verify_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        if &loaded.binding != binding
            || !records.attempts().contains(attempt)
            || !records.observations().contains(terminal)
            || attempt.binding_digest() != binding.binding_digest()
            || terminal.binding_digest() != binding.binding_digest()
            || terminal.attempt_id() != attempt.attempt_id()
            || terminal.attempt_number() != attempt.attempt_number()
            || terminal.kind() != lattice_task_ledger::WorkerObservationKind::TerminalCompleted
        {
            return Err(known("LATTICE_MANAGED_REVIEW_DISPATCH_REPLAY_REJECTED"));
        }
        let subject = managed_review_dispatch_subject_digest(
            "REVIEW_THREAD",
            binding,
            attempt,
            terminal,
            request,
            None,
        )?;
        let retained = self
            .foreman
            .load_provider_dispatch_claim(
                binding.task_ref(),
                attempt.attempt_number(),
                ProviderDispatchKind::ReviewThread,
            )
            .map_err(map_foreman_read)?
            .ok_or_else(|| known("LATTICE_MANAGED_REVIEW_DISPATCH_REPLAY_REQUIRED"))?;
        if retained.kind() != ProviderDispatchKind::ReviewThread
            || retained.task_ref() != binding.task_ref()
            || u64::from(retained.attempt_number()) != attempt.attempt_number()
            || retained.attempt_id() != attempt.attempt_id()
            || retained.binding_digest() != binding.binding_digest()
            || retained.writer_fence() != attempt.writer_fence()
            || retained.foreman_generation() != attempt.foreman_generation()
            || retained.foreman_checkpoint_digest() != attempt.foreman_checkpoint_digest()
            || retained.anchor_digest() != terminal.payload_digest()
            || retained.supporting_digest() != request.evidence_artifact_digest()
            || retained.subject_digest() != &subject
            || is_zero(retained.dispatch_digest())
            || is_zero(retained.claim_receipt_digest())
            || OffsetDateTime::parse(retained.claimed_at(), &Rfc3339).is_err()
        {
            return Err(known("LATTICE_MANAGED_REVIEW_DISPATCH_REPLAY_REJECTED"));
        }
        Ok(VerifiedReviewThreadDispatch {
            claimed_at: retained.claimed_at().to_owned(),
        })
    }

    /// Persists and immediately replays one exact blocker-backed attempt closure.
    /// The subordinate row affects capacity only; it cannot create verification
    /// success or advance Task lifecycle state.
    pub(crate) fn record_attempt_closure(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        blocker_code: &str,
        blocker_descriptor_digest: &ContentDigest,
    ) -> ManagedPortResult<AttemptClosure> {
        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        let expected_binding = self
            .expected_binding
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_BINDING_NOT_CURRENT"))?;
        if attempt.task_ref() != expected_binding.task_ref() {
            return Err(known("LATTICE_MANAGED_ATTEMPT_CLOSURE_REJECTED"));
        }
        let disposition = self
            .foreman
            .record_attempt_closure(
                attempt.task_ref(),
                attempt_number,
                blocker_code,
                blocker_descriptor_digest,
                attempt.writer_fence(),
            )
            .map_err(map_foreman_write)?;
        if !matches!(
            disposition,
            AppendDisposition::Inserted | AppendDisposition::ExactReplay
        ) {
            return Err(known("LATTICE_MANAGED_ATTEMPT_CLOSURE_REJECTED"));
        }
        let closure = self
            .foreman
            .load_attempt_closure(attempt.task_ref(), attempt_number)
            .map_err(map_foreman_read)?
            .ok_or_else(|| known("LATTICE_MANAGED_ATTEMPT_CLOSURE_RECONCILE_REQUIRED"))?;
        if closure.blocker_code() != blocker_code
            || closure.blocker_descriptor_digest() != blocker_descriptor_digest
            || closure.reconciliation_proof_descriptor_digest().is_some()
            || closure.writer_fence() != attempt.writer_fence()
            || OffsetDateTime::parse(closure.closed_at(), &Rfc3339).is_err()
        {
            return Err(known("LATTICE_MANAGED_ATTEMPT_CLOSURE_RECONCILE_REQUIRED"));
        }
        Ok(closure)
    }

    /// Persists and immediately replays one retained blocker closure whose
    /// provider inactivity is proven by a second immutable evidence object.
    pub(crate) fn record_retained_attempt_closure(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
        blocker_code: &str,
        blocker_descriptor_digest: &ContentDigest,
        reconciliation_proof_descriptor_digest: &ContentDigest,
    ) -> ManagedPortResult<AttemptClosure> {
        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        let expected_binding = self
            .expected_binding
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_BINDING_NOT_CURRENT"))?;
        if attempt.task_ref() != expected_binding.task_ref()
            || blocker_descriptor_digest == reconciliation_proof_descriptor_digest
        {
            return Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_REJECTED"));
        }
        let disposition = self
            .foreman
            .close_retained_worker_without_provider_effect(
                attempt.task_ref(),
                attempt_number,
                blocker_code,
                blocker_descriptor_digest,
                reconciliation_proof_descriptor_digest,
                attempt.writer_fence(),
            )
            .map_err(map_foreman_write)?;
        if !matches!(
            disposition,
            AppendDisposition::Inserted | AppendDisposition::ExactReplay
        ) {
            return Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_REJECTED"));
        }
        let closure = self
            .foreman
            .load_attempt_closure(attempt.task_ref(), attempt_number)
            .map_err(map_foreman_read)?
            .ok_or_else(|| known("LATTICE_MANAGED_RETAINED_CLOSURE_RECONCILE_REQUIRED"))?;
        if closure.blocker_code() != blocker_code
            || closure.blocker_descriptor_digest() != blocker_descriptor_digest
            || closure.reconciliation_proof_descriptor_digest()
                != Some(reconciliation_proof_descriptor_digest)
            || closure.writer_fence() != attempt.writer_fence()
            || OffsetDateTime::parse(closure.closed_at(), &Rfc3339).is_err()
        {
            return Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_RECONCILE_REQUIRED"));
        }
        Ok(closure)
    }

    pub(crate) fn load_attempt_closure(
        &mut self,
        attempt: &VerifiedWorkerAttemptRecord,
    ) -> ManagedPortResult<Option<AttemptClosure>> {
        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        let closure = self
            .foreman
            .load_attempt_closure(attempt.task_ref(), attempt_number)
            .map_err(map_foreman_read)?;
        if closure.as_ref().is_some_and(|closure| {
            closure.writer_fence() != attempt.writer_fence()
                || OffsetDateTime::parse(closure.closed_at(), &Rfc3339).is_err()
        }) {
            return Err(known("LATTICE_MANAGED_ATTEMPT_CLOSURE_RECONCILE_REQUIRED"));
        }
        Ok(closure)
    }

    fn load_runtime_unreconciled(&mut self) -> ManagedPortResult<LoadedRuntime> {
        let intake = self
            .ledger
            .load_stream(self.submission.identity().clone())
            .map_err(map_ledger_read)?;
        let successor = self
            .ledger
            .load_stream(self.successor_identity.clone())
            .map_err(map_ledger_read)?;
        require_managed_successor(successor.stream())?;
        let rows = self
            .foreman
            .load_task_runtime_rows(self.submission.task_ref())
            .map_err(map_foreman_read)?;
        let binding = verify_untrusted_task_execution_binding(
            intake.stream(),
            successor.stream(),
            &self.submission,
            rows.binding(),
        )
        .map_err(map_domain)?;
        if self
            .expected_binding
            .as_ref()
            .is_some_and(|expected_binding| &binding != expected_binding)
        {
            return Err(known("LATTICE_MANAGED_BINDING_NOT_CURRENT"));
        }
        let references = self
            .foreman
            .load_reference_links(binding.task_ref())
            .map_err(map_foreman_read)?;
        let budget = self
            .foreman
            .load_worker_budget(binding.task_ref())
            .map_err(map_foreman_read)?;
        if pointer_content(budget.digest(), "budget")? != *binding.budget_digest() {
            return Err(known("LATTICE_MANAGED_BUDGET_BINDING_REJECTED"));
        }
        let source = self
            .foreman
            .load_task_promotion_source(binding.task_ref())
            .map_err(map_foreman_read)?
            .ok_or_else(|| reconcile("LATTICE_MANAGED_PROMOTION_SOURCE_RECONCILE_REQUIRED"))?;
        verify_promotion_source(
            &self.submission,
            &self.managed_submission,
            &self.successor_identity,
            &binding,
            &source,
        )?;
        let pending_attempt = self
            .foreman
            .load_pending_worker_attempt(binding.task_ref())
            .map_err(map_foreman_read)?;
        if pending_attempt.as_ref().is_some_and(|pending| {
            pending.max_attempts() != budget.max_attempts()
                || OffsetDateTime::parse(pending.reserved_at(), &Rfc3339).is_err()
        }) {
            return Err(known("LATTICE_MANAGED_PENDING_ATTEMPT_REPLAY_REJECTED"));
        }
        let execution_environments = self
            .foreman
            .load_execution_environments(binding.task_ref())
            .map_err(map_foreman_read)?;
        let staged_artifact = self
            .foreman
            .load_staged_artifact_reference(binding.task_ref())
            .map_err(map_foreman_read)?;
        let task_replay = self
            .foreman
            .read_task_replay(binding.task_ref())
            .map_err(map_foreman_read)?;
        if task_replay.task_ref() != binding.task_ref() || is_zero(task_replay.evidence_digest()) {
            return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
        }
        Ok(LoadedRuntime {
            stream: successor.stream().clone(),
            binding,
            rows,
            references,
            budget,
            source,
            pending_attempt,
            execution_environments,
            staged_artifact,
            task_replay,
        })
    }

    /// Completes at most the one database-bounded staged Artifact Store row
    /// before any full reference/replay verification. The staged row already
    /// binds the exact owner-planned Ledger request, so recovery cannot invent
    /// a new command, descriptor, head, or external effect.
    fn load_runtime(&mut self) -> ManagedPortResult<LoadedRuntime> {
        let loaded = self.load_runtime_unreconciled()?;
        if !self.recover_staged_artifact(&loaded)? {
            return Ok(loaded);
        }
        let recovered = self.load_runtime_unreconciled()?;
        if recovered.staged_artifact.is_some() {
            return Err(reconcile(
                "LATTICE_MANAGED_ARTIFACT_STAGE_RECONCILE_REQUIRED",
            ));
        }
        Ok(recovered)
    }

    fn ensure_pending_execution_environment_for_closure(
        &mut self,
        loaded: &LoadedRuntime,
        attempt: &VerifiedWorkerAttemptRecord,
    ) -> ManagedPortResult<()> {
        let pending = loaded
            .pending_attempt
            .as_ref()
            .filter(|pending| pending.row() == &attempt.to_untrusted())
            .ok_or_else(|| known("LATTICE_MANAGED_PENDING_ATTEMPT_REPLAY_REJECTED"))?;
        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        let durable = loaded
            .execution_environments
            .iter()
            .find(|candidate| candidate.attempt_number() == attempt_number);
        if durable.is_some_and(|candidate| {
            candidate.task_ref() != attempt.task_ref()
                || candidate.attempt_id() != attempt.attempt_id()
                || candidate.packet_digest() != attempt.packet_digest()
        }) {
            return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION"));
        }
        let configured = self.execution_environment.as_ref();
        let persistence = pending_execution_environment_persistence(
            pending.execution_environment_ref(),
            durable.map(|candidate| candidate.descriptor().environment_ref().as_str()),
            configured.map(|descriptor| descriptor.environment_ref().as_str()),
        )?;
        match persistence {
            PendingExecutionEnvironmentPersistence::NativeWindows => Ok(()),
            PendingExecutionEnvironmentPersistence::DurableExact => {
                if configured.is_some_and(|descriptor| {
                    durable.is_none_or(|candidate| candidate.descriptor() != descriptor)
                }) {
                    return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION"));
                }
                Ok(())
            }
            PendingExecutionEnvironmentPersistence::RecordConfigured => self
                .foreman
                .record_execution_environment(
                    attempt,
                    configured.ok_or_else(|| {
                        known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION")
                    })?,
                )
                .map(|_| ())
                .map_err(map_foreman_write),
        }
    }

    fn recover_staged_artifact(&mut self, loaded: &LoadedRuntime) -> ManagedPortResult<bool> {
        let Some(staged) = loaded.staged_artifact.as_ref() else {
            return Ok(false);
        };
        let records = Self::verify_loaded_runtime_records(loaded)?;
        let pending_attempt = Self::verified_pending_attempt(loaded, &records)?;
        self.verify_approval_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        let retained =
            self.verify_artifact_evidence(&loaded.stream, &loaded.binding, &loaded.references)?;
        let evidence = staged.evidence();
        let attempt = records
            .attempts()
            .iter()
            .find(|attempt| attempt.attempt_number() == u64::from(evidence.attempt()))
            .ok_or_else(|| known("LATTICE_MANAGED_ARTIFACT_STAGE_BINDING_REJECTED"))?;
        let pending_blocker = if pending_attempt.as_ref() == Some(attempt) {
            Some(pending_prestart_blocker_code(evidence, evidence.attempt())?)
        } else {
            None
        };
        if evidence.task_ref() != loaded.binding.task_ref()
            || evidence.project_id() != loaded.stream.identity().project_id()
            || staged.link().stream_id() != loaded.binding.successor_stream_id()
            || staged.link().expected_head().stream_id() != loaded.binding.successor_stream_id()
            || staged.link().payload_digest() != evidence.descriptor_digest()
            || retained.contains(evidence)
        {
            return Err(known("LATTICE_MANAGED_ARTIFACT_STAGE_BINDING_REJECTED"));
        }
        let metadata = TaskRuntimeAppendMetadata::new(
            staged.link().command_id().clone(),
            staged.correlation_id().clone(),
            staged.command_occurred_at(),
        )
        .map_err(map_domain)?;
        let existing_links = loaded
            .references
            .artifact_links()
            .iter()
            .map(|reference| reference.link().clone())
            .collect::<Vec<_>>();
        let plan = plan_artifact_reference_append(
            &loaded.stream,
            &loaded.binding,
            records.attempts(),
            &existing_links,
            metadata,
            attempt.attempt_number(),
            evidence.descriptor_digest().clone(),
        )
        .map_err(map_domain)?;
        if plan.link() != staged.link() {
            return Err(known("LATTICE_MANAGED_ARTIFACT_STAGE_LINK_REJECTED"));
        }
        Self::assert_artifact_quota(&loaded.references, &retained, evidence)?;
        if pending_blocker.is_some() {
            self.ensure_pending_execution_environment_for_closure(loaded, attempt)?;
        }
        self.execute_ledger(plan.ledger_plan())?;
        if let Some(blocker_code) = pending_blocker {
            self.foreman
                .close_pending_worker_attempt(
                    evidence.task_ref(),
                    evidence.attempt(),
                    blocker_code.as_str(),
                    evidence.descriptor_digest(),
                    attempt.writer_fence(),
                )
                .map_err(map_foreman_write)?;
        } else {
            self.foreman
                .finalize_staged_artifact_reference(
                    evidence.task_ref(),
                    evidence.attempt(),
                    evidence.descriptor_digest(),
                )
                .map_err(map_foreman_write)?;
        }
        Ok(true)
    }

    fn fresh_runtime(&mut self) -> ManagedPortResult<FreshRuntime> {
        // Each formal owner commits its own append before the corresponding
        // cross-owner row becomes visible. A reader can therefore straddle
        // that small commit window even though the final state is valid. The
        // load/recovery half is idempotent; retry only this closed replay error
        // and never repeat the caller's subsequent write.
        let mut last_skew = None;
        for pass in 0..CROSS_OWNER_SNAPSHOT_RETRY_LIMIT {
            match self.fresh_runtime_once() {
                Ok(runtime) => return Ok(runtime),
                Err(error) if cross_owner_snapshot_retry_allowed(pass, &error) => {
                    last_skew = Some(error);
                    std::thread::sleep(cross_owner_snapshot_retry_delay(pass));
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_skew.unwrap_or_else(|| known("LEDGER_INVALID_TASK_RUNTIME_RECORD")))
    }

    fn fresh_runtime_once(&mut self) -> ManagedPortResult<FreshRuntime> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        let evidence =
            self.verify_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        let provider_dispatches = self.load_provider_dispatches(&loaded.binding, &records)?;
        Self::verify_provider_dispatches(
            &loaded.binding,
            &records,
            &loaded.references,
            &evidence,
            pending_attempt.as_ref(),
            &loaded.execution_environments,
            &provider_dispatches,
        )?;
        Self::verify_task_replay(
            &loaded.binding,
            &records,
            &loaded.references,
            pending_attempt.as_ref(),
            &provider_dispatches,
            &loaded.task_replay,
        )?;
        Ok(FreshRuntime {
            binding: loaded.binding,
            authority: None,
            records,
            references: loaded.references,
            evidence,
            budget: loaded.budget,
            source: loaded.source,
            pending_attempt,
            task_replay: loaded.task_replay,
        })
    }

    fn fresh_runtime_read_only(&mut self) -> ManagedPortResult<FreshRuntime> {
        let mut last_skew = None;
        for pass in 0..CROSS_OWNER_SNAPSHOT_RETRY_LIMIT {
            match self.fresh_runtime_read_only_once() {
                Ok(runtime) => return Ok(runtime),
                Err(error) if cross_owner_snapshot_retry_allowed(pass, &error) => {
                    last_skew = Some(error);
                    std::thread::sleep(cross_owner_snapshot_retry_delay(pass));
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_skew.unwrap_or_else(|| known("LEDGER_INVALID_TASK_RUNTIME_RECORD")))
    }

    fn fresh_runtime_read_only_once(&mut self) -> ManagedPortResult<FreshRuntime> {
        let loaded = self.load_runtime_unreconciled()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        let authority =
            self.verified_approval_authority(&loaded.stream, &loaded.binding, &loaded.references)?;
        let evidence =
            self.verify_artifact_projection_read_only(&loaded, &records, pending_attempt.as_ref())?;
        let provider_dispatches = self.load_provider_dispatches(&loaded.binding, &records)?;
        Self::verify_provider_dispatches(
            &loaded.binding,
            &records,
            &loaded.references,
            &evidence,
            pending_attempt.as_ref(),
            &loaded.execution_environments,
            &provider_dispatches,
        )?;
        Self::verify_task_replay(
            &loaded.binding,
            &records,
            &loaded.references,
            pending_attempt.as_ref(),
            &provider_dispatches,
            &loaded.task_replay,
        )?;
        Ok(FreshRuntime {
            binding: loaded.binding,
            authority,
            records,
            references: loaded.references,
            evidence,
            budget: loaded.budget,
            source: loaded.source,
            pending_attempt,
            task_replay: loaded.task_replay,
        })
    }

    fn verify_artifact_projection_read_only(
        &mut self,
        loaded: &LoadedRuntime,
        records: &VerifiedTaskRuntimeRecords,
        pending_attempt: Option<&VerifiedWorkerAttemptRecord>,
    ) -> ManagedPortResult<Vec<VerifiedManagedEvidence>> {
        let retained =
            self.verify_artifact_evidence(&loaded.stream, &loaded.binding, &loaded.references)?;
        let Some(staged) = loaded.staged_artifact.as_ref() else {
            let links = loaded
                .references
                .artifact_links()
                .iter()
                .map(|reference| reference.link().clone())
                .collect::<Vec<_>>();
            verify_artifact_reference_links(&loaded.stream, &loaded.binding, &links)
                .map_err(map_domain)?;
            return Ok(retained);
        };
        let evidence = staged.evidence();
        let attempt = records
            .attempts()
            .iter()
            .find(|attempt| attempt.attempt_number() == u64::from(evidence.attempt()))
            .ok_or_else(|| known("LATTICE_MANAGED_ARTIFACT_STAGE_BINDING_REJECTED"))?;
        if pending_attempt == Some(attempt)
            || evidence.task_ref() != loaded.binding.task_ref()
            || evidence.project_id() != loaded.stream.identity().project_id()
            || staged.link().stream_id() != loaded.binding.successor_stream_id()
            || staged.link().expected_head().stream_id() != loaded.binding.successor_stream_id()
            || staged.link().payload_digest() != evidence.descriptor_digest()
            || retained.contains(evidence)
        {
            return Err(known("LATTICE_MANAGED_ARTIFACT_STAGE_BINDING_REJECTED"));
        }
        let metadata = TaskRuntimeAppendMetadata::new(
            staged.link().command_id().clone(),
            staged.correlation_id().clone(),
            staged.command_occurred_at(),
        )
        .map_err(map_domain)?;
        let mut links = loaded
            .references
            .artifact_links()
            .iter()
            .map(|reference| reference.link().clone())
            .collect::<Vec<_>>();
        let plan = plan_artifact_reference_append(
            &loaded.stream,
            &loaded.binding,
            records.attempts(),
            &links,
            metadata,
            attempt.attempt_number(),
            evidence.descriptor_digest().clone(),
        )
        .map_err(map_domain)?;
        if plan.link() != staged.link() {
            return Err(known("LATTICE_MANAGED_ARTIFACT_STAGE_LINK_REJECTED"));
        }
        if plan.ledger_plan().is_exact_retry() {
            if plan.new_link().is_none() {
                return Err(known("LATTICE_MANAGED_ARTIFACT_STAGE_BINDING_REJECTED"));
            }
            links.push(staged.link().clone());
        }
        verify_artifact_reference_links(&loaded.stream, &loaded.binding, &links)
            .map_err(map_domain)?;
        Self::assert_artifact_quota(&loaded.references, &retained, evidence)?;
        Ok(retained)
    }

    fn attempt_rows_with_pending(
        loaded: &LoadedRuntime,
    ) -> ManagedPortResult<Vec<UntrustedWorkerAttemptRow>> {
        let mut attempts = loaded.rows.attempts().to_vec();
        if let Some(pending) = loaded.pending_attempt.as_ref() {
            if pending.max_attempts() != loaded.budget.max_attempts()
                || attempts.contains(pending.row())
            {
                return Err(known("LATTICE_MANAGED_PENDING_ATTEMPT_REPLAY_REJECTED"));
            }
            attempts.push(pending.row().clone());
        }
        Ok(attempts)
    }

    fn verify_loaded_runtime_records(
        loaded: &LoadedRuntime,
    ) -> ManagedPortResult<VerifiedTaskRuntimeRecords> {
        let attempts = Self::attempt_rows_with_pending(loaded)?;
        verify_untrusted_task_runtime_records(
            &loaded.stream,
            &loaded.binding,
            &attempts,
            loaded.rows.observations(),
            loaded.rows.verifications(),
        )
        .map_err(map_domain)
    }

    fn verified_pending_attempt(
        loaded: &LoadedRuntime,
        records: &VerifiedTaskRuntimeRecords,
    ) -> ManagedPortResult<Option<VerifiedWorkerAttemptRecord>> {
        let Some(pending) = loaded.pending_attempt.as_ref() else {
            return Ok(None);
        };
        let mut matches = records
            .attempts()
            .iter()
            .filter(|attempt| attempt.to_untrusted() == *pending.row());
        let retained = matches
            .next()
            .cloned()
            .ok_or_else(|| known("LATTICE_MANAGED_PENDING_ATTEMPT_REPLAY_REJECTED"))?;
        if matches.next().is_some()
            || records
                .observations()
                .iter()
                .any(|observation| observation.attempt_number() == retained.attempt_number())
            || records
                .verifications()
                .iter()
                .any(|verification| verification.attempt_number() == retained.attempt_number())
            || loaded
                .references
                .artifact_links()
                .iter()
                .any(|reference| u64::from(reference.attempt_number()) == retained.attempt_number())
        {
            return Err(known("LATTICE_MANAGED_PENDING_ATTEMPT_REPLAY_REJECTED"));
        }
        Ok(Some(retained))
    }

    fn verify_references(
        &mut self,
        stream: &VerifiedStream,
        binding: &VerifiedTaskExecutionBinding,
        references: &PersistedReferenceLinks,
    ) -> ManagedPortResult<Vec<VerifiedManagedEvidence>> {
        self.verify_approval_references(stream, binding, references)?;
        self.verify_artifact_references(stream, binding, references)
    }

    fn verify_approval_references(
        &mut self,
        stream: &VerifiedStream,
        binding: &VerifiedTaskExecutionBinding,
        references: &PersistedReferenceLinks,
    ) -> ManagedPortResult<()> {
        self.verified_approval_authority(stream, binding, references)
            .map(|_| ())
    }

    fn verified_approval_authority(
        &mut self,
        stream: &VerifiedStream,
        binding: &VerifiedTaskExecutionBinding,
        references: &PersistedReferenceLinks,
    ) -> ManagedPortResult<Option<VerifiedExecutionAuthority>> {
        let approval_links = references
            .approval_links()
            .iter()
            .map(|reference| reference.link().clone())
            .collect::<Vec<_>>();
        verify_approval_evidence_links(stream, binding, &approval_links).map_err(map_domain)?;

        let authorities = self
            .foreman
            .load_execution_authorities(binding.task_ref())
            .map_err(map_foreman_read)?;
        let linked_authorities = references
            .approval_links()
            .iter()
            .map(|reference| reference.authority_digest().as_str())
            .collect::<BTreeSet<_>>();
        let retained_authorities = authorities
            .iter()
            .map(|authority| authority.authority_digest().as_str())
            .collect::<BTreeSet<_>>();
        if linked_authorities != retained_authorities
            || authorities
                .iter()
                .any(|authority| !authority_matches_binding(authority, binding))
        {
            return Err(known("LATTICE_MANAGED_APPROVAL_REFERENCE_REJECTED"));
        }
        match authorities.as_slice() {
            [] => Ok(None),
            [authority]
                if authority.capability() == ExecutionCapability::LocalReversibleTaskExecution
                    && references.approval_links().len() == 1 =>
            {
                Ok(Some(authority.clone()))
            }
            _ => Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_AMBIGUOUS")),
        }
    }

    fn verify_artifact_references(
        &mut self,
        stream: &VerifiedStream,
        binding: &VerifiedTaskExecutionBinding,
        references: &PersistedReferenceLinks,
    ) -> ManagedPortResult<Vec<VerifiedManagedEvidence>> {
        let artifact_links = references
            .artifact_links()
            .iter()
            .map(|reference| reference.link().clone())
            .collect::<Vec<_>>();
        verify_artifact_reference_links(stream, binding, &artifact_links).map_err(map_domain)?;
        self.verify_artifact_evidence(stream, binding, references)
    }

    fn verify_artifact_evidence(
        &mut self,
        stream: &VerifiedStream,
        binding: &VerifiedTaskExecutionBinding,
        references: &PersistedReferenceLinks,
    ) -> ManagedPortResult<Vec<VerifiedManagedEvidence>> {
        let mut by_attempt = BTreeMap::<u8, Vec<&ContentDigest>>::new();
        for reference in references.artifact_links() {
            by_attempt
                .entry(reference.attempt_number())
                .or_default()
                .push(reference.descriptor_digest());
        }
        let mut retained_by_attempt = BTreeMap::new();
        for (attempt, descriptors) in by_attempt {
            let evidence = self
                .foreman
                .load_managed_evidence(binding.task_ref(), attempt)
                .map_err(map_foreman_read)?;
            let retained = evidence
                .iter()
                .map(VerifiedManagedEvidence::descriptor_digest)
                .collect::<Vec<_>>();
            if retained.len() != descriptors.len()
                || descriptors
                    .iter()
                    .any(|descriptor| !retained.contains(descriptor))
                || evidence.iter().any(|value| {
                    value.task_ref() != binding.task_ref()
                        || value.attempt() != attempt
                        || value.project_id() != stream.identity().project_id()
                })
            {
                return Err(known("LATTICE_MANAGED_ARTIFACT_REFERENCE_REJECTED"));
            }
            retained_by_attempt.insert(attempt, evidence);
        }
        references
            .artifact_links()
            .iter()
            .map(|reference| {
                retained_by_attempt
                    .get(&reference.attempt_number())
                    .and_then(|values| {
                        values.iter().find(|value| {
                            value.descriptor_digest() == reference.descriptor_digest()
                        })
                    })
                    .cloned()
                    .ok_or_else(|| known("LATTICE_MANAGED_ARTIFACT_REFERENCE_REJECTED"))
            })
            .collect()
    }

    fn assert_artifact_quota(
        references: &PersistedReferenceLinks,
        retained: &[VerifiedManagedEvidence],
        candidate: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<()> {
        if references.artifact_links().iter().any(|reference| {
            reference.attempt_number() == candidate.attempt()
                && reference.descriptor_digest() == candidate.descriptor_digest()
        }) {
            return Ok(());
        }
        let attempt_count = retained
            .iter()
            .filter(|evidence| evidence.attempt() == candidate.attempt())
            .count();
        let attempt_bytes = retained
            .iter()
            .filter(|evidence| evidence.attempt() == candidate.attempt())
            .try_fold(0usize, |total, evidence| {
                total.checked_add(evidence.bytes().len())
            })
            .ok_or_else(|| known("FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED"))?;
        let task_bytes = retained
            .iter()
            .try_fold(0usize, |total, evidence| {
                total.checked_add(evidence.bytes().len())
            })
            .ok_or_else(|| known("FOREMAN_ARTIFACT_TASK_QUOTA_EXHAUSTED"))?;
        let candidate_bytes = candidate.bytes().len();
        let attempt_total = attempt_bytes
            .checked_add(candidate_bytes)
            .and_then(|value| u64::try_from(value).ok());
        if attempt_count >= usize::from(MAX_ARTIFACTS_PER_ATTEMPT)
            || attempt_total.is_none_or(|value| value > MAX_ARTIFACT_BYTES_PER_ATTEMPT)
        {
            return Err(known("FOREMAN_ARTIFACT_ATTEMPT_QUOTA_EXHAUSTED"));
        }
        let task_total = task_bytes
            .checked_add(candidate_bytes)
            .and_then(|value| u64::try_from(value).ok());
        if retained.len() >= usize::from(MAX_ARTIFACTS_PER_TASK)
            || task_total.is_none_or(|value| value > MAX_ARTIFACT_BYTES_PER_TASK)
        {
            return Err(known("FOREMAN_ARTIFACT_TASK_QUOTA_EXHAUSTED"));
        }
        Ok(())
    }

    fn load_provider_dispatches(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        records: &VerifiedTaskRuntimeRecords,
    ) -> ManagedPortResult<Vec<ProviderDispatchClaim>> {
        let mut claims = Vec::new();
        for attempt in records.attempts() {
            for kind in PROVIDER_DISPATCH_KINDS {
                if let Some(claim) = self
                    .foreman
                    .load_provider_dispatch_claim(
                        binding.task_ref(),
                        attempt.attempt_number(),
                        kind,
                    )
                    .map_err(map_foreman_read)?
                {
                    claims.push(claim);
                }
            }
        }
        Ok(claims)
    }

    fn verify_provider_dispatches(
        binding: &VerifiedTaskExecutionBinding,
        records: &VerifiedTaskRuntimeRecords,
        references: &PersistedReferenceLinks,
        evidence: &[VerifiedManagedEvidence],
        pending_attempt: Option<&VerifiedWorkerAttemptRecord>,
        execution_environments: &[PersistedExecutionEnvironment],
        claims: &[ProviderDispatchClaim],
    ) -> ManagedPortResult<()> {
        let mut environments_by_attempt = BTreeMap::new();
        for persisted in execution_environments {
            let attempt = records
                .attempts()
                .iter()
                .find(|attempt| attempt.attempt_number() == u64::from(persisted.attempt_number()))
                .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))?;
            if persisted.task_ref() != binding.task_ref()
                || persisted.attempt_id() != attempt.attempt_id()
                || persisted.packet_digest() != attempt.packet_digest()
                || environments_by_attempt
                    .insert(attempt.attempt_number(), persisted.descriptor())
                    .is_some()
            {
                return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
            }
        }
        let mut by_attempt = BTreeMap::<u64, BTreeMap<&'static str, &ProviderDispatchClaim>>::new();
        for claim in claims {
            let attempt = records
                .attempts()
                .iter()
                .find(|attempt| attempt.attempt_number() == u64::from(claim.attempt_number()))
                .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))?;
            if pending_attempt == Some(attempt)
                || claim.task_ref() != binding.task_ref()
                || claim.attempt_id() != attempt.attempt_id()
                || claim.binding_digest() != binding.binding_digest()
                || claim.writer_fence() != attempt.writer_fence()
                || claim.foreman_generation() != attempt.foreman_generation()
                || claim.foreman_checkpoint_digest() != attempt.foreman_checkpoint_digest()
                || is_zero(claim.anchor_digest())
                || is_zero(claim.supporting_digest())
                || is_zero(claim.subject_digest())
                || is_zero(claim.dispatch_digest())
                || is_zero(claim.claim_receipt_digest())
                || OffsetDateTime::parse(claim.claimed_at(), &Rfc3339).is_err()
            {
                return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
            }
            let per_attempt = by_attempt.entry(attempt.attempt_number()).or_default();
            if per_attempt.insert(claim.kind().as_str(), claim).is_some() {
                return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
            }
        }

        for attempt in records.attempts() {
            let per_attempt = by_attempt.get(&attempt.attempt_number());
            let worker_thread = per_attempt
                .and_then(|claims| claims.get(ProviderDispatchKind::WorkerThread.as_str()));
            let worker_turn = per_attempt
                .and_then(|claims| claims.get(ProviderDispatchKind::WorkerTurn.as_str()));
            let review_thread = per_attempt
                .and_then(|claims| claims.get(ProviderDispatchKind::ReviewThread.as_str()));
            let review_turn = per_attempt
                .and_then(|claims| claims.get(ProviderDispatchKind::ReviewTurn.as_str()));

            let attempt_observations = records
                .observations()
                .iter()
                .filter(|observation| observation.attempt_number() == attempt.attempt_number())
                .collect::<Vec<_>>();
            let worker_thread_observed = !attempt_observations.is_empty();
            let worker_turn_observed = attempt_observations.iter().any(|observation| {
                observation.kind() != lattice_task_ledger::WorkerObservationKind::ThreadAccepted
            });
            let mut review_thread_observed = false;
            let mut review_turn_observed = false;
            for item in evidence.iter().filter(|item| {
                item.task_ref() == binding.task_ref()
                    && u64::from(item.attempt()) == attempt.attempt_number()
                    && item.kind() == ManagedEvidenceKind::WorkerLifecycle
                    && item.payload_schema() == "lattice.managed-review-lifecycle/1.0"
            }) {
                let identity = parse_review_lifecycle_identity(item)
                    .map_err(|_| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))?;
                review_thread_observed = true;
                review_turn_observed |= review_lifecycle_implies_turn_dispatch(&identity);
            }
            verify_provider_dispatch_presence(
                ProviderDispatchProgress {
                    worker_thread_observed,
                    worker_turn_observed,
                    review_thread_observed,
                    review_turn_observed,
                },
                ProviderDispatchPresence {
                    worker_thread: worker_thread.is_some(),
                    worker_turn: worker_turn.is_some(),
                    review_thread: review_thread.is_some(),
                    review_turn: review_turn.is_some(),
                },
            )?;

            if let Some(claim) = worker_thread {
                let subject = managed_dispatch_subject_digest(
                    "WORKER_THREAD",
                    binding,
                    attempt,
                    &[attempt.payload_digest(), attempt.packet_digest()],
                )?;
                if claim.anchor_digest() != attempt.payload_digest()
                    || claim.supporting_digest() != attempt.packet_digest()
                    || claim.subject_digest() != &subject
                {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                }
            }

            if let Some(claim) = worker_turn {
                let matching_threads = records
                    .observations()
                    .iter()
                    .filter(|observation| {
                        observation.attempt_number() == attempt.attempt_number()
                            && observation.kind()
                                == lattice_task_ledger::WorkerObservationKind::ThreadAccepted
                            && observation.turn_id().is_none()
                    })
                    .collect::<Vec<_>>();
                let [thread] = matching_threads.as_slice() else {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                };
                let subject = managed_dispatch_subject_digest(
                    "WORKER_TURN",
                    binding,
                    attempt,
                    &[thread.payload_digest(), thread.evidence_digest()],
                )?;
                if worker_thread.is_none()
                    || claim.anchor_digest() != thread.payload_digest()
                    || claim.supporting_digest() != thread.evidence_digest()
                    || claim.subject_digest() != &subject
                {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                }
            }

            if let Some(claim) = review_thread {
                let matching_terminals = records
                    .observations()
                    .iter()
                    .filter(|observation| {
                        observation.attempt_number() == attempt.attempt_number()
                            && observation.kind()
                                == lattice_task_ledger::WorkerObservationKind::TerminalCompleted
                    })
                    .collect::<Vec<_>>();
                let [terminal] = matching_terminals.as_slice() else {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                };
                let supporting_is_linked = references.artifact_links().iter().any(|reference| {
                    u64::from(reference.attempt_number()) == attempt.attempt_number()
                        && reference.descriptor_digest() == claim.supporting_digest()
                });
                let matching_snapshots = evidence
                    .iter()
                    .filter(|item| item.descriptor_digest() == claim.supporting_digest())
                    .collect::<Vec<_>>();
                let [snapshot] = matching_snapshots.as_slice() else {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                };
                let request = managed_verification_request_from_snapshot(
                    binding,
                    attempt,
                    terminal,
                    snapshot,
                    environments_by_attempt
                        .get(&attempt.attempt_number())
                        .copied(),
                )?;
                let subject = managed_review_dispatch_subject_digest(
                    "REVIEW_THREAD",
                    binding,
                    attempt,
                    terminal,
                    &request,
                    None,
                )?;
                if worker_thread.is_none()
                    || worker_turn.is_none()
                    || claim.anchor_digest() != terminal.payload_digest()
                    || !supporting_is_linked
                    || claim.subject_digest() != &subject
                {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                }
            }

            if let Some(claim) = review_turn {
                let matching_anchors = evidence
                    .iter()
                    .filter(|item| {
                        u64::from(item.attempt()) == attempt.attempt_number()
                            && item.descriptor_digest() == claim.anchor_digest()
                    })
                    .filter_map(|item| {
                        parse_review_lifecycle_identity(item)
                            .ok()
                            .filter(|identity| identity.event_type == "THREAD_START_ACCEPTED")
                            .map(|_| item)
                    })
                    .collect::<Vec<_>>();
                let [anchor] = matching_anchors.as_slice() else {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                };
                let Some(review_thread) = review_thread else {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                };
                let matching_terminals = records
                    .observations()
                    .iter()
                    .filter(|observation| {
                        observation.attempt_number() == attempt.attempt_number()
                            && observation.kind()
                                == lattice_task_ledger::WorkerObservationKind::TerminalCompleted
                    })
                    .collect::<Vec<_>>();
                let [terminal] = matching_terminals.as_slice() else {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                };
                let matching_snapshots = evidence
                    .iter()
                    .filter(|item| item.descriptor_digest() == review_thread.supporting_digest())
                    .collect::<Vec<_>>();
                let [snapshot] = matching_snapshots.as_slice() else {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                };
                let request = managed_verification_request_from_snapshot(
                    binding,
                    attempt,
                    terminal,
                    snapshot,
                    environments_by_attempt
                        .get(&attempt.attempt_number())
                        .copied(),
                )?;
                let subject = managed_review_dispatch_subject_digest(
                    "REVIEW_TURN",
                    binding,
                    attempt,
                    terminal,
                    &request,
                    Some(anchor.descriptor_digest()),
                )?;
                if anchor.descriptor_digest() != claim.anchor_digest()
                    || claim.supporting_digest() != review_thread.supporting_digest()
                    || claim.subject_digest() != &subject
                {
                    return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
                }
            }

            let ordered = [worker_thread, worker_turn, review_thread, review_turn]
                .into_iter()
                .flatten()
                .map(|claim| {
                    OffsetDateTime::parse(claim.claimed_at(), &Rfc3339)
                        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))
                })
                .collect::<ManagedPortResult<Vec<_>>>()?;
            if ordered.windows(2).any(|pair| pair[0] > pair[1]) {
                return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
            }
        }
        Ok(())
    }

    fn verify_task_replay(
        binding: &VerifiedTaskExecutionBinding,
        records: &VerifiedTaskRuntimeRecords,
        references: &PersistedReferenceLinks,
        pending_attempt: Option<&VerifiedWorkerAttemptRecord>,
        provider_dispatches: &[ProviderDispatchClaim],
        replay: &TaskReplay,
    ) -> ManagedPortResult<()> {
        if replay.task_ref() != binding.task_ref()
            || replay
                .records()
                .iter()
                .any(|record| OffsetDateTime::parse(record.recorded_at(), &Rfc3339).is_err())
        {
            return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
        }
        let mut expected = vec![ReplayRecordIdentity::from_link(
            "TASK_PROMOTION",
            ReplayRecordState::Retained,
            None,
            1,
            binding.binding_digest(),
            binding.link(),
        )];
        for attempt in records.attempts() {
            expected.push(ReplayRecordIdentity::from_link(
                "WORKER_ATTEMPT",
                if pending_attempt == Some(attempt) {
                    ReplayRecordState::PendingClaim
                } else {
                    ReplayRecordState::Retained
                },
                Some(
                    u8::try_from(attempt.attempt_number())
                        .map_err(|_| known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"))?,
                ),
                1,
                attempt.payload_digest(),
                attempt.link(),
            ));
        }
        let mut per_attempt = BTreeMap::<u64, u64>::new();
        for observation in records.observations() {
            let ordinal = per_attempt.entry(observation.attempt_number()).or_default();
            *ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"))?;
            expected.push(ReplayRecordIdentity::from_link(
                "WORKER_OBSERVATION",
                ReplayRecordState::Retained,
                Some(
                    u8::try_from(observation.attempt_number())
                        .map_err(|_| known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"))?,
                ),
                *ordinal,
                observation.payload_digest(),
                observation.link(),
            ));
        }
        for verification in records.verifications() {
            expected.push(ReplayRecordIdentity::from_link(
                "VERIFICATION",
                ReplayRecordState::Retained,
                Some(
                    u8::try_from(verification.attempt_number())
                        .map_err(|_| known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"))?,
                ),
                1,
                verification.payload_digest(),
                verification.link(),
            ));
        }
        for reference in references.artifact_links() {
            expected.push(ReplayRecordIdentity::from_link(
                "ARTIFACT_REFERENCE",
                ReplayRecordState::Retained,
                Some(reference.attempt_number()),
                reference.link().event_sequence(),
                reference.descriptor_digest(),
                reference.link(),
            ));
        }
        for reference in references.approval_links() {
            expected.push(ReplayRecordIdentity::from_link(
                "APPROVAL_EVIDENCE",
                ReplayRecordState::Retained,
                None,
                1,
                reference.authority_digest(),
                reference.link(),
            ));
        }
        for claim in provider_dispatches {
            let attempt = records
                .attempts()
                .iter()
                .find(|attempt| attempt.attempt_number() == u64::from(claim.attempt_number()))
                .ok_or_else(|| known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"))?;
            expected.push(ReplayRecordIdentity::from_link(
                provider_replay_kind(claim.kind()),
                ReplayRecordState::Retained,
                Some(claim.attempt_number()),
                provider_replay_ordinal(claim.kind()),
                claim.claim_receipt_digest(),
                attempt.link(),
            ));
        }
        expected.sort_by_key(ReplayRecordIdentity::canonical_key);
        let actual = replay
            .records()
            .iter()
            .map(ReplayRecordIdentity::from_record)
            .collect::<Vec<_>>();
        verify_replay_identities(&actual, &expected)
    }

    fn append_metadata(
        stream: &VerifiedStream,
        command_id: CommandId,
    ) -> ManagedPortResult<TaskRuntimeAppendMetadata> {
        let occurred_at = stream
            .commands()
            .iter()
            .find(|record| record.request().command_id() == &command_id)
            .map_or_else(now_utc, |record| {
                Ok(record.request().occurred_at().to_owned())
            })?;
        TaskRuntimeAppendMetadata::new(
            command_id,
            CorrelationId::new(CORRELATION_ID).map_err(map_domain)?,
            occurred_at,
        )
        .map_err(map_domain)
    }

    fn execute_ledger(
        &mut self,
        plan: &lattice_task_ledger::LedgerAppendPlan,
    ) -> ManagedPortResult<()> {
        self.ledger
            .execute(
                plan.command_record().request().clone(),
                self.store_authority.clone(),
            )
            .map(|_| ())
            .map_err(map_ledger_write)
    }

    fn validate_packet(
        binding: &VerifiedTaskExecutionBinding,
        packet: &AttemptPacketIdentity,
        authority_digest: &ContentDigest,
        budget: &WorkerBudget,
    ) -> ManagedPortResult<()> {
        if packet.task_ref() != binding.task_ref().as_str()
            || pointer_content(packet.project_ref(), "project")?
                != *binding.project_authority_receipt_digest()
            || pointer_content(packet.spec_ref(), "spec")? != *binding.task_spec_digest()
            || pointer_content(packet.approval_ref(), "approval")?
                != *binding.approval_subject_digest()
            || pointer_content(packet.budget_digest(), "budget")? != *binding.budget_digest()
            || pointer_content(packet.verification_ref(), "verification")?
                != *binding.verification_policy_digest()
            || packet.deadline_at() != budget.deadline_at()
            || !budget.allows_attempt(packet.attempt())
            || is_zero(authority_digest)
        {
            return Err(known("LATTICE_MANAGED_ATTEMPT_PACKET_REJECTED"));
        }
        Ok(())
    }

    fn plan_attempt_reservation(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<(WorkerAttemptAppendPlan, u8)> {
        let loaded = self.load_runtime()?;
        if &loaded.binding != binding {
            return Err(known("LATTICE_MANAGED_BINDING_NOT_CURRENT"));
        }
        let retained_evidence =
            self.verify_references(&loaded.stream, binding, &loaded.references)?;
        let authority_digest = self
            .asserted_authority_digest
            .as_ref()
            .ok_or_else(|| known("LATTICE_MANAGED_EXECUTION_AUTHORITY_ASSERTION_REQUIRED"))?;
        let authority = self
            .foreman
            .load_execution_authority(binding.task_ref(), authority_digest)
            .map_err(map_foreman_read)?;
        if !authority_matches_binding(&authority, binding)
            || authority.capability() != ExecutionCapability::LocalReversibleTaskExecution
            || !authority_is_current(&authority)?
        {
            return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"));
        }
        self.policy_authority.reverify(
            &self.submission,
            &self.managed_submission,
            &loaded,
            &authority,
        )?;
        Self::validate_packet(
            binding,
            packet,
            authority.authority_digest(),
            &loaded.budget,
        )?;
        let retained_pending = if loaded.pending_attempt.is_some() {
            let records = Self::verify_loaded_runtime_records(&loaded)?;
            Self::verified_pending_attempt(&loaded, &records)?
        } else {
            None
        };
        let packet_digest = pointer_content(packet.digest(), "attempt-packet")?;
        let command_id = operation_command_id("attempt", packet.attempt(), &packet_digest)?;
        let metadata = Self::append_metadata(&loaded.stream, command_id)?;
        let input = WorkerAttemptInput::new(
            managed_attempt_id(binding.task_ref(), packet.attempt())?,
            u64::from(packet.attempt()),
            retained_pending.as_ref().map_or(
                self.foreman_generation,
                VerifiedWorkerAttemptRecord::foreman_generation,
            ),
            packet.model_selection().model(),
            packet.model_selection().reasoning(),
            packet.model_selection().reason(),
            packet.writer_fence(),
            retained_pending.as_ref().map_or_else(
                || self.foreman_checkpoint_digest.clone(),
                |pending| pending.foreman_checkpoint_digest().clone(),
            ),
            authority.authority_digest().clone(),
            packet_digest.clone(),
            pointer_content(packet.worktree_ref(), "worktree")?,
            sha256_bytes(packet.base_commit().as_bytes())?,
            pointer_content(packet.model_selection().digest(), "model-selection")?,
        )
        .map_err(map_domain)?;
        let mut attempt_rows = Self::attempt_rows_with_pending(&loaded)?;
        if let Some(recovered) =
            recover_worker_attempt_record(&loaded.stream, binding, &metadata, &input)
                .map_err(map_domain)?
        {
            let recovered = recovered.to_untrusted();
            if !attempt_rows.contains(&recovered) {
                attempt_rows.push(recovered);
            }
        }
        let records = verify_untrusted_task_runtime_records(
            &loaded.stream,
            binding,
            &attempt_rows,
            loaded.rows.observations(),
            loaded.rows.verifications(),
        )
        .map_err(map_domain)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        let predecessor = if packet.attempt() > 1 {
            let predecessor_number = u64::from(packet.attempt() - 1);
            let candidates = records
                .attempts()
                .iter()
                .filter(|attempt| attempt.attempt_number() == predecessor_number)
                .collect::<Vec<_>>();
            let [predecessor] = candidates.as_slice() else {
                return Err(reconcile(
                    "LATTICE_MANAGED_RETRY_PREDECESSOR_RECONCILE_REQUIRED",
                ));
            };
            let terminals = records
                .observations()
                .iter()
                .filter(|observation| {
                    observation.attempt_number() == predecessor_number
                        && observation.kind().is_terminal()
                })
                .collect::<Vec<_>>();
            if let [terminal] = terminals.as_slice() {
                validate_terminal_repair_successor(predecessor, terminal, packet)?;
                None
            } else if terminals.is_empty() {
                let closure = self
                    .foreman
                    .load_attempt_closure(binding.task_ref(), packet.attempt() - 1)
                    .map_err(map_foreman_read)?
                    .ok_or_else(|| {
                        reconcile("LATTICE_MANAGED_RETRY_PREDECESSOR_RECONCILE_REQUIRED")
                    })?;
                Some(verified_no_provider_effect_predecessor(
                    binding,
                    predecessor,
                    &closure,
                    &retained_evidence,
                    packet,
                    &packet_digest,
                )?)
            } else {
                return Err(reconcile(
                    "LATTICE_MANAGED_RETRY_PREDECESSOR_RECONCILE_REQUIRED",
                ));
            }
        } else {
            None
        };
        let plan = if let Some(predecessor) = predecessor.as_ref() {
            plan_worker_attempt_append_with_no_provider_effect_predecessor(
                &loaded.stream,
                binding,
                records.attempts(),
                records.observations(),
                predecessor,
                metadata,
                input,
            )
        } else {
            plan_worker_attempt_append(
                &loaded.stream,
                binding,
                records.attempts(),
                records.observations(),
                metadata,
                input,
            )
        }
        .map_err(map_domain)?;
        if pending_attempt
            .as_ref()
            .is_some_and(|pending| pending != plan.record())
        {
            return Err(known("LATTICE_MANAGED_PENDING_ATTEMPT_SUBSTITUTION"));
        }
        Ok((plan, loaded.budget.max_attempts()))
    }

    /// Re-evaluates the persisted task-bound execution authority immediately
    /// before any new provider-effect claim. An already retained exact claim
    /// is historical reconciliation evidence and is deliberately replayable
    /// after expiry; the SQL owner independently validates every immutable
    /// field before returning `EXACT_REPLAY`.
    fn reverify_new_provider_dispatch_authority(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        kind: ProviderDispatchKind,
    ) -> ManagedPortResult<()> {
        if attempt.task_ref() != binding.task_ref()
            || attempt.binding_digest() != binding.binding_digest()
        {
            return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"));
        }
        if self
            .foreman
            .load_provider_dispatch_claim(binding.task_ref(), attempt.attempt_number(), kind)
            .map_err(map_foreman_read)?
            .is_some()
        {
            return Ok(());
        }
        self.assert_execution_authority_current(binding, attempt.approval_receipt_digest())
    }

    /// Durably records the exact Ledger-linked attempt packet without claiming
    /// capacity or authorizing any provider effect. Exact retries return the
    /// same pending record after a fresh PostgreSQL replay.
    pub(crate) fn reserve_attempt(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<VerifiedWorkerAttemptRecord> {
        let (plan, maximum_attempts) = self.plan_attempt_reservation(binding, packet)?;
        self.execute_ledger(plan.ledger_plan())?;
        let disposition = self
            .foreman
            .reserve_worker_attempt_with_execution_environment_ref(
                plan.record(),
                maximum_attempts,
                packet.execution_environment_ref(),
            )
            .map_err(map_foreman_write)?;
        let pending = self
            .foreman
            .load_pending_worker_attempt(binding.task_ref())
            .map_err(map_foreman_read)?;
        if pending.as_ref().is_some_and(|candidate| {
            candidate.execution_environment_ref() != packet.execution_environment_ref()
        }) {
            return Err(known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION"));
        }
        let replay = self.fresh_runtime()?;
        let pending_matches = replay
            .pending_attempt
            .as_ref()
            .map(|candidate| candidate == plan.record());
        let active_matches = replay
            .records
            .attempts()
            .iter()
            .filter(|candidate| {
                replay.pending_attempt.as_ref() != Some(*candidate) && *candidate == plan.record()
            })
            .count();
        match attempt_reservation_replay_source(disposition, pending_matches, active_matches)? {
            AttemptReservationReplaySource::Pending => replay
                .pending_attempt
                .ok_or_else(|| reconcile("LATTICE_MANAGED_ATTEMPT_RESERVATION_RECONCILE_REQUIRED")),
            AttemptReservationReplaySource::Active => replay
                .records
                .attempts()
                .iter()
                .find(|candidate| {
                    replay.pending_attempt.as_ref() != Some(*candidate)
                        && *candidate == plan.record()
                })
                .cloned()
                .ok_or_else(|| reconcile("LATTICE_MANAGED_ATTEMPT_RESERVATION_RECONCILE_REQUIRED")),
        }
    }

    /// Serializes the durable-evidence predicate and one existing Artifact
    /// outbox append against terminal, verification, and attempt-closure
    /// writers. The session guard deliberately spans the outbox's independent
    /// commits without folding them into one transaction.
    pub(crate) fn record_restart_writer_blocker_atomically(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        evidence: &VerifiedManagedEvidence,
        blocker_code: &str,
    ) -> ManagedPortResult<RestartWriterBlockerRecordDisposition> {
        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        self.foreman
            .begin_restart_writer_blocker_guard(binding.task_ref(), attempt_number)
            .map_err(map_foreman_write)?;
        let guarded = (|| {
            let projection = self.load_replay_projection()?;
            if projection.binding() != binding
                || projection.records().attempts().last() != Some(attempt)
                || evidence.task_ref() != binding.task_ref()
                || evidence.attempt() != attempt_number
                || evidence.kind() != ManagedEvidenceKind::WorkerLifecycle
                || evidence.payload_schema() != "lattice.managed-blocker.v1"
            {
                return Err(known("LATTICE_MANAGED_WRITER_BLOCKER_REPLAY_REJECTED"));
            }
            let closure = self.load_attempt_closure(attempt)?;
            let verification = projection
                .records()
                .verifications()
                .iter()
                .any(|record| record.attempt_number() == attempt.attempt_number());
            let terminal_for_attempt = projection.records().observations().iter().any(|record| {
                record.attempt_number() == attempt.attempt_number() && record.kind().is_terminal()
            });
            if closure.is_some() || verification || terminal_for_attempt {
                return Ok(RestartWriterBlockerRecordDisposition::DurableEvidenceReady);
            }
            let retained = projection
                .evidence()
                .iter()
                .filter(|candidate| {
                    candidate.attempt() == attempt_number
                        && candidate.kind() == ManagedEvidenceKind::WorkerLifecycle
                        && candidate.payload_schema() == "lattice.managed-blocker.v1"
                })
                .collect::<Vec<_>>();
            match retained.as_slice() {
                [] => {}
                [candidate]
                    if serde_json::from_slice::<Value>(candidate.bytes())
                        .ok()
                        .and_then(|payload| {
                            payload
                                .get("code")
                                .and_then(Value::as_str)
                                .map(str::to_owned)
                        })
                        .as_deref()
                        == Some(blocker_code) =>
                {
                    return Ok(RestartWriterBlockerRecordDisposition::ExactReplay);
                }
                _ => {
                    return Err(known("LATTICE_MANAGED_WRITER_BLOCKER_REPLAY_REJECTED"));
                }
            }
            let receipt = self.record_artifact(binding, attempt, evidence)?;
            if !receipt.matches(evidence) {
                return Err(reconcile("LATTICE_MANAGED_WRITER_BLOCKER_REPLAY_REJECTED"));
            }
            Ok(RestartWriterBlockerRecordDisposition::Persisted)
        })();
        let released = self
            .foreman
            .end_restart_writer_blocker_guard(binding.task_ref(), attempt_number)
            .map_err(map_foreman_write);
        match (guarded, released) {
            (Ok(disposition), Ok(())) => Ok(disposition),
            (Err(failure), Ok(())) => Err(failure),
            (_, Err(_)) => Err(reconcile(
                "LATTICE_MANAGED_WRITER_BLOCKER_GUARD_RECONCILE_REQUIRED",
            )),
        }
    }

    /// Test-only crash injector for the two durable Artifact Store outbox
    /// windows. It executes the exact production validation, append planner,
    /// and stage call used by `record_artifact`, then deliberately stops either
    /// before the Ledger append or before finalization. A fresh concrete
    /// repository must recover the retained row through `load_runtime`.
    #[cfg(test)]
    pub(crate) fn inject_artifact_crash_window_for_test(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        evidence: &VerifiedManagedEvidence,
        append_ledger: bool,
    ) -> ManagedPortResult<lattice_task_ledger::TaskRuntimeEventLink> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        let _pending_blocker = if pending_attempt.as_ref() == Some(attempt) {
            Some(pending_prestart_blocker_code(evidence, attempt_number)?)
        } else {
            None
        };
        if &loaded.binding != binding
            || !records.attempts().contains(attempt)
            || evidence.task_ref() != binding.task_ref()
            || evidence.attempt() != attempt_number
            || evidence.project_id() != loaded.stream.identity().project_id()
        {
            return Err(known("LATTICE_MANAGED_ARTIFACT_BINDING_REJECTED"));
        }
        self.verify_approval_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        let retained_evidence =
            self.verify_artifact_evidence(&loaded.stream, &loaded.binding, &loaded.references)?;
        let command_id =
            operation_command_id("artifact", attempt_number, evidence.descriptor_digest())?;
        let metadata = Self::append_metadata(&loaded.stream, command_id)?;
        let existing_links = loaded
            .references
            .artifact_links()
            .iter()
            .map(|reference| reference.link().clone())
            .collect::<Vec<_>>();
        let plan = plan_artifact_reference_append(
            &loaded.stream,
            binding,
            records.attempts(),
            &existing_links,
            metadata,
            attempt.attempt_number(),
            evidence.descriptor_digest().clone(),
        )
        .map_err(map_domain)?;
        Self::assert_artifact_quota(&loaded.references, &retained_evidence, evidence)?;
        let correlation_id = CorrelationId::new(CORRELATION_ID).map_err(map_domain)?;
        let command_occurred_at = plan
            .ledger_plan()
            .command_record()
            .request()
            .occurred_at()
            .to_owned();
        self.foreman
            .stage_artifact_reference(evidence, plan.link(), &correlation_id, &command_occurred_at)
            .map_err(map_foreman_write)?;
        if append_ledger {
            self.execute_ledger(plan.ledger_plan())?;
        }
        Ok(plan.link().clone())
    }
}

impl ManagedForemanRepositoryPort for PostgresManagedForemanRepository {
    fn assert_execution_authority_current(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        authority_digest: &ContentDigest,
    ) -> ManagedPortResult<()> {
        self.asserted_authority_digest = None;
        let loaded = self.load_runtime()?;
        if &loaded.binding != binding {
            return Err(known("LATTICE_MANAGED_BINDING_NOT_CURRENT"));
        }
        self.verify_approval_references(&loaded.stream, binding, &loaded.references)?;
        let authority = self
            .foreman
            .load_execution_authority(binding.task_ref(), authority_digest)
            .map_err(map_foreman_read)?;
        if authority.authority_digest() != authority_digest
            || !authority_matches_binding(&authority, binding)
            || authority.capability() != ExecutionCapability::LocalReversibleTaskExecution
            || !loaded.references.approval_links().iter().any(|reference| {
                reference.authority_digest() == authority_digest
                    && reference.link().payload_digest() == authority_digest
            })
            || !authority_is_current(&authority)?
        {
            return Err(known("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT"));
        }
        self.policy_authority.reverify(
            &self.submission,
            &self.managed_submission,
            &loaded,
            &authority,
        )?;
        self.asserted_authority_digest = Some(authority_digest.clone());
        Ok(())
    }

    fn claim_attempt(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        packet: &AttemptPacketIdentity,
    ) -> ManagedPortResult<ManagedAttemptClaim> {
        let reserved = self.reserve_attempt(binding, packet)?;
        let execution_environment = self.execution_environment.clone();
        let persistence_steps = attempt_claim_persistence_steps(
            packet,
            execution_environment
                .as_ref()
                .map(|descriptor| descriptor.environment_ref().as_str()),
        )?;
        let mut claim = None;
        for step in persistence_steps {
            match step {
                AttemptClaimPersistenceStep::RecordExecutionEnvironment => {
                    let descriptor = execution_environment.as_ref().ok_or_else(|| {
                        known("LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION")
                    })?;
                    self.foreman
                        .record_execution_environment(&reserved, descriptor)
                        .map_err(map_foreman_write)?;
                }
                AttemptClaimPersistenceStep::ClaimCapacity => {
                    let maximum_attempts = self.load_runtime()?.budget.max_attempts();
                    claim = Some(
                        self.foreman
                            .claim_worker_attempt_with_execution_environment_ref(
                                &reserved,
                                maximum_attempts,
                                packet.execution_environment_ref(),
                            )
                            .map_err(map_foreman_write)?,
                    );
                }
            }
        }
        let claim = claim.ok_or_else(|| reconcile("LATTICE_MANAGED_ATTEMPT_RECONCILE_REQUIRED"))?;
        let disposition = match claim.disposition() {
            ClaimDisposition::Claimed => ManagedAttemptClaimDisposition::Claimed,
            ClaimDisposition::ExactReplay => ManagedAttemptClaimDisposition::ExactReplay,
        };

        let replay = self.fresh_runtime()?;
        if replay.pending_attempt.is_some() {
            return Err(reconcile("LATTICE_MANAGED_ATTEMPT_RECONCILE_REQUIRED"));
        }
        let attempt = replay
            .records
            .attempts()
            .iter()
            .find(|record| record.payload_digest() == reserved.payload_digest())
            .cloned()
            .ok_or_else(|| reconcile("LATTICE_MANAGED_ATTEMPT_RECONCILE_REQUIRED"))?;
        Ok(ManagedAttemptClaim::new(attempt, disposition))
    }

    fn record_observation(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        observation: &ManagedWorkerObservation,
    ) -> ManagedPortResult<VerifiedWorkerObservationRecord> {
        let loaded = self.load_runtime()?;
        if &loaded.binding != binding {
            return Err(known("LATTICE_MANAGED_OBSERVATION_BINDING_REJECTED"));
        }
        let attempt_rows = Self::attempt_rows_with_pending(&loaded)?;
        let current_records = verify_untrusted_task_runtime_records(
            &loaded.stream,
            binding,
            &attempt_rows,
            loaded.rows.observations(),
            loaded.rows.verifications(),
        )
        .map_err(map_domain)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &current_records)?;
        if !current_records.attempts().contains(attempt)
            || pending_attempt.as_ref() == Some(attempt)
            || observation.ledger_input().attempt_number() != attempt.attempt_number()
        {
            return Err(known("LATTICE_MANAGED_OBSERVATION_BINDING_REJECTED"));
        }
        let command_id = operation_command_id(
            observation.kind().as_str(),
            u8::try_from(attempt.attempt_number())
                .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?,
            observation.evidence_digest(),
        )?;
        let metadata = Self::append_metadata(&loaded.stream, command_id)?;
        let mut observation_rows = loaded.rows.observations().to_vec();
        if let Some(recovered) = recover_worker_observation_record(
            &loaded.stream,
            binding,
            current_records.attempts(),
            &metadata,
            observation.ledger_input(),
        )
        .map_err(map_domain)?
        {
            let recovered = recovered.to_untrusted();
            if !observation_rows.contains(&recovered) {
                observation_rows.push(recovered);
            }
        }
        let records = verify_untrusted_task_runtime_records(
            &loaded.stream,
            binding,
            &attempt_rows,
            &observation_rows,
            loaded.rows.verifications(),
        )
        .map_err(map_domain)?;
        self.verify_references(&loaded.stream, binding, &loaded.references)?;
        let plan = plan_worker_observation_append(
            &loaded.stream,
            binding,
            records.attempts(),
            records.observations(),
            metadata,
            observation.ledger_input().clone(),
        )
        .map_err(map_domain)?;
        self.execute_ledger(plan.ledger_plan())?;
        self.foreman
            .record_worker_observation(plan.record())
            .map_err(map_foreman_write)?;
        let replay = self.fresh_runtime()?;
        replay
            .records
            .observations()
            .iter()
            .find(|record| record.payload_digest() == plan.record().payload_digest())
            .cloned()
            .ok_or_else(|| reconcile("LATTICE_MANAGED_OBSERVATION_RECONCILE_REQUIRED"))
    }

    fn claim_worker_thread_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
    ) -> ManagedPortResult<ManagedWorkerThreadDispatchDisposition> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        self.verify_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        if &loaded.binding != binding
            || pending_attempt.as_ref() == Some(attempt)
            || !records.attempts().contains(attempt)
            || attempt.task_ref() != binding.task_ref()
            || attempt.binding_digest() != binding.binding_digest()
        {
            return Err(known("LATTICE_MANAGED_WORKER_THREAD_DISPATCH_REJECTED"));
        }
        let subject = managed_dispatch_subject_digest(
            "WORKER_THREAD",
            binding,
            attempt,
            &[attempt.payload_digest(), attempt.packet_digest()],
        )?;
        self.reverify_new_provider_dispatch_authority(
            binding,
            attempt,
            ProviderDispatchKind::WorkerThread,
        )?;
        match self
            .foreman
            .claim_provider_dispatch(
                attempt,
                ProviderDispatchKind::WorkerThread,
                attempt.payload_digest(),
                attempt.packet_digest(),
                &subject,
            )
            .map_err(map_foreman_write)?
        {
            ClaimDisposition::Claimed => Ok(ManagedWorkerThreadDispatchDisposition::Claimed),
            ClaimDisposition::ExactReplay => {
                Ok(ManagedWorkerThreadDispatchDisposition::ExactReplay)
            }
        }
    }

    fn claim_worker_turn_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        thread: &VerifiedWorkerObservationRecord,
    ) -> ManagedPortResult<ManagedWorkerTurnDispatchDisposition> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        self.verify_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        if &loaded.binding != binding
            || pending_attempt.as_ref() == Some(attempt)
            || !records.attempts().contains(attempt)
            || !records.observations().contains(thread)
            || thread.task_ref() != binding.task_ref()
            || thread.attempt_number() != attempt.attempt_number()
            || thread.attempt_id() != attempt.attempt_id()
            || thread.binding_digest() != binding.binding_digest()
            || thread.kind() != lattice_task_ledger::WorkerObservationKind::ThreadAccepted
            || thread.turn_id().is_some()
        {
            return Err(known("LATTICE_MANAGED_WORKER_TURN_DISPATCH_REJECTED"));
        }
        let subject = managed_dispatch_subject_digest(
            "WORKER_TURN",
            binding,
            attempt,
            &[thread.payload_digest(), thread.evidence_digest()],
        )?;
        self.reverify_new_provider_dispatch_authority(
            binding,
            attempt,
            ProviderDispatchKind::WorkerTurn,
        )?;
        match self
            .foreman
            .claim_provider_dispatch(
                attempt,
                ProviderDispatchKind::WorkerTurn,
                thread.payload_digest(),
                thread.evidence_digest(),
                &subject,
            )
            .map_err(map_foreman_write)?
        {
            ClaimDisposition::Claimed => Ok(ManagedWorkerTurnDispatchDisposition::Claimed),
            ClaimDisposition::ExactReplay => Ok(ManagedWorkerTurnDispatchDisposition::ExactReplay),
        }
    }

    fn load_worker_dispatch_state(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
    ) -> ManagedPortResult<ManagedWorkerDispatchState> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        self.verify_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        if &loaded.binding != binding
            || attempt.task_ref() != binding.task_ref()
            || attempt.binding_digest() != binding.binding_digest()
            || (!records.attempts().contains(attempt) && pending_attempt.as_ref() != Some(attempt))
        {
            return Err(known("LATTICE_MANAGED_WORKER_DISPATCH_STATE_REJECTED"));
        }
        let thread_claim = self
            .foreman
            .load_provider_dispatch_claim(
                binding.task_ref(),
                attempt.attempt_number(),
                ProviderDispatchKind::WorkerThread,
            )
            .map_err(map_foreman_read)?;
        let turn_claim = self
            .foreman
            .load_provider_dispatch_claim(
                binding.task_ref(),
                attempt.attempt_number(),
                ProviderDispatchKind::WorkerTurn,
            )
            .map_err(map_foreman_read)?;

        let validate_claim = |claim: &lattice_postgres_foreman::ProviderDispatchClaim,
                              kind: ProviderDispatchKind,
                              anchor: &ContentDigest,
                              supporting: &ContentDigest,
                              subject: &ContentDigest| {
            claim.kind() == kind
                && claim.task_ref() == binding.task_ref()
                && u64::from(claim.attempt_number()) == attempt.attempt_number()
                && claim.attempt_id() == attempt.attempt_id()
                && claim.binding_digest() == binding.binding_digest()
                && claim.writer_fence() == attempt.writer_fence()
                && claim.foreman_generation() == attempt.foreman_generation()
                && claim.foreman_checkpoint_digest() == attempt.foreman_checkpoint_digest()
                && claim.anchor_digest() == anchor
                && claim.supporting_digest() == supporting
                && claim.subject_digest() == subject
                && !is_zero(claim.dispatch_digest())
                && !is_zero(claim.claim_receipt_digest())
                && OffsetDateTime::parse(claim.claimed_at(), &Rfc3339).is_ok()
        };

        if let Some(thread_claim) = thread_claim.as_ref() {
            let subject = managed_dispatch_subject_digest(
                "WORKER_THREAD",
                binding,
                attempt,
                &[attempt.payload_digest(), attempt.packet_digest()],
            )?;
            if !validate_claim(
                thread_claim,
                ProviderDispatchKind::WorkerThread,
                attempt.payload_digest(),
                attempt.packet_digest(),
                &subject,
            ) {
                return Err(known("LATTICE_MANAGED_WORKER_DISPATCH_STATE_REJECTED"));
            }
        }

        if let Some(turn_claim) = turn_claim.as_ref() {
            let matching_threads = records
                .observations()
                .iter()
                .filter(|observation| {
                    observation.attempt_number() == attempt.attempt_number()
                        && observation.kind()
                            == lattice_task_ledger::WorkerObservationKind::ThreadAccepted
                        && observation.turn_id().is_none()
                })
                .collect::<Vec<_>>();
            let [thread] = matching_threads.as_slice() else {
                return Err(known("LATTICE_MANAGED_WORKER_DISPATCH_STATE_REJECTED"));
            };
            let subject = managed_dispatch_subject_digest(
                "WORKER_TURN",
                binding,
                attempt,
                &[thread.payload_digest(), thread.evidence_digest()],
            )?;
            if thread_claim.is_none()
                || !validate_claim(
                    turn_claim,
                    ProviderDispatchKind::WorkerTurn,
                    thread.payload_digest(),
                    thread.evidence_digest(),
                    &subject,
                )
            {
                return Err(known("LATTICE_MANAGED_WORKER_DISPATCH_STATE_REJECTED"));
            }
        }

        Ok(match (thread_claim.is_some(), turn_claim.is_some()) {
            (false, false) => ManagedWorkerDispatchState::NoWorkerThread,
            (true, false) => ManagedWorkerDispatchState::WorkerThreadClaimed,
            (true, true) => ManagedWorkerDispatchState::WorkerTurnClaimed,
            (false, true) => {
                return Err(known("LATTICE_MANAGED_WORKER_DISPATCH_STATE_REJECTED"));
            }
        })
    }

    fn close_prestart_without_provider_effect(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        proof: &ManagedPrestartNoEffectProof,
        blocker_code: &'static str,
    ) -> ManagedPortResult<ManagedPrestartClosureDisposition> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        let retained_evidence =
            self.verify_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        if &loaded.binding != binding
            || attempt.task_ref() != binding.task_ref()
            || attempt.binding_digest() != binding.binding_digest()
            || (!records.attempts().contains(attempt) && pending_attempt.as_ref() != Some(attempt))
        {
            return Err(known("LATTICE_MANAGED_PRESTART_CLOSURE_REJECTED"));
        }
        let dispatch_state = self.load_worker_dispatch_state(binding, attempt)?;
        let attempt_observations = records
            .observations()
            .iter()
            .filter(|observation| observation.attempt_number() == attempt.attempt_number())
            .collect::<Vec<_>>();
        let retained_blocker_shape = retained_prestart_closure_blocker_shape(blocker_code).ok();
        match proof {
            ManagedPrestartNoEffectProof::PendingReservation => {
                if retained_blocker_shape.is_some()
                    || pending_attempt.as_ref() != Some(attempt)
                    || dispatch_state != ManagedWorkerDispatchState::NoWorkerThread
                    || !attempt_observations.is_empty()
                {
                    return Err(reconcile(
                        "LATTICE_MANAGED_PRESTART_CLOSURE_RECONCILE_REQUIRED",
                    ));
                }
            }
            ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
                worker_thread_claimed,
            } => {
                if *worker_thread_claimed
                    || pending_attempt.as_ref() == Some(attempt)
                    || !records.attempts().contains(attempt)
                    || dispatch_state != ManagedWorkerDispatchState::NoWorkerThread
                    || !attempt_observations.is_empty()
                {
                    return Err(reconcile(
                        "LATTICE_MANAGED_PRESTART_CLOSURE_RECONCILE_REQUIRED",
                    ));
                }
            }
            ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn {
                thread,
                worker_turn_claimed,
            } => {
                let expected_dispatch = if *worker_turn_claimed {
                    ManagedWorkerDispatchState::WorkerTurnClaimed
                } else {
                    ManagedWorkerDispatchState::WorkerThreadClaimed
                };
                if pending_attempt.as_ref() == Some(attempt)
                    || dispatch_state != expected_dispatch
                    || attempt_observations.as_slice() != [thread.as_ref()]
                    || thread.kind() != lattice_task_ledger::WorkerObservationKind::ThreadAccepted
                    || thread.turn_id().is_some()
                    || (*worker_turn_claimed && retained_blocker_shape.is_none())
                {
                    return Err(reconcile(
                        "LATTICE_MANAGED_PRESTART_CLOSURE_RECONCILE_REQUIRED",
                    ));
                }
            }
        }

        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        if let Some((reason, retryable)) = retained_blocker_shape {
            let blocker_bytes = serde_json::to_vec(&serde_json::json!({
                "schema": "lattice.managed-blocker.v1",
                "attempt": attempt_number,
                "code": blocker_code,
                "reason": reason,
                "retryable": retryable,
            }))
            .map_err(|_| known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED"))?;
            let blockers = retained_evidence
                .iter()
                .filter(|evidence| {
                    evidence.attempt() == attempt_number
                        && evidence.kind() == ManagedEvidenceKind::WorkerLifecycle
                        && evidence.payload_schema() == "lattice.managed-blocker.v1"
                })
                .collect::<Vec<_>>();
            let [blocker] = blockers.as_slice() else {
                return Err(reconcile(
                    "LATTICE_MANAGED_RETAINED_CLOSURE_RECONCILE_REQUIRED",
                ));
            };
            if blocker.media_type() != "application/json"
                || blocker.producer_id() != "lattice-foreman"
                || blocker.producer_version() != "1"
                || blocker.producer_digest() != attempt.foreman_checkpoint_digest()
                || blocker.bytes() != blocker_bytes
            {
                return Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_BLOCKER_REJECTED"));
            }

            let proof_bytes = retained_no_effect_proof_bytes(
                binding.task_ref(),
                attempt_number,
                blocker.descriptor_digest(),
                proof,
            )?;
            let proof_matches = retained_evidence
                .iter()
                .filter(|evidence| {
                    evidence.attempt() == attempt_number
                        && evidence.kind() == ManagedEvidenceKind::WorkerLifecycle
                        && evidence.payload_schema()
                            == "lattice.managed-no-provider-effect-proof.v1"
                })
                .collect::<Vec<_>>();
            let proof_evidence = match proof_matches.as_slice() {
                [] => VerifiedManagedEvidence::new(
                    ManagedEvidenceInput::new(
                        self.submission.identity().project_id().clone(),
                        binding.task_ref().clone(),
                        attempt_number,
                        ManagedEvidenceKind::WorkerLifecycle,
                        "application/json",
                        "lattice.managed-no-provider-effect-proof.v1",
                        "lattice-foreman",
                        "1",
                        attempt.foreman_checkpoint_digest().clone(),
                        match proof {
                            ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn {
                                thread, ..
                            } => thread.observed_at().to_owned(),
                            ManagedPrestartNoEffectProof::ProvenNoProviderCandidate { .. } => {
                                now_utc()?
                            }
                            ManagedPrestartNoEffectProof::PendingReservation => {
                                return Err(known(
                                    "LATTICE_MANAGED_RETAINED_CLOSURE_PROOF_REJECTED",
                                ));
                            }
                        },
                        proof_bytes,
                    )
                    .map_err(|_| known("LATTICE_MANAGED_RETAINED_CLOSURE_PROOF_REJECTED"))?,
                )
                .map_err(|_| known("LATTICE_MANAGED_RETAINED_CLOSURE_PROOF_REJECTED"))?,
                [evidence]
                    if evidence.media_type() == "application/json"
                        && evidence.producer_id() == "lattice-foreman"
                        && evidence.producer_version() == "1"
                        && evidence.producer_digest() == attempt.foreman_checkpoint_digest()
                        && evidence.bytes() == proof_bytes =>
                {
                    (*evidence).clone()
                }
                [_] => {
                    return Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_PROOF_REJECTED"));
                }
                _ => {
                    return Err(reconcile(
                        "LATTICE_MANAGED_RETAINED_CLOSURE_RECONCILE_REQUIRED",
                    ));
                }
            };
            let replayed = self.load_attempt_closure(attempt)?.is_some_and(|closure| {
                closure.blocker_code() == blocker_code
                    && closure.blocker_descriptor_digest() == blocker.descriptor_digest()
                    && closure.reconciliation_proof_descriptor_digest()
                        == Some(proof_evidence.descriptor_digest())
                    && closure.writer_fence() == attempt.writer_fence()
            });
            let receipt = self.record_artifact(binding, attempt, &proof_evidence)?;
            if !receipt.matches(&proof_evidence) {
                return Err(reconcile(
                    "LATTICE_MANAGED_RETAINED_CLOSURE_RECONCILE_REQUIRED",
                ));
            }
            let closure = self.record_retained_attempt_closure(
                attempt,
                blocker_code,
                blocker.descriptor_digest(),
                proof_evidence.descriptor_digest(),
            )?;
            if closure.blocker_code() != blocker_code
                || closure.blocker_descriptor_digest() != blocker.descriptor_digest()
                || closure.reconciliation_proof_descriptor_digest()
                    != Some(proof_evidence.descriptor_digest())
                || closure.writer_fence() != attempt.writer_fence()
            {
                return Err(reconcile(
                    "LATTICE_MANAGED_RETAINED_CLOSURE_RECONCILE_REQUIRED",
                ));
            }
            return Ok(if replayed {
                ManagedPrestartClosureDisposition::ExactReplay
            } else {
                ManagedPrestartClosureDisposition::Closed
            });
        }

        let (reason, retryable) = prestart_closure_blocker_shape(blocker_code)?;
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema": "lattice.managed-blocker.v1",
            "attempt": attempt_number,
            "code": blocker_code,
            "reason": reason,
            "retryable": retryable,
        }))
        .map_err(|_| known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED"))?;
        let matching_blockers = retained_evidence
            .iter()
            .filter(|evidence| {
                evidence.attempt() == attempt_number
                    && evidence.kind() == ManagedEvidenceKind::WorkerLifecycle
                    && evidence.payload_schema() == "lattice.managed-blocker.v1"
            })
            .collect::<Vec<_>>();
        let evidence = match matching_blockers.as_slice() {
            [] => {
                let created_at = match proof {
                    ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn { thread, .. } => {
                        thread.observed_at().to_owned()
                    }
                    ManagedPrestartNoEffectProof::PendingReservation
                    | ManagedPrestartNoEffectProof::ProvenNoProviderCandidate { .. } => now_utc()?,
                };
                VerifiedManagedEvidence::new(
                    ManagedEvidenceInput::new(
                        self.submission.identity().project_id().clone(),
                        binding.task_ref().clone(),
                        attempt_number,
                        ManagedEvidenceKind::WorkerLifecycle,
                        "application/json",
                        "lattice.managed-blocker.v1",
                        "lattice-foreman",
                        "1",
                        attempt.foreman_checkpoint_digest().clone(),
                        created_at,
                        bytes,
                    )
                    .map_err(|_| known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED"))?,
                )
                .map_err(|_| known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED"))?
            }
            [evidence]
                if evidence.media_type() == "application/json"
                    && evidence.producer_id() == "lattice-foreman"
                    && evidence.producer_version() == "1"
                    && evidence.producer_digest() == attempt.foreman_checkpoint_digest()
                    && evidence.bytes() == bytes =>
            {
                (*evidence).clone()
            }
            [_] => return Err(known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED")),
            _ => {
                return Err(reconcile(
                    "LATTICE_MANAGED_PRESTART_CLOSURE_RECONCILE_REQUIRED",
                ));
            }
        };
        let replayed = self.load_attempt_closure(attempt)?.is_some_and(|closure| {
            closure.blocker_code() == blocker_code
                && closure.blocker_descriptor_digest() == evidence.descriptor_digest()
                && closure.reconciliation_proof_descriptor_digest().is_none()
                && closure.writer_fence() == attempt.writer_fence()
        });
        let receipt = self.record_artifact(binding, attempt, &evidence)?;
        if !receipt.matches(&evidence) {
            return Err(reconcile(
                "LATTICE_MANAGED_PRESTART_CLOSURE_RECONCILE_REQUIRED",
            ));
        }
        let closure =
            self.record_attempt_closure(attempt, blocker_code, evidence.descriptor_digest())?;
        if closure.blocker_code() != blocker_code
            || closure.blocker_descriptor_digest() != evidence.descriptor_digest()
            || closure.reconciliation_proof_descriptor_digest().is_some()
            || closure.writer_fence() != attempt.writer_fence()
        {
            return Err(reconcile(
                "LATTICE_MANAGED_PRESTART_CLOSURE_RECONCILE_REQUIRED",
            ));
        }
        Ok(if replayed {
            ManagedPrestartClosureDisposition::ExactReplay
        } else {
            ManagedPrestartClosureDisposition::Closed
        })
    }

    fn claim_review_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        terminal: &VerifiedWorkerObservationRecord,
        request: &ManagedVerificationRequest,
    ) -> ManagedPortResult<ManagedReviewDispatchDisposition> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        self.verify_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        if &loaded.binding != binding
            || pending_attempt.as_ref() == Some(attempt)
            || !records.attempts().contains(attempt)
            || !records.observations().contains(terminal)
            || terminal.task_ref() != binding.task_ref()
            || terminal.attempt_number() != attempt.attempt_number()
            || terminal.attempt_id() != attempt.attempt_id()
            || terminal.binding_digest() != binding.binding_digest()
            || terminal.kind() != lattice_task_ledger::WorkerObservationKind::TerminalCompleted
            || request.profile_identity() != binding.verification_policy_digest()
            || request.base_commit_digest() != attempt.base_commit_digest()
            || request.worker_evidence_digest() != terminal.evidence_digest()
            || !loaded.references.artifact_links().iter().any(|reference| {
                u64::from(reference.attempt_number()) == attempt.attempt_number()
                    && reference.descriptor_digest() == request.evidence_artifact_digest()
            })
        {
            return Err(known("LATTICE_MANAGED_REVIEW_DISPATCH_REJECTED"));
        }
        let subject = managed_review_dispatch_subject_digest(
            "REVIEW_THREAD",
            binding,
            attempt,
            terminal,
            request,
            None,
        )?;
        self.reverify_new_provider_dispatch_authority(
            binding,
            attempt,
            ProviderDispatchKind::ReviewThread,
        )?;
        match self
            .foreman
            .claim_provider_dispatch(
                attempt,
                ProviderDispatchKind::ReviewThread,
                terminal.payload_digest(),
                request.evidence_artifact_digest(),
                &subject,
            )
            .map_err(map_foreman_write)?
        {
            ClaimDisposition::Claimed => Ok(ManagedReviewDispatchDisposition::Claimed),
            ClaimDisposition::ExactReplay => Ok(ManagedReviewDispatchDisposition::ExactReplay),
        }
    }

    fn claim_review_turn_dispatch(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        request: &ManagedVerificationRequest,
        thread_lifecycle: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedReviewDispatchDisposition> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        let retained_evidence =
            self.verify_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        let terminal = records
            .observations()
            .iter()
            .find(|observation| {
                observation.attempt_number() == attempt.attempt_number()
                    && observation.kind()
                        == lattice_task_ledger::WorkerObservationKind::TerminalCompleted
            })
            .ok_or_else(|| known("LATTICE_MANAGED_REVIEW_TURN_DISPATCH_REJECTED"))?;
        if &loaded.binding != binding
            || pending_attempt.as_ref() == Some(attempt)
            || !records.attempts().contains(attempt)
            || request.profile_identity() != binding.verification_policy_digest()
            || request.base_commit_digest() != attempt.base_commit_digest()
            || request.worker_evidence_digest() != terminal.evidence_digest()
            || thread_lifecycle.task_ref() != binding.task_ref()
            || u64::from(thread_lifecycle.attempt()) != attempt.attempt_number()
            || thread_lifecycle.kind()
                != lattice_artifact_store::ManagedEvidenceKind::WorkerLifecycle
            || thread_lifecycle.payload_schema() != "lattice.managed-review-lifecycle/1.0"
            || !retained_evidence.contains(thread_lifecycle)
            || !loaded.references.artifact_links().iter().any(|reference| {
                u64::from(reference.attempt_number()) == attempt.attempt_number()
                    && reference.descriptor_digest() == request.evidence_artifact_digest()
            })
        {
            return Err(known("LATTICE_MANAGED_REVIEW_TURN_DISPATCH_REJECTED"));
        }
        let original_thread_anchor =
            select_review_turn_anchor(&retained_evidence, thread_lifecycle)?;
        let subject = managed_review_dispatch_subject_digest(
            "REVIEW_TURN",
            binding,
            attempt,
            terminal,
            request,
            Some(original_thread_anchor.descriptor_digest()),
        )?;
        self.reverify_new_provider_dispatch_authority(
            binding,
            attempt,
            ProviderDispatchKind::ReviewTurn,
        )?;
        match self
            .foreman
            .claim_provider_dispatch(
                attempt,
                ProviderDispatchKind::ReviewTurn,
                original_thread_anchor.descriptor_digest(),
                request.evidence_artifact_digest(),
                &subject,
            )
            .map_err(map_foreman_write)?
        {
            ClaimDisposition::Claimed => Ok(ManagedReviewDispatchDisposition::Claimed),
            ClaimDisposition::ExactReplay => Ok(ManagedReviewDispatchDisposition::ExactReplay),
        }
    }

    fn record_artifact(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        evidence: &VerifiedManagedEvidence,
    ) -> ManagedPortResult<ManagedArtifactReceipt> {
        let loaded = self.load_runtime()?;
        let records = Self::verify_loaded_runtime_records(&loaded)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &records)?;
        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        let pending_blocker = if pending_attempt.as_ref() == Some(attempt) {
            Some(pending_prestart_blocker_code(evidence, attempt_number)?)
        } else {
            None
        };
        if &loaded.binding != binding
            || !records.attempts().contains(attempt)
            || evidence.task_ref() != binding.task_ref()
            || evidence.attempt() != attempt_number
            || evidence.project_id() != loaded.stream.identity().project_id()
        {
            return Err(known("LATTICE_MANAGED_ARTIFACT_BINDING_REJECTED"));
        }
        // Approval links remain closed independently. Artifact links are
        // intentionally reconciled by the exact append plan below before a
        // complete reference replay: the Ledger event is committed first, so
        // a process crash may leave that one exact event without its Artifact
        // Store row. Rejecting the partial shape here would make the durable
        // outbox impossible to finish on restart.
        self.verify_approval_references(&loaded.stream, &loaded.binding, &loaded.references)?;
        let retained_evidence =
            self.verify_artifact_evidence(&loaded.stream, &loaded.binding, &loaded.references)?;
        let command_id =
            operation_command_id("artifact", attempt_number, evidence.descriptor_digest())?;
        let metadata = Self::append_metadata(&loaded.stream, command_id)?;
        let existing_links = loaded
            .references
            .artifact_links()
            .iter()
            .map(|reference| reference.link().clone())
            .collect::<Vec<_>>();
        let plan = plan_artifact_reference_append(
            &loaded.stream,
            binding,
            records.attempts(),
            &existing_links,
            metadata,
            attempt.attempt_number(),
            evidence.descriptor_digest().clone(),
        )
        .map_err(map_domain)?;
        Self::assert_artifact_quota(&loaded.references, &retained_evidence, evidence)?;
        if pending_blocker.is_some() {
            self.ensure_pending_execution_environment_for_closure(&loaded, attempt)?;
        }
        let correlation_id = CorrelationId::new(CORRELATION_ID).map_err(map_domain)?;
        let command_occurred_at = plan
            .ledger_plan()
            .command_record()
            .request()
            .occurred_at()
            .to_owned();
        self.foreman
            .stage_artifact_reference(evidence, plan.link(), &correlation_id, &command_occurred_at)
            .map_err(map_foreman_write)?;
        self.execute_ledger(plan.ledger_plan())?;
        if let Some(blocker_code) = pending_blocker {
            self.foreman
                .close_pending_worker_attempt(
                    evidence.task_ref(),
                    evidence.attempt(),
                    blocker_code.as_str(),
                    evidence.descriptor_digest(),
                    attempt.writer_fence(),
                )
                .map_err(map_foreman_write)?;
        } else {
            self.foreman
                .finalize_staged_artifact_reference(
                    evidence.task_ref(),
                    evidence.attempt(),
                    evidence.descriptor_digest(),
                )
                .map_err(map_foreman_write)?;
        }
        let replay = self.fresh_runtime()?;
        let retained = replay.references.artifact_links().iter().any(|reference| {
            reference.attempt_number() == attempt_number
                && reference.descriptor_digest() == evidence.descriptor_digest()
                && reference.link() == plan.link()
        });
        if !retained {
            return Err(reconcile("LATTICE_MANAGED_ARTIFACT_RECONCILE_REQUIRED"));
        }
        ManagedArtifactReceipt::new(evidence, storage_receipt_digest(evidence, plan.link())?)
    }

    fn record_verification(
        &mut self,
        binding: &VerifiedTaskExecutionBinding,
        attempt: &VerifiedWorkerAttemptRecord,
        evidence: &ManagedVerificationEvidence,
    ) -> ManagedPortResult<VerifiedTaskVerificationRecord> {
        let loaded = self.load_runtime()?;
        if &loaded.binding != binding {
            return Err(known("LATTICE_MANAGED_VERIFICATION_BINDING_REJECTED"));
        }
        let attempt_rows = Self::attempt_rows_with_pending(&loaded)?;
        let current_records = verify_untrusted_task_runtime_records(
            &loaded.stream,
            binding,
            &attempt_rows,
            loaded.rows.observations(),
            loaded.rows.verifications(),
        )
        .map_err(map_domain)?;
        let pending_attempt = Self::verified_pending_attempt(&loaded, &current_records)?;
        let attempts = current_records.attempts();
        let observations = current_records.observations();
        self.verify_references(&loaded.stream, binding, &loaded.references)?;
        let request = evidence.request();
        let terminal = observations.iter().find(|observation| {
            observation.attempt_number() == attempt.attempt_number()
                && observation.kind().is_terminal()
        });
        if !attempts.contains(attempt)
            || pending_attempt.as_ref() == Some(attempt)
            || terminal.is_none_or(|terminal| {
                terminal.evidence_digest() != request.worker_evidence_digest()
            })
            || request.profile_identity() != binding.verification_policy_digest()
            || request.base_commit_digest() != attempt.base_commit_digest()
            || !loaded.references.artifact_links().iter().any(|reference| {
                u64::from(reference.attempt_number()) == attempt.attempt_number()
                    && reference.descriptor_digest() == request.evidence_artifact_digest()
            })
        {
            return Err(known("LATTICE_MANAGED_VERIFICATION_BINDING_REJECTED"));
        }
        let attempt_number = u8::try_from(attempt.attempt_number())
            .map_err(|_| known("LATTICE_MANAGED_ATTEMPT_REJECTED"))?;
        let command_id =
            operation_command_id("verification", attempt_number, evidence.result_digest())?;
        let metadata = Self::append_metadata(&loaded.stream, command_id)?;
        let input = TaskVerificationInput::new(
            attempt.attempt_number(),
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
        .map_err(map_domain)?;
        let mut verification_rows = loaded.rows.verifications().to_vec();
        if let Some(recovered) = recover_task_verification_record(
            &loaded.stream,
            binding,
            attempts,
            observations,
            &metadata,
            &input,
        )
        .map_err(map_domain)?
        {
            let recovered = recovered.to_untrusted();
            if !verification_rows.contains(&recovered) {
                verification_rows.push(recovered);
            }
        }
        let records = verify_untrusted_task_runtime_records(
            &loaded.stream,
            binding,
            &attempt_rows,
            loaded.rows.observations(),
            &verification_rows,
        )
        .map_err(map_domain)?;
        let plan = plan_task_verification_append(
            &loaded.stream,
            binding,
            records.attempts(),
            records.observations(),
            records.verifications(),
            metadata,
            input,
        )
        .map_err(map_domain)?;
        self.execute_ledger(plan.ledger_plan())?;
        self.foreman
            .record_verification(plan.record())
            .map_err(map_foreman_write)?;
        let replay = self.fresh_runtime()?;
        replay
            .records
            .verifications()
            .iter()
            .find(|record| record.payload_digest() == plan.record().payload_digest())
            .cloned()
            .ok_or_else(|| reconcile("LATTICE_MANAGED_VERIFICATION_RECONCILE_REQUIRED"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayRecordIdentity {
    record_kind: String,
    record_state: ReplayRecordState,
    attempt_number: Option<u8>,
    record_ordinal: u64,
    record_digest: ContentDigest,
    ledger_stream_id: ContentDigest,
    ledger_event_sequence: u64,
    ledger_event_digest: ContentDigest,
}

impl ReplayRecordIdentity {
    fn from_record(record: &ReplayRecord) -> Self {
        Self {
            record_kind: record.record_kind().to_owned(),
            record_state: record.record_state(),
            attempt_number: record.attempt_number(),
            record_ordinal: record.record_ordinal(),
            record_digest: record.record_digest().clone(),
            ledger_stream_id: record.ledger_stream_id().clone(),
            ledger_event_sequence: record.ledger_event_sequence(),
            ledger_event_digest: record.ledger_event_digest().clone(),
        }
    }

    fn from_link(
        record_kind: &str,
        record_state: ReplayRecordState,
        attempt_number: Option<u8>,
        record_ordinal: u64,
        record_digest: &ContentDigest,
        link: &lattice_task_ledger::TaskRuntimeEventLink,
    ) -> Self {
        Self {
            record_kind: record_kind.to_owned(),
            record_state,
            attempt_number,
            record_ordinal,
            record_digest: record_digest.clone(),
            ledger_stream_id: link.stream_id().clone(),
            ledger_event_sequence: link.event_sequence(),
            ledger_event_digest: link.event_digest().clone(),
        }
    }

    fn canonical_key(&self) -> (u64, u8, u64, String) {
        (
            self.ledger_event_sequence,
            replay_kind_phase(&self.record_kind).unwrap_or(u8::MAX),
            self.record_ordinal,
            self.record_kind.clone(),
        )
    }
}

struct ReplayEventGroup {
    ledger_stream_id: ContentDigest,
    ledger_event_sequence: u64,
    attempt_number: Option<u8>,
    record_kinds: BTreeSet<String>,
}

fn verify_replay_identities(
    actual: &[ReplayRecordIdentity],
    expected: &[ReplayRecordIdentity],
) -> ManagedPortResult<()> {
    if actual.len() != expected.len() {
        return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
    }
    let mut previous_key = None;
    let mut record_keys = BTreeSet::new();
    let mut sequence_digests = BTreeMap::<(String, u64), String>::new();
    let mut event_groups = BTreeMap::<String, ReplayEventGroup>::new();
    for record in actual {
        if replay_kind_phase(&record.record_kind).is_none()
            || record.ledger_event_sequence == 0
            || record.record_ordinal == 0
            || is_zero(&record.record_digest)
            || is_zero(&record.ledger_stream_id)
            || is_zero(&record.ledger_event_digest)
            || !record_keys.insert((
                record.record_kind.clone(),
                record.attempt_number,
                record.record_ordinal,
            ))
        {
            return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
        }
        let key = record.canonical_key();
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous >= &key)
        {
            return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
        }
        previous_key = Some(key);

        let sequence_key = (
            record.ledger_stream_id.as_str().to_owned(),
            record.ledger_event_sequence,
        );
        if sequence_digests
            .insert(sequence_key, record.ledger_event_digest.as_str().to_owned())
            .is_some_and(|digest| digest != record.ledger_event_digest.as_str())
        {
            return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
        }
        let event_key = record.ledger_event_digest.as_str().to_owned();
        if let Some(group) = event_groups.get_mut(&event_key) {
            if group.ledger_stream_id != record.ledger_stream_id
                || group.ledger_event_sequence != record.ledger_event_sequence
                || group.attempt_number != record.attempt_number
                || !group.record_kinds.insert(record.record_kind.clone())
            {
                return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
            }
        } else {
            event_groups.insert(
                event_key,
                ReplayEventGroup {
                    ledger_stream_id: record.ledger_stream_id.clone(),
                    ledger_event_sequence: record.ledger_event_sequence,
                    attempt_number: record.attempt_number,
                    record_kinds: BTreeSet::from([record.record_kind.clone()]),
                },
            );
        }
    }
    for group in event_groups
        .values()
        .filter(|group| group.record_kinds.len() > 1)
    {
        if group.attempt_number.is_none()
            || !group.record_kinds.contains("WORKER_ATTEMPT")
            || group
                .record_kinds
                .iter()
                .any(|kind| kind != "WORKER_ATTEMPT" && !kind.starts_with("PROVIDER_DISPATCH_"))
        {
            return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
        }
    }
    if actual != expected {
        return Err(known("LATTICE_MANAGED_TASK_REPLAY_REJECTED"));
    }
    Ok(())
}

const fn provider_replay_kind(kind: ProviderDispatchKind) -> &'static str {
    match kind {
        ProviderDispatchKind::WorkerThread => "PROVIDER_DISPATCH_WORKER_THREAD",
        ProviderDispatchKind::WorkerTurn => "PROVIDER_DISPATCH_WORKER_TURN",
        ProviderDispatchKind::ReviewThread => "PROVIDER_DISPATCH_REVIEW_THREAD",
        ProviderDispatchKind::ReviewTurn => "PROVIDER_DISPATCH_REVIEW_TURN",
    }
}

const fn provider_replay_ordinal(kind: ProviderDispatchKind) -> u64 {
    match kind {
        ProviderDispatchKind::WorkerThread => 101,
        ProviderDispatchKind::WorkerTurn => 102,
        ProviderDispatchKind::ReviewThread => 103,
        ProviderDispatchKind::ReviewTurn => 104,
    }
}

fn replay_kind_phase(kind: &str) -> Option<u8> {
    match kind {
        "TASK_PROMOTION" => Some(1),
        "WORKER_ATTEMPT" => Some(2),
        "PROVIDER_DISPATCH_WORKER_THREAD" => Some(3),
        "PROVIDER_DISPATCH_WORKER_TURN" => Some(4),
        "PROVIDER_DISPATCH_REVIEW_THREAD" => Some(5),
        "PROVIDER_DISPATCH_REVIEW_TURN" => Some(6),
        "WORKER_OBSERVATION" => Some(7),
        "APPROVAL_EVIDENCE" => Some(8),
        "ARTIFACT_REFERENCE" => Some(9),
        "VERIFICATION" => Some(10),
        _ => None,
    }
}

fn authority_matches_binding(
    authority: &VerifiedExecutionAuthority,
    binding: &VerifiedTaskExecutionBinding,
) -> bool {
    authority.task_ref() == binding.task_ref()
        && authority.successor_stream_id() == binding.successor_stream_id()
        && authority.task_spec_digest() == binding.task_spec_digest()
        && authority.approval_subject_digest() == binding.approval_subject_digest()
        && authority.budget_digest() == binding.budget_digest()
}

fn authority_is_bootstrap_evidence(
    authority: &VerifiedExecutionAuthority,
    approval_subject_digest: &ContentDigest,
) -> bool {
    authority.approval_subject_digest() == approval_subject_digest
        && authority.capability() == ExecutionCapability::LocalReversibleTaskExecution
}

fn require_managed_successor(stream: &VerifiedStream) -> ManagedPortResult<()> {
    if stream
        .events()
        .first()
        .map(classify_task_created_profile)
        .transpose()
        .map_err(map_domain)?
        == Some(Some(TaskCreatedProfile::ManagedGeneralTaskV1))
    {
        Ok(())
    } else {
        Err(known("LATTICE_MANAGED_TASK_SPEC_ADMISSION_REJECTED"))
    }
}

fn managed_submission_matches_identity(
    submission: &TaskSpecSubmission,
    identity: &TaskLedgerStreamIdentity,
) -> bool {
    let binding = submission.binding();
    identity.subject_kind() == lattice_contracts::TaskLedgerSubjectKind::TaskSpec
        && binding.project_id() == identity.project_id()
        && binding.project_snapshot_id() == identity.project_snapshot_id()
        && binding.task_id() == identity.task_id()
        && binding.task_revision() == identity.task_revision()
        && Some(binding.task_spec_digest()) == identity.task_spec_digest()
        && submission.claimed_spec_digest() == binding.task_spec_digest()
}

fn verify_promotion_source(
    intake: &TaskSubmissionEnvelope,
    managed_submission: &TaskSpecSubmission,
    successor_identity: &TaskLedgerStreamIdentity,
    binding: &VerifiedTaskExecutionBinding,
    source: &ManagedPromotionSource,
) -> ManagedPortResult<()> {
    let rebuilt = rebuild_managed_task_spec_from_submission(
        intake,
        source.base_ref(),
        source.base_commit(),
        managed_submission,
    )
    .map_err(|_| known("LATTICE_MANAGED_PROMOTION_SOURCE_REJECTED"))?;
    if !managed_submission_matches_identity(rebuilt.submission(), successor_identity)
        || rebuilt.submission().binding().task_spec_digest() != binding.task_spec_digest()
        || rebuilt.approval_subject_digest() != binding.approval_subject_digest()
        || rebuilt.verification_policy_digest() != binding.verification_policy_digest()
    {
        return Err(known("LATTICE_MANAGED_PROMOTION_SOURCE_REJECTED"));
    }
    Ok(())
}

fn authority_is_current(authority: &VerifiedExecutionAuthority) -> ManagedPortResult<bool> {
    authority_window_is_current(authority.issued_at(), authority.expires_at())
}

fn authority_window_is_current(issued_at: &str, expires_at: &str) -> ManagedPortResult<bool> {
    let now = OffsetDateTime::now_utc();
    let issued = OffsetDateTime::parse(issued_at, &Rfc3339)
        .map_err(|_| known("LATTICE_MANAGED_EXECUTION_AUTHORITY_REJECTED"))?;
    let expires = OffsetDateTime::parse(expires_at, &Rfc3339)
        .map_err(|_| known("LATTICE_MANAGED_EXECUTION_AUTHORITY_REJECTED"))?;
    Ok(issued <= now && now < expires)
}

fn prestart_closure_blocker_shape(code: &str) -> ManagedPortResult<(&'static str, bool)> {
    match code {
        "LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT" => {
            Ok(("TASK_BOUND_EXECUTION_AUTHORITY_NOT_CURRENT", false))
        }
        "LATTICE_MANAGED_PRESTART_CONFIGURATION_REJECTED" => Ok((
            "TRUSTED_WORKER_OR_VERIFIER_CONFIGURATION_REJECTED_BEFORE_PROVIDER_EFFECT",
            false,
        )),
        "LATTICE_MANAGED_MODEL_UNAVAILABLE" => Ok((
            "SELECTED_ALLOWLISTED_MODEL_UNAVAILABLE_NO_SUBSTITUTION",
            false,
        )),
        "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED" => Ok((
            "WORKER_MODEL_PROBE_TIMED_OUT_EXACT_PRESTART_SUBTREE_REAPED",
            false,
        )),
        "LATTICE_MANAGED_REPOSITORY_LINEAGE_MISMATCH" => Ok((
            "LIVE_REPOSITORY_DOES_NOT_MATCH_RETAINED_PROMOTION_SOURCE",
            false,
        )),
        "LATTICE_MANAGED_TOKEN_BUDGET_EXHAUSTED" => {
            Ok(("CUMULATIVE_TOKEN_BUDGET_EXHAUSTED", false))
        }
        "LATTICE_MANAGED_MODEL_CALL_BUDGET_EXHAUSTED" => {
            Ok(("CUMULATIVE_MODEL_CALL_BUDGET_EXHAUSTED", false))
        }
        "LATTICE_MANAGED_RETRY_BUDGET_EXHAUSTED" => {
            Ok(("ATTEMPT_ONE_PLUS_TWO_REPAIRS_EXHAUSTED", false))
        }
        _ => Err(known("LATTICE_MANAGED_PRESTART_CLOSURE_BLOCKER_REJECTED")),
    }
}

fn retained_prestart_closure_blocker_shape(code: &str) -> ManagedPortResult<(&'static str, bool)> {
    match code {
        "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL" => {
            Ok(("PROVIDER_PROCESS_EXITED_WITHOUT_EXACT_TURN_TERMINAL", false))
        }
        "LATTICE_MANAGED_RPC_DISCONNECT_RECONCILIATION_EXHAUSTED" => {
            Ok(("BOUNDED_EXACT_PROVIDER_RECONCILIATION_EXHAUSTED", false))
        }
        "LATTICE_MANAGED_BRIDGE_HEARTBEAT_TIMEOUT_RECONCILIATION_REQUIRED" => Ok((
            "BRIDGE_SILENCE_REQUIRES_EXACT_PROVIDER_RECONCILIATION",
            false,
        )),
        "LATTICE_MANAGED_THREAD_START_RPC_INVALID_PARAMS" => Ok((
            "WORKER_THREAD_START_REJECTED_REQUIRES_EXACT_NO_EFFECT_RECONCILIATION",
            false,
        )),
        "LATTICE_MANAGED_THREAD_START_RPC_REJECTED" => Ok((
            "WORKER_THREAD_START_RPC_REJECTED_EFFECT_REMAINS_AMBIGUOUS",
            false,
        )),
        "LATTICE_MANAGED_TURN_START_RPC_INVALID_PARAMS" => Ok((
            "WORKER_TURN_START_REJECTED_REQUIRES_EXACT_NO_EFFECT_RECONCILIATION",
            false,
        )),
        "LATTICE_MANAGED_TURN_START_RPC_REJECTED" => Ok((
            "WORKER_TURN_START_RPC_REJECTED_EFFECT_REMAINS_AMBIGUOUS",
            false,
        )),
        _ => Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_BLOCKER_REJECTED")),
    }
}

fn verified_no_provider_effect_predecessor(
    binding: &VerifiedTaskExecutionBinding,
    predecessor: &VerifiedWorkerAttemptRecord,
    closure: &AttemptClosure,
    retained_evidence: &[VerifiedManagedEvidence],
    successor_packet: &AttemptPacketIdentity,
    successor_packet_digest: &ContentDigest,
) -> ManagedPortResult<VerifiedNoProviderEffectPredecessor> {
    let attempt = u8::try_from(predecessor.attempt_number())
        .map_err(|_| known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"))?;
    let (reason, retryable) = retained_prestart_closure_blocker_shape(closure.blocker_code())?;
    if retryable
        || closure.writer_fence() != predecessor.writer_fence()
        || closure.reconciliation_proof_descriptor_digest().is_none()
    {
        return Err(known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"));
    }

    let blockers = retained_evidence
        .iter()
        .filter(|evidence| {
            evidence.attempt() == attempt
                && evidence.kind() == ManagedEvidenceKind::WorkerLifecycle
                && evidence.payload_schema() == "lattice.managed-blocker.v1"
        })
        .collect::<Vec<_>>();
    let [blocker] = blockers.as_slice() else {
        return Err(reconcile(
            "LATTICE_MANAGED_RETRY_PREDECESSOR_RECONCILE_REQUIRED",
        ));
    };
    let blocker_payload: Value = serde_json::from_slice(blocker.bytes())
        .map_err(|_| known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"))?;
    let blocker_object = blocker_payload
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"))?;
    if blocker.descriptor_digest() != closure.blocker_descriptor_digest()
        || blocker.task_ref() != binding.task_ref()
        || blocker.producer_digest() != predecessor.foreman_checkpoint_digest()
        || blocker.media_type() != "application/json"
        || blocker.producer_id() != "lattice-foreman"
        || blocker.producer_version() != "1"
        || blocker_object.len() != 5
        || blocker_object.get("schema").and_then(Value::as_str)
            != Some("lattice.managed-blocker.v1")
        || blocker_object.get("attempt").and_then(Value::as_u64) != Some(u64::from(attempt))
        || blocker_object.get("code").and_then(Value::as_str) != Some(closure.blocker_code())
        || blocker_object.get("reason").and_then(Value::as_str) != Some(reason)
        || blocker_object.get("retryable").and_then(Value::as_bool) != Some(false)
    {
        return Err(known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"));
    }

    let proof_descriptor = closure
        .reconciliation_proof_descriptor_digest()
        .ok_or_else(|| known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"))?;
    let proof_ref = format!("evidence:sha256:{}", proof_descriptor.as_str());
    if successor_packet.attempt() != attempt.saturating_add(1)
        || successor_packet.writer_fence() <= predecessor.writer_fence()
        || successor_packet.prior_terminal_evidence_ref() != Some(proof_ref.as_str())
        || successor_packet.continuation().is_none()
        || pointer_content(successor_packet.worktree_ref(), "worktree")?
            != *predecessor.worktree_digest()
        || (successor_packet.model_selection().reason().as_str() == "TERRA_INSUFFICIENT"
            && (predecessor.model().as_str() != "gpt-5.6-terra"
                || successor_packet.model_selection().evidence_ref()
                    != successor_packet.prior_terminal_evidence_ref()))
    {
        return Err(known("LATTICE_MANAGED_RETRY_LINEAGE_REJECTED"));
    }
    let proofs = retained_evidence
        .iter()
        .filter(|evidence| {
            evidence.attempt() == attempt
                && evidence.kind() == ManagedEvidenceKind::WorkerLifecycle
                && evidence.payload_schema() == "lattice.managed-no-provider-effect-proof.v1"
        })
        .collect::<Vec<_>>();
    let [proof] = proofs.as_slice() else {
        return Err(reconcile(
            "LATTICE_MANAGED_RETRY_PREDECESSOR_RECONCILE_REQUIRED",
        ));
    };
    let proof_payload: Value = serde_json::from_slice(proof.bytes())
        .map_err(|_| known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"))?;
    let proof_object = proof_payload
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"))?;
    if proof.descriptor_digest() != proof_descriptor
        || proof.descriptor_digest() == blocker.descriptor_digest()
        || proof.task_ref() != binding.task_ref()
        || proof.producer_digest() != predecessor.foreman_checkpoint_digest()
        || proof.media_type() != "application/json"
        || proof.producer_id() != "lattice-foreman"
        || proof.producer_version() != "1"
        || proof_object.len() != 9
        || proof_object.get("schema").and_then(Value::as_str)
            != Some("lattice.managed-no-provider-effect-proof.v1")
        || proof_object.get("task_ref").and_then(Value::as_str) != Some(binding.task_ref().as_str())
        || proof_object.get("attempt").and_then(Value::as_u64) != Some(u64::from(attempt))
        || proof_object
            .get("blocker_descriptor_digest")
            .and_then(Value::as_str)
            != Some(blocker.descriptor_digest().as_str())
        || !matches!(
            proof_object.get("proof_kind").and_then(Value::as_str),
            Some("PROVEN_NO_PROVIDER_CANDIDATE" | "EXACT_EMPTY_THREAD_NO_TURN")
        )
        || proof_object
            .get("worker_thread_claimed")
            .and_then(Value::as_bool)
            .is_none()
        || proof_object
            .get("worker_turn_claimed")
            .and_then(Value::as_bool)
            .is_none()
        || !proof_object.contains_key("thread_observation_payload_digest")
        || !proof_object.contains_key("thread_observation_evidence_digest")
    {
        return Err(known("LATTICE_MANAGED_RETRY_PREDECESSOR_REJECTED"));
    }

    VerifiedNoProviderEffectPredecessor::new(
        binding,
        predecessor,
        NO_PROVIDER_EFFECT_CLOSURE_OWNER,
        binding.task_ref(),
        predecessor.attempt_id(),
        predecessor.attempt_number(),
        predecessor.writer_fence(),
        closure.blocker_code(),
        blocker.descriptor_digest().clone(),
        proof.descriptor_digest().clone(),
        successor_packet_digest.clone(),
    )
    .map_err(map_domain)
}

fn validate_terminal_repair_successor(
    predecessor: &VerifiedWorkerAttemptRecord,
    terminal: &VerifiedWorkerObservationRecord,
    successor_packet: &AttemptPacketIdentity,
) -> ManagedPortResult<()> {
    let predecessor_attempt = u8::try_from(predecessor.attempt_number())
        .map_err(|_| known("LATTICE_MANAGED_RETRY_LINEAGE_REJECTED"))?;
    if terminal.task_ref() != predecessor.task_ref()
        || terminal.successor_stream_id() != predecessor.successor_stream_id()
        || terminal.binding_digest() != predecessor.binding_digest()
        || terminal.attempt_id() != predecessor.attempt_id()
    {
        return Err(known("LATTICE_MANAGED_RETRY_LINEAGE_REJECTED"));
    }
    validate_terminal_repair_packet(
        predecessor_attempt,
        predecessor.writer_fence(),
        predecessor.model(),
        predecessor.worktree_digest(),
        terminal.evidence_digest(),
        successor_packet,
    )
}

fn validate_terminal_repair_packet(
    predecessor_attempt: u8,
    predecessor_writer_fence: u64,
    predecessor_model: WorkerModel,
    predecessor_worktree_digest: &ContentDigest,
    terminal_evidence_digest: &ContentDigest,
    successor_packet: &AttemptPacketIdentity,
) -> ManagedPortResult<()> {
    let terminal_ref = format!("evidence:sha256:{}", terminal_evidence_digest.as_str());
    if successor_packet.attempt() != predecessor_attempt.saturating_add(1)
        || successor_packet.writer_fence() <= predecessor_writer_fence
        || successor_packet.prior_terminal_evidence_ref() != Some(terminal_ref.as_str())
        || successor_packet.continuation().is_none()
        || pointer_content(successor_packet.worktree_ref(), "worktree")?
            != *predecessor_worktree_digest
        || (successor_packet.model_selection().reason().as_str() == "TERRA_INSUFFICIENT"
            && (predecessor_model.as_str() != "gpt-5.6-terra"
                || successor_packet.model_selection().evidence_ref()
                    != successor_packet.prior_terminal_evidence_ref()))
    {
        return Err(known("LATTICE_MANAGED_RETRY_LINEAGE_REJECTED"));
    }
    Ok(())
}

fn retained_no_effect_proof_bytes(
    task_ref: &ContentDigest,
    attempt: u8,
    blocker_descriptor_digest: &ContentDigest,
    proof: &ManagedPrestartNoEffectProof,
) -> ManagedPortResult<Vec<u8>> {
    let (proof_kind, worker_thread_claimed, worker_turn_claimed, payload, evidence) = match proof {
        ManagedPrestartNoEffectProof::PendingReservation => {
            return Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_PROOF_REJECTED"));
        }
        ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
            worker_thread_claimed,
        } => {
            if *worker_thread_claimed {
                return Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_PROOF_REJECTED"));
            }
            ("PROVEN_NO_PROVIDER_CANDIDATE", false, false, None, None)
        }
        ManagedPrestartNoEffectProof::ExactEmptyThreadNoTurn {
            thread,
            worker_turn_claimed,
        } => {
            if thread.attempt_number() != u64::from(attempt)
                || thread.kind() != lattice_task_ledger::WorkerObservationKind::ThreadAccepted
                || thread.turn_id().is_some()
            {
                return Err(known("LATTICE_MANAGED_RETAINED_CLOSURE_PROOF_REJECTED"));
            }
            (
                "EXACT_EMPTY_THREAD_NO_TURN",
                true,
                *worker_turn_claimed,
                Some(thread.payload_digest().as_str()),
                Some(thread.evidence_digest().as_str()),
            )
        }
    };
    serde_json::to_vec(&serde_json::json!({
        "schema": "lattice.managed-no-provider-effect-proof.v1",
        "task_ref": task_ref.as_str(),
        "attempt": attempt,
        "blocker_descriptor_digest": blocker_descriptor_digest.as_str(),
        "proof_kind": proof_kind,
        "worker_thread_claimed": worker_thread_claimed,
        "worker_turn_claimed": worker_turn_claimed,
        "thread_observation_payload_digest": payload,
        "thread_observation_evidence_digest": evidence,
    }))
    .map_err(|_| known("LATTICE_MANAGED_RETAINED_CLOSURE_PROOF_REJECTED"))
}

fn pending_prestart_blocker_code(
    evidence: &VerifiedManagedEvidence,
    attempt: u8,
) -> ManagedPortResult<String> {
    if evidence.kind() != ManagedEvidenceKind::WorkerLifecycle
        || evidence.media_type() != "application/json"
        || evidence.payload_schema() != "lattice.managed-blocker.v1"
        || evidence.producer_id() != "lattice-foreman"
        || evidence.producer_version() != "1"
    {
        return Err(known("LATTICE_MANAGED_ARTIFACT_BINDING_REJECTED"));
    }
    let payload: Value = serde_json::from_slice(evidence.bytes())
        .map_err(|_| known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED"))?;
    let object = payload
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED"))?;
    let code = object
        .get("code")
        .and_then(Value::as_str)
        .ok_or_else(|| known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED"))?;
    let (reason, retryable) = prestart_closure_blocker_shape(code)?;
    if object.len() != 5
        || object.get("schema").and_then(Value::as_str) != Some("lattice.managed-blocker.v1")
        || object.get("attempt").and_then(Value::as_u64) != Some(u64::from(attempt))
        || object.get("reason").and_then(Value::as_str) != Some(reason)
        || object.get("retryable").and_then(Value::as_bool) != Some(retryable)
    {
        return Err(known("LATTICE_MANAGED_BLOCKER_EVIDENCE_REJECTED"));
    }
    Ok(code.to_owned())
}

fn operation_command_id(
    operation: &str,
    attempt: u8,
    digest: &ContentDigest,
) -> ManagedPortResult<CommandId> {
    CommandId::new(format!("managed-{operation}-{attempt}-{}", digest.as_str())).map_err(map_domain)
}

fn managed_attempt_id(task_ref: &ContentDigest, attempt: u8) -> ManagedPortResult<AttemptId> {
    AttemptId::new(format!("managed-attempt-{}-{attempt}", task_ref.as_str())).map_err(map_contract)
}

fn pointer_content(value: &str, kind: &str) -> ManagedPortResult<ContentDigest> {
    let prefix = format!("{kind}:sha256:");
    let digest = value
        .strip_prefix(&prefix)
        .ok_or_else(|| known("LATTICE_MANAGED_DIGEST_POINTER_REJECTED"))?;
    ContentDigest::from_sha256(digest.to_owned())
        .map_err(|_| known("LATTICE_MANAGED_DIGEST_POINTER_REJECTED"))
}

fn now_utc() -> ManagedPortResult<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|_| known("LATTICE_MANAGED_CLOCK_REJECTED"))
}

fn sha256_bytes(bytes: &[u8]) -> ManagedPortResult<ContentDigest> {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").map_err(|_| known("LATTICE_MANAGED_DIGEST_REJECTED"))?;
    }
    ContentDigest::from_sha256(value).map_err(|_| known("LATTICE_MANAGED_DIGEST_REJECTED"))
}

fn storage_receipt_digest(
    evidence: &VerifiedManagedEvidence,
    link: &lattice_task_ledger::TaskRuntimeEventLink,
) -> ManagedPortResult<ContentDigest> {
    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_MANAGED_POSTGRES_ARTIFACT_RECEIPT_V1\0");
    hasher.update(evidence.task_ref().as_str().as_bytes());
    hasher.update([evidence.attempt()]);
    hasher.update(evidence.descriptor_digest().as_str().as_bytes());
    hasher.update(link.event_digest().as_str().as_bytes());
    sha256_bytes(&hasher.finalize())
}

fn update_dispatch_frame(hasher: &mut Sha256, value: &[u8]) -> ManagedPortResult<()> {
    hasher.update(
        u64::try_from(value.len())
            .map_err(|_| known("LATTICE_MANAGED_DISPATCH_DIGEST_REJECTED"))?
            .to_be_bytes(),
    );
    hasher.update(value);
    Ok(())
}

fn managed_dispatch_subject_digest(
    operation: &str,
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    operation_digests: &[&ContentDigest],
) -> ManagedPortResult<ContentDigest> {
    let mut hasher = Sha256::new();
    hasher.update(b"LATTICE_MANAGED_PROVIDER_DISPATCH_SUBJECT_V1\0");
    let attempt_number = attempt.attempt_number().to_string();
    let writer_fence = attempt.writer_fence().to_string();
    let foreman_generation = attempt.foreman_generation().to_string();
    for value in [
        operation,
        binding.task_ref().as_str(),
        binding.binding_digest().as_str(),
        attempt.payload_digest().as_str(),
        &attempt_number,
        &writer_fence,
        &foreman_generation,
        attempt.foreman_checkpoint_digest().as_str(),
    ] {
        update_dispatch_frame(&mut hasher, value.as_bytes())?;
    }
    for digest in operation_digests {
        update_dispatch_frame(&mut hasher, digest.as_str().as_bytes())?;
    }
    sha256_bytes(&hasher.finalize())
}

fn managed_review_dispatch_subject_digest(
    operation: &str,
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    terminal: &VerifiedWorkerObservationRecord,
    request: &ManagedVerificationRequest,
    lifecycle_digest: Option<&ContentDigest>,
) -> ManagedPortResult<ContentDigest> {
    let operation_digests = managed_review_dispatch_operation_digests(
        terminal.payload_digest(),
        terminal.evidence_digest(),
        request,
        lifecycle_digest,
    );
    managed_dispatch_subject_digest(operation, binding, attempt, &operation_digests)
}

fn managed_review_dispatch_operation_digests<'digest>(
    terminal_payload_digest: &'digest ContentDigest,
    terminal_evidence_digest: &'digest ContentDigest,
    request: &'digest ManagedVerificationRequest,
    lifecycle_digest: Option<&'digest ContentDigest>,
) -> Vec<&'digest ContentDigest> {
    let mut operation_digests = vec![
        terminal_payload_digest,
        terminal_evidence_digest,
        request.profile_identity(),
        request.command_identity(),
        request.base_commit_digest(),
        request.result_commit_digest(),
        request.tree_digest(),
        request.diff_digest(),
        request.worker_evidence_digest(),
        request.evidence_artifact_digest(),
    ];
    if let Some(lifecycle_digest) = lifecycle_digest {
        operation_digests.push(lifecycle_digest);
    }
    operation_digests
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProviderDispatchProgress {
    worker_thread_observed: bool,
    worker_turn_observed: bool,
    review_thread_observed: bool,
    review_turn_observed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProviderDispatchPresence {
    worker_thread: bool,
    worker_turn: bool,
    review_thread: bool,
    review_turn: bool,
}

fn verify_provider_dispatch_presence(
    progress: ProviderDispatchProgress,
    presence: ProviderDispatchPresence,
) -> ManagedPortResult<()> {
    let missing_observed_claim = (progress.worker_thread_observed && !presence.worker_thread)
        || (progress.worker_turn_observed && !presence.worker_turn)
        || (progress.review_thread_observed && !presence.review_thread)
        || (progress.review_turn_observed && !presence.review_turn);
    let broken_claim_dependency = (presence.worker_turn && !presence.worker_thread)
        || (presence.review_thread && (!presence.worker_thread || !presence.worker_turn))
        || (presence.review_turn && !presence.review_thread);
    if missing_observed_claim || broken_claim_dependency {
        return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
    }
    Ok(())
}

fn managed_verification_request_from_snapshot(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    terminal: &VerifiedWorkerObservationRecord,
    evidence: &VerifiedManagedEvidence,
    execution_environment: Option<&ExecutionEnvironmentDescriptor>,
) -> ManagedPortResult<ManagedVerificationRequest> {
    if evidence.kind() != ManagedEvidenceKind::GitSnapshot
        || evidence.payload_schema() != "lattice.managed-git-snapshot/1.0"
        || evidence.producer_id() != "lattice-runtime-managed-verifier"
        || evidence.producer_version() != "1.0"
        || evidence.task_ref() != binding.task_ref()
        || u64::from(evidence.attempt()) != attempt.attempt_number()
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
    }
    let value: Value = serde_json::from_slice(evidence.bytes())
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))?;
    let object = value
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))?;
    let keys = [
        "schema",
        "base_commit",
        "result_commit",
        "tree",
        "diff_digest",
        "command_identity",
        "execution_environment_ref",
        "execution_environment_descriptor_digest",
        "changed_paths",
        "checks",
    ];
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))
    };
    let base_commit = text("base_commit")?;
    let result_commit = text("result_commit")?;
    let tree = text("tree")?;
    if object.len() != keys.len()
        || keys.iter().any(|key| !object.contains_key(*key))
        || text("schema")? != "lattice.managed-git-snapshot/1.0"
        || !valid_git_object_id(base_commit)
        || !valid_git_object_id(result_commit)
        || !valid_git_object_id(tree)
        || sha256_bytes(base_commit.as_bytes())? != *attempt.base_commit_digest()
        || !snapshot_execution_environment_matches(
            object,
            execution_environment.map(|descriptor| {
                (
                    descriptor.environment_ref().as_str(),
                    descriptor.descriptor_digest().as_str(),
                )
            }),
        )
    {
        return Err(known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"));
    }
    let command_identity = ContentDigest::from_sha256(text("command_identity")?.to_owned())
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))?;
    let diff_digest = ContentDigest::from_sha256(text("diff_digest")?.to_owned())
        .map_err(|_| known("LATTICE_MANAGED_PROVIDER_DISPATCH_REPLAY_REJECTED"))?;
    ManagedVerificationRequest::new(
        binding.verification_policy_digest().clone(),
        command_identity,
        attempt.base_commit_digest().clone(),
        sha256_bytes(result_commit.as_bytes())?,
        sha256_bytes(tree.as_bytes())?,
        diff_digest,
        terminal.evidence_digest().clone(),
        evidence,
    )
}

fn snapshot_execution_environment_matches(
    object: &serde_json::Map<String, Value>,
    expected: Option<(&str, &str)>,
) -> bool {
    match expected {
        Some((environment_ref, descriptor_digest)) => {
            object
                .get("execution_environment_ref")
                .and_then(Value::as_str)
                == Some(environment_ref)
                && object
                    .get("execution_environment_descriptor_digest")
                    .and_then(Value::as_str)
                    == Some(descriptor_digest)
        }
        None => {
            matches!(object.get("execution_environment_ref"), Some(Value::Null))
                && matches!(
                    object.get("execution_environment_descriptor_digest"),
                    Some(Value::Null)
                )
        }
    }
}

fn valid_git_object_id(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[derive(Debug, Eq, PartialEq)]
struct ReviewLifecycleIdentity {
    event_type: String,
    task_ref: String,
    attempt: u64,
    subject_digest: String,
    prompt_digest: String,
    thread_id: String,
    turn_id: Option<String>,
}

fn parse_review_lifecycle_identity(
    evidence: &VerifiedManagedEvidence,
) -> ManagedPortResult<ReviewLifecycleIdentity> {
    if evidence.kind() != lattice_artifact_store::ManagedEvidenceKind::WorkerLifecycle
        || evidence.payload_schema() != "lattice.managed-review-lifecycle/1.0"
    {
        return Err(known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"));
    }
    let value: Value = serde_json::from_slice(evidence.bytes())
        .map_err(|_| known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"))?;
    let object = value
        .as_object()
        .ok_or_else(|| known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"))?;
    let text = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"))
    };
    let event_type = text("event_type")?.to_owned();
    let task_ref = text("task_ref")?.to_owned();
    let attempt = object
        .get("attempt")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"))?;
    let subject_digest = text("subject_digest")?.to_owned();
    let prompt_digest = text("prompt_digest")?.to_owned();
    let thread_id = text("thread_id")?.to_owned();
    let turn_id = match object.get("turn_id") {
        Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        _ => return Err(known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED")),
    };
    let event_is_thread = matches!(
        event_type.as_str(),
        "THREAD_START_ACCEPTED" | "THREAD_STARTED" | "THREAD_RECONCILED"
    );
    let event_is_turn = matches!(
        event_type.as_str(),
        "TURN_START_ACCEPTED" | "TURN_STARTED" | "TURN_RECONCILED" | "TURN_TERMINAL"
    );
    if task_ref != evidence.task_ref().as_str()
        || attempt != u64::from(evidence.attempt())
        || (!event_is_thread && !event_is_turn)
        || !valid_dispatch_identifier(&thread_id)
        || turn_id
            .as_deref()
            .is_some_and(|value| !valid_dispatch_identifier(value))
        || (matches!(
            event_type.as_str(),
            "THREAD_START_ACCEPTED" | "THREAD_STARTED"
        ) && turn_id.is_some())
        || (event_is_turn && turn_id.is_none())
        || ContentDigest::from_sha256(subject_digest.clone()).is_err()
        || ContentDigest::from_sha256(prompt_digest.clone()).is_err()
    {
        return Err(known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"));
    }
    Ok(ReviewLifecycleIdentity {
        event_type,
        task_ref,
        attempt,
        subject_digest,
        prompt_digest,
        thread_id,
        turn_id,
    })
}

fn review_lifecycle_implies_turn_dispatch(identity: &ReviewLifecycleIdentity) -> bool {
    matches!(
        identity.event_type.as_str(),
        "TURN_START_ACCEPTED" | "TURN_STARTED" | "TURN_RECONCILED" | "TURN_TERMINAL"
    ) || (identity.event_type == "THREAD_RECONCILED" && identity.turn_id.is_some())
}

fn select_review_turn_anchor<'evidence>(
    evidence: &'evidence [VerifiedManagedEvidence],
    incoming: &VerifiedManagedEvidence,
) -> ManagedPortResult<&'evidence VerifiedManagedEvidence> {
    let incoming_identity = parse_review_lifecycle_identity(incoming)?;
    if !matches!(
        incoming_identity.event_type.as_str(),
        "THREAD_STARTED" | "THREAD_RECONCILED"
    ) || incoming_identity.turn_id.is_some()
    {
        return Err(known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"));
    }
    let mut anchors = Vec::new();
    for item in evidence.iter().filter(|item| {
        item.attempt() == incoming.attempt()
            && item.payload_schema() == "lattice.managed-review-lifecycle/1.0"
    }) {
        let identity = parse_review_lifecycle_identity(item)?;
        if identity.event_type == "THREAD_START_ACCEPTED" {
            anchors.push((item, identity));
        }
    }
    let [(anchor, anchor_identity)] = anchors.as_slice() else {
        return Err(known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"));
    };
    if anchor.project_id() != incoming.project_id()
        || anchor.task_ref() != incoming.task_ref()
        || anchor_identity.turn_id.is_some()
        || anchor_identity.task_ref != incoming_identity.task_ref
        || anchor_identity.attempt != incoming_identity.attempt
        || anchor_identity.subject_digest != incoming_identity.subject_digest
        || anchor_identity.prompt_digest != incoming_identity.prompt_digest
        || anchor_identity.thread_id != incoming_identity.thread_id
    {
        return Err(known("LATTICE_MANAGED_REVIEW_TURN_ANCHOR_REJECTED"));
    }
    Ok(anchor)
}

fn valid_dispatch_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b':' | b'-'))
        })
}

fn is_zero(digest: &ContentDigest) -> bool {
    digest.as_str().bytes().all(|byte| byte == b'0')
}

fn cross_owner_snapshot_retry_allowed(pass: usize, error: &ManagedPortError) -> bool {
    pass + 1 < CROSS_OWNER_SNAPSHOT_RETRY_LIMIT
        && error.kind() == ManagedPortErrorKind::Known
        && error.code() == "LEDGER_INVALID_TASK_RUNTIME_RECORD"
}

fn cross_owner_snapshot_retry_delay(pass: usize) -> Duration {
    CROSS_OWNER_SNAPSHOT_RETRY_BASE_DELAY.saturating_mul(1_u32 << pass.min(2))
}

fn known(code: &'static str) -> ManagedPortError {
    ManagedPortError::new(ManagedPortErrorKind::Known, code)
}

fn reconcile(code: &'static str) -> ManagedPortError {
    ManagedPortError::new(ManagedPortErrorKind::ReconcileRequired, code)
}

fn map_contract(error: lattice_contracts::ContractError) -> ManagedPortError {
    let _ = error;
    known("LATTICE_MANAGED_CONTRACT_REJECTED")
}

#[allow(clippy::needless_pass_by_value)]
fn map_domain(error: lattice_task_ledger::LedgerError) -> ManagedPortError {
    ManagedPortError::new(ManagedPortErrorKind::Known, error.code())
}

fn map_ledger_read(error: PostgresTaskLedgerError) -> ManagedPortError {
    match error.kind() {
        PostgresTaskLedgerErrorKind::Unavailable
        | PostgresTaskLedgerErrorKind::SerializationExhausted
        | PostgresTaskLedgerErrorKind::TransactionFailed
        | PostgresTaskLedgerErrorKind::CommitOutcomeUnknown => reconcile(error.code()),
        _ => known(error.code()),
    }
}

fn map_ledger_write(error: PostgresTaskLedgerError) -> ManagedPortError {
    match error.kind() {
        PostgresTaskLedgerErrorKind::Malformed
        | PostgresTaskLedgerErrorKind::CommandSubstitution
        | PostgresTaskLedgerErrorKind::ProjectRegistryInactive
        | PostgresTaskLedgerErrorKind::AdmissionDenied
        | PostgresTaskLedgerErrorKind::AuthorityMismatch
        | PostgresTaskLedgerErrorKind::RevisionOverflow => known(error.code()),
        _ => reconcile(error.code()),
    }
}

fn map_foreman_read(error: AdapterError) -> ManagedPortError {
    match error.kind() {
        AdapterErrorKind::Database => reconcile(error.code()),
        AdapterErrorKind::Setup
        | AdapterErrorKind::InvalidInput
        | AdapterErrorKind::ClaimRejected
        | AdapterErrorKind::QuotaRejected
        | AdapterErrorKind::CorruptReplay => known(error.code()),
    }
}

fn map_foreman_write(error: AdapterError) -> ManagedPortError {
    match error.kind() {
        AdapterErrorKind::Database => reconcile(error.code()),
        AdapterErrorKind::Setup
        | AdapterErrorKind::InvalidInput
        | AdapterErrorKind::ClaimRejected
        | AdapterErrorKind::QuotaRejected
        | AdapterErrorKind::CorruptReplay => known(error.code()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        AttemptClaimPersistenceStep, AttemptReservationReplaySource,
        CROSS_OWNER_SNAPSHOT_RETRY_LIMIT, PendingExecutionEnvironmentPersistence,
        ProviderDispatchPresence, ProviderDispatchProgress, ReplayRecordIdentity,
        attempt_claim_persistence_steps, attempt_reservation_replay_source,
        authority_window_is_current, cross_owner_snapshot_retry_allowed,
        cross_owner_snapshot_retry_delay, managed_attempt_id,
        managed_review_dispatch_operation_digests, operation_command_id,
        pending_execution_environment_persistence, pointer_content, prestart_closure_blocker_shape,
        retained_no_effect_proof_bytes, retained_prestart_closure_blocker_shape,
        select_review_turn_anchor, sha256_bytes, snapshot_execution_environment_matches,
        validate_terminal_repair_packet, verify_provider_dispatch_presence,
        verify_replay_identities,
    };
    use lattice_artifact_store::{
        ManagedEvidenceInput, ManagedEvidenceKind, VerifiedManagedEvidence,
    };
    use lattice_contracts::{ContentDigest, ProjectId};
    use lattice_foreman_state::{
        AttemptPacketIdentity, ContinuationSummary, ExternalCostBudget, ModelSelection,
        WorkerBudget,
    };
    use lattice_ports::{ManagedPortError, ManagedPortErrorKind};
    use lattice_ports::{ManagedPrestartNoEffectProof, ManagedVerificationRequest};
    use lattice_postgres_foreman::ClaimReservationDisposition;
    use lattice_postgres_foreman::ReplayRecordState;
    use lattice_task_ledger::{ModelReason, ReasoningEffort, WorkerModel};

    fn digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
    }

    fn terminal_repair_packet(
        prior_terminal: &ContentDigest,
        worktree: &ContentDigest,
    ) -> AttemptPacketIdentity {
        let budget = WorkerBudget::new(
            4,
            1,
            2,
            900,
            100_000,
            3,
            ExternalCostBudget::Unavailable,
            "2026-08-29T12:30:00Z",
        )
        .expect("budget");
        let selection = ModelSelection::new(
            WorkerModel::Terra,
            ReasoningEffort::Medium,
            ModelReason::RoutineEngineering,
            None,
        )
        .expect("selection");
        let prior_ref = format!("evidence:sha256:{}", prior_terminal.as_str());
        AttemptPacketIdentity::new(
            "taskref-terminal-repair",
            2,
            &format!("project:sha256:{}", digest('1').as_str()),
            &format!("spec:sha256:{}", digest('2').as_str()),
            &format!("approval:sha256:{}", digest('3').as_str()),
            &budget,
            &format!("verification:sha256:{}", digest('4').as_str()),
            &format!("worktree:sha256:{}", worktree.as_str()),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            selection,
            2,
            Some(&prior_ref),
            Some(ContinuationSummary::new("bounded repair").expect("continuation")),
        )
        .expect("repair packet")
    }

    #[test]
    fn terminal_retry_lineage_binds_exact_terminal_evidence_and_worktree() {
        let terminal = digest('6');
        let worktree = digest('5');
        let exact = terminal_repair_packet(&terminal, &worktree);
        validate_terminal_repair_packet(1, 1, WorkerModel::Terra, &worktree, &terminal, &exact)
            .expect("exact terminal repair lineage");

        let substituted_terminal = terminal_repair_packet(&digest('7'), &worktree);
        let error = validate_terminal_repair_packet(
            1,
            1,
            WorkerModel::Terra,
            &worktree,
            &terminal,
            &substituted_terminal,
        )
        .expect_err("terminal evidence substitution must fail closed");
        assert_eq!(error.code(), "LATTICE_MANAGED_RETRY_LINEAGE_REJECTED");

        let foreign_worktree = terminal_repair_packet(&terminal, &digest('8'));
        assert!(
            validate_terminal_repair_packet(
                1,
                1,
                WorkerModel::Terra,
                &worktree,
                &terminal,
                &foreign_worktree,
            )
            .is_err()
        );
    }

    #[test]
    fn cross_owner_skew_retry_is_bounded_and_never_reclassifies_tamper() {
        let skew = ManagedPortError::new(
            ManagedPortErrorKind::Known,
            "LEDGER_INVALID_TASK_RUNTIME_RECORD",
        );
        for pass in 0..CROSS_OWNER_SNAPSHOT_RETRY_LIMIT - 1 {
            assert!(cross_owner_snapshot_retry_allowed(pass, &skew));
        }
        assert!(!cross_owner_snapshot_retry_allowed(
            CROSS_OWNER_SNAPSHOT_RETRY_LIMIT - 1,
            &skew,
        ));
        assert_eq!(
            cross_owner_snapshot_retry_delay(0),
            Duration::from_millis(5)
        );
        assert_eq!(
            cross_owner_snapshot_retry_delay(1),
            Duration::from_millis(10)
        );
        assert_eq!(
            cross_owner_snapshot_retry_delay(2),
            Duration::from_millis(20)
        );
        assert_eq!(
            cross_owner_snapshot_retry_delay(10),
            Duration::from_millis(20)
        );
        assert!(!cross_owner_snapshot_retry_allowed(
            0,
            &ManagedPortError::new(
                ManagedPortErrorKind::Known,
                "LATTICE_MANAGED_TASK_REPLAY_REJECTED",
            ),
        ));
        assert!(!cross_owner_snapshot_retry_allowed(
            0,
            &ManagedPortError::new(
                ManagedPortErrorKind::ReconcileRequired,
                "LEDGER_INVALID_TASK_RUNTIME_RECORD",
            ),
        ));
    }

    fn review_lifecycle(event_type: &str, thread_id: &str) -> VerifiedManagedEvidence {
        let task_ref = digest('a');
        let bytes = serde_json::to_vec(&serde_json::json!({
            "schema": "lattice.managed-review-lifecycle/1.0",
            "sequence": if event_type == "THREAD_START_ACCEPTED" { 1 } else { 2 },
            "event_type": event_type,
            "task_ref": task_ref.as_str(),
            "attempt": 1,
            "subject_digest": digest('b').as_str(),
            "prompt_digest": digest('c').as_str(),
            "thread_id": thread_id,
            "turn_id": null,
            "app_server_generation": 7,
            "model": "gpt-5.6-terra",
            "reasoning": "medium",
            "model_reason": "INDEPENDENT_CODE_REVIEW",
            "model_call_identity": format!("managed-review-{}-1", task_ref.as_str()),
            "observed_at": "2026-08-27T12:00:00Z",
            "terminal_status": null,
        }))
        .expect("lifecycle json");
        VerifiedManagedEvidence::new(
            ManagedEvidenceInput::new(
                ProjectId::new("project-review-anchor").expect("project"),
                task_ref,
                1,
                ManagedEvidenceKind::WorkerLifecycle,
                "application/json",
                "lattice.managed-review-lifecycle/1.0",
                "reviewer",
                "1",
                digest('d'),
                "2026-08-27T12:00:00Z",
                bytes,
            )
            .expect("evidence input"),
        )
        .expect("evidence")
    }

    fn replay_identity(
        kind: &str,
        attempt: Option<u8>,
        ordinal: u64,
        record_digest: char,
        sequence: u64,
        event_digest: char,
    ) -> ReplayRecordIdentity {
        ReplayRecordIdentity {
            record_kind: kind.to_owned(),
            record_state: ReplayRecordState::Retained,
            attempt_number: attempt,
            record_ordinal: ordinal,
            record_digest: digest(record_digest),
            ledger_stream_id: digest('f'),
            ledger_event_sequence: sequence,
            ledger_event_digest: digest(event_digest),
        }
    }

    fn exact_provider_replay() -> Vec<ReplayRecordIdentity> {
        vec![
            replay_identity("TASK_PROMOTION", None, 1, '1', 1, 'a'),
            replay_identity("WORKER_ATTEMPT", Some(1), 1, '2', 2, 'b'),
            replay_identity("PROVIDER_DISPATCH_WORKER_THREAD", Some(1), 101, '3', 2, 'b'),
            replay_identity("PROVIDER_DISPATCH_WORKER_TURN", Some(1), 102, '4', 2, 'b'),
            replay_identity("PROVIDER_DISPATCH_REVIEW_THREAD", Some(1), 103, '5', 2, 'b'),
            replay_identity("PROVIDER_DISPATCH_REVIEW_TURN", Some(1), 104, '6', 2, 'b'),
            replay_identity("WORKER_OBSERVATION", Some(1), 1, '7', 3, 'c'),
            replay_identity("ARTIFACT_REFERENCE", Some(1), 4, '8', 4, 'd'),
            replay_identity("ARTIFACT_REFERENCE", Some(1), 5, '9', 5, 'e'),
            replay_identity("VERIFICATION", Some(1), 1, 'a', 6, '1'),
        ]
    }

    #[test]
    fn operation_ids_are_attempt_and_evidence_bound_without_free_form_input() {
        let one = operation_command_id("attempt", 1, &digest('a')).expect("command");
        let replay = operation_command_id("attempt", 1, &digest('a')).expect("command");
        let retry = operation_command_id("attempt", 2, &digest('a')).expect("command");
        assert_eq!(one, replay);
        assert_ne!(one, retry);
        assert!(!one.as_str().contains(' '));
    }

    #[test]
    fn provider_dispatch_replay_is_exact_and_fresh_restart_idempotent() {
        let expected = exact_provider_replay();
        verify_replay_identities(&expected, &expected).expect("exact replay");
        verify_replay_identities(&expected.clone(), &expected).expect("fresh restart replay");
    }

    #[test]
    fn provider_dispatch_replay_rejects_missing_duplicate_and_tamper() {
        let expected = exact_provider_replay();

        let mut missing = expected.clone();
        missing.remove(3);
        assert!(verify_replay_identities(&missing, &expected).is_err());

        let mut duplicate = expected.clone();
        duplicate[4] = duplicate[3].clone();
        assert!(verify_replay_identities(&duplicate, &expected).is_err());

        let mut tampered = expected.clone();
        tampered[2].record_digest = digest('a');
        assert!(verify_replay_identities(&tampered, &expected).is_err());
    }

    #[test]
    fn multiple_artifacts_use_owner_ledger_sequence_and_reject_reorder_duplicate_or_tamper() {
        let expected = exact_provider_replay();
        let artifacts = expected
            .iter()
            .filter(|record| record.record_kind == "ARTIFACT_REFERENCE")
            .collect::<Vec<_>>();
        assert_eq!(artifacts.len(), 2);
        assert_eq!(
            artifacts[0].record_ordinal,
            artifacts[0].ledger_event_sequence
        );
        assert_eq!(
            artifacts[1].record_ordinal,
            artifacts[1].ledger_event_sequence
        );
        verify_replay_identities(&expected, &expected).expect("two artifact exact replay");

        let mut reordered = expected.clone();
        reordered.swap(7, 8);
        assert!(verify_replay_identities(&reordered, &expected).is_err());

        let mut duplicate = expected.clone();
        duplicate[8].record_ordinal = duplicate[7].record_ordinal;
        assert!(verify_replay_identities(&duplicate, &expected).is_err());

        let mut tampered = expected.clone();
        tampered[8].record_ordinal = tampered[8].record_ordinal.saturating_add(1);
        assert!(verify_replay_identities(&tampered, &expected).is_err());
    }

    #[test]
    fn observed_provider_progress_requires_every_exact_predecessor_claim() {
        let none = ProviderDispatchProgress {
            worker_thread_observed: false,
            worker_turn_observed: false,
            review_thread_observed: false,
            review_turn_observed: false,
        };
        let no_claims = ProviderDispatchPresence {
            worker_thread: false,
            worker_turn: false,
            review_thread: false,
            review_turn: false,
        };
        verify_provider_dispatch_presence(none, no_claims).expect("pre-dispatch attempt");

        let complete = ProviderDispatchPresence {
            worker_thread: true,
            worker_turn: true,
            review_thread: true,
            review_turn: true,
        };
        verify_provider_dispatch_presence(
            ProviderDispatchProgress {
                worker_thread_observed: true,
                worker_turn_observed: true,
                review_thread_observed: true,
                review_turn_observed: true,
            },
            complete,
        )
        .expect("fully linked replay");

        for (progress, presence) in [
            (
                ProviderDispatchProgress {
                    worker_thread_observed: true,
                    ..none
                },
                no_claims,
            ),
            (
                ProviderDispatchProgress {
                    worker_thread_observed: true,
                    worker_turn_observed: true,
                    ..none
                },
                ProviderDispatchPresence {
                    worker_thread: true,
                    ..no_claims
                },
            ),
            (
                ProviderDispatchProgress {
                    worker_thread_observed: true,
                    worker_turn_observed: true,
                    review_thread_observed: true,
                    ..none
                },
                ProviderDispatchPresence {
                    worker_thread: true,
                    worker_turn: true,
                    ..no_claims
                },
            ),
            (
                ProviderDispatchProgress {
                    worker_thread_observed: true,
                    worker_turn_observed: true,
                    review_thread_observed: true,
                    review_turn_observed: true,
                },
                ProviderDispatchPresence {
                    worker_thread: true,
                    worker_turn: true,
                    review_thread: true,
                    review_turn: false,
                },
            ),
        ] {
            assert!(verify_provider_dispatch_presence(progress, presence).is_err());
        }

        for broken in [
            ProviderDispatchPresence {
                worker_turn: true,
                ..no_claims
            },
            ProviderDispatchPresence {
                review_thread: true,
                ..no_claims
            },
            ProviderDispatchPresence {
                review_turn: true,
                ..no_claims
            },
        ] {
            assert!(verify_provider_dispatch_presence(none, broken).is_err());
        }
    }

    #[test]
    fn review_dispatch_subject_preimage_rejects_command_and_anchor_substitution() {
        let artifact = review_lifecycle("THREAD_START_ACCEPTED", "review-thread-exact");
        let exact = ManagedVerificationRequest::new(
            digest('1'),
            digest('2'),
            digest('3'),
            digest('4'),
            digest('5'),
            digest('6'),
            digest('7'),
            &artifact,
        )
        .expect("exact request");
        let substituted_command = ManagedVerificationRequest::new(
            digest('1'),
            digest('8'),
            digest('3'),
            digest('4'),
            digest('5'),
            digest('6'),
            digest('7'),
            &artifact,
        )
        .expect("self-consistent substituted request");
        let terminal_payload = digest('9');
        let terminal_evidence = digest('a');
        let exact_anchor = digest('b');
        let substituted_anchor = digest('c');

        let exact_preimage = managed_review_dispatch_operation_digests(
            &terminal_payload,
            &terminal_evidence,
            &exact,
            Some(&exact_anchor),
        );
        let command_tamper = managed_review_dispatch_operation_digests(
            &terminal_payload,
            &terminal_evidence,
            &substituted_command,
            Some(&exact_anchor),
        );
        let anchor_tamper = managed_review_dispatch_operation_digests(
            &terminal_payload,
            &terminal_evidence,
            &exact,
            Some(&substituted_anchor),
        );
        assert_ne!(exact_preimage, command_tamper);
        assert_ne!(exact_preimage, anchor_tamper);
    }

    #[test]
    fn only_exact_provider_kinds_share_the_attempt_ledger_event() {
        let expected = exact_provider_replay();
        verify_replay_identities(&expected, &expected).expect("legal subordinate sharing");

        let mut unrelated = expected.clone();
        unrelated[2].record_kind = "ARTIFACT_REFERENCE".to_owned();
        unrelated[2].record_ordinal = 1;
        assert!(verify_replay_identities(&unrelated, &unrelated).is_err());

        let mut substituted_event = expected.clone();
        substituted_event[2].ledger_event_digest = digest('c');
        assert!(verify_replay_identities(&substituted_event, &substituted_event).is_err());

        let mut wrong_order = expected.clone();
        wrong_order.swap(1, 2);
        assert!(verify_replay_identities(&wrong_order, &expected).is_err());
    }

    #[test]
    fn first_attempt_ids_are_globally_task_bound_and_exactly_replayable() {
        let first = managed_attempt_id(&digest('a'), 1).expect("first task attempt");
        let first_replay = managed_attempt_id(&digest('a'), 1).expect("exact retry");
        let second_task = managed_attempt_id(&digest('b'), 1).expect("second task attempt");
        let repair = managed_attempt_id(&digest('a'), 2).expect("repair attempt");

        assert_eq!(first, first_replay);
        assert_ne!(first, second_task);
        assert_ne!(first, repair);
        assert!(first.as_str().len() <= 128);
        assert!(second_task.as_str().len() <= 128);
    }

    #[test]
    fn prestart_closure_accepts_only_closed_canonical_blockers() {
        assert_eq!(
            prestart_closure_blocker_shape("LATTICE_MANAGED_EXECUTION_AUTHORITY_NOT_CURRENT")
                .expect("authority blocker"),
            ("TASK_BOUND_EXECUTION_AUTHORITY_NOT_CURRENT", false)
        );
        assert_eq!(
            prestart_closure_blocker_shape("LATTICE_MANAGED_MODEL_UNAVAILABLE")
                .expect("model blocker"),
            (
                "SELECTED_ALLOWLISTED_MODEL_UNAVAILABLE_NO_SUBSTITUTION",
                false
            )
        );
        assert_eq!(
            prestart_closure_blocker_shape(
                "LATTICE_MANAGED_MODEL_PROBE_TIMEOUT_RECONCILIATION_REQUIRED"
            )
            .expect("worker probe timeout blocker"),
            (
                "WORKER_MODEL_PROBE_TIMED_OUT_EXACT_PRESTART_SUBTREE_REAPED",
                false
            )
        );
        assert!(
            prestart_closure_blocker_shape(
                "LATTICE_MANAGED_REVIEW_MODEL_PROBE_TIMEOUT_NO_PROVIDER_EFFECT"
            )
            .is_err(),
            "review probe timeout follows the completed-worker closure path"
        );
        assert!(
            prestart_closure_blocker_shape("LATTICE_MANAGED_DISPATCH_RECONCILIATION_REQUIRED")
                .is_err()
        );
    }

    #[test]
    fn retained_closure_keeps_the_original_blocker_and_binds_typed_no_effect_proof() {
        assert_eq!(
            retained_prestart_closure_blocker_shape(
                "LATTICE_MANAGED_PROCESS_EXIT_WITHOUT_TERMINAL"
            )
            .expect("retained blocker"),
            ("PROVIDER_PROCESS_EXITED_WITHOUT_EXACT_TURN_TERMINAL", false)
        );
        assert!(
            retained_prestart_closure_blocker_shape(
                "LATTICE_MANAGED_REVIEW_RECONCILIATION_REQUIRED"
            )
            .is_err()
        );

        let task_ref = digest('1');
        let blocker = digest('2');
        assert!(
            retained_no_effect_proof_bytes(
                &task_ref,
                1,
                &blocker,
                &ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
                    worker_thread_claimed: true,
                },
            )
            .is_err(),
            "a claimed provider thread can be only temporarily absent from bounded discovery"
        );

        let bytes = retained_no_effect_proof_bytes(
            &task_ref,
            1,
            &blocker,
            &ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
                worker_thread_claimed: false,
            },
        )
        .expect("bounded proof");
        let payload: serde_json::Value = serde_json::from_slice(&bytes).expect("proof json");
        assert_eq!(payload.as_object().expect("object").len(), 9);
        assert_eq!(
            payload["schema"],
            "lattice.managed-no-provider-effect-proof.v1"
        );
        assert_eq!(payload["task_ref"], task_ref.as_str());
        assert_eq!(payload["blocker_descriptor_digest"], blocker.as_str());
        assert_eq!(payload["proof_kind"], "PROVEN_NO_PROVIDER_CANDIDATE");
        assert_eq!(payload["worker_thread_claimed"], false);
        assert_eq!(payload["worker_turn_claimed"], false);

        let substituted = retained_no_effect_proof_bytes(
            &task_ref,
            1,
            &digest('3'),
            &ManagedPrestartNoEffectProof::ProvenNoProviderCandidate {
                worker_thread_claimed: false,
            },
        )
        .expect("substituted proof");
        assert_ne!(bytes, substituted);
    }

    #[test]
    fn reconciled_review_thread_supports_but_never_substitutes_the_original_anchor() {
        let original = review_lifecycle("THREAD_START_ACCEPTED", "review-thread-exact");
        let reconciled = review_lifecycle("THREAD_RECONCILED", "review-thread-exact");
        assert_eq!(
            select_review_turn_anchor(&[original.clone(), reconciled.clone()], &reconciled)
                .expect("exact replay anchor")
                .descriptor_digest(),
            original.descriptor_digest()
        );
        let substituted = review_lifecycle("THREAD_RECONCILED", "review-thread-substituted");
        assert!(select_review_turn_anchor(&[original, substituted.clone()], &substituted).is_err());
    }

    #[test]
    fn pointer_and_git_commit_commitments_are_exact() {
        assert_eq!(
            pointer_content(
                &format!("worktree:sha256:{}", digest('b').as_str()),
                "worktree"
            )
            .expect("pointer"),
            digest('b')
        );
        assert!(pointer_content(digest('b').as_str(), "worktree").is_err());
        assert_eq!(
            sha256_bytes(b"5555555555555555555555555555555555555555").expect("digest"),
            sha256_bytes(b"5555555555555555555555555555555555555555").expect("digest")
        );
    }

    #[test]
    fn git_snapshot_environment_binding_is_exact_and_native_null_is_explicit() {
        let environment_ref = format!("execution-environment:sha256:{}", "a".repeat(64));
        let descriptor_digest = "b".repeat(64);
        let exact = serde_json::json!({
            "execution_environment_ref": environment_ref,
            "execution_environment_descriptor_digest": descriptor_digest,
        });
        let exact_object = exact.as_object().expect("snapshot object");
        assert!(snapshot_execution_environment_matches(
            exact_object,
            Some((environment_ref.as_str(), descriptor_digest.as_str())),
        ));
        assert!(!snapshot_execution_environment_matches(
            exact_object,
            Some((environment_ref.as_str(), &"c".repeat(64))),
        ));
        assert!(!snapshot_execution_environment_matches(exact_object, None));

        let native = serde_json::json!({
            "execution_environment_ref": null,
            "execution_environment_descriptor_digest": null,
        });
        let native_object = native.as_object().expect("native snapshot object");
        assert!(snapshot_execution_environment_matches(native_object, None));
        assert!(!snapshot_execution_environment_matches(
            native_object,
            Some((environment_ref.as_str(), descriptor_digest.as_str())),
        ));
    }

    #[test]
    fn wsl_claim_persistence_plan_records_the_exact_descriptor_before_capacity_claim() {
        let native = terminal_repair_packet(&digest('1'), &digest('2'));
        assert_eq!(
            attempt_claim_persistence_steps(&native, None).expect("native claim plan"),
            &[AttemptClaimPersistenceStep::ClaimCapacity]
        );

        let environment_ref = format!("execution-environment:sha256:{}", digest('3').as_str());
        let wsl = terminal_repair_packet(&digest('1'), &digest('2'))
            .with_execution_environment_ref(&environment_ref)
            .expect("WSL-bound packet");
        assert_eq!(
            attempt_claim_persistence_steps(&wsl, Some(&environment_ref))
                .expect("typed WSL claim plan"),
            &[
                AttemptClaimPersistenceStep::RecordExecutionEnvironment,
                AttemptClaimPersistenceStep::ClaimCapacity,
            ]
        );

        let substituted_ref = format!("execution-environment:sha256:{}", digest('4').as_str());
        for rejected in [
            attempt_claim_persistence_steps(&wsl, None),
            attempt_claim_persistence_steps(&wsl, Some(&substituted_ref)),
            attempt_claim_persistence_steps(&native, Some(&environment_ref)),
        ] {
            assert_eq!(
                rejected
                    .expect_err("missing or substituted descriptor/ref must fail closed")
                    .code(),
                "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION"
            );
        }
    }

    #[test]
    fn reservation_exact_replay_accepts_one_active_or_pending_record_without_duplication() {
        assert_eq!(
            attempt_reservation_replay_source(
                ClaimReservationDisposition::Reserved,
                Some(true),
                0,
            )
            .expect("new reservation remains pending"),
            AttemptReservationReplaySource::Pending,
        );
        assert_eq!(
            attempt_reservation_replay_source(
                ClaimReservationDisposition::ExactReplay,
                Some(true),
                0,
            )
            .expect("pending reservation exact replay"),
            AttemptReservationReplaySource::Pending,
        );
        assert_eq!(
            attempt_reservation_replay_source(ClaimReservationDisposition::ExactReplay, None, 1)
                .expect("consumed reservation exact active replay"),
            AttemptReservationReplaySource::Active,
        );
        for rejected in [
            attempt_reservation_replay_source(ClaimReservationDisposition::Reserved, None, 1),
            attempt_reservation_replay_source(ClaimReservationDisposition::ExactReplay, None, 0),
            attempt_reservation_replay_source(
                ClaimReservationDisposition::ExactReplay,
                Some(true),
                1,
            ),
            attempt_reservation_replay_source(
                ClaimReservationDisposition::ExactReplay,
                Some(false),
                1,
            ),
        ] {
            assert_eq!(
                rejected
                    .expect_err("ambiguous or substituted reservation replay must fail closed")
                    .code(),
                "LATTICE_MANAGED_ATTEMPT_RESERVATION_RECONCILE_REQUIRED",
            );
        }
    }

    #[test]
    fn production_wsl_claim_keeps_the_pending_crash_window_replayable_and_provider_fenced() {
        let source = include_str!("managed_repository.rs");
        let reserve = source
            .split("pub(crate) fn reserve_attempt(")
            .nth(1)
            .expect("production reservation")
            .split("pub(crate) fn record_restart_writer_blocker_atomically(")
            .next()
            .expect("reservation boundary");
        let durable_reservation = reserve
            .find(".reserve_worker_attempt_with_execution_environment_ref(")
            .expect("durable pending reservation");
        let pending_reload = reserve
            .find(".load_pending_worker_attempt(")
            .expect("pending reload");
        let fresh_replay = reserve
            .find("self.fresh_runtime()")
            .expect("fresh pending replay");
        let replay_source = reserve
            .find("attempt_reservation_replay_source(")
            .expect("reservation disposition selects pending or active replay");
        assert!(
            durable_reservation < pending_reload
                && pending_reload < fresh_replay
                && fresh_replay < replay_source
        );

        let claim = source
            .split("fn claim_attempt(")
            .nth(1)
            .expect("production claim")
            .split("fn record_observation(")
            .next()
            .expect("claim boundary");
        let reserve_pending = claim
            .find("self.reserve_attempt(binding, packet)")
            .expect("reserve pending before claim");
        let descriptor_plan = claim
            .find("attempt_claim_persistence_steps(")
            .expect("typed descriptor plan");
        let record_descriptor = claim
            .find(".record_execution_environment(")
            .expect("typed descriptor persistence");
        let capacity_claim = claim
            .find(".claim_worker_attempt_with_execution_environment_ref(")
            .expect("capacity claim");
        let claimed_replay = claim
            .rfind("self.fresh_runtime()")
            .expect("fresh claimed replay");
        assert!(
            reserve_pending < descriptor_plan
                && descriptor_plan < record_descriptor
                && record_descriptor < capacity_claim
                && capacity_claim < claimed_replay
        );

        let provider = source
            .split("fn claim_worker_thread_dispatch(")
            .nth(1)
            .expect("worker provider dispatch")
            .split("fn claim_worker_turn_dispatch(")
            .next()
            .expect("worker dispatch boundary");
        let pending_guard = provider
            .find("pending_attempt.as_ref() == Some(attempt)")
            .expect("pending provider fence");
        let provider_claim = provider
            .find(".claim_provider_dispatch(")
            .expect("provider effect claim");
        assert!(pending_guard < provider_claim);
    }

    #[test]
    fn pending_wsl_closure_requires_an_exact_durable_or_configured_descriptor() {
        let environment_ref = format!("execution-environment:sha256:{}", digest('5').as_str());
        let substituted_ref = format!("execution-environment:sha256:{}", digest('6').as_str());
        assert_eq!(
            pending_execution_environment_persistence(
                lattice_postgres_foreman::NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
                None,
                None,
            )
            .expect("native pending closure needs no descriptor row"),
            PendingExecutionEnvironmentPersistence::NativeWindows
        );
        assert_eq!(
            pending_execution_environment_persistence(
                &environment_ref,
                None,
                Some(&environment_ref)
            )
            .expect("record the configured crash-window descriptor"),
            PendingExecutionEnvironmentPersistence::RecordConfigured
        );
        assert_eq!(
            pending_execution_environment_persistence(
                &environment_ref,
                Some(&environment_ref),
                None
            )
            .expect("fresh process reuses the durable descriptor"),
            PendingExecutionEnvironmentPersistence::DurableExact
        );
        assert_eq!(
            pending_execution_environment_persistence(
                &environment_ref,
                Some(&environment_ref),
                Some(&environment_ref),
            )
            .expect("configured and durable descriptor are exact"),
            PendingExecutionEnvironmentPersistence::DurableExact
        );

        for rejected in [
            pending_execution_environment_persistence(&environment_ref, None, None),
            pending_execution_environment_persistence(
                &environment_ref,
                None,
                Some(&substituted_ref),
            ),
            pending_execution_environment_persistence(
                &environment_ref,
                Some(&substituted_ref),
                Some(&environment_ref),
            ),
            pending_execution_environment_persistence(
                &environment_ref,
                Some(&environment_ref),
                Some(&substituted_ref),
            ),
            pending_execution_environment_persistence(
                lattice_postgres_foreman::NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
                None,
                Some(&environment_ref),
            ),
        ] {
            assert_eq!(
                rejected
                    .expect_err("missing or substituted closure descriptor must fail closed")
                    .code(),
                "LATTICE_MANAGED_EXECUTION_ENVIRONMENT_SUBSTITUTION"
            );
        }
    }

    #[test]
    fn pending_wsl_closure_records_the_descriptor_before_artifact_or_close_writes() {
        let source = include_str!("managed_repository.rs");
        let construction = source
            .split("fn new_with_recovery(")
            .nth(1)
            .expect("repository construction")
            .split("pub(crate) fn with_execution_environment(")
            .next()
            .expect("construction boundary");
        assert!(construction.contains("repository.load_runtime_unreconciled()?"));
        assert!(!construction.contains("repository.load_runtime()?"));
        let install = source
            .split("pub(crate) fn with_execution_environment(")
            .nth(1)
            .expect("execution environment install")
            .split("pub fn load_replay_projection(")
            .next()
            .expect("environment install boundary");
        let configured = install
            .find("self.execution_environment = descriptor")
            .expect("configured descriptor install");
        let first_recovery = install
            .find("self.load_runtime()")
            .expect("deferred staged artifact recovery");
        assert!(configured < first_recovery);

        let recovery = source
            .split("fn recover_staged_artifact(")
            .nth(1)
            .expect("staged artifact recovery")
            .split("fn fresh_runtime(")
            .next()
            .expect("recovery boundary");
        let recovery_descriptor = recovery
            .find("ensure_pending_execution_environment_for_closure(")
            .expect("recovery descriptor guard");
        let recovery_ledger = recovery
            .find("self.execute_ledger(")
            .expect("recovery Ledger append");
        let recovery_close = recovery
            .find(".close_pending_worker_attempt(")
            .expect("recovery pending close");
        assert!(recovery_descriptor < recovery_ledger && recovery_ledger < recovery_close);

        let record = source
            .split("fn record_artifact(")
            .nth(1)
            .expect("production artifact record")
            .split("fn record_verification(")
            .next()
            .expect("artifact boundary");
        let record_descriptor = record
            .find("ensure_pending_execution_environment_for_closure(")
            .expect("record descriptor guard");
        let stage = record
            .find(".stage_artifact_reference(")
            .expect("artifact stage");
        let record_close = record
            .find(".close_pending_worker_attempt(")
            .expect("record pending close");
        assert!(record_descriptor < stage && stage < record_close);
    }

    #[test]
    fn expired_authority_window_is_historical_only() {
        assert!(
            !authority_window_is_current("2025-01-01T00:00:00Z", "2025-01-01T00:15:00Z",)
                .expect("valid historical window")
        );
    }
}
