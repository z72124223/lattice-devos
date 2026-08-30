//! Exact, task-bound local execution authority for managed foreman attempts.

use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ApprovalAuthority, ApprovalAuthorityHead, ApprovalAuthorityReceipt, ApprovalLane,
    ApprovalStatus, ApprovalSubject, ContentDigest, ProjectAuthorityHead, ProjectAuthorityReceipt,
    SubjectBinding,
};
use lattice_policy::{DecisionKind, DecisionStage, ExecutionGateDecisionEvidence, PolicyDecision};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

use crate::{ApprovalChallenge, FakeApprovalProof, FakeNormalSigner, proof_matches};

/// Persistence schema for one execution-authority envelope.
pub const EXECUTION_AUTHORITY_SCHEMA: &str = "lattice.approval.execution-authority/1.0";

/// Closed source of authority. An objective is deliberately not a source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionAuthoritySource {
    /// A trusted closed policy established that this bounded local action does
    /// not require a human approval.
    ClosedPolicyNoApprovalRequired,
    /// An exact approval receipt was independently verified.
    VerifiedApproval,
}

impl ExecutionAuthoritySource {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClosedPolicyNoApprovalRequired => "CLOSED_POLICY_NO_APPROVAL_REQUIRED",
            Self::VerifiedApproval => "VERIFIED_APPROVAL",
        }
    }

    /// Parses a persisted closed value.
    ///
    /// # Errors
    /// Unknown values fail closed.
    pub fn parse(value: &str) -> Result<Self, ExecutionAuthorityError> {
        match value {
            "CLOSED_POLICY_NO_APPROVAL_REQUIRED" => Ok(Self::ClosedPolicyNoApprovalRequired),
            "VERIFIED_APPROVAL" => Ok(Self::VerifiedApproval),
            _ => Err(ExecutionAuthorityError::MalformedField),
        }
    }
}

/// Closed capability granted by a Phase-4 execution authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionCapability {
    /// Bounded, reversible work in the captured local project/worktree only.
    LocalReversibleTaskExecution,
}

impl ExecutionCapability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalReversibleTaskExecution => "LOCAL_REVERSIBLE_TASK_EXECUTION",
        }
    }

    /// Phase 4 never grants an external or irreversible effect.
    #[must_use]
    pub const fn allows_external_effect(self, _effect: &str) -> bool {
        false
    }

    /// Parses a persisted closed value.
    ///
    /// # Errors
    /// Unknown values fail closed.
    pub fn parse(value: &str) -> Result<Self, ExecutionAuthorityError> {
        match value {
            "LOCAL_REVERSIBLE_TASK_EXECUTION" => Ok(Self::LocalReversibleTaskExecution),
            _ => Err(ExecutionAuthorityError::MalformedField),
        }
    }
}

/// Fail-closed execution-authority errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionAuthorityError {
    MalformedField,
    InvalidValidityWindow,
    ApprovalReceiptRequired,
    UnexpectedApprovalReceipt,
    DigestMismatch,
    /// Closed-policy authority can only be minted through the formal Policy
    /// Engine gate, never from caller-selected evidence bytes.
    PolicyEvaluationRequired,
    /// The formal execution gate denied the exact current context.
    PolicyDenied,
    /// Persisted evidence does not match a fresh evaluation of the exact
    /// task/spec/budget/project/runtime context.
    PolicyEvidenceMismatch,
    /// An otherwise valid authority was observed before its issue instant.
    NotYetValid,
    /// An otherwise valid authority reached its exclusive expiry instant.
    Expired,
    /// An authority or policy context was substituted across bindings.
    BindingMismatch,
    Canonicalization,
}

impl fmt::Display for ExecutionAuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "EXECUTION_AUTHORITY_{self:?}")
    }
}

impl Error for ExecutionAuthorityError {}

/// Fully constructed, untrusted authority input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionAuthorityInput {
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    task_spec_digest: ContentDigest,
    approval_subject_digest: ContentDigest,
    budget_digest: ContentDigest,
    source: ExecutionAuthoritySource,
    capability: ExecutionCapability,
    authority_evidence_digest: ContentDigest,
    approval_receipt_digest: Option<ContentDigest>,
    issued_at: String,
    expires_at: String,
}

impl ExecutionAuthorityInput {
    /// Constructs one exact authority binding without performing I/O.
    ///
    /// # Errors
    /// Rejects zero digests, non-canonical validity timestamps, an invalid
    /// validity window, or a receipt/source mismatch.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_ref: ContentDigest,
        successor_stream_id: ContentDigest,
        task_spec_digest: ContentDigest,
        approval_subject_digest: ContentDigest,
        budget_digest: ContentDigest,
        source: ExecutionAuthoritySource,
        capability: ExecutionCapability,
        authority_evidence_digest: ContentDigest,
        approval_receipt_digest: Option<ContentDigest>,
        issued_at: impl Into<String>,
        expires_at: impl Into<String>,
    ) -> Result<Self, ExecutionAuthorityError> {
        let value = Self {
            task_ref,
            successor_stream_id,
            task_spec_digest,
            approval_subject_digest,
            budget_digest,
            source,
            capability,
            authority_evidence_digest,
            approval_receipt_digest,
            issued_at: issued_at.into(),
            expires_at: expires_at.into(),
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), ExecutionAuthorityError> {
        if [
            &self.task_ref,
            &self.successor_stream_id,
            &self.task_spec_digest,
            &self.approval_subject_digest,
            &self.budget_digest,
            &self.authority_evidence_digest,
        ]
        .into_iter()
        .any(is_zero_digest)
            || self
                .approval_receipt_digest
                .as_ref()
                .is_some_and(is_zero_digest)
        {
            return Err(ExecutionAuthorityError::MalformedField);
        }

        let issued = canonical_utc(&self.issued_at)?;
        let expires = canonical_utc(&self.expires_at)?;
        if issued >= expires {
            return Err(ExecutionAuthorityError::InvalidValidityWindow);
        }

        match (self.source, self.approval_receipt_digest.is_some()) {
            (ExecutionAuthoritySource::VerifiedApproval, false) => {
                Err(ExecutionAuthorityError::ApprovalReceiptRequired)
            }
            (ExecutionAuthoritySource::ClosedPolicyNoApprovalRequired, true) => {
                Err(ExecutionAuthorityError::UnexpectedApprovalReceipt)
            }
            _ => Ok(()),
        }
    }
}

