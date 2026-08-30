//! Pure managed-task lineage and Task Ledger child-record semantics.
//!
//! This module hashes and verifies persistence-shaped records. It performs no
//! I/O, schedules no work, and never interprets a child record as Task Domain
//! state or authority.

use std::collections::{BTreeMap, BTreeSet};

use lattice_contracts::{AttemptId, ContentDigest, TaskLedgerStreamHead, TaskLedgerSubjectKind};
pub use lattice_foreman_state::{ModelReason, ReasoningEffort, WorkerModel};

use super::{ActionId, ActorId};
use super::{
    AppendCommand, CommandId, CorrelationId, LedgerAppendPlan, LedgerError, LedgerEvent,
    LedgerEventKind, LedgerOutcome, ReasonCode, TaskCreatedProfile, TaskSubmissionEnvelope,
    VerifiedStream, canonicalize, classify_task_created_profile, digest_value,
    hash_value_at_version, is_zero_digest, object, optional, plan_append, recognized_secret_text,
    text, unsigned, valid_identifier, validate_utc_timestamp,
};

/// Persistence row schema for one immutable intake-to-TaskSpec linkage.
pub const TASK_EXECUTION_BINDING_RECORD_SCHEMA: &str =
    "lattice.task-ledger.task-execution-binding-record/1.0";
/// Canonical payload schema committed by the binding Ledger event.
pub const TASK_EXECUTION_BINDING_PAYLOAD_SCHEMA: &str =
    "lattice.task-ledger.task-execution-binding/1.0";

const TASK_EXECUTION_BINDING_HASH_DOMAIN: &str = "lattice.task-ledger.task-execution-binding";
const TASK_RUNTIME_HASH_VERSION: &str = "1.0";
const BINDING_ACTION: &str = "RECORD_TASK_EXECUTION_BINDING_V1";
const BINDING_REASON: &str = "TASK_EXECUTION_BINDING_RECORDED";
/// Persistence row schema for one worker-attempt claim.
pub const WORKER_ATTEMPT_RECORD_SCHEMA: &str = "lattice.task-ledger.worker-attempt-record/1.0";
/// Persistence row schema for one exact worker lifecycle observation.
pub const WORKER_OBSERVATION_RECORD_SCHEMA: &str =
    "lattice.task-ledger.worker-observation-record/1.1";
/// Persistence row schema for one independent verification result.
pub const TASK_VERIFICATION_RECORD_SCHEMA: &str =
    "lattice.task-ledger.task-verification-record/1.0";

const WORKER_ATTEMPT_PAYLOAD_SCHEMA: &str = "lattice.task-ledger.worker-attempt/1.0";
const WORKER_OBSERVATION_PAYLOAD_SCHEMA: &str = "lattice.task-ledger.worker-observation/1.1";
const WORKER_OBSERVATION_HASH_VERSION: &str = "1.1";
const TASK_VERIFICATION_PAYLOAD_SCHEMA: &str = "lattice.task-ledger.task-verification/1.0";
const WORKER_ATTEMPT_HASH_DOMAIN: &str = "lattice.task-ledger.worker-attempt";
const NO_PROVIDER_EFFECT_PREDECESSOR_HASH_DOMAIN: &str =
    "lattice.task-ledger.no-provider-effect-predecessor";
const NO_PROVIDER_EFFECT_PREDECESSOR_SCHEMA: &str =
    "lattice.task-ledger.no-provider-effect-predecessor/1.0";
/// Closed owner profile allowed to attest that a predecessor attempt produced
/// no provider effect and is therefore safe to follow without a fake terminal.
pub const NO_PROVIDER_EFFECT_CLOSURE_OWNER: &str = "foreman-execution/attempt-closure-v1";
const WORKER_OBSERVATION_HASH_DOMAIN: &str = "lattice.task-ledger.worker-observation";
const TASK_VERIFICATION_HASH_DOMAIN: &str = "lattice.task-ledger.task-verification";
const WORKER_ATTEMPT_ACTION: &str = "DISPATCH_WORKER_ATTEMPT_V1";
const WORKER_ATTEMPT_REASON: &str = "WORKER_ATTEMPT_CLAIMED";
const WORKER_OBSERVATION_ACTION: &str = "RECORD_WORKER_OBSERVATION_V1";
const TASK_VERIFICATION_ACTION: &str = "RECORD_TASK_VERIFICATION_V1";
const APPROVAL_EVIDENCE_ACTION: &str = "RECORD_APPROVAL_EVIDENCE_V1";
const APPROVAL_EVIDENCE_REASON: &str = "APPROVAL_EVIDENCE_RECORDED";
const ARTIFACT_REFERENCE_ACTION: &str = "RECORD_ARTIFACT_REFERENCE_V1";
const ARTIFACT_REFERENCE_REASON: &str = "ARTIFACT_REFERENCE_RECORDED";
const TASK_VERIFICATION_PASSED_REASON: &str = "TASK_VERIFICATION_PASSED";
const TASK_VERIFICATION_FAILED_REASON: &str = "TASK_VERIFICATION_FAILED";

/// Caller-supplied metadata for one pure managed-task Ledger append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRuntimeAppendMetadata {
    command_id: CommandId,
    correlation_id: CorrelationId,
    occurred_at: String,
}

impl TaskRuntimeAppendMetadata {
    /// Constructs metadata without reading a clock.
    ///
    /// # Errors
    ///
    /// Rejects a non-canonical UTC timestamp.
    pub fn new(
        command_id: CommandId,
        correlation_id: CorrelationId,
        occurred_at: impl Into<String>,
    ) -> Result<Self, LedgerError> {
        let occurred_at = occurred_at.into();
        validate_utc_timestamp(&occurred_at)?;
        Ok(Self {
            command_id,
            correlation_id,
            occurred_at,
        })
    }

    /// Returns the stable command identity.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }
}

/// Immutable external commitments captured by one promotion.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct TaskExecutionBindingInput {
    approval_subject_digest: ContentDigest,
    budget_digest: ContentDigest,
    verification_policy_digest: ContentDigest,
}

impl TaskExecutionBindingInput {
    /// Constructs the closed promotion commitments.
    ///
    /// # Errors
    ///
    /// Rejects a zero approval, budget, or verification commitment.
    pub fn new(
        approval_subject_digest: ContentDigest,
        budget_digest: ContentDigest,
        verification_policy_digest: ContentDigest,
    ) -> Result<Self, LedgerError> {
        if [
            &approval_subject_digest,
            &budget_digest,
            &verification_policy_digest,
        ]
        .into_iter()
        .any(is_zero_digest)
        {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        Ok(Self {
            approval_subject_digest,
            budget_digest,
            verification_policy_digest,
        })
    }
}

/// Exact Ledger event and request linkage shared by managed-task child rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskRuntimeEventLink {
    expected_head: TaskLedgerStreamHead,
    stream_id: ContentDigest,
    event_sequence: u64,
    event_digest: ContentDigest,
    command_id: CommandId,
    request_digest: ContentDigest,
    payload_digest: ContentDigest,
}

impl TaskRuntimeEventLink {
    /// Constructs one explicitly untrusted persistence linkage.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        expected_head: TaskLedgerStreamHead,
        stream_id: ContentDigest,
        event_sequence: u64,
        event_digest: ContentDigest,
        command_id: CommandId,
        request_digest: ContentDigest,
        payload_digest: ContentDigest,
    ) -> Self {
        Self {
            expected_head,
            stream_id,
            event_sequence,
            event_digest,
            command_id,
            request_digest,
            payload_digest,
        }
    }

    /// Returns the exact pre-append stream head.
    #[must_use]
    pub const fn expected_head(&self) -> &TaskLedgerStreamHead {
        &self.expected_head
    }

    /// Returns the linked stream ID.
    #[must_use]
    pub const fn stream_id(&self) -> &ContentDigest {
        &self.stream_id
    }

    /// Returns the linked event sequence.
    #[must_use]
    pub const fn event_sequence(&self) -> u64 {
        self.event_sequence
    }

    /// Returns the linked event digest.
    #[must_use]
    pub const fn event_digest(&self) -> &ContentDigest {
        &self.event_digest
    }

    /// Returns the linked command ID.
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }

    /// Returns the linked request digest.
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }

    /// Returns the canonical child payload digest.
    #[must_use]
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaskExecutionBindingPayload {
    task_ref: ContentDigest,
    intake_stream_id: ContentDigest,
    intake_event_digest: ContentDigest,
    project_authority_receipt_digest: ContentDigest,
    successor_stream_id: ContentDigest,
    successor_task_created_event_digest: ContentDigest,
    task_spec_digest: ContentDigest,
    approval_subject_digest: ContentDigest,
    budget_digest: ContentDigest,
    verification_policy_digest: ContentDigest,
    binding_digest: ContentDigest,
}

/// Verified immutable intake-to-TaskSpec linkage child record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTaskExecutionBinding {
    link: TaskRuntimeEventLink,
    payload: TaskExecutionBindingPayload,
}

impl VerifiedTaskExecutionBinding {
    /// Returns the Ledger event linkage.
    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }

    /// Returns the stable public task reference.
    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.payload.task_ref
    }

    /// Returns the immutable general-intake stream ID.
    #[must_use]
    pub const fn intake_stream_id(&self) -> &ContentDigest {
        &self.payload.intake_stream_id
    }

    /// Returns the exact intake `TASK_CREATED` event digest.
    #[must_use]
    pub const fn intake_event_digest(&self) -> &ContentDigest {
        &self.payload.intake_event_digest
    }

    /// Returns the Project Registry authority receipt captured at intake.
    #[must_use]
    pub const fn project_authority_receipt_digest(&self) -> &ContentDigest {
        &self.payload.project_authority_receipt_digest
    }

    /// Returns the unique executable successor stream ID.
    #[must_use]
    pub const fn successor_stream_id(&self) -> &ContentDigest {
        &self.payload.successor_stream_id
    }

    /// Returns the successor `TASK_CREATED` event digest.
    #[must_use]
    pub const fn successor_task_created_event_digest(&self) -> &ContentDigest {
        &self.payload.successor_task_created_event_digest
    }

    /// Returns the complete `TaskSpec` digest.
    #[must_use]
    pub const fn task_spec_digest(&self) -> &ContentDigest {
        &self.payload.task_spec_digest
    }

    /// Returns the exact execution-approval subject commitment.
    #[must_use]
    pub const fn approval_subject_digest(&self) -> &ContentDigest {
        &self.payload.approval_subject_digest
    }

    /// Returns the immutable budget digest.
    #[must_use]
    pub const fn budget_digest(&self) -> &ContentDigest {
        &self.payload.budget_digest
    }

    /// Returns the closed verification-policy commitment.
    #[must_use]
    pub const fn verification_policy_digest(&self) -> &ContentDigest {
        &self.payload.verification_policy_digest
    }

    /// Returns the canonical binding digest committed by the Ledger event.
    #[must_use]
    pub const fn binding_digest(&self) -> &ContentDigest {
        &self.payload.binding_digest
    }

    /// Returns the payload digest committed by the exact Ledger event.
    #[must_use]
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload.binding_digest
    }

    /// Returns canonical payload bytes for a reflection-free live adapter.
    ///
    /// # Errors
    ///
    /// Propagates canonical encoding failure.
    pub fn payload_canonical_bytes(&self) -> Result<Vec<u8>, LedgerError> {
        canonicalize(&task_execution_binding_value(&self.payload))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(LedgerError::from)
    }

    /// Exports an explicitly untrusted persistence row.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedTaskExecutionBinding {
        UntrustedTaskExecutionBinding {
            record_schema: TASK_EXECUTION_BINDING_RECORD_SCHEMA.to_owned(),
            link: self.link.clone(),
            task_ref: self.payload.task_ref.clone(),
            intake_stream_id: self.payload.intake_stream_id.clone(),
            intake_event_digest: self.payload.intake_event_digest.clone(),
            project_authority_receipt_digest: self.payload.project_authority_receipt_digest.clone(),
            successor_stream_id: self.payload.successor_stream_id.clone(),
            successor_task_created_event_digest: self
                .payload
                .successor_task_created_event_digest
                .clone(),
            task_spec_digest: self.payload.task_spec_digest.clone(),
            approval_subject_digest: self.payload.approval_subject_digest.clone(),
            budget_digest: self.payload.budget_digest.clone(),
            verification_policy_digest: self.payload.verification_policy_digest.clone(),
            binding_digest: self.payload.binding_digest.clone(),
        }
    }
}

/// Explicitly untrusted persisted promotion linkage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedTaskExecutionBinding {
    record_schema: String,
    link: TaskRuntimeEventLink,
    task_ref: ContentDigest,
    intake_stream_id: ContentDigest,
    intake_event_digest: ContentDigest,
    project_authority_receipt_digest: ContentDigest,
    successor_stream_id: ContentDigest,
    successor_task_created_event_digest: ContentDigest,
    task_spec_digest: ContentDigest,
    approval_subject_digest: ContentDigest,
    budget_digest: ContentDigest,
    verification_policy_digest: ContentDigest,
    binding_digest: ContentDigest,
}

impl UntrustedTaskExecutionBinding {
    /// Constructs one untrusted row from persistence scalars.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_schema: impl Into<String>,
        link: TaskRuntimeEventLink,
        task_ref: ContentDigest,
        intake_stream_id: ContentDigest,
        intake_event_digest: ContentDigest,
        project_authority_receipt_digest: ContentDigest,
        successor_stream_id: ContentDigest,
        successor_task_created_event_digest: ContentDigest,
        task_spec_digest: ContentDigest,
        approval_subject_digest: ContentDigest,
        budget_digest: ContentDigest,
        verification_policy_digest: ContentDigest,
        binding_digest: ContentDigest,
    ) -> Self {
        Self {
            record_schema: record_schema.into(),
            link,
            task_ref,
            intake_stream_id,
            intake_event_digest,
            project_authority_receipt_digest,
            successor_stream_id,
            successor_task_created_event_digest,
            task_spec_digest,
            approval_subject_digest,
            budget_digest,
            verification_policy_digest,
            binding_digest,
        }
    }

    /// Returns a copy with a substituted budget commitment for tamper tests.
    #[must_use]
    pub fn with_budget_digest(mut self, budget_digest: ContentDigest) -> Self {
        self.budget_digest = budget_digest;
        self
    }

    /// Returns a copy with a substituted record schema for compatibility tests.
    #[must_use]
    pub fn with_record_schema(mut self, record_schema: impl Into<String>) -> Self {
        self.record_schema = record_schema.into();
        self
    }
}

/// Pure promotion decision paired with the binding Ledger append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskExecutionBindingPlan {
    ledger_plan: LedgerAppendPlan,
    binding: VerifiedTaskExecutionBinding,
    new_binding: Option<VerifiedTaskExecutionBinding>,
}

impl TaskExecutionBindingPlan {
    /// Returns whether the exact command is a non-mutating replay.
    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.ledger_plan.is_exact_retry()
    }

    /// Returns the Task Ledger append plan.
    #[must_use]
    pub const fn ledger_plan(&self) -> &LedgerAppendPlan {
        &self.ledger_plan
    }

    /// Returns the retained or newly planned binding.
    #[must_use]
    pub const fn binding(&self) -> &VerifiedTaskExecutionBinding {
        &self.binding
    }

    /// Returns the persistence row that still needs an owner extension write.
    /// An exact Ledger retry returns `Some` only while repairing a missing
    /// extension row; a fully retained retry returns `None`.
    #[must_use]
    pub const fn new_binding(&self) -> Option<&VerifiedTaskExecutionBinding> {
        self.new_binding.as_ref()
    }
}

