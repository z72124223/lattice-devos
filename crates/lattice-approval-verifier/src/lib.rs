//! Pure Approval Verifier 1.0 semantics and a deterministic non-durable fake.

mod execution_authority;

pub use execution_authority::*;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_contracts::{
    APPROVAL_VERIFIER_PRODUCER_ID, APPROVAL_VERIFIER_PRODUCER_VERSION, ApprovalAuthority,
    ApprovalAuthorityHead, ApprovalAuthorityReceipt, ApprovalIdentity, ApprovalLane,
    ApprovalOrigin, ApprovalRevision, ApprovalStatus, ApprovalSubject, CONTRACT_VERSION,
    ContentDigest, DaemonEpoch, ExternalCostSubject, GuardianRuntimeSubject,
    MemoryCandidateSubject, MemoryKind, MergeSubject, MergeTarget, ProjectId, ProjectSnapshotId,
    ProtectedChangeClass, ProtectedChangeSubject, ProtectedReleaseSubject, ReleaseSubject,
    RuntimeKind, SubjectBinding, TaskId, UpgradeDelta,
};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

const SCHEMA_VERSION: &str = "1.0";
const MAX_SIGNED_BIGINT: u64 = i64::MAX as u64;
const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

/// One stable terminal approval denial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalDenial {
    /// The complete supplied expected state head is no longer current.
    StaleHead,
    /// The approval identity is already retained.
    ApprovalAlreadyExists,
    /// No challenged approval exists under the supplied identity.
    ApprovalMissing,
    /// The requested transition is illegal for the current approval state.
    InvalidState,
    /// The nonce commitment or operational nonce identifier was already bound.
    NonceAlreadyBound,
    /// The observation precedes the challenge issue instant.
    NotYetValid,
    /// The observation is at or after the exclusive expiry instant.
    Expired,
    /// Fake proof identity, lane, key, evidence, or digest did not match.
    ProofMismatch,
    /// Only a normal-lane approval can be claimed through this module.
    NormalClaimRequired,
    /// The supplied revoker is not the original verified approver.
    RevokerMismatch,
    /// A signed `BIGINT` state revision or command ordinal would overflow.
    CounterExhausted,
}

impl ApprovalDenial {
    /// Returns a stable machine-readable denial code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::StaleHead => "APPROVAL_STALE_HEAD",
            Self::ApprovalAlreadyExists => "APPROVAL_ALREADY_EXISTS",
            Self::ApprovalMissing => "APPROVAL_MISSING",
            Self::InvalidState => "APPROVAL_INVALID_STATE",
            Self::NonceAlreadyBound => "APPROVAL_NONCE_ALREADY_BOUND",
            Self::NotYetValid => "APPROVAL_NOT_YET_VALID",
            Self::Expired => "APPROVAL_EXPIRED",
            Self::ProofMismatch => "APPROVAL_PROOF_MISMATCH",
            Self::NormalClaimRequired => "APPROVAL_NORMAL_CLAIM_REQUIRED",
            Self::RevokerMismatch => "APPROVAL_REVOKER_MISMATCH",
            Self::CounterExhausted => "APPROVAL_COUNTER_EXHAUSTED",
        }
    }
}

/// Approval construction, planning, or replay failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalVerifierError {
    /// A command or safe proof identifier violates the bounded ASCII contract.
    InvalidIdentifier,
    /// A timestamp is not exact canonical UTC RFC 3339.
    InvalidTimestamp,
    /// Challenge issue and exclusive expiry order is invalid.
    InvalidExpiry,
    /// A required digest is the all-zero sentinel.
    ZeroDigest,
    /// Secret material was empty or unreasonably large.
    InvalidSecret,
    /// A command identifier was reused with changed canonical content.
    CommandIdReuse,
    /// The deterministic fake was asked to represent a live runtime.
    FakeRuntimeRequired,
    /// A signer received a substituted or internally inconsistent challenge.
    ChallengeIntegrity,
    /// A pure plan no longer applies to the aggregate it was planned against.
    PlanPreconditionChanged,
    /// Restore attempted to overwrite non-empty retained state.
    RestoreWouldOverwrite,
    /// A verified snapshot disagrees with an independent trusted checkpoint.
    CheckpointMismatch,
    /// Shared-contract construction failed.
    Contract,
    /// Canonical hashing failed.
    Canonical,
    /// An untrusted raw snapshot failed strict replay.
    CorruptSnapshot,
}

impl ApprovalVerifierError {
    /// Returns a stable machine-readable error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidIdentifier => "APPROVAL_VERIFIER_INVALID_IDENTIFIER",
            Self::InvalidTimestamp => "APPROVAL_VERIFIER_INVALID_TIMESTAMP",
            Self::InvalidExpiry => "APPROVAL_VERIFIER_INVALID_EXPIRY",
            Self::ZeroDigest => "APPROVAL_VERIFIER_ZERO_DIGEST",
            Self::InvalidSecret => "APPROVAL_VERIFIER_INVALID_SECRET",
            Self::CommandIdReuse => "APPROVAL_VERIFIER_COMMAND_ID_REUSE",
            Self::FakeRuntimeRequired => "APPROVAL_VERIFIER_FAKE_RUNTIME_REQUIRED",
            Self::ChallengeIntegrity => "APPROVAL_VERIFIER_CHALLENGE_INTEGRITY",
            Self::PlanPreconditionChanged => "APPROVAL_VERIFIER_PLAN_PRECONDITION_CHANGED",
            Self::RestoreWouldOverwrite => "APPROVAL_VERIFIER_RESTORE_WOULD_OVERWRITE",
            Self::CheckpointMismatch => "APPROVAL_VERIFIER_CHECKPOINT_MISMATCH",
            Self::Contract => "APPROVAL_VERIFIER_CONTRACT",
            Self::Canonical => "APPROVAL_VERIFIER_CANONICAL",
            Self::CorruptSnapshot => "APPROVAL_VERIFIER_CORRUPT_SNAPSHOT",
        }
    }
}

impl fmt::Display for ApprovalVerifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ApprovalVerifierError {}

/// Ephemeral raw material accepted only to derive a safe commitment.
///
/// The bytes are never cloned, serialized, persisted, or printed. They are
/// overwritten on drop on a best-effort basis.
pub struct SecretMaterial {
    bytes: Vec<u8>,
}

impl SecretMaterial {
    /// Owns bounded non-empty raw bytes.
    ///
    /// # Errors
    ///
    /// Rejects empty input and values larger than 64 KiB.
    pub fn new(bytes: Vec<u8>) -> Result<Self, ApprovalVerifierError> {
        if bytes.is_empty() || bytes.len() > 65_536 {
            return Err(ApprovalVerifierError::InvalidSecret);
        }
        Ok(Self { bytes })
    }
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretMaterial")
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

impl Drop for SecretMaterial {
    fn drop(&mut self) {
        self.bytes.fill(0);
    }
}

/// Derives the globally comparable safe commitment for one raw nonce.
///
/// # Errors
///
/// Returns a canonical hashing or shared-contract construction error.
pub fn nonce_commitment(secret: &SecretMaterial) -> Result<ContentDigest, ApprovalVerifierError> {
    digest(
        "lattice-approval-nonce-commitment",
        CanonicalValue::Object(vec![(
            "secret_hex".to_owned(),
            string(hex_bytes(&secret.bytes)),
        )]),
    )
}

/// One deterministic fake normal-lane signer.
#[derive(Clone, Eq, PartialEq)]
pub struct FakeNormalSigner {
    approver_id: String,
    authenticator_id: String,
    key_id: String,
    verification_key_commitment: ContentDigest,
    evidence_digest: ContentDigest,
}

impl FakeNormalSigner {
    /// Constructs a visibly fake normal signer from ephemeral key material.
    ///
    /// # Errors
    ///
    /// Rejects malformed identifiers or canonical hashing failure.
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        approver_id: impl Into<String>,
        authenticator_id: impl Into<String>,
        key_id: impl Into<String>,
        secret: SecretMaterial,
    ) -> Result<Self, ApprovalVerifierError> {
        let approver_id = approver_id.into();
        let authenticator_id = authenticator_id.into();
        let key_id = key_id.into();
        validate_identifiers([&approver_id, &authenticator_id, &key_id])?;
        let verification_key_commitment = fake_key_commitment(
            ApprovalLane::Normal,
            &approver_id,
            &authenticator_id,
            &key_id,
            &secret,
        )?;
        let evidence_digest = fake_signer_evidence(
            ApprovalLane::Normal,
            &approver_id,
            &authenticator_id,
            &key_id,
            &verification_key_commitment,
            None,
        )?;
        Ok(Self {
            approver_id,
            authenticator_id,
            key_id,
            verification_key_commitment,
            evidence_digest,
        })
    }

    #[must_use]
    pub fn approver_id(&self) -> &str {
        &self.approver_id
    }

    #[must_use]
    pub fn authenticator_id(&self) -> &str {
        &self.authenticator_id
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    #[must_use]
    pub const fn verification_key_commitment(&self) -> &ContentDigest {
        &self.verification_key_commitment
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    /// Produces deterministic fake proof material for the supplied challenge.
    ///
    /// # Errors
    ///
    /// Returns a canonical hashing error.
    pub fn sign(
        &self,
        challenge: &ApprovalChallenge,
    ) -> Result<FakeApprovalProof, ApprovalVerifierError> {
        validate_normal_signer_challenge(self, challenge)?;
        FakeApprovalProof::normal(
            challenge,
            &self.approver_id,
            &self.authenticator_id,
            &self.key_id,
            self.verification_key_commitment.clone(),
            self.evidence_digest.clone(),
        )
    }
}

impl fmt::Debug for FakeNormalSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeNormalSigner")
            .field("approver_id", &self.approver_id)
            .field("authenticator_id", &self.authenticator_id)
            .field("key_id", &self.key_id)
            .field(
                "verification_key_commitment",
                &self.verification_key_commitment,
            )
            .field("evidence_digest", &self.evidence_digest)
            .field("key_material", &"[REDACTED]")
            .finish()
    }
}

/// One deterministic fake protected Guardian signer.
#[derive(Clone, Eq, PartialEq)]
pub struct FakeProtectedSigner {
    guardian_id: String,
    authenticator_id: String,
    key_id: String,
    trust_root_digest: ContentDigest,
    evidence_digest: ContentDigest,
    daemon_instance_id: String,
    observed_epoch: u64,
}

impl FakeProtectedSigner {
    /// Constructs a visibly fake protected signer from ephemeral trust-root
    /// material and exact Guardian runtime identity.
    ///
    /// # Errors
    ///
    /// Rejects malformed identifiers, epoch zero, or hashing failure.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(
        guardian_id: impl Into<String>,
        authenticator_id: impl Into<String>,
        key_id: impl Into<String>,
        secret: SecretMaterial,
        daemon_instance_id: impl Into<String>,
        observed_epoch: u64,
    ) -> Result<Self, ApprovalVerifierError> {
        let guardian_id = guardian_id.into();
        let authenticator_id = authenticator_id.into();
        let key_id = key_id.into();
        let daemon_instance_id = daemon_instance_id.into();
        validate_identifiers([
            &guardian_id,
            &authenticator_id,
            &key_id,
            &daemon_instance_id,
        ])?;
        if observed_epoch == 0 || observed_epoch > MAX_SIGNED_BIGINT {
            return Err(ApprovalVerifierError::InvalidIdentifier);
        }
        let trust_root_digest = fake_key_commitment(
            ApprovalLane::Protected,
            &guardian_id,
            &authenticator_id,
            &key_id,
            &secret,
        )?;
        let guardian = FakeGuardianBinding {
            guardian_id: guardian_id.clone(),
            daemon_instance_id: daemon_instance_id.clone(),
            observed_epoch,
            trust_root_digest: trust_root_digest.clone(),
        };
        let evidence_digest = fake_signer_evidence(
            ApprovalLane::Protected,
            &guardian_id,
            &authenticator_id,
            &key_id,
            &trust_root_digest,
            Some(&guardian),
        )?;
        Ok(Self {
            guardian_id,
            authenticator_id,
            key_id,
            trust_root_digest,
            evidence_digest,
            daemon_instance_id,
            observed_epoch,
        })
    }

    #[must_use]
    pub fn guardian_id(&self) -> &str {
        &self.guardian_id
    }

    #[must_use]
    pub fn authenticator_id(&self) -> &str {
        &self.authenticator_id
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    #[must_use]
    pub const fn trust_root_digest(&self) -> &ContentDigest {
        &self.trust_root_digest
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    #[must_use]
    pub fn daemon_instance_id(&self) -> &str {
        &self.daemon_instance_id
    }

    #[must_use]
    pub const fn observed_epoch(&self) -> u64 {
        self.observed_epoch
    }

    /// Produces deterministic fake protected proof material.
    ///
    /// # Errors
    ///
    /// Returns a canonical hashing error.
    pub fn sign(
        &self,
        challenge: &ApprovalChallenge,
    ) -> Result<FakeApprovalProof, ApprovalVerifierError> {
        validate_protected_signer_challenge(self, challenge)?;
        FakeApprovalProof::protected(
            challenge,
            &self.guardian_id,
            &self.authenticator_id,
            &self.key_id,
            self.trust_root_digest.clone(),
            self.evidence_digest.clone(),
            &self.daemon_instance_id,
            self.observed_epoch,
        )
    }
}

impl fmt::Debug for FakeProtectedSigner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeProtectedSigner")
            .field("guardian_id", &self.guardian_id)
            .field("authenticator_id", &self.authenticator_id)
            .field("key_id", &self.key_id)
            .field("daemon_instance_id", &self.daemon_instance_id)
            .field("observed_epoch", &self.observed_epoch)
            .field("trust_root_digest", &self.trust_root_digest)
            .field("evidence_digest", &self.evidence_digest)
            .field("trust_root_material", &"[REDACTED]")
            .finish()
    }
}

/// Safe Guardian identity carried by protected fake proof.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeGuardianBinding {
    guardian_id: String,
    daemon_instance_id: String,
    observed_epoch: u64,
    trust_root_digest: ContentDigest,
}

impl FakeGuardianBinding {
    #[must_use]
    pub fn guardian_id(&self) -> &str {
        &self.guardian_id
    }

    #[must_use]
    pub fn daemon_instance_id(&self) -> &str {
        &self.daemon_instance_id
    }

    #[must_use]
    pub const fn observed_epoch(&self) -> u64 {
        self.observed_epoch
    }

    #[must_use]
    pub const fn trust_root_digest(&self) -> &ContentDigest {
        &self.trust_root_digest
    }
}

/// Safe deterministic fake proof; no assertion or key bytes are retained.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeApprovalProof {
    challenge_digest: ContentDigest,
    lane: ApprovalLane,
    approver_id: String,
    authenticator_id: String,
    key_id: String,
    verification_key_commitment: ContentDigest,
    evidence_digest: ContentDigest,
    guardian: Option<FakeGuardianBinding>,
    proof_digest: ContentDigest,
}

