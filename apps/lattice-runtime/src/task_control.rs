//! PostgreSQL-backed Task Domain lifecycle projection for bounded MCP work.

use std::time::Instant;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, StoreAuthorityHead, SubjectBinding, TaskIngressPeerEvidence,
    TaskLedgerStreamIdentity, WriterLeaseAuthorityHead,
};
use lattice_ports::{
    HermesTaskReflectionCandidatePort, HermesTaskReflectionHistoryPort, TaskLifecycleError,
    TaskLifecycleErrorKind, TaskLifecycleEvidence, TaskLifecyclePort, TaskLifecycleResult,
    TaskReflectionError, TaskReflectionErrorKind, TaskReflectionEventKind,
    TaskReflectionEventReference, TaskReflectionEvidence, TaskReflectionHistory,
    TaskReflectionHistoryEvent, TaskReflectionHistoryQuery, TaskReflectionQueuePort,
    TaskReflectionResult,
};
use lattice_postgres_store::{MigrationTarget, PostgresTaskLedger};
use lattice_task_domain::{
    ReflectionCandidateKind, ReflectionFailureKind, ReflectionState, TaskState,
    reflection_transition, transition,
};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CommandOutcome, CorrelationId, Diagnostic,
    LedgerEvent, LedgerEventKind, LedgerOutcome, OutboxAdmission, ReasonCode, VerifiedStream,
};

use crate::delivery_ledger::{DeliveryDatabaseBinding, connect_fixed_runtime_client};

const CORRELATION_ID: &str = "task038-controlled-codex-canary";
const TASK_CREATED_ACTION: &str = "CONTROLLED_CODEX_CANARY";
const TASK_CREATED_REASON: &str = "TASK038_TASK_ACCEPTED";
const TASK_CREATED_AUDIT_SCHEMA: &str = "lattice.task-created-ingress-audit.v1";
const STATE_REASON: &str = "TASK038_STATE_TRANSITION";
const RESULT_ACTION: &str = "TASK_RESULT";
const RESULT_REASON: &str = "TASK038_FULL_CHAIN_RESULT";
const RESULT_COMMAND_ID: &str = "task038-result";
const INGRESS_HANDOFF_ACTION: &str = "HANDOFF_INGRESS_RECEIPT_V1";
const INGRESS_HANDOFF_REASON: &str = "INGRESS_RECEIPT_HANDOFF_RECORDED";
const INGRESS_HANDOFF_COMMAND_ID: &str = "ingress-receipt-handoff-v1";
const REFLECTION_RUNTIME_ACTOR: &str = "lattice-runtime";
const REFLECTION_BATCH_ACTOR: &str = "lattice-reflection-batch";
const REFLECTION_HERMES_ACTOR: &str = "lattice-hermes-adapter";
const REFLECTION_PENDING_COMMAND_PREFIX: &str = "gh9-reflection-pending:";
const REFLECTION_PENDING_ACTION: &str = "REFLECTION_PENDING";
const REFLECTION_PENDING_REASON: &str = "GH9_REFLECTION_PENDING";
const REFLECTION_CLAIMED_ACTION: &str = "REFLECTION_CLAIMED";
const REFLECTION_CLAIMED_REASON: &str = "GH9_REFLECTION_CLAIMED";
const REFLECTION_RETRY_ACTION: &str = "REFLECTION_RETRY_PENDING";
const REFLECTION_RETRY_REASON: &str = "GH9_REFLECTION_RETRY_PENDING";
const REFLECTION_DEGRADED_ACTION: &str = "REFLECTION_DEGRADED";
const REFLECTION_DEGRADED_REASON: &str = "GH9_REFLECTION_DEGRADED";
const REFLECTION_TASK_FAILURE_ACTION: &str = "REFLECTION_TASK_FAILED";
const REFLECTION_TASK_FAILURE_REASON: &str = "GH9_TASK_FAILURE_RECORDED";
const REFLECTION_OUTPUT_REJECTED_ACTION: &str = "REFLECTION_OUTPUT_REJECTED";
const REFLECTION_OUTPUT_REJECTED_REASON: &str = "GH9_OUTPUT_REJECTED_RECORDED";
const REFLECTION_HERMES_FAILURE_ACTION: &str = "REFLECTION_HERMES_FAILED";
const REFLECTION_HERMES_FAILURE_REASON: &str = "GH9_HERMES_FAILURE_RECORDED";
const REFLECTION_OBSERVATION_ACTION: &str = "REFLECTION_OBSERVATION";
const REFLECTION_INFERENCE_ACTION: &str = "REFLECTION_INFERENCE";
const REFLECTION_ROOT_CAUSE_ACTION: &str = "REFLECTION_ROOT_CAUSE_CANDIDATE";
const REFLECTION_IMPROVEMENT_ACTION: &str = "REFLECTION_IMPROVEMENT_CANDIDATE";
const REFLECTION_CANDIDATE_REASON: &str = "GH9_HERMES_CANDIDATE_RECORDED";
const REFLECTION_EVIDENCE_DIAGNOSTIC_SCHEMA: &str = "lattice.task-reflection.evidence-digest.v1";
const REFLECTION_CANDIDATE_DIAGNOSTIC_SCHEMA: &str = "lattice.task-reflection.candidate-digest.v1";

/// Live Task lifecycle adapter over the same authoritative Task Ledger stream
/// used by the delivery, graph, Hermes, and memory chain.
pub struct PostgresTaskLifecycle {
    ledger: PostgresTaskLedger,
    identity: TaskLedgerStreamIdentity,
    authority: StoreAuthorityHead,
    deadline: Instant,
    ingress_peer: Option<TaskIngressPeerEvidence>,
}

/// Exact verified global persistence identity reused to bind independent
/// same-database extensions without querying caller-selected SQL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskPersistenceFoundation {
    database_identity_digest: ContentDigest,
    global_manifest_digest: ContentDigest,
}

impl TaskPersistenceFoundation {
    #[must_use]
    pub const fn database_identity_digest(&self) -> &ContentDigest {
        &self.database_identity_digest
    }

    #[must_use]
    pub const fn global_manifest_digest(&self) -> &ContentDigest {
        &self.global_manifest_digest
    }
}

impl PostgresTaskLifecycle {
    /// Opens one fixed runtime-role connection without Task ingress authority.
    ///
    /// This compatibility constructor supports persistence-foundation reads.
    /// Task admission, transition, result, and status replay fail closed until
    /// composition uses [`Self::connect_with_ingress_peer`].
    ///
    /// # Errors
    ///
    /// Fails closed for an invalid target, unavailable fixed connection,
    /// rejected schema profile, or expired operation deadline.
    pub fn connect(
        database: &DeliveryDatabaseBinding,
        password: &str,
        deadline: Instant,
        identity: TaskLedgerStreamIdentity,
        authority: StoreAuthorityHead,
    ) -> TaskLifecycleResult<Self> {
        Self::connect_inner(database, password, deadline, identity, authority, None)
    }

    /// Opens one fixed runtime-role connection with server-configured live
    /// Task ingress evidence. MCP request bytes and `clientInfo` never enter
    /// this constructor.
    ///
    /// # Errors
    ///
    /// Fails closed for an invalid target, unavailable fixed connection,
    /// rejected schema profile, or expired operation deadline.
    pub fn connect_with_ingress_peer(
        database: &DeliveryDatabaseBinding,
        password: &str,
        deadline: Instant,
        identity: TaskLedgerStreamIdentity,
        authority: StoreAuthorityHead,
        ingress_peer: TaskIngressPeerEvidence,
    ) -> TaskLifecycleResult<Self> {
        Self::connect_inner(
            database,
            password,
            deadline,
            identity,
            authority,
            Some(ingress_peer),
        )
    }

    fn connect_inner(
        database: &DeliveryDatabaseBinding,
        password: &str,
        deadline: Instant,
        identity: TaskLedgerStreamIdentity,
        authority: StoreAuthorityHead,
        ingress_peer: Option<TaskIngressPeerEvidence>,
    ) -> TaskLifecycleResult<Self> {
        let target = MigrationTarget::new(database.database_name(), database.run_id())
            .map_err(|_| corrupt("LATTICE_TASK_LEDGER_TARGET_REJECTED"))?;
        let client = connect_fixed_runtime_client(database, password, deadline)
            .map_err(|_| unavailable("LATTICE_TASK_LEDGER_CONNECT_REJECTED"))?;
        let ledger = PostgresTaskLedger::new(client, &target).map_err(map_store_error)?;
        ensure_before(deadline)?;
        Ok(Self {
            ledger,
            identity,
            authority,
            deadline,
            ingress_peer,
        })
    }

    fn required_ingress_peer(&self) -> TaskLifecycleResult<TaskIngressPeerEvidence> {
        self.ingress_peer
            .clone()
            .ok_or_else(|| corrupt("LATTICE_TASK_INGRESS_PEER_REQUIRED"))
    }

    fn load_verified(&mut self, binding: &SubjectBinding) -> TaskLifecycleResult<VerifiedStream> {
        ensure_binding(binding, &self.identity)?;
        ensure_before(self.deadline)?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_store_error)?;
        ensure_before(self.deadline)?;
        Ok(loaded.stream().clone())
    }

    /// Replays the same verified stream and returns its exact database/global
    /// manifest commitments for the Writer Lease extension target.
    ///
    /// # Errors
    ///
    /// Fails closed for cross-bound input, an unavailable or corrupt durable
    /// stream, or an expired operation deadline.
    pub fn persistence_foundation(
        &mut self,
        binding: &SubjectBinding,
    ) -> TaskLifecycleResult<TaskPersistenceFoundation> {
        ensure_binding(binding, &self.identity)?;
        ensure_before(self.deadline)?;
        let loaded = self
            .ledger
            .load_stream(self.identity.clone())
            .map_err(map_store_error)?;
        ensure_before(self.deadline)?;
        Ok(TaskPersistenceFoundation {
            database_identity_digest: loaded.persistence().database_identity_digest().clone(),
            global_manifest_digest: loaded.persistence().manifest_digest().clone(),
        })
    }

    fn execute(
        &mut self,
        binding: &SubjectBinding,
        command: AppendCommand,
        writer_authority: Option<&WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        let ingress_peer = self.required_ingress_peer()?;
        ensure_before(self.deadline)?;
        let execution = match writer_authority {
            Some(authority) => {
                self.ledger
                    .execute_fenced(command, self.authority.clone(), authority.clone())
            }
            None => self.ledger.execute(command, self.authority.clone()),
        }
        .map_err(map_store_error)?;
        ensure_after_mutation(self.deadline)?;
        if execution.receipt().outcome() != &CommandOutcome::Appended && !execution.is_exact_retry()
        {
            return Err(rejected("LATTICE_TASK_LEDGER_APPEND_REJECTED"));
        }
        let stream = self.load_verified(binding)?;
        replay_lifecycle(&stream, binding, &ingress_peer)
    }

    /// Appends the sole deterministic successor receipt after replaying a
    /// completed historical stream. No historical event or result is changed.
    pub fn handoff_completed_ingress_receipt(
        &mut self,
        binding: &SubjectBinding,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        let ingress_peer = self.required_ingress_peer()?;
        let stream = self.load_verified(binding)?;
        let core = replay_lifecycle_inner(&stream, binding, &ingress_peer, true)?;
        let result = core
            .result_digest()
            .ok_or_else(|| rejected("LATTICE_TASK_HANDOFF_RESULT_REQUIRED"))?;
        if core.state() != TaskState::Completed {
            return Err(rejected("LATTICE_TASK_HANDOFF_COMPLETION_REQUIRED"));
        }
        let historical = historical_ingress_commitment(&stream)?;
        if historical == task_ingress_profile_adapter_commitment(&ingress_peer)? {
            return Err(rejected("LATTICE_TASK_HANDOFF_NOT_REQUIRED"));
        }
        let command = AppendCommand::new_ingress_receipt_handoff(
            stream.head().clone(),
            CommandId::new(INGRESS_HANDOFF_COMMAND_ID)
                .map_err(|_| corrupt("LATTICE_TASK_HANDOFF_EVIDENCE_REJECTED"))?,
            CorrelationId::new(CORRELATION_ID)
                .map_err(|_| corrupt("LATTICE_TASK_HANDOFF_EVIDENCE_REJECTED"))?,
            "2000-01-01T00:00:21Z",
            ActorId::new(ingress_peer.actor_id().as_str())
                .map_err(|_| corrupt("LATTICE_TASK_HANDOFF_EVIDENCE_REJECTED"))?,
            ingress_receipt_handoff_digest(
                binding,
                &historical,
                &task_ingress_profile_adapter_commitment(&ingress_peer)?,
                result,
            )?,
        )
        .map_err(|_| corrupt("LATTICE_TASK_HANDOFF_EVIDENCE_REJECTED"))?;
        self.execute(binding, command, None)
    }

    fn load_reflection_snapshot(
        &mut self,
        binding: &SubjectBinding,
    ) -> TaskReflectionResult<(VerifiedStream, TaskIngressPeerEvidence, ReplayedReflection)> {
        let ingress_peer = self
            .required_ingress_peer()
            .map_err(reflection_from_lifecycle)?;
        let stream = self
            .load_verified(binding)
            .map_err(reflection_from_lifecycle)?;
        let reflection = replay_reflection(&stream, binding, &ingress_peer)?;
        Ok((stream, ingress_peer, reflection))
    }

    fn execute_reflection(
        &mut self,
        binding: &SubjectBinding,
        command: AppendCommand,
    ) -> TaskReflectionResult<ReplayedReflection> {
        let ingress_peer = self
            .required_ingress_peer()
            .map_err(reflection_from_lifecycle)?;
        ensure_before(self.deadline).map_err(reflection_from_lifecycle)?;
        let execution = self
            .ledger
            .execute(command, self.authority.clone())
            .map_err(|error| reflection_from_lifecycle(map_store_error(error)))?;
        ensure_after_mutation(self.deadline).map_err(reflection_from_lifecycle)?;
        if execution.receipt().outcome() != &CommandOutcome::Appended && !execution.is_exact_retry()
        {
            return Err(reflection_rejected(
                "LATTICE_REFLECTION_LEDGER_APPEND_REJECTED",
            ));
        }
        let stream = self
            .load_verified(binding)
            .map_err(reflection_from_lifecycle)?;
        replay_reflection(&stream, binding, &ingress_peer)
    }
}

impl TaskLifecyclePort for PostgresTaskLifecycle {
    fn admit(
        &mut self,
        binding: &SubjectBinding,
        client_request_id: &str,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        let ingress_peer = self.required_ingress_peer()?;
        let stream = self.load_verified(binding)?;
        let current = replay_lifecycle(&stream, binding, &ingress_peer)?;
        let command_id = format!("mcp-submit:{client_request_id}");
        if let Some(created) = stream
            .events()
            .iter()
            .find(|event| event.kind() == LedgerEventKind::TaskCreated)
        {
            if created.command_id().as_str() != command_id {
                return Err(rejected("LATTICE_TASK_REQUEST_SUBSTITUTED"));
            }
            return Ok(current);
        }
        let command =
            task_created_command(stream.head().clone(), &command_id, binding, &ingress_peer)?;
        self.execute(binding, command, None)
    }

    fn transition(
        &mut self,
        binding: &SubjectBinding,
        from: TaskState,
        to: TaskState,
        writer_authority: Option<&WriterLeaseAuthorityHead>,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        let ingress_peer = self.required_ingress_peer()?;
        transition(from, to).map_err(|_| rejected("LATTICE_TASK_STATE_TRANSITION_REJECTED"))?;
        let stream = self.load_verified(binding)?;
        let current = replay_lifecycle(&stream, binding, &ingress_peer)?;
        if current.state() == to {
            return Ok(current);
        }
        if current.state() != from {
            return Err(rejected("LATTICE_TASK_STATE_STALE"));
        }
        enforce_transition_writer_policy(from, to, writer_authority.is_some())?;
        let action = state_action(to);
        let command_id = state_command_id(to);
        let command = append_command(
            stream.head().clone(),
            &command_id,
            state_timestamp(to),
            LedgerEventKind::StateTransition,
            ingress_peer.actor_id().as_str(),
            &action,
            STATE_REASON,
            transition_digest(binding, from, to)?,
            None,
        )?;
        self.execute(binding, command, writer_authority)
    }

    fn record_result(
        &mut self,
        binding: &SubjectBinding,
        result_digest: &ContentDigest,
        writer_authority: &WriterLeaseAuthorityHead,
    ) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        let ingress_peer = self.required_ingress_peer()?;
        let stream = self.load_verified(binding)?;
        let current = replay_lifecycle(&stream, binding, &ingress_peer)?;
        if let Some(existing) = current.result_digest() {
            if existing == result_digest {
                return Ok(current);
            }
            return Err(rejected("LATTICE_TASK_RESULT_SUBSTITUTED"));
        }
        if current.state() != TaskState::Merging {
            return Err(rejected("LATTICE_TASK_RESULT_STATE_REJECTED"));
        }
        let command = append_command(
            stream.head().clone(),
            RESULT_COMMAND_ID,
            "2000-01-01T00:00:20Z",
            LedgerEventKind::EvidenceRecorded,
            ingress_peer.actor_id().as_str(),
            RESULT_ACTION,
            RESULT_REASON,
            result_digest.clone(),
            None,
        )?;
        self.execute(binding, command, Some(writer_authority))
    }

    fn load(&mut self, binding: &SubjectBinding) -> TaskLifecycleResult<TaskLifecycleEvidence> {
        let ingress_peer = self.required_ingress_peer()?;
        let stream = self.load_verified(binding)?;
        replay_lifecycle(&stream, binding, &ingress_peer)
    }
}