/// Plans one immutable general-intake to `TaskSpec` successor linkage.
///
/// # Errors
///
/// Rejects non-create-only intake, project/task substitution, malformed
/// successor creation, duplicate rows, changed immutable commitments, or a
/// changed command retry.
#[allow(clippy::needless_pass_by_value)]
pub fn plan_task_execution_binding(
    intake: &VerifiedStream,
    successor: &VerifiedStream,
    submission: &TaskSubmissionEnvelope,
    existing: &[VerifiedTaskExecutionBinding],
    metadata: TaskRuntimeAppendMetadata,
    input: TaskExecutionBindingInput,
) -> Result<TaskExecutionBindingPlan, LedgerError> {
    validate_lineage(intake, successor, submission)?;
    if existing.len() > 1 {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let retained = match existing {
        [] => None,
        [retained] => {
            let verified = verify_untrusted_task_execution_binding(
                intake,
                successor,
                submission,
                &retained.to_untrusted(),
            )?;
            Some(verified)
        }
        _ => return Err(LedgerError::InvalidTaskRuntimeRecord),
    };
    let payload = build_binding_payload(intake, successor, submission, &input)?;
    if retained
        .as_ref()
        .is_some_and(|record| record.payload != payload)
    {
        return Err(LedgerError::TaskRuntimeSubstitution);
    }
    if retained
        .as_ref()
        .is_some_and(|record| record.link.command_id != metadata.command_id)
    {
        return Err(LedgerError::TaskRuntimeSubstitution);
    }
    let expected_head = retained.as_ref().map_or_else(
        || retained_command_expected_head(successor, &metadata.command_id),
        |record| record.link.expected_head.clone(),
    );
    let command = binding_append_command(expected_head, metadata, payload.binding_digest.clone())?;
    let ledger_plan = plan_append(successor, command)?;
    if let Some(retained) = retained {
        if !ledger_plan.is_exact_retry() {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        return Ok(TaskExecutionBindingPlan {
            ledger_plan,
            binding: retained,
            new_binding: None,
        });
    }
    if ledger_plan.is_exact_retry() {
        let event = retained_event_for_command(
            successor,
            BINDING_ACTION,
            ledger_plan.receipt().command_id(),
        )?;
        validate_binding_event_shape(event, &payload.binding_digest)?;
        let command = successor
            .commands()
            .iter()
            .find(|record| record.request().command_id() == event.command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        let binding = VerifiedTaskExecutionBinding {
            link: link_from_event(command.request().expected_head().clone(), event),
            payload,
        };
        let binding = verify_untrusted_task_execution_binding(
            intake,
            successor,
            submission,
            &binding.to_untrusted(),
        )?;
        return Ok(TaskExecutionBindingPlan {
            ledger_plan,
            binding: binding.clone(),
            new_binding: Some(binding),
        });
    }
    ensure_no_binding_event(successor)?;
    let event = ledger_plan
        .new_event()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    validate_binding_event_shape(event, &payload.binding_digest)?;
    let binding = VerifiedTaskExecutionBinding {
        link: link_from_event(successor.head().clone(), event),
        payload,
    };
    Ok(TaskExecutionBindingPlan {
        ledger_plan,
        binding: binding.clone(),
        new_binding: Some(binding),
    })
}

/// Classifies whether the verified successor contains its one formal
/// intake-to-TaskSpec binding event.
///
/// This is a presence check for restart routing, not a replacement for
/// [`verify_untrusted_task_execution_binding`]. A present binding still needs
/// its exact owner-extension row before it can be used. Any duplicate binding
/// or managed child event without a preceding binding fails closed.
///
/// # Errors
///
/// Invalid lineage, a malformed or duplicate binding event, a missing command
/// record, or an orphan managed-runtime child event is rejected.
pub fn task_execution_binding_is_recorded(
    intake: &VerifiedStream,
    successor: &VerifiedStream,
    submission: &TaskSubmissionEnvelope,
) -> Result<bool, LedgerError> {
    validate_lineage(intake, successor, submission)?;
    let events = binding_events(successor);
    match events.as_slice() {
        [] => {
            if successor
                .events()
                .iter()
                .any(|event| is_managed_runtime_child_action(event.action().as_str()))
            {
                return Err(LedgerError::InvalidTaskRuntimeRecord);
            }
            Ok(false)
        }
        [event] => {
            validate_binding_event_shape(event, event.subject_digest())?;
            if successor
                .commands()
                .iter()
                .filter(|record| record.request().command_id() == event.command_id())
                .count()
                != 1
            {
                return Err(LedgerError::InvalidTaskRuntimeRecord);
            }
            Ok(true)
        }
        _ => Err(LedgerError::InvalidTaskRuntimeRecord),
    }
}

/// Verifies one untrusted promotion row against both verified Ledger streams.
///
/// # Errors
///
/// Unknown schema, missing/duplicate event, changed linkage, digest drift, or
/// cross-project/task substitution fails closed.
pub fn verify_untrusted_task_execution_binding(
    intake: &VerifiedStream,
    successor: &VerifiedStream,
    submission: &TaskSubmissionEnvelope,
    row: &UntrustedTaskExecutionBinding,
) -> Result<VerifiedTaskExecutionBinding, LedgerError> {
    validate_lineage(intake, successor, submission)?;
    if row.record_schema != TASK_EXECUTION_BINDING_RECORD_SCHEMA {
        return Err(LedgerError::UnknownTaskRuntimeRecordVersion);
    }
    let events = binding_events(successor);
    if events.len() != 1 {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let event = events[0];
    let command = successor
        .commands()
        .iter()
        .find(|command| command.request().command_id() == event.command_id())
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    let input = TaskExecutionBindingInput::new(
        row.approval_subject_digest.clone(),
        row.budget_digest.clone(),
        row.verification_policy_digest.clone(),
    )?;
    let payload = build_binding_payload(intake, successor, submission, &input)?;
    validate_binding_event_shape(event, &payload.binding_digest)?;
    let expected_link = link_from_event(command.request().expected_head().clone(), event);
    if row.link != expected_link
        || row.task_ref != payload.task_ref
        || row.intake_stream_id != payload.intake_stream_id
        || row.intake_event_digest != payload.intake_event_digest
        || row.project_authority_receipt_digest != payload.project_authority_receipt_digest
        || row.successor_stream_id != payload.successor_stream_id
        || row.successor_task_created_event_digest != payload.successor_task_created_event_digest
        || row.task_spec_digest != payload.task_spec_digest
        || row.approval_subject_digest != payload.approval_subject_digest
        || row.budget_digest != payload.budget_digest
        || row.verification_policy_digest != payload.verification_policy_digest
        || row.binding_digest != payload.binding_digest
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(VerifiedTaskExecutionBinding {
        link: expected_link,
        payload,
    })
}

/// Immutable payload supplied when claiming one bounded worker attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerAttemptInput {
    attempt_id: AttemptId,
    attempt_number: u64,
    foreman_generation: u64,
    model: WorkerModel,
    reasoning: ReasoningEffort,
    model_reason: ModelReason,
    writer_fence: u64,
    foreman_checkpoint_digest: ContentDigest,
    approval_receipt_digest: ContentDigest,
    packet_digest: ContentDigest,
    worktree_digest: ContentDigest,
    base_commit_digest: ContentDigest,
    model_reason_digest: ContentDigest,
}

impl WorkerAttemptInput {
    /// Constructs a secret-free worker-attempt commitment.
    ///
    /// # Errors
    ///
    /// Rejects zero counters/digests or a malformed attempt identifier.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        attempt_id: AttemptId,
        attempt_number: u64,
        foreman_generation: u64,
        model: WorkerModel,
        reasoning: ReasoningEffort,
        model_reason: ModelReason,
        writer_fence: u64,
        foreman_checkpoint_digest: ContentDigest,
        approval_receipt_digest: ContentDigest,
        packet_digest: ContentDigest,
        worktree_digest: ContentDigest,
        base_commit_digest: ContentDigest,
        model_reason_digest: ContentDigest,
    ) -> Result<Self, LedgerError> {
        if attempt_number == 0
            || foreman_generation == 0
            || writer_fence == 0
            || !model_reason.is_allowed_for(model)
            || !valid_runtime_identifier(attempt_id.as_str())
            || [
                &foreman_checkpoint_digest,
                &approval_receipt_digest,
                &packet_digest,
                &worktree_digest,
                &base_commit_digest,
                &model_reason_digest,
            ]
            .into_iter()
            .any(is_zero_digest)
        {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        Ok(Self {
            attempt_id,
            attempt_number,
            foreman_generation,
            model,
            reasoning,
            model_reason,
            writer_fence,
            foreman_checkpoint_digest,
            approval_receipt_digest,
            packet_digest,
            worktree_digest,
            base_commit_digest,
            model_reason_digest,
        })
    }
}

/// Owner-verified, digest-bound proof that one exact predecessor attempt no
/// longer owns a provider effect even though it has no fabricated terminal.
///
/// Construction is deliberately tied to an already verified Task Ledger
/// binding and attempt. The Foreman repository remains responsible for
/// verifying the server-owned closure and its Artifact Store descriptors
/// before constructing this value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedNoProviderEffectPredecessor {
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    binding_digest: ContentDigest,
    predecessor_attempt_id: AttemptId,
    predecessor_attempt_number: u64,
    predecessor_writer_fence: u64,
    blocker_code: String,
    blocker_descriptor_digest: ContentDigest,
    reconciliation_proof_descriptor_digest: ContentDigest,
    successor_packet_digest: ContentDigest,
    digest: ContentDigest,
}

impl VerifiedNoProviderEffectPredecessor {
    /// Constructs the closed retry predecessor from one exact owner replay.
    ///
    /// # Errors
    ///
    /// Rejects any owner, task, attempt, fence, evidence, or packet
    /// substitution. The blocker and reconciliation proof must be distinct,
    /// non-zero Artifact Store descriptors.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        binding: &VerifiedTaskExecutionBinding,
        predecessor: &VerifiedWorkerAttemptRecord,
        owner_profile: &str,
        closure_task_ref: &ContentDigest,
        closure_attempt_id: &AttemptId,
        closure_attempt_number: u64,
        closure_writer_fence: u64,
        blocker_code: &str,
        blocker_descriptor_digest: ContentDigest,
        reconciliation_proof_descriptor_digest: ContentDigest,
        successor_packet_digest: ContentDigest,
    ) -> Result<Self, LedgerError> {
        if owner_profile != NO_PROVIDER_EFFECT_CLOSURE_OWNER
            || predecessor.task_ref() != binding.task_ref()
            || predecessor.successor_stream_id() != binding.successor_stream_id()
            || predecessor.binding_digest() != binding.binding_digest()
            || closure_task_ref != binding.task_ref()
            || closure_attempt_id != predecessor.attempt_id()
            || closure_attempt_number != predecessor.attempt_number()
            || closure_writer_fence != predecessor.writer_fence()
        {
            return Err(LedgerError::TaskRuntimeSubstitution);
        }
        if closure_attempt_number == 0
            || closure_attempt_number == u64::MAX
            || !valid_runtime_identifier(blocker_code)
            || is_zero_digest(&blocker_descriptor_digest)
            || is_zero_digest(&reconciliation_proof_descriptor_digest)
            || blocker_descriptor_digest == reconciliation_proof_descriptor_digest
            || is_zero_digest(&successor_packet_digest)
        {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        let mut verified = Self {
            task_ref: binding.task_ref().clone(),
            successor_stream_id: binding.successor_stream_id().clone(),
            binding_digest: binding.binding_digest().clone(),
            predecessor_attempt_id: predecessor.attempt_id().clone(),
            predecessor_attempt_number: predecessor.attempt_number(),
            predecessor_writer_fence: predecessor.writer_fence(),
            blocker_code: blocker_code.to_owned(),
            blocker_descriptor_digest,
            reconciliation_proof_descriptor_digest,
            successor_packet_digest,
            digest: binding.binding_digest().clone(),
        };
        verified.digest = no_provider_effect_predecessor_digest(&verified)?;
        Ok(verified)
    }

    /// Returns the canonical digest of the complete predecessor proof.
    #[must_use]
    pub const fn digest(&self) -> &ContentDigest {
        &self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerAttemptPayload {
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    task_spec_digest: ContentDigest,
    binding_digest: ContentDigest,
    budget_digest: ContentDigest,
    attempt_id: AttemptId,
    attempt_number: u64,
    foreman_generation: u64,
    model: WorkerModel,
    reasoning: ReasoningEffort,
    model_reason: ModelReason,
    writer_fence: u64,
    foreman_checkpoint_digest: ContentDigest,
    approval_receipt_digest: ContentDigest,
    packet_digest: ContentDigest,
    worktree_digest: ContentDigest,
    base_commit_digest: ContentDigest,
    model_reason_digest: ContentDigest,
    claimed_at: String,
    payload_digest: ContentDigest,
}

/// Verified worker-attempt child row bound to one `EFFECT_INTENT`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedWorkerAttemptRecord {
    link: TaskRuntimeEventLink,
    payload: WorkerAttemptPayload,
}

impl VerifiedWorkerAttemptRecord {
    /// Returns the exact event linkage.
    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.payload.task_ref
    }

    #[must_use]
    pub const fn successor_stream_id(&self) -> &ContentDigest {
        &self.payload.successor_stream_id
    }

    #[must_use]
    pub const fn task_spec_digest(&self) -> &ContentDigest {
        &self.payload.task_spec_digest
    }

    #[must_use]
    pub const fn binding_digest(&self) -> &ContentDigest {
        &self.payload.binding_digest
    }

    #[must_use]
    pub const fn budget_digest(&self) -> &ContentDigest {
        &self.payload.budget_digest
    }

    /// Returns the stable attempt ID.
    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.payload.attempt_id
    }

    /// Returns the exact monotonic attempt number.
    #[must_use]
    pub const fn attempt_number(&self) -> u64 {
        self.payload.attempt_number
    }

    #[must_use]
    pub const fn foreman_generation(&self) -> u64 {
        self.payload.foreman_generation
    }

    /// Returns the monotonic Writer fence observation.
    #[must_use]
    pub const fn writer_fence(&self) -> u64 {
        self.payload.writer_fence
    }

    /// Returns the selected model.
    #[must_use]
    pub const fn model(&self) -> WorkerModel {
        self.payload.model
    }

    #[must_use]
    pub const fn reasoning(&self) -> ReasoningEffort {
        self.payload.reasoning
    }

    /// Returns the closed, replayable routing reason for the selected model.
    #[must_use]
    pub const fn model_reason(&self) -> ModelReason {
        self.payload.model_reason
    }

    #[must_use]
    pub const fn foreman_checkpoint_digest(&self) -> &ContentDigest {
        &self.payload.foreman_checkpoint_digest
    }

    #[must_use]
    pub const fn approval_receipt_digest(&self) -> &ContentDigest {
        &self.payload.approval_receipt_digest
    }

    #[must_use]
    pub const fn packet_digest(&self) -> &ContentDigest {
        &self.payload.packet_digest
    }

    #[must_use]
    pub const fn worktree_digest(&self) -> &ContentDigest {
        &self.payload.worktree_digest
    }

    #[must_use]
    pub const fn base_commit_digest(&self) -> &ContentDigest {
        &self.payload.base_commit_digest
    }

    #[must_use]
    pub const fn model_reason_digest(&self) -> &ContentDigest {
        &self.payload.model_reason_digest
    }

    #[must_use]
    pub fn claimed_at(&self) -> &str {
        &self.payload.claimed_at
    }

    #[must_use]
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload.payload_digest
    }

    /// Returns canonical payload bytes for direct adapter persistence.
    ///
    /// # Errors
    ///
    /// Propagates canonical encoding failure.
    pub fn payload_canonical_bytes(&self) -> Result<Vec<u8>, LedgerError> {
        canonicalize(&worker_attempt_payload_value(&self.payload))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(LedgerError::from)
    }

    /// Exports an explicitly untrusted persistence row.
    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedWorkerAttemptRow {
        UntrustedWorkerAttemptRow {
            record_schema: WORKER_ATTEMPT_RECORD_SCHEMA.to_owned(),
            link: self.link.clone(),
            payload: self.payload.clone(),
        }
    }
}