impl FakeApprovalProof {
    fn normal(
        challenge: &ApprovalChallenge,
        approver_id: &str,
        authenticator_id: &str,
        key_id: &str,
        verification_key_commitment: ContentDigest,
        evidence_digest: ContentDigest,
    ) -> Result<Self, ApprovalVerifierError> {
        Self::build(
            challenge,
            ApprovalLane::Normal,
            approver_id,
            authenticator_id,
            key_id,
            verification_key_commitment,
            evidence_digest,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn protected(
        challenge: &ApprovalChallenge,
        guardian_id: &str,
        authenticator_id: &str,
        key_id: &str,
        trust_root_digest: ContentDigest,
        evidence_digest: ContentDigest,
        daemon_instance_id: &str,
        observed_epoch: u64,
    ) -> Result<Self, ApprovalVerifierError> {
        let guardian = FakeGuardianBinding {
            guardian_id: guardian_id.to_owned(),
            daemon_instance_id: daemon_instance_id.to_owned(),
            observed_epoch,
            trust_root_digest: trust_root_digest.clone(),
        };
        Self::build(
            challenge,
            ApprovalLane::Protected,
            guardian_id,
            authenticator_id,
            key_id,
            trust_root_digest,
            evidence_digest,
            Some(guardian),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        challenge: &ApprovalChallenge,
        lane: ApprovalLane,
        approver_id: &str,
        authenticator_id: &str,
        key_id: &str,
        verification_key_commitment: ContentDigest,
        evidence_digest: ContentDigest,
        guardian: Option<FakeGuardianBinding>,
    ) -> Result<Self, ApprovalVerifierError> {
        let mut proof = Self {
            challenge_digest: challenge.challenge_digest.clone(),
            lane,
            approver_id: approver_id.to_owned(),
            authenticator_id: authenticator_id.to_owned(),
            key_id: key_id.to_owned(),
            verification_key_commitment,
            evidence_digest,
            guardian,
            proof_digest: placeholder_digest()?,
        };
        proof.proof_digest = fake_proof_digest(&proof)?;
        Ok(proof)
    }

    #[must_use]
    pub const fn challenge_digest(&self) -> &ContentDigest {
        &self.challenge_digest
    }

    #[must_use]
    pub const fn lane(&self) -> ApprovalLane {
        self.lane
    }

    #[must_use]
    pub fn approver_id(&self) -> &str {
        &self.approver_id
    }

    #[must_use]
    pub fn authenticator_id(&self) -> &str {
        &self.authenticator_id
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    #[must_use]
    pub const fn verification_key_commitment(&self) -> &ContentDigest {
        &self.verification_key_commitment
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    #[must_use]
    pub const fn guardian(&self) -> Option<&FakeGuardianBinding> {
        self.guardian.as_ref()
    }

    #[must_use]
    pub const fn proof_digest(&self) -> &ContentDigest {
        &self.proof_digest
    }
}

/// Immutable safe challenge issued by the verifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalChallenge {
    identity: ApprovalIdentity,
    runtime: RuntimeKind,
    nonce_id: String,
    nonce_commitment: ContentDigest,
    issued_at: String,
    expires_at: String,
    subject_digest: ContentDigest,
    authenticator_id: String,
    key_id: String,
    verification_key_commitment: ContentDigest,
    evidence_digest: ContentDigest,
    review_set_digest: Option<ContentDigest>,
    challenge_digest: ContentDigest,
}

impl ApprovalChallenge {
    #[must_use]
    pub const fn identity(&self) -> &ApprovalIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }

    #[must_use]
    pub fn nonce_id(&self) -> &str {
        &self.nonce_id
    }

    #[must_use]
    pub const fn nonce_commitment(&self) -> &ContentDigest {
        &self.nonce_commitment
    }

    #[must_use]
    pub fn issued_at(&self) -> &str {
        &self.issued_at
    }

    #[must_use]
    pub fn expires_at(&self) -> &str {
        &self.expires_at
    }

    #[must_use]
    pub const fn subject_digest(&self) -> &ContentDigest {
        &self.subject_digest
    }

    #[must_use]
    pub fn authenticator_id(&self) -> &str {
        &self.authenticator_id
    }

    #[must_use]
    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    #[must_use]
    pub const fn verification_key_commitment(&self) -> &ContentDigest {
        &self.verification_key_commitment
    }

    #[must_use]
    pub const fn evidence_digest(&self) -> &ContentDigest {
        &self.evidence_digest
    }

    #[must_use]
    pub const fn review_set_digest(&self) -> Option<&ContentDigest> {
        self.review_set_digest.as_ref()
    }

    #[must_use]
    pub const fn challenge_digest(&self) -> &ContentDigest {
        &self.challenge_digest
    }
}

/// Immutable, evidence-bound revocation of one previously verified authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalRevocation {
    approval_id: String,
    revision: ApprovalRevision,
    status: ApprovalStatus,
    revoker_id: String,
    observed_at: String,
    revocation_evidence_digest: ContentDigest,
    prior_authority_receipt_digest: ContentDigest,
    revocation_digest: ContentDigest,
}

impl ApprovalRevocation {
    /// Constructs one complete revocation and derives its domain-separated
    /// commitment.
    ///
    /// # Errors
    ///
    /// Rejects malformed identifiers or timestamps, zero digests, contract
    /// failures, and canonical hashing failures.
    pub fn new(
        approval_id: impl Into<String>,
        revision: ApprovalRevision,
        revoker_id: impl Into<String>,
        observed_at: impl Into<String>,
        revocation_evidence_digest: ContentDigest,
        prior_authority_receipt_digest: ContentDigest,
    ) -> Result<Self, ApprovalVerifierError> {
        let approval_id = approval_id.into();
        let revoker_id = revoker_id.into();
        let observed_at = observed_at.into();
        validate_identifiers([&approval_id, &revoker_id])?;
        parse_canonical_utc(&observed_at)?;
        if is_zero_digest(&revocation_evidence_digest)
            || is_zero_digest(&prior_authority_receipt_digest)
        {
            return Err(ApprovalVerifierError::ZeroDigest);
        }
        let mut revocation = Self {
            approval_id,
            revision,
            status: ApprovalStatus::Revoked,
            revoker_id,
            observed_at,
            revocation_evidence_digest,
            prior_authority_receipt_digest,
            revocation_digest: placeholder_digest()?,
        };
        revocation.revocation_digest = digest(
            "lattice-approval-revocation",
            revocation_value_without_digest(&revocation),
        )?;
        Ok(revocation)
    }

    #[must_use]
    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    #[must_use]
    pub const fn revision(&self) -> ApprovalRevision {
        self.revision
    }

    #[must_use]
    pub const fn status(&self) -> ApprovalStatus {
        self.status
    }

    #[must_use]
    pub fn revoker_id(&self) -> &str {
        &self.revoker_id
    }

    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }

    #[must_use]
    pub const fn revocation_evidence_digest(&self) -> &ContentDigest {
        &self.revocation_evidence_digest
    }

    #[must_use]
    pub const fn prior_authority_receipt_digest(&self) -> &ContentDigest {
        &self.prior_authority_receipt_digest
    }

    #[must_use]
    pub const fn revocation_digest(&self) -> &ContentDigest {
        &self.revocation_digest
    }
}

/// Closed state retained for one approval identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalPhase {
    Challenged,
    VerifiedAvailable,
    VerifiedProtectedPendingClaim,
    ClaimedNormal,
    Revoked,
}

impl ApprovalPhase {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Challenged => "CHALLENGED",
            Self::VerifiedAvailable => "VERIFIED_AVAILABLE",
            Self::VerifiedProtectedPendingClaim => "VERIFIED_PROTECTED_PENDING_CLAIM",
            Self::ClaimedNormal => "CLAIMED_NORMAL",
            Self::Revoked => "REVOKED",
        }
    }
}

/// Complete optimistic-concurrency head for verifier-owned state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalStateHead {
    approval_id: String,
    revision: u64,
    phase: ApprovalPhase,
    state_digest: ContentDigest,
}

impl ApprovalStateHead {
    #[must_use]
    pub fn approval_id(&self) -> &str {
        &self.approval_id
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn phase(&self) -> ApprovalPhase {
        self.phase
    }

    #[must_use]
    pub const fn state_digest(&self) -> &ContentDigest {
        &self.state_digest
    }
}

/// Issue one exact challenge and globally bind one safe nonce commitment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssueApprovalCommand {
    pub command_id: String,
    pub expected_head: Option<ApprovalStateHead>,
    pub runtime: RuntimeKind,
    pub identity: ApprovalIdentity,
    pub nonce_id: String,
    pub nonce_commitment: ContentDigest,
    pub issued_at: String,
    pub expires_at: String,
    pub authenticator_id: String,
    pub key_id: String,
    pub verification_key_commitment: ContentDigest,
    pub evidence_digest: ContentDigest,
    pub review_set_digest: Option<ContentDigest>,
}

/// Verify one exact fake proof against an issued challenge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifyApprovalCommand {
    pub command_id: String,
    pub approval_id: String,
    pub expected_head: ApprovalStateHead,
    pub observed_at: String,
    pub proof: FakeApprovalProof,
}

/// Append-only owner command binding one verified available normal approval
/// to an exact task, successor stream, Task Spec, and budget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindExecutionApprovalCommand {
    pub command_id: String,
    pub approval_id: String,
    pub expected_head: ApprovalStateHead,
    pub observed_at: String,
    pub execution_challenge: ExecutionApprovalChallenge,
    pub execution_proof: FakeExecutionApprovalProof,
}

/// Claim one exact available normal approval.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsumeNormalApprovalCommand {
    pub command_id: String,
    pub approval_id: String,
    pub expected_head: ApprovalStateHead,
    pub observed_at: String,
    pub claim_digest: ContentDigest,
}

/// Revoke one exact, still-available verified approval authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevokeApprovalCommand {
    pub command_id: String,
    pub approval_id: String,
    pub expected_head: ApprovalStateHead,
    pub observed_at: String,
    pub revoker_id: String,
    pub revocation_evidence_digest: ContentDigest,
}

/// Closed public command set. There is intentionally no protected consume
/// command.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalCommand {
    Issue(IssueApprovalCommand),
    Verify(VerifyApprovalCommand),
    BindExecution(BindExecutionApprovalCommand),
    ConsumeNormal(ConsumeNormalApprovalCommand),
    Revoke(RevokeApprovalCommand),
}

impl ApprovalCommand {
    fn command_id(&self) -> &str {
        match self {
            Self::Issue(command) => &command.command_id,
            Self::Verify(command) => &command.command_id,
            Self::BindExecution(command) => &command.command_id,
            Self::ConsumeNormal(command) => &command.command_id,
            Self::Revoke(command) => &command.command_id,
        }
    }

    fn approval_id(&self) -> &str {
        match self {
            Self::Issue(command) => command.identity.approval_id(),
            Self::Verify(command) => &command.approval_id,
            Self::BindExecution(command) => &command.approval_id,
            Self::ConsumeNormal(command) => &command.approval_id,
            Self::Revoke(command) => &command.approval_id,
        }
    }

    fn expected_head(&self) -> Option<&ApprovalStateHead> {
        match self {
            Self::Issue(command) => command.expected_head.as_ref(),
            Self::Verify(command) => Some(&command.expected_head),
            Self::BindExecution(command) => Some(&command.expected_head),
            Self::ConsumeNormal(command) => Some(&command.expected_head),
            Self::Revoke(command) => Some(&command.expected_head),
        }
    }
}

/// Applied or denied terminal command result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalCommandOutcome {
    Applied,
    Denied(ApprovalDenial),
}

/// Immutable terminal command receipt. Applied and denied results share one
/// predecessor chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalCommandReceipt {
    pub ordinal: u64,
    pub previous_receipt_digest: Option<ContentDigest>,
    pub request: ApprovalCommand,
    pub request_digest: ContentDigest,
    pub before: Option<ApprovalStateHead>,
    pub after: Option<ApprovalStateHead>,
    pub outcome: ApprovalCommandOutcome,
    pub challenge: Option<ApprovalChallenge>,
    pub authority_receipt: Option<ApprovalAuthorityReceipt>,
    pub execution_binding_receipt: Option<ExecutionApprovalBindingReceipt>,
    pub revocation: Option<ApprovalRevocation>,
    pub receipt_digest: ContentDigest,
}

/// Complete raw persistence payload. No nested field is trusted until replay.
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedApprovalSnapshot {
    /// Raw canonical-value payload supplied by a future persistence adapter.
    pub payload: CanonicalValue,
}

impl fmt::Debug for UntrustedApprovalSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedApprovalSnapshot")
            .field("raw_fields", &"[ELIDED]")
            .finish_non_exhaustive()
    }
}

impl UntrustedApprovalSnapshot {
    /// Encodes the raw snapshot as exact canonical UTF-8 JSON bytes for a
    /// persistence adapter. This does not make an untrusted snapshot verified.
    ///
    /// # Errors
    ///
    /// Rejects duplicate normalized keys or snapshots above the closed byte
    /// limit.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ApprovalVerifierError> {
        let bytes = canonicalize(&self.payload)
            .map_err(|_| ApprovalVerifierError::Canonical)?
            .into_vec();
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(ApprovalVerifierError::CorruptSnapshot);
        }
        Ok(bytes)
    }

    /// Parses exact canonical UTF-8 JSON bytes, strictly replays the Approval
    /// owner history, and returns its normalized raw snapshot projection.
    ///
    /// # Errors
    ///
    /// Rejects oversized, malformed, numeric, duplicate-key, non-canonical,
    /// or trailing input.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, ApprovalVerifierError> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(ApprovalVerifierError::CorruptSnapshot);
        }
        let json: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
        let payload = canonical_value_from_json(json)?;
        let canonical =
            canonicalize(&payload).map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
        if canonical.as_slice() != bytes {
            return Err(ApprovalVerifierError::CorruptSnapshot);
        }
        let parsed = Self { payload };
        verify_snapshot_internal(&parsed, false).map(|verified| verified.export_untrusted())
    }
}

/// Independently retained commitment to one complete verified global
/// aggregate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalVerifierCheckpoint {
    command_high_water: u64,
    command_tail_digest: Option<ContentDigest>,
    nonce_bindings_digest: ContentDigest,
    snapshot_digest: ContentDigest,
}

impl ApprovalVerifierCheckpoint {
    /// Constructs a trusted checkpoint from independently persisted columns.
    ///
    /// # Errors
    ///
    /// Rejects out-of-range high-water values, tail disagreement, or zero
    /// commitments.
    pub fn new(
        command_high_water: u64,
        command_tail_digest: Option<ContentDigest>,
        nonce_bindings_digest: ContentDigest,
        snapshot_digest: ContentDigest,
    ) -> Result<Self, ApprovalVerifierError> {
        if command_high_water > MAX_SIGNED_BIGINT
            || (command_high_water == 0) != command_tail_digest.is_none()
            || command_tail_digest.as_ref().is_some_and(is_zero_digest)
            || is_zero_digest(&nonce_bindings_digest)
            || is_zero_digest(&snapshot_digest)
        {
            return Err(ApprovalVerifierError::CheckpointMismatch);
        }
        Ok(Self {
            command_high_water,
            command_tail_digest,
            nonce_bindings_digest,
            snapshot_digest,
        })
    }

    #[must_use]
    pub const fn command_high_water(&self) -> u64 {
        self.command_high_water
    }

    #[must_use]
    pub const fn command_tail_digest(&self) -> Option<&ContentDigest> {
        self.command_tail_digest.as_ref()
    }

    #[must_use]
    pub const fn nonce_bindings_digest(&self) -> &ContentDigest {
        &self.nonce_bindings_digest
    }