impl TaskReflectionQueuePort for PostgresTaskLifecycle {
    fn ensure_pending(
        &mut self,
        binding: &SubjectBinding,
    ) -> TaskReflectionResult<TaskReflectionEvidence> {
        let ingress_peer = self
            .required_ingress_peer()
            .map_err(reflection_from_lifecycle)?;
        let stream = self
            .load_verified(binding)
            .map_err(reflection_from_lifecycle)?;
        let core =
            replay_lifecycle(&stream, binding, &ingress_peer).map_err(reflection_from_lifecycle)?;
        if core.state() != TaskState::Completed {
            return Err(reflection_rejected("LATTICE_REFLECTION_CORE_NOT_COMPLETED"));
        }
        let result_digest = core
            .result_digest()
            .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_CORE_RESULT_MISSING"))?;
        let generation = match replay_reflection(&stream, binding, &ingress_peer) {
            Ok(reflection) if reflection.evidence.state() == ReflectionState::Pending => {
                return Ok(reflection.evidence);
            }
            Ok(reflection) if reflection.evidence.state() == ReflectionState::RetryPending => {
                reflection
                    .evidence
                    .generation()
                    .checked_add(1)
                    .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_GENERATION_REJECTED"))?
            }
            Ok(_) => {
                return Err(reflection_rejected(
                    "LATTICE_REFLECTION_PENDING_STATE_REJECTED",
                ));
            }
            Err(error) if error.code() == "LATTICE_REFLECTION_PENDING_NOT_ADMITTED" => 0,
            Err(error) => return Err(error),
        };
        let command_id = reflection_pending_command_id(generation);
        let subject =
            reflection_pending_digest(binding, core.core_head_digest(), result_digest, generation)?;
        let command = reflection_append_command(
            stream.head().clone(),
            &command_id,
            "2000-01-01T00:00:30Z",
            LedgerEventKind::EffectIntent,
            REFLECTION_RUNTIME_ACTOR,
            REFLECTION_PENDING_ACTION,
            REFLECTION_PENDING_REASON,
            subject,
            None,
        )?;
        Ok(self.execute_reflection(binding, command)?.evidence)
    }

    fn claim_pending(
        &mut self,
        binding: &SubjectBinding,
        command_id: &str,
    ) -> TaskReflectionResult<TaskReflectionEvidence> {
        validate_external_reflection_command_id(command_id)?;
        let (stream, _ingress_peer, reflection) = self.load_reflection_snapshot(binding)?;
        if let Some(existing) = reflection_command_event(&stream, command_id) {
            validate_existing_reflection_action(existing, REFLECTION_CLAIMED_ACTION)?;
            return Ok(reflection.evidence);
        }
        let admission = reflection_outbox_for_digest(
            &stream,
            reflection
                .evidence
                .pending_admission_digest()
                .ok_or_else(|| {
                    reflection_corrupt("LATTICE_REFLECTION_PENDING_ADMISSION_MISSING")
                })?,
        )?;
        let subject = reflection_claim_digest(
            binding,
            reflection.evidence.core_head_digest(),
            admission,
            reflection.evidence.generation(),
            command_id,
        )?;
        if reflection.evidence.state() != ReflectionState::Pending
            || reflection.evidence.claim_digest().is_some()
        {
            return Err(reflection_rejected(
                "LATTICE_REFLECTION_CLAIM_STATE_REJECTED",
            ));
        }
        let command = reflection_append_command(
            stream.head().clone(),
            command_id,
            "2000-01-01T00:00:31Z",
            LedgerEventKind::EvidenceRecorded,
            REFLECTION_BATCH_ACTOR,
            REFLECTION_CLAIMED_ACTION,
            REFLECTION_CLAIMED_REASON,
            subject,
            None,
        )?;
        Ok(self.execute_reflection(binding, command)?.evidence)
    }

    fn record_failure(
        &mut self,
        binding: &SubjectBinding,
        command_id: &str,
        kind: ReflectionFailureKind,
        evidence_digest: &ContentDigest,
    ) -> TaskReflectionResult<TaskReflectionEvidence> {
        validate_external_reflection_command_id(command_id)?;
        let ingress_peer = self
            .required_ingress_peer()
            .map_err(reflection_from_lifecycle)?;
        let stream = self
            .load_verified(binding)
            .map_err(reflection_from_lifecycle)?;
        let core =
            replay_lifecycle(&stream, binding, &ingress_peer).map_err(reflection_from_lifecycle)?;
        let reflection = match replay_reflection(&stream, binding, &ingress_peer) {
            Ok(reflection) => Some(reflection),
            Err(error)
                if error.code() == "LATTICE_REFLECTION_PENDING_NOT_ADMITTED"
                    && direct_failure_core_allowed(core.state(), kind) =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        let (action, actor, reason) = reflection_failure_write_profile(kind);
        if let Some(existing) = reflection_command_event(&stream, command_id) {
            validate_existing_reflection_evidence(existing, action, evidence_digest)?;
            return Ok(reflection
                .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_REPLAY_MISSING"))?
                .evidence);
        }
        let generation = reflection
            .as_ref()
            .map_or(0, |value| value.evidence.generation());
        let claim = reflection
            .as_ref()
            .and_then(|value| value.evidence.claim_digest());
        if kind == ReflectionFailureKind::HermesFailure && claim.is_none() {
            return Err(reflection_rejected(
                "LATTICE_REFLECTION_FAILURE_CLAIM_REQUIRED",
            ));
        }
        let subject = reflection_failure_digest(
            binding,
            core.core_head_digest(),
            claim,
            generation,
            kind,
            evidence_digest,
        )?;
        reflection_failure_next_state(
            core.state(),
            reflection.as_ref().map(|value| value.evidence.state()),
            claim,
            kind,
        )
        .map_err(reflection_rejected)?;
        let command = reflection_append_command(
            stream.head().clone(),
            command_id,
            "2000-01-01T00:00:32Z",
            LedgerEventKind::EffectOutcome,
            actor,
            action,
            reason,
            subject,
            Some(reflection_evidence_diagnostic(evidence_digest)),
        )?;
        Ok(self.execute_reflection(binding, command)?.evidence)
    }

    fn mark_retry_pending(
        &mut self,
        binding: &SubjectBinding,
        command_id: &str,
    ) -> TaskReflectionResult<TaskReflectionEvidence> {
        validate_external_reflection_command_id(command_id)?;
        let (stream, ingress_peer, reflection) = self.load_reflection_snapshot(binding)?;
        if let Some(existing) = reflection_command_event(&stream, command_id) {
            validate_existing_reflection_action(existing, REFLECTION_RETRY_ACTION)?;
            return Ok(reflection.evidence);
        }
        let core =
            replay_lifecycle(&stream, binding, &ingress_peer).map_err(reflection_from_lifecycle)?;
        if core.state() != TaskState::Completed {
            return Err(reflection_rejected(
                "LATTICE_REFLECTION_RETRY_CORE_STATE_REJECTED",
            ));
        }
        if reflection.evidence.claim_digest().is_none() {
            return Err(reflection_rejected(
                "LATTICE_REFLECTION_RETRY_CLAIM_REQUIRED",
            ));
        }
        let subject = reflection_transition_digest(
            binding,
            reflection.evidence.core_head_digest(),
            reflection.evidence.claim_digest(),
            reflection.evidence.generation(),
            ReflectionState::RetryPending,
        )?;
        reflection_transition(reflection.evidence.state(), ReflectionState::RetryPending)
            .map_err(|_| reflection_rejected("LATTICE_REFLECTION_RETRY_STATE_REJECTED"))?;
        let command = reflection_append_command(
            stream.head().clone(),
            command_id,
            "2000-01-01T00:00:33Z",
            LedgerEventKind::EffectOutcome,
            REFLECTION_RUNTIME_ACTOR,
            REFLECTION_RETRY_ACTION,
            REFLECTION_RETRY_REASON,
            subject,
            None,
        )?;
        Ok(self.execute_reflection(binding, command)?.evidence)
    }

    fn mark_degraded(
        &mut self,
        binding: &SubjectBinding,
        command_id: &str,
        evidence_digest: &ContentDigest,
    ) -> TaskReflectionResult<TaskReflectionEvidence> {
        validate_external_reflection_command_id(command_id)?;
        let (stream, ingress_peer, reflection) = self.load_reflection_snapshot(binding)?;
        if let Some(existing) = reflection_command_event(&stream, command_id) {
            validate_existing_reflection_evidence(
                existing,
                REFLECTION_DEGRADED_ACTION,
                evidence_digest,
            )?;
            return Ok(reflection.evidence);
        }
        let core =
            replay_lifecycle(&stream, binding, &ingress_peer).map_err(reflection_from_lifecycle)?;
        if core.state() != TaskState::Completed {
            return Err(reflection_rejected(
                "LATTICE_REFLECTION_DEGRADED_CORE_STATE_REJECTED",
            ));
        }
        if reflection.evidence.claim_digest().is_none() {
            return Err(reflection_rejected(
                "LATTICE_REFLECTION_DEGRADED_CLAIM_REQUIRED",
            ));
        }
        let subject = reflection_degraded_digest(
            binding,
            reflection.evidence.core_head_digest(),
            reflection.evidence.claim_digest(),
            reflection.evidence.generation(),
            evidence_digest,
        )?;
        reflection_transition(reflection.evidence.state(), ReflectionState::Degraded)
            .map_err(|_| reflection_rejected("LATTICE_REFLECTION_DEGRADED_STATE_REJECTED"))?;
        let command = reflection_append_command(
            stream.head().clone(),
            command_id,
            "2000-01-01T00:00:34Z",
            LedgerEventKind::EffectOutcome,
            REFLECTION_RUNTIME_ACTOR,
            REFLECTION_DEGRADED_ACTION,
            REFLECTION_DEGRADED_REASON,
            subject,
            Some(reflection_evidence_diagnostic(evidence_digest)),
        )?;
        Ok(self.execute_reflection(binding, command)?.evidence)
    }

    fn load_reflection(
        &mut self,
        binding: &SubjectBinding,
    ) -> TaskReflectionResult<TaskReflectionEvidence> {
        Ok(self.load_reflection_snapshot(binding)?.2.evidence)
    }
}

impl HermesTaskReflectionHistoryPort for PostgresTaskLifecycle {
    fn read_authorized_history(
        &mut self,
        binding: &SubjectBinding,
        query: TaskReflectionHistoryQuery,
    ) -> TaskReflectionResult<TaskReflectionHistory> {
        let (_stream, _ingress_peer, reflection) = self.load_reflection_snapshot(binding)?;
        project_authorized_history(binding, &reflection, query)
    }
}

impl HermesTaskReflectionCandidatePort for PostgresTaskLifecycle {
    fn append_candidate(
        &mut self,
        binding: &SubjectBinding,
        command_id: &str,
        kind: ReflectionCandidateKind,
        history_query: TaskReflectionHistoryQuery,
        history_digest: &ContentDigest,
        candidate_digest: &ContentDigest,
    ) -> TaskReflectionResult<TaskReflectionEvidence> {
        validate_external_reflection_command_id(command_id)?;
        let (stream, _ingress_peer, reflection) = self.load_reflection_snapshot(binding)?;
        let action = reflection_candidate_action(kind);
        if let Some(existing) = reflection_command_event(&stream, command_id) {
            validate_existing_reflection_candidate(
                existing,
                action,
                history_query,
                history_digest,
                candidate_digest,
            )?;
            return Ok(reflection.evidence);
        }
        let claim = reflection
            .evidence
            .claim_digest()
            .ok_or_else(|| reflection_rejected("LATTICE_REFLECTION_CANDIDATE_CLAIM_REQUIRED"))?;
        let subject = reflection_candidate_digest(
            binding,
            reflection.evidence.core_head_digest(),
            claim,
            reflection.evidence.generation(),
            kind,
            history_query,
            history_digest,
            candidate_digest,
        )?;
        validate_candidate_history(binding, &reflection, history_query, history_digest)?;
        if reflection.evidence.state() != ReflectionState::Pending {
            return Err(reflection_rejected(
                "LATTICE_REFLECTION_CANDIDATE_STATE_REJECTED",
            ));
        }
        let command = reflection_append_command(
            stream.head().clone(),
            command_id,
            "2000-01-01T00:00:35Z",
            LedgerEventKind::EvidenceRecorded,
            REFLECTION_HERMES_ACTOR,
            action,
            REFLECTION_CANDIDATE_REASON,
            subject,
            Some(reflection_candidate_diagnostic(
                candidate_digest,
                history_query,
                history_digest,
            )),
        )?;
        Ok(self.execute_reflection(binding, command)?.evidence)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransitionWriterPolicy {
    Fenced,
    Unfenced,
}

fn transition_writer_policy(
    from: TaskState,
    to: TaskState,
) -> TaskLifecycleResult<TransitionWriterPolicy> {
    match (from, to) {
        (TaskState::Preparing, TaskState::Executing)
        | (TaskState::Executing, TaskState::Verifying | TaskState::Stopping)
        | (TaskState::Verifying, TaskState::Reviewing)
        | (TaskState::Reviewing, TaskState::AwaitingMergeApproval)
        | (TaskState::AwaitingMergeApproval, TaskState::Merging) => {
            Ok(TransitionWriterPolicy::Fenced)
        }
        (TaskState::Draft, TaskState::AwaitingExecutionApproval)
        | (TaskState::AwaitingExecutionApproval, TaskState::Preparing)
        | (TaskState::Merging, TaskState::Completed)
        | (TaskState::Stopping, TaskState::Failed) => Ok(TransitionWriterPolicy::Unfenced),
        _ => Err(rejected("LATTICE_TASK_STATE_TRANSITION_PROFILE_REJECTED")),
    }
}

fn enforce_transition_writer_policy(
    from: TaskState,
    to: TaskState,
    has_writer_authority: bool,
) -> TaskLifecycleResult<()> {
    match (transition_writer_policy(from, to)?, has_writer_authority) {
        (TransitionWriterPolicy::Fenced, true) | (TransitionWriterPolicy::Unfenced, false) => {
            Ok(())
        }
        (TransitionWriterPolicy::Fenced, false) => Err(rejected(
            "LATTICE_TASK_TRANSITION_WRITER_AUTHORITY_REQUIRED",
        )),
        (TransitionWriterPolicy::Unfenced, true) => Err(rejected(
            "LATTICE_TASK_TRANSITION_WRITER_AUTHORITY_REJECTED",
        )),
    }
}

fn replay_lifecycle(
    stream: &VerifiedStream,
    binding: &SubjectBinding,
    ingress_peer: &TaskIngressPeerEvidence,
) -> TaskLifecycleResult<TaskLifecycleEvidence> {
    replay_lifecycle_inner(stream, binding, ingress_peer, false)
}

fn replay_lifecycle_inner(
    stream: &VerifiedStream,
    binding: &SubjectBinding,
    ingress_peer: &TaskIngressPeerEvidence,
    allow_pending_handoff: bool,
) -> TaskLifecycleResult<TaskLifecycleEvidence> {
    ensure_binding(binding, stream.identity())?;
    let expected_actor = ingress_peer.actor_id().as_str();
    let current_commitment = task_ingress_profile_adapter_commitment(ingress_peer)?;
    let mut state = TaskState::Draft;
    let mut created = false;
    let mut result_digest = None;
    let mut core_head_digest = stream.head().head_digest().clone();
    let mut historical_commitment = None;
    let mut handoff_recorded = false;
    for event in stream.events() {
        match event.kind() {
            LedgerEventKind::TaskCreated => {
                if created
                    || event.actor_id().as_str() != expected_actor
                    || event.action().as_str() != TASK_CREATED_ACTION
                    || event.outcome() != LedgerOutcome::Recorded
                    || event.reason_code().as_str() != TASK_CREATED_REASON
                {
                    return Err(corrupt("LATTICE_TASK_CREATED_EVIDENCE_REJECTED"));
                }
                let historical = historical_ingress_commitment_from_event(event)?;
                if event.subject_digest()
                    != &task_created_subject_digest_for_commitment(binding, &historical)?
                {
                    return Err(corrupt("LATTICE_TASK_INGRESS_PROFILE_COMMITMENT_MISMATCH"));
                }
                let audit = event
                    .diagnostic()
                    .map(Diagnostic::value)
                    .ok_or_else(|| corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"))?;
                if historical == current_commitment {
                    validate_task_created_audit(audit, ingress_peer)?;
                }
                historical_commitment = Some(historical);
                created = true;
                core_head_digest = event_resulting_head_digest(stream, event)?;
            }
            LedgerEventKind::IngressReceiptHandoff => {
                let historical = historical_commitment
                    .as_ref()
                    .ok_or_else(|| corrupt("LATTICE_TASK_HANDOFF_EVIDENCE_REJECTED"))?;
                let result = result_digest
                    .as_ref()
                    .ok_or_else(|| corrupt("LATTICE_TASK_HANDOFF_EVIDENCE_REJECTED"))?;
                if handoff_recorded
                    || event.actor_id().as_str() != expected_actor
                    || event.action().as_str() != INGRESS_HANDOFF_ACTION
                    || event.reason_code().as_str() != INGRESS_HANDOFF_REASON
                    || event.outcome() != LedgerOutcome::Recorded
                    || event.subject_digest()
                        != &ingress_receipt_handoff_digest(
                            binding,
                            historical,
                            &current_commitment,
                            result,
                        )?
                {
                    return Err(corrupt("LATTICE_TASK_HANDOFF_EVIDENCE_REJECTED"));
                }
                handoff_recorded = true;
            }
            LedgerEventKind::StateTransition => {
                if !created
                    || event.actor_id().as_str() != expected_actor
                    || event.outcome() != LedgerOutcome::Recorded
                    || event.reason_code().as_str() != STATE_REASON
                {
                    return Err(corrupt("LATTICE_TASK_STATE_EVIDENCE_REJECTED"));
                }
                let to = parse_state_action(event.action().as_str())?;
                let expected = transition_digest(binding, state, to)?;
                if event.subject_digest() != &expected {
                    return Err(corrupt("LATTICE_TASK_STATE_DIGEST_REJECTED"));
                }
                state = transition(state, to)
                    .map_err(|_| corrupt("LATTICE_TASK_STATE_GRAPH_REJECTED"))?;
                core_head_digest = event_resulting_head_digest(stream, event)?;
            }
            LedgerEventKind::EvidenceRecorded
                if event.action().as_str() == RESULT_ACTION
                    && event.reason_code().as_str() == RESULT_REASON =>
            {
                if !created
                    || event.actor_id().as_str() != expected_actor
                    || event.outcome() != LedgerOutcome::Recorded
                    || result_digest.is_some()
                    || state != TaskState::Merging
                {
                    return Err(corrupt("LATTICE_TASK_RESULT_EVIDENCE_REJECTED"));
                }
                result_digest = Some(event.subject_digest().clone());
                core_head_digest = event_resulting_head_digest(stream, event)?;
            }
            _ => {}
        }
    }
    if !created && !stream.events().is_empty() {
        // A TaskSpec-bound stream may contain no delivery work before admission;
        // any other event without TASK_CREATED is not a valid task authority.
        return Err(corrupt("LATTICE_TASK_ADMISSION_MISSING"));
    }
    if historical_commitment
        .as_ref()
        .is_some_and(|value| value != &current_commitment)
        && !handoff_recorded
        && !allow_pending_handoff
    {
        return Err(corrupt("LATTICE_TASK_INGRESS_PROFILE_COMMITMENT_MISMATCH"));
    }
    Ok(TaskLifecycleEvidence::new_with_core_head(
        binding.clone(),
        created,
        state,
        stream.head().head_digest().clone(),
        core_head_digest,
        result_digest,
    ))
}

fn historical_ingress_commitment(stream: &VerifiedStream) -> TaskLifecycleResult<ContentDigest> {
    stream
        .events()
        .iter()
        .find(|event| event.kind() == LedgerEventKind::TaskCreated)
        .ok_or_else(|| corrupt("LATTICE_TASK_ADMISSION_MISSING"))
        .and_then(historical_ingress_commitment_from_event)
}

fn historical_ingress_commitment_from_event(
    event: &LedgerEvent,
) -> TaskLifecycleResult<ContentDigest> {
    let CanonicalValue::Object(fields) = event
        .diagnostic()
        .map(Diagnostic::value)
        .ok_or_else(|| corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"))?
    else {
        return Err(corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"));
    };
    audit_string_field(fields, "profile_adapter_commitment")
        .and_then(|value| ContentDigest::from_sha256(value.to_owned()).ok())
        .ok_or_else(|| corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"))
}

fn event_resulting_head_digest(
    stream: &VerifiedStream,
    event: &lattice_task_ledger::LedgerEvent,
) -> TaskLifecycleResult<ContentDigest> {
    let receipt = stream
        .receipt(event.command_id())
        .ok_or_else(|| corrupt("LATTICE_TASK_CORE_HEAD_RECEIPT_MISSING"))?;
    if receipt.outcome() != &CommandOutcome::Appended
        || receipt.event_digest() != Some(event.event_digest())
    {
        return Err(corrupt("LATTICE_TASK_CORE_HEAD_RECEIPT_REJECTED"));
    }
    Ok(receipt.after().head_digest().clone())
}

fn event_preceding_head_digest<'a>(
    stream: &'a VerifiedStream,
    event: &LedgerEvent,
) -> TaskReflectionResult<&'a ContentDigest> {
    let receipt = stream
        .receipt(event.command_id())
        .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_RECEIPT_MISSING"))?;
    if receipt.outcome() != &CommandOutcome::Appended
        || receipt.event_digest() != Some(event.event_digest())
    {
        return Err(reflection_corrupt("LATTICE_REFLECTION_RECEIPT_REJECTED"));
    }
    Ok(receipt.before().head_digest())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayedReflection {
    evidence: TaskReflectionEvidence,
    events: Vec<TaskReflectionHistoryEvent>,
}

fn replay_reflection(
    stream: &VerifiedStream,
    binding: &SubjectBinding,
    ingress_peer: &TaskIngressPeerEvidence,
) -> TaskReflectionResult<ReplayedReflection> {
    let core =
        replay_lifecycle(stream, binding, ingress_peer).map_err(reflection_from_lifecycle)?;
    let core_head_digest = core.core_head_digest();
    let mut state = None;
    let mut generation = 0_u64;
    let mut pending_admission = None;
    let mut claim_digest = None;
    let mut events = Vec::new();

    for event in stream.events() {
        let action = event.action().as_str();
        if action != REFLECTION_PENDING_ACTION
            && event
                .command_id()
                .as_str()
                .starts_with(REFLECTION_PENDING_COMMAND_PREFIX)
        {
            return Err(reflection_corrupt(
                "LATTICE_REFLECTION_COMMAND_NAMESPACE_REJECTED",
            ));
        }
        match action {
            REFLECTION_PENDING_ACTION => {
                if core.state() != TaskState::Completed {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_PENDING_CORE_STATE_REJECTED",
                    ));
                }
                let result_digest = core
                    .result_digest()
                    .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_CORE_RESULT_MISSING"))?;
                validate_reflection_event(
                    event,
                    LedgerEventKind::EffectIntent,
                    REFLECTION_RUNTIME_ACTOR,
                    REFLECTION_PENDING_REASON,
                    false,
                )?;
                generation = match state {
                    None => 0,
                    Some(ReflectionState::RetryPending) => {
                        generation.checked_add(1).ok_or_else(|| {
                            reflection_corrupt("LATTICE_REFLECTION_GENERATION_REJECTED")
                        })?
                    }
                    _ => {
                        return Err(reflection_corrupt(
                            "LATTICE_REFLECTION_PENDING_ORDER_REJECTED",
                        ));
                    }
                };
                if event.command_id().as_str() != reflection_pending_command_id(generation) {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_PENDING_COMMAND_REJECTED",
                    ));
                }
                let expected = reflection_pending_digest(
                    binding,
                    core_head_digest,
                    result_digest,
                    generation,
                )?;
                if event.subject_digest() != &expected {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_PENDING_BINDING_REJECTED",
                    ));
                }
                let admission = reflection_outbox_for_event(stream, event)?;
                if admission.intent_digest() != event.subject_digest() {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_PENDING_ADMISSION_REJECTED",
                    ));
                }
                pending_admission = Some(admission.admission_digest().clone());
                claim_digest = None;
                state = Some(ReflectionState::Pending);
                events.push(reflection_history_event(
                    event,
                    generation,
                    TaskReflectionEventKind::Pending,
                    TaskReflectionEventReference::None,
                ));
            }
            REFLECTION_CLAIMED_ACTION => {
                validate_reflection_event(
                    event,
                    LedgerEventKind::EvidenceRecorded,
                    REFLECTION_BATCH_ACTOR,
                    REFLECTION_CLAIMED_REASON,
                    false,
                )?;
                if state != Some(ReflectionState::Pending) || claim_digest.is_some() {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_CLAIM_ORDER_REJECTED",
                    ));
                }
                let admission = reflection_outbox_for_digest(
                    stream,
                    pending_admission.as_ref().ok_or_else(|| {
                        reflection_corrupt("LATTICE_REFLECTION_PENDING_ADMISSION_MISSING")
                    })?,
                )?;
                let expected = reflection_claim_digest(
                    binding,
                    core_head_digest,
                    admission,
                    generation,
                    event.command_id().as_str(),
                )?;
                if event.subject_digest() != &expected {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_CLAIM_BINDING_REJECTED",
                    ));
                }
                claim_digest = Some(expected);
                events.push(reflection_history_event(
                    event,
                    generation,
                    TaskReflectionEventKind::Claimed,
                    TaskReflectionEventReference::None,
                ));
            }
            REFLECTION_TASK_FAILURE_ACTION
            | REFLECTION_OUTPUT_REJECTED_ACTION
            | REFLECTION_HERMES_FAILURE_ACTION => {
                let (failure_kind, actor, reason) = reflection_failure_profile(action)
                    .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_FAILURE_REJECTED"))?;
                validate_reflection_event(
                    event,
                    LedgerEventKind::EffectOutcome,
                    actor,
                    reason,
                    true,
                )?;
                let evidence_digest = reflection_evidence_reference(event)?;
                let expected = reflection_failure_digest(
                    binding,
                    core_head_digest,
                    claim_digest.as_ref(),
                    generation,
                    failure_kind,
                    &evidence_digest,
                )?;
                if event.subject_digest() != &expected {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_FAILURE_BINDING_REJECTED",
                    ));
                }
                state = Some(
                    reflection_failure_next_state(
                        core.state(),
                        state,
                        claim_digest.as_ref(),
                        failure_kind,
                    )
                    .map_err(reflection_corrupt)?,
                );
                events.push(reflection_history_event(
                    event,
                    generation,
                    TaskReflectionEventKind::Failure(failure_kind),
                    TaskReflectionEventReference::Evidence(evidence_digest),
                ));
            }
            REFLECTION_RETRY_ACTION => {
                validate_reflection_event(
                    event,
                    LedgerEventKind::EffectOutcome,
                    REFLECTION_RUNTIME_ACTOR,
                    REFLECTION_RETRY_REASON,
                    false,
                )?;
                if core.state() != TaskState::Completed {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_RETRY_CORE_STATE_REJECTED",
                    ));
                }
                let current = state
                    .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_RETRY_ORDER_REJECTED"))?;
                if claim_digest.is_none() {
                    return Err(reflection_corrupt("LATTICE_REFLECTION_RETRY_CLAIM_MISSING"));
                }
                let expected = reflection_transition_digest(
                    binding,
                    core_head_digest,
                    claim_digest.as_ref(),
                    generation,
                    ReflectionState::RetryPending,
                )?;
                if event.subject_digest() != &expected {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_RETRY_BINDING_REJECTED",
                    ));
                }
                state = Some(
                    reflection_transition(current, ReflectionState::RetryPending).map_err(
                        |_| reflection_corrupt("LATTICE_REFLECTION_RETRY_ORDER_REJECTED"),
                    )?,
                );
                events.push(reflection_history_event(
                    event,
                    generation,
                    TaskReflectionEventKind::RetryPending,
                    TaskReflectionEventReference::None,
                ));
            }
            REFLECTION_DEGRADED_ACTION => {
                validate_reflection_event(
                    event,
                    LedgerEventKind::EffectOutcome,
                    REFLECTION_RUNTIME_ACTOR,
                    REFLECTION_DEGRADED_REASON,
                    true,
                )?;
                if core.state() != TaskState::Completed {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_DEGRADED_CORE_STATE_REJECTED",
                    ));
                }
                let current = state.ok_or_else(|| {
                    reflection_corrupt("LATTICE_REFLECTION_DEGRADED_ORDER_REJECTED")
                })?;
                if claim_digest.is_none() {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_DEGRADED_CLAIM_MISSING",
                    ));
                }
                let evidence_digest = reflection_evidence_reference(event)?;
                let expected = reflection_degraded_digest(
                    binding,
                    core_head_digest,
                    claim_digest.as_ref(),
                    generation,
                    &evidence_digest,
                )?;
                if event.subject_digest() != &expected {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_DEGRADED_BINDING_REJECTED",
                    ));
                }
                state = Some(
                    reflection_transition(current, ReflectionState::Degraded).map_err(|_| {
                        reflection_corrupt("LATTICE_REFLECTION_DEGRADED_ORDER_REJECTED")
                    })?,
                );
                events.push(reflection_history_event(
                    event,
                    generation,
                    TaskReflectionEventKind::Degraded,
                    TaskReflectionEventReference::Evidence(evidence_digest),
                ));
            }
            REFLECTION_OBSERVATION_ACTION
            | REFLECTION_INFERENCE_ACTION
            | REFLECTION_ROOT_CAUSE_ACTION
            | REFLECTION_IMPROVEMENT_ACTION => {
                let candidate_kind = reflection_candidate_profile(action)
                    .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_CANDIDATE_REJECTED"))?;
                validate_reflection_event(
                    event,
                    LedgerEventKind::EvidenceRecorded,
                    REFLECTION_HERMES_ACTOR,
                    REFLECTION_CANDIDATE_REASON,
                    true,
                )?;
                if state != Some(ReflectionState::Pending) {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_CANDIDATE_STATE_REJECTED",
                    ));
                }
                let claim = claim_digest.as_ref().ok_or_else(|| {
                    reflection_corrupt("LATTICE_REFLECTION_CANDIDATE_CLAIM_MISSING")
                })?;
                let (candidate_digest, history_digest, history_query) =
                    reflection_candidate_reference(event)?;
                let preceding_head = event_preceding_head_digest(stream, event)?;
                let preceding_reflection = ReplayedReflection {
                    evidence: TaskReflectionEvidence::new(
                        binding.clone(),
                        ReflectionState::Pending,
                        generation,
                        core_head_digest.clone(),
                        preceding_head.clone(),
                        pending_admission.clone(),
                        claim_digest.clone(),
                    ),
                    events: events.clone(),
                };
                let expected_history =
                    project_authorized_history(binding, &preceding_reflection, history_query)?;
                if &history_digest != expected_history.history_digest() {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_CANDIDATE_HISTORY_REJECTED",
                    ));
                }
                let expected = reflection_candidate_digest(
                    binding,
                    core_head_digest,
                    claim,
                    generation,
                    candidate_kind,
                    history_query,
                    &history_digest,
                    &candidate_digest,
                )?;
                if event.subject_digest() != &expected {
                    return Err(reflection_corrupt(
                        "LATTICE_REFLECTION_CANDIDATE_BINDING_REJECTED",
                    ));
                }
                events.push(reflection_history_event(
                    event,
                    generation,
                    TaskReflectionEventKind::Candidate(candidate_kind),
                    TaskReflectionEventReference::Candidate {
                        candidate_digest,
                        history_digest,
                        history_query,
                    },
                ));
            }
            _ if action.starts_with("REFLECTION_")
                || event.reason_code().as_str().starts_with("GH9_REFLECTION_") =>
            {
                return Err(reflection_corrupt(
                    "LATTICE_REFLECTION_EVENT_SCHEMA_REJECTED",
                ));
            }
            _ => {}
        }
    }

    let state =
        state.ok_or_else(|| reflection_rejected("LATTICE_REFLECTION_PENDING_NOT_ADMITTED"))?;
    Ok(ReplayedReflection {
        evidence: TaskReflectionEvidence::new(
            binding.clone(),
            state,
            generation,
            core_head_digest.clone(),
            stream.head().head_digest().clone(),
            pending_admission,
            claim_digest,
        ),
        events,
    })
}