/// Explicitly untrusted worker-attempt persistence row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedWorkerAttemptRow {
    record_schema: String,
    link: TaskRuntimeEventLink,
    payload: WorkerAttemptPayload,
}

impl UntrustedWorkerAttemptRow {
    /// Constructs one untrusted row from persistence scalars.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_schema: impl Into<String>,
        link: TaskRuntimeEventLink,
        task_ref: ContentDigest,
        successor_stream_id: ContentDigest,
        task_spec_digest: ContentDigest,
        binding_digest: ContentDigest,
        budget_digest: ContentDigest,
        attempt_id: AttemptId,
        attempt_number: u64,
        foreman_generation: u64,
        model: WorkerModel,
        reasoning: ReasoningEffort,
        model_reason: ModelReason,
        writer_fence: u64,
        foreman_checkpoint_digest: ContentDigest,
        approval_receipt_digest: ContentDigest,
        packet_digest: ContentDigest,
        worktree_digest: ContentDigest,
        base_commit_digest: ContentDigest,
        model_reason_digest: ContentDigest,
        claimed_at: impl Into<String>,
        payload_digest: ContentDigest,
    ) -> Self {
        Self {
            record_schema: record_schema.into(),
            link,
            payload: WorkerAttemptPayload {
                task_ref,
                successor_stream_id,
                task_spec_digest,
                binding_digest,
                budget_digest,
                attempt_id,
                attempt_number,
                foreman_generation,
                model,
                reasoning,
                model_reason,
                writer_fence,
                foreman_checkpoint_digest,
                approval_receipt_digest,
                packet_digest,
                worktree_digest,
                base_commit_digest,
                model_reason_digest,
                claimed_at: claimed_at.into(),
                payload_digest,
            },
        }
    }

    /// Returns a copy with a changed attempt number for tamper tests.
    #[must_use]
    pub fn with_attempt_number(mut self, attempt_number: u64) -> Self {
        self.payload.attempt_number = attempt_number;
        self
    }

    /// Returns a copy with a changed model routing reason for tamper tests.
    #[must_use]
    pub fn with_model_reason(mut self, model_reason: ModelReason) -> Self {
        self.payload.model_reason = model_reason;
        self
    }

    /// Returns a copy with a changed linked event digest for tamper tests.
    #[must_use]
    pub fn with_event_digest(mut self, event_digest: ContentDigest) -> Self {
        self.link.event_digest = event_digest;
        self
    }
}

/// Pure Ledger append paired with an optional new worker-attempt row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerAttemptAppendPlan {
    ledger_plan: LedgerAppendPlan,
    record: VerifiedWorkerAttemptRecord,
    new_record: Option<VerifiedWorkerAttemptRecord>,
}

impl WorkerAttemptAppendPlan {
    #[must_use]
    pub const fn ledger_plan(&self) -> &LedgerAppendPlan {
        &self.ledger_plan
    }

    #[must_use]
    pub const fn record(&self) -> &VerifiedWorkerAttemptRecord {
        &self.record
    }

    /// Returns the persistence row that still needs an owner extension write.
    /// An exact Ledger retry returns `Some` only while repairing a missing
    /// extension row; a fully retained retry returns `None`.
    #[must_use]
    pub const fn new_record(&self) -> Option<&VerifiedWorkerAttemptRecord> {
        self.new_record.as_ref()
    }
}

/// Closed exact worker lifecycle observation kinds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerObservationKind {
    ThreadAccepted,
    TurnAccepted,
    TurnStarted,
    PrestartTerminalFailed,
    MeaningfulProgress,
    Heartbeat,
    StallClassified,
    InterruptRequested,
    Reconciled,
    TerminalCompleted,
    TerminalFailed,
    TerminalInterrupted,
}

impl WorkerObservationKind {
    /// Returns the stable child-record value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ThreadAccepted => "THREAD_ACCEPTED",
            Self::TurnAccepted => "TURN_ACCEPTED",
            Self::TurnStarted => "TURN_STARTED",
            Self::PrestartTerminalFailed => "PRESTART_TERMINAL_FAILED",
            Self::MeaningfulProgress => "MEANINGFUL_PROGRESS",
            Self::Heartbeat => "HEARTBEAT",
            Self::StallClassified => "STALL_CLASSIFIED",
            Self::InterruptRequested => "INTERRUPT_REQUESTED",
            Self::Reconciled => "RECONCILED",
            Self::TerminalCompleted => "TERMINAL_COMPLETED",
            Self::TerminalFailed => "TERMINAL_FAILED",
            Self::TerminalInterrupted => "TERMINAL_INTERRUPTED",
        }
    }

    /// Parses one closed observation kind.
    ///
    /// # Errors
    ///
    /// Rejects unknown provider lifecycle values.
    pub fn parse(value: &str) -> Result<Self, LedgerError> {
        match value {
            "THREAD_ACCEPTED" => Ok(Self::ThreadAccepted),
            "TURN_ACCEPTED" => Ok(Self::TurnAccepted),
            "TURN_STARTED" => Ok(Self::TurnStarted),
            "PRESTART_TERMINAL_FAILED" => Ok(Self::PrestartTerminalFailed),
            "MEANINGFUL_PROGRESS" => Ok(Self::MeaningfulProgress),
            "HEARTBEAT" => Ok(Self::Heartbeat),
            "STALL_CLASSIFIED" => Ok(Self::StallClassified),
            "INTERRUPT_REQUESTED" => Ok(Self::InterruptRequested),
            "RECONCILED" => Ok(Self::Reconciled),
            "TERMINAL_COMPLETED" => Ok(Self::TerminalCompleted),
            "TERMINAL_FAILED" => Ok(Self::TerminalFailed),
            "TERMINAL_INTERRUPTED" => Ok(Self::TerminalInterrupted),
            _ => Err(LedgerError::InvalidTaskRuntimeRecord),
        }
    }

    const fn event_kind(self) -> LedgerEventKind {
        if self.is_terminal() {
            LedgerEventKind::EffectOutcome
        } else {
            LedgerEventKind::EvidenceRecorded
        }
    }

    const fn outcome(self) -> LedgerOutcome {
        match self {
            Self::TerminalCompleted => LedgerOutcome::Passed,
            Self::PrestartTerminalFailed | Self::TerminalFailed => LedgerOutcome::Failed,
            Self::TerminalInterrupted => LedgerOutcome::Cancelled,
            Self::ThreadAccepted
            | Self::TurnAccepted
            | Self::TurnStarted
            | Self::MeaningfulProgress
            | Self::Heartbeat
            | Self::StallClassified
            | Self::InterruptRequested
            | Self::Reconciled => LedgerOutcome::Recorded,
        }
    }

    /// Returns whether the exact attempt reached a terminal provider outcome.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::PrestartTerminalFailed
                | Self::TerminalCompleted
                | Self::TerminalFailed
                | Self::TerminalInterrupted
        )
    }
}

/// One bounded exact provider observation supplied by the connector owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerObservationInput {
    attempt_number: u64,
    kind: WorkerObservationKind,
    thread_id: String,
    turn_id: Option<String>,
    app_server_generation: u64,
    app_server_identity_digest: ContentDigest,
    provider_observed_at: Option<String>,
    evidence_digest: ContentDigest,
}

impl WorkerObservationInput {
    /// Constructs one secret-free thread/turn observation.
    ///
    /// # Errors
    ///
    /// Thread acceptance must omit a turn; every later exact observation must
    /// bind both identifiers. All identifiers and digests are bounded.
    pub fn new(
        attempt_number: u64,
        kind: WorkerObservationKind,
        thread_id: Option<impl Into<String>>,
        turn_id: Option<impl Into<String>>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> Result<Self, LedgerError> {
        Self::new_inner(
            attempt_number,
            kind,
            thread_id,
            turn_id,
            app_server_generation,
            app_server_identity_digest,
            None,
            evidence_digest,
        )
    }

    /// Constructs the one exact `turn/started` input with its connector-owned
    /// provider observation time. A start without this durable time has no
    /// valid representation.
    ///
    /// # Errors
    ///
    /// Rejects malformed IDs, a non-canonical provider time, or zero evidence.
    pub fn exact_started(
        attempt_number: u64,
        thread_id: impl Into<String>,
        turn_id: impl Into<String>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        provider_observed_at: impl Into<String>,
        evidence_digest: ContentDigest,
    ) -> Result<Self, LedgerError> {
        Self::new_inner(
            attempt_number,
            WorkerObservationKind::TurnStarted,
            Some(thread_id),
            Some(turn_id),
            app_server_generation,
            app_server_identity_digest,
            Some(provider_observed_at.into()),
            evidence_digest,
        )
    }

    fn new_inner(
        attempt_number: u64,
        kind: WorkerObservationKind,
        thread_id: Option<impl Into<String>>,
        turn_id: Option<impl Into<String>>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        provider_observed_at: Option<String>,
        evidence_digest: ContentDigest,
    ) -> Result<Self, LedgerError> {
        let thread_id = thread_id.map(Into::into);
        let turn_id = turn_id.map(Into::into);
        let valid_shape = match kind {
            WorkerObservationKind::ThreadAccepted => thread_id.is_some() && turn_id.is_none(),
            WorkerObservationKind::TurnAccepted
            | WorkerObservationKind::PrestartTerminalFailed
            | WorkerObservationKind::MeaningfulProgress
            | WorkerObservationKind::Heartbeat
            | WorkerObservationKind::StallClassified
            | WorkerObservationKind::InterruptRequested
            | WorkerObservationKind::Reconciled
            | WorkerObservationKind::TerminalCompleted
            | WorkerObservationKind::TerminalFailed
            | WorkerObservationKind::TerminalInterrupted => {
                thread_id.is_some() && turn_id.is_some()
            }
            WorkerObservationKind::TurnStarted => {
                thread_id.is_some()
                    && turn_id.is_some()
                    && provider_observed_at
                        .as_deref()
                        .is_some_and(|value| validate_utc_timestamp(value).is_ok())
            }
        };
        if attempt_number == 0
            || app_server_generation == 0
            || is_zero_digest(&app_server_identity_digest)
            || !valid_shape
            || (kind != WorkerObservationKind::TurnStarted && provider_observed_at.is_some())
            || thread_id
                .as_deref()
                .is_some_and(|value| !valid_runtime_identifier(value))
            || turn_id
                .as_deref()
                .is_some_and(|value| !valid_runtime_identifier(value))
            || is_zero_digest(&evidence_digest)
        {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        let Some(thread_id) = thread_id else {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        };
        Ok(Self {
            attempt_number,
            kind,
            thread_id,
            turn_id,
            app_server_generation,
            app_server_identity_digest,
            provider_observed_at,
            evidence_digest,
        })
    }

    /// Returns the exact worker attempt this provider observation targets.
    #[must_use]
    pub const fn attempt_number(&self) -> u64 {
        self.attempt_number
    }

    /// Returns the exact provider time only for `TURN_STARTED`.
    #[must_use]
    pub fn provider_observed_at(&self) -> Option<&str> {
        self.provider_observed_at.as_deref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkerObservationPayload {
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    binding_digest: ContentDigest,
    attempt_id: AttemptId,
    attempt_number: u64,
    kind: WorkerObservationKind,
    thread_id: String,
    turn_id: Option<String>,
    app_server_generation: u64,
    app_server_identity_digest: ContentDigest,
    observed_at: String,
    evidence_digest: ContentDigest,
    payload_digest: ContentDigest,
}

/// Verified exact worker lifecycle child row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedWorkerObservationRecord {
    link: TaskRuntimeEventLink,
    payload: WorkerObservationPayload,
}

impl VerifiedWorkerObservationRecord {
    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.payload.task_ref
    }

    #[must_use]
    pub const fn successor_stream_id(&self) -> &ContentDigest {
        &self.payload.successor_stream_id
    }

    #[must_use]
    pub const fn binding_digest(&self) -> &ContentDigest {
        &self.payload.binding_digest
    }

    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.payload.attempt_id
    }

    #[must_use]
    pub const fn attempt_number(&self) -> u64 {
        self.payload.attempt_number
    }

    #[must_use]
    pub const fn kind(&self) -> WorkerObservationKind {
        self.payload.kind
    }

    #[must_use]
    pub fn thread_id(&self) -> &str {
        &self.payload.thread_id
    }

    #[must_use]
    pub fn turn_id(&self) -> Option<&str> {
        self.payload.turn_id.as_deref()
    }

    #[must_use]
    pub const fn app_server_generation(&self) -> u64 {
        self.payload.app_server_generation
    }

    /// Returns the digest of the exact server-owned App Server session, home,
    /// and keyring-only configuration used by this observation.
    #[must_use]
    pub const fn app_server_identity_digest(&self) -> &ContentDigest {
        &self.payload.app_server_identity_digest
    }

    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.payload.observed_at
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.payload.evidence_digest
    }

    #[must_use]
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload.payload_digest
    }

    /// Returns canonical payload bytes for direct adapter persistence.
    ///
    /// # Errors
    ///
    /// Propagates canonical encoding failure.
    pub fn payload_canonical_bytes(&self) -> Result<Vec<u8>, LedgerError> {
        canonicalize(&worker_observation_payload_value(&self.payload))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(LedgerError::from)
    }

    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedWorkerObservationRow {
        UntrustedWorkerObservationRow {
            record_schema: WORKER_OBSERVATION_RECORD_SCHEMA.to_owned(),
            link: self.link.clone(),
            payload: self.payload.clone(),
        }
    }
}

/// Explicitly untrusted worker-observation persistence row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedWorkerObservationRow {
    record_schema: String,
    link: TaskRuntimeEventLink,
    payload: WorkerObservationPayload,
}

impl UntrustedWorkerObservationRow {
    /// Constructs one untrusted row from persistence scalars.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_schema: impl Into<String>,
        link: TaskRuntimeEventLink,
        task_ref: ContentDigest,
        successor_stream_id: ContentDigest,
        binding_digest: ContentDigest,
        attempt_id: AttemptId,
        attempt_number: u64,
        kind: WorkerObservationKind,
        thread_id: impl Into<String>,
        turn_id: Option<impl Into<String>>,
        app_server_generation: u64,
        app_server_identity_digest: ContentDigest,
        observed_at: impl Into<String>,
        evidence_digest: ContentDigest,
        payload_digest: ContentDigest,
    ) -> Self {
        Self {
            record_schema: record_schema.into(),
            link,
            payload: WorkerObservationPayload {
                task_ref,
                successor_stream_id,
                binding_digest,
                attempt_id,
                attempt_number,
                kind,
                thread_id: thread_id.into(),
                turn_id: turn_id.map(Into::into),
                app_server_generation,
                app_server_identity_digest,
                observed_at: observed_at.into(),
                evidence_digest,
                payload_digest,
            },
        }
    }

    /// Returns a copy with a changed closed observation kind for tamper tests.
    #[must_use]
    pub fn with_kind(mut self, kind: WorkerObservationKind) -> Self {
        self.payload.kind = kind;
        self
    }

    /// Returns a copy with a changed immutable thread ID for tamper tests.
    #[must_use]
    pub fn with_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.payload.thread_id = thread_id.into();
        self
    }

    /// Returns a copy with a changed immutable turn ID for tamper tests.
    #[must_use]
    pub fn with_turn_id(mut self, turn_id: Option<impl Into<String>>) -> Self {
        self.payload.turn_id = turn_id.map(Into::into);
        self
    }

    /// Returns a copy with a changed immutable observation time for tamper
    /// tests and persistence-adapter verification.
    #[must_use]
    pub fn with_observed_at(mut self, observed_at: impl Into<String>) -> Self {
        self.payload.observed_at = observed_at.into();
        self
    }
}