    #[must_use]
    pub const fn snapshot_digest(&self) -> &ContentDigest {
        &self.snapshot_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct NonceBinding {
    nonce_id: String,
    approval_id: String,
    challenge_id: String,
    subject_digest: ContentDigest,
    lane: ApprovalLane,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ApprovalRecord {
    challenge: ApprovalChallenge,
    phase: ApprovalPhase,
    revision: u64,
    authority_receipt: Option<ApprovalAuthorityReceipt>,
    execution_binding_receipt: Option<ExecutionApprovalBindingReceipt>,
    claim_digest: Option<ContentDigest>,
    revocation: Option<ApprovalRevocation>,
}

/// A fully verified global in-memory approval aggregate.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VerifiedApprovalAggregate {
    approvals: BTreeMap<String, ApprovalRecord>,
    nonce_bindings: BTreeMap<String, NonceBinding>,
    nonce_ids: BTreeMap<String, String>,
    command_receipts: Vec<ApprovalCommandReceipt>,
}

impl VerifiedApprovalAggregate {
    /// Constructs an empty global verifier aggregate.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns one current state head, including non-authorizing challenged or
    /// claimed state.
    #[must_use]
    pub fn state_head(&self, approval_id: &str) -> Option<ApprovalStateHead> {
        self.approvals
            .get(approval_id)
            .and_then(|record| state_head(record).ok())
    }

    /// Returns one issued challenge.
    #[must_use]
    pub fn challenge(&self, approval_id: &str) -> Option<&ApprovalChallenge> {
        self.approvals
            .get(approval_id)
            .map(|record| &record.challenge)
    }

    /// Returns one retained immutable revocation.
    #[must_use]
    pub fn revocation(&self, approval_id: &str) -> Option<&ApprovalRevocation> {
        self.approvals
            .get(approval_id)
            .and_then(|record| record.revocation.as_ref())
    }

    /// Returns the immutable execution-specific binding receipt retained by
    /// the Approval owner, if the verified approval was bound.
    #[must_use]
    pub fn execution_binding_receipt(
        &self,
        approval_id: &str,
    ) -> Option<&ExecutionApprovalBindingReceipt> {
        self.approvals
            .get(approval_id)
            .and_then(|record| record.execution_binding_receipt.as_ref())
    }

    /// Returns immutable terminal command history.
    #[must_use]
    pub fn command_receipts(&self) -> &[ApprovalCommandReceipt] {
        &self.command_receipts
    }

    /// Exports the complete raw persistence projection.
    #[must_use]
    pub fn export_untrusted(&self) -> UntrustedApprovalSnapshot {
        UntrustedApprovalSnapshot {
            payload: snapshot_value(self),
        }
    }

    /// Produces one independently retainable rollback checkpoint.
    ///
    /// # Errors
    ///
    /// Returns a canonical hashing failure if the complete aggregate cannot
    /// be committed.
    pub fn checkpoint(&self) -> Result<ApprovalVerifierCheckpoint, ApprovalVerifierError> {
        ApprovalVerifierCheckpoint::new(
            u64::try_from(self.command_receipts.len())
                .map_err(|_| ApprovalVerifierError::CorruptSnapshot)?,
            self.command_receipts
                .last()
                .map(|receipt| receipt.receipt_digest.clone()),
            nonce_bindings_digest(self)?,
            snapshot_digest(self)?,
        )
    }
}

/// One pure command plan guarded by a complete aggregate commitment.
#[derive(Clone, Debug)]
pub struct ApprovalVerifierPlan {
    base_snapshot_digest: ContentDigest,
    next: VerifiedApprovalAggregate,
    receipt: ApprovalCommandReceipt,
    exact_retry: bool,
}

impl ApprovalVerifierPlan {
    #[must_use]
    pub const fn receipt(&self) -> &ApprovalCommandReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.exact_retry
    }
}

/// Plans one terminal command without I/O or mutation.
///
/// Exact retry lookup happens before stale-head and time evaluation.
///
/// # Errors
///
/// Rejects malformed inputs, changed command-ID reuse, live fake commands,
/// contract failures, and canonical hashing failures.
pub fn plan_command(
    current: &VerifiedApprovalAggregate,
    command: &ApprovalCommand,
) -> Result<ApprovalVerifierPlan, ApprovalVerifierError> {
    validate_identifier(command.command_id())?;
    let request_digest = digest("lattice-approval-command-request", command_value(command))?;
    let base_snapshot_digest = aggregate_digest(current)?;
    if let Some(existing) = current
        .command_receipts
        .iter()
        .find(|receipt| receipt.request.command_id() == command.command_id())
    {
        if existing.request_digest != request_digest || existing.request != *command {
            return Err(ApprovalVerifierError::CommandIdReuse);
        }
        return Ok(ApprovalVerifierPlan {
            base_snapshot_digest,
            next: current.clone(),
            receipt: existing.clone(),
            exact_retry: true,
        });
    }
    validate_command(command)?;
    let ordinal = u64::try_from(current.command_receipts.len())
        .ok()
        .and_then(|value| value.checked_add(1))
        .filter(|value| *value <= MAX_SIGNED_BIGINT)
        .ok_or(ApprovalVerifierError::CorruptSnapshot)?;
    let before = current.state_head(command.approval_id());
    let mut next = current.clone();
    let transition = transition_for(&mut next, command, ordinal)?;
    let after = next.state_head(command.approval_id());
    let previous_receipt_digest = current
        .command_receipts
        .last()
        .map(|receipt| receipt.receipt_digest.clone());
    let receipt_digest = terminal_receipt_digest(
        ordinal,
        previous_receipt_digest.as_ref(),
        command,
        &request_digest,
        before.as_ref(),
        after.as_ref(),
        transition.outcome,
        transition.challenge.as_ref(),
        transition.authority_receipt.as_ref(),
        transition.execution_binding_receipt.as_ref(),
        transition.revocation.as_ref(),
    )?;
    let receipt = ApprovalCommandReceipt {
        ordinal,
        previous_receipt_digest,
        request: command.clone(),
        request_digest,
        before,
        after,
        outcome: transition.outcome,
        challenge: transition.challenge,
        authority_receipt: transition.authority_receipt,
        execution_binding_receipt: transition.execution_binding_receipt,
        revocation: transition.revocation,
        receipt_digest,
    };
    next.command_receipts.push(receipt.clone());
    Ok(ApprovalVerifierPlan {
        base_snapshot_digest,
        next,
        receipt,
        exact_retry: false,
    })
}

/// Applies a pure plan only to the exact aggregate used during planning.
///
/// # Errors
///
/// Rejects changed aggregate state.
pub fn apply_plan(
    current: &VerifiedApprovalAggregate,
    plan: ApprovalVerifierPlan,
) -> Result<VerifiedApprovalAggregate, ApprovalVerifierError> {
    if aggregate_digest(current)? != plan.base_snapshot_digest {
        return Err(ApprovalVerifierError::PlanPreconditionChanged);
    }
    Ok(plan.next)
}

/// Strictly decodes and replays every command from an untrusted snapshot.
///
/// # Errors
///
/// Rejects unknown fields/versions/kinds, malformed values, tamper, reorder,
/// duplication, orphan state, nonce rebinding, receipt-chain disagreement,
/// and derived-state disagreement.
pub fn verify_snapshot(
    snapshot: &UntrustedApprovalSnapshot,
) -> Result<VerifiedApprovalAggregate, ApprovalVerifierError> {
    verify_snapshot_internal(snapshot, true)
}

fn verify_snapshot_internal(
    snapshot: &UntrustedApprovalSnapshot,
    require_owner_projection_order: bool,
) -> Result<VerifiedApprovalAggregate, ApprovalVerifierError> {
    let decoded = decode_snapshot(&snapshot.payload)?;
    if decoded.command_high_water > MAX_SIGNED_BIGINT {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    if decoded.command_high_water
        != u64::try_from(decoded.commands.len())
            .map_err(|_| ApprovalVerifierError::CorruptSnapshot)?
    {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    let decoded_tail = decoded
        .commands
        .last()
        .map(|command| command.receipt_digest.clone());
    if decoded.command_tail_digest != decoded_tail {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }

    let mut replayed = VerifiedApprovalAggregate::empty();
    for (index, raw) in decoded.commands.iter().enumerate() {
        let expected_ordinal =
            u64::try_from(index + 1).map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
        let expected_previous = replayed
            .command_receipts
            .last()
            .map(|receipt| receipt.receipt_digest.clone());
        if raw.ordinal != expected_ordinal || raw.previous_receipt_digest != expected_previous {
            return Err(ApprovalVerifierError::CorruptSnapshot);
        }
        let plan = plan_command(&replayed, &raw.request)
            .map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
        if plan.is_exact_retry()
            || plan.receipt.receipt_digest != raw.receipt_digest
            || !snapshot_values_match(
                &terminal_receipt_value(plan.receipt()),
                &raw.raw,
                require_owner_projection_order,
            )?
        {
            return Err(ApprovalVerifierError::CorruptSnapshot);
        }
        replayed =
            apply_plan(&replayed, plan).map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
    }
    if nonce_bindings_digest(&replayed)? != decoded.nonce_bindings_digest
        || !snapshot_values_match(
            &replayed.export_untrusted().payload,
            &snapshot.payload,
            require_owner_projection_order,
        )?
    {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    Ok(replayed)
}

fn snapshot_values_match(
    expected: &CanonicalValue,
    actual: &CanonicalValue,
    require_owner_projection_order: bool,
) -> Result<bool, ApprovalVerifierError> {
    if require_owner_projection_order {
        return Ok(expected == actual);
    }
    let expected = canonicalize(expected).map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
    let actual = canonicalize(actual).map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
    Ok(expected == actual)
}

/// Verifies a raw snapshot against an independently retained current
/// checkpoint.
///
/// # Errors
///
/// Returns checkpoint mismatch for an internally coherent older prefix or
/// substituted aggregate.
pub fn verify_snapshot_against_checkpoint(
    snapshot: &UntrustedApprovalSnapshot,
    expected: &ApprovalVerifierCheckpoint,
) -> Result<VerifiedApprovalAggregate, ApprovalVerifierError> {
    let verified = verify_snapshot(snapshot)?;
    if verified.checkpoint()? != *expected {
        return Err(ApprovalVerifierError::CheckpointMismatch);
    }
    Ok(verified)
}

/// Deterministic non-durable fake owner.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FakeApprovalVerifier {
    aggregate: VerifiedApprovalAggregate,
}

impl FakeApprovalVerifier {
    /// Creates an empty visibly fake verifier.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Executes any public fake command.
    ///
    /// # Errors
    ///
    /// Returns planner or apply errors without partial mutation.
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute(
        &mut self,
        command: ApprovalCommand,
    ) -> Result<ApprovalCommandReceipt, ApprovalVerifierError> {
        let plan = plan_command(&self.aggregate, &command)?;
        let receipt = plan.receipt.clone();
        self.aggregate = apply_plan(&self.aggregate, plan)?;
        Ok(receipt)
    }

    /// Issues one challenge.
    ///
    /// # Errors
    ///
    /// Returns planner or apply errors without partial mutation.
    pub fn issue(
        &mut self,
        command: IssueApprovalCommand,
    ) -> Result<ApprovalCommandReceipt, ApprovalVerifierError> {
        self.execute(ApprovalCommand::Issue(command))
    }

    /// Verifies one fake proof.
    ///
    /// # Errors
    ///
    /// Returns planner or apply errors without partial mutation.
    pub fn verify(
        &mut self,
        command: VerifyApprovalCommand,
    ) -> Result<ApprovalCommandReceipt, ApprovalVerifierError> {
        self.execute(ApprovalCommand::Verify(command))
    }

    /// Binds one exact verified available execution approval append-only.
    ///
    /// # Errors
    ///
    /// Returns planner or apply errors without partial mutation.
    pub fn bind_execution(
        &mut self,
        command: BindExecutionApprovalCommand,
    ) -> Result<ApprovalCommandReceipt, ApprovalVerifierError> {
        self.execute(ApprovalCommand::BindExecution(command))
    }

    /// Claims one available normal approval.
    ///
    /// # Errors
    ///
    /// Returns planner or apply errors without partial mutation.
    pub fn consume_normal(
        &mut self,
        command: ConsumeNormalApprovalCommand,
    ) -> Result<ApprovalCommandReceipt, ApprovalVerifierError> {
        self.execute(ApprovalCommand::ConsumeNormal(command))
    }

    /// Revokes one exact available normal or protected authority.
    ///
    /// # Errors
    ///
    /// Returns planner or apply errors without partial mutation.
    pub fn revoke(
        &mut self,
        command: RevokeApprovalCommand,
    ) -> Result<ApprovalCommandReceipt, ApprovalVerifierError> {
        self.execute(ApprovalCommand::Revoke(command))
    }

    /// Returns one verifier-owned structural state head.
    #[must_use]
    pub fn state_head(&self, approval_id: &str) -> Option<ApprovalStateHead> {
        self.aggregate.state_head(approval_id)
    }

    /// Returns one retained issued challenge.
    #[must_use]
    pub fn challenge(&self, approval_id: &str) -> Option<&ApprovalChallenge> {
        self.aggregate.challenge(approval_id)
    }

    /// Returns the retained immutable revocation, if any.
    #[must_use]
    pub fn revocation(&self, approval_id: &str) -> Option<&ApprovalRevocation> {
        self.aggregate.revocation(approval_id)
    }

    /// Returns one retained execution-specific owner binding receipt.
    #[must_use]
    pub fn execution_binding_receipt(
        &self,
        approval_id: &str,
    ) -> Option<&ExecutionApprovalBindingReceipt> {
        self.aggregate.execution_binding_receipt(approval_id)
    }

    /// Returns an independently queried complete authority head only while the
    /// approval is verified, available, and time-current.
    ///
    /// # Errors
    ///
    /// Rejects a malformed observation timestamp.
    pub fn current_head_at(
        &self,
        approval_id: &str,
        observed_at: &str,
    ) -> Result<Option<ApprovalAuthorityHead>, ApprovalVerifierError> {
        let observed = parse_canonical_utc(observed_at)?;
        let Some(record) = self.aggregate.approvals.get(approval_id) else {
            return Ok(None);
        };
        if !matches!(
            record.phase,
            ApprovalPhase::VerifiedAvailable | ApprovalPhase::VerifiedProtectedPendingClaim
        ) {
            return Ok(None);
        }
        let issued = parse_canonical_utc(&record.challenge.issued_at)?;
        let expires = parse_canonical_utc(&record.challenge.expires_at)?;
        if observed < issued || observed >= expires {
            return Ok(None);
        }
        Ok(record
            .authority_receipt
            .as_ref()
            .map(ApprovalAuthorityReceipt::head))
    }

    /// Returns immutable terminal command history.
    #[must_use]
    pub fn command_receipts(&self) -> &[ApprovalCommandReceipt] {
        self.aggregate.command_receipts()
    }

    /// Exports one complete untrusted raw snapshot.
    #[must_use]
    pub fn export_snapshot(&self) -> UntrustedApprovalSnapshot {
        self.aggregate.export_untrusted()
    }

    /// Exports the complete owner snapshot as exact canonical persistence
    /// bytes.
    ///
    /// # Errors
    ///
    /// Returns canonicalization or closed size-limit failures.
    pub fn export_snapshot_bytes(&self) -> Result<Vec<u8>, ApprovalVerifierError> {
        self.export_snapshot().canonical_bytes()
    }

    /// Returns the independently retained checkpoint for current fake state.
    ///
    /// # Errors
    ///
    /// Returns canonical hashing failures.
    pub fn current_checkpoint(&self) -> Result<ApprovalVerifierCheckpoint, ApprovalVerifierError> {
        self.aggregate.checkpoint()
    }

    /// Restores one complete replay-verified fake snapshot against a trusted
    /// checkpoint.
    ///
    /// # Errors
    ///
    /// Rejects corruption, rollback, substitution, or overwrite.
    pub fn restore_snapshot(
        &mut self,
        snapshot: &UntrustedApprovalSnapshot,
        expected: &ApprovalVerifierCheckpoint,
    ) -> Result<(), ApprovalVerifierError> {
        if !self.aggregate.approvals.is_empty()
            || !self.aggregate.nonce_bindings.is_empty()
            || !self.aggregate.command_receipts.is_empty()
        {
            return Err(ApprovalVerifierError::RestoreWouldOverwrite);
        }
        self.aggregate = verify_snapshot_against_checkpoint(snapshot, expected)?;
        Ok(())
    }

    /// Strictly parses and restores canonical snapshot bytes against one
    /// independently retained checkpoint.
    ///
    /// # Errors
    ///
    /// Rejects malformed/non-canonical bytes, corruption, rollback,
    /// substitution, or overwrite.
    pub fn restore_snapshot_bytes(
        &mut self,
        bytes: &[u8],
        expected: &ApprovalVerifierCheckpoint,
    ) -> Result<(), ApprovalVerifierError> {
        let snapshot = UntrustedApprovalSnapshot::from_canonical_bytes(bytes)?;
        self.restore_snapshot(&snapshot, expected)
    }
}

fn canonical_value_from_json(
    value: serde_json::Value,
) -> Result<CanonicalValue, ApprovalVerifierError> {
    match value {
        serde_json::Value::Null => Ok(CanonicalValue::Null),
        serde_json::Value::Bool(value) => Ok(CanonicalValue::Bool(value)),
        serde_json::Value::String(value) => Ok(CanonicalValue::String(value)),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(canonical_value_from_json)
            .collect::<Result<Vec<_>, _>>()
            .map(CanonicalValue::Array),
        serde_json::Value::Object(entries) => entries
            .into_iter()
            .map(|(key, value)| canonical_value_from_json(value).map(|value| (key, value)))
            .collect::<Result<Vec<_>, _>>()
            .map(CanonicalValue::Object),
        serde_json::Value::Number(_) => Err(ApprovalVerifierError::CorruptSnapshot),
    }
}

struct TransitionResult {
    outcome: ApprovalCommandOutcome,
    challenge: Option<ApprovalChallenge>,
    authority_receipt: Option<ApprovalAuthorityReceipt>,
    execution_binding_receipt: Option<ExecutionApprovalBindingReceipt>,
    revocation: Option<ApprovalRevocation>,
}

impl TransitionResult {
    const fn denied(denial: ApprovalDenial) -> Self {
        Self {
            outcome: ApprovalCommandOutcome::Denied(denial),
            challenge: None,
            authority_receipt: None,
            execution_binding_receipt: None,
            revocation: None,
        }
    }

    const fn applied() -> Self {
        Self {
            outcome: ApprovalCommandOutcome::Applied,
            challenge: None,
            authority_receipt: None,
            execution_binding_receipt: None,
            revocation: None,
        }
    }
}

fn transition_for(
    next: &mut VerifiedApprovalAggregate,
    command: &ApprovalCommand,
    ordinal: u64,
) -> Result<TransitionResult, ApprovalVerifierError> {
    let current_head = next.state_head(command.approval_id());
    if current_head.as_ref() != command.expected_head() {
        return Ok(TransitionResult::denied(ApprovalDenial::StaleHead));
    }
    match command {
        ApprovalCommand::Issue(command) => issue_transition(next, command),
        ApprovalCommand::Verify(command) => verify_transition(next, command, ordinal),
        ApprovalCommand::BindExecution(command) => bind_execution_transition(next, command),
        ApprovalCommand::ConsumeNormal(command) => consume_transition(next, command),
        ApprovalCommand::Revoke(command) => revoke_transition(next, command),
    }
}

fn issue_transition(
    next: &mut VerifiedApprovalAggregate,
    command: &IssueApprovalCommand,
) -> Result<TransitionResult, ApprovalVerifierError> {
    if next.approvals.contains_key(command.identity.approval_id()) {
        return Ok(TransitionResult::denied(
            ApprovalDenial::ApprovalAlreadyExists,
        ));
    }
    if next
        .nonce_bindings
        .contains_key(command.nonce_commitment.as_str())
        || next.nonce_ids.contains_key(&command.nonce_id)
    {
        return Ok(TransitionResult::denied(ApprovalDenial::NonceAlreadyBound));
    }
    let subject_digest =
        approval_subject_digest(command.identity.binding(), command.identity.subject())?;
    let challenge = make_challenge(command, subject_digest.clone())?;
    let binding = NonceBinding {
        nonce_id: command.nonce_id.clone(),
        approval_id: command.identity.approval_id().to_owned(),
        challenge_id: command.identity.challenge_id().to_owned(),
        subject_digest,
        lane: command.identity.lane(),
    };
    next.nonce_ids.insert(
        command.nonce_id.clone(),
        command.nonce_commitment.as_str().to_owned(),
    );
    next.nonce_bindings
        .insert(command.nonce_commitment.as_str().to_owned(), binding);
    next.approvals.insert(
        command.identity.approval_id().to_owned(),
        ApprovalRecord {
            challenge: challenge.clone(),
            phase: ApprovalPhase::Challenged,
            revision: 1,
            authority_receipt: None,
            execution_binding_receipt: None,
            claim_digest: None,
            revocation: None,
        },
    );
    Ok(TransitionResult {
        challenge: Some(challenge),
        ..TransitionResult::applied()
    })
}

fn bind_execution_transition(
    next: &mut VerifiedApprovalAggregate,
    command: &BindExecutionApprovalCommand,
) -> Result<TransitionResult, ApprovalVerifierError> {
    let Some(record) = next.approvals.get_mut(&command.approval_id) else {
        return Ok(TransitionResult::denied(ApprovalDenial::ApprovalMissing));
    };
    if record.phase != ApprovalPhase::VerifiedAvailable
        || record.execution_binding_receipt.is_some()
    {
        return Ok(TransitionResult::denied(ApprovalDenial::InvalidState));
    }
    if let Some(denial) = observe_window(
        &record.challenge.issued_at,
        &record.challenge.expires_at,
        &command.observed_at,
    )? {
        return Ok(TransitionResult::denied(denial));
    }
    let authority_receipt = record
        .authority_receipt
        .as_ref()
        .ok_or(ApprovalVerifierError::Contract)?;
    let current_head = authority_receipt.head();
    let Ok(execution_binding_receipt) = issue_execution_approval_binding_receipt(
        &command.execution_challenge,
        &command.execution_proof,
        authority_receipt,
        &current_head,
    ) else {
        return Ok(TransitionResult::denied(ApprovalDenial::ProofMismatch));
    };
    let Some(revision) = record.revision.checked_add(1) else {
        return Ok(TransitionResult::denied(ApprovalDenial::CounterExhausted));
    };
    if revision > MAX_SIGNED_BIGINT {
        return Ok(TransitionResult::denied(ApprovalDenial::CounterExhausted));
    }
    record.revision = revision;
    record.execution_binding_receipt = Some(execution_binding_receipt.clone());
    Ok(TransitionResult {
        execution_binding_receipt: Some(execution_binding_receipt),
        ..TransitionResult::applied()
    })
}

fn verify_transition(
    next: &mut VerifiedApprovalAggregate,
    command: &VerifyApprovalCommand,
    ordinal: u64,
) -> Result<TransitionResult, ApprovalVerifierError> {
    let Some(record) = next.approvals.get_mut(&command.approval_id) else {
        return Ok(TransitionResult::denied(ApprovalDenial::ApprovalMissing));
    };
    if record.phase != ApprovalPhase::Challenged {
        return Ok(TransitionResult::denied(ApprovalDenial::InvalidState));
    }
    if let Some(denial) = observe_window(
        &record.challenge.issued_at,
        &record.challenge.expires_at,
        &command.observed_at,
    )? {
        return Ok(TransitionResult::denied(denial));
    }
    if !proof_matches(&record.challenge, &command.proof)? {
        return Ok(TransitionResult::denied(ApprovalDenial::ProofMismatch));
    }
    let Some(revision_value) = record.revision.checked_add(1) else {
        return Ok(TransitionResult::denied(ApprovalDenial::CounterExhausted));
    };
    if revision_value > MAX_SIGNED_BIGINT {
        return Ok(TransitionResult::denied(ApprovalDenial::CounterExhausted));
    }
    let status = match record.challenge.identity.lane() {
        ApprovalLane::Normal => ApprovalStatus::Available,
        ApprovalLane::Protected => ApprovalStatus::ProtectedPendingClaim,
    };
    let revision =
        ApprovalRevision::new(revision_value).map_err(|_| ApprovalVerifierError::Contract)?;
    let receipt_digest = authority_receipt_digest(
        ordinal,
        &record.challenge,
        revision,
        status,
        command.proof.proof_digest(),
    )?;
    let authority_receipt = ApprovalAuthorityReceipt::new(
        CONTRACT_VERSION,
        APPROVAL_VERIFIER_PRODUCER_ID,
        APPROVAL_VERIFIER_PRODUCER_VERSION,
        RuntimeKind::Fake,
        record.challenge.identity.clone(),
        revision,
        status,
        record.challenge.nonce_id.clone(),
        record.challenge.nonce_commitment.clone(),
        record.challenge.issued_at.clone(),
        record.challenge.expires_at.clone(),
        record.challenge.subject_digest.clone(),
        record.challenge.challenge_digest.clone(),
        record.challenge.authenticator_id.clone(),
        record.challenge.key_id.clone(),
        command.proof.proof_digest().clone(),
        record.challenge.evidence_digest.clone(),
        record.challenge.review_set_digest.clone(),
        receipt_digest,
    )
    .map_err(|_| ApprovalVerifierError::Contract)?;
    record.revision = revision_value;
    record.phase = match status {
        ApprovalStatus::Available => ApprovalPhase::VerifiedAvailable,
        ApprovalStatus::ProtectedPendingClaim => ApprovalPhase::VerifiedProtectedPendingClaim,
        ApprovalStatus::ClaimedNormal | ApprovalStatus::Revoked => {
            return Err(ApprovalVerifierError::Contract);
        }
    };
    record.authority_receipt = Some(authority_receipt.clone());
    Ok(TransitionResult {
        authority_receipt: Some(authority_receipt),
        ..TransitionResult::applied()
    })
}

fn consume_transition(
    next: &mut VerifiedApprovalAggregate,
    command: &ConsumeNormalApprovalCommand,
) -> Result<TransitionResult, ApprovalVerifierError> {
    let Some(record) = next.approvals.get_mut(&command.approval_id) else {
        return Ok(TransitionResult::denied(ApprovalDenial::ApprovalMissing));
    };
    if record.challenge.identity.lane() != ApprovalLane::Normal {
        return Ok(TransitionResult::denied(
            ApprovalDenial::NormalClaimRequired,
        ));
    }
    if record.phase != ApprovalPhase::VerifiedAvailable {
        return Ok(TransitionResult::denied(ApprovalDenial::InvalidState));
    }
    if let Some(denial) = observe_window(
        &record.challenge.issued_at,
        &record.challenge.expires_at,
        &command.observed_at,
    )? {
        return Ok(TransitionResult::denied(denial));
    }
    let Some(revision) = record.revision.checked_add(1) else {
        return Ok(TransitionResult::denied(ApprovalDenial::CounterExhausted));
    };
    if revision > MAX_SIGNED_BIGINT {
        return Ok(TransitionResult::denied(ApprovalDenial::CounterExhausted));
    }
    record.revision = revision;
    record.phase = ApprovalPhase::ClaimedNormal;
    record.claim_digest = Some(command.claim_digest.clone());
    Ok(TransitionResult::applied())
}

fn revoke_transition(
    next: &mut VerifiedApprovalAggregate,
    command: &RevokeApprovalCommand,
) -> Result<TransitionResult, ApprovalVerifierError> {
    let Some(record) = next.approvals.get_mut(&command.approval_id) else {
        return Ok(TransitionResult::denied(ApprovalDenial::ApprovalMissing));
    };
    if !matches!(
        record.phase,
        ApprovalPhase::VerifiedAvailable | ApprovalPhase::VerifiedProtectedPendingClaim
    ) {
        return Ok(TransitionResult::denied(ApprovalDenial::InvalidState));
    }
    if command.revoker_id != record.challenge.identity.approver_id() {
        return Ok(TransitionResult::denied(ApprovalDenial::RevokerMismatch));
    }
    if let Some(denial) = observe_window(
        &record.challenge.issued_at,
        &record.challenge.expires_at,
        &command.observed_at,
    )? {
        return Ok(TransitionResult::denied(denial));
    }
    let Some(revision_value) = record.revision.checked_add(1) else {
        return Ok(TransitionResult::denied(ApprovalDenial::CounterExhausted));
    };
    if revision_value > MAX_SIGNED_BIGINT {
        return Ok(TransitionResult::denied(ApprovalDenial::CounterExhausted));
    }
    let revision =
        ApprovalRevision::new(revision_value).map_err(|_| ApprovalVerifierError::Contract)?;
    let prior_authority_receipt_digest = record
        .authority_receipt
        .as_ref()
        .ok_or(ApprovalVerifierError::Contract)?
        .receipt_digest()
        .clone();
    let revocation = ApprovalRevocation::new(
        &command.approval_id,
        revision,
        &command.revoker_id,
        &command.observed_at,
        command.revocation_evidence_digest.clone(),
        prior_authority_receipt_digest,
    )?;
    record.revision = revision_value;
    record.phase = ApprovalPhase::Revoked;
    record.revocation = Some(revocation.clone());
    Ok(TransitionResult {
        revocation: Some(revocation),
        ..TransitionResult::applied()
    })
}

fn validate_command(command: &ApprovalCommand) -> Result<(), ApprovalVerifierError> {
    validate_identifier(command.approval_id())?;
    match command {
        ApprovalCommand::Issue(command) => {
            if command.runtime != RuntimeKind::Fake {
                return Err(ApprovalVerifierError::FakeRuntimeRequired);
            }
            validate_identifiers([
                command.identity.approval_id(),
                command.identity.challenge_id(),
                command.identity.requester_id(),
                command.identity.approver_id(),
                command.identity.channel_id(),
                command.identity.session_id(),
                &command.nonce_id,
                &command.authenticator_id,
                &command.key_id,
            ])?;
            for value in [
                &command.nonce_commitment,
                &command.verification_key_commitment,
                &command.evidence_digest,
            ]
            .into_iter()
            .chain(command.review_set_digest.iter())
            {
                if is_zero_digest(value) {
                    return Err(ApprovalVerifierError::ZeroDigest);
                }
            }
            let issued = parse_canonical_utc(&command.issued_at)?;
            let expires = parse_canonical_utc(&command.expires_at)?;
            if issued >= expires {
                return Err(ApprovalVerifierError::InvalidExpiry);
            }
            command
                .identity
                .subject()
                .validate()
                .map_err(|_| ApprovalVerifierError::Contract)?;
        }
        ApprovalCommand::Verify(command) => {
            parse_canonical_utc(&command.observed_at)?;
            validate_identifiers([
                &command.proof.approver_id,
                &command.proof.authenticator_id,
                &command.proof.key_id,
            ])?;
            for value in [
                &command.proof.challenge_digest,
                &command.proof.verification_key_commitment,
                &command.proof.evidence_digest,
                &command.proof.proof_digest,
            ] {
                if is_zero_digest(value) {
                    return Err(ApprovalVerifierError::ZeroDigest);
                }
            }
            if let Some(guardian) = &command.proof.guardian {
                validate_identifiers([&guardian.guardian_id, &guardian.daemon_instance_id])?;
                if guardian.observed_epoch == 0
                    || guardian.observed_epoch > MAX_SIGNED_BIGINT
                    || is_zero_digest(&guardian.trust_root_digest)
                {
                    return Err(ApprovalVerifierError::InvalidIdentifier);
                }
            }
        }
        ApprovalCommand::BindExecution(command) => {
            parse_canonical_utc(&command.observed_at)?;
            if command
                .execution_challenge
                .approval_challenge()
                .identity()
                .approval_id()
                != command.approval_id
                || is_zero_digest(command.execution_challenge.challenge_digest())
                || is_zero_digest(command.execution_proof.proof_digest())
            {
                return Err(ApprovalVerifierError::ChallengeIntegrity);
            }
        }
        ApprovalCommand::ConsumeNormal(command) => {
            parse_canonical_utc(&command.observed_at)?;
            if is_zero_digest(&command.claim_digest) {
                return Err(ApprovalVerifierError::ZeroDigest);
            }
        }
        ApprovalCommand::Revoke(command) => {
            parse_canonical_utc(&command.observed_at)?;
            validate_identifier(&command.revoker_id)?;
            if is_zero_digest(&command.revocation_evidence_digest) {
                return Err(ApprovalVerifierError::ZeroDigest);
            }
        }
    }
    Ok(())
}

fn make_challenge(
    command: &IssueApprovalCommand,
    subject_digest: ContentDigest,
) -> Result<ApprovalChallenge, ApprovalVerifierError> {
    let mut challenge = ApprovalChallenge {
        identity: command.identity.clone(),
        runtime: command.runtime,
        nonce_id: command.nonce_id.clone(),
        nonce_commitment: command.nonce_commitment.clone(),
        issued_at: command.issued_at.clone(),
        expires_at: command.expires_at.clone(),
        subject_digest,
        authenticator_id: command.authenticator_id.clone(),
        key_id: command.key_id.clone(),
        verification_key_commitment: command.verification_key_commitment.clone(),
        evidence_digest: command.evidence_digest.clone(),
        review_set_digest: command.review_set_digest.clone(),
        challenge_digest: placeholder_digest()?,
    };
    challenge.challenge_digest = digest(
        "lattice-approval-challenge",
        challenge_value_without_digest(&challenge),
    )?;
    Ok(challenge)
}

fn validate_challenge_integrity(
    challenge: &ApprovalChallenge,
) -> Result<(), ApprovalVerifierError> {
    let integrity_error = || ApprovalVerifierError::ChallengeIntegrity;
    if challenge.runtime != RuntimeKind::Fake {
        return Err(integrity_error());
    }
    validate_identifiers([
        challenge.identity.approval_id(),
        challenge.identity.challenge_id(),
        challenge.identity.requester_id(),
        challenge.identity.approver_id(),
        challenge.identity.channel_id(),
        challenge.identity.session_id(),
        &challenge.nonce_id,
        &challenge.authenticator_id,
        &challenge.key_id,
    ])
    .map_err(|_| integrity_error())?;
    challenge
        .identity
        .subject()
        .validate()
        .map_err(|_| integrity_error())?;
    let issued = parse_canonical_utc(&challenge.issued_at).map_err(|_| integrity_error())?;
    let expires = parse_canonical_utc(&challenge.expires_at).map_err(|_| integrity_error())?;
    if issued >= expires {
        return Err(integrity_error());
    }
    for value in [
        &challenge.nonce_commitment,
        &challenge.subject_digest,
        &challenge.verification_key_commitment,
        &challenge.evidence_digest,
        &challenge.challenge_digest,
    ]
    .into_iter()
    .chain(challenge.review_set_digest.iter())
    {
        if is_zero_digest(value) {
            return Err(integrity_error());
        }
    }
    let subject_digest =
        approval_subject_digest(challenge.identity.binding(), challenge.identity.subject())
            .map_err(|_| integrity_error())?;
    if subject_digest != challenge.subject_digest {
        return Err(integrity_error());
    }
    let challenge_digest = digest(
        "lattice-approval-challenge",
        challenge_value_without_digest(challenge),
    )
    .map_err(|_| integrity_error())?;
    if challenge_digest != challenge.challenge_digest {
        return Err(integrity_error());
    }
    Ok(())
}

fn validate_normal_signer_challenge(
    signer: &FakeNormalSigner,
    challenge: &ApprovalChallenge,
) -> Result<(), ApprovalVerifierError> {
    validate_challenge_integrity(challenge)?;
    let expected_evidence = fake_signer_evidence(
        ApprovalLane::Normal,
        &signer.approver_id,
        &signer.authenticator_id,
        &signer.key_id,
        &signer.verification_key_commitment,
        None,
    )
    .map_err(|_| ApprovalVerifierError::ChallengeIntegrity)?;
    if challenge.identity.lane() != ApprovalLane::Normal
        || challenge.identity.authority() != ApprovalAuthority::ResponsibleUser
        || challenge.identity.origin() != ApprovalOrigin::OsAuthenticatedUser
        || challenge.identity.approver_id() != signer.approver_id
        || challenge.authenticator_id != signer.authenticator_id
        || challenge.key_id != signer.key_id
        || challenge.verification_key_commitment != signer.verification_key_commitment
        || challenge.evidence_digest != signer.evidence_digest
        || signer.evidence_digest != expected_evidence
    {
        return Err(ApprovalVerifierError::ChallengeIntegrity);
    }
    Ok(())
}

fn validate_protected_signer_challenge(
    signer: &FakeProtectedSigner,
    challenge: &ApprovalChallenge,
) -> Result<(), ApprovalVerifierError> {
    validate_challenge_integrity(challenge)?;
    let guardian = FakeGuardianBinding {
        guardian_id: signer.guardian_id.clone(),
        daemon_instance_id: signer.daemon_instance_id.clone(),
        observed_epoch: signer.observed_epoch,
        trust_root_digest: signer.trust_root_digest.clone(),
    };
    let expected_evidence = fake_signer_evidence(
        ApprovalLane::Protected,
        &signer.guardian_id,
        &signer.authenticator_id,
        &signer.key_id,
        &signer.trust_root_digest,
        Some(&guardian),
    )
    .map_err(|_| ApprovalVerifierError::ChallengeIntegrity)?;
    let ApprovalSubject::ProtectedRelease(release) = challenge.identity.subject() else {
        return Err(ApprovalVerifierError::ChallengeIntegrity);
    };
    let expected_guardian = release.guardian();
    if challenge.identity.lane() != ApprovalLane::Protected
        || challenge.identity.authority() != ApprovalAuthority::ProtectedGuardian
        || challenge.identity.origin() != ApprovalOrigin::GuardianTrustRoot
        || challenge.identity.approver_id() != signer.guardian_id
        || challenge.authenticator_id != signer.authenticator_id
        || challenge.key_id != signer.key_id
        || challenge.verification_key_commitment != signer.trust_root_digest
        || challenge.evidence_digest != signer.evidence_digest
        || signer.evidence_digest != expected_evidence
        || expected_guardian.guardian_id() != signer.guardian_id
        || expected_guardian.daemon_instance_id() != signer.daemon_instance_id
        || expected_guardian.observed_epoch().get() != signer.observed_epoch
        || expected_guardian.trust_root_digest() != &signer.trust_root_digest
    {
        return Err(ApprovalVerifierError::ChallengeIntegrity);
    }
    Ok(())
}

fn proof_matches(
    challenge: &ApprovalChallenge,
    proof: &FakeApprovalProof,
) -> Result<bool, ApprovalVerifierError> {
    if challenge.runtime != RuntimeKind::Fake
        || proof.challenge_digest != challenge.challenge_digest
        || proof.lane != challenge.identity.lane()
        || proof.approver_id != challenge.identity.approver_id()
        || proof.authenticator_id != challenge.authenticator_id
        || proof.key_id != challenge.key_id
        || proof.verification_key_commitment != challenge.verification_key_commitment
        || proof.evidence_digest != challenge.evidence_digest
        || fake_proof_digest(proof)? != proof.proof_digest
    {
        return Ok(false);
    }
    match proof.lane {
        ApprovalLane::Normal => {
            if proof.guardian.is_some()
                || challenge.identity.authority() != ApprovalAuthority::ResponsibleUser
                || challenge.identity.origin() != ApprovalOrigin::OsAuthenticatedUser
            {
                return Ok(false);
            }
        }
        ApprovalLane::Protected => {
            let Some(guardian) = proof.guardian.as_ref() else {
                return Ok(false);
            };
            if challenge.identity.authority() != ApprovalAuthority::ProtectedGuardian
                || challenge.identity.origin() != ApprovalOrigin::GuardianTrustRoot
                || guardian.guardian_id != challenge.identity.approver_id()
                || guardian.trust_root_digest != proof.verification_key_commitment
            {
                return Ok(false);
            }
            if let ApprovalSubject::ProtectedRelease(release) = challenge.identity.subject() {
                let expected = release.guardian();
                if expected.guardian_id() != guardian.guardian_id
                    || expected.daemon_instance_id() != guardian.daemon_instance_id
                    || expected.observed_epoch().get() != guardian.observed_epoch
                    || expected.trust_root_digest() != &guardian.trust_root_digest
                {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

fn observe_window(
    issued_at: &str,
    expires_at: &str,
    observed_at: &str,
) -> Result<Option<ApprovalDenial>, ApprovalVerifierError> {
    let issued = parse_canonical_utc(issued_at)?;
    let expires = parse_canonical_utc(expires_at)?;
    let observed = parse_canonical_utc(observed_at)?;
    if observed < issued {
        Ok(Some(ApprovalDenial::NotYetValid))
    } else if observed >= expires {
        Ok(Some(ApprovalDenial::Expired))
    } else {
        Ok(None)
    }
}

fn state_head(record: &ApprovalRecord) -> Result<ApprovalStateHead, ApprovalVerifierError> {
    let state_digest = digest(
        "lattice-approval-state-head",
        CanonicalValue::Object(vec![
            (
                "approval_id".to_owned(),
                string(record.challenge.identity.approval_id()),
            ),
            ("revision".to_owned(), string(record.revision.to_string())),
            ("phase".to_owned(), string(record.phase.as_str())),
            (
                "challenge_digest".to_owned(),
                string(record.challenge.challenge_digest.as_str()),
            ),
            (
                "authority_receipt_digest".to_owned(),
                optional_digest(
                    record
                        .authority_receipt
                        .as_ref()
                        .map(ApprovalAuthorityReceipt::receipt_digest),
                ),
            ),
            (
                "claim_digest".to_owned(),
                optional_digest(record.claim_digest.as_ref()),
            ),
            (
                "revocation_digest".to_owned(),
                optional_digest(
                    record
                        .revocation
                        .as_ref()
                        .map(ApprovalRevocation::revocation_digest),
                ),
            ),
        ]),
    )?;
    Ok(ApprovalStateHead {
        approval_id: record.challenge.identity.approval_id().to_owned(),
        revision: record.revision,
        phase: record.phase,
        state_digest,
    })
}

fn authority_receipt_digest(
    ordinal: u64,
    challenge: &ApprovalChallenge,
    revision: ApprovalRevision,
    status: ApprovalStatus,
    proof_digest: &ContentDigest,
) -> Result<ContentDigest, ApprovalVerifierError> {
    digest(
        "lattice-approval-authority-receipt",
        CanonicalValue::Object(vec![
            ("ordinal".to_owned(), string(ordinal.to_string())),
            (
                "producer_id".to_owned(),
                string(APPROVAL_VERIFIER_PRODUCER_ID),
            ),
            (
                "producer_version".to_owned(),
                string(APPROVAL_VERIFIER_PRODUCER_VERSION),
            ),
            ("runtime".to_owned(), string("FAKE")),
            ("identity".to_owned(), identity_value(&challenge.identity)),
            ("revision".to_owned(), string(revision.get().to_string())),
            ("status".to_owned(), string(status.as_str())),
            ("nonce_id".to_owned(), string(&challenge.nonce_id)),
            (
                "nonce_commitment".to_owned(),
                string(challenge.nonce_commitment.as_str()),
            ),
            ("issued_at".to_owned(), string(&challenge.issued_at)),
            ("expires_at".to_owned(), string(&challenge.expires_at)),
            (
                "subject_digest".to_owned(),
                string(challenge.subject_digest.as_str()),
            ),
            (
                "challenge_digest".to_owned(),
                string(challenge.challenge_digest.as_str()),
            ),
            (
                "authenticator_id".to_owned(),
                string(&challenge.authenticator_id),
            ),
            ("key_id".to_owned(), string(&challenge.key_id)),
            ("proof_digest".to_owned(), string(proof_digest.as_str())),
            (
                "evidence_digest".to_owned(),
                string(challenge.evidence_digest.as_str()),
            ),
            (
                "review_set_digest".to_owned(),
                optional_digest(challenge.review_set_digest.as_ref()),
            ),
        ]),
    )
}

#[allow(clippy::too_many_arguments)]
fn terminal_receipt_digest(
    ordinal: u64,
    previous_receipt_digest: Option<&ContentDigest>,
    command: &ApprovalCommand,
    request_digest: &ContentDigest,
    before: Option<&ApprovalStateHead>,
    after: Option<&ApprovalStateHead>,
    outcome: ApprovalCommandOutcome,
    challenge: Option<&ApprovalChallenge>,
    authority_receipt: Option<&ApprovalAuthorityReceipt>,
    execution_binding_receipt: Option<&ExecutionApprovalBindingReceipt>,
    revocation: Option<&ApprovalRevocation>,
) -> Result<ContentDigest, ApprovalVerifierError> {
    let mut value = vec![
        ("ordinal".to_owned(), string(ordinal.to_string())),
        (
            "previous_receipt_digest".to_owned(),
            optional_digest(previous_receipt_digest),
        ),
        ("request".to_owned(), command_value(command)),
        ("request_digest".to_owned(), string(request_digest.as_str())),
        ("before".to_owned(), optional_head(before)),
        ("after".to_owned(), optional_head(after)),
        ("outcome".to_owned(), outcome_value(outcome)),
        (
            "challenge_digest".to_owned(),
            optional_digest(challenge.map(|value| &value.challenge_digest)),
        ),
        (
            "authority_receipt_digest".to_owned(),
            optional_digest(authority_receipt.map(ApprovalAuthorityReceipt::receipt_digest)),
        ),
    ];
    if let Some(binding_receipt) = execution_binding_receipt {
        value.push((
            "execution_binding_receipt_digest".to_owned(),
            string(binding_receipt.binding_receipt_digest().as_str()),
        ));
    }
    value.push((
        "revocation_digest".to_owned(),
        optional_digest(revocation.map(ApprovalRevocation::revocation_digest)),
    ));
    digest(
        "lattice-approval-terminal-receipt",
        CanonicalValue::Object(value),
    )
}

fn aggregate_digest(
    aggregate: &VerifiedApprovalAggregate,
) -> Result<ContentDigest, ApprovalVerifierError> {
    digest(
        "lattice-approval-aggregate-state",
        snapshot_value(aggregate),
    )
}

fn snapshot_digest(
    aggregate: &VerifiedApprovalAggregate,
) -> Result<ContentDigest, ApprovalVerifierError> {
    digest("lattice-approval-snapshot", snapshot_value(aggregate))
}

fn nonce_bindings_digest(
    aggregate: &VerifiedApprovalAggregate,
) -> Result<ContentDigest, ApprovalVerifierError> {
    digest(
        "lattice-approval-nonce-bindings",
        nonce_bindings_value(aggregate),
    )
}

fn nonce_bindings_value(aggregate: &VerifiedApprovalAggregate) -> CanonicalValue {
    CanonicalValue::Array(
        aggregate
            .nonce_bindings
            .iter()
            .map(|(commitment, binding)| nonce_binding_value(commitment, binding))
            .collect(),
    )
}

fn snapshot_value(aggregate: &VerifiedApprovalAggregate) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("version".to_owned(), string(SCHEMA_VERSION)),
        (
            "command_high_water".to_owned(),
            string(aggregate.command_receipts.len().to_string()),
        ),
        (
            "command_tail_digest".to_owned(),
            optional_digest(
                aggregate
                    .command_receipts
                    .last()
                    .map(|receipt| &receipt.receipt_digest),
            ),
        ),
        (
            "nonce_bindings_digest".to_owned(),
            nonce_bindings_digest(aggregate)
                .map_or(CanonicalValue::Null, |value| string(value.as_str())),
        ),
        (
            "approvals".to_owned(),
            CanonicalValue::Array(aggregate.approvals.values().map(record_value).collect()),
        ),
        ("nonce_bindings".to_owned(), nonce_bindings_value(aggregate)),
        (
            "commands".to_owned(),
            CanonicalValue::Array(
                aggregate
                    .command_receipts
                    .iter()
                    .map(terminal_receipt_value)
                    .collect(),
            ),
        ),
    ])
}

fn record_value(record: &ApprovalRecord) -> CanonicalValue {
    let mut value = vec![
        ("challenge".to_owned(), challenge_value(&record.challenge)),
        ("phase".to_owned(), string(record.phase.as_str())),
        ("revision".to_owned(), string(record.revision.to_string())),
        (
            "authority_receipt".to_owned(),
            record
                .authority_receipt
                .as_ref()
                .map_or(CanonicalValue::Null, authority_receipt_value),
        ),
    ];
    if let Some(binding_receipt) = record.execution_binding_receipt.as_ref() {
        value.push((
            "execution_binding_receipt".to_owned(),
            execution_binding_receipt_value(binding_receipt),
        ));
    }
    value.extend([
        (
            "claim_digest".to_owned(),
            optional_digest(record.claim_digest.as_ref()),
        ),
        (
            "revocation".to_owned(),
            record
                .revocation
                .as_ref()
                .map_or(CanonicalValue::Null, revocation_value),
        ),
    ]);
    CanonicalValue::Object(value)
}

fn terminal_receipt_value(receipt: &ApprovalCommandReceipt) -> CanonicalValue {
    let mut value = vec![
        ("ordinal".to_owned(), string(receipt.ordinal.to_string())),
        (
            "previous_receipt_digest".to_owned(),
            optional_digest(receipt.previous_receipt_digest.as_ref()),
        ),
        ("request".to_owned(), command_value(&receipt.request)),
        (
            "request_digest".to_owned(),
            string(receipt.request_digest.as_str()),
        ),
        ("before".to_owned(), optional_head(receipt.before.as_ref())),
        ("after".to_owned(), optional_head(receipt.after.as_ref())),
        ("outcome".to_owned(), outcome_value(receipt.outcome)),
        (
            "challenge".to_owned(),
            receipt
                .challenge
                .as_ref()
                .map_or(CanonicalValue::Null, challenge_value),
        ),
        (
            "authority_receipt".to_owned(),
            receipt
                .authority_receipt
                .as_ref()
                .map_or(CanonicalValue::Null, authority_receipt_value),
        ),
    ];
    if let Some(binding_receipt) = receipt.execution_binding_receipt.as_ref() {
        value.push((
            "execution_binding_receipt".to_owned(),
            execution_binding_receipt_value(binding_receipt),
        ));
    }
    value.extend([
        (
            "revocation".to_owned(),
            receipt
                .revocation
                .as_ref()
                .map_or(CanonicalValue::Null, revocation_value),
        ),
        (
            "receipt_digest".to_owned(),
            string(receipt.receipt_digest.as_str()),
        ),
    ]);
    CanonicalValue::Object(value)
}

fn authority_receipt_value(receipt: &ApprovalAuthorityReceipt) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("version".to_owned(), string(receipt.version().to_string())),
        ("producer_id".to_owned(), string(receipt.producer_id())),
        (
            "producer_version".to_owned(),
            string(receipt.producer_version()),
        ),
        ("runtime".to_owned(), runtime_value(receipt.runtime())),
        ("identity".to_owned(), identity_value(receipt.identity())),
        (
            "revision".to_owned(),
            string(receipt.revision().get().to_string()),
        ),
        ("status".to_owned(), string(receipt.status().as_str())),
        ("nonce_id".to_owned(), string(receipt.nonce_id())),
        (
            "nonce_commitment".to_owned(),
            string(receipt.nonce_commitment().as_str()),
        ),
        ("issued_at".to_owned(), string(receipt.issued_at())),
        ("expires_at".to_owned(), string(receipt.expires_at())),
        (
            "subject_digest".to_owned(),
            string(receipt.subject_digest().as_str()),
        ),
        (
            "challenge_digest".to_owned(),
            string(receipt.challenge_digest().as_str()),
        ),
        (
            "authenticator_id".to_owned(),
            string(receipt.authenticator_id()),
        ),
        ("key_id".to_owned(), string(receipt.key_id())),
        (
            "proof_digest".to_owned(),
            string(receipt.proof_digest().as_str()),
        ),
        (
            "evidence_digest".to_owned(),
            string(receipt.evidence_digest().as_str()),
        ),
        (
            "review_set_digest".to_owned(),
            optional_digest(receipt.review_set_digest()),
        ),
        (
            "receipt_digest".to_owned(),
            string(receipt.receipt_digest().as_str()),
        ),
    ])
}

fn revocation_value_without_digest(revocation: &ApprovalRevocation) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("approval_id".to_owned(), string(&revocation.approval_id)),
        (
            "revision".to_owned(),
            string(revocation.revision.get().to_string()),
        ),
        ("status".to_owned(), string(revocation.status.as_str())),
        ("revoker_id".to_owned(), string(&revocation.revoker_id)),
        ("observed_at".to_owned(), string(&revocation.observed_at)),
        (
            "revocation_evidence_digest".to_owned(),
            string(revocation.revocation_evidence_digest.as_str()),
        ),
        (
            "prior_authority_receipt_digest".to_owned(),
            string(revocation.prior_authority_receipt_digest.as_str()),
        ),
    ])
}