const fn direct_failure_core_allowed(
    core_state: TaskState,
    failure_kind: ReflectionFailureKind,
) -> bool {
    match failure_kind {
        ReflectionFailureKind::TaskFailure => matches!(
            core_state,
            TaskState::Failed | TaskState::Blocked | TaskState::Cancelled
        ),
        ReflectionFailureKind::OutputRejected => matches!(core_state, TaskState::Failed),
        ReflectionFailureKind::HermesFailure => false,
    }
}

fn reflection_failure_next_state(
    core_state: TaskState,
    current: Option<ReflectionState>,
    claim_digest: Option<&ContentDigest>,
    failure_kind: ReflectionFailureKind,
) -> Result<ReflectionState, &'static str> {
    match (current, failure_kind) {
        (None, kind) if direct_failure_core_allowed(core_state, kind) => {
            Ok(ReflectionState::Failed)
        }
        (Some(ReflectionState::Pending), ReflectionFailureKind::HermesFailure)
            if core_state == TaskState::Completed && claim_digest.is_some() =>
        {
            Ok(ReflectionState::Failed)
        }
        (Some(ReflectionState::Pending), ReflectionFailureKind::HermesFailure) => {
            Err("LATTICE_REFLECTION_FAILURE_CLAIM_MISSING")
        }
        _ => Err("LATTICE_REFLECTION_FAILURE_ORDER_REJECTED"),
    }
}

fn validate_reflection_event(
    event: &LedgerEvent,
    kind: LedgerEventKind,
    actor: &str,
    reason: &str,
    requires_diagnostic: bool,
) -> TaskReflectionResult<()> {
    if event.kind() != kind
        || event.actor_id().as_str() != actor
        || event.outcome() != LedgerOutcome::Recorded
        || event.reason_code().as_str() != reason
        || event.diagnostic().is_some() != requires_diagnostic
        || event.resource_snapshot().is_some()
    {
        return Err(reflection_corrupt(
            "LATTICE_REFLECTION_EVENT_SCHEMA_REJECTED",
        ));
    }
    Ok(())
}

fn reflection_evidence_diagnostic(evidence_digest: &ContentDigest) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "evidence_digest".to_owned(),
            CanonicalValue::String(evidence_digest.as_str().to_owned()),
        ),
        (
            "schema_version".to_owned(),
            CanonicalValue::String(REFLECTION_EVIDENCE_DIAGNOSTIC_SCHEMA.to_owned()),
        ),
    ])
}

fn reflection_candidate_diagnostic(
    candidate_digest: &ContentDigest,
    history_query: TaskReflectionHistoryQuery,
    history_digest: &ContentDigest,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "candidate_digest".to_owned(),
            CanonicalValue::String(candidate_digest.as_str().to_owned()),
        ),
        (
            "history_before_sequence".to_owned(),
            history_query
                .before_sequence()
                .map_or(CanonicalValue::Null, |value| {
                    CanonicalValue::String(value.to_string())
                }),
        ),
        (
            "history_digest".to_owned(),
            CanonicalValue::String(history_digest.as_str().to_owned()),
        ),
        (
            "history_limit".to_owned(),
            CanonicalValue::String(history_query.limit().to_string()),
        ),
        (
            "schema_version".to_owned(),
            CanonicalValue::String(REFLECTION_CANDIDATE_DIAGNOSTIC_SCHEMA.to_owned()),
        ),
    ])
}

fn reflection_diagnostic_fields(
    event: &LedgerEvent,
    expected_len: usize,
) -> TaskReflectionResult<&[(String, CanonicalValue)]> {
    let Some(CanonicalValue::Object(fields)) = event.diagnostic().map(Diagnostic::value) else {
        return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
    };
    if fields.len() != expected_len {
        return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
    }
    Ok(fields)
}

fn reflection_diagnostic_string<'a>(
    fields: &'a [(String, CanonicalValue)],
    key: &str,
) -> TaskReflectionResult<&'a str> {
    let mut values = fields
        .iter()
        .filter(|(candidate, _)| candidate == key)
        .map(|(_, value)| value);
    let Some(CanonicalValue::String(value)) = values.next() else {
        return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
    };
    if values.next().is_some() {
        return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
    }
    Ok(value)
}