/// Verified immutable authority with a domain-separated record digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedExecutionAuthority {
    input: ExecutionAuthorityInput,
    authority_digest: ContentDigest,
}

impl VerifiedExecutionAuthority {
    fn from_validated_input(
        input: ExecutionAuthorityInput,
    ) -> Result<Self, ExecutionAuthorityError> {
        input.validate()?;
        let authority_digest = authority_digest(&input)?;
        Ok(Self {
            input,
            authority_digest,
        })
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.input.task_ref
    }

    #[must_use]
    pub const fn successor_stream_id(&self) -> &ContentDigest {
        &self.input.successor_stream_id
    }

    #[must_use]
    pub const fn task_spec_digest(&self) -> &ContentDigest {
        &self.input.task_spec_digest
    }

    #[must_use]
    pub const fn approval_subject_digest(&self) -> &ContentDigest {
        &self.input.approval_subject_digest
    }

    #[must_use]
    pub const fn budget_digest(&self) -> &ContentDigest {
        &self.input.budget_digest
    }

    #[must_use]
    pub const fn source(&self) -> ExecutionAuthoritySource {
        self.input.source
    }

    #[must_use]
    pub const fn capability(&self) -> ExecutionCapability {
        self.input.capability
    }

    #[must_use]
    pub const fn authority_evidence_digest(&self) -> &ContentDigest {
        &self.input.authority_evidence_digest
    }

    #[must_use]
    pub const fn approval_receipt_digest(&self) -> Option<&ContentDigest> {
        self.input.approval_receipt_digest.as_ref()
    }

    #[must_use]
    pub fn issued_at(&self) -> &str {
        &self.input.issued_at
    }

    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.input.expires_at
    }

    #[must_use]
    pub const fn authority_digest(&self) -> &ContentDigest {
        &self.authority_digest
    }

    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedExecutionAuthority {
        UntrustedExecutionAuthority {
            record_schema: EXECUTION_AUTHORITY_SCHEMA.to_owned(),
            input: self.input.clone(),
            authority_digest: self.authority_digest.clone(),
        }
    }
}

/// Exact execution-specific commitment signed through the normal Approval
/// owner lane before its secondary binding receipt is issued.
///
/// The ordinary typed `ApprovalSubject::Execution` commits the Task Spec but
/// has no task-reference, successor-stream, or budget fields. This
/// domain-separated subject closes that gap without widening the approval
/// capability. The base receipt retains its signer/authenticator evidence;
/// only the separate owner-issued binding receipt can authorize this subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionApprovalSubject {
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    binding: SubjectBinding,
    approval_subject_digest: ContentDigest,
    budget_digest: ContentDigest,
    subject_digest: ContentDigest,
}

impl ExecutionApprovalSubject {
    /// Constructs and hashes one complete execution-specific approval subject.
    ///
    /// # Errors
    /// Rejects zero task, successor, subject, or budget digests.
    pub fn new(
        task_ref: ContentDigest,
        successor_stream_id: ContentDigest,
        binding: SubjectBinding,
        approval_subject_digest: ContentDigest,
        budget_digest: ContentDigest,
    ) -> Result<Self, ExecutionAuthorityError> {
        if [
            &task_ref,
            &successor_stream_id,
            binding.task_spec_digest(),
            &approval_subject_digest,
            &budget_digest,
        ]
        .into_iter()
        .any(is_zero_digest)
        {
            return Err(ExecutionAuthorityError::MalformedField);
        }
        let subject_digest = execution_approval_subject_digest(
            &task_ref,
            &successor_stream_id,
            &binding,
            &approval_subject_digest,
            &budget_digest,
        )?;
        Ok(Self {
            task_ref,
            successor_stream_id,
            binding,
            approval_subject_digest,
            budget_digest,
            subject_digest,
        })
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.task_ref
    }

    #[must_use]
    pub const fn successor_stream_id(&self) -> &ContentDigest {
        &self.successor_stream_id
    }

    #[must_use]
    pub const fn binding(&self) -> &SubjectBinding {
        &self.binding
    }

    #[must_use]
    pub const fn approval_subject_digest(&self) -> &ContentDigest {
        &self.approval_subject_digest
    }

    #[must_use]
    pub const fn budget_digest(&self) -> &ContentDigest {
        &self.budget_digest
    }

    /// Returns the domain-separated execution-specific subject digest.
    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }
}

fn execution_approval_subject_digest(
    task_ref: &ContentDigest,
    successor_stream_id: &ContentDigest,
    binding: &SubjectBinding,
    approval_subject_digest: &ContentDigest,
    budget_digest: &ContentDigest,
) -> Result<ContentDigest, ExecutionAuthorityError> {
    let value = CanonicalValue::Object(vec![
        (
            "schema".to_owned(),
            CanonicalValue::String("lattice.approval.execution-approval-subject/1.0".to_owned()),
        ),
        (
            "task_ref".to_owned(),
            CanonicalValue::String(task_ref.as_str().to_owned()),
        ),
        (
            "successor_stream_id".to_owned(),
            CanonicalValue::String(successor_stream_id.as_str().to_owned()),
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
        (
            "approval_subject_digest".to_owned(),
            CanonicalValue::String(approval_subject_digest.as_str().to_owned()),
        ),
        (
            "budget_digest".to_owned(),
            CanonicalValue::String(budget_digest.as_str().to_owned()),
        ),
    ]);
    let domain = HashDomain::new("lattice.approval.execution-approval-subject", "1.0")
        .map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    let digest =
        canonical_sha256(&domain, &value).map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| ExecutionAuthorityError::Canonicalization)
}