fn revocation_value(revocation: &ApprovalRevocation) -> CanonicalValue {
    let CanonicalValue::Object(mut entries) = revocation_value_without_digest(revocation) else {
        unreachable!("revocation projection is always an object");
    };
    entries.push((
        "revocation_digest".to_owned(),
        string(revocation.revocation_digest.as_str()),
    ));
    CanonicalValue::Object(entries)
}

fn nonce_binding_value(commitment: &str, binding: &NonceBinding) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("commitment".to_owned(), string(commitment)),
        ("nonce_id".to_owned(), string(&binding.nonce_id)),
        ("approval_id".to_owned(), string(&binding.approval_id)),
        ("challenge_id".to_owned(), string(&binding.challenge_id)),
        (
            "subject_digest".to_owned(),
            string(binding.subject_digest.as_str()),
        ),
        ("lane".to_owned(), string(binding.lane.as_str())),
    ])
}

fn fake_key_commitment(
    lane: ApprovalLane,
    actor_id: &str,
    authenticator_id: &str,
    key_id: &str,
    secret: &SecretMaterial,
) -> Result<ContentDigest, ApprovalVerifierError> {
    digest(
        "lattice-approval-fake-key-commitment",
        CanonicalValue::Object(vec![
            ("runtime".to_owned(), string("FAKE")),
            ("lane".to_owned(), string(lane.as_str())),
            ("actor_id".to_owned(), string(actor_id)),
            ("authenticator_id".to_owned(), string(authenticator_id)),
            ("key_id".to_owned(), string(key_id)),
            ("secret_hex".to_owned(), string(hex_bytes(&secret.bytes))),
        ]),
    )
}