/// Pure Ledger append paired with an optional new worker-observation row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerObservationAppendPlan {
    ledger_plan: LedgerAppendPlan,
    record: VerifiedWorkerObservationRecord,
    new_record: Option<VerifiedWorkerObservationRecord>,
}

impl WorkerObservationAppendPlan {
    #[must_use]
    pub const fn ledger_plan(&self) -> &LedgerAppendPlan {
        &self.ledger_plan
    }

    #[must_use]
    pub const fn record(&self) -> &VerifiedWorkerObservationRecord {
        &self.record
    }

    /// Returns the persistence row that still needs an owner extension write.
    /// An exact Ledger retry returns `Some` only while repairing a missing
    /// extension row; a fully retained retry returns `None`.
    #[must_use]
    pub const fn new_record(&self) -> Option<&VerifiedWorkerObservationRecord> {
        self.new_record.as_ref()
    }
}

/// Plans one exact monotonic worker-attempt claim and `EFFECT_INTENT`.
///
/// # Errors
///
/// Rejects gaps/rollback, non-increasing Writer fences, duplicate attempt IDs,
/// a retry before exact terminal, changed command reuse, or corrupt child rows.
#[allow(clippy::needless_pass_by_value)]
pub fn plan_worker_attempt_append(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    existing_attempts: &[VerifiedWorkerAttemptRecord],
    existing_observations: &[VerifiedWorkerObservationRecord],
    metadata: TaskRuntimeAppendMetadata,
    input: WorkerAttemptInput,
) -> Result<WorkerAttemptAppendPlan, LedgerError> {
    plan_worker_attempt_append_inner(
        stream,
        binding,
        existing_attempts,
        existing_observations,
        None,
        metadata,
        input,
    )
}

/// Plans one worker attempt whose immediately preceding attempt has no exact
/// terminal but has an owner-verified, no-provider-effect closure.
///
/// # Errors
///
/// In addition to normal attempt validation, rejects foreign, changed, stale,
/// or packet-substituted closure proofs and rejects a proof when an exact
/// terminal already exists.
#[allow(clippy::needless_pass_by_value)]
pub fn plan_worker_attempt_append_with_no_provider_effect_predecessor(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    existing_attempts: &[VerifiedWorkerAttemptRecord],
    existing_observations: &[VerifiedWorkerObservationRecord],
    predecessor: &VerifiedNoProviderEffectPredecessor,
    metadata: TaskRuntimeAppendMetadata,
    input: WorkerAttemptInput,
) -> Result<WorkerAttemptAppendPlan, LedgerError> {
    plan_worker_attempt_append_inner(
        stream,
        binding,
        existing_attempts,
        existing_observations,
        Some(predecessor),
        metadata,
        input,
    )
}