/// Approval-owner challenge that adds exact task/successor/budget commitment
/// without changing the signer/authenticator evidence carried by the base
/// Approval challenge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionApprovalChallenge {
    approval_challenge: ApprovalChallenge,
    subject: ExecutionApprovalSubject,
    challenge_digest: ContentDigest,
}

impl ExecutionApprovalChallenge {
    /// Binds an already owner-issued normal execution challenge to the full
    /// managed execution subject before the responsible user signs it.
    pub fn new(
        approval_challenge: ApprovalChallenge,
        subject: ExecutionApprovalSubject,
    ) -> Result<Self, ExecutionAuthorityError> {
        validate_execution_approval_challenge(&approval_challenge, &subject)?;
        let challenge_digest = execution_approval_challenge_digest(
            approval_challenge.challenge_digest(),
            subject.subject_digest(),
        )?;
        Ok(Self {
            approval_challenge,
            subject,
            challenge_digest,
        })
    }

    #[must_use]
    pub const fn approval_challenge(&self) -> &ApprovalChallenge {
        &self.approval_challenge
    }

    #[must_use]
    pub const fn subject(&self) -> &ExecutionApprovalSubject {
        &self.subject
    }

    #[must_use]
    pub const fn challenge_digest(&self) -> &ContentDigest {
        &self.challenge_digest
    }
}

/// Safe fake proof that the responsible-user signer saw the exact execution
/// binding. It retains the ordinary proof unchanged, including its independent
/// signer/authenticator evidence digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeExecutionApprovalProof {
    base_proof: FakeApprovalProof,
    execution_challenge_digest: ContentDigest,
    proof_digest: ContentDigest,
}

impl FakeExecutionApprovalProof {
    #[must_use]
    pub const fn base_proof(&self) -> &FakeApprovalProof {
        &self.base_proof
    }

    #[must_use]
    pub const fn execution_challenge_digest(&self) -> &ContentDigest {
        &self.execution_challenge_digest
    }

    #[must_use]
    pub const fn proof_digest(&self) -> &ContentDigest {
        &self.proof_digest
    }
}

pub(crate) fn recover_fake_execution_approval_proof(
    base_proof: FakeApprovalProof,
    execution_challenge_digest: ContentDigest,
    proof_digest: ContentDigest,
) -> Result<FakeExecutionApprovalProof, ExecutionAuthorityError> {
    if proof_digest
        != execution_approval_proof_digest(&execution_challenge_digest, base_proof.proof_digest())?
    {
        return Err(ExecutionAuthorityError::BindingMismatch);
    }
    Ok(FakeExecutionApprovalProof {
        base_proof,
        execution_challenge_digest,
        proof_digest,
    })
}

impl FakeNormalSigner {
    /// Signs the exact execution-binding challenge while preserving the base
    /// normal-lane proof and signer evidence semantics.
    pub fn sign_execution(
        &self,
        challenge: &ExecutionApprovalChallenge,
    ) -> Result<FakeExecutionApprovalProof, ExecutionAuthorityError> {
        let base_proof = self
            .sign(challenge.approval_challenge())
            .map_err(|_| ExecutionAuthorityError::BindingMismatch)?;
        let proof_digest = execution_approval_proof_digest(
            challenge.challenge_digest(),
            base_proof.proof_digest(),
        )?;
        Ok(FakeExecutionApprovalProof {
            base_proof,
            execution_challenge_digest: challenge.challenge_digest().clone(),
            proof_digest,
        })
    }
}

/// Approval-owner receipt proving that the exact responsible-user proof and
/// ordinary authority receipt are bound to one execution-specific subject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionApprovalBindingReceipt {
    subject: ExecutionApprovalSubject,
    approval_receipt_digest: ContentDigest,
    execution_challenge_digest: ContentDigest,
    execution_proof_digest: ContentDigest,
    binding_receipt_digest: ContentDigest,
}

impl ExecutionApprovalBindingReceipt {
    #[must_use]
    pub const fn subject(&self) -> &ExecutionApprovalSubject {
        &self.subject
    }

    #[must_use]
    pub const fn approval_receipt_digest(&self) -> &ContentDigest {
        &self.approval_receipt_digest
    }

    #[must_use]
    pub const fn execution_challenge_digest(&self) -> &ContentDigest {
        &self.execution_challenge_digest
    }

    #[must_use]
    pub const fn execution_proof_digest(&self) -> &ContentDigest {
        &self.execution_proof_digest
    }

    #[must_use]
    pub const fn binding_receipt_digest(&self) -> &ContentDigest {
        &self.binding_receipt_digest
    }
}