fn fake_signer_evidence(
    lane: ApprovalLane,
    actor_id: &str,
    authenticator_id: &str,
    key_id: &str,
    key_commitment: &ContentDigest,
    guardian: Option<&FakeGuardianBinding>,
) -> Result<ContentDigest, ApprovalVerifierError> {
    digest(
        "lattice-approval-fake-signer-evidence",
        CanonicalValue::Object(vec![
            ("runtime".to_owned(), string("FAKE")),
            ("lane".to_owned(), string(lane.as_str())),
            ("actor_id".to_owned(), string(actor_id)),
            ("authenticator_id".to_owned(), string(authenticator_id)),
            ("key_id".to_owned(), string(key_id)),
            ("key_commitment".to_owned(), string(key_commitment.as_str())),
            (
                "guardian".to_owned(),
                guardian.map_or(CanonicalValue::Null, guardian_value),
            ),
        ]),
    )
}

fn fake_proof_digest(proof: &FakeApprovalProof) -> Result<ContentDigest, ApprovalVerifierError> {
    digest(
        "lattice-approval-fake-proof",
        proof_value_without_digest(proof),
    )
}

fn approval_subject_digest(
    binding: &SubjectBinding,
    subject: &ApprovalSubject,
) -> Result<ContentDigest, ApprovalVerifierError> {
    digest(
        "lattice-approval-subject",
        CanonicalValue::Object(vec![
            ("binding".to_owned(), binding_value(binding)),
            ("subject".to_owned(), subject_value(subject)),
        ]),
    )
}