fn reflection_evidence_reference(event: &LedgerEvent) -> TaskReflectionResult<ContentDigest> {
    let fields = reflection_diagnostic_fields(event, 2)?;
    if reflection_diagnostic_string(fields, "schema_version")?
        != REFLECTION_EVIDENCE_DIAGNOSTIC_SCHEMA
    {
        return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
    }
    ContentDigest::from_sha256(reflection_diagnostic_string(fields, "evidence_digest")?.to_owned())
        .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"))
}

fn reflection_candidate_reference(
    event: &LedgerEvent,
) -> TaskReflectionResult<(ContentDigest, ContentDigest, TaskReflectionHistoryQuery)> {
    let fields = reflection_diagnostic_fields(event, 5)?;
    if reflection_diagnostic_string(fields, "schema_version")?
        != REFLECTION_CANDIDATE_DIAGNOSTIC_SCHEMA
    {
        return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
    }
    let candidate = ContentDigest::from_sha256(
        reflection_diagnostic_string(fields, "candidate_digest")?.to_owned(),
    )
    .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"))?;
    let history = ContentDigest::from_sha256(
        reflection_diagnostic_string(fields, "history_digest")?.to_owned(),
    )
    .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"))?;
    let before_values = fields
        .iter()
        .filter(|(key, _)| key == "history_before_sequence")
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    let before_sequence = match before_values.as_slice() {
        [CanonicalValue::Null] => None,
        [CanonicalValue::String(value)] => {
            let parsed = value
                .parse::<u64>()
                .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"))?;
            if parsed.to_string() != *value {
                return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
            }
            Some(parsed)
        }
        _ => {
            return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
        }
    };
    let limit_text = reflection_diagnostic_string(fields, "history_limit")?;
    let limit = limit_text
        .parse::<usize>()
        .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"))?;
    if limit.to_string() != limit_text {
        return Err(reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"));
    }
    let query = TaskReflectionHistoryQuery::new(before_sequence, limit)
        .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIAGNOSTIC_REJECTED"))?;
    Ok((candidate, history, query))
}

fn reflection_outbox_for_event<'a>(
    stream: &'a VerifiedStream,
    event: &LedgerEvent,
) -> TaskReflectionResult<&'a OutboxAdmission> {
    let mut matching = stream
        .outboxes()
        .iter()
        .filter(|admission| admission.event_digest() == event.event_digest());
    let admission = matching
        .next()
        .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_PENDING_ADMISSION_MISSING"))?;
    if matching.next().is_some() {
        return Err(reflection_corrupt(
            "LATTICE_REFLECTION_PENDING_ADMISSION_REJECTED",
        ));
    }
    Ok(admission)
}

fn reflection_outbox_for_digest<'a>(
    stream: &'a VerifiedStream,
    admission_digest: &ContentDigest,
) -> TaskReflectionResult<&'a OutboxAdmission> {
    stream
        .outboxes()
        .iter()
        .find(|admission| admission.admission_digest() == admission_digest)
        .ok_or_else(|| reflection_corrupt("LATTICE_REFLECTION_PENDING_ADMISSION_MISSING"))
}

fn reflection_history_event(
    event: &LedgerEvent,
    generation: u64,
    kind: TaskReflectionEventKind,
    reference: TaskReflectionEventReference,
) -> TaskReflectionHistoryEvent {
    TaskReflectionHistoryEvent::new(
        event.sequence(),
        generation,
        kind,
        reference,
        event.subject_digest().clone(),
        event.event_digest().clone(),
    )
}

fn reflection_failure_profile(
    action: &str,
) -> Option<(ReflectionFailureKind, &'static str, &'static str)> {
    match action {
        REFLECTION_TASK_FAILURE_ACTION => Some((
            ReflectionFailureKind::TaskFailure,
            REFLECTION_RUNTIME_ACTOR,
            REFLECTION_TASK_FAILURE_REASON,
        )),
        REFLECTION_OUTPUT_REJECTED_ACTION => Some((
            ReflectionFailureKind::OutputRejected,
            REFLECTION_RUNTIME_ACTOR,
            REFLECTION_OUTPUT_REJECTED_REASON,
        )),
        REFLECTION_HERMES_FAILURE_ACTION => Some((
            ReflectionFailureKind::HermesFailure,
            REFLECTION_HERMES_ACTOR,
            REFLECTION_HERMES_FAILURE_REASON,
        )),
        _ => None,
    }
}

fn reflection_candidate_profile(action: &str) -> Option<ReflectionCandidateKind> {
    match action {
        REFLECTION_OBSERVATION_ACTION => Some(ReflectionCandidateKind::Observation),
        REFLECTION_INFERENCE_ACTION => Some(ReflectionCandidateKind::Inference),
        REFLECTION_ROOT_CAUSE_ACTION => Some(ReflectionCandidateKind::RootCauseCandidate),
        REFLECTION_IMPROVEMENT_ACTION => Some(ReflectionCandidateKind::ImprovementCandidate),
        _ => None,
    }
}

fn reflection_pending_digest(
    binding: &SubjectBinding,
    core_head_digest: &ContentDigest,
    result_digest: &ContentDigest,
    generation: u64,
) -> TaskReflectionResult<ContentDigest> {
    reflection_content_digest(
        "lattice.task-reflection.pending",
        &CanonicalValue::Object(vec![
            (
                "core_head_digest".to_owned(),
                CanonicalValue::String(core_head_digest.as_str().to_owned()),
            ),
            (
                "generation".to_owned(),
                CanonicalValue::String(generation.to_string()),
            ),
            (
                "project_id".to_owned(),
                CanonicalValue::String(binding.project_id().as_str().to_owned()),
            ),
            (
                "project_snapshot_id".to_owned(),
                CanonicalValue::String(binding.project_snapshot_id().as_str().to_owned()),
            ),
            (
                "result_digest".to_owned(),
                CanonicalValue::String(result_digest.as_str().to_owned()),
            ),
            (
                "task_id".to_owned(),
                CanonicalValue::String(binding.task_id().as_str().to_owned()),
            ),
            (
                "task_revision".to_owned(),
                CanonicalValue::String(binding.task_revision().to_owned()),
            ),
            (
                "task_spec_digest".to_owned(),
                CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
            ),
        ]),
    )
}

fn reflection_claim_digest(
    binding: &SubjectBinding,
    core_head_digest: &ContentDigest,
    admission: &OutboxAdmission,
    generation: u64,
    command_id: &str,
) -> TaskReflectionResult<ContentDigest> {
    reflection_content_digest(
        "lattice.task-reflection.claim",
        &CanonicalValue::Object(vec![
            (
                "admission_digest".to_owned(),
                CanonicalValue::String(admission.admission_digest().as_str().to_owned()),
            ),
            (
                "claim_command_id".to_owned(),
                CanonicalValue::String(command_id.to_owned()),
            ),
            (
                "claimant".to_owned(),
                CanonicalValue::String(REFLECTION_BATCH_ACTOR.to_owned()),
            ),
            (
                "core_head_digest".to_owned(),
                CanonicalValue::String(core_head_digest.as_str().to_owned()),
            ),
            (
                "generation".to_owned(),
                CanonicalValue::String(generation.to_string()),
            ),
            (
                "intent_digest".to_owned(),
                CanonicalValue::String(admission.intent_digest().as_str().to_owned()),
            ),
            (
                "task_spec_digest".to_owned(),
                CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
            ),
        ]),
    )
}

fn reflection_failure_digest(
    binding: &SubjectBinding,
    core_head_digest: &ContentDigest,
    claim_digest: Option<&ContentDigest>,
    generation: u64,
    kind: ReflectionFailureKind,
    evidence_digest: &ContentDigest,
) -> TaskReflectionResult<ContentDigest> {
    reflection_content_digest(
        "lattice.task-reflection.failure",
        &CanonicalValue::Object(vec![
            (
                "claim_digest".to_owned(),
                claim_digest.map_or(CanonicalValue::Null, |digest| {
                    CanonicalValue::String(digest.as_str().to_owned())
                }),
            ),
            (
                "core_head_digest".to_owned(),
                CanonicalValue::String(core_head_digest.as_str().to_owned()),
            ),
            (
                "evidence_digest".to_owned(),
                CanonicalValue::String(evidence_digest.as_str().to_owned()),
            ),
            (
                "failure_kind".to_owned(),
                CanonicalValue::String(kind.as_str().to_owned()),
            ),
            (
                "generation".to_owned(),
                CanonicalValue::String(generation.to_string()),
            ),
            (
                "task_spec_digest".to_owned(),
                CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
            ),
        ]),
    )
}

fn reflection_content_digest(
    domain_id: &str,
    value: &CanonicalValue,
) -> TaskReflectionResult<ContentDigest> {
    let domain = HashDomain::new(domain_id, "1.0")
        .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIGEST_REJECTED"))?;
    let digest = canonical_sha256(&domain, value)
        .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIGEST_REJECTED"))?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| reflection_corrupt("LATTICE_REFLECTION_DIGEST_REJECTED"))
}

fn reflection_append_command(
    head: lattice_contracts::TaskLedgerStreamHead,
    command_id: &str,
    occurred_at: &str,
    kind: LedgerEventKind,
    actor: &str,
    action: &str,
    reason: &str,
    subject_digest: ContentDigest,
    diagnostic: Option<CanonicalValue>,
) -> TaskReflectionResult<AppendCommand> {
    append_command(
        head,
        command_id,
        occurred_at,
        kind,
        actor,
        action,
        reason,
        subject_digest,
        diagnostic,
    )
    .map_err(reflection_from_lifecycle)
}

fn reflection_command_event<'a>(
    stream: &'a VerifiedStream,
    command_id: &str,
) -> Option<&'a LedgerEvent> {
    stream
        .events()
        .iter()
        .find(|event| event.command_id().as_str() == command_id)
}

fn reflection_pending_command_id(generation: u64) -> String {
    format!("{REFLECTION_PENDING_COMMAND_PREFIX}{generation}")
}

fn validate_external_reflection_command_id(command_id: &str) -> TaskReflectionResult<()> {
    if command_id.starts_with(REFLECTION_PENDING_COMMAND_PREFIX) {
        return Err(reflection_rejected(
            "LATTICE_REFLECTION_COMMAND_NAMESPACE_REJECTED",
        ));
    }
    Ok(())
}

fn validate_existing_reflection_action(
    event: &LedgerEvent,
    action: &str,
) -> TaskReflectionResult<()> {
    if event.action().as_str() != action {
        return Err(reflection_rejected(
            "LATTICE_REFLECTION_COMMAND_SUBSTITUTED",
        ));
    }
    Ok(())
}

fn validate_existing_reflection_evidence(
    event: &LedgerEvent,
    action: &str,
    evidence_digest: &ContentDigest,
) -> TaskReflectionResult<()> {
    validate_existing_reflection_action(event, action)?;
    if reflection_evidence_reference(event)? != *evidence_digest {
        return Err(reflection_rejected(
            "LATTICE_REFLECTION_COMMAND_SUBSTITUTED",
        ));
    }
    Ok(())
}

fn validate_existing_reflection_candidate(
    event: &LedgerEvent,
    action: &str,
    history_query: TaskReflectionHistoryQuery,
    history_digest: &ContentDigest,
    candidate_digest: &ContentDigest,
) -> TaskReflectionResult<()> {
    validate_existing_reflection_action(event, action)?;
    let (existing_candidate, existing_history, existing_query) =
        reflection_candidate_reference(event)?;
    if existing_candidate != *candidate_digest
        || existing_history != *history_digest
        || existing_query != history_query
    {
        return Err(reflection_rejected(
            "LATTICE_REFLECTION_COMMAND_SUBSTITUTED",
        ));
    }
    Ok(())
}

fn validate_candidate_history(
    binding: &SubjectBinding,
    reflection: &ReplayedReflection,
    history_query: TaskReflectionHistoryQuery,
    history_digest: &ContentDigest,
) -> TaskReflectionResult<()> {
    let authorized_history = project_authorized_history(binding, reflection, history_query)?;
    if authorized_history.history_digest() != history_digest {
        return Err(reflection_rejected(
            "LATTICE_REFLECTION_CANDIDATE_HISTORY_REJECTED",
        ));
    }
    Ok(())
}

const fn reflection_failure_write_profile(
    kind: ReflectionFailureKind,
) -> (&'static str, &'static str, &'static str) {
    match kind {
        ReflectionFailureKind::TaskFailure => (
            REFLECTION_TASK_FAILURE_ACTION,
            REFLECTION_RUNTIME_ACTOR,
            REFLECTION_TASK_FAILURE_REASON,
        ),
        ReflectionFailureKind::OutputRejected => (
            REFLECTION_OUTPUT_REJECTED_ACTION,
            REFLECTION_RUNTIME_ACTOR,
            REFLECTION_OUTPUT_REJECTED_REASON,
        ),
        ReflectionFailureKind::HermesFailure => (
            REFLECTION_HERMES_FAILURE_ACTION,
            REFLECTION_HERMES_ACTOR,
            REFLECTION_HERMES_FAILURE_REASON,
        ),
    }
}

fn reflection_transition_digest(
    binding: &SubjectBinding,
    core_head_digest: &ContentDigest,
    claim_digest: Option<&ContentDigest>,
    generation: u64,
    to: ReflectionState,
) -> TaskReflectionResult<ContentDigest> {
    reflection_content_digest(
        "lattice.task-reflection.transition",
        &CanonicalValue::Object(vec![
            (
                "claim_digest".to_owned(),
                claim_digest.map_or(CanonicalValue::Null, |digest| {
                    CanonicalValue::String(digest.as_str().to_owned())
                }),
            ),
            (
                "core_head_digest".to_owned(),
                CanonicalValue::String(core_head_digest.as_str().to_owned()),
            ),
            (
                "generation".to_owned(),
                CanonicalValue::String(generation.to_string()),
            ),
            (
                "task_spec_digest".to_owned(),
                CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
            ),
            (
                "to".to_owned(),
                CanonicalValue::String(to.as_str().to_owned()),
            ),
        ]),
    )
}

fn reflection_degraded_digest(
    binding: &SubjectBinding,
    core_head_digest: &ContentDigest,
    claim_digest: Option<&ContentDigest>,
    generation: u64,
    evidence_digest: &ContentDigest,
) -> TaskReflectionResult<ContentDigest> {
    reflection_content_digest(
        "lattice.task-reflection.degraded",
        &CanonicalValue::Object(vec![
            (
                "claim_digest".to_owned(),
                claim_digest.map_or(CanonicalValue::Null, |digest| {
                    CanonicalValue::String(digest.as_str().to_owned())
                }),
            ),
            (
                "core_head_digest".to_owned(),
                CanonicalValue::String(core_head_digest.as_str().to_owned()),
            ),
            (
                "evidence_digest".to_owned(),
                CanonicalValue::String(evidence_digest.as_str().to_owned()),
            ),
            (
                "generation".to_owned(),
                CanonicalValue::String(generation.to_string()),
            ),
            (
                "task_spec_digest".to_owned(),
                CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
            ),
        ]),
    )
}

fn reflection_candidate_digest(
    binding: &SubjectBinding,
    core_head_digest: &ContentDigest,
    claim_digest: &ContentDigest,
    generation: u64,
    kind: ReflectionCandidateKind,
    history_query: TaskReflectionHistoryQuery,
    history_digest: &ContentDigest,
    candidate_digest: &ContentDigest,
) -> TaskReflectionResult<ContentDigest> {
    reflection_content_digest(
        "lattice.task-reflection.candidate",
        &CanonicalValue::Object(vec![
            (
                "candidate_digest".to_owned(),
                CanonicalValue::String(candidate_digest.as_str().to_owned()),
            ),
            (
                "candidate_kind".to_owned(),
                CanonicalValue::String(kind.as_str().to_owned()),
            ),
            (
                "claim_digest".to_owned(),
                CanonicalValue::String(claim_digest.as_str().to_owned()),
            ),
            (
                "core_head_digest".to_owned(),
                CanonicalValue::String(core_head_digest.as_str().to_owned()),
            ),
            (
                "generation".to_owned(),
                CanonicalValue::String(generation.to_string()),
            ),
            (
                "history_before_sequence".to_owned(),
                history_query
                    .before_sequence()
                    .map_or(CanonicalValue::Null, |value| {
                        CanonicalValue::String(value.to_string())
                    }),
            ),
            (
                "history_digest".to_owned(),
                CanonicalValue::String(history_digest.as_str().to_owned()),
            ),
            (
                "history_limit".to_owned(),
                CanonicalValue::String(history_query.limit().to_string()),
            ),
            (
                "task_spec_digest".to_owned(),
                CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
            ),
        ]),
    )
}

const fn reflection_candidate_action(kind: ReflectionCandidateKind) -> &'static str {
    match kind {
        ReflectionCandidateKind::Observation => REFLECTION_OBSERVATION_ACTION,
        ReflectionCandidateKind::Inference => REFLECTION_INFERENCE_ACTION,
        ReflectionCandidateKind::RootCauseCandidate => REFLECTION_ROOT_CAUSE_ACTION,
        ReflectionCandidateKind::ImprovementCandidate => REFLECTION_IMPROVEMENT_ACTION,
    }
}