/// Verifies the exact execution-aware responsible-user proof and issues the
/// secondary owner receipt consumed by local execution-authority issuance.
pub(crate) fn issue_execution_approval_binding_receipt(
    challenge: &ExecutionApprovalChallenge,
    proof: &FakeExecutionApprovalProof,
    approval_receipt: &ApprovalAuthorityReceipt,
    current_approval_head: &ApprovalAuthorityHead,
) -> Result<ExecutionApprovalBindingReceipt, ExecutionAuthorityError> {
    validate_execution_approval_challenge(challenge.approval_challenge(), challenge.subject())?;
    if proof.execution_challenge_digest() != challenge.challenge_digest()
        || proof.proof_digest()
            != &execution_approval_proof_digest(
                challenge.challenge_digest(),
                proof.base_proof().proof_digest(),
            )?
        || !proof_matches(challenge.approval_challenge(), proof.base_proof())
            .map_err(|_| ExecutionAuthorityError::BindingMismatch)?
        || approval_receipt.head() != *current_approval_head
        || approval_receipt.status() != ApprovalStatus::Available
        || approval_receipt.identity() != challenge.approval_challenge().identity()
        || approval_receipt.challenge_digest() != challenge.approval_challenge().challenge_digest()
        || approval_receipt.proof_digest() != proof.base_proof().proof_digest()
        || approval_receipt.evidence_digest() != proof.base_proof().evidence_digest()
    {
        return Err(ExecutionAuthorityError::BindingMismatch);
    }
    let binding_receipt_digest = execution_approval_binding_receipt_digest(
        challenge.subject().subject_digest(),
        approval_receipt.receipt_digest(),
        challenge.challenge_digest(),
        proof.proof_digest(),
    )?;
    Ok(ExecutionApprovalBindingReceipt {
        subject: challenge.subject().clone(),
        approval_receipt_digest: approval_receipt.receipt_digest().clone(),
        execution_challenge_digest: challenge.challenge_digest().clone(),
        execution_proof_digest: proof.proof_digest().clone(),
        binding_receipt_digest,
    })
}

fn validate_execution_approval_challenge(
    challenge: &ApprovalChallenge,
    subject: &ExecutionApprovalSubject,
) -> Result<(), ExecutionAuthorityError> {
    let ApprovalSubject::Execution { task_spec_hash, .. } = challenge.identity().subject() else {
        return Err(ExecutionAuthorityError::BindingMismatch);
    };
    if challenge.identity().lane() != ApprovalLane::Normal
        || challenge.identity().authority() != ApprovalAuthority::ResponsibleUser
        || challenge.identity().binding() != subject.binding()
        || challenge.subject_digest() != subject.approval_subject_digest()
        || task_spec_hash != subject.binding().task_spec_digest()
    {
        return Err(ExecutionAuthorityError::BindingMismatch);
    }
    Ok(())
}

fn execution_approval_challenge_digest(
    base_challenge_digest: &ContentDigest,
    execution_subject_digest: &ContentDigest,
) -> Result<ContentDigest, ExecutionAuthorityError> {
    hash_execution_binding(
        "lattice.approval.execution-approval-challenge",
        CanonicalValue::Object(vec![
            (
                "base_challenge_digest".to_owned(),
                CanonicalValue::String(base_challenge_digest.as_str().to_owned()),
            ),
            (
                "execution_subject_digest".to_owned(),
                CanonicalValue::String(execution_subject_digest.as_str().to_owned()),
            ),
        ]),
    )
}

fn execution_approval_proof_digest(
    execution_challenge_digest: &ContentDigest,
    base_proof_digest: &ContentDigest,
) -> Result<ContentDigest, ExecutionAuthorityError> {
    hash_execution_binding(
        "lattice.approval.execution-approval-proof",
        CanonicalValue::Object(vec![
            (
                "execution_challenge_digest".to_owned(),
                CanonicalValue::String(execution_challenge_digest.as_str().to_owned()),
            ),
            (
                "base_proof_digest".to_owned(),
                CanonicalValue::String(base_proof_digest.as_str().to_owned()),
            ),
        ]),
    )
}

fn execution_approval_binding_receipt_digest(
    subject_digest: &ContentDigest,
    approval_receipt_digest: &ContentDigest,
    challenge_digest: &ContentDigest,
    proof_digest: &ContentDigest,
) -> Result<ContentDigest, ExecutionAuthorityError> {
    hash_execution_binding(
        "lattice.approval.execution-approval-binding-receipt",
        CanonicalValue::Object(vec![
            (
                "execution_subject_digest".to_owned(),
                CanonicalValue::String(subject_digest.as_str().to_owned()),
            ),
            (
                "approval_receipt_digest".to_owned(),
                CanonicalValue::String(approval_receipt_digest.as_str().to_owned()),
            ),
            (
                "execution_challenge_digest".to_owned(),
                CanonicalValue::String(challenge_digest.as_str().to_owned()),
            ),
            (
                "execution_proof_digest".to_owned(),
                CanonicalValue::String(proof_digest.as_str().to_owned()),
            ),
        ]),
    )
}

fn hash_execution_binding(
    domain_name: &'static str,
    value: CanonicalValue,
) -> Result<ContentDigest, ExecutionAuthorityError> {
    let domain = HashDomain::new(domain_name, "1.0")
        .map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    let digest =
        canonical_sha256(&domain, &value).map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| ExecutionAuthorityError::Canonicalization)
}

/// Complete owner-supplied proof needed to convert one independently current
/// normal execution approval into bounded local execution authority. Callers
/// cannot substitute a bare receipt digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedApprovalExecutionContext {
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    budget_digest: ContentDigest,
    approval_subject_digest: ContentDigest,
    execution_binding_receipt: ExecutionApprovalBindingReceipt,
    approval_receipt: ApprovalAuthorityReceipt,
    current_approval_head: ApprovalAuthorityHead,
}

impl VerifiedApprovalExecutionContext {
    /// Legacy constructor retained only to fail closed for receipts that lack
    /// the execution-specific owner challenge/proof/binding receipt.
    ///
    /// # Errors
    ///
    /// Always rejects. Use [`Self::new_with_binding_receipt`] after the exact
    /// execution-aware owner flow.
    pub fn new(
        task_ref: ContentDigest,
        successor_stream_id: ContentDigest,
        budget_digest: ContentDigest,
        approval_subject_digest: ContentDigest,
        approval_receipt: ApprovalAuthorityReceipt,
        current_approval_head: ApprovalAuthorityHead,
    ) -> Result<Self, ExecutionAuthorityError> {
        let _ = (
            task_ref,
            successor_stream_id,
            budget_digest,
            approval_subject_digest,
            approval_receipt,
            current_approval_head,
        );
        Err(ExecutionAuthorityError::BindingMismatch)
    }

