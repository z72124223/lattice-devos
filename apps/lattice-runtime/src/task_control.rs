//! PostgreSQL-backed Task Domain lifecycle projection for bounded MCP work.

use std::time::Instant;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, StoreAuthorityHead, SubjectBinding, TaskIngressPeerEvidence,
    TaskLedgerStreamIdentity, WriterLeaseAuthorityHead,
};
use lattice_ports::{
    TaskLifecycleError, TaskLifecycleErrorKind, TaskLifecycleEvidence, TaskLifecyclePort,
    TaskLifecycleResult,
};
use lattice_postgres_store::{MigrationTarget, PostgresTaskLedger};
use lattice_task_domain::{TaskState, transition};
use lattice_task_ledger::{
    ActionId, ActorId, AppendCommand, CommandId, CommandOutcome, CorrelationId, Diagnostic,
    LedgerEventKind, LedgerOutcome, ReasonCode, VerifiedStream,
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

#[must_use]
pub(crate) fn task_admission_command_id(client_request_id: &str) -> String {
    format!("mcp-submit:{client_request_id}")
}

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

    /// Returns the exact durable TaskCreated command after replay validation.
    ///
    /// # Errors
    ///
    /// Fails closed for a missing, corrupt, or cross-bound admission stream.
    pub(crate) fn verified_admission_command_id(
        &mut self,
        binding: &SubjectBinding,
    ) -> TaskLifecycleResult<String> {
        let ingress_peer = self.required_ingress_peer()?;
        let stream = self.load_verified(binding)?;
        let evidence = replay_lifecycle(&stream, binding, &ingress_peer)?;
        if !evidence.admitted() {
            return Err(rejected("LATTICE_TASK_ADMISSION_MISSING"));
        }
        stream
            .events()
            .iter()
            .find(|event| event.kind() == LedgerEventKind::TaskCreated)
            .map(|event| event.command_id().as_str().to_owned())
            .ok_or_else(|| corrupt("LATTICE_TASK_ADMISSION_MISSING"))
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
        let command_id = task_admission_command_id(client_request_id);
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
    ensure_binding(binding, stream.identity())?;
    let expected_actor = ingress_peer.actor_id().as_str();
    let expected_created_subject = task_created_subject_digest(binding, ingress_peer)?;
    let mut state = TaskState::Draft;
    let mut created = false;
    let mut result_digest = None;
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
                if event.subject_digest() != &expected_created_subject {
                    return Err(corrupt("LATTICE_TASK_INGRESS_PROFILE_COMMITMENT_MISMATCH"));
                }
                let audit = event
                    .diagnostic()
                    .map(Diagnostic::value)
                    .ok_or_else(|| corrupt("LATTICE_TASK_INGRESS_AUDIT_REJECTED"))?;
                validate_task_created_audit(audit, ingress_peer)?;
                created = true;
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
            }
            _ => {}
        }
    }
    if !created && !stream.events().is_empty() {
        // A TaskSpec-bound stream may contain no delivery work before admission;
        // any other event without TASK_CREATED is not a valid task authority.
        return Err(corrupt("LATTICE_TASK_ADMISSION_MISSING"));
    }
    Ok(TaskLifecycleEvidence::new(
        binding.clone(),
        created,
        state,
        stream.head().head_digest().clone(),
        result_digest,
    ))
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