fn reflection_history_digest(
    binding: &SubjectBinding,
    core_head_digest: &ContentDigest,
    journal_head_digest: &ContentDigest,
    query: TaskReflectionHistoryQuery,
    next_before_sequence: Option<u64>,
    events: &[TaskReflectionHistoryEvent],
) -> TaskReflectionResult<ContentDigest> {
    let event_values = events
        .iter()
        .map(|event| {
            CanonicalValue::Object(vec![
                (
                    "event_digest".to_owned(),
                    CanonicalValue::String(event.event_digest().as_str().to_owned()),
                ),
                (
                    "generation".to_owned(),
                    CanonicalValue::String(event.generation().to_string()),
                ),
                (
                    "kind".to_owned(),
                    CanonicalValue::String(reflection_event_kind_text(event.kind())),
                ),
                (
                    "reference".to_owned(),
                    reflection_event_reference_value(event.reference()),
                ),
                (
                    "sequence".to_owned(),
                    CanonicalValue::String(event.sequence().to_string()),
                ),
                (
                    "subject_digest".to_owned(),
                    CanonicalValue::String(event.subject_digest().as_str().to_owned()),
                ),
            ])
        })
        .collect();
    reflection_content_digest(
        "lattice.task-reflection.authorized-history",
        &CanonicalValue::Object(vec![
            (
                "before_sequence".to_owned(),
                query
                    .before_sequence()
                    .map_or(CanonicalValue::Null, |value| {
                        CanonicalValue::String(value.to_string())
                    }),
            ),
            (
                "core_head_digest".to_owned(),
                CanonicalValue::String(core_head_digest.as_str().to_owned()),
            ),
            ("events".to_owned(), CanonicalValue::Array(event_values)),
            (
                "journal_head_digest".to_owned(),
                CanonicalValue::String(journal_head_digest.as_str().to_owned()),
            ),
            (
                "limit".to_owned(),
                CanonicalValue::String(query.limit().to_string()),
            ),
            (
                "next_before_sequence".to_owned(),
                next_before_sequence.map_or(CanonicalValue::Null, |value| {
                    CanonicalValue::String(value.to_string())
                }),
            ),
            (
                "project_id".to_owned(),
                CanonicalValue::String(binding.project_id().as_str().to_owned()),
            ),
            (
                "project_snapshot_id".to_owned(),
                CanonicalValue::String(binding.project_snapshot_id().as_str().to_owned()),
            ),
            (
                "task_id".to_owned(),
                CanonicalValue::String(binding.task_id().as_str().to_owned()),
            ),
            (
                "task_revision".to_owned(),
                CanonicalValue::String(binding.task_revision().to_owned()),
            ),
            (
                "task_spec_digest".to_owned(),
                CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
            ),
        ]),
    )
}

fn reflection_event_reference_value(reference: &TaskReflectionEventReference) -> CanonicalValue {
    match reference {
        TaskReflectionEventReference::None => CanonicalValue::Null,
        TaskReflectionEventReference::Evidence(evidence_digest) => CanonicalValue::Object(vec![
            (
                "evidence_digest".to_owned(),
                CanonicalValue::String(evidence_digest.as_str().to_owned()),
            ),
            (
                "kind".to_owned(),
                CanonicalValue::String("EVIDENCE".to_owned()),
            ),
        ]),
        TaskReflectionEventReference::Candidate {
            candidate_digest,
            history_digest,
            history_query,
        } => CanonicalValue::Object(vec![
            (
                "candidate_digest".to_owned(),
                CanonicalValue::String(candidate_digest.as_str().to_owned()),
            ),
            (
                "history_before_sequence".to_owned(),
                history_query
                    .before_sequence()
                    .map_or(CanonicalValue::Null, |value| {
                        CanonicalValue::String(value.to_string())
                    }),
            ),
            (
                "history_digest".to_owned(),
                CanonicalValue::String(history_digest.as_str().to_owned()),
            ),
            (
                "history_limit".to_owned(),
                CanonicalValue::String(history_query.limit().to_string()),
            ),
            (
                "kind".to_owned(),
                CanonicalValue::String("CANDIDATE".to_owned()),
            ),
        ]),
    }
}

fn project_authorized_history(
    binding: &SubjectBinding,
    reflection: &ReplayedReflection,
    query: TaskReflectionHistoryQuery,
) -> TaskReflectionResult<TaskReflectionHistory> {
    if binding != reflection.evidence.binding() {
        return Err(reflection_rejected(
            "LATTICE_REFLECTION_HISTORY_BINDING_REJECTED",
        ));
    }
    let end = match query.before_sequence() {
        None => reflection.events.len(),
        Some(cursor) => {
            let position = reflection
                .events
                .iter()
                .position(|event| event.sequence() == cursor)
                .ok_or_else(|| reflection_rejected("LATTICE_REFLECTION_HISTORY_CURSOR_REJECTED"))?;
            if position == 0 {
                return Err(reflection_rejected(
                    "LATTICE_REFLECTION_HISTORY_CURSOR_REJECTED",
                ));
            }
            position
        }
    };
    let start = end.saturating_sub(query.limit());
    let next_before_sequence = (start > 0).then(|| reflection.events[start].sequence());
    let events = reflection.events[start..end].to_vec();
    let history_digest = reflection_history_digest(
        binding,
        reflection.evidence.core_head_digest(),
        reflection.evidence.journal_head_digest(),
        query,
        next_before_sequence,
        &events,
    )?;
    Ok(TaskReflectionHistory::new(
        binding.clone(),
        reflection.evidence.core_head_digest().clone(),
        reflection.evidence.journal_head_digest().clone(),
        history_digest,
        query,
        next_before_sequence,
        events,
    ))
}

fn reflection_event_kind_text(kind: TaskReflectionEventKind) -> String {
    match kind {
        TaskReflectionEventKind::Pending => "PENDING".to_owned(),
        TaskReflectionEventKind::Claimed => "CLAIMED".to_owned(),
        TaskReflectionEventKind::Failure(failure) => {
            format!("FAILURE:{}", failure.as_str())
        }
        TaskReflectionEventKind::RetryPending => "RETRY_PENDING".to_owned(),
        TaskReflectionEventKind::Degraded => "DEGRADED".to_owned(),
        TaskReflectionEventKind::Candidate(candidate) => {
            format!("CANDIDATE:{}", candidate.as_str())
        }
    }
}

fn task_created_command(
    head: lattice_contracts::TaskLedgerStreamHead,
    command_id: &str,
    binding: &SubjectBinding,
    ingress_peer: &TaskIngressPeerEvidence,
) -> TaskLifecycleResult<AppendCommand> {
    append_command(
        head,
        command_id,
        "2000-01-01T00:00:00Z",
        LedgerEventKind::TaskCreated,
        ingress_peer.actor_id().as_str(),
        TASK_CREATED_ACTION,
        TASK_CREATED_REASON,
        task_created_subject_digest(binding, ingress_peer)?,
        Some(task_created_audit_value(ingress_peer)?),
    )
}

fn task_created_subject_digest(
    binding: &SubjectBinding,
    ingress_peer: &TaskIngressPeerEvidence,
) -> TaskLifecycleResult<ContentDigest> {
    let profile_adapter_commitment = task_ingress_profile_adapter_commitment(ingress_peer)?;
    task_created_subject_digest_for_commitment(binding, &profile_adapter_commitment)
}

fn task_created_subject_digest_for_commitment(
    binding: &SubjectBinding,
    profile_adapter_commitment: &ContentDigest,
) -> TaskLifecycleResult<ContentDigest> {
    let value = CanonicalValue::Object(vec![
        (
            "project_id".to_owned(),
            CanonicalValue::String(binding.project_id().as_str().to_owned()),
        ),
        (
            "project_snapshot_id".to_owned(),
            CanonicalValue::String(binding.project_snapshot_id().as_str().to_owned()),
        ),
        (
            "task_id".to_owned(),
            CanonicalValue::String(binding.task_id().as_str().to_owned()),
        ),
        (
            "task_revision".to_owned(),
            CanonicalValue::String(binding.task_revision().to_owned()),
        ),
        (
            "task_spec_digest".to_owned(),
            CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
        ),
        (
            "profile_adapter_commitment".to_owned(),
            CanonicalValue::String(profile_adapter_commitment.as_str().to_owned()),
        ),
    ]);
    canonical_content_digest(
        "lattice.task.created-subject",
        "1.0",
        &value,
        "LATTICE_TASK_CREATED_SUBJECT_REJECTED",
    )
}

fn ingress_receipt_handoff_digest(
    binding: &SubjectBinding,
    historical: &ContentDigest,
    successor: &ContentDigest,
    result: &ContentDigest,
) -> TaskLifecycleResult<ContentDigest> {
    canonical_content_digest(
        "lattice.task.ingress-receipt-handoff",
        "1.0",
        &CanonicalValue::Object(vec![
            (
                "historical_profile_adapter_commitment".to_owned(),
                CanonicalValue::String(historical.as_str().to_owned()),
            ),
            (
                "result_digest".to_owned(),
                CanonicalValue::String(result.as_str().to_owned()),
            ),
            (
                "successor_profile_adapter_commitment".to_owned(),
                CanonicalValue::String(successor.as_str().to_owned()),
            ),
            (
                "task_spec_digest".to_owned(),
                CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
            ),
        ]),
        "LATTICE_TASK_HANDOFF_EVIDENCE_REJECTED",
    )
}

fn task_created_audit_value(
    ingress_peer: &TaskIngressPeerEvidence,
) -> TaskLifecycleResult<CanonicalValue> {
    let commitment = task_ingress_profile_adapter_commitment(ingress_peer)?;
    let admission_observation = task_ingress_admission_observation_commitment(
        &commitment,
        ingress_peer.process_start_authority_digest(),
    )?;
    Ok(CanonicalValue::Object(vec![
        (
            "actor_kind".to_owned(),
            CanonicalValue::String(ingress_peer.actor_kind().as_str().to_owned()),
        ),
        (
            "adapter_id".to_owned(),
            CanonicalValue::String(ingress_peer.adapter_id().as_str().to_owned()),
        ),
        (
            "admission_observation_commitment".to_owned(),
            CanonicalValue::String(admission_observation.as_str().to_owned()),
        ),
        (
            "client_kind".to_owned(),
            CanonicalValue::String(ingress_peer.client_kind().as_str().to_owned()),
        ),
        (
            "process_start_authority_digest".to_owned(),
            CanonicalValue::String(
                ingress_peer
                    .process_start_authority_digest()
                    .as_str()
                    .to_owned(),
            ),
        ),
        (
            "profile_adapter_commitment".to_owned(),
            CanonicalValue::String(commitment.as_str().to_owned()),
        ),
        (
            "schema".to_owned(),
            CanonicalValue::String(TASK_CREATED_AUDIT_SCHEMA.to_owned()),
        ),
    ]))
}

fn validate_task_created_audit(
    audit: &CanonicalValue,
    ingress_peer: &TaskIngressPeerEvidence,
) -> TaskLifecycleResult<()> {
    const FIELDS: [&str; 7] = [
        "schema",
        "client_kind",
        "actor_kind",
        "adapter_id",
        "profile_adapter_commitment",
        "process_start_authority_digest",
        "admission_observation_commitment",
    ];
    let CanonicalValue::Object(fields) = audit else {
        return Err(corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"));
    };
    if fields.len() != FIELDS.len()
        || FIELDS
            .iter()
            .any(|expected| fields.iter().filter(|(name, _)| name == expected).count() != 1)
    {
        return Err(corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"));
    }
    let expected_profile_adapter = task_ingress_profile_adapter_commitment(ingress_peer)?;
    for (name, expected) in [
        ("schema", TASK_CREATED_AUDIT_SCHEMA),
        ("client_kind", ingress_peer.client_kind().as_str()),
        ("actor_kind", ingress_peer.actor_kind().as_str()),
        ("adapter_id", ingress_peer.adapter_id().as_str()),
        (
            "profile_adapter_commitment",
            expected_profile_adapter.as_str(),
        ),
    ] {
        if audit_string_field(fields, name) != Some(expected) {
            return Err(corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"));
        }
    }
    let process_start_authority = audit_string_field(fields, "process_start_authority_digest")
        .and_then(|value| ContentDigest::from_sha256(value.to_owned()).ok())
        .filter(|value| !value.as_str().bytes().all(|byte| byte == b'0'))
        .ok_or_else(|| corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"))?;
    let expected_observation = task_ingress_admission_observation_commitment(
        &expected_profile_adapter,
        &process_start_authority,
    )?;
    if audit_string_field(fields, "admission_observation_commitment")
        != Some(expected_observation.as_str())
    {
        return Err(corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"));
    }
    Ok(())
}

fn audit_string_field<'a>(fields: &'a [(String, CanonicalValue)], name: &str) -> Option<&'a str> {
    fields.iter().find_map(|(field, value)| {
        if field != name {
            return None;
        }
        let CanonicalValue::String(value) = value else {
            return None;
        };
        Some(value.as_str())
    })
}

fn task_ingress_admission_observation_commitment(
    profile_adapter_commitment: &ContentDigest,
    process_start_authority_digest: &ContentDigest,
) -> TaskLifecycleResult<ContentDigest> {
    let value = CanonicalValue::Object(vec![
        (
            "profile_adapter_commitment".to_owned(),
            CanonicalValue::String(profile_adapter_commitment.as_str().to_owned()),
        ),
        (
            "process_start_authority_digest".to_owned(),
            CanonicalValue::String(process_start_authority_digest.as_str().to_owned()),
        ),
    ]);
    canonical_content_digest(
        "lattice.task.ingress-admission-observation",
        "1.0",
        &value,
        "LATTICE_TASK_INGRESS_AUDIT_REJECTED",
    )
}

fn task_ingress_profile_adapter_commitment(
    ingress_peer: &TaskIngressPeerEvidence,
) -> TaskLifecycleResult<ContentDigest> {
    if !ingress_peer.runtime().is_live() {
        return Err(corrupt("LATTICE_TASK_INGRESS_PEER_NOT_LIVE"));
    }
    let value = CanonicalValue::Object(vec![
        (
            "runtime".to_owned(),
            CanonicalValue::String("LIVE".to_owned()),
        ),
        (
            "client_kind".to_owned(),
            CanonicalValue::String(ingress_peer.client_kind().as_str().to_owned()),
        ),
        (
            "gateway_instance_id".to_owned(),
            CanonicalValue::String(ingress_peer.gateway_instance_id().as_str().to_owned()),
        ),
        (
            "adapter_id".to_owned(),
            CanonicalValue::String(ingress_peer.adapter_id().as_str().to_owned()),
        ),
        (
            "adapter_version".to_owned(),
            CanonicalValue::String(ingress_peer.adapter_version().to_owned()),
        ),
        (
            "adapter_binary_digest".to_owned(),
            CanonicalValue::String(ingress_peer.adapter_binary_digest().as_str().to_owned()),
        ),
        (
            "schema_digest".to_owned(),
            CanonicalValue::String(ingress_peer.schema_digest().as_str().to_owned()),
        ),
        (
            "actor_id".to_owned(),
            CanonicalValue::String(ingress_peer.actor_id().as_str().to_owned()),
        ),
        (
            "actor_kind".to_owned(),
            CanonicalValue::String(ingress_peer.actor_kind().as_str().to_owned()),
        ),
        (
            "channel_id".to_owned(),
            CanonicalValue::String(ingress_peer.channel_id().as_str().to_owned()),
        ),
        (
            "profile_digest".to_owned(),
            CanonicalValue::String(ingress_peer.profile_digest().as_str().to_owned()),
        ),
    ]);
    canonical_content_digest(
        "lattice.task.ingress-profile-adapter",
        "1.0",
        &value,
        "LATTICE_TASK_INGRESS_COMMITMENT_REJECTED",
    )
}

fn canonical_content_digest(
    domain_id: &str,
    version: &str,
    value: &CanonicalValue,
    error_code: &'static str,
) -> TaskLifecycleResult<ContentDigest> {
    let domain = HashDomain::new(domain_id, version).map_err(|_| corrupt(error_code))?;
    let digest = canonical_sha256(&domain, value).map_err(|_| corrupt(error_code))?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| corrupt(error_code))
}

#[allow(clippy::too_many_arguments)]
fn append_command(
    head: lattice_contracts::TaskLedgerStreamHead,
    command_id: &str,
    occurred_at: &str,
    kind: LedgerEventKind,
    actor_id: &str,
    action: &str,
    reason: &str,
    subject_digest: ContentDigest,
    diagnostic: Option<CanonicalValue>,
) -> TaskLifecycleResult<AppendCommand> {
    AppendCommand::new(
        head,
        CommandId::new(command_id).map_err(|_| corrupt("LATTICE_TASK_COMMAND_REJECTED"))?,
        CorrelationId::new(CORRELATION_ID).map_err(|_| corrupt("LATTICE_TASK_COMMAND_REJECTED"))?,
        occurred_at,
        kind,
        ActorId::new(actor_id).map_err(|_| corrupt("LATTICE_TASK_COMMAND_REJECTED"))?,
        ActionId::new(action).map_err(|_| corrupt("LATTICE_TASK_COMMAND_REJECTED"))?,
        LedgerOutcome::Recorded,
        ReasonCode::new(reason).map_err(|_| corrupt("LATTICE_TASK_COMMAND_REJECTED"))?,
        subject_digest,
        diagnostic
            .map(Diagnostic::new)
            .transpose()
            .map_err(|_| corrupt("LATTICE_TASK_COMMAND_REJECTED"))?,
        None,
    )
    .map_err(|_| corrupt("LATTICE_TASK_COMMAND_REJECTED"))
}

fn transition_digest(
    binding: &SubjectBinding,
    from: TaskState,
    to: TaskState,
) -> TaskLifecycleResult<ContentDigest> {
    let value = CanonicalValue::Object(vec![
        (
            "project_id".to_owned(),
            CanonicalValue::String(binding.project_id().as_str().to_owned()),
        ),
        (
            "project_snapshot_id".to_owned(),
            CanonicalValue::String(binding.project_snapshot_id().as_str().to_owned()),
        ),
        (
            "task_id".to_owned(),
            CanonicalValue::String(binding.task_id().as_str().to_owned()),
        ),
        (
            "task_revision".to_owned(),
            CanonicalValue::String(binding.task_revision().to_owned()),
        ),
        (
            "task_spec_digest".to_owned(),
            CanonicalValue::String(binding.task_spec_digest().as_str().to_owned()),
        ),
        (
            "from".to_owned(),
            CanonicalValue::String(from.as_str().to_owned()),
        ),
        (
            "to".to_owned(),
            CanonicalValue::String(to.as_str().to_owned()),
        ),
    ]);
    let domain = HashDomain::new("lattice.task.state-transition", "1.0")
        .map_err(|_| corrupt("LATTICE_TASK_STATE_DIGEST_REJECTED"))?;
    let digest = canonical_sha256(&domain, &value)
        .map_err(|_| corrupt("LATTICE_TASK_STATE_DIGEST_REJECTED"))?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| corrupt("LATTICE_TASK_STATE_DIGEST_REJECTED"))
}

fn ensure_binding(
    binding: &SubjectBinding,
    identity: &TaskLedgerStreamIdentity,
) -> TaskLifecycleResult<()> {
    if identity.project_id() != binding.project_id()
        || identity.project_snapshot_id() != binding.project_snapshot_id()
        || identity.task_id() != binding.task_id()
        || identity.task_revision() != binding.task_revision()
        || identity.task_spec_digest() != binding.task_spec_digest()
        || identity.accounting_currency() != "TWD"
    {
        return Err(rejected("LATTICE_TASK_BINDING_REJECTED"));
    }
    Ok(())
}

fn state_action(state: TaskState) -> String {
    format!("TASK_STATE_{}", state.as_str())
}

fn parse_state_action(action: &str) -> TaskLifecycleResult<TaskState> {
    let state = action
        .strip_prefix("TASK_STATE_")
        .ok_or_else(|| corrupt("LATTICE_TASK_STATE_EVIDENCE_REJECTED"))?;
    TaskState::parse(state).map_err(|_| corrupt("LATTICE_TASK_STATE_EVIDENCE_REJECTED"))
}

fn state_command_id(state: TaskState) -> String {
    format!("task038-state-{}", state.as_str().to_ascii_lowercase())
}

const fn state_timestamp(state: TaskState) -> &'static str {
    match state {
        TaskState::AwaitingExecutionApproval => "2000-01-01T00:00:01Z",
        TaskState::Preparing => "2000-01-01T00:00:02Z",
        TaskState::Executing => "2000-01-01T00:00:03Z",
        TaskState::Verifying => "2000-01-01T00:00:04Z",
        TaskState::Reviewing => "2000-01-01T00:00:05Z",
        TaskState::AwaitingMergeApproval => "2000-01-01T00:00:06Z",
        TaskState::Merging => "2000-01-01T00:00:07Z",
        TaskState::Completed => "2000-01-01T00:00:08Z",
        TaskState::Rejected => "2000-01-01T00:00:09Z",
        TaskState::Blocked => "2000-01-01T00:00:10Z",
        TaskState::Failed => "2000-01-01T00:00:11Z",
        TaskState::Stopping => "2000-01-01T00:00:12Z",
        TaskState::Cancelled => "2000-01-01T00:00:13Z",
        TaskState::Draft => "2000-01-01T00:00:00Z",
    }
}