    /// Constructs a verified-approval execution context from the secondary
    /// owner receipt produced by the exact execution-aware challenge/proof.
    pub fn new_with_binding_receipt(
        execution_binding_receipt: ExecutionApprovalBindingReceipt,
        approval_receipt: ApprovalAuthorityReceipt,
        current_approval_head: ApprovalAuthorityHead,
    ) -> Result<Self, ExecutionAuthorityError> {
        let subject = execution_binding_receipt.subject();
        let value = Self {
            task_ref: subject.task_ref().clone(),
            successor_stream_id: subject.successor_stream_id().clone(),
            budget_digest: subject.budget_digest().clone(),
            approval_subject_digest: subject.approval_subject_digest().clone(),
            execution_binding_receipt,
            approval_receipt,
            current_approval_head,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), ExecutionAuthorityError> {
        if [
            &self.task_ref,
            &self.successor_stream_id,
            &self.budget_digest,
            &self.approval_subject_digest,
        ]
        .into_iter()
        .any(is_zero_digest)
            || self.approval_receipt.head() != self.current_approval_head
            || self.approval_receipt.status() != ApprovalStatus::Available
            || self.current_approval_head.status() != ApprovalStatus::Available
            || self.approval_receipt.identity().lane() != ApprovalLane::Normal
            || self.approval_receipt.identity().authority() != ApprovalAuthority::ResponsibleUser
            || self.approval_receipt.subject_digest() != &self.approval_subject_digest
        {
            return Err(ExecutionAuthorityError::BindingMismatch);
        }
        let ApprovalSubject::Execution { task_spec_hash, .. } =
            self.approval_receipt.identity().subject()
        else {
            return Err(ExecutionAuthorityError::BindingMismatch);
        };
        let execution_subject = self.execution_binding_receipt.subject();
        if task_spec_hash != execution_subject.binding().task_spec_digest()
            || self.approval_receipt.identity().binding() != execution_subject.binding()
            || self.execution_binding_receipt.approval_receipt_digest()
                != self.approval_receipt.receipt_digest()
            || self.execution_binding_receipt.binding_receipt_digest()
                != &execution_approval_binding_receipt_digest(
                    execution_subject.subject_digest(),
                    self.approval_receipt.receipt_digest(),
                    self.execution_binding_receipt.execution_challenge_digest(),
                    self.execution_binding_receipt.execution_proof_digest(),
                )?
        {
            return Err(ExecutionAuthorityError::BindingMismatch);
        }
        canonical_utc(self.approval_receipt.issued_at())?;
        canonical_utc(self.approval_receipt.expires_at())?;
        Ok(())
    }
}

/// Issues bounded local execution authority only from an actual current
/// Approval Verifier receipt/head pair. The authority validity is exactly the
/// receipt validity; callers cannot widen it or supply an arbitrary digest.
///
/// # Errors
///
/// Rejects stale/substituted approval state or an observation outside the
/// receipt validity window.
pub fn issue_verified_approval_execution_authority(
    context: &VerifiedApprovalExecutionContext,
    observed_at: &str,
) -> Result<VerifiedExecutionAuthority, ExecutionAuthorityError> {
    context.validate()?;
    require_current_approval_window(&context.approval_receipt, observed_at)?;
    let evidence = verified_approval_evidence_digest(context)?;
    VerifiedExecutionAuthority::from_validated_input(ExecutionAuthorityInput::new(
        context.task_ref.clone(),
        context.successor_stream_id.clone(),
        context
            .approval_receipt
            .identity()
            .binding()
            .task_spec_digest()
            .clone(),
        context.approval_subject_digest.clone(),
        context.budget_digest.clone(),
        ExecutionAuthoritySource::VerifiedApproval,
        ExecutionCapability::LocalReversibleTaskExecution,
        evidence,
        Some(context.approval_receipt.receipt_digest().clone()),
        context.approval_receipt.issued_at(),
        context.approval_receipt.expires_at(),
    )?)
}

/// Revalidates a persisted verified-approval authority against a fresh
/// Approval Verifier current head and the exact issuance context.
///
/// # Errors
///
/// Rejects expiry, revocation/claim, or any task/spec/budget/receipt change.
pub fn reverify_verified_approval_execution_authority(
    authority: &VerifiedExecutionAuthority,
    context: &VerifiedApprovalExecutionContext,
    observed_at: &str,
) -> Result<(), ExecutionAuthorityError> {
    context.validate()?;
    require_current_approval_window(&context.approval_receipt, observed_at)?;
    if authority.source() != ExecutionAuthoritySource::VerifiedApproval
        || authority.capability() != ExecutionCapability::LocalReversibleTaskExecution
        || authority.task_ref() != &context.task_ref
        || authority.successor_stream_id() != &context.successor_stream_id
        || authority.task_spec_digest()
            != context
                .approval_receipt
                .identity()
                .binding()
                .task_spec_digest()
        || authority.approval_subject_digest() != &context.approval_subject_digest
        || authority.budget_digest() != &context.budget_digest
        || authority.approval_receipt_digest() != Some(context.approval_receipt.receipt_digest())
        || authority.issued_at() != context.approval_receipt.issued_at()
        || authority.expires_at() != context.approval_receipt.expires_at()
        || authority.authority_evidence_digest() != &verified_approval_evidence_digest(context)?
    {
        return Err(ExecutionAuthorityError::BindingMismatch);
    }
    Ok(())
}

fn require_current_approval_window(
    receipt: &ApprovalAuthorityReceipt,
    observed_at: &str,
) -> Result<(), ExecutionAuthorityError> {
    let observed = canonical_utc(observed_at)?;
    let issued = canonical_utc(receipt.issued_at())?;
    let expires = canonical_utc(receipt.expires_at())?;
    if observed < issued {
        return Err(ExecutionAuthorityError::NotYetValid);
    }
    if observed >= expires {
        return Err(ExecutionAuthorityError::Expired);
    }
    Ok(())
}

fn verified_approval_evidence_digest(
    context: &VerifiedApprovalExecutionContext,
) -> Result<ContentDigest, ExecutionAuthorityError> {
    let value = CanonicalValue::Object(vec![
        (
            "schema".to_owned(),
            CanonicalValue::String(
                "lattice.approval.verified-approval-execution-evidence/1.0".to_owned(),
            ),
        ),
        (
            "task_ref".to_owned(),
            CanonicalValue::String(context.task_ref.as_str().to_owned()),
        ),
        (
            "successor_stream_id".to_owned(),
            CanonicalValue::String(context.successor_stream_id.as_str().to_owned()),
        ),
        (
            "task_spec_digest".to_owned(),
            CanonicalValue::String(
                context
                    .approval_receipt
                    .identity()
                    .binding()
                    .task_spec_digest()
                    .as_str()
                    .to_owned(),
            ),
        ),
        (
            "approval_subject_digest".to_owned(),
            CanonicalValue::String(context.approval_subject_digest.as_str().to_owned()),
        ),
        (
            "budget_digest".to_owned(),
            CanonicalValue::String(context.budget_digest.as_str().to_owned()),
        ),
        (
            "approval_receipt_digest".to_owned(),
            CanonicalValue::String(
                context
                    .approval_receipt
                    .receipt_digest()
                    .as_str()
                    .to_owned(),
            ),
        ),
        (
            "current_head_receipt_digest".to_owned(),
            CanonicalValue::String(
                context
                    .current_approval_head
                    .receipt_digest()
                    .as_str()
                    .to_owned(),
            ),
        ),
        (
            "execution_binding_receipt_digest".to_owned(),
            CanonicalValue::String(
                context
                    .execution_binding_receipt
                    .binding_receipt_digest()
                    .as_str()
                    .to_owned(),
            ),
        ),
    ]);
    let domain = HashDomain::new(
        "lattice.approval.verified-approval-execution-evidence",
        "1.0",
    )
    .map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    let digest =
        canonical_sha256(&domain, &value).map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| ExecutionAuthorityError::Canonicalization)
}