fn challenge_value_without_digest(challenge: &ApprovalChallenge) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("identity".to_owned(), identity_value(&challenge.identity)),
        ("runtime".to_owned(), runtime_value(challenge.runtime)),
        ("nonce_id".to_owned(), string(&challenge.nonce_id)),
        (
            "nonce_commitment".to_owned(),
            string(challenge.nonce_commitment.as_str()),
        ),
        ("issued_at".to_owned(), string(&challenge.issued_at)),
        ("expires_at".to_owned(), string(&challenge.expires_at)),
        (
            "subject_digest".to_owned(),
            string(challenge.subject_digest.as_str()),
        ),
        (
            "authenticator_id".to_owned(),
            string(&challenge.authenticator_id),
        ),
        ("key_id".to_owned(), string(&challenge.key_id)),
        (
            "verification_key_commitment".to_owned(),
            string(challenge.verification_key_commitment.as_str()),
        ),
        (
            "evidence_digest".to_owned(),
            string(challenge.evidence_digest.as_str()),
        ),
        (
            "review_set_digest".to_owned(),
            optional_digest(challenge.review_set_digest.as_ref()),
        ),
    ])
}

fn challenge_value(challenge: &ApprovalChallenge) -> CanonicalValue {
    let CanonicalValue::Object(mut entries) = challenge_value_without_digest(challenge) else {
        unreachable!("challenge projection is always an object");
    };
    entries.push((
        "challenge_digest".to_owned(),
        string(challenge.challenge_digest.as_str()),
    ));
    CanonicalValue::Object(entries)
}

fn proof_value_without_digest(proof: &FakeApprovalProof) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "challenge_digest".to_owned(),
            string(proof.challenge_digest.as_str()),
        ),
        ("runtime".to_owned(), string("FAKE")),
        ("lane".to_owned(), string(proof.lane.as_str())),
        ("approver_id".to_owned(), string(&proof.approver_id)),
        (
            "authenticator_id".to_owned(),
            string(&proof.authenticator_id),
        ),
        ("key_id".to_owned(), string(&proof.key_id)),
        (
            "verification_key_commitment".to_owned(),
            string(proof.verification_key_commitment.as_str()),
        ),
        (
            "evidence_digest".to_owned(),
            string(proof.evidence_digest.as_str()),
        ),
        (
            "guardian".to_owned(),
            proof
                .guardian
                .as_ref()
                .map_or(CanonicalValue::Null, guardian_value),
        ),
    ])
}

fn proof_value(proof: &FakeApprovalProof) -> CanonicalValue {
    let CanonicalValue::Object(mut entries) = proof_value_without_digest(proof) else {
        unreachable!("proof projection is always an object");
    };
    entries.push((
        "proof_digest".to_owned(),
        string(proof.proof_digest.as_str()),
    ));
    CanonicalValue::Object(entries)
}

fn guardian_value(guardian: &FakeGuardianBinding) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("guardian_id".to_owned(), string(&guardian.guardian_id)),
        (
            "daemon_instance_id".to_owned(),
            string(&guardian.daemon_instance_id),
        ),
        (
            "observed_epoch".to_owned(),
            string(guardian.observed_epoch.to_string()),
        ),
        (
            "trust_root_digest".to_owned(),
            string(guardian.trust_root_digest.as_str()),
        ),
    ])
}

fn execution_approval_subject_value(subject: &ExecutionApprovalSubject) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("task_ref".to_owned(), string(subject.task_ref().as_str())),
        (
            "successor_stream_id".to_owned(),
            string(subject.successor_stream_id().as_str()),
        ),
        ("binding".to_owned(), binding_value(subject.binding())),
        (
            "approval_subject_digest".to_owned(),
            string(subject.approval_subject_digest().as_str()),
        ),
        (
            "budget_digest".to_owned(),
            string(subject.budget_digest().as_str()),
        ),
        (
            "subject_digest".to_owned(),
            string(subject.subject_digest().as_str()),
        ),
    ])
}

fn execution_approval_challenge_value(challenge: &ExecutionApprovalChallenge) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "approval_challenge".to_owned(),
            challenge_value(challenge.approval_challenge()),
        ),
        (
            "subject".to_owned(),
            execution_approval_subject_value(challenge.subject()),
        ),
        (
            "challenge_digest".to_owned(),
            string(challenge.challenge_digest().as_str()),
        ),
    ])
}

fn execution_approval_proof_value(proof: &FakeExecutionApprovalProof) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("base_proof".to_owned(), proof_value(proof.base_proof())),
        (
            "execution_challenge_digest".to_owned(),
            string(proof.execution_challenge_digest().as_str()),
        ),
        (
            "proof_digest".to_owned(),
            string(proof.proof_digest().as_str()),
        ),
    ])
}

fn execution_binding_receipt_value(receipt: &ExecutionApprovalBindingReceipt) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "subject".to_owned(),
            execution_approval_subject_value(receipt.subject()),
        ),
        (
            "approval_receipt_digest".to_owned(),
            string(receipt.approval_receipt_digest().as_str()),
        ),
        (
            "execution_challenge_digest".to_owned(),
            string(receipt.execution_challenge_digest().as_str()),
        ),
        (
            "execution_proof_digest".to_owned(),
            string(receipt.execution_proof_digest().as_str()),
        ),
        (
            "binding_receipt_digest".to_owned(),
            string(receipt.binding_receipt_digest().as_str()),
        ),
    ])
}

fn command_value(command: &ApprovalCommand) -> CanonicalValue {
    match command {
        ApprovalCommand::Issue(command) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("ISSUE")),
            ("command_id".to_owned(), string(&command.command_id)),
            (
                "expected_head".to_owned(),
                optional_head(command.expected_head.as_ref()),
            ),
            ("runtime".to_owned(), runtime_value(command.runtime)),
            ("identity".to_owned(), identity_value(&command.identity)),
            ("nonce_id".to_owned(), string(&command.nonce_id)),
            (
                "nonce_commitment".to_owned(),
                string(command.nonce_commitment.as_str()),
            ),
            ("issued_at".to_owned(), string(&command.issued_at)),
            ("expires_at".to_owned(), string(&command.expires_at)),
            (
                "authenticator_id".to_owned(),
                string(&command.authenticator_id),
            ),
            ("key_id".to_owned(), string(&command.key_id)),
            (
                "verification_key_commitment".to_owned(),
                string(command.verification_key_commitment.as_str()),
            ),
            (
                "evidence_digest".to_owned(),
                string(command.evidence_digest.as_str()),
            ),
            (
                "review_set_digest".to_owned(),
                optional_digest(command.review_set_digest.as_ref()),
            ),
        ]),
        ApprovalCommand::Verify(command) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("VERIFY")),
            ("command_id".to_owned(), string(&command.command_id)),
            ("approval_id".to_owned(), string(&command.approval_id)),
            (
                "expected_head".to_owned(),
                state_head_value(&command.expected_head),
            ),
            ("observed_at".to_owned(), string(&command.observed_at)),
            ("proof".to_owned(), proof_value(&command.proof)),
        ]),
        ApprovalCommand::BindExecution(command) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("BIND_EXECUTION")),
            ("command_id".to_owned(), string(&command.command_id)),
            ("approval_id".to_owned(), string(&command.approval_id)),
            (
                "expected_head".to_owned(),
                state_head_value(&command.expected_head),
            ),
            ("observed_at".to_owned(), string(&command.observed_at)),
            (
                "execution_challenge".to_owned(),
                execution_approval_challenge_value(&command.execution_challenge),
            ),
            (
                "execution_proof".to_owned(),
                execution_approval_proof_value(&command.execution_proof),
            ),
        ]),
        ApprovalCommand::ConsumeNormal(command) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("CONSUME_NORMAL")),
            ("command_id".to_owned(), string(&command.command_id)),
            ("approval_id".to_owned(), string(&command.approval_id)),
            (
                "expected_head".to_owned(),
                state_head_value(&command.expected_head),
            ),
            ("observed_at".to_owned(), string(&command.observed_at)),
            (
                "claim_digest".to_owned(),
                string(command.claim_digest.as_str()),
            ),
        ]),
        ApprovalCommand::Revoke(command) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("REVOKE")),
            ("command_id".to_owned(), string(&command.command_id)),
            ("approval_id".to_owned(), string(&command.approval_id)),
            (
                "expected_head".to_owned(),
                state_head_value(&command.expected_head),
            ),
            ("observed_at".to_owned(), string(&command.observed_at)),
            ("revoker_id".to_owned(), string(&command.revoker_id)),
            (
                "revocation_evidence_digest".to_owned(),
                string(command.revocation_evidence_digest.as_str()),
            ),
        ]),
    }
}

fn optional_head(head: Option<&ApprovalStateHead>) -> CanonicalValue {
    head.map_or(CanonicalValue::Null, state_head_value)
}

fn state_head_value(head: &ApprovalStateHead) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("approval_id".to_owned(), string(&head.approval_id)),
        ("revision".to_owned(), string(head.revision.to_string())),
        ("phase".to_owned(), string(head.phase.as_str())),
        (
            "state_digest".to_owned(),
            string(head.state_digest.as_str()),
        ),
    ])
}

fn outcome_value(outcome: ApprovalCommandOutcome) -> CanonicalValue {
    match outcome {
        ApprovalCommandOutcome::Applied => {
            CanonicalValue::Object(vec![("kind".to_owned(), string("APPLIED"))])
        }
        ApprovalCommandOutcome::Denied(denial) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("DENIED")),
            ("reason".to_owned(), string(denial.code())),
        ]),
    }
}

fn identity_value(identity: &ApprovalIdentity) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("approval_id".to_owned(), string(identity.approval_id())),
        ("challenge_id".to_owned(), string(identity.challenge_id())),
        ("binding".to_owned(), binding_value(identity.binding())),
        ("subject".to_owned(), subject_value(identity.subject())),
        ("requester_id".to_owned(), string(identity.requester_id())),
        ("approver_id".to_owned(), string(identity.approver_id())),
        (
            "authority".to_owned(),
            string(identity.authority().as_str()),
        ),
        ("origin".to_owned(), string(identity.origin().as_str())),
        ("lane".to_owned(), string(identity.lane().as_str())),
        ("channel_id".to_owned(), string(identity.channel_id())),
        ("session_id".to_owned(), string(identity.session_id())),
    ])
}

fn binding_value(binding: &SubjectBinding) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "project_id".to_owned(),
            string(binding.project_id().as_str()),
        ),
        (
            "project_snapshot_id".to_owned(),
            string(binding.project_snapshot_id().as_str()),
        ),
        ("task_id".to_owned(), string(binding.task_id().as_str())),
        ("task_revision".to_owned(), string(binding.task_revision())),
        (
            "task_spec_digest".to_owned(),
            string(binding.task_spec_digest().as_str()),
        ),
    ])
}

fn subject_value(subject: &ApprovalSubject) -> CanonicalValue {
    match subject {
        ApprovalSubject::Execution {
            task_spec_hash,
            external_cost,
        } => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("EXECUTION")),
            ("task_spec_hash".to_owned(), string(task_spec_hash.as_str())),
            (
                "external_cost".to_owned(),
                external_cost
                    .as_ref()
                    .map_or(CanonicalValue::Null, external_cost_value),
            ),
        ]),
        ApprovalSubject::Merge(subject) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("MERGE")),
            ("merge".to_owned(), merge_subject_value(subject)),
        ]),
        ApprovalSubject::Preference(subject) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("PREFERENCE")),
            ("memory".to_owned(), memory_subject_value(subject)),
        ]),
        ApprovalSubject::ProtectedChange(subject) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("PROTECTED_CHANGE")),
            (
                "protected_change".to_owned(),
                protected_change_value(subject),
            ),
        ]),
        ApprovalSubject::ProtectedRelease(subject) => CanonicalValue::Object(vec![
            ("kind".to_owned(), string("PROTECTED_RELEASE")),
            (
                "protected_release".to_owned(),
                protected_release_value(subject),
            ),
        ]),
    }
}

fn external_cost_value(subject: &ExternalCostSubject) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("amount".to_owned(), string(subject.amount())),
        ("currency".to_owned(), string(subject.currency())),
        ("provider_id".to_owned(), string(subject.provider_id())),
        (
            "quote_digest".to_owned(),
            string(subject.quote_digest().as_str()),
        ),
        (
            "pricing_digest".to_owned(),
            string(subject.pricing_digest().as_str()),
        ),
    ])
}

fn merge_subject_value(subject: &MergeSubject) -> CanonicalValue {
    let (target_kind, reference) = match subject.target() {
        MergeTarget::Unbound => ("UNBOUND", CanonicalValue::Null),
        MergeTarget::FeatureBranch(reference) => ("FEATURE_BRANCH", string(reference)),
        MergeTarget::PrimaryBranch(reference) => ("PRIMARY_BRANCH", string(reference)),
    };
    CanonicalValue::Object(vec![
        ("target_kind".to_owned(), string(target_kind)),
        ("reference".to_owned(), reference),
        (
            "reviewed_commit".to_owned(),
            string(subject.reviewed_commit()),
        ),
        ("target_head".to_owned(), string(subject.target_head())),
        (
            "diff_digest".to_owned(),
            string(subject.diff_digest().as_str()),
        ),
    ])
}

fn memory_subject_value(subject: &MemoryCandidateSubject) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("binding".to_owned(), binding_value(subject.binding())),
        (
            "candidate_digest".to_owned(),
            string(subject.candidate_digest().as_str()),
        ),
        ("memory_kind".to_owned(), string(subject.kind().as_str())),
    ])
}

fn protected_change_value(subject: &ProtectedChangeSubject) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("class".to_owned(), string(subject.class().as_str())),
        (
            "operation_digest".to_owned(),
            string(subject.operation_digest().as_str()),
        ),
    ])
}

fn protected_release_value(subject: &ProtectedReleaseSubject) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("release".to_owned(), release_value(subject.release())),
        (
            "guardian".to_owned(),
            guardian_runtime_value(subject.guardian()),
        ),
    ])
}

fn release_value(subject: &ReleaseSubject) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("activation_id".to_owned(), string(subject.activation_id())),
        ("saga_id".to_owned(), string(subject.saga_id())),
        ("release_id".to_owned(), string(subject.release_id())),
        (
            "release_revision".to_owned(),
            string(subject.release_revision()),
        ),
        (
            "manifest_digest".to_owned(),
            string(subject.manifest_digest().as_str()),
        ),
        ("source_commit".to_owned(), string(subject.source_commit())),
        (
            "source_tree_digest".to_owned(),
            string(subject.source_tree_digest().as_str()),
        ),
        (
            "dependency_lock_digest".to_owned(),
            string(subject.dependency_lock_digest().as_str()),
        ),
        (
            "binary_digests".to_owned(),
            digest_array(subject.binary_digests()),
        ),
        (
            "migration_digests".to_owned(),
            digest_array(subject.migration_digests()),
        ),
        (
            "evidence_digest".to_owned(),
            string(subject.evidence_digest().as_str()),
        ),
        (
            "source_release_id".to_owned(),
            string(subject.source_release_id()),
        ),
        (
            "source_manifest_digest".to_owned(),
            string(subject.source_manifest_digest().as_str()),
        ),
        (
            "source_slot_id".to_owned(),
            string(subject.source_slot_id()),
        ),
        (
            "target_slot_id".to_owned(),
            string(subject.target_slot_id()),
        ),
        (
            "requested_epoch".to_owned(),
            string(subject.requested_epoch().get().to_string()),
        ),
        (
            "schema_compatible".to_owned(),
            CanonicalValue::Bool(subject.schema_compatible()),
        ),
        ("delta".to_owned(), upgrade_delta_value(subject.delta())),
    ])
}

fn guardian_runtime_value(subject: &GuardianRuntimeSubject) -> CanonicalValue {
    CanonicalValue::Object(vec![
        ("guardian_id".to_owned(), string(subject.guardian_id())),
        (
            "trust_root_digest".to_owned(),
            string(subject.trust_root_digest().as_str()),
        ),
        (
            "daemon_instance_id".to_owned(),
            string(subject.daemon_instance_id()),
        ),
        (
            "observed_epoch".to_owned(),
            string(subject.observed_epoch().get().to_string()),
        ),
    ])
}