fn ensure_before(deadline: Instant) -> TaskLifecycleResult<()> {
    if Instant::now() >= deadline {
        return Err(unavailable("LATTICE_TASK_LEDGER_TIMEOUT"));
    }
    Ok(())
}

fn ensure_after_mutation(deadline: Instant) -> TaskLifecycleResult<()> {
    if Instant::now() >= deadline {
        return Err(ambiguous("LATTICE_TASK_LEDGER_POST_MUTATION_TIMEOUT"));
    }
    Ok(())
}

fn map_store_error(error: lattice_postgres_store::PostgresTaskLedgerError) -> TaskLifecycleError {
    use lattice_postgres_store::PostgresTaskLedgerErrorKind as Kind;
    match error.kind() {
        Kind::CommandSubstitution | Kind::AdmissionDenied | Kind::AuthorityMismatch => {
            rejected("LATTICE_TASK_LEDGER_REJECTED")
        }
        Kind::CommitOutcomeUnknown => ambiguous("LATTICE_TASK_LEDGER_COMMIT_UNKNOWN"),
        Kind::Malformed
        | Kind::PhysicalStateMismatch
        | Kind::CheckpointCorrupt
        | Kind::RetainedRowCorrupt => corrupt("LATTICE_TASK_LEDGER_CORRUPT"),
        Kind::RevisionOverflow
        | Kind::SerializationExhausted
        | Kind::TransactionFailed
        | Kind::Unavailable => unavailable("LATTICE_TASK_LEDGER_UNAVAILABLE"),
    }
}

const fn rejected(code: &'static str) -> TaskLifecycleError {
    TaskLifecycleError::new(TaskLifecycleErrorKind::Rejected, code)
}

const fn unavailable(code: &'static str) -> TaskLifecycleError {
    TaskLifecycleError::new(TaskLifecycleErrorKind::Unavailable, code)
}

const fn ambiguous(code: &'static str) -> TaskLifecycleError {
    TaskLifecycleError::new(TaskLifecycleErrorKind::Ambiguous, code)
}

const fn corrupt(code: &'static str) -> TaskLifecycleError {
    TaskLifecycleError::new(TaskLifecycleErrorKind::Corrupt, code)
}

const fn reflection_rejected(code: &'static str) -> TaskReflectionError {
    TaskReflectionError::new(TaskReflectionErrorKind::Rejected, code)
}

const fn reflection_corrupt(code: &'static str) -> TaskReflectionError {
    TaskReflectionError::new(TaskReflectionErrorKind::Corrupt, code)
}