/// Complete typed binding required to issue or reverify a bounded local
/// execution authority. The Policy owner evaluates the exact immutable
/// `TaskSpec` and current facts before supplying its opaque typed decision;
/// this module never calls Policy or interprets objective text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClosedPolicyExecutionContext {
    task_ref: ContentDigest,
    successor_stream_id: ContentDigest,
    binding: SubjectBinding,
    approval_subject_digest: ContentDigest,
    budget_digest: ContentDigest,
    project_receipt: ProjectAuthorityReceipt,
    current_project_head: ProjectAuthorityHead,
    issued_at: String,
    expires_at: String,
}

impl ClosedPolicyExecutionContext {
    /// Constructs the complete closed binding. Policy evaluation is a
    /// separate mandatory Runtime step; issue and reverify receive only its
    /// opaque typed decision result.
    ///
    /// # Errors
    ///
    /// Returns an error when a digest is zero, the project identities do not
    /// match, either timestamp is non-canonical, or the validity window is not
    /// strictly increasing.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_ref: ContentDigest,
        successor_stream_id: ContentDigest,
        binding: SubjectBinding,
        approval_subject_digest: ContentDigest,
        budget_digest: ContentDigest,
        project_receipt: ProjectAuthorityReceipt,
        current_project_head: ProjectAuthorityHead,
        issued_at: impl Into<String>,
        expires_at: impl Into<String>,
    ) -> Result<Self, ExecutionAuthorityError> {
        let value = Self {
            task_ref,
            successor_stream_id,
            binding,
            approval_subject_digest,
            budget_digest,
            project_receipt,
            current_project_head,
            issued_at: issued_at.into(),
            expires_at: expires_at.into(),
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), ExecutionAuthorityError> {
        if [
            &self.task_ref,
            &self.successor_stream_id,
            &self.approval_subject_digest,
            &self.budget_digest,
        ]
        .into_iter()
        .any(is_zero_digest)
        {
            return Err(ExecutionAuthorityError::MalformedField);
        }
        if self.binding.project_id() != self.project_receipt.project_id()
            || self.binding.project_snapshot_id() != self.project_receipt.project_snapshot_id()
            || self.binding.project_id() != self.current_project_head.project_id()
            || self.binding.project_snapshot_id() != self.current_project_head.project_snapshot_id()
        {
            return Err(ExecutionAuthorityError::BindingMismatch);
        }
        let issued = canonical_utc(&self.issued_at)?;
        let expires = canonical_utc(&self.expires_at)?;
        if issued >= expires {
            return Err(ExecutionAuthorityError::InvalidValidityWindow);
        }
        Ok(())
    }
}

/// Issues one formal closed-policy authority only from the opaque typed result
/// returned by Policy V2 for the exact runtime-owned `ExecutionGate` input.
/// `PolicyDecision` cannot be caller-constructed outside the Policy crate.
///
/// # Errors
///
/// Returns an error when the context or policy evidence is malformed,
/// non-current, denied, or not bound to the exact task/spec/budget subject.
pub fn issue_closed_policy_execution_authority(
    context: &ClosedPolicyExecutionContext,
    policy_evidence: &ExecutionGateDecisionEvidence,
) -> Result<VerifiedExecutionAuthority, ExecutionAuthorityError> {
    context.validate()?;
    let decision = require_execution_evidence(context, policy_evidence)?;
    let evidence = closed_policy_evidence_digest(context, decision)?;
    VerifiedExecutionAuthority::from_validated_input(ExecutionAuthorityInput::new(
        context.task_ref.clone(),
        context.successor_stream_id.clone(),
        context.binding.task_spec_digest().clone(),
        context.approval_subject_digest.clone(),
        context.budget_digest.clone(),
        ExecutionAuthoritySource::ClosedPolicyNoApprovalRequired,
        ExecutionCapability::LocalReversibleTaskExecution,
        evidence,
        None,
        context.issued_at.clone(),
        context.expires_at.clone(),
    )?)
}