fn upgrade_delta_value(delta: UpgradeDelta) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "schema_migration".to_owned(),
            CanonicalValue::Bool(delta.schema_migration()),
        ),
        ("policy".to_owned(), CanonicalValue::Bool(delta.policy())),
        (
            "constitution".to_owned(),
            CanonicalValue::Bool(delta.constitution()),
        ),
        (
            "supervisor".to_owned(),
            CanonicalValue::Bool(delta.supervisor()),
        ),
        (
            "credentials".to_owned(),
            CanonicalValue::Bool(delta.credentials()),
        ),
        (
            "public_exposure".to_owned(),
            CanonicalValue::Bool(delta.public_exposure()),
        ),
        (
            "destructive".to_owned(),
            CanonicalValue::Bool(delta.destructive()),
        ),
        (
            "capability_expansion".to_owned(),
            CanonicalValue::Bool(delta.capability_expansion()),
        ),
    ])
}

fn digest_array(values: &[ContentDigest]) -> CanonicalValue {
    CanonicalValue::Array(values.iter().map(|value| string(value.as_str())).collect())
}

fn runtime_value(runtime: RuntimeKind) -> CanonicalValue {
    string(match runtime {
        RuntimeKind::Fake => "FAKE",
        RuntimeKind::Live => "LIVE",
    })
}

fn optional_digest(value: Option<&ContentDigest>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |digest| string(digest.as_str()))
}

struct DecodedSnapshot {
    command_high_water: u64,
    command_tail_digest: Option<ContentDigest>,
    nonce_bindings_digest: ContentDigest,
    commands: Vec<RawTerminalCommand>,
}

struct RawTerminalCommand {
    ordinal: u64,
    previous_receipt_digest: Option<ContentDigest>,
    request: ApprovalCommand,
    receipt_digest: ContentDigest,
    raw: CanonicalValue,
}

struct RawObject<'a> {
    fields: BTreeMap<&'a str, &'a CanonicalValue>,
}

impl<'a> RawObject<'a> {
    fn new(value: &'a CanonicalValue) -> Result<Self, ApprovalVerifierError> {
        let CanonicalValue::Object(entries) = value else {
            return Err(ApprovalVerifierError::CorruptSnapshot);
        };
        let mut fields = BTreeMap::new();
        for (key, value) in entries {
            if fields.insert(key.as_str(), value).is_some() {
                return Err(ApprovalVerifierError::CorruptSnapshot);
            }
        }
        Ok(Self { fields })
    }

    fn exact(value: &'a CanonicalValue, expected: &[&str]) -> Result<Self, ApprovalVerifierError> {
        let object = Self::new(value)?;
        object.expect_fields(expected)?;
        Ok(object)
    }

    fn expect_fields(&self, expected: &[&str]) -> Result<(), ApprovalVerifierError> {
        if self.fields.len() != expected.len()
            || expected
                .iter()
                .any(|field| !self.fields.contains_key(field))
        {
            return Err(ApprovalVerifierError::CorruptSnapshot);
        }
        Ok(())
    }

    fn value(&self, field: &str) -> Result<&'a CanonicalValue, ApprovalVerifierError> {
        self.fields
            .get(field)
            .copied()
            .ok_or(ApprovalVerifierError::CorruptSnapshot)
    }
}

fn decode_snapshot(value: &CanonicalValue) -> Result<DecodedSnapshot, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "version",
            "command_high_water",
            "command_tail_digest",
            "nonce_bindings_digest",
            "approvals",
            "nonce_bindings",
            "commands",
        ],
    )?;
    if raw_string(object.value("version")?)? != SCHEMA_VERSION {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    let commands = raw_array(object.value("commands")?)?
        .iter()
        .map(parse_terminal_command)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DecodedSnapshot {
        command_high_water: raw_u64(object.value("command_high_water")?)?,
        command_tail_digest: parse_optional_digest(object.value("command_tail_digest")?)?,
        nonce_bindings_digest: parse_digest(object.value("nonce_bindings_digest")?)?,
        commands,
    })
}

fn parse_terminal_command(
    value: &CanonicalValue,
) -> Result<RawTerminalCommand, ApprovalVerifierError> {
    let object = RawObject::new(value)?;
    if object.fields.contains_key("execution_binding_receipt") {
        object.expect_fields(&[
            "ordinal",
            "previous_receipt_digest",
            "request",
            "request_digest",
            "before",
            "after",
            "outcome",
            "challenge",
            "authority_receipt",
            "execution_binding_receipt",
            "revocation",
            "receipt_digest",
        ])?;
    } else {
        object.expect_fields(&[
            "ordinal",
            "previous_receipt_digest",
            "request",
            "request_digest",
            "before",
            "after",
            "outcome",
            "challenge",
            "authority_receipt",
            "revocation",
            "receipt_digest",
        ])?;
    }
    Ok(RawTerminalCommand {
        ordinal: raw_u64(object.value("ordinal")?)?,
        previous_receipt_digest: parse_optional_digest(object.value("previous_receipt_digest")?)?,
        request: parse_command(object.value("request")?)?,
        receipt_digest: parse_digest(object.value("receipt_digest")?)?,
        raw: value.clone(),
    })
}

fn parse_command(value: &CanonicalValue) -> Result<ApprovalCommand, ApprovalVerifierError> {
    let object = RawObject::new(value)?;
    match raw_string(object.value("kind")?)? {
        "ISSUE" => {
            object.expect_fields(&[
                "kind",
                "command_id",
                "expected_head",
                "runtime",
                "identity",
                "nonce_id",
                "nonce_commitment",
                "issued_at",
                "expires_at",
                "authenticator_id",
                "key_id",
                "verification_key_commitment",
                "evidence_digest",
                "review_set_digest",
            ])?;
            Ok(ApprovalCommand::Issue(IssueApprovalCommand {
                command_id: raw_string(object.value("command_id")?)?.to_owned(),
                expected_head: parse_optional_state_head(object.value("expected_head")?)?,
                runtime: parse_runtime(object.value("runtime")?)?,
                identity: parse_identity(object.value("identity")?)?,
                nonce_id: raw_string(object.value("nonce_id")?)?.to_owned(),
                nonce_commitment: parse_digest(object.value("nonce_commitment")?)?,
                issued_at: raw_string(object.value("issued_at")?)?.to_owned(),
                expires_at: raw_string(object.value("expires_at")?)?.to_owned(),
                authenticator_id: raw_string(object.value("authenticator_id")?)?.to_owned(),
                key_id: raw_string(object.value("key_id")?)?.to_owned(),
                verification_key_commitment: parse_digest(
                    object.value("verification_key_commitment")?,
                )?,
                evidence_digest: parse_digest(object.value("evidence_digest")?)?,
                review_set_digest: parse_optional_digest(object.value("review_set_digest")?)?,
            }))
        }
        "VERIFY" => {
            object.expect_fields(&[
                "kind",
                "command_id",
                "approval_id",
                "expected_head",
                "observed_at",
                "proof",
            ])?;
            Ok(ApprovalCommand::Verify(VerifyApprovalCommand {
                command_id: raw_string(object.value("command_id")?)?.to_owned(),
                approval_id: raw_string(object.value("approval_id")?)?.to_owned(),
                expected_head: parse_state_head(object.value("expected_head")?)?,
                observed_at: raw_string(object.value("observed_at")?)?.to_owned(),
                proof: parse_proof(object.value("proof")?)?,
            }))
        }
        "BIND_EXECUTION" => {
            object.expect_fields(&[
                "kind",
                "command_id",
                "approval_id",
                "expected_head",
                "observed_at",
                "execution_challenge",
                "execution_proof",
            ])?;
            Ok(ApprovalCommand::BindExecution(
                BindExecutionApprovalCommand {
                    command_id: raw_string(object.value("command_id")?)?.to_owned(),
                    approval_id: raw_string(object.value("approval_id")?)?.to_owned(),
                    expected_head: parse_state_head(object.value("expected_head")?)?,
                    observed_at: raw_string(object.value("observed_at")?)?.to_owned(),
                    execution_challenge: parse_execution_approval_challenge(
                        object.value("execution_challenge")?,
                    )?,
                    execution_proof: parse_execution_approval_proof(
                        object.value("execution_proof")?,
                    )?,
                },
            ))
        }
        "CONSUME_NORMAL" => {
            object.expect_fields(&[
                "kind",
                "command_id",
                "approval_id",
                "expected_head",
                "observed_at",
                "claim_digest",
            ])?;
            Ok(ApprovalCommand::ConsumeNormal(
                ConsumeNormalApprovalCommand {
                    command_id: raw_string(object.value("command_id")?)?.to_owned(),
                    approval_id: raw_string(object.value("approval_id")?)?.to_owned(),
                    expected_head: parse_state_head(object.value("expected_head")?)?,
                    observed_at: raw_string(object.value("observed_at")?)?.to_owned(),
                    claim_digest: parse_digest(object.value("claim_digest")?)?,
                },
            ))
        }
        "REVOKE" => {
            object.expect_fields(&[
                "kind",
                "command_id",
                "approval_id",
                "expected_head",
                "observed_at",
                "revoker_id",
                "revocation_evidence_digest",
            ])?;
            Ok(ApprovalCommand::Revoke(RevokeApprovalCommand {
                command_id: raw_string(object.value("command_id")?)?.to_owned(),
                approval_id: raw_string(object.value("approval_id")?)?.to_owned(),
                expected_head: parse_state_head(object.value("expected_head")?)?,
                observed_at: raw_string(object.value("observed_at")?)?.to_owned(),
                revoker_id: raw_string(object.value("revoker_id")?)?.to_owned(),
                revocation_evidence_digest: parse_digest(
                    object.value("revocation_evidence_digest")?,
                )?,
            }))
        }
        _ => Err(ApprovalVerifierError::CorruptSnapshot),
    }
}

fn parse_state_head(value: &CanonicalValue) -> Result<ApprovalStateHead, ApprovalVerifierError> {
    let object = RawObject::exact(value, &["approval_id", "revision", "phase", "state_digest"])?;
    let revision = raw_u64(object.value("revision")?)?;
    if revision == 0 || revision > MAX_SIGNED_BIGINT {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    Ok(ApprovalStateHead {
        approval_id: raw_string(object.value("approval_id")?)?.to_owned(),
        revision,
        phase: parse_phase(object.value("phase")?)?,
        state_digest: parse_digest(object.value("state_digest")?)?,
    })
}

fn parse_optional_state_head(
    value: &CanonicalValue,
) -> Result<Option<ApprovalStateHead>, ApprovalVerifierError> {
    if matches!(value, CanonicalValue::Null) {
        Ok(None)
    } else {
        parse_state_head(value).map(Some)
    }
}

fn parse_phase(value: &CanonicalValue) -> Result<ApprovalPhase, ApprovalVerifierError> {
    match raw_string(value)? {
        "CHALLENGED" => Ok(ApprovalPhase::Challenged),
        "VERIFIED_AVAILABLE" => Ok(ApprovalPhase::VerifiedAvailable),
        "VERIFIED_PROTECTED_PENDING_CLAIM" => Ok(ApprovalPhase::VerifiedProtectedPendingClaim),
        "CLAIMED_NORMAL" => Ok(ApprovalPhase::ClaimedNormal),
        "REVOKED" => Ok(ApprovalPhase::Revoked),
        _ => Err(ApprovalVerifierError::CorruptSnapshot),
    }
}

fn parse_proof(value: &CanonicalValue) -> Result<FakeApprovalProof, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "challenge_digest",
            "runtime",
            "lane",
            "approver_id",
            "authenticator_id",
            "key_id",
            "verification_key_commitment",
            "evidence_digest",
            "guardian",
            "proof_digest",
        ],
    )?;
    if parse_runtime(object.value("runtime")?)? != RuntimeKind::Fake {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    Ok(FakeApprovalProof {
        challenge_digest: parse_digest(object.value("challenge_digest")?)?,
        lane: parse_lane(object.value("lane")?)?,
        approver_id: raw_string(object.value("approver_id")?)?.to_owned(),
        authenticator_id: raw_string(object.value("authenticator_id")?)?.to_owned(),
        key_id: raw_string(object.value("key_id")?)?.to_owned(),
        verification_key_commitment: parse_digest(object.value("verification_key_commitment")?)?,
        evidence_digest: parse_digest(object.value("evidence_digest")?)?,
        guardian: parse_optional_fake_guardian(object.value("guardian")?)?,
        proof_digest: parse_digest(object.value("proof_digest")?)?,
    })
}

fn parse_approval_challenge(
    value: &CanonicalValue,
) -> Result<ApprovalChallenge, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "identity",
            "runtime",
            "nonce_id",
            "nonce_commitment",
            "issued_at",
            "expires_at",
            "subject_digest",
            "authenticator_id",
            "key_id",
            "verification_key_commitment",
            "evidence_digest",
            "review_set_digest",
            "challenge_digest",
        ],
    )?;
    let challenge = ApprovalChallenge {
        identity: parse_identity(object.value("identity")?)?,
        runtime: parse_runtime(object.value("runtime")?)?,
        nonce_id: raw_string(object.value("nonce_id")?)?.to_owned(),
        nonce_commitment: parse_digest(object.value("nonce_commitment")?)?,
        issued_at: raw_string(object.value("issued_at")?)?.to_owned(),
        expires_at: raw_string(object.value("expires_at")?)?.to_owned(),
        subject_digest: parse_digest(object.value("subject_digest")?)?,
        authenticator_id: raw_string(object.value("authenticator_id")?)?.to_owned(),
        key_id: raw_string(object.value("key_id")?)?.to_owned(),
        verification_key_commitment: parse_digest(object.value("verification_key_commitment")?)?,
        evidence_digest: parse_digest(object.value("evidence_digest")?)?,
        review_set_digest: parse_optional_digest(object.value("review_set_digest")?)?,
        challenge_digest: parse_digest(object.value("challenge_digest")?)?,
    };
    let expected = digest(
        "lattice-approval-challenge",
        challenge_value_without_digest(&challenge),
    )?;
    if challenge.challenge_digest != expected {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    Ok(challenge)
}

fn parse_execution_approval_subject(
    value: &CanonicalValue,
) -> Result<ExecutionApprovalSubject, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "task_ref",
            "successor_stream_id",
            "binding",
            "approval_subject_digest",
            "budget_digest",
            "subject_digest",
        ],
    )?;
    let expected_digest = parse_digest(object.value("subject_digest")?)?;
    let subject = ExecutionApprovalSubject::new(
        parse_digest(object.value("task_ref")?)?,
        parse_digest(object.value("successor_stream_id")?)?,
        parse_binding(object.value("binding")?)?,
        parse_digest(object.value("approval_subject_digest")?)?,
        parse_digest(object.value("budget_digest")?)?,
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
    if subject.subject_digest() != &expected_digest {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    Ok(subject)
}

fn parse_execution_approval_challenge(
    value: &CanonicalValue,
) -> Result<ExecutionApprovalChallenge, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &["approval_challenge", "subject", "challenge_digest"],
    )?;
    let expected_digest = parse_digest(object.value("challenge_digest")?)?;
    let challenge = ExecutionApprovalChallenge::new(
        parse_approval_challenge(object.value("approval_challenge")?)?,
        parse_execution_approval_subject(object.value("subject")?)?,
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)?;
    if challenge.challenge_digest() != &expected_digest {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    Ok(challenge)
}