fn reflection_from_lifecycle(error: TaskLifecycleError) -> TaskReflectionError {
    let kind = match error.kind() {
        TaskLifecycleErrorKind::Rejected => TaskReflectionErrorKind::Rejected,
        TaskLifecycleErrorKind::Unavailable => TaskReflectionErrorKind::Unavailable,
        TaskLifecycleErrorKind::Ambiguous => TaskReflectionErrorKind::Ambiguous,
        TaskLifecycleErrorKind::Corrupt => TaskReflectionErrorKind::Corrupt,
    };
    TaskReflectionError::new(kind, error.code())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_contracts::{
        GatewayChannelId, GatewayInstanceId, ProjectId, ProjectSnapshotId, TaskId,
        TaskIngressPeerEvidence,
    };
    use lattice_task_ledger::{FakeTaskLedger, verify_untrusted_snapshot};

    #[test]
    fn post_mutation_deadline_is_ambiguous() {
        let error = ensure_after_mutation(Instant::now()).expect_err("expired mutation deadline");
        assert_eq!(error.kind(), TaskLifecycleErrorKind::Ambiguous);
        assert_eq!(error.code(), "LATTICE_TASK_LEDGER_POST_MUTATION_TIMEOUT");
    }

    #[test]
    fn controlled_transition_profile_enforces_the_writer_boundary() {
        for (from, to) in [
            (TaskState::Preparing, TaskState::Executing),
            (TaskState::Executing, TaskState::Verifying),
            (TaskState::Executing, TaskState::Stopping),
            (TaskState::Verifying, TaskState::Reviewing),
            (TaskState::Reviewing, TaskState::AwaitingMergeApproval),
            (TaskState::AwaitingMergeApproval, TaskState::Merging),
        ] {
            assert_eq!(
                transition_writer_policy(from, to).unwrap(),
                TransitionWriterPolicy::Fenced
            );
            let error = enforce_transition_writer_policy(from, to, false)
                .expect_err("fenced transition must reject missing authority");
            assert_eq!(error.kind(), TaskLifecycleErrorKind::Rejected);
            assert_eq!(
                error.code(),
                "LATTICE_TASK_TRANSITION_WRITER_AUTHORITY_REQUIRED"
            );
            enforce_transition_writer_policy(from, to, true)
                .expect("fenced transition accepts authority");
        }

        for (from, to) in [
            (TaskState::Draft, TaskState::AwaitingExecutionApproval),
            (TaskState::AwaitingExecutionApproval, TaskState::Preparing),
            (TaskState::Merging, TaskState::Completed),
            (TaskState::Stopping, TaskState::Failed),
        ] {
            assert_eq!(
                transition_writer_policy(from, to).unwrap(),
                TransitionWriterPolicy::Unfenced
            );
            enforce_transition_writer_policy(from, to, false)
                .expect("post-release or pre-acquire transition is unfenced");
            let error = enforce_transition_writer_policy(from, to, true)
                .expect_err("unfenced transition must reject ambient authority");
            assert_eq!(error.kind(), TaskLifecycleErrorKind::Rejected);
            assert_eq!(
                error.code(),
                "LATTICE_TASK_TRANSITION_WRITER_AUTHORITY_REJECTED"
            );
        }

        for (from, to) in [
            (TaskState::Draft, TaskState::Cancelled),
            (TaskState::Executing, TaskState::Failed),
            (TaskState::Merging, TaskState::Stopping),
        ] {
            transition(from, to).expect("edge remains legal in Task Domain");
            let error = transition_writer_policy(from, to)
                .expect_err("bounded controlled profile rejects unused edges");
            assert_eq!(error.kind(), TaskLifecycleErrorKind::Rejected);
            assert_eq!(
                error.code(),
                "LATTICE_TASK_STATE_TRANSITION_PROFILE_REJECTED"
            );
        }
    }

    fn binding() -> SubjectBinding {
        SubjectBinding::new(
            ProjectId::new("task038-project").unwrap(),
            ProjectSnapshotId::new("task038-snapshot").unwrap(),
            TaskId::new("TASK-038").unwrap(),
            "1",
            ContentDigest::from_sha256("1".repeat(64)).unwrap(),
        )
        .unwrap()
    }

    fn identity(binding: &SubjectBinding) -> TaskLedgerStreamIdentity {
        TaskLedgerStreamIdentity::new(
            binding.project_id().clone(),
            binding.project_snapshot_id().clone(),
            binding.task_id().clone(),
            binding.task_revision(),
            binding.task_spec_digest().clone(),
            "TWD",
        )
        .unwrap()
    }

    fn ingress_peer(adapter: char, profile: char) -> TaskIngressPeerEvidence {
        ingress_peer_with_authority(adapter, profile, 'd')
    }

    fn ingress_peer_with_authority(
        adapter: char,
        profile: char,
        authority: char,
    ) -> TaskIngressPeerEvidence {
        TaskIngressPeerEvidence::new_chatgpt_secure_mcp_tunnel_live(
            GatewayInstanceId::new("lattice-mcp-production").unwrap(),
            "1.0.0",
            ContentDigest::from_sha256(adapter.to_string().repeat(64)).unwrap(),
            ContentDigest::from_sha256("b".repeat(64)).unwrap(),
            GatewayChannelId::new("stdio").unwrap(),
            ContentDigest::from_sha256(profile.to_string().repeat(64)).unwrap(),
            ContentDigest::from_sha256(authority.to_string().repeat(64)).unwrap(),
        )
        .unwrap()
    }

    fn local_ingress_peer() -> TaskIngressPeerEvidence {
        TaskIngressPeerEvidence::new_local_canonical_mcp_acceptance_live(
            GatewayInstanceId::new("lattice-mcp-local-acceptance").unwrap(),
            "1.0.0",
            ContentDigest::from_sha256("a".repeat(64)).unwrap(),
            ContentDigest::from_sha256("b".repeat(64)).unwrap(),
            GatewayChannelId::new("stdio").unwrap(),
            ContentDigest::from_sha256("c".repeat(64)).unwrap(),
            ContentDigest::from_sha256("d".repeat(64)).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn task_created_audit_keys_are_byte_lexicographic() {
        let audit = task_created_audit_value(&local_ingress_peer()).unwrap();
        let CanonicalValue::Object(entries) = audit else {
            panic!("task-created audit must remain an object");
        };
        let keys = entries
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>();
        let mut canonical_keys = keys.clone();
        canonical_keys.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        assert_eq!(keys, canonical_keys);
    }

    fn verified(fake: &FakeTaskLedger, identity: &TaskLedgerStreamIdentity) -> VerifiedStream {
        let head = FakeTaskLedger::zero_head(identity.clone()).unwrap();
        match fake.untrusted_snapshot(head.stream_id()) {
            Ok(snapshot) => verify_untrusted_snapshot(&snapshot).unwrap(),
            Err(_) => {
                VerifiedStream::vacant(identity.clone(), lattice_contracts::RuntimeKind::Fake)
                    .unwrap()
            }
        }
    }

    fn append_transition(
        fake: &mut FakeTaskLedger,
        identity: &TaskLedgerStreamIdentity,
        binding: &SubjectBinding,
        ingress_peer: &TaskIngressPeerEvidence,
        from: TaskState,
        to: TaskState,
    ) {
        let stream = verified(fake, identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                &state_command_id(to),
                state_timestamp(to),
                LedgerEventKind::StateTransition,
                ingress_peer.actor_id().as_str(),
                &state_action(to),
                STATE_REASON,
                transition_digest(binding, from, to).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
    }

    fn completed_fake() -> (
        SubjectBinding,
        TaskLedgerStreamIdentity,
        TaskIngressPeerEvidence,
        FakeTaskLedger,
        ContentDigest,
    ) {
        let binding = binding();
        let identity = identity(&binding);
        let ingress_peer = ingress_peer('a', 'c');
        let mut fake = FakeTaskLedger::new();
        let stream = verified(&fake, &identity);
        fake.execute(
            task_created_command(
                stream.head().clone(),
                "mcp-submit:req-1",
                &binding,
                &ingress_peer,
            )
            .unwrap(),
        )
        .unwrap();
        for (from, to) in [
            (TaskState::Draft, TaskState::AwaitingExecutionApproval),
            (TaskState::AwaitingExecutionApproval, TaskState::Preparing),
            (TaskState::Preparing, TaskState::Executing),
            (TaskState::Executing, TaskState::Verifying),
            (TaskState::Verifying, TaskState::Reviewing),
            (TaskState::Reviewing, TaskState::AwaitingMergeApproval),
            (TaskState::AwaitingMergeApproval, TaskState::Merging),
        ] {
            append_transition(&mut fake, &identity, &binding, &ingress_peer, from, to);
        }
        let result = ContentDigest::from_sha256("2".repeat(64)).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                RESULT_COMMAND_ID,
                "2000-01-01T00:00:20Z",
                LedgerEventKind::EvidenceRecorded,
                ingress_peer.actor_id().as_str(),
                RESULT_ACTION,
                RESULT_REASON,
                result.clone(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        append_transition(
            &mut fake,
            &identity,
            &binding,
            &ingress_peer,
            TaskState::Merging,
            TaskState::Completed,
        );
        (binding, identity, ingress_peer, fake, result)
    }

    #[test]
    fn completed_legacy_task_requires_one_verified_successor_handoff() {
        let (binding, identity, ingress_peer, mut fake, result) = completed_fake();
        let successor = ingress_peer_with_authority('a', 'e', 'd');
        assert_eq!(
            replay_lifecycle(&verified(&fake, &identity), &binding, &successor)
                .expect_err("missing handoff must fail")
                .code(),
            "LATTICE_TASK_INGRESS_PROFILE_COMMITMENT_MISMATCH"
        );
        let historical = task_ingress_profile_adapter_commitment(&ingress_peer).unwrap();
        let successor_commitment = task_ingress_profile_adapter_commitment(&successor).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            AppendCommand::new_ingress_receipt_handoff(
                stream.head().clone(),
                CommandId::new(INGRESS_HANDOFF_COMMAND_ID).unwrap(),
                CorrelationId::new(CORRELATION_ID).unwrap(),
                "2000-01-01T00:00:21Z",
                ActorId::new(successor.actor_id().as_str()).unwrap(),
                ingress_receipt_handoff_digest(
                    &binding,
                    &historical,
                    &successor_commitment,
                    &result,
                )
                .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        let replayed = replay_lifecycle(&verified(&fake, &identity), &binding, &successor)
            .expect("verified successor handoff replays");
        assert_eq!(replayed.state(), TaskState::Completed);
        assert_eq!(replayed.result_digest(), Some(&result));
    }

    fn claimed_fake() -> (
        SubjectBinding,
        TaskLedgerStreamIdentity,
        TaskIngressPeerEvidence,
        FakeTaskLedger,
        ContentDigest,
        ContentDigest,
    ) {
        let (binding, identity, ingress_peer, mut fake, result) = completed_fake();
        let core = replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        let core_head = core.core_head_digest().clone();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                &reflection_pending_command_id(0),
                "2000-01-01T00:00:30Z",
                LedgerEventKind::EffectIntent,
                REFLECTION_RUNTIME_ACTOR,
                REFLECTION_PENDING_ACTION,
                REFLECTION_PENDING_REASON,
                reflection_pending_digest(&binding, &core_head, &result, 0).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let stream = verified(&fake, &identity);
        let admission = stream.outboxes().last().expect("pending admission");
        let claim =
            reflection_claim_digest(&binding, &core_head, admission, 0, "gh9-reflection-claim:0")
                .unwrap();
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-claim:0",
                "2000-01-01T00:00:31Z",
                LedgerEventKind::EvidenceRecorded,
                REFLECTION_BATCH_ACTOR,
                REFLECTION_CLAIMED_ACTION,
                REFLECTION_CLAIMED_REASON,
                claim.clone(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        (binding, identity, ingress_peer, fake, result, claim)
    }

    fn append_observation_candidate(
        fake: &mut FakeTaskLedger,
        identity: &TaskLedgerStreamIdentity,
        binding: &SubjectBinding,
        ingress_peer: &TaskIngressPeerEvidence,
        claim: &ContentDigest,
        index: u64,
        history_limit: usize,
    ) -> (TaskReflectionHistoryQuery, ContentDigest, ContentDigest) {
        let reflection = replay_reflection(&verified(fake, identity), binding, ingress_peer)
            .expect("candidate pre-state");
        let query = TaskReflectionHistoryQuery::latest(history_limit).expect("history query");
        let history =
            project_authorized_history(binding, &reflection, query).expect("candidate history");
        let history_digest = history.history_digest().clone();
        let candidate_digest =
            ContentDigest::from_sha256(format!("{index:064x}")).expect("candidate digest");
        let core_head = reflection.evidence.core_head_digest().clone();
        let generation = reflection.evidence.generation();
        let stream = verified(fake, identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                &format!("gh9-reflection-candidate:{index}"),
                "2000-01-01T00:00:35Z",
                LedgerEventKind::EvidenceRecorded,
                REFLECTION_HERMES_ACTOR,
                REFLECTION_OBSERVATION_ACTION,
                REFLECTION_CANDIDATE_REASON,
                reflection_candidate_digest(
                    binding,
                    &core_head,
                    claim,
                    generation,
                    ReflectionCandidateKind::Observation,
                    query,
                    &history_digest,
                    &candidate_digest,
                )
                .expect("candidate subject"),
                Some(reflection_candidate_diagnostic(
                    &candidate_digest,
                    query,
                    &history_digest,
                )),
            )
            .expect("candidate command"),
        )
        .expect("candidate append");
        (query, history_digest, candidate_digest)
    }

    fn terminal_fake(
        terminal: TaskState,
    ) -> (
        SubjectBinding,
        TaskLedgerStreamIdentity,
        TaskIngressPeerEvidence,
        FakeTaskLedger,
    ) {
        let binding = binding();
        let identity = identity(&binding);
        let ingress_peer = ingress_peer('a', 'c');
        let mut fake = FakeTaskLedger::new();
        let stream = verified(&fake, &identity);
        fake.execute(
            task_created_command(
                stream.head().clone(),
                "mcp-submit:req-1",
                &binding,
                &ingress_peer,
            )
            .unwrap(),
        )
        .unwrap();
        let transitions = match terminal {
            TaskState::Failed => vec![
                (TaskState::Draft, TaskState::AwaitingExecutionApproval),
                (TaskState::AwaitingExecutionApproval, TaskState::Preparing),
                (TaskState::Preparing, TaskState::Executing),
                (TaskState::Executing, TaskState::Stopping),
                (TaskState::Stopping, TaskState::Failed),
            ],
            _ => panic!("unsupported terminal test state"),
        };
        for (from, to) in transitions {
            append_transition(&mut fake, &identity, &binding, &ingress_peer, from, to);
        }
        (binding, identity, ingress_peer, fake)
    }

    fn replay_created_override(
        actor_id: &str,
        action: &str,
        reason: &str,
        diagnostic: Option<CanonicalValue>,
    ) -> TaskLifecycleError {
        let binding = binding();
        let identity = identity(&binding);
        let ingress_peer = ingress_peer('a', 'c');
        let mut fake = FakeTaskLedger::new();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "mcp-submit:req-1",
                "2000-01-01T00:00:00Z",
                LedgerEventKind::TaskCreated,
                actor_id,
                action,
                reason,
                task_created_subject_digest(&binding, &ingress_peer).unwrap(),
                diagnostic,
            )
            .unwrap(),
        )
        .unwrap();
        replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap_err()
    }

    #[test]
    fn same_request_replays_after_restart_with_the_same_fixed_profile_commitment() {
        let binding = binding();
        let identity = identity(&binding);
        let mut fake = FakeTaskLedger::new();
        let vacant = verified(&fake, &identity);
        let first_process_peer = ingress_peer_with_authority('a', 'c', 'd');
        fake.execute(
            task_created_command(
                vacant.head().clone(),
                "mcp-submit:req-1",
                &binding,
                &first_process_peer,
            )
            .unwrap(),
        )
        .unwrap();

        let restarted_process_peer = ingress_peer_with_authority('a', 'c', 'f');
        let stream = verified(&fake, &identity);
        let created = stream.events().first().expect("task created");
        assert_eq!(created.command_id().as_str(), "mcp-submit:req-1");
        assert_ne!(created.subject_digest(), binding.task_spec_digest());
        assert_eq!(
            created.diagnostic().expect("audit diagnostic").value(),
            &task_created_audit_value(&first_process_peer).unwrap()
        );

        let evidence = replay_lifecycle(&stream, &binding, &restarted_process_peer).unwrap();
        assert!(evidence.admitted());
    }

    #[test]
    fn replay_fails_closed_when_the_fixed_profile_commitment_changes() {
        let binding = binding();
        let identity = identity(&binding);
        let mut fake = FakeTaskLedger::new();
        let vacant = verified(&fake, &identity);
        fake.execute(
            task_created_command(
                vacant.head().clone(),
                "mcp-submit:req-1",
                &binding,
                &ingress_peer('a', 'c'),
            )
            .unwrap(),
        )
        .unwrap();

        let error = replay_lifecycle(
            &verified(&fake, &identity),
            &binding,
            &ingress_peer('a', 'e'),
        )
        .unwrap_err();
        assert_eq!(
            error.code(),
            "LATTICE_TASK_INGRESS_PROFILE_COMMITMENT_MISMATCH"
        );

        let adapter_error = replay_lifecycle(
            &verified(&fake, &identity),
            &binding,
            &ingress_peer('e', 'c'),
        )
        .unwrap_err();
        assert_eq!(
            adapter_error.code(),
            "LATTICE_TASK_INGRESS_PROFILE_COMMITMENT_MISMATCH"
        );

        let local_error =
            replay_lifecycle(&verified(&fake, &identity), &binding, &local_ingress_peer())
                .unwrap_err();
        assert_eq!(local_error.code(), "LATTICE_TASK_CREATED_EVIDENCE_REJECTED");
    }

    #[test]
    fn replay_validates_task_created_actor_action_reason_and_audit() {
        let peer = ingress_peer('a', 'c');
        let audit = task_created_audit_value(&peer).unwrap();

        for error in [
            replay_created_override(
                "local-canonical-mcp-acceptance-profile",
                TASK_CREATED_ACTION,
                TASK_CREATED_REASON,
                Some(audit.clone()),
            ),
            replay_created_override(
                peer.actor_id().as_str(),
                "UNEXPECTED_ACTION",
                TASK_CREATED_REASON,
                Some(audit.clone()),
            ),
            replay_created_override(
                peer.actor_id().as_str(),
                TASK_CREATED_ACTION,
                "UNEXPECTED_REASON",
                Some(audit),
            ),
        ] {
            assert_eq!(error.code(), "LATTICE_TASK_CREATED_EVIDENCE_REJECTED");
        }

        let audit_error = replay_created_override(
            peer.actor_id().as_str(),
            TASK_CREATED_ACTION,
            TASK_CREATED_REASON,
            None,
        );
        assert_eq!(audit_error.code(), "LATTICE_TASK_INGRESS_AUDIT_REJECTED");

        let mut inconsistent_audit = task_created_audit_value(&peer).unwrap();
        let CanonicalValue::Object(fields) = &mut inconsistent_audit else {
            unreachable!("audit is an object")
        };
        let observation = fields
            .iter_mut()
            .find(|(name, _)| name == "admission_observation_commitment")
            .expect("observation field");
        observation.1 = CanonicalValue::String("0".repeat(64));
        let inconsistent_error = replay_created_override(
            peer.actor_id().as_str(),
            TASK_CREATED_ACTION,
            TASK_CREATED_REASON,
            Some(inconsistent_audit),
        );
        assert_eq!(
            inconsistent_error.code(),
            "LATTICE_TASK_INGRESS_AUDIT_REJECTED"
        );
    }

    #[test]
    fn replay_derives_only_legal_task_domain_transitions_and_result() {
        let binding = binding();
        let identity = identity(&binding);
        let mut fake = FakeTaskLedger::new();
        let ingress_peer = ingress_peer('a', 'c');
        let stream = verified(&fake, &identity);
        fake.execute(
            task_created_command(
                stream.head().clone(),
                "mcp-submit:req-1",
                &binding,
                &ingress_peer,
            )
            .unwrap(),
        )
        .unwrap();
        for (from, to) in [
            (TaskState::Draft, TaskState::AwaitingExecutionApproval),
            (TaskState::AwaitingExecutionApproval, TaskState::Preparing),
            (TaskState::Preparing, TaskState::Executing),
            (TaskState::Executing, TaskState::Verifying),
            (TaskState::Verifying, TaskState::Reviewing),
            (TaskState::Reviewing, TaskState::AwaitingMergeApproval),
            (TaskState::AwaitingMergeApproval, TaskState::Merging),
        ] {
            append_transition(&mut fake, &identity, &binding, &ingress_peer, from, to);
        }

        let stream = verified(&fake, &identity);
        let result = ContentDigest::from_sha256("2".repeat(64)).unwrap();
        fake.execute(
            append_command(
                stream.head().clone(),
                RESULT_COMMAND_ID,
                "2000-01-01T00:00:20Z",
                LedgerEventKind::EvidenceRecorded,
                ingress_peer.actor_id().as_str(),
                RESULT_ACTION,
                RESULT_REASON,
                result.clone(),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        let evidence =
            replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        assert_eq!(evidence.state(), TaskState::Merging);
        assert_eq!(evidence.result_digest(), Some(&result));
    }

    #[test]
    fn reflection_tail_does_not_rewrite_completed_core_projection() {
        let binding = binding();
        let identity = identity(&binding);
        let mut fake = FakeTaskLedger::new();
        let ingress_peer = ingress_peer('a', 'c');
        let stream = verified(&fake, &identity);
        fake.execute(
            task_created_command(
                stream.head().clone(),
                "mcp-submit:req-1",
                &binding,
                &ingress_peer,
            )
            .unwrap(),
        )
        .unwrap();
        for (from, to) in [
            (TaskState::Draft, TaskState::AwaitingExecutionApproval),
            (TaskState::AwaitingExecutionApproval, TaskState::Preparing),
            (TaskState::Preparing, TaskState::Executing),
            (TaskState::Executing, TaskState::Verifying),
            (TaskState::Verifying, TaskState::Reviewing),
            (TaskState::Reviewing, TaskState::AwaitingMergeApproval),
            (TaskState::AwaitingMergeApproval, TaskState::Merging),
        ] {
            append_transition(&mut fake, &identity, &binding, &ingress_peer, from, to);
        }

        let result = ContentDigest::from_sha256("2".repeat(64)).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                RESULT_COMMAND_ID,
                "2000-01-01T00:00:20Z",
                LedgerEventKind::EvidenceRecorded,
                ingress_peer.actor_id().as_str(),
                RESULT_ACTION,
                RESULT_REASON,
                result.clone(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        append_transition(
            &mut fake,
            &identity,
            &binding,
            &ingress_peer,
            TaskState::Merging,
            TaskState::Completed,
        );

        let completed =
            replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        let completed_head = completed.core_head_digest().clone();
        let completed_journal_head = completed.ledger_head_digest().clone();
        assert_eq!(completed.state(), TaskState::Completed);
        assert_eq!(completed.result_digest(), Some(&result));

        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-hermes-failure",
                "2000-01-01T00:00:30Z",
                LedgerEventKind::EvidenceRecorded,
                "lattice-hermes-adapter",
                "REFLECTION_HERMES_FAILED",
                "GH9_HERMES_FAILURE_RECORDED",
                ContentDigest::from_sha256("3".repeat(64)).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        let replayed =
            replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        assert_eq!(replayed.state(), TaskState::Completed);
        assert_eq!(replayed.result_digest(), Some(&result));
        assert_ne!(replayed.ledger_head_digest(), &completed_journal_head);
        assert_eq!(
            replayed.core_head_digest(),
            &completed_head,
            "Reflection evidence must not advance the completed core projection head"
        );
    }

    #[test]
    fn external_reflection_command_cannot_reserve_a_future_pending_generation() {
        let (_binding, identity, _ingress_peer, fake, _result, _claim) = claimed_fake();
        let before = verified(&fake, &identity);
        let before_head = before.head().head_digest().clone();
        let before_event_count = before.events().len();

        let error = validate_external_reflection_command_id(&reflection_pending_command_id(1))
            .expect_err("the runtime-owned pending namespace is not caller authority");

        assert_eq!(
            error.code(),
            "LATTICE_REFLECTION_COMMAND_NAMESPACE_REJECTED"
        );
        let after = verified(&fake, &identity);
        assert_eq!(after.head().head_digest(), &before_head);
        assert_eq!(after.events().len(), before_event_count);
        validate_external_reflection_command_id("gh9-reflection-claim:1")
            .expect("a caller-owned command namespace remains available");
    }

    #[test]
    fn reflection_replay_rejects_forged_pending_command_authority() {
        let (binding, identity, ingress_peer, mut fake, result) = completed_fake();
        let core = replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-pending:00",
                "2000-01-01T00:00:30Z",
                LedgerEventKind::EffectIntent,
                REFLECTION_RUNTIME_ACTOR,
                REFLECTION_PENDING_ACTION,
                REFLECTION_PENDING_REASON,
                reflection_pending_digest(&binding, core.core_head_digest(), &result, 0).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        let error = replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer)
            .expect_err("pending command identity is derived, not caller-selected");
        assert_eq!(error.code(), "LATTICE_REFLECTION_PENDING_COMMAND_REJECTED");
    }

    #[test]
    fn reflection_replay_rejects_non_pending_use_of_reserved_command_namespace() {
        let (binding, identity, ingress_peer, mut fake, _result, _claim) = claimed_fake();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                &reflection_pending_command_id(1),
                "2000-01-01T00:00:32Z",
                LedgerEventKind::EvidenceRecorded,
                REFLECTION_BATCH_ACTOR,
                REFLECTION_CLAIMED_ACTION,
                REFLECTION_CLAIMED_REASON,
                ContentDigest::from_sha256("f".repeat(64)).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        let error = replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer)
            .expect_err("reserved pending IDs cannot be retained by another event");
        assert_eq!(
            error.code(),
            "LATTICE_REFLECTION_COMMAND_NAMESPACE_REJECTED"
        );
    }

    #[test]
    fn hermes_failure_replays_independently_from_completed_core() {
        let (binding, identity, ingress_peer, mut fake, result) = completed_fake();
        let completed =
            replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        let core_head = completed.core_head_digest().clone();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                &reflection_pending_command_id(0),
                "2000-01-01T00:00:30Z",
                LedgerEventKind::EffectIntent,
                REFLECTION_RUNTIME_ACTOR,
                REFLECTION_PENDING_ACTION,
                REFLECTION_PENDING_REASON,
                reflection_pending_digest(&binding, &core_head, &result, 0).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        let stream = verified(&fake, &identity);
        let admission = stream.outboxes().last().expect("pending admission").clone();
        let claim = reflection_claim_digest(
            &binding,
            &core_head,
            &admission,
            0,
            "gh9-reflection-claim:0",
        )
        .unwrap();
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-claim:0",
                "2000-01-01T00:00:31Z",
                LedgerEventKind::EvidenceRecorded,
                REFLECTION_BATCH_ACTOR,
                REFLECTION_CLAIMED_ACTION,
                REFLECTION_CLAIMED_REASON,
                claim.clone(),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        let failure_evidence = ContentDigest::from_sha256("4".repeat(64)).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-failure:0",
                "2000-01-01T00:00:32Z",
                LedgerEventKind::EffectOutcome,
                REFLECTION_HERMES_ACTOR,
                REFLECTION_HERMES_FAILURE_ACTION,
                REFLECTION_HERMES_FAILURE_REASON,
                reflection_failure_digest(
                    &binding,
                    &core_head,
                    Some(&claim),
                    0,
                    ReflectionFailureKind::HermesFailure,
                    &failure_evidence,
                )
                .unwrap(),
                Some(reflection_evidence_diagnostic(&failure_evidence)),
            )
            .unwrap(),
        )
        .unwrap();

        let stream = verified(&fake, &identity);
        let reflection = replay_reflection(&stream, &binding, &ingress_peer)
            .expect("Hermes failure must replay from immutable events");
        assert_eq!(reflection.evidence.state(), ReflectionState::Failed);
        assert_eq!(reflection.evidence.core_head_digest(), &core_head);
        let core = replay_lifecycle(&stream, &binding, &ingress_peer).unwrap();
        assert_eq!(core.state(), TaskState::Completed);
        assert_eq!(core.result_digest(), Some(&result));
        assert_eq!(core.core_head_digest(), &core_head);
    }

    #[test]
    fn hermes_candidate_cannot_cross_a_failed_generation_boundary() {
        let (binding, identity, ingress_peer, mut fake, result) = completed_fake();
        let core = replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        let core_head = core.core_head_digest().clone();
        let failure_evidence = ContentDigest::from_sha256("4".repeat(64)).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                &reflection_pending_command_id(0),
                "2000-01-01T00:00:30Z",
                LedgerEventKind::EffectIntent,
                REFLECTION_RUNTIME_ACTOR,
                REFLECTION_PENDING_ACTION,
                REFLECTION_PENDING_REASON,
                reflection_pending_digest(&binding, &core_head, &result, 0).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let stream = verified(&fake, &identity);
        let admission = stream.outboxes().last().expect("pending admission");
        let claim =
            reflection_claim_digest(&binding, &core_head, admission, 0, "gh9-reflection-claim:0")
                .unwrap();
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-claim:0",
                "2000-01-01T00:00:31Z",
                LedgerEventKind::EvidenceRecorded,
                REFLECTION_BATCH_ACTOR,
                REFLECTION_CLAIMED_ACTION,
                REFLECTION_CLAIMED_REASON,
                claim.clone(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let failed_reflection =
            replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        let failed_history_query =
            TaskReflectionHistoryQuery::latest(lattice_ports::MAX_TASK_REFLECTION_HISTORY_EVENTS)
                .unwrap();
        let failed_history =
            project_authorized_history(&binding, &failed_reflection, failed_history_query).unwrap();
        let failed_history_digest = failed_history.history_digest().clone();
        let candidate_digest = ContentDigest::from_sha256("5".repeat(64)).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-failure:0",
                "2000-01-01T00:00:32Z",
                LedgerEventKind::EffectOutcome,
                REFLECTION_HERMES_ACTOR,
                REFLECTION_HERMES_FAILURE_ACTION,
                REFLECTION_HERMES_FAILURE_REASON,
                reflection_failure_digest(
                    &binding,
                    &core_head,
                    Some(&claim),
                    0,
                    ReflectionFailureKind::HermesFailure,
                    &failure_evidence,
                )
                .unwrap(),
                Some(reflection_evidence_diagnostic(&failure_evidence)),
            )
            .unwrap(),
        )
        .unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-candidate-after-failure",
                "2000-01-01T00:00:35Z",
                LedgerEventKind::EvidenceRecorded,
                REFLECTION_HERMES_ACTOR,
                REFLECTION_INFERENCE_ACTION,
                REFLECTION_CANDIDATE_REASON,
                reflection_candidate_digest(
                    &binding,
                    &core_head,
                    &claim,
                    0,
                    ReflectionCandidateKind::Inference,
                    failed_history_query,
                    &failed_history_digest,
                    &candidate_digest,
                )
                .unwrap(),
                Some(reflection_candidate_diagnostic(
                    &candidate_digest,
                    failed_history_query,
                    &failed_history_digest,
                )),
            )
            .unwrap(),
        )
        .unwrap();

        let error = replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer)
            .expect_err("a failed generation is closed to later candidates");
        assert_eq!(error.code(), "LATTICE_REFLECTION_CANDIDATE_STATE_REJECTED");
    }

    #[test]
    fn authorized_history_is_bounded_and_digest_only() {
        let (binding, identity, ingress_peer, fake, _result, _claim) = claimed_fake();
        let reflection = replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer)
            .expect("claimed Reflection history");
        let history = project_authorized_history(
            &binding,
            &reflection,
            TaskReflectionHistoryQuery::latest(2).unwrap(),
        )
        .expect("bounded authorized history");
        assert_eq!(history.binding(), &binding);
        assert_eq!(history.events().len(), 2);
        assert_eq!(history.events()[0].kind(), TaskReflectionEventKind::Pending);
        assert_eq!(history.events()[1].kind(), TaskReflectionEventKind::Claimed);
        assert_ne!(
            history.history_digest(),
            history.journal_head_digest(),
            "the window commitment is distinct from the full journal head"
        );
    }

    #[test]
    fn authorized_history_pages_are_complete_and_candidate_bound() {
        let (binding, identity, ingress_peer, mut fake, _result, claim) = claimed_fake();
        let reflection = replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer)
            .expect("claimed Reflection history");
        let mut long_reflection = reflection.clone();
        let first_synthetic_sequence = long_reflection
            .events
            .last()
            .expect("claimed event")
            .sequence()
            + 1;
        for index in 0..70_u64 {
            let digest = ContentDigest::from_sha256(format!("{:064x}", index + 10)).unwrap();
            long_reflection.events.push(TaskReflectionHistoryEvent::new(
                first_synthetic_sequence + index,
                0,
                TaskReflectionEventKind::Candidate(ReflectionCandidateKind::Observation),
                TaskReflectionEventReference::Evidence(digest.clone()),
                digest.clone(),
                digest,
            ));
        }
        assert_eq!(long_reflection.events.len(), 72);
        let mut query = TaskReflectionHistoryQuery::latest(7).unwrap();
        let mut sequences = Vec::new();
        loop {
            let page = project_authorized_history(&binding, &long_reflection, query)
                .expect("authorized history page");
            assert_eq!(page.query(), query);
            assert!(
                page.events()
                    .windows(2)
                    .all(|pair| { pair[0].sequence() < pair[1].sequence() })
            );
            sequences.extend(
                page.events()
                    .iter()
                    .map(TaskReflectionHistoryEvent::sequence),
            );
            let Some(cursor) = page.next_before_sequence() else {
                break;
            };
            query = TaskReflectionHistoryQuery::new(Some(cursor), 7).unwrap();
        }
        sequences.sort_unstable();
        sequences.dedup();
        assert_eq!(sequences.len(), long_reflection.events.len());
        assert_eq!(
            sequences.first().copied(),
            long_reflection
                .events
                .first()
                .map(TaskReflectionHistoryEvent::sequence)
        );
        assert_eq!(
            sequences.last().copied(),
            long_reflection
                .events
                .last()
                .map(TaskReflectionHistoryEvent::sequence)
        );

        let latest_one_query = TaskReflectionHistoryQuery::latest(1).unwrap();
        let latest_one = project_authorized_history(&binding, &reflection, latest_one_query)
            .expect("one-event authorized history");
        validate_candidate_history(
            &binding,
            &reflection,
            latest_one_query,
            latest_one.history_digest(),
        )
        .expect("the exact caller-selected page authorizes a candidate");
        let stale_digest = latest_one.history_digest().clone();
        append_observation_candidate(&mut fake, &identity, &binding, &ingress_peer, &claim, 80, 1);
        let advanced = replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer)
            .expect("advanced Reflection history");
        assert_eq!(
            validate_candidate_history(&binding, &advanced, latest_one_query, &stale_digest,)
                .expect_err("stale page must not authorize a new candidate")
                .code(),
            "LATTICE_REFLECTION_CANDIDATE_HISTORY_REJECTED"
        );
    }

    #[test]
    fn authorized_history_rejects_invalid_cursors_and_limits() {
        assert_eq!(
            TaskReflectionHistoryQuery::latest(0)
                .expect_err("zero limit")
                .code(),
            "LATTICE_REFLECTION_HISTORY_LIMIT_REJECTED"
        );
        assert_eq!(
            TaskReflectionHistoryQuery::latest(
                lattice_ports::MAX_TASK_REFLECTION_HISTORY_EVENTS + 1,
            )
            .expect_err("oversized limit")
            .code(),
            "LATTICE_REFLECTION_HISTORY_LIMIT_REJECTED"
        );
        assert_eq!(
            TaskReflectionHistoryQuery::new(Some(0), 1)
                .expect_err("zero cursor")
                .code(),
            "LATTICE_REFLECTION_HISTORY_CURSOR_REJECTED"
        );
        let (binding, identity, ingress_peer, fake, _result, _claim) = claimed_fake();
        let reflection = replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer)
            .expect("claimed Reflection");
        let error = project_authorized_history(
            &binding,
            &reflection,
            TaskReflectionHistoryQuery::new(Some(u64::MAX), 1).unwrap(),
        )
        .expect_err("unknown cursor must fail closed");
        assert_eq!(error.code(), "LATTICE_REFLECTION_HISTORY_CURSOR_REJECTED");
    }

    #[test]
    fn terminal_core_failure_records_failure_and_output_rejection_evidence() {
        for failure_kind in [
            ReflectionFailureKind::TaskFailure,
            ReflectionFailureKind::OutputRejected,
        ] {
            let terminal = TaskState::Failed;
            let (binding, identity, ingress_peer, mut fake) = terminal_fake(terminal);
            let core = replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer)
                .expect("terminal core projection");
            let core_head = core.core_head_digest().clone();
            let evidence_digest = ContentDigest::from_sha256("6".repeat(64)).unwrap();
            let (action, actor, reason) = reflection_failure_write_profile(failure_kind);
            let stream = verified(&fake, &identity);
            fake.execute(
                append_command(
                    stream.head().clone(),
                    &format!("gh9-direct-failure:{}", failure_kind.as_str()),
                    "2000-01-01T00:00:30Z",
                    LedgerEventKind::EffectOutcome,
                    actor,
                    action,
                    reason,
                    reflection_failure_digest(
                        &binding,
                        &core_head,
                        None,
                        0,
                        failure_kind,
                        &evidence_digest,
                    )
                    .unwrap(),
                    Some(reflection_evidence_diagnostic(&evidence_digest)),
                )
                .unwrap(),
            )
            .unwrap();

            let stream = verified(&fake, &identity);
            let reflection = replay_reflection(&stream, &binding, &ingress_peer)
                .expect("direct terminal evidence must replay");
            assert_eq!(reflection.evidence.state(), ReflectionState::Failed);
            assert_eq!(reflection.evidence.core_head_digest(), &core_head);
            assert_eq!(reflection.evidence.pending_admission_digest(), None);
            assert_eq!(reflection.evidence.claim_digest(), None);
            assert_eq!(
                reflection
                    .events
                    .last()
                    .map(TaskReflectionHistoryEvent::kind),
                Some(TaskReflectionEventKind::Failure(failure_kind))
            );
            let history = project_authorized_history(
                &binding,
                &reflection,
                TaskReflectionHistoryQuery::latest(1).unwrap(),
            )
            .expect("terminal evidence remains queryable without a claim");
            assert_eq!(history.events().len(), 1);
            assert_eq!(
                history.events()[0].kind(),
                TaskReflectionEventKind::Failure(failure_kind)
            );

            let replayed_core = replay_lifecycle(&stream, &binding, &ingress_peer)
                .expect("Reflection failure cannot rewrite core");
            assert_eq!(replayed_core.state(), terminal);
            assert_eq!(replayed_core.core_head_digest(), &core_head);
            assert_ne!(replayed_core.ledger_head_digest(), &core_head);
        }
    }

    #[test]
    fn failure_context_is_closed_before_any_append() {
        let claim = ContentDigest::from_sha256("a".repeat(64)).unwrap();
        assert_eq!(
            reflection_failure_next_state(
                TaskState::Completed,
                Some(ReflectionState::Pending),
                None,
                ReflectionFailureKind::HermesFailure,
            ),
            Err("LATTICE_REFLECTION_FAILURE_CLAIM_MISSING")
        );
        assert_eq!(
            reflection_failure_next_state(
                TaskState::Completed,
                Some(ReflectionState::Pending),
                Some(&claim),
                ReflectionFailureKind::TaskFailure,
            ),
            Err("LATTICE_REFLECTION_FAILURE_ORDER_REJECTED")
        );
        assert_eq!(
            reflection_failure_next_state(
                TaskState::Failed,
                None,
                None,
                ReflectionFailureKind::TaskFailure,
            ),
            Ok(ReflectionState::Failed)
        );
        assert_eq!(
            reflection_failure_next_state(
                TaskState::Failed,
                None,
                None,
                ReflectionFailureKind::OutputRejected,
            ),
            Ok(ReflectionState::Failed)
        );
        assert_eq!(
            reflection_failure_next_state(
                TaskState::Completed,
                Some(ReflectionState::Pending),
                Some(&claim),
                ReflectionFailureKind::HermesFailure,
            ),
            Ok(ReflectionState::Failed)
        );
    }

    #[test]
    fn retry_generation_preserves_core_and_original_event_commitments() {
        let (binding, identity, ingress_peer, mut fake, result, first_claim) = claimed_fake();
        let core = replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        let core_head = core.core_head_digest().clone();
        let immutable_prefix = verified(&fake, &identity)
            .events()
            .iter()
            .map(|event| event.event_digest().clone())
            .collect::<Vec<_>>();

        let candidate_digest = ContentDigest::from_sha256("7".repeat(64)).unwrap();
        let before_candidate =
            replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer).unwrap();
        let candidate_history_query =
            TaskReflectionHistoryQuery::latest(lattice_ports::MAX_TASK_REFLECTION_HISTORY_EVENTS)
                .unwrap();
        let candidate_history =
            project_authorized_history(&binding, &before_candidate, candidate_history_query)
                .unwrap();
        let candidate_history_digest = candidate_history.history_digest().clone();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-candidate:0",
                "2000-01-01T00:00:32Z",
                LedgerEventKind::EvidenceRecorded,
                REFLECTION_HERMES_ACTOR,
                REFLECTION_OBSERVATION_ACTION,
                REFLECTION_CANDIDATE_REASON,
                reflection_candidate_digest(
                    &binding,
                    &core_head,
                    &first_claim,
                    0,
                    ReflectionCandidateKind::Observation,
                    candidate_history_query,
                    &candidate_history_digest,
                    &candidate_digest,
                )
                .unwrap(),
                Some(reflection_candidate_diagnostic(
                    &candidate_digest,
                    candidate_history_query,
                    &candidate_history_digest,
                )),
            )
            .unwrap(),
        )
        .unwrap();
        let failure_evidence = ContentDigest::from_sha256("8".repeat(64)).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-failure:0",
                "2000-01-01T00:00:33Z",
                LedgerEventKind::EffectOutcome,
                REFLECTION_HERMES_ACTOR,
                REFLECTION_HERMES_FAILURE_ACTION,
                REFLECTION_HERMES_FAILURE_REASON,
                reflection_failure_digest(
                    &binding,
                    &core_head,
                    Some(&first_claim),
                    0,
                    ReflectionFailureKind::HermesFailure,
                    &failure_evidence,
                )
                .unwrap(),
                Some(reflection_evidence_diagnostic(&failure_evidence)),
            )
            .unwrap(),
        )
        .unwrap();
        let degraded_evidence = ContentDigest::from_sha256("9".repeat(64)).unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-retry:0",
                "2000-01-01T00:00:34Z",
                LedgerEventKind::EffectOutcome,
                REFLECTION_RUNTIME_ACTOR,
                REFLECTION_RETRY_ACTION,
                REFLECTION_RETRY_REASON,
                reflection_transition_digest(
                    &binding,
                    &core_head,
                    Some(&first_claim),
                    0,
                    ReflectionState::RetryPending,
                )
                .unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let retry = replay_reflection(&verified(&fake, &identity), &binding, &ingress_peer)
            .expect("retry authorization replays");
        assert_eq!(retry.evidence.state(), ReflectionState::RetryPending);
        assert_eq!(retry.evidence.claim_digest(), Some(&first_claim));

        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                &reflection_pending_command_id(1),
                "2000-01-01T00:00:35Z",
                LedgerEventKind::EffectIntent,
                REFLECTION_RUNTIME_ACTOR,
                REFLECTION_PENDING_ACTION,
                REFLECTION_PENDING_REASON,
                reflection_pending_digest(&binding, &core_head, &result, 1).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let stream = verified(&fake, &identity);
        let second_admission = stream.outboxes().last().expect("second pending admission");
        let second_claim = reflection_claim_digest(
            &binding,
            &core_head,
            second_admission,
            1,
            "gh9-reflection-claim:1",
        )
        .unwrap();
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-claim:1",
                "2000-01-01T00:00:36Z",
                LedgerEventKind::EvidenceRecorded,
                REFLECTION_BATCH_ACTOR,
                REFLECTION_CLAIMED_ACTION,
                REFLECTION_CLAIMED_REASON,
                second_claim.clone(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "gh9-reflection-degraded:1",
                "2000-01-01T00:00:37Z",
                LedgerEventKind::EffectOutcome,
                REFLECTION_RUNTIME_ACTOR,
                REFLECTION_DEGRADED_ACTION,
                REFLECTION_DEGRADED_REASON,
                reflection_degraded_digest(
                    &binding,
                    &core_head,
                    Some(&second_claim),
                    1,
                    &degraded_evidence,
                )
                .unwrap(),
                Some(reflection_evidence_diagnostic(&degraded_evidence)),
            )
            .unwrap(),
        )
        .unwrap();

        let stream = verified(&fake, &identity);
        let reflection =
            replay_reflection(&stream, &binding, &ingress_peer).expect("second generation replays");
        assert_eq!(reflection.evidence.state(), ReflectionState::Degraded);
        assert_eq!(reflection.evidence.generation(), 1);
        assert_eq!(reflection.evidence.core_head_digest(), &core_head);
        assert_eq!(
            stream
                .events()
                .iter()
                .take(immutable_prefix.len())
                .map(|event| event.event_digest().clone())
                .collect::<Vec<_>>(),
            immutable_prefix,
            "Reflection appends cannot overwrite or delete original events"
        );
        let replayed_core = replay_lifecycle(&stream, &binding, &ingress_peer).unwrap();
        assert_eq!(replayed_core.state(), TaskState::Completed);
        assert_eq!(replayed_core.core_head_digest(), &core_head);

        validate_existing_reflection_action(
            reflection_command_event(&stream, "gh9-reflection-claim:0").unwrap(),
            REFLECTION_CLAIMED_ACTION,
        )
        .expect("old claim remains an exact retry after later generations");
        validate_existing_reflection_candidate(
            reflection_command_event(&stream, "gh9-reflection-candidate:0").unwrap(),
            REFLECTION_OBSERVATION_ACTION,
            candidate_history_query,
            &candidate_history_digest,
            &candidate_digest,
        )
        .expect("old candidate remains an exact retry after later generations");
        validate_existing_reflection_evidence(
            reflection_command_event(&stream, "gh9-reflection-failure:0").unwrap(),
            REFLECTION_HERMES_FAILURE_ACTION,
            &failure_evidence,
        )
        .expect("old failure remains an exact retry after later generations");
        validate_existing_reflection_action(
            reflection_command_event(&stream, "gh9-reflection-retry:0").unwrap(),
            REFLECTION_RETRY_ACTION,
        )
        .expect("old retry command remains exact after the next generation");
        validate_existing_reflection_evidence(
            reflection_command_event(&stream, "gh9-reflection-degraded:1").unwrap(),
            REFLECTION_DEGRADED_ACTION,
            &degraded_evidence,
        )
        .expect("degraded evidence exact retry");

        let substituted = ContentDigest::from_sha256("f".repeat(64)).unwrap();
        assert_eq!(
            validate_existing_reflection_candidate(
                reflection_command_event(&stream, "gh9-reflection-candidate:0").unwrap(),
                REFLECTION_OBSERVATION_ACTION,
                candidate_history_query,
                &candidate_history_digest,
                &substituted,
            )
            .expect_err("same command with another candidate is substitution")
            .code(),
            "LATTICE_REFLECTION_COMMAND_SUBSTITUTED"
        );
    }

    #[test]
    fn replay_rejects_result_before_merging() {
        let binding = binding();
        let identity = identity(&binding);
        let ingress_peer = ingress_peer('a', 'c');
        let mut fake = FakeTaskLedger::new();
        let stream = verified(&fake, &identity);
        fake.execute(
            task_created_command(
                stream.head().clone(),
                "mcp-submit:req-1",
                &binding,
                &ingress_peer,
            )
            .unwrap(),
        )
        .unwrap();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                RESULT_COMMAND_ID,
                "2000-01-01T00:00:20Z",
                LedgerEventKind::EvidenceRecorded,
                ingress_peer.actor_id().as_str(),
                RESULT_ACTION,
                RESULT_REASON,
                ContentDigest::from_sha256("2".repeat(64)).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        let error =
            replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap_err();
        assert_eq!(error.code(), "LATTICE_TASK_RESULT_EVIDENCE_REJECTED");
    }

    #[test]
    fn replay_rejects_delivery_work_without_durable_task_admission() {
        let binding = binding();
        let identity = identity(&binding);
        let ingress_peer = ingress_peer('a', 'c');
        let mut fake = FakeTaskLedger::new();
        let stream = verified(&fake, &identity);
        fake.execute(
            append_command(
                stream.head().clone(),
                "task032-delivery-intent",
                "2000-01-01T00:00:00Z",
                LedgerEventKind::EffectIntent,
                ingress_peer.actor_id().as_str(),
                "TASK032_CODEX_DELIVERY",
                "TASK032_CODEX_INTENT",
                ContentDigest::from_sha256("3".repeat(64)).unwrap(),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        let error =
            replay_lifecycle(&verified(&fake, &identity), &binding, &ingress_peer).unwrap_err();
        assert_eq!(error.code(), "LATTICE_TASK_ADMISSION_MISSING");
    }
}