/// Validates a fresh opaque Policy V2 result plus every persisted
/// task/spec/budget/time/evidence binding. Runtime must re-evaluate the exact
/// owner facts before this call; merely rehashing the persistence row is
/// deliberately insufficient.
///
/// # Errors
///
/// Returns an error when any retained authority binding differs, the fresh
/// policy evidence is denied or mismatched, or `observed_at` falls outside the
/// exact validity window.
pub fn reverify_closed_policy_execution_authority(
    authority: &VerifiedExecutionAuthority,
    context: &ClosedPolicyExecutionContext,
    policy_evidence: &ExecutionGateDecisionEvidence,
    observed_at: &str,
) -> Result<(), ExecutionAuthorityError> {
    context.validate()?;
    if authority.source() != ExecutionAuthoritySource::ClosedPolicyNoApprovalRequired
        || authority.capability() != ExecutionCapability::LocalReversibleTaskExecution
        || authority.task_ref() != &context.task_ref
        || authority.successor_stream_id() != &context.successor_stream_id
        || authority.task_spec_digest() != context.binding.task_spec_digest()
        || authority.approval_subject_digest() != &context.approval_subject_digest
        || authority.budget_digest() != &context.budget_digest
        || authority.approval_receipt_digest().is_some()
        || authority.issued_at() != context.issued_at
        || authority.expires_at() != context.expires_at
    {
        return Err(ExecutionAuthorityError::BindingMismatch);
    }
    let observed = canonical_utc(observed_at)?;
    let issued = canonical_utc(authority.issued_at())?;
    let expires = canonical_utc(authority.expires_at())?;
    if observed < issued {
        return Err(ExecutionAuthorityError::NotYetValid);
    }
    if observed >= expires {
        return Err(ExecutionAuthorityError::Expired);
    }
    let decision = require_execution_evidence(context, policy_evidence)?;
    if authority.authority_evidence_digest() != &closed_policy_evidence_digest(context, decision)? {
        return Err(ExecutionAuthorityError::PolicyEvidenceMismatch);
    }
    Ok(())
}

fn require_execution_evidence(
    context: &ClosedPolicyExecutionContext,
    policy_evidence: &ExecutionGateDecisionEvidence,
) -> Result<PolicyDecision, ExecutionAuthorityError> {
    let decision = policy_evidence.decision();
    require_execution_allow(decision)?;
    let managed_binding = policy_evidence
        .managed_execution_binding()
        .ok_or(ExecutionAuthorityError::PolicyEvidenceMismatch)?;
    if policy_evidence.task_spec_digest() != Some(context.binding.task_spec_digest())
        || policy_evidence.project_binding() != Some(&context.binding)
        || policy_evidence.project_receipt() != Some(&context.project_receipt)
        || policy_evidence.current_project_head() != Some(&context.current_project_head)
        || managed_binding.task_ref != context.task_ref
        || managed_binding.successor_stream_id != context.successor_stream_id
        || managed_binding.task_spec_digest != *context.binding.task_spec_digest()
        || managed_binding.approval_subject_digest != context.approval_subject_digest
        || managed_binding.budget_digest != context.budget_digest
        || !policy_evidence.is_awaiting_execution_approval()
        || !policy_evidence.is_runtime_active()
    {
        return Err(ExecutionAuthorityError::PolicyEvidenceMismatch);
    }
    Ok(decision)
}

fn require_execution_allow(decision: PolicyDecision) -> Result<(), ExecutionAuthorityError> {
    if decision.allowed()
        && decision.evidence().contract_version() == lattice_policy::POLICY_CONTRACT_VERSION
        && decision.evidence().subject() == DecisionKind::ExecutionGate
        && decision.evidence().checked_through() == DecisionStage::Complete
        && decision.reason().code() == "EXECUTION_GATE_ALLOWED"
    {
        Ok(())
    } else {
        Err(ExecutionAuthorityError::PolicyDenied)
    }
}

fn closed_policy_evidence_digest(
    context: &ClosedPolicyExecutionContext,
    decision: PolicyDecision,
) -> Result<ContentDigest, ExecutionAuthorityError> {
    let value = CanonicalValue::Object(vec![
        (
            "schema".to_owned(),
            CanonicalValue::String(
                "lattice.approval.closed-policy-execution-evidence/1.0".to_owned(),
            ),
        ),
        (
            "policy_contract_version".to_owned(),
            CanonicalValue::String(decision.evidence().contract_version().to_string()),
        ),
        (
            "decision_subject".to_owned(),
            CanonicalValue::String("EXECUTION_GATE".to_owned()),
        ),
        (
            "decision_reason".to_owned(),
            CanonicalValue::String(decision.reason().code().to_owned()),
        ),
        (
            "checked_through".to_owned(),
            CanonicalValue::String("COMPLETE".to_owned()),
        ),
        (
            "task_ref".to_owned(),
            CanonicalValue::String(context.task_ref.as_str().to_owned()),
        ),
        (
            "successor_stream_id".to_owned(),
            CanonicalValue::String(context.successor_stream_id.as_str().to_owned()),
        ),
        (
            "task_spec_digest".to_owned(),
            CanonicalValue::String(context.binding.task_spec_digest().as_str().to_owned()),
        ),
        (
            "approval_subject_digest".to_owned(),
            CanonicalValue::String(context.approval_subject_digest.as_str().to_owned()),
        ),
        (
            "budget_digest".to_owned(),
            CanonicalValue::String(context.budget_digest.as_str().to_owned()),
        ),
        (
            "project_authority_receipt_digest".to_owned(),
            CanonicalValue::String(context.project_receipt.receipt_digest().as_str().to_owned()),
        ),
        (
            "current_project_head_receipt_digest".to_owned(),
            CanonicalValue::String(
                context
                    .current_project_head
                    .receipt_digest()
                    .as_str()
                    .to_owned(),
            ),
        ),
        (
            "task_state".to_owned(),
            CanonicalValue::String("AWAITING_EXECUTION_APPROVAL".to_owned()),
        ),
        (
            "runtime_admission".to_owned(),
            CanonicalValue::String("ACTIVE".to_owned()),
        ),
        (
            "issued_at".to_owned(),
            CanonicalValue::String(context.issued_at.clone()),
        ),
        (
            "expires_at".to_owned(),
            CanonicalValue::String(context.expires_at.clone()),
        ),
        (
            "capability".to_owned(),
            CanonicalValue::String(
                ExecutionCapability::LocalReversibleTaskExecution
                    .as_str()
                    .to_owned(),
            ),
        ),
    ]);
    let domain = HashDomain::new("lattice.approval.closed-policy-execution-evidence", "1.0")
        .map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    let digest =
        canonical_sha256(&domain, &value).map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| ExecutionAuthorityError::Canonicalization)
}