fn parse_execution_approval_proof(
    value: &CanonicalValue,
) -> Result<FakeExecutionApprovalProof, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &["base_proof", "execution_challenge_digest", "proof_digest"],
    )?;
    recover_fake_execution_approval_proof(
        parse_proof(object.value("base_proof")?)?,
        parse_digest(object.value("execution_challenge_digest")?)?,
        parse_digest(object.value("proof_digest")?)?,
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_optional_fake_guardian(
    value: &CanonicalValue,
) -> Result<Option<FakeGuardianBinding>, ApprovalVerifierError> {
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    let object = RawObject::exact(
        value,
        &[
            "guardian_id",
            "daemon_instance_id",
            "observed_epoch",
            "trust_root_digest",
        ],
    )?;
    Ok(Some(FakeGuardianBinding {
        guardian_id: raw_string(object.value("guardian_id")?)?.to_owned(),
        daemon_instance_id: raw_string(object.value("daemon_instance_id")?)?.to_owned(),
        observed_epoch: raw_u64(object.value("observed_epoch")?)?,
        trust_root_digest: parse_digest(object.value("trust_root_digest")?)?,
    }))
}

fn parse_identity(value: &CanonicalValue) -> Result<ApprovalIdentity, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "approval_id",
            "challenge_id",
            "binding",
            "subject",
            "requester_id",
            "approver_id",
            "authority",
            "origin",
            "lane",
            "channel_id",
            "session_id",
        ],
    )?;
    ApprovalIdentity::new(
        raw_string(object.value("approval_id")?)?,
        raw_string(object.value("challenge_id")?)?,
        parse_binding(object.value("binding")?)?,
        parse_subject(object.value("subject")?)?,
        raw_string(object.value("requester_id")?)?,
        raw_string(object.value("approver_id")?)?,
        parse_authority(object.value("authority")?)?,
        parse_origin(object.value("origin")?)?,
        parse_lane(object.value("lane")?)?,
        raw_string(object.value("channel_id")?)?,
        raw_string(object.value("session_id")?)?,
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_binding(value: &CanonicalValue) -> Result<SubjectBinding, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "project_id",
            "project_snapshot_id",
            "task_id",
            "task_revision",
            "task_spec_digest",
        ],
    )?;
    SubjectBinding::new(
        ProjectId::new(raw_string(object.value("project_id")?)?)
            .map_err(|_| ApprovalVerifierError::CorruptSnapshot)?,
        ProjectSnapshotId::new(raw_string(object.value("project_snapshot_id")?)?)
            .map_err(|_| ApprovalVerifierError::CorruptSnapshot)?,
        TaskId::new(raw_string(object.value("task_id")?)?)
            .map_err(|_| ApprovalVerifierError::CorruptSnapshot)?,
        raw_string(object.value("task_revision")?)?,
        parse_digest(object.value("task_spec_digest")?)?,
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_subject(value: &CanonicalValue) -> Result<ApprovalSubject, ApprovalVerifierError> {
    let object = RawObject::new(value)?;
    match raw_string(object.value("kind")?)? {
        "EXECUTION" => {
            object.expect_fields(&["kind", "task_spec_hash", "external_cost"])?;
            Ok(ApprovalSubject::Execution {
                task_spec_hash: parse_digest(object.value("task_spec_hash")?)?,
                external_cost: parse_optional_external_cost(object.value("external_cost")?)?,
            })
        }
        "MERGE" => {
            object.expect_fields(&["kind", "merge"])?;
            Ok(ApprovalSubject::Merge(parse_merge_subject(
                object.value("merge")?,
            )?))
        }
        "PREFERENCE" => {
            object.expect_fields(&["kind", "memory"])?;
            Ok(ApprovalSubject::Preference(parse_memory_subject(
                object.value("memory")?,
            )?))
        }
        "PROTECTED_CHANGE" => {
            object.expect_fields(&["kind", "protected_change"])?;
            Ok(ApprovalSubject::ProtectedChange(parse_protected_change(
                object.value("protected_change")?,
            )?))
        }
        "PROTECTED_RELEASE" => {
            object.expect_fields(&["kind", "protected_release"])?;
            Ok(ApprovalSubject::ProtectedRelease(Box::new(
                parse_protected_release(object.value("protected_release")?)?,
            )))
        }
        _ => Err(ApprovalVerifierError::CorruptSnapshot),
    }
}

fn parse_optional_external_cost(
    value: &CanonicalValue,
) -> Result<Option<ExternalCostSubject>, ApprovalVerifierError> {
    if matches!(value, CanonicalValue::Null) {
        return Ok(None);
    }
    let object = RawObject::exact(
        value,
        &[
            "amount",
            "currency",
            "provider_id",
            "quote_digest",
            "pricing_digest",
        ],
    )?;
    ExternalCostSubject::new(
        raw_string(object.value("amount")?)?,
        raw_string(object.value("currency")?)?,
        raw_string(object.value("provider_id")?)?,
        parse_digest(object.value("quote_digest")?)?,
        parse_digest(object.value("pricing_digest")?)?,
    )
    .map(Some)
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_merge_subject(value: &CanonicalValue) -> Result<MergeSubject, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "target_kind",
            "reference",
            "reviewed_commit",
            "target_head",
            "diff_digest",
        ],
    )?;
    let target = match raw_string(object.value("target_kind")?)? {
        "UNBOUND" if matches!(object.value("reference")?, CanonicalValue::Null) => {
            MergeTarget::Unbound
        }
        "FEATURE_BRANCH" => {
            MergeTarget::FeatureBranch(raw_string(object.value("reference")?)?.to_owned())
        }
        "PRIMARY_BRANCH" => {
            MergeTarget::PrimaryBranch(raw_string(object.value("reference")?)?.to_owned())
        }
        _ => return Err(ApprovalVerifierError::CorruptSnapshot),
    };
    MergeSubject::new(
        target,
        raw_string(object.value("reviewed_commit")?)?,
        raw_string(object.value("target_head")?)?,
        parse_digest(object.value("diff_digest")?)?,
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_memory_subject(
    value: &CanonicalValue,
) -> Result<MemoryCandidateSubject, ApprovalVerifierError> {
    let object = RawObject::exact(value, &["binding", "candidate_digest", "memory_kind"])?;
    MemoryCandidateSubject::new(
        parse_binding(object.value("binding")?)?,
        parse_digest(object.value("candidate_digest")?)?,
        match raw_string(object.value("memory_kind")?)? {
            "FACT" => MemoryKind::Fact,
            "OBSERVATION" => MemoryKind::Observation,
            "INFERENCE" => MemoryKind::Inference,
            "PREFERENCE" => MemoryKind::Preference,
            _ => return Err(ApprovalVerifierError::CorruptSnapshot),
        },
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_protected_change(
    value: &CanonicalValue,
) -> Result<ProtectedChangeSubject, ApprovalVerifierError> {
    let object = RawObject::exact(value, &["class", "operation_digest"])?;
    let class = match raw_string(object.value("class")?)? {
        "ACCOUNT_OR_CREDENTIAL" => ProtectedChangeClass::AccountOrCredential,
        "PAYMENT_OR_PURCHASE" => ProtectedChangeClass::PaymentOrPurchase,
        "PUBLIC_EXPOSURE" => ProtectedChangeClass::PublicExposure,
        "PRODUCTION_DEPLOYMENT" => ProtectedChangeClass::ProductionDeployment,
        "PERMANENT_DELETE" => ProtectedChangeClass::PermanentDelete,
        "DISABLE_SECURITY" => ProtectedChangeClass::DisableSecurity,
        "DESTRUCTIVE_MIGRATION" => ProtectedChangeClass::DestructiveMigration,
        "POLICY" => ProtectedChangeClass::Policy,
        "CONSTITUTION" => ProtectedChangeClass::Constitution,
        "SUPERVISOR" => ProtectedChangeClass::Supervisor,
        "CAPABILITY_EXPANSION" => ProtectedChangeClass::CapabilityExpansion,
        "PRIMARY_BRANCH_MERGE" => ProtectedChangeClass::PrimaryBranchMerge,
        "CORE_RELEASE_ACTIVATION" => ProtectedChangeClass::CoreReleaseActivation,
        _ => return Err(ApprovalVerifierError::CorruptSnapshot),
    };
    ProtectedChangeSubject::new(class, parse_digest(object.value("operation_digest")?)?)
        .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_protected_release(
    value: &CanonicalValue,
) -> Result<ProtectedReleaseSubject, ApprovalVerifierError> {
    let object = RawObject::exact(value, &["release", "guardian"])?;
    Ok(ProtectedReleaseSubject::new(
        parse_release(object.value("release")?)?,
        parse_guardian_runtime(object.value("guardian")?)?,
    ))
}

fn parse_release(value: &CanonicalValue) -> Result<ReleaseSubject, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "activation_id",
            "saga_id",
            "release_id",
            "release_revision",
            "manifest_digest",
            "source_commit",
            "source_tree_digest",
            "dependency_lock_digest",
            "binary_digests",
            "migration_digests",
            "evidence_digest",
            "source_release_id",
            "source_manifest_digest",
            "source_slot_id",
            "target_slot_id",
            "requested_epoch",
            "schema_compatible",
            "delta",
        ],
    )?;
    ReleaseSubject::new(
        raw_string(object.value("activation_id")?)?,
        raw_string(object.value("saga_id")?)?,
        raw_string(object.value("release_id")?)?,
        raw_string(object.value("release_revision")?)?,
        parse_digest(object.value("manifest_digest")?)?,
        raw_string(object.value("source_commit")?)?,
        parse_digest(object.value("source_tree_digest")?)?,
        parse_digest(object.value("dependency_lock_digest")?)?,
        parse_digest_array(object.value("binary_digests")?)?,
        parse_digest_array(object.value("migration_digests")?)?,
        parse_digest(object.value("evidence_digest")?)?,
        raw_string(object.value("source_release_id")?)?,
        parse_digest(object.value("source_manifest_digest")?)?,
        raw_string(object.value("source_slot_id")?)?,
        raw_string(object.value("target_slot_id")?)?,
        parse_epoch(object.value("requested_epoch")?)?,
        raw_bool(object.value("schema_compatible")?)?,
        parse_upgrade_delta(object.value("delta")?)?,
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_guardian_runtime(
    value: &CanonicalValue,
) -> Result<GuardianRuntimeSubject, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "guardian_id",
            "trust_root_digest",
            "daemon_instance_id",
            "observed_epoch",
        ],
    )?;
    GuardianRuntimeSubject::new(
        raw_string(object.value("guardian_id")?)?,
        parse_digest(object.value("trust_root_digest")?)?,
        raw_string(object.value("daemon_instance_id")?)?,
        parse_epoch(object.value("observed_epoch")?)?,
    )
    .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_upgrade_delta(value: &CanonicalValue) -> Result<UpgradeDelta, ApprovalVerifierError> {
    let object = RawObject::exact(
        value,
        &[
            "schema_migration",
            "policy",
            "constitution",
            "supervisor",
            "credentials",
            "public_exposure",
            "destructive",
            "capability_expansion",
        ],
    )?;
    Ok(UpgradeDelta::new(
        raw_bool(object.value("schema_migration")?)?,
        raw_bool(object.value("policy")?)?,
        raw_bool(object.value("constitution")?)?,
        raw_bool(object.value("supervisor")?)?,
        raw_bool(object.value("credentials")?)?,
        raw_bool(object.value("public_exposure")?)?,
        raw_bool(object.value("destructive")?)?,
        raw_bool(object.value("capability_expansion")?)?,
    ))
}

fn parse_authority(value: &CanonicalValue) -> Result<ApprovalAuthority, ApprovalVerifierError> {
    match raw_string(value)? {
        "INTERNAL_POLICY" => Ok(ApprovalAuthority::InternalPolicy),
        "RESPONSIBLE_USER" => Ok(ApprovalAuthority::ResponsibleUser),
        "PROTECTED_GUARDIAN" => Ok(ApprovalAuthority::ProtectedGuardian),
        _ => Err(ApprovalVerifierError::CorruptSnapshot),
    }
}

fn parse_origin(value: &CanonicalValue) -> Result<ApprovalOrigin, ApprovalVerifierError> {
    match raw_string(value)? {
        "POLICY_ENGINE" => Ok(ApprovalOrigin::PolicyEngine),
        "OS_AUTHENTICATED_USER" => Ok(ApprovalOrigin::OsAuthenticatedUser),
        "GUARDIAN_TRUST_ROOT" => Ok(ApprovalOrigin::GuardianTrustRoot),
        "NORMAL_GATEWAY" => Ok(ApprovalOrigin::NormalGateway),
        "MODEL_OR_CANDIDATE" => Ok(ApprovalOrigin::ModelOrCandidate),
        "ACTIVE_DAEMON" => Ok(ApprovalOrigin::ActiveDaemon),
        _ => Err(ApprovalVerifierError::CorruptSnapshot),
    }
}

fn parse_lane(value: &CanonicalValue) -> Result<ApprovalLane, ApprovalVerifierError> {
    match raw_string(value)? {
        "NORMAL" => Ok(ApprovalLane::Normal),
        "PROTECTED" => Ok(ApprovalLane::Protected),
        _ => Err(ApprovalVerifierError::CorruptSnapshot),
    }
}

fn parse_runtime(value: &CanonicalValue) -> Result<RuntimeKind, ApprovalVerifierError> {
    match raw_string(value)? {
        "FAKE" => Ok(RuntimeKind::Fake),
        "LIVE" => Ok(RuntimeKind::Live),
        _ => Err(ApprovalVerifierError::CorruptSnapshot),
    }
}

fn parse_epoch(value: &CanonicalValue) -> Result<DaemonEpoch, ApprovalVerifierError> {
    DaemonEpoch::new(raw_u64(value)?).map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn parse_digest_array(value: &CanonicalValue) -> Result<Vec<ContentDigest>, ApprovalVerifierError> {
    raw_array(value)?
        .iter()
        .map(parse_digest)
        .collect::<Result<Vec<_>, _>>()
}

fn parse_optional_digest(
    value: &CanonicalValue,
) -> Result<Option<ContentDigest>, ApprovalVerifierError> {
    if matches!(value, CanonicalValue::Null) {
        Ok(None)
    } else {
        parse_digest(value).map(Some)
    }
}

fn parse_digest(value: &CanonicalValue) -> Result<ContentDigest, ApprovalVerifierError> {
    ContentDigest::from_sha256(raw_string(value)?)
        .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

fn raw_string(value: &CanonicalValue) -> Result<&str, ApprovalVerifierError> {
    let CanonicalValue::String(value) = value else {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    };
    Ok(value)
}

fn raw_bool(value: &CanonicalValue) -> Result<bool, ApprovalVerifierError> {
    let CanonicalValue::Bool(value) = value else {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    };
    Ok(*value)
}

fn raw_array(value: &CanonicalValue) -> Result<&[CanonicalValue], ApprovalVerifierError> {
    let CanonicalValue::Array(values) = value else {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    };
    Ok(values)
}

fn raw_u64(value: &CanonicalValue) -> Result<u64, ApprovalVerifierError> {
    let value = raw_string(value)?;
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ApprovalVerifierError::CorruptSnapshot);
    }
    value
        .parse()
        .map_err(|_| ApprovalVerifierError::CorruptSnapshot)
}

#[allow(clippy::needless_pass_by_value)]
fn digest(schema_id: &str, value: CanonicalValue) -> Result<ContentDigest, ApprovalVerifierError> {
    let domain =
        HashDomain::new(schema_id, SCHEMA_VERSION).map_err(|_| ApprovalVerifierError::Canonical)?;
    let digest = canonical_sha256(&domain, &value).map_err(|_| ApprovalVerifierError::Canonical)?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| ApprovalVerifierError::Contract)
}

fn placeholder_digest() -> Result<ContentDigest, ApprovalVerifierError> {
    ContentDigest::from_sha256("1".repeat(64)).map_err(|_| ApprovalVerifierError::Contract)
}

fn parse_canonical_utc(value: &str) -> Result<OffsetDateTime, ApprovalVerifierError> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|_| ApprovalVerifierError::InvalidTimestamp)?;
    if parsed.offset() != UtcOffset::UTC
        || parsed
            .format(&Rfc3339)
            .map_err(|_| ApprovalVerifierError::InvalidTimestamp)?
            != value
    {
        return Err(ApprovalVerifierError::InvalidTimestamp);
    }
    Ok(parsed)
}

fn validate_identifiers<const N: usize>(values: [&str; N]) -> Result<(), ApprovalVerifierError> {
    for value in values {
        validate_identifier(value)?;
    }
    Ok(())
}

fn validate_identifier(value: &str) -> Result<(), ApprovalVerifierError> {
    let valid = (1..=128).contains(&value.len())
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(ApprovalVerifierError::InvalidIdentifier)
    }
}

fn is_zero_digest(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}

fn hex_bytes(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_digest(character: char) -> ContentDigest {
        ContentDigest::from_sha256(character.to_string().repeat(64)).expect("test digest")
    }

    #[test]
    fn signer_rejects_tampered_cloned_challenge_with_preserved_digest() {
        let signer = FakeNormalSigner::new(
            "approver-integrity",
            "authenticator-integrity",
            "key-integrity",
            SecretMaterial::new(b"fake-integrity-key".to_vec()).expect("signer secret"),
        )
        .expect("signer");
        let task_spec_digest = test_digest('a');
        let binding = SubjectBinding::new(
            ProjectId::new("project-integrity").expect("project"),
            ProjectSnapshotId::new("snapshot-integrity").expect("snapshot"),
            TaskId::new("task-integrity").expect("task"),
            "1",
            task_spec_digest.clone(),
        )
        .expect("binding");
        let identity = ApprovalIdentity::new(
            "approval-integrity",
            "challenge-integrity",
            binding,
            ApprovalSubject::Execution {
                task_spec_hash: task_spec_digest,
                external_cost: None,
            },
            "requester-integrity",
            signer.approver_id(),
            ApprovalAuthority::ResponsibleUser,
            ApprovalOrigin::OsAuthenticatedUser,
            ApprovalLane::Normal,
            "channel-integrity",
            "session-integrity",
        )
        .expect("identity");
        let nonce = SecretMaterial::new(b"fake-integrity-nonce".to_vec()).expect("nonce material");
        let mut verifier = FakeApprovalVerifier::new();
        let issued = verifier
            .issue(IssueApprovalCommand {
                command_id: "issue-integrity".to_owned(),
                expected_head: None,
                runtime: RuntimeKind::Fake,
                identity,
                nonce_id: "nonce-integrity".to_owned(),
                nonce_commitment: nonce_commitment(&nonce).expect("nonce commitment"),
                issued_at: "2026-07-29T00:00:00Z".to_owned(),
                expires_at: "2026-07-29T00:05:00Z".to_owned(),
                authenticator_id: signer.authenticator_id().to_owned(),
                key_id: signer.key_id().to_owned(),
                verification_key_commitment: signer.verification_key_commitment().clone(),
                evidence_digest: signer.evidence_digest().clone(),
                review_set_digest: None,
            })
            .expect("issue");
        let challenge = issued.challenge.expect("challenge");
        let preserved_digest = challenge.challenge_digest().clone();
        let mut tampered = challenge.clone();
        tampered.nonce_id = "nonce-substituted".to_owned();

        assert_eq!(tampered.challenge_digest(), &preserved_digest);
        assert_eq!(
            signer.sign(&tampered),
            Err(ApprovalVerifierError::ChallengeIntegrity)
        );
    }
}
