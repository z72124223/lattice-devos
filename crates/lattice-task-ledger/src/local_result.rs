//! A verified local result closes the original intake without deployment claims.
use super::{
    ActionId, ActorId, AppendCommand, AppendConstruction, CommandId, ContentDigest, CorrelationId,
    GENERAL_TASK_INTAKE_CORRELATION_ID, LedgerError, LedgerEventKind, LedgerOutcome, ReasonCode,
    TaskLedgerStreamHead, TaskLedgerSubjectKind, digest_value, hash_value_at_version,
    is_zero_digest, object, text, valid_evidence_reference, valid_task_ingress_client_request_id,
};

pub const LOCAL_RESULT_ADOPTION_ACTION: &str = "LOCAL_VERIFIED_RESULT_ADOPTED";
pub const LOCAL_RESULT_ADOPTION_REASON: &str = "LOCAL_VERIFIED_RESULT_ADOPTED";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalVerifiedResultAdoption {
    task_ref: ContentDigest,
    client_request_id: String,
    expected_ledger_head_digest: ContentDigest,
    artifact_ref: String,
    acceptance_ref: String,
    result_digest: ContentDigest,
}

impl LocalVerifiedResultAdoption {
    /// Binds the original intake identity to immutable artifact and acceptance receipts.
    ///
    /// # Errors
    /// Rejects invalid intake identities, receipt references, or canonical digest encoding.
    pub fn new(
        task_ref: ContentDigest,
        client_request_id: impl Into<String>,
        expected_ledger_head_digest: ContentDigest,
        artifact_ref: impl Into<String>,
        acceptance_ref: impl Into<String>,
    ) -> Result<Self, LedgerError> {
        let client_request_id = client_request_id.into();
        let artifact_ref = artifact_ref.into();
        let acceptance_ref = acceptance_ref.into();
        if is_zero_digest(&task_ref)
            || is_zero_digest(&expected_ledger_head_digest)
            || !valid_task_ingress_client_request_id(&client_request_id)
            || !valid_evidence_reference(&artifact_ref)
            || !valid_evidence_reference(&acceptance_ref)
            || artifact_ref == acceptance_ref
        {
            return Err(LedgerError::LocalVerifiedResultAdoptionMismatch);
        }
        let result_digest = hash_value_at_version(
            "lattice.task-ledger.local-verified-result-adoption",
            "1.0",
            &object(vec![
                (
                    "schema",
                    text("lattice.task-ledger.local-verified-result-adoption/1.0"),
                ),
                ("task_ref", digest_value(&task_ref)),
                ("client_request_id", text(&client_request_id)),
                (
                    "expected_ledger_head_digest",
                    digest_value(&expected_ledger_head_digest),
                ),
                ("artifact_ref", text(&artifact_ref)),
                ("acceptance_ref", text(&acceptance_ref)),
            ]),
        )?;
        Ok(Self {
            task_ref,
            client_request_id,
            expected_ledger_head_digest,
            artifact_ref,
            acceptance_ref,
            result_digest,
        })
    }
    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }
    #[must_use]
    pub fn client_request_id(&self) -> &str {
        &self.client_request_id
    }
    #[must_use]
    pub const fn expected_ledger_head_digest(&self) -> &ContentDigest {
        &self.expected_ledger_head_digest
    }
    #[must_use]
    pub fn artifact_ref(&self) -> &str {
        &self.artifact_ref
    }
    #[must_use]
    pub fn acceptance_ref(&self) -> &str {
        &self.acceptance_ref
    }
    #[must_use]
    pub fn command_id(&self) -> String {
        format!("local-result-adoption:{}", self.client_request_id)
    }
    #[must_use]
    pub const fn result_digest(&self) -> &ContentDigest {
        &self.result_digest
    }
}

impl AppendCommand {
    /// The repository must verify the retained local descriptor before executing this command.
    ///
    /// # Errors
    /// Rejects a mismatched intake head, invalid timestamp, or invalid command fields.
    pub fn new_local_verified_result_adopted(
        expected_head: TaskLedgerStreamHead,
        occurred_at: impl Into<String>,
        actor_id: ActorId,
        adoption: &LocalVerifiedResultAdoption,
    ) -> Result<Self, LedgerError> {
        if expected_head.identity().subject_kind() != TaskLedgerSubjectKind::GeneralTaskIntake
            || expected_head.sequence() != 1
            || expected_head.head_digest() != adoption.expected_ledger_head_digest()
        {
            return Err(LedgerError::LocalVerifiedResultAdoptionMismatch);
        }
        Self::from_fields(
            expected_head,
            CommandId::new(adoption.command_id())?,
            CorrelationId::new(GENERAL_TASK_INTAKE_CORRELATION_ID)?,
            occurred_at,
            LedgerEventKind::EvidenceRecorded,
            actor_id,
            ActionId::new(LOCAL_RESULT_ADOPTION_ACTION)?,
            LedgerOutcome::Recorded,
            ReasonCode::new(LOCAL_RESULT_ADOPTION_REASON)?,
            adoption.result_digest().clone(),
            None,
            None,
            AppendConstruction::VerifiedLocalResultAdoption,
        )
    }
}

#[must_use]
pub fn is_local_verified_result(kind: LedgerEventKind, action: &str) -> bool {
    kind == LedgerEventKind::EvidenceRecorded && action == LOCAL_RESULT_ADOPTION_ACTION
}