/// Persistence-shaped row that must be reverified after loading.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UntrustedExecutionAuthority {
    record_schema: String,
    input: ExecutionAuthorityInput,
    authority_digest: ContentDigest,
}

impl UntrustedExecutionAuthority {
    /// Rehydrates one explicitly untrusted persistence row. Call
    /// [`verify_untrusted_execution_authority`] to prove structural integrity;
    /// a closed-policy consumer must additionally call
    /// [`reverify_closed_policy_execution_authority`] with fresh owner facts
    /// before treating the row as current authority.
    #[must_use]
    pub fn new(
        record_schema: impl Into<String>,
        input: ExecutionAuthorityInput,
        authority_digest: ContentDigest,
    ) -> Self {
        Self {
            record_schema: record_schema.into(),
            input,
            authority_digest,
        }
    }

    #[must_use]
    pub fn with_budget_digest(mut self, budget_digest: ContentDigest) -> Self {
        self.input.budget_digest = budget_digest;
        self
    }

    #[must_use]
    pub fn record_schema(&self) -> &str {
        &self.record_schema
    }

    #[must_use]
    pub const fn input(&self) -> &ExecutionAuthorityInput {
        &self.input
    }

    #[must_use]
    pub const fn authority_digest(&self) -> &ContentDigest {
        &self.authority_digest
    }
}

/// Revalidates and rehashes a loaded authority row without granting current
/// execution authority. Closed-policy consumers must additionally perform the
/// fresh Policy/Registry/runtime check in
/// [`reverify_closed_policy_execution_authority`].
///
/// # Errors
/// Unknown schema, invalid bindings, or a changed digest fail closed.
pub fn verify_untrusted_execution_authority(
    value: &UntrustedExecutionAuthority,
) -> Result<VerifiedExecutionAuthority, ExecutionAuthorityError> {
    if value.record_schema != EXECUTION_AUTHORITY_SCHEMA {
        return Err(ExecutionAuthorityError::MalformedField);
    }
    let verified = VerifiedExecutionAuthority::from_validated_input(value.input.clone())?;
    if verified.authority_digest != value.authority_digest {
        return Err(ExecutionAuthorityError::DigestMismatch);
    }
    Ok(verified)
}

fn authority_digest(
    input: &ExecutionAuthorityInput,
) -> Result<ContentDigest, ExecutionAuthorityError> {
    let domain = HashDomain::new("lattice.approval.execution-authority", "1.0")
        .map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    let digest = canonical_sha256(&domain, &authority_value(input))
        .map_err(|_| ExecutionAuthorityError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| ExecutionAuthorityError::Canonicalization)
}

fn authority_value(input: &ExecutionAuthorityInput) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "record_schema".to_owned(),
            CanonicalValue::String(EXECUTION_AUTHORITY_SCHEMA.to_owned()),
        ),
        (
            "task_ref".to_owned(),
            CanonicalValue::String(input.task_ref.as_str().to_owned()),
        ),
        (
            "successor_stream_id".to_owned(),
            CanonicalValue::String(input.successor_stream_id.as_str().to_owned()),
        ),
        (
            "task_spec_digest".to_owned(),
            CanonicalValue::String(input.task_spec_digest.as_str().to_owned()),
        ),
        (
            "approval_subject_digest".to_owned(),
            CanonicalValue::String(input.approval_subject_digest.as_str().to_owned()),
        ),
        (
            "budget_digest".to_owned(),
            CanonicalValue::String(input.budget_digest.as_str().to_owned()),
        ),
        (
            "source".to_owned(),
            CanonicalValue::String(input.source.as_str().to_owned()),
        ),
        (
            "capability".to_owned(),
            CanonicalValue::String(input.capability.as_str().to_owned()),
        ),
        (
            "authority_evidence_digest".to_owned(),
            CanonicalValue::String(input.authority_evidence_digest.as_str().to_owned()),
        ),
        (
            "approval_receipt_digest".to_owned(),
            match &input.approval_receipt_digest {
                Some(value) => CanonicalValue::String(value.as_str().to_owned()),
                None => CanonicalValue::Null,
            },
        ),
        (
            "issued_at".to_owned(),
            CanonicalValue::String(input.issued_at.clone()),
        ),
        (
            "expires_at".to_owned(),
            CanonicalValue::String(input.expires_at.clone()),
        ),
    ])
}

fn canonical_utc(value: &str) -> Result<OffsetDateTime, ExecutionAuthorityError> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| ExecutionAuthorityError::MalformedField)?;
    if parsed.offset() != UtcOffset::UTC || parsed.format(&Rfc3339).ok().as_deref() != Some(value) {
        return Err(ExecutionAuthorityError::MalformedField);
    }
    Ok(parsed)
}

fn is_zero_digest(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}