#[allow(clippy::needless_pass_by_value)]
fn plan_worker_attempt_append_inner(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    existing_attempts: &[VerifiedWorkerAttemptRecord],
    existing_observations: &[VerifiedWorkerObservationRecord],
    no_provider_effect_predecessor: Option<&VerifiedNoProviderEffectPredecessor>,
    metadata: TaskRuntimeAppendMetadata,
    input: WorkerAttemptInput,
) -> Result<WorkerAttemptAppendPlan, LedgerError> {
    ensure_runtime_stream(stream, binding)?;
    let payload = build_attempt_payload(binding, &input, &metadata.occurred_at)?;
    let retained_row = existing_attempts
        .iter()
        .find(|record| record.link.command_id == metadata.command_id);
    let expected_head = retained_row.map_or_else(
        || retained_command_expected_head(stream, &metadata.command_id),
        |record| record.link.expected_head.clone(),
    );
    let command = attempt_append_command(
        expected_head,
        metadata.clone(),
        payload.payload_digest.clone(),
    )?;
    let ledger_plan = plan_append(stream, command)?;
    if ledger_plan.is_exact_retry() {
        let recovered = recover_worker_attempt_record(stream, binding, &metadata, &input)?
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        let missing_row = retained_row.is_none();
        let mut rows = existing_attempts
            .iter()
            .map(VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>();
        if missing_row {
            rows.push(recovered.to_untrusted());
        }
        let attempts = verify_untrusted_worker_attempt_rows(stream, binding, &rows)?;
        let record = attempts
            .iter()
            .find(|record| record.link.command_id == *ledger_plan.receipt().command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?
            .clone();
        let observations = verify_untrusted_worker_observation_rows(
            stream,
            binding,
            &attempts,
            &existing_observations
                .iter()
                .map(VerifiedWorkerObservationRecord::to_untrusted)
                .collect::<Vec<_>>(),
        )?;
        validate_worker_attempt_predecessor(
            binding,
            &attempts,
            &observations,
            &input,
            no_provider_effect_predecessor,
        )?;
        return Ok(WorkerAttemptAppendPlan {
            ledger_plan,
            record: record.clone(),
            new_record: missing_row.then_some(record),
        });
    }
    let attempts = verify_untrusted_worker_attempt_rows(
        stream,
        binding,
        &existing_attempts
            .iter()
            .map(VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    let observations = verify_untrusted_worker_observation_rows(
        stream,
        binding,
        &attempts,
        &existing_observations
            .iter()
            .map(VerifiedWorkerObservationRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    let expected_number = u64::try_from(attempts.len())
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(LedgerError::WorkerAttemptNotMonotonic)?;
    if input.attempt_number != expected_number
        || attempts
            .last()
            .is_some_and(|prior| input.writer_fence <= prior.writer_fence())
        || attempts
            .iter()
            .any(|record| record.attempt_id() == &input.attempt_id)
    {
        return Err(LedgerError::WorkerAttemptNotMonotonic);
    }
    validate_worker_attempt_predecessor(
        binding,
        &attempts,
        &observations,
        &input,
        no_provider_effect_predecessor,
    )?;
    let event = ledger_plan
        .new_event()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    validate_attempt_event_shape(event, &payload.payload_digest)?;
    let record = VerifiedWorkerAttemptRecord {
        link: link_from_event(stream.head().clone(), event),
        payload,
    };
    Ok(WorkerAttemptAppendPlan {
        ledger_plan,
        record: record.clone(),
        new_record: Some(record),
    })
}

fn validate_worker_attempt_predecessor(
    binding: &VerifiedTaskExecutionBinding,
    attempts: &[VerifiedWorkerAttemptRecord],
    observations: &[VerifiedWorkerObservationRecord],
    input: &WorkerAttemptInput,
    no_provider_effect_predecessor: Option<&VerifiedNoProviderEffectPredecessor>,
) -> Result<(), LedgerError> {
    if input.attempt_number == 1 {
        return if no_provider_effect_predecessor.is_none() {
            Ok(())
        } else {
            Err(LedgerError::TaskRuntimeSubstitution)
        };
    }
    let predecessor_number = input
        .attempt_number
        .checked_sub(1)
        .ok_or(LedgerError::WorkerAttemptNotMonotonic)?;
    let predecessor = attempts
        .iter()
        .find(|record| record.attempt_number() == predecessor_number)
        .ok_or(LedgerError::WorkerAttemptNotMonotonic)?;
    let has_exact_terminal = observations
        .iter()
        .any(|record| record.attempt_number() == predecessor_number && record.kind().is_terminal());
    match (has_exact_terminal, no_provider_effect_predecessor) {
        (true, None) => Ok(()),
        (true, Some(_)) => Err(LedgerError::TaskRuntimeSubstitution),
        (false, None) => Err(LedgerError::WorkerAttemptBeforeTerminal),
        (false, Some(verified)) => {
            let recomputed = no_provider_effect_predecessor_digest(verified)?;
            if verified.task_ref != *binding.task_ref()
                || verified.successor_stream_id != *binding.successor_stream_id()
                || verified.binding_digest != *binding.binding_digest()
                || verified.predecessor_attempt_id != *predecessor.attempt_id()
                || verified.predecessor_attempt_number != predecessor.attempt_number()
                || verified.predecessor_writer_fence != predecessor.writer_fence()
                || verified.successor_packet_digest != input.packet_digest
                || verified.digest != recomputed
            {
                return Err(LedgerError::TaskRuntimeSubstitution);
            }
            Ok(())
        }
    }
}

/// Reconstructs one exact worker-attempt record whose Ledger append was
/// retained before its owner-extension row was written.
///
/// The returned record is only the missing-row candidate. Callers must add it
/// to the loaded untrusted row set and run
/// [`verify_untrusted_task_runtime_records`] before performing any effect.
///
/// # Errors
///
/// Changed command input, event substitution, or malformed retained content
/// fails closed.
pub fn recover_worker_attempt_record(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    metadata: &TaskRuntimeAppendMetadata,
    input: &WorkerAttemptInput,
) -> Result<Option<VerifiedWorkerAttemptRecord>, LedgerError> {
    ensure_runtime_stream(stream, binding)?;
    let payload = build_attempt_payload(binding, input, &metadata.occurred_at)?;
    let command = attempt_append_command(
        retained_command_expected_head(stream, &metadata.command_id),
        metadata.clone(),
        payload.payload_digest.clone(),
    )?;
    let ledger_plan = plan_append(stream, command)?;
    if !ledger_plan.is_exact_retry() {
        return Ok(None);
    }
    let event = retained_event_for_command(
        stream,
        WORKER_ATTEMPT_ACTION,
        ledger_plan.receipt().command_id(),
    )?;
    validate_attempt_event_shape(event, &payload.payload_digest)?;
    let command = stream
        .commands()
        .iter()
        .find(|record| record.request().command_id() == event.command_id())
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    Ok(Some(VerifiedWorkerAttemptRecord {
        link: link_from_event(command.request().expected_head().clone(), event),
        payload,
    }))
}

/// Plans one exact worker lifecycle observation.
///
/// # Errors
///
/// Rejects unknown attempts, thread/turn drift, duplicate terminal evidence,
/// changed command reuse, or corrupt retained child rows.
#[allow(clippy::needless_pass_by_value)]
pub fn plan_worker_observation_append(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    existing_attempts: &[VerifiedWorkerAttemptRecord],
    existing_observations: &[VerifiedWorkerObservationRecord],
    metadata: TaskRuntimeAppendMetadata,
    input: WorkerObservationInput,
) -> Result<WorkerObservationAppendPlan, LedgerError> {
    let attempts = verify_untrusted_worker_attempt_rows(
        stream,
        binding,
        &existing_attempts
            .iter()
            .map(VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    let attempt = attempts
        .iter()
        .find(|attempt| attempt.attempt_number() == input.attempt_number)
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    let payload = build_observation_payload(binding, attempt, &input, &metadata.occurred_at)?;
    let retained_row = existing_observations
        .iter()
        .find(|record| record.link.command_id == metadata.command_id);
    let expected_head = retained_row.map_or_else(
        || retained_command_expected_head(stream, &metadata.command_id),
        |record| record.link.expected_head.clone(),
    );
    let command = observation_append_command(expected_head, metadata, &payload)?;
    let ledger_plan = plan_append(stream, command)?;
    if ledger_plan.is_exact_retry() {
        let event = retained_event_for_command(
            stream,
            WORKER_OBSERVATION_ACTION,
            ledger_plan.receipt().command_id(),
        )?;
        validate_observation_event_shape(event, &payload)?;
        let command = stream
            .commands()
            .iter()
            .find(|record| record.request().command_id() == event.command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        let recovered = VerifiedWorkerObservationRecord {
            link: link_from_event(command.request().expected_head().clone(), event),
            payload,
        };
        let missing_row = retained_row.is_none();
        let mut rows = existing_observations
            .iter()
            .map(VerifiedWorkerObservationRecord::to_untrusted)
            .collect::<Vec<_>>();
        if missing_row {
            rows.push(recovered.to_untrusted());
        }
        let observations =
            verify_untrusted_worker_observation_rows(stream, binding, &attempts, &rows)?;
        let record = observations
            .iter()
            .find(|record| record.link.command_id == *ledger_plan.receipt().command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?
            .clone();
        return Ok(WorkerObservationAppendPlan {
            ledger_plan,
            record: record.clone(),
            new_record: missing_row.then_some(record),
        });
    }
    let observations = verify_untrusted_worker_observation_rows(
        stream,
        binding,
        &attempts,
        &existing_observations
            .iter()
            .map(VerifiedWorkerObservationRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    validate_provider_identity(&observations, &payload)?;
    validate_next_observation_lifecycle(&observations, &payload)?;
    if payload.kind.is_terminal()
        && observations.iter().any(|record| {
            record.attempt_number() == payload.attempt_number && record.kind().is_terminal()
        })
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let event = ledger_plan
        .new_event()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    validate_observation_event_shape(event, &payload)?;
    let record = VerifiedWorkerObservationRecord {
        link: link_from_event(stream.head().clone(), event),
        payload,
    };
    Ok(WorkerObservationAppendPlan {
        ledger_plan,
        record: record.clone(),
        new_record: Some(record),
    })
}

/// Reconstructs one exact observation whose Ledger append was retained before
/// its owner-extension row was written.
///
/// The returned candidate must be combined with every loaded untrusted row and
/// passed through [`verify_untrusted_task_runtime_records`] before use.
///
/// # Errors
///
/// Unknown attempts, changed command input, provider/event substitution, or
/// malformed retained content fails closed.
pub fn recover_worker_observation_record(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    attempts: &[VerifiedWorkerAttemptRecord],
    metadata: &TaskRuntimeAppendMetadata,
    input: &WorkerObservationInput,
) -> Result<Option<VerifiedWorkerObservationRecord>, LedgerError> {
    let attempts = verify_untrusted_worker_attempt_rows(
        stream,
        binding,
        &attempts
            .iter()
            .map(VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    let attempt = attempts
        .iter()
        .find(|attempt| attempt.attempt_number() == input.attempt_number)
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    let payload = build_observation_payload(binding, attempt, input, &metadata.occurred_at)?;
    let command = observation_append_command(
        retained_command_expected_head(stream, &metadata.command_id),
        metadata.clone(),
        &payload,
    )?;
    let ledger_plan = plan_append(stream, command)?;
    if !ledger_plan.is_exact_retry() {
        return Ok(None);
    }
    let event = retained_event_for_command(
        stream,
        WORKER_OBSERVATION_ACTION,
        ledger_plan.receipt().command_id(),
    )?;
    validate_observation_event_shape(event, &payload)?;
    let command = stream
        .commands()
        .iter()
        .find(|record| record.request().command_id() == event.command_id())
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    Ok(Some(VerifiedWorkerObservationRecord {
        link: link_from_event(command.request().expected_head().clone(), event),
        payload,
    }))
}

/// Verifies all worker-attempt child rows against their exact Ledger events.
///
/// # Errors
///
/// Missing, duplicate, changed, reordered, cross-stream, or non-monotonic rows
/// fail closed.
pub fn verify_untrusted_worker_attempt_rows(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    rows: &[UntrustedWorkerAttemptRow],
) -> Result<Vec<VerifiedWorkerAttemptRecord>, LedgerError> {
    ensure_runtime_stream(stream, binding)?;
    let events = runtime_events(stream, WORKER_ATTEMPT_ACTION);
    if events.len() != rows.len() {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let mut verified = Vec::with_capacity(rows.len());
    let mut seen_events = BTreeSet::new();
    let mut seen_attempt_ids = BTreeSet::new();
    let mut previous_fence = None;
    for (index, event) in events.into_iter().enumerate() {
        let Some(row) = rows
            .iter()
            .find(|row| row.link.event_digest == *event.event_digest())
        else {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        };
        if row.record_schema != WORKER_ATTEMPT_RECORD_SCHEMA
            || !seen_events.insert(row.link.event_digest.as_str().to_owned())
        {
            return Err(if row.record_schema == WORKER_ATTEMPT_RECORD_SCHEMA {
                LedgerError::InvalidTaskRuntimeRecord
            } else {
                LedgerError::UnknownTaskRuntimeRecordVersion
            });
        }
        let expected_number = u64::try_from(index)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(LedgerError::WorkerAttemptNotMonotonic)?;
        if row.payload.task_ref != *binding.task_ref()
            || row.payload.successor_stream_id != *binding.successor_stream_id()
            || row.payload.task_spec_digest != *binding.task_spec_digest()
            || row.payload.binding_digest != *binding.binding_digest()
            || row.payload.budget_digest != *binding.budget_digest()
            || row.payload.attempt_number != expected_number
            || !seen_attempt_ids.insert(row.payload.attempt_id.as_str().to_owned())
            || previous_fence.is_some_and(|fence| row.payload.writer_fence <= fence)
            || !valid_runtime_identifier(row.payload.attempt_id.as_str())
            || !row.payload.model_reason.is_allowed_for(row.payload.model)
        {
            return Err(LedgerError::WorkerAttemptNotMonotonic);
        }
        let recomputed = worker_attempt_payload_digest(&row.payload)?;
        validate_runtime_link(stream, event, &row.link)?;
        validate_attempt_event_shape(event, &recomputed)?;
        if row.payload.payload_digest != recomputed || row.link.payload_digest != recomputed {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        previous_fence = Some(row.payload.writer_fence);
        verified.push(VerifiedWorkerAttemptRecord {
            link: row.link.clone(),
            payload: row.payload.clone(),
        });
    }
    Ok(verified)
}

/// Verifies all worker lifecycle rows and immutable provider identity binding.
///
/// # Errors
///
/// Missing, duplicate, unknown-attempt, changed thread/turn, malformed
/// terminal, or digest/linkage tamper fails closed.
pub fn verify_untrusted_worker_observation_rows(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    attempts: &[VerifiedWorkerAttemptRecord],
    rows: &[UntrustedWorkerObservationRow],
) -> Result<Vec<VerifiedWorkerObservationRecord>, LedgerError> {
    ensure_runtime_stream(stream, binding)?;
    let events = runtime_events(stream, WORKER_OBSERVATION_ACTION);
    if events.len() != rows.len() {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let mut verified = Vec::with_capacity(rows.len());
    let mut seen_events = BTreeSet::new();
    let mut provider_ids = BTreeMap::<u64, (String, Option<String>, u64, ContentDigest)>::new();
    let mut terminal_attempts = BTreeSet::new();
    let mut lifecycle_by_attempt = BTreeMap::new();
    for event in events {
        let Some(row) = rows
            .iter()
            .find(|row| row.link.event_digest == *event.event_digest())
        else {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        };
        if row.record_schema != WORKER_OBSERVATION_RECORD_SCHEMA
            || !seen_events.insert(row.link.event_digest.as_str().to_owned())
        {
            return Err(if row.record_schema == WORKER_OBSERVATION_RECORD_SCHEMA {
                LedgerError::InvalidTaskRuntimeRecord
            } else {
                LedgerError::UnknownTaskRuntimeRecordVersion
            });
        }
        let attempt = attempts
            .iter()
            .find(|attempt| attempt.attempt_number() == row.payload.attempt_number)
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        if attempt.link.event_sequence >= event.sequence()
            || row.payload.task_ref != *binding.task_ref()
            || row.payload.successor_stream_id != *binding.successor_stream_id()
            || row.payload.binding_digest != *binding.binding_digest()
            || row.payload.attempt_id != *attempt.attempt_id()
            || (row.payload.kind != WorkerObservationKind::TurnStarted
                && row.payload.observed_at != event.occurred_at())
            || !valid_observation_shape(&row.payload)
        {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        let recomputed = worker_observation_payload_digest(&row.payload)?;
        validate_runtime_link(stream, event, &row.link)?;
        validate_observation_event_shape(event, &row.payload)?;
        if row.payload.payload_digest != recomputed || row.link.payload_digest != recomputed {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        validate_provider_identity_map(&mut provider_ids, &row.payload)?;
        advance_observation_lifecycle(&mut lifecycle_by_attempt, &row.payload)?;
        if row.payload.kind.is_terminal() && !terminal_attempts.insert(row.payload.attempt_number) {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        verified.push(VerifiedWorkerObservationRecord {
            link: row.link.clone(),
            payload: row.payload.clone(),
        });
    }
    Ok(verified)
}

/// Closed independent verification outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationOutcome {
    Passed,
    Failed,
}

impl VerificationOutcome {
    /// Returns the stable record value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "PASSED",
            Self::Failed => "FAILED",
        }
    }

    /// Parses one closed verification outcome.
    ///
    /// # Errors
    ///
    /// Rejects unknown outcomes.
    pub fn parse(value: &str) -> Result<Self, LedgerError> {
        match value {
            "PASSED" => Ok(Self::Passed),
            "FAILED" => Ok(Self::Failed),
            _ => Err(LedgerError::InvalidTaskRuntimeRecord),
        }
    }

    const fn ledger_outcome(self) -> LedgerOutcome {
        match self {
            Self::Passed => LedgerOutcome::Passed,
            Self::Failed => LedgerOutcome::Failed,
        }
    }

    const fn reason(self) -> &'static str {
        match self {
            Self::Passed => TASK_VERIFICATION_PASSED_REASON,
            Self::Failed => TASK_VERIFICATION_FAILED_REASON,
        }
    }
}

/// Immutable closed verification commitments for one terminal attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskVerificationInput {
    attempt_number: u64,
    outcome: VerificationOutcome,
    verification_profile_digest: ContentDigest,
    base_commit_digest: ContentDigest,
    result_commit_digest: ContentDigest,
    tree_digest: ContentDigest,
    diff_digest: ContentDigest,
    result_digest: ContentDigest,
    evidence_artifact_digest: ContentDigest,
    review_digest: Option<ContentDigest>,
}

impl TaskVerificationInput {
    /// Constructs one verification result without accepting commands or paths.
    ///
    /// # Errors
    ///
    /// Rejects zero attempt/digest values.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        attempt_number: u64,
        outcome: VerificationOutcome,
        verification_profile_digest: ContentDigest,
        base_commit_digest: ContentDigest,
        result_commit_digest: ContentDigest,
        tree_digest: ContentDigest,
        diff_digest: ContentDigest,
        result_digest: ContentDigest,
        evidence_artifact_digest: ContentDigest,
        review_digest: Option<ContentDigest>,
    ) -> Result<Self, LedgerError> {
        if attempt_number == 0
            || [
                &verification_profile_digest,
                &base_commit_digest,
                &result_commit_digest,
                &tree_digest,
                &diff_digest,
                &result_digest,
                &evidence_artifact_digest,
            ]
            .into_iter()
            .any(is_zero_digest)
            || review_digest.as_ref().is_some_and(is_zero_digest)
        {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        Ok(Self {
            attempt_number,
            outcome,
            verification_profile_digest,
            base_commit_digest,
            result_commit_digest,
            tree_digest,
            diff_digest,
            result_digest,
            evidence_artifact_digest,
            review_digest,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TaskVerificationPayload {
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    task_spec_digest: ContentDigest,
    binding_digest: ContentDigest,
    attempt_id: AttemptId,
    attempt_number: u64,
    outcome: VerificationOutcome,
    verification_profile_digest: ContentDigest,
    base_commit_digest: ContentDigest,
    result_commit_digest: ContentDigest,
    tree_digest: ContentDigest,
    diff_digest: ContentDigest,
    result_digest: ContentDigest,
    evidence_artifact_digest: ContentDigest,
    review_digest: Option<ContentDigest>,
    verified_at: String,
    payload_digest: ContentDigest,
}

/// Verified independent-verification child row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTaskVerificationRecord {
    link: TaskRuntimeEventLink,
    payload: TaskVerificationPayload,
}

impl VerifiedTaskVerificationRecord {
    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.payload.task_ref
    }

    #[must_use]
    pub const fn successor_stream_id(&self) -> &ContentDigest {
        &self.payload.successor_stream_id
    }

    #[must_use]
    pub const fn task_spec_digest(&self) -> &ContentDigest {
        &self.payload.task_spec_digest
    }

    #[must_use]
    pub const fn binding_digest(&self) -> &ContentDigest {
        &self.payload.binding_digest
    }

    #[must_use]
    pub const fn attempt_id(&self) -> &AttemptId {
        &self.payload.attempt_id
    }

    #[must_use]
    pub const fn attempt_number(&self) -> u64 {
        self.payload.attempt_number
    }

    #[must_use]
    pub const fn outcome(&self) -> VerificationOutcome {
        self.payload.outcome
    }

    #[must_use]
    pub const fn result_digest(&self) -> &ContentDigest {
        &self.payload.result_digest
    }

    #[must_use]
    pub const fn verification_profile_digest(&self) -> &ContentDigest {
        &self.payload.verification_profile_digest
    }

    #[must_use]
    pub const fn base_commit_digest(&self) -> &ContentDigest {
        &self.payload.base_commit_digest
    }

    #[must_use]
    pub const fn result_commit_digest(&self) -> &ContentDigest {
        &self.payload.result_commit_digest
    }

    #[must_use]
    pub const fn tree_digest(&self) -> &ContentDigest {
        &self.payload.tree_digest
    }

    #[must_use]
    pub const fn diff_digest(&self) -> &ContentDigest {
        &self.payload.diff_digest
    }

    #[must_use]
    pub const fn evidence_artifact_digest(&self) -> &ContentDigest {
        &self.payload.evidence_artifact_digest
    }

    #[must_use]
    pub const fn review_digest(&self) -> Option<&ContentDigest> {
        self.payload.review_digest.as_ref()
    }

    #[must_use]
    pub fn verified_at(&self) -> &str {
        &self.payload.verified_at
    }

    #[must_use]
    pub const fn payload_digest(&self) -> &ContentDigest {
        &self.payload.payload_digest
    }

    /// Returns canonical payload bytes for direct adapter persistence.
    ///
    /// # Errors
    ///
    /// Propagates canonical encoding failure.
    pub fn payload_canonical_bytes(&self) -> Result<Vec<u8>, LedgerError> {
        canonicalize(&task_verification_payload_value(&self.payload))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(LedgerError::from)
    }

    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedTaskVerificationRow {
        UntrustedTaskVerificationRow {
            record_schema: TASK_VERIFICATION_RECORD_SCHEMA.to_owned(),
            link: self.link.clone(),
            payload: self.payload.clone(),
        }
    }
}

/// Explicitly untrusted independent-verification persistence row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedTaskVerificationRow {
    record_schema: String,
    link: TaskRuntimeEventLink,
    payload: TaskVerificationPayload,
}

impl UntrustedTaskVerificationRow {
    /// Constructs one untrusted row from persistence scalars.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        record_schema: impl Into<String>,
        link: TaskRuntimeEventLink,
        task_ref: ContentDigest,
        successor_stream_id: ContentDigest,
        task_spec_digest: ContentDigest,
        binding_digest: ContentDigest,
        attempt_id: AttemptId,
        attempt_number: u64,
        outcome: VerificationOutcome,
        verification_profile_digest: ContentDigest,
        base_commit_digest: ContentDigest,
        result_commit_digest: ContentDigest,
        tree_digest: ContentDigest,
        diff_digest: ContentDigest,
        result_digest: ContentDigest,
        evidence_artifact_digest: ContentDigest,
        review_digest: Option<ContentDigest>,
        verified_at: impl Into<String>,
        payload_digest: ContentDigest,
    ) -> Self {
        Self {
            record_schema: record_schema.into(),
            link,
            payload: TaskVerificationPayload {
                task_ref,
                successor_stream_id,
                task_spec_digest,
                binding_digest,
                attempt_id,
                attempt_number,
                outcome,
                verification_profile_digest,
                base_commit_digest,
                result_commit_digest,
                tree_digest,
                diff_digest,
                result_digest,
                evidence_artifact_digest,
                review_digest,
                verified_at: verified_at.into(),
                payload_digest,
            },
        }
    }

    /// Returns a copy with a changed result digest for tamper tests.
    #[must_use]
    pub fn with_result_digest(mut self, result_digest: ContentDigest) -> Self {
        self.payload.result_digest = result_digest;
        self
    }
}

/// Pure Ledger append paired with an optional new verification child row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskVerificationAppendPlan {
    ledger_plan: LedgerAppendPlan,
    record: VerifiedTaskVerificationRecord,
    new_record: Option<VerifiedTaskVerificationRecord>,
}

impl TaskVerificationAppendPlan {
    #[must_use]
    pub const fn ledger_plan(&self) -> &LedgerAppendPlan {
        &self.ledger_plan
    }

    #[must_use]
    pub const fn record(&self) -> &VerifiedTaskVerificationRecord {
        &self.record
    }

    /// Returns the persistence row that still needs an owner extension write.
    /// An exact Ledger retry returns `Some` only while repairing a missing
    /// extension row; a fully retained retry returns `None`.
    #[must_use]
    pub const fn new_record(&self) -> Option<&VerifiedTaskVerificationRecord> {
        self.new_record.as_ref()
    }
}

/// Complete replay-verified managed-task child-record projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTaskRuntimeRecords {
    attempts: Vec<VerifiedWorkerAttemptRecord>,
    observations: Vec<VerifiedWorkerObservationRecord>,
    verifications: Vec<VerifiedTaskVerificationRecord>,
}

impl VerifiedTaskRuntimeRecords {
    #[must_use]
    pub fn attempts(&self) -> &[VerifiedWorkerAttemptRecord] {
        &self.attempts
    }

    #[must_use]
    pub fn observations(&self) -> &[VerifiedWorkerObservationRecord] {
        &self.observations
    }

    #[must_use]
    pub fn verifications(&self) -> &[VerifiedTaskVerificationRecord] {
        &self.verifications
    }
}

/// Plans one typed independent-verification evidence append.
///
/// # Errors
///
/// Rejects verification before an exact terminal, duplicate verification for
/// an attempt, changed command reuse, or corrupt retained child rows.
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub fn plan_task_verification_append(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    existing_attempts: &[VerifiedWorkerAttemptRecord],
    existing_observations: &[VerifiedWorkerObservationRecord],
    existing_verifications: &[VerifiedTaskVerificationRecord],
    metadata: TaskRuntimeAppendMetadata,
    input: TaskVerificationInput,
) -> Result<TaskVerificationAppendPlan, LedgerError> {
    let attempts = verify_untrusted_worker_attempt_rows(
        stream,
        binding,
        &existing_attempts
            .iter()
            .map(VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    let observations = verify_untrusted_worker_observation_rows(
        stream,
        binding,
        &attempts,
        &existing_observations
            .iter()
            .map(VerifiedWorkerObservationRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    let attempt = attempts
        .iter()
        .find(|attempt| attempt.attempt_number() == input.attempt_number)
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    if !observations.iter().any(|observation| {
        observation.attempt_number() == input.attempt_number && observation.kind().is_terminal()
    }) {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let payload = build_verification_payload(binding, attempt, &input, &metadata.occurred_at)?;
    let retained_row = existing_verifications
        .iter()
        .find(|record| record.link.command_id == metadata.command_id);
    let expected_head = retained_row.map_or_else(
        || retained_command_expected_head(stream, &metadata.command_id),
        |record| record.link.expected_head.clone(),
    );
    let command = verification_append_command(expected_head, metadata.clone(), &payload)?;
    let ledger_plan = plan_append(stream, command)?;
    if ledger_plan.is_exact_retry() {
        let recovered = recover_task_verification_record(
            stream,
            binding,
            &attempts,
            &observations,
            &metadata,
            &input,
        )?
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        let missing_row = retained_row.is_none();
        let mut rows = existing_verifications
            .iter()
            .map(VerifiedTaskVerificationRecord::to_untrusted)
            .collect::<Vec<_>>();
        if missing_row {
            rows.push(recovered.to_untrusted());
        }
        let verifications = verify_untrusted_task_verification_rows(
            stream,
            binding,
            &attempts,
            &observations,
            &rows,
        )?;
        let record = verifications
            .iter()
            .find(|record| record.link.command_id == *ledger_plan.receipt().command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?
            .clone();
        return Ok(TaskVerificationAppendPlan {
            ledger_plan,
            record: record.clone(),
            new_record: missing_row.then_some(record),
        });
    }
    let verifications = verify_untrusted_task_verification_rows(
        stream,
        binding,
        &attempts,
        &observations,
        &existing_verifications
            .iter()
            .map(VerifiedTaskVerificationRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    if verifications
        .iter()
        .any(|record| record.attempt_number() == input.attempt_number)
    {
        return Err(LedgerError::TaskRuntimeSubstitution);
    }
    let event = ledger_plan
        .new_event()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    validate_verification_event_shape(event, &payload)?;
    let record = VerifiedTaskVerificationRecord {
        link: link_from_event(stream.head().clone(), event),
        payload,
    };
    Ok(TaskVerificationAppendPlan {
        ledger_plan,
        record: record.clone(),
        new_record: Some(record),
    })
}

/// Reconstructs one exact verification record whose Ledger append was
/// retained before its owner-extension row was written.
///
/// The returned candidate must be combined with every loaded untrusted row and
/// passed through [`verify_untrusted_task_runtime_records`] before use.
///
/// # Errors
///
/// Missing terminal evidence, changed command input, event substitution, or
/// malformed retained content fails closed.
pub fn recover_task_verification_record(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    attempts: &[VerifiedWorkerAttemptRecord],
    observations: &[VerifiedWorkerObservationRecord],
    metadata: &TaskRuntimeAppendMetadata,
    input: &TaskVerificationInput,
) -> Result<Option<VerifiedTaskVerificationRecord>, LedgerError> {
    let attempts = verify_untrusted_worker_attempt_rows(
        stream,
        binding,
        &attempts
            .iter()
            .map(VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    let observations = verify_untrusted_worker_observation_rows(
        stream,
        binding,
        &attempts,
        &observations
            .iter()
            .map(VerifiedWorkerObservationRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    let attempt = attempts
        .iter()
        .find(|attempt| attempt.attempt_number() == input.attempt_number)
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    if !observations.iter().any(|observation| {
        observation.attempt_number() == input.attempt_number && observation.kind().is_terminal()
    }) {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let payload = build_verification_payload(binding, attempt, input, &metadata.occurred_at)?;
    let command = verification_append_command(
        retained_command_expected_head(stream, &metadata.command_id),
        metadata.clone(),
        &payload,
    )?;
    let ledger_plan = plan_append(stream, command)?;
    if !ledger_plan.is_exact_retry() {
        return Ok(None);
    }
    let event = retained_event_for_command(
        stream,
        TASK_VERIFICATION_ACTION,
        ledger_plan.receipt().command_id(),
    )?;
    validate_verification_event_shape(event, &payload)?;
    let command = stream
        .commands()
        .iter()
        .find(|record| record.request().command_id() == event.command_id())
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    Ok(Some(VerifiedTaskVerificationRecord {
        link: link_from_event(command.request().expected_head().clone(), event),
        payload,
    }))
}

/// Verifies all three Phase-4 child-record families from untrusted rows.
///
/// # Errors
///
/// Any missing, extra, reordered, cross-linked, or digest-tampered row fails
/// closed before a runtime projection is returned.
pub fn verify_untrusted_task_runtime_records(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    attempt_rows: &[UntrustedWorkerAttemptRow],
    observation_rows: &[UntrustedWorkerObservationRow],
    verification_rows: &[UntrustedTaskVerificationRow],
) -> Result<VerifiedTaskRuntimeRecords, LedgerError> {
    let attempts = verify_untrusted_worker_attempt_rows(stream, binding, attempt_rows)?;
    let observations =
        verify_untrusted_worker_observation_rows(stream, binding, &attempts, observation_rows)?;
    let verifications = verify_untrusted_task_verification_rows(
        stream,
        binding,
        &attempts,
        &observations,
        verification_rows,
    )?;
    Ok(VerifiedTaskRuntimeRecords {
        attempts,
        observations,
        verifications,
    })
}

/// Verifies all independent-verification child rows.
///
/// # Errors
///
/// Unknown schema, missing/duplicate row, pre-terminal verification, or any
/// event/payload/linkage substitution fails closed.
pub fn verify_untrusted_task_verification_rows(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    attempts: &[VerifiedWorkerAttemptRecord],
    observations: &[VerifiedWorkerObservationRecord],
    rows: &[UntrustedTaskVerificationRow],
) -> Result<Vec<VerifiedTaskVerificationRecord>, LedgerError> {
    ensure_runtime_stream(stream, binding)?;
    let events = runtime_events(stream, TASK_VERIFICATION_ACTION);
    if events.len() != rows.len() {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let mut verified = Vec::with_capacity(rows.len());
    let mut seen_events = BTreeSet::new();
    let mut seen_attempts = BTreeSet::new();
    for event in events {
        let row = rows
            .iter()
            .find(|row| row.link.event_digest == *event.event_digest())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        if row.record_schema != TASK_VERIFICATION_RECORD_SCHEMA
            || !seen_events.insert(row.link.event_digest.as_str().to_owned())
        {
            return Err(if row.record_schema == TASK_VERIFICATION_RECORD_SCHEMA {
                LedgerError::InvalidTaskRuntimeRecord
            } else {
                LedgerError::UnknownTaskRuntimeRecordVersion
            });
        }
        let attempt = attempts
            .iter()
            .find(|attempt| attempt.attempt_number() == row.payload.attempt_number)
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        let terminal = observations.iter().find(|observation| {
            observation.attempt_number() == row.payload.attempt_number
                && observation.kind().is_terminal()
        });
        if terminal.is_none_or(|terminal| terminal.link.event_sequence >= event.sequence())
            || !seen_attempts.insert(row.payload.attempt_number)
            || row.payload.task_ref != *binding.task_ref()
            || row.payload.successor_stream_id != *binding.successor_stream_id()
            || row.payload.task_spec_digest != *binding.task_spec_digest()
            || row.payload.binding_digest != *binding.binding_digest()
            || row.payload.attempt_id != *attempt.attempt_id()
            || row.payload.verified_at != event.occurred_at()
            || !valid_verification_payload(&row.payload)
        {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        let recomputed = task_verification_payload_digest(&row.payload)?;
        validate_runtime_link(stream, event, &row.link)?;
        validate_verification_event_shape(event, &row.payload)?;
        if row.payload.payload_digest != recomputed || row.link.payload_digest != recomputed {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        verified.push(VerifiedTaskVerificationRecord {
            link: row.link.clone(),
            payload: row.payload.clone(),
        });
    }
    Ok(verified)
}

/// Pure Ledger append paired with the approval-evidence event link that an
/// approval owner persists beside its already-verified authority record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalEvidenceAppendPlan {
    ledger_plan: LedgerAppendPlan,
    link: TaskRuntimeEventLink,
    new_link: Option<TaskRuntimeEventLink>,
}

impl ApprovalEvidenceAppendPlan {
    /// Returns whether the exact command is a non-mutating replay.
    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.ledger_plan.is_exact_retry()
    }

    /// Returns the Task Ledger append plan.
    #[must_use]
    pub const fn ledger_plan(&self) -> &LedgerAppendPlan {
        &self.ledger_plan
    }

    /// Returns the retained or newly planned approval-evidence link.
    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }

    /// Returns the persistence link that still needs an owner extension write.
    /// An exact Ledger retry returns `Some` only while repairing a missing
    /// extension row; a fully retained retry returns `None`.
    #[must_use]
    pub const fn new_link(&self) -> Option<&TaskRuntimeEventLink> {
        self.new_link.as_ref()
    }
}

/// Pure Ledger append paired with the artifact-reference event link that an
/// Artifact Store owner persists beside its verified descriptor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReferenceAppendPlan {
    ledger_plan: LedgerAppendPlan,
    link: TaskRuntimeEventLink,
    new_link: Option<TaskRuntimeEventLink>,
}

impl ArtifactReferenceAppendPlan {
    /// Returns whether the exact command is a non-mutating replay.
    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.ledger_plan.is_exact_retry()
    }

    /// Returns the Task Ledger append plan.
    #[must_use]
    pub const fn ledger_plan(&self) -> &LedgerAppendPlan {
        &self.ledger_plan
    }

    /// Returns the retained or newly planned artifact-reference link.
    #[must_use]
    pub const fn link(&self) -> &TaskRuntimeEventLink {
        &self.link
    }

    /// Returns the persistence link that still needs an owner extension write.
    /// An exact Ledger retry returns `Some` only while repairing a missing
    /// extension row; a fully retained retry returns `None`.
    #[must_use]
    pub const fn new_link(&self) -> Option<&TaskRuntimeEventLink> {
        self.new_link.as_ref()
    }
}

/// Verifies every approval-evidence link against its exclusive Ledger event.
///
/// # Errors
///
/// Missing, extra, duplicate-authority, changed, cross-stream, or
/// artifact-reference links fail closed.
pub fn verify_approval_evidence_links(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    links: &[TaskRuntimeEventLink],
) -> Result<Vec<TaskRuntimeEventLink>, LedgerError> {
    verify_reference_links(
        stream,
        binding,
        links,
        APPROVAL_EVIDENCE_ACTION,
        APPROVAL_EVIDENCE_REASON,
        true,
    )
}

/// Verifies every artifact-reference link against its exclusive Ledger event.
///
/// # Errors
///
/// Missing, extra, changed, cross-stream, or approval-evidence links fail
/// closed. Descriptor reuse is permitted because distinct attempts may retain
/// the same immutable Artifact Store object under separate child rows.
pub fn verify_artifact_reference_links(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    links: &[TaskRuntimeEventLink],
) -> Result<Vec<TaskRuntimeEventLink>, LedgerError> {
    verify_reference_links(
        stream,
        binding,
        links,
        ARTIFACT_REFERENCE_ACTION,
        ARTIFACT_REFERENCE_REASON,
        false,
    )
}

/// Plans one approval-evidence child event for an already verified authority
/// digest. This function verifies linkage only and never grants authority.
///
/// # Errors
///
/// Zero or duplicate authority digests, changed command retries, corrupt
/// retained links, or cross-stream substitution fail closed.
#[allow(clippy::needless_pass_by_value)]
pub fn plan_approval_evidence_append(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    existing_links: &[TaskRuntimeEventLink],
    metadata: TaskRuntimeAppendMetadata,
    authority_digest: ContentDigest,
) -> Result<ApprovalEvidenceAppendPlan, LedgerError> {
    if is_zero_digest(&authority_digest) {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let retained_row = existing_links
        .iter()
        .find(|link| link.payload_digest == authority_digest);
    if retained_row.is_some_and(|link| link.command_id != metadata.command_id) {
        return Err(LedgerError::TaskRuntimeSubstitution);
    }
    let expected_head = retained_row.map_or_else(
        || retained_command_expected_head(stream, &metadata.command_id),
        |link| link.expected_head.clone(),
    );
    let command = reference_append_command(
        expected_head,
        metadata,
        APPROVAL_EVIDENCE_ACTION,
        APPROVAL_EVIDENCE_REASON,
        authority_digest.clone(),
    )?;
    let ledger_plan = plan_append(stream, command)?;
    if ledger_plan.is_exact_retry() {
        let event = retained_event_for_command(
            stream,
            APPROVAL_EVIDENCE_ACTION,
            ledger_plan.receipt().command_id(),
        )?;
        validate_reference_event_shape(
            event,
            APPROVAL_EVIDENCE_ACTION,
            APPROVAL_EVIDENCE_REASON,
            &authority_digest,
        )?;
        let command = stream
            .commands()
            .iter()
            .find(|record| record.request().command_id() == event.command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        let recovered = link_from_event(command.request().expected_head().clone(), event);
        let missing_link = retained_row.is_none();
        let mut links = existing_links.to_vec();
        if missing_link {
            links.push(recovered);
        }
        let links = verify_approval_evidence_links(stream, binding, &links)?;
        let link = links
            .iter()
            .find(|link| link.command_id == *ledger_plan.receipt().command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?
            .clone();
        return Ok(ApprovalEvidenceAppendPlan {
            ledger_plan,
            link: link.clone(),
            new_link: missing_link.then_some(link),
        });
    }
    verify_approval_evidence_links(stream, binding, existing_links)?;
    let event = ledger_plan
        .new_event()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    validate_reference_event_shape(
        event,
        APPROVAL_EVIDENCE_ACTION,
        APPROVAL_EVIDENCE_REASON,
        &authority_digest,
    )?;
    let link = link_from_event(stream.head().clone(), event);
    Ok(ApprovalEvidenceAppendPlan {
        ledger_plan,
        link: link.clone(),
        new_link: Some(link),
    })
}

/// Plans one artifact-reference child event for an already verified Artifact
/// Store descriptor and an existing worker attempt.
///
/// # Errors
///
/// Unknown attempts, zero descriptors, changed command retries, corrupt
/// retained links, or cross-stream substitution fail closed.
#[allow(clippy::needless_pass_by_value)]
pub fn plan_artifact_reference_append(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    existing_attempts: &[VerifiedWorkerAttemptRecord],
    existing_links: &[TaskRuntimeEventLink],
    metadata: TaskRuntimeAppendMetadata,
    attempt_number: u64,
    descriptor_digest: ContentDigest,
) -> Result<ArtifactReferenceAppendPlan, LedgerError> {
    if attempt_number == 0 || is_zero_digest(&descriptor_digest) {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let attempts = verify_untrusted_worker_attempt_rows(
        stream,
        binding,
        &existing_attempts
            .iter()
            .map(VerifiedWorkerAttemptRecord::to_untrusted)
            .collect::<Vec<_>>(),
    )?;
    if !attempts
        .iter()
        .any(|attempt| attempt.attempt_number() == attempt_number)
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let retained_row = existing_links
        .iter()
        .find(|link| link.command_id == metadata.command_id);
    if retained_row.is_some_and(|link| link.payload_digest != descriptor_digest) {
        return Err(LedgerError::CommandIdReuse);
    }
    let expected_head = retained_row.map_or_else(
        || retained_command_expected_head(stream, &metadata.command_id),
        |link| link.expected_head.clone(),
    );
    let command = reference_append_command(
        expected_head,
        metadata,
        ARTIFACT_REFERENCE_ACTION,
        ARTIFACT_REFERENCE_REASON,
        descriptor_digest.clone(),
    )?;
    let ledger_plan = plan_append(stream, command)?;
    if ledger_plan.is_exact_retry() {
        let event = retained_event_for_command(
            stream,
            ARTIFACT_REFERENCE_ACTION,
            ledger_plan.receipt().command_id(),
        )?;
        validate_reference_event_shape(
            event,
            ARTIFACT_REFERENCE_ACTION,
            ARTIFACT_REFERENCE_REASON,
            &descriptor_digest,
        )?;
        let command = stream
            .commands()
            .iter()
            .find(|record| record.request().command_id() == event.command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        let recovered = link_from_event(command.request().expected_head().clone(), event);
        let missing_link = retained_row.is_none();
        let mut links = existing_links.to_vec();
        if missing_link {
            links.push(recovered);
        }
        let links = verify_artifact_reference_links(stream, binding, &links)?;
        let link = links
            .iter()
            .find(|link| link.command_id == *ledger_plan.receipt().command_id())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?
            .clone();
        return Ok(ArtifactReferenceAppendPlan {
            ledger_plan,
            link: link.clone(),
            new_link: missing_link.then_some(link),
        });
    }
    verify_artifact_reference_links(stream, binding, existing_links)?;
    let event = ledger_plan
        .new_event()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    validate_reference_event_shape(
        event,
        ARTIFACT_REFERENCE_ACTION,
        ARTIFACT_REFERENCE_REASON,
        &descriptor_digest,
    )?;
    let link = link_from_event(stream.head().clone(), event);
    Ok(ArtifactReferenceAppendPlan {
        ledger_plan,
        link: link.clone(),
        new_link: Some(link),
    })
}

fn build_verification_payload(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    input: &TaskVerificationInput,
    verified_at: &str,
) -> Result<TaskVerificationPayload, LedgerError> {
    let mut payload = TaskVerificationPayload {
        task_ref: binding.task_ref().clone(),
        successor_stream_id: binding.successor_stream_id().clone(),
        task_spec_digest: binding.task_spec_digest().clone(),
        binding_digest: binding.binding_digest().clone(),
        attempt_id: attempt.attempt_id().clone(),
        attempt_number: input.attempt_number,
        outcome: input.outcome,
        verification_profile_digest: input.verification_profile_digest.clone(),
        base_commit_digest: input.base_commit_digest.clone(),
        result_commit_digest: input.result_commit_digest.clone(),
        tree_digest: input.tree_digest.clone(),
        diff_digest: input.diff_digest.clone(),
        result_digest: input.result_digest.clone(),
        evidence_artifact_digest: input.evidence_artifact_digest.clone(),
        review_digest: input.review_digest.clone(),
        verified_at: verified_at.to_owned(),
        payload_digest: input.result_digest.clone(),
    };
    payload.payload_digest = task_verification_payload_digest(&payload)?;
    Ok(payload)
}

fn task_verification_payload_digest(
    payload: &TaskVerificationPayload,
) -> Result<ContentDigest, LedgerError> {
    hash_value_at_version(
        TASK_VERIFICATION_HASH_DOMAIN,
        TASK_RUNTIME_HASH_VERSION,
        &task_verification_payload_value(payload),
    )
}

fn task_verification_payload_value(
    payload: &TaskVerificationPayload,
) -> lattice_cjson::CanonicalValue {
    object(vec![
        ("schema_version", text(TASK_VERIFICATION_PAYLOAD_SCHEMA)),
        ("task_ref", digest_value(&payload.task_ref)),
        (
            "successor_stream_id",
            digest_value(&payload.successor_stream_id),
        ),
        ("task_spec_digest", digest_value(&payload.task_spec_digest)),
        ("binding_digest", digest_value(&payload.binding_digest)),
        ("attempt_id", text(payload.attempt_id.as_str())),
        ("attempt_number", unsigned(payload.attempt_number)),
        ("outcome", text(payload.outcome.as_str())),
        (
            "verification_profile_digest",
            digest_value(&payload.verification_profile_digest),
        ),
        (
            "base_commit_digest",
            digest_value(&payload.base_commit_digest),
        ),
        (
            "result_commit_digest",
            digest_value(&payload.result_commit_digest),
        ),
        ("tree_digest", digest_value(&payload.tree_digest)),
        ("diff_digest", digest_value(&payload.diff_digest)),
        ("result_digest", digest_value(&payload.result_digest)),
        (
            "evidence_artifact_digest",
            digest_value(&payload.evidence_artifact_digest),
        ),
        (
            "review_digest",
            optional(payload.review_digest.as_ref().map(digest_value)),
        ),
        ("verified_at", text(&payload.verified_at)),
    ])
}

fn verification_append_command(
    expected_head: TaskLedgerStreamHead,
    metadata: TaskRuntimeAppendMetadata,
    payload: &TaskVerificationPayload,
) -> Result<AppendCommand, LedgerError> {
    AppendCommand::new(
        expected_head,
        metadata.command_id,
        metadata.correlation_id,
        metadata.occurred_at,
        LedgerEventKind::EvidenceRecorded,
        ActorId::new("lattice-foreman")?,
        ActionId::new(TASK_VERIFICATION_ACTION)?,
        payload.outcome.ledger_outcome(),
        ReasonCode::new(payload.outcome.reason())?,
        payload.payload_digest.clone(),
        None,
        None,
    )
}

fn validate_verification_event_shape(
    event: &LedgerEvent,
    payload: &TaskVerificationPayload,
) -> Result<(), LedgerError> {
    if event.kind() != LedgerEventKind::EvidenceRecorded
        || event.action().as_str() != TASK_VERIFICATION_ACTION
        || event.outcome() != payload.outcome.ledger_outcome()
        || event.reason_code().as_str() != payload.outcome.reason()
        || event.subject_digest() != &payload.payload_digest
        || event.diagnostic().is_some()
        || event.resource_snapshot().is_some()
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(())
}

fn valid_verification_payload(payload: &TaskVerificationPayload) -> bool {
    payload.attempt_number > 0
        && [
            &payload.verification_profile_digest,
            &payload.base_commit_digest,
            &payload.result_commit_digest,
            &payload.tree_digest,
            &payload.diff_digest,
            &payload.result_digest,
            &payload.evidence_artifact_digest,
        ]
        .into_iter()
        .all(|digest| !is_zero_digest(digest))
        && payload
            .review_digest
            .as_ref()
            .is_none_or(|digest| !is_zero_digest(digest))
}

fn build_attempt_payload(
    binding: &VerifiedTaskExecutionBinding,
    input: &WorkerAttemptInput,
    claimed_at: &str,
) -> Result<WorkerAttemptPayload, LedgerError> {
    let mut payload = WorkerAttemptPayload {
        task_ref: binding.task_ref().clone(),
        successor_stream_id: binding.successor_stream_id().clone(),
        task_spec_digest: binding.task_spec_digest().clone(),
        binding_digest: binding.binding_digest().clone(),
        budget_digest: binding.budget_digest().clone(),
        attempt_id: input.attempt_id.clone(),
        attempt_number: input.attempt_number,
        foreman_generation: input.foreman_generation,
        model: input.model,
        reasoning: input.reasoning,
        model_reason: input.model_reason,
        writer_fence: input.writer_fence,
        foreman_checkpoint_digest: input.foreman_checkpoint_digest.clone(),
        approval_receipt_digest: input.approval_receipt_digest.clone(),
        packet_digest: input.packet_digest.clone(),
        worktree_digest: input.worktree_digest.clone(),
        base_commit_digest: input.base_commit_digest.clone(),
        model_reason_digest: input.model_reason_digest.clone(),
        claimed_at: claimed_at.to_owned(),
        payload_digest: binding.binding_digest().clone(),
    };
    payload.payload_digest = worker_attempt_payload_digest(&payload)?;
    Ok(payload)
}

fn worker_attempt_payload_digest(
    payload: &WorkerAttemptPayload,
) -> Result<ContentDigest, LedgerError> {
    hash_value_at_version(
        WORKER_ATTEMPT_HASH_DOMAIN,
        TASK_RUNTIME_HASH_VERSION,
        &worker_attempt_payload_value(payload),
    )
}

fn no_provider_effect_predecessor_digest(
    predecessor: &VerifiedNoProviderEffectPredecessor,
) -> Result<ContentDigest, LedgerError> {
    hash_value_at_version(
        NO_PROVIDER_EFFECT_PREDECESSOR_HASH_DOMAIN,
        TASK_RUNTIME_HASH_VERSION,
        &object(vec![
            (
                "schema_version",
                text(NO_PROVIDER_EFFECT_PREDECESSOR_SCHEMA),
            ),
            ("owner_profile", text(NO_PROVIDER_EFFECT_CLOSURE_OWNER)),
            ("task_ref", digest_value(&predecessor.task_ref)),
            (
                "successor_stream_id",
                digest_value(&predecessor.successor_stream_id),
            ),
            ("binding_digest", digest_value(&predecessor.binding_digest)),
            (
                "predecessor_attempt_id",
                text(predecessor.predecessor_attempt_id.as_str()),
            ),
            (
                "predecessor_attempt_number",
                unsigned(predecessor.predecessor_attempt_number),
            ),
            (
                "predecessor_writer_fence",
                unsigned(predecessor.predecessor_writer_fence),
            ),
            ("blocker_code", text(&predecessor.blocker_code)),
            (
                "blocker_descriptor_digest",
                digest_value(&predecessor.blocker_descriptor_digest),
            ),
            (
                "reconciliation_proof_descriptor_digest",
                digest_value(&predecessor.reconciliation_proof_descriptor_digest),
            ),
            (
                "successor_packet_digest",
                digest_value(&predecessor.successor_packet_digest),
            ),
        ]),
    )
}

fn worker_attempt_payload_value(payload: &WorkerAttemptPayload) -> lattice_cjson::CanonicalValue {
    object(vec![
        ("schema_version", text(WORKER_ATTEMPT_PAYLOAD_SCHEMA)),
        ("task_ref", digest_value(&payload.task_ref)),
        (
            "successor_stream_id",
            digest_value(&payload.successor_stream_id),
        ),
        ("task_spec_digest", digest_value(&payload.task_spec_digest)),
        ("binding_digest", digest_value(&payload.binding_digest)),
        ("budget_digest", digest_value(&payload.budget_digest)),
        ("attempt_id", text(payload.attempt_id.as_str())),
        ("attempt_number", unsigned(payload.attempt_number)),
        ("foreman_generation", unsigned(payload.foreman_generation)),
        ("model", text(payload.model.as_str())),
        ("reasoning", text(payload.reasoning.as_str())),
        ("model_reason", text(payload.model_reason.as_str())),
        ("writer_fence", unsigned(payload.writer_fence)),
        (
            "foreman_checkpoint_digest",
            digest_value(&payload.foreman_checkpoint_digest),
        ),
        (
            "approval_receipt_digest",
            digest_value(&payload.approval_receipt_digest),
        ),
        ("packet_digest", digest_value(&payload.packet_digest)),
        ("worktree_digest", digest_value(&payload.worktree_digest)),
        (
            "base_commit_digest",
            digest_value(&payload.base_commit_digest),
        ),
        (
            "model_reason_digest",
            digest_value(&payload.model_reason_digest),
        ),
        ("claimed_at", text(&payload.claimed_at)),
    ])
}

fn build_observation_payload(
    binding: &VerifiedTaskExecutionBinding,
    attempt: &VerifiedWorkerAttemptRecord,
    input: &WorkerObservationInput,
    ledger_observed_at: &str,
) -> Result<WorkerObservationPayload, LedgerError> {
    let observed_at = input
        .provider_observed_at
        .as_deref()
        .unwrap_or(ledger_observed_at);
    let mut payload = WorkerObservationPayload {
        task_ref: binding.task_ref().clone(),
        successor_stream_id: binding.successor_stream_id().clone(),
        binding_digest: binding.binding_digest().clone(),
        attempt_id: attempt.attempt_id().clone(),
        attempt_number: input.attempt_number,
        kind: input.kind,
        thread_id: input.thread_id.clone(),
        turn_id: input.turn_id.clone(),
        app_server_generation: input.app_server_generation,
        app_server_identity_digest: input.app_server_identity_digest.clone(),
        observed_at: observed_at.to_owned(),
        evidence_digest: input.evidence_digest.clone(),
        payload_digest: input.evidence_digest.clone(),
    };
    payload.payload_digest = worker_observation_payload_digest(&payload)?;
    Ok(payload)
}

fn worker_observation_payload_digest(
    payload: &WorkerObservationPayload,
) -> Result<ContentDigest, LedgerError> {
    hash_value_at_version(
        WORKER_OBSERVATION_HASH_DOMAIN,
        WORKER_OBSERVATION_HASH_VERSION,
        &worker_observation_payload_value(payload),
    )
}

fn worker_observation_payload_value(
    payload: &WorkerObservationPayload,
) -> lattice_cjson::CanonicalValue {
    object(vec![
        ("schema_version", text(WORKER_OBSERVATION_PAYLOAD_SCHEMA)),
        ("task_ref", digest_value(&payload.task_ref)),
        (
            "successor_stream_id",
            digest_value(&payload.successor_stream_id),
        ),
        ("binding_digest", digest_value(&payload.binding_digest)),
        ("attempt_id", text(payload.attempt_id.as_str())),
        ("attempt_number", unsigned(payload.attempt_number)),
        ("kind", text(payload.kind.as_str())),
        ("thread_id", text(&payload.thread_id)),
        ("turn_id", optional(payload.turn_id.as_ref().map(text))),
        (
            "app_server_generation",
            unsigned(payload.app_server_generation),
        ),
        (
            "app_server_identity_digest",
            digest_value(&payload.app_server_identity_digest),
        ),
        ("observed_at", text(&payload.observed_at)),
        ("evidence_digest", digest_value(&payload.evidence_digest)),
    ])
}

fn attempt_append_command(
    expected_head: TaskLedgerStreamHead,
    metadata: TaskRuntimeAppendMetadata,
    subject_digest: ContentDigest,
) -> Result<AppendCommand, LedgerError> {
    AppendCommand::new(
        expected_head,
        metadata.command_id,
        metadata.correlation_id,
        metadata.occurred_at,
        LedgerEventKind::EffectIntent,
        ActorId::new("lattice-foreman")?,
        ActionId::new(WORKER_ATTEMPT_ACTION)?,
        LedgerOutcome::Recorded,
        ReasonCode::new(WORKER_ATTEMPT_REASON)?,
        subject_digest,
        None,
        None,
    )
}

fn observation_append_command(
    expected_head: TaskLedgerStreamHead,
    metadata: TaskRuntimeAppendMetadata,
    payload: &WorkerObservationPayload,
) -> Result<AppendCommand, LedgerError> {
    AppendCommand::new(
        expected_head,
        metadata.command_id,
        metadata.correlation_id,
        metadata.occurred_at,
        payload.kind.event_kind(),
        ActorId::new("lattice-foreman")?,
        ActionId::new(WORKER_OBSERVATION_ACTION)?,
        payload.kind.outcome(),
        ReasonCode::new(payload.kind.as_str())?,
        payload.payload_digest.clone(),
        None,
        None,
    )
}

fn validate_attempt_event_shape(
    event: &LedgerEvent,
    payload_digest: &ContentDigest,
) -> Result<(), LedgerError> {
    if event.kind() != LedgerEventKind::EffectIntent
        || event.action().as_str() != WORKER_ATTEMPT_ACTION
        || event.outcome() != LedgerOutcome::Recorded
        || event.reason_code().as_str() != WORKER_ATTEMPT_REASON
        || event.subject_digest() != payload_digest
        || event.diagnostic().is_some()
        || event.resource_snapshot().is_some()
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(())
}

fn validate_observation_event_shape(
    event: &LedgerEvent,
    payload: &WorkerObservationPayload,
) -> Result<(), LedgerError> {
    if event.kind() != payload.kind.event_kind()
        || event.action().as_str() != WORKER_OBSERVATION_ACTION
        || event.outcome() != payload.kind.outcome()
        || event.reason_code().as_str() != payload.kind.as_str()
        || event.subject_digest() != &payload.payload_digest
        || event.diagnostic().is_some()
        || event.resource_snapshot().is_some()
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(())
}

fn validate_provider_identity(
    observations: &[VerifiedWorkerObservationRecord],
    payload: &WorkerObservationPayload,
) -> Result<(), LedgerError> {
    let mut ids = BTreeMap::new();
    for observation in observations {
        validate_provider_identity_map(&mut ids, &observation.payload)?;
    }
    validate_provider_identity_map(&mut ids, payload)
}

fn validate_provider_identity_map(
    identities: &mut BTreeMap<u64, (String, Option<String>, u64, ContentDigest)>,
    payload: &WorkerObservationPayload,
) -> Result<(), LedgerError> {
    match identities.get_mut(&payload.attempt_number) {
        None => {
            identities.insert(
                payload.attempt_number,
                (
                    payload.thread_id.clone(),
                    payload.turn_id.clone(),
                    payload.app_server_generation,
                    payload.app_server_identity_digest.clone(),
                ),
            );
        }
        Some((thread, turn, generation, identity_digest)) => {
            if thread != &payload.thread_id
                || turn
                    .as_ref()
                    .is_some_and(|retained| payload.turn_id.as_ref() != Some(retained))
            {
                return Err(LedgerError::WorkerIdentityDrift);
            }
            let app_server_changed = *generation != payload.app_server_generation
                || identity_digest != &payload.app_server_identity_digest;
            if app_server_changed && payload.kind != WorkerObservationKind::Reconciled {
                return Err(LedgerError::WorkerIdentityDrift);
            }
            if app_server_changed {
                *generation = payload.app_server_generation;
                identity_digest.clone_from(&payload.app_server_identity_digest);
            }
            if turn.is_none() {
                turn.clone_from(&payload.turn_id);
            }
        }
    }
    Ok(())
}

fn valid_observation_shape(payload: &WorkerObservationPayload) -> bool {
    payload.attempt_number > 0
        && payload.app_server_generation > 0
        && !is_zero_digest(&payload.app_server_identity_digest)
        && validate_utc_timestamp(&payload.observed_at).is_ok()
        && valid_runtime_identifier(&payload.thread_id)
        && payload
            .turn_id
            .as_deref()
            .is_none_or(valid_runtime_identifier)
        && !is_zero_digest(&payload.evidence_digest)
        && match payload.kind {
            WorkerObservationKind::ThreadAccepted => payload.turn_id.is_none(),
            WorkerObservationKind::TurnAccepted
            | WorkerObservationKind::TurnStarted
            | WorkerObservationKind::PrestartTerminalFailed
            | WorkerObservationKind::MeaningfulProgress
            | WorkerObservationKind::Heartbeat
            | WorkerObservationKind::StallClassified
            | WorkerObservationKind::InterruptRequested
            | WorkerObservationKind::Reconciled
            | WorkerObservationKind::TerminalCompleted
            | WorkerObservationKind::TerminalFailed
            | WorkerObservationKind::TerminalInterrupted => payload.turn_id.is_some(),
        }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkerObservationLifecycle {
    AwaitingThread,
    AwaitingTurn,
    AwaitingExactStart,
    Executing,
    Terminal,
}

fn validate_next_observation_lifecycle(
    observations: &[VerifiedWorkerObservationRecord],
    payload: &WorkerObservationPayload,
) -> Result<(), LedgerError> {
    let mut lifecycle_by_attempt = BTreeMap::new();
    for observation in observations {
        advance_observation_lifecycle(&mut lifecycle_by_attempt, &observation.payload)?;
    }
    advance_observation_lifecycle(&mut lifecycle_by_attempt, payload)
}

fn advance_observation_lifecycle(
    lifecycle_by_attempt: &mut BTreeMap<u64, WorkerObservationLifecycle>,
    payload: &WorkerObservationPayload,
) -> Result<(), LedgerError> {
    use WorkerObservationKind as Kind;
    use WorkerObservationLifecycle as Lifecycle;

    let current = lifecycle_by_attempt
        .get(&payload.attempt_number)
        .copied()
        .unwrap_or(Lifecycle::AwaitingThread);
    let next = match (current, payload.kind) {
        (Lifecycle::AwaitingThread, Kind::ThreadAccepted) => Lifecycle::AwaitingTurn,
        (Lifecycle::AwaitingTurn, Kind::TurnAccepted) => Lifecycle::AwaitingExactStart,
        (Lifecycle::AwaitingExactStart, Kind::TurnStarted) => Lifecycle::Executing,
        (
            Lifecycle::Executing,
            Kind::MeaningfulProgress
            | Kind::Heartbeat
            | Kind::StallClassified
            | Kind::InterruptRequested
            | Kind::Reconciled,
        ) => Lifecycle::Executing,
        (Lifecycle::AwaitingExactStart, Kind::PrestartTerminalFailed)
        | (
            Lifecycle::Executing,
            Kind::TerminalCompleted | Kind::TerminalFailed | Kind::TerminalInterrupted,
        ) => Lifecycle::Terminal,
        _ => return Err(LedgerError::InvalidTaskRuntimeRecord),
    };
    lifecycle_by_attempt.insert(payload.attempt_number, next);
    Ok(())
}

fn ensure_runtime_stream(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
) -> Result<(), LedgerError> {
    if stream.head().stream_id() != binding.successor_stream_id()
        || stream.identity().subject_kind() != TaskLedgerSubjectKind::TaskSpec
        || !stream.events().iter().any(|event| {
            event.event_digest() == binding.link.event_digest()
                && event.subject_digest() == binding.binding_digest()
                && event.action().as_str() == BINDING_ACTION
        })
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(())
}

fn runtime_events<'a>(stream: &'a VerifiedStream, action: &str) -> Vec<&'a LedgerEvent> {
    stream
        .events()
        .iter()
        .filter(|event| event.action().as_str() == action)
        .collect()
}

fn is_managed_runtime_child_action(action: &str) -> bool {
    matches!(
        action,
        BINDING_ACTION
            | WORKER_ATTEMPT_ACTION
            | WORKER_OBSERVATION_ACTION
            | TASK_VERIFICATION_ACTION
            | APPROVAL_EVIDENCE_ACTION
            | ARTIFACT_REFERENCE_ACTION
    )
}

fn retained_command_expected_head(
    stream: &VerifiedStream,
    command_id: &CommandId,
) -> TaskLedgerStreamHead {
    stream
        .commands()
        .iter()
        .find(|record| record.request().command_id() == command_id)
        .map_or_else(
            || stream.head().clone(),
            |record| record.request().expected_head().clone(),
        )
}

fn retained_event_for_command<'a>(
    stream: &'a VerifiedStream,
    action: &str,
    command_id: &CommandId,
) -> Result<&'a LedgerEvent, LedgerError> {
    let events = stream
        .events()
        .iter()
        .filter(|event| event.command_id() == command_id)
        .collect::<Vec<_>>();
    let [event] = events.as_slice() else {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    };
    if event.action().as_str() != action {
        return Err(LedgerError::CommandIdReuse);
    }
    Ok(event)
}

fn validate_runtime_link(
    stream: &VerifiedStream,
    event: &LedgerEvent,
    link: &TaskRuntimeEventLink,
) -> Result<(), LedgerError> {
    let command = stream
        .commands()
        .iter()
        .find(|command| command.request().command_id() == event.command_id())
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    let expected = link_from_event(command.request().expected_head().clone(), event);
    if link != &expected {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(())
}

fn valid_runtime_identifier(value: &str) -> bool {
    valid_identifier(value) && !recognized_secret_text(value)
}

fn validate_lineage(
    intake: &VerifiedStream,
    successor: &VerifiedStream,
    submission: &TaskSubmissionEnvelope,
) -> Result<(), LedgerError> {
    let [intake_event] = intake.events() else {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    };
    if intake.commands().len() != 1
        || intake.identity() != submission.identity()
        || intake.head().stream_id() != submission.stream_id()
        || classify_task_created_profile(intake_event)?
            != Some(TaskCreatedProfile::GeneralTaskIntakeV1)
        || intake_event.subject_digest() != submission.envelope_digest()
        || successor.identity().subject_kind() != TaskLedgerSubjectKind::TaskSpec
        || intake.runtime() != successor.runtime()
        || intake.identity().project_id() != successor.identity().project_id()
        || intake.identity().project_snapshot_id() != successor.identity().project_snapshot_id()
        || intake.identity().task_id() != successor.identity().task_id()
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let created = successor
        .events()
        .first()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    let task_spec_digest = successor
        .identity()
        .task_spec_digest()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
    if created.kind() != LedgerEventKind::TaskCreated
        || classify_task_created_profile(created)? == Some(TaskCreatedProfile::GeneralTaskIntakeV1)
        || created.subject_digest() != task_spec_digest
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(())
}

fn build_binding_payload(
    intake: &VerifiedStream,
    successor: &VerifiedStream,
    submission: &TaskSubmissionEnvelope,
    input: &TaskExecutionBindingInput,
) -> Result<TaskExecutionBindingPayload, LedgerError> {
    let intake_event = &intake.events()[0];
    let successor_event = &successor.events()[0];
    let task_spec_digest = successor
        .identity()
        .task_spec_digest()
        .ok_or(LedgerError::InvalidTaskRuntimeRecord)?
        .clone();
    let mut payload = TaskExecutionBindingPayload {
        task_ref: submission.task_ref().clone(),
        intake_stream_id: submission.stream_id().clone(),
        intake_event_digest: intake_event.event_digest().clone(),
        project_authority_receipt_digest: submission.project_authority_receipt_digest().clone(),
        successor_stream_id: successor.head().stream_id().clone(),
        successor_task_created_event_digest: successor_event.event_digest().clone(),
        task_spec_digest,
        approval_subject_digest: input.approval_subject_digest.clone(),
        budget_digest: input.budget_digest.clone(),
        verification_policy_digest: input.verification_policy_digest.clone(),
        binding_digest: submission.envelope_digest().clone(),
    };
    payload.binding_digest = hash_value_at_version(
        TASK_EXECUTION_BINDING_HASH_DOMAIN,
        TASK_RUNTIME_HASH_VERSION,
        &task_execution_binding_value(&payload),
    )?;
    Ok(payload)
}

fn task_execution_binding_value(
    payload: &TaskExecutionBindingPayload,
) -> lattice_cjson::CanonicalValue {
    object(vec![
        (
            "schema_version",
            text(TASK_EXECUTION_BINDING_PAYLOAD_SCHEMA),
        ),
        ("task_ref", digest_value(&payload.task_ref)),
        ("intake_stream_id", digest_value(&payload.intake_stream_id)),
        (
            "intake_event_digest",
            digest_value(&payload.intake_event_digest),
        ),
        (
            "project_authority_receipt_digest",
            digest_value(&payload.project_authority_receipt_digest),
        ),
        (
            "successor_stream_id",
            digest_value(&payload.successor_stream_id),
        ),
        (
            "successor_task_created_event_digest",
            digest_value(&payload.successor_task_created_event_digest),
        ),
        ("task_spec_digest", digest_value(&payload.task_spec_digest)),
        (
            "approval_subject_digest",
            digest_value(&payload.approval_subject_digest),
        ),
        ("budget_digest", digest_value(&payload.budget_digest)),
        (
            "verification_policy_digest",
            digest_value(&payload.verification_policy_digest),
        ),
    ])
}

fn binding_append_command(
    expected_head: TaskLedgerStreamHead,
    metadata: TaskRuntimeAppendMetadata,
    subject_digest: ContentDigest,
) -> Result<AppendCommand, LedgerError> {
    AppendCommand::new(
        expected_head,
        metadata.command_id,
        metadata.correlation_id,
        metadata.occurred_at,
        LedgerEventKind::EvidenceRecorded,
        ActorId::new("lattice-foreman")?,
        ActionId::new(BINDING_ACTION)?,
        LedgerOutcome::Recorded,
        ReasonCode::new(BINDING_REASON)?,
        subject_digest,
        None,
        None,
    )
}

fn binding_events(stream: &VerifiedStream) -> Vec<&LedgerEvent> {
    stream
        .events()
        .iter()
        .filter(|event| event.action().as_str() == BINDING_ACTION)
        .collect()
}

fn ensure_no_binding_event(stream: &VerifiedStream) -> Result<(), LedgerError> {
    if binding_events(stream).is_empty() {
        Ok(())
    } else {
        Err(LedgerError::InvalidTaskRuntimeRecord)
    }
}

fn validate_binding_event_shape(
    event: &LedgerEvent,
    payload_digest: &ContentDigest,
) -> Result<(), LedgerError> {
    if event.kind() != LedgerEventKind::EvidenceRecorded
        || event.action().as_str() != BINDING_ACTION
        || event.outcome() != LedgerOutcome::Recorded
        || event.reason_code().as_str() != BINDING_REASON
        || event.subject_digest() != payload_digest
        || event.diagnostic().is_some()
        || event.resource_snapshot().is_some()
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(())
}

fn verify_reference_links(
    stream: &VerifiedStream,
    binding: &VerifiedTaskExecutionBinding,
    links: &[TaskRuntimeEventLink],
    action: &str,
    reason: &str,
    require_unique_payloads: bool,
) -> Result<Vec<TaskRuntimeEventLink>, LedgerError> {
    ensure_runtime_stream(stream, binding)?;
    let events = runtime_events(stream, action);
    if events.len() != links.len() {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    let mut verified = Vec::with_capacity(links.len());
    let mut seen_events = BTreeSet::new();
    let mut seen_payloads = BTreeSet::new();
    for event in events {
        let link = links
            .iter()
            .find(|link| link.event_digest == *event.event_digest())
            .ok_or(LedgerError::InvalidTaskRuntimeRecord)?;
        if !seen_events.insert(link.event_digest.as_str().to_owned())
            || (require_unique_payloads
                && !seen_payloads.insert(link.payload_digest.as_str().to_owned()))
            || is_zero_digest(&link.payload_digest)
        {
            return Err(LedgerError::InvalidTaskRuntimeRecord);
        }
        validate_runtime_link(stream, event, link)?;
        validate_reference_event_shape(event, action, reason, &link.payload_digest)?;
        verified.push(link.clone());
    }
    Ok(verified)
}

fn reference_append_command(
    expected_head: TaskLedgerStreamHead,
    metadata: TaskRuntimeAppendMetadata,
    action: &str,
    reason: &str,
    subject_digest: ContentDigest,
) -> Result<AppendCommand, LedgerError> {
    AppendCommand::new(
        expected_head,
        metadata.command_id,
        metadata.correlation_id,
        metadata.occurred_at,
        LedgerEventKind::EvidenceRecorded,
        ActorId::new("lattice-foreman")?,
        ActionId::new(action)?,
        LedgerOutcome::Recorded,
        ReasonCode::new(reason)?,
        subject_digest,
        None,
        None,
    )
}

fn validate_reference_event_shape(
    event: &LedgerEvent,
    action: &str,
    reason: &str,
    subject_digest: &ContentDigest,
) -> Result<(), LedgerError> {
    if event.kind() != LedgerEventKind::EvidenceRecorded
        || event.action().as_str() != action
        || event.outcome() != LedgerOutcome::Recorded
        || event.reason_code().as_str() != reason
        || event.subject_digest() != subject_digest
        || event.diagnostic().is_some()
        || event.resource_snapshot().is_some()
    {
        return Err(LedgerError::InvalidTaskRuntimeRecord);
    }
    Ok(())
}

fn link_from_event(
    expected_head: TaskLedgerStreamHead,
    event: &LedgerEvent,
) -> TaskRuntimeEventLink {
    TaskRuntimeEventLink {
        expected_head,
        stream_id: event.stream_id().clone(),
        event_sequence: event.sequence(),
        event_digest: event.event_digest().clone(),
        command_id: event.command_id().clone(),
        request_digest: event.request_digest().clone(),
        payload_digest: event.subject_digest().clone(),
    }
}
