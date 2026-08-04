//! Deterministic, I/O-free Artifact Store lifecycle semantics.

#[path = "semantics/snapshot_restore.rs"]
mod snapshot_restore;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_contracts::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactAuthorityHead,
    ArtifactAuthorityReceipt, ArtifactAvailability, ArtifactByteLength, ArtifactCounter,
    ArtifactDeleteStatus, ArtifactGeneration, ArtifactObjectHead, ArtifactObjectIdentity,
    ArtifactObjectKey, ArtifactReadAuthorityAction, ArtifactReadAuthorityHead,
    ArtifactReadAuthorityPair, ArtifactReadClosureEvidenceHead, ArtifactReadClosureEvidencePair,
    ArtifactReadHead, ArtifactReadStatus, ArtifactReferenceAuthorityAction,
    ArtifactReferenceAuthorityHead, ArtifactReferenceAuthorityPair, ArtifactReferenceHead,
    ArtifactReferenceManifest, ArtifactReferenceStatus, ArtifactRevision,
    ArtifactSweepAuthorityHead, ArtifactSweepAuthorityPair, ContentDigest, ProjectId, RuntimeKind,
    TaskId,
};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    ArtifactLimitKind, ArtifactObjectQuotaRecord, ArtifactObjectQuotaState, ArtifactReadIdentity,
    ArtifactReadQuotaRecord, ArtifactReadQuotaState, ArtifactReferenceIdentity,
    ArtifactReferenceQuotaRecord, ArtifactReferenceQuotaState, ArtifactStoreLimits,
};

const MAX_READ_LEASE_SECONDS: i64 = 15 * 60;
const CONTRACT_VERSION: u16 = 1;

/// Stable fail-closed lifecycle error without raw artifact bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactLifecycleError {
    /// A shared contract constructor rejected an internally projected value.
    InvalidContract,
    /// Exact content bytes do not match the project-scoped SHA-256 identity.
    DigestMismatch,
    /// Exact content bytes do not match the immutable manifest byte length.
    LengthMismatch,
    /// The immutable manifest projection or its configured limit binding drifted.
    ManifestMismatch,
    /// A configured object, reference, read, task, project, or store bound failed.
    LimitExceeded {
        /// The stable limit field that rejected the transition.
        field: &'static str,
    },
    /// The independently queried authority record does not exist.
    AuthorityMissing,
    /// A receipt no longer equals the independently queried current head.
    AuthorityStale,
    /// The deterministic fake cannot accept live authority.
    AuthorityRuntimeMismatch,
    /// The typed authority action does not match the requested transition.
    AuthorityActionMismatch,
    /// The typed authority scope does not match the exact object or record.
    AuthorityScopeMismatch,
    /// No current object exists for the exact project-scoped key.
    ObjectNotFound,
    /// The exact object generation is not the current generation.
    GenerationMismatch,
    /// The current generation does not accept the requested transition.
    ObjectUnavailable,
    /// The reference identifier already exists or was terminally released.
    ReferenceTerminal,
    /// The exact reference does not exist.
    ReferenceNotFound,
    /// The exact read claim already exists or was terminally released.
    ReadTerminal,
    /// The exact read claim does not exist.
    ReadNotFound,
    /// The explicit canonical observation reached the lease deadline.
    ReadExpiredSuspect,
    /// A read lease is empty, inverted, malformed, or exceeds 15 minutes.
    InvalidReadLease,
    /// Expiry or reconciliation evidence is not exact/current.
    InvalidReadEvidence,
    /// The separated fake byte backend has no bytes for this generation.
    MissingBytes,
    /// The separated fake byte backend contains digest- or length-corrupt bytes.
    CorruptBytes,
    /// Active references or active/suspect reads block deletion.
    DeleteBlocked,
    /// Retention or grace has not elapsed at the explicit observation time.
    RetentionActive,
    /// The supplied full Artifact Store head or delete plan is stale.
    StalePlan,
    /// Root-composed quota/history evidence is missing, duplicated, or stale.
    IntegratedEvidenceMismatch,
    /// A delete-claim token is absent or does not match exactly.
    ClaimTokenMismatch,
    /// The requested delete result is illegal for the current lifecycle state.
    InvalidDeleteOutcome,
    /// Reconciliation evidence disagrees with metadata or owned bytes.
    ReconciliationMismatch,
    /// Canonical hashing or RFC 3339 parsing failed.
    Canonicalization,
    /// A non-wrapping generation or revision was exhausted.
    CounterExhausted,
}

impl ArtifactLifecycleError {
    /// Returns a stable machine-readable failure code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidContract => "ARTIFACT_INVALID_CONTRACT",
            Self::DigestMismatch => "ARTIFACT_DIGEST_MISMATCH",
            Self::LengthMismatch => "ARTIFACT_LENGTH_MISMATCH",
            Self::ManifestMismatch => "ARTIFACT_MANIFEST_MISMATCH",
            Self::LimitExceeded { .. } => "ARTIFACT_LIMIT_EXCEEDED",
            Self::AuthorityMissing => "ARTIFACT_AUTHORITY_MISSING",
            Self::AuthorityStale => "ARTIFACT_AUTHORITY_STALE",
            Self::AuthorityRuntimeMismatch => "ARTIFACT_AUTHORITY_RUNTIME_MISMATCH",
            Self::AuthorityActionMismatch => "ARTIFACT_AUTHORITY_ACTION_MISMATCH",
            Self::AuthorityScopeMismatch => "ARTIFACT_AUTHORITY_SCOPE_MISMATCH",
            Self::ObjectNotFound => "ARTIFACT_OBJECT_NOT_FOUND",
            Self::GenerationMismatch => "ARTIFACT_GENERATION_MISMATCH",
            Self::ObjectUnavailable => "ARTIFACT_OBJECT_UNAVAILABLE",
            Self::ReferenceTerminal => "ARTIFACT_REFERENCE_TERMINAL",
            Self::ReferenceNotFound => "ARTIFACT_REFERENCE_NOT_FOUND",
            Self::ReadTerminal => "ARTIFACT_READ_TERMINAL",
            Self::ReadNotFound => "ARTIFACT_READ_NOT_FOUND",
            Self::ReadExpiredSuspect => "ARTIFACT_READ_EXPIRED_SUSPECT",
            Self::InvalidReadLease => "ARTIFACT_INVALID_READ_LEASE",
            Self::InvalidReadEvidence => "ARTIFACT_INVALID_READ_EVIDENCE",
            Self::MissingBytes => "ARTIFACT_BYTES_MISSING",
            Self::CorruptBytes => "ARTIFACT_BYTES_CORRUPT",
            Self::DeleteBlocked => "ARTIFACT_DELETE_BLOCKED",
            Self::RetentionActive => "ARTIFACT_RETENTION_ACTIVE",
            Self::StalePlan => "ARTIFACT_STALE_DELETE_PLAN",
            Self::IntegratedEvidenceMismatch => "ARTIFACT_INTEGRATED_EVIDENCE_MISMATCH",
            Self::ClaimTokenMismatch => "ARTIFACT_DELETE_CLAIM_TOKEN_MISMATCH",
            Self::InvalidDeleteOutcome => "ARTIFACT_INVALID_DELETE_OUTCOME",
            Self::ReconciliationMismatch => "ARTIFACT_RECONCILIATION_MISMATCH",
            Self::Canonicalization => "ARTIFACT_CANONICALIZATION_FAILED",
            Self::CounterExhausted => "ARTIFACT_COUNTER_EXHAUSTED",
        }
    }
}

impl fmt::Display for ArtifactLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Self::LimitExceeded { field } = self {
            write!(formatter, "artifact limit exceeded: {field}")
        } else {
            formatter.write_str(self.code())
        }
    }
}

impl Error for ArtifactLifecycleError {}

/// Physically separate, explicitly non-durable fake byte storage.
///
/// Its `Debug` representation reports only an object count and never byte
/// payloads, byte previews, digests derived from payload display, or paths.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct FakeArtifactBytes {
    objects: HashMap<String, Vec<u8>>,
}

impl fmt::Debug for FakeArtifactBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeArtifactBytes")
            .field("object_count", &self.objects.len())
            .finish_non_exhaustive()
    }
}

impl FakeArtifactBytes {
    /// Returns the number of physically represented fake generations.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Removes bytes to inject a deterministic missing-byte read fault.
    pub fn remove_for_test(&mut self, object: &ArtifactObjectIdentity) {
        self.objects.remove(&identity_token(object));
    }

    /// Replaces bytes to inject a deterministic corruption read fault.
    pub fn replace_for_test(&mut self, object: &ArtifactObjectIdentity, replacement: Vec<u8>) {
        self.objects.insert(identity_token(object), replacement);
    }

    fn get(&self, object: &ArtifactObjectIdentity) -> Option<&[u8]> {
        self.objects.get(&identity_token(object)).map(Vec::as_slice)
    }

    fn insert(&mut self, object: &ArtifactObjectIdentity, bytes: &[u8]) {
        self.objects.insert(identity_token(object), bytes.to_vec());
    }

    pub(crate) fn remove(&mut self, object: &ArtifactObjectIdentity) {
        self.objects.remove(&identity_token(object));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AuthorityHead {
    Reference(ArtifactReferenceAuthorityHead),
    Read(ArtifactReadAuthorityHead),
    ReadClosure(ArtifactReadClosureEvidenceHead),
    Sweep(ArtifactSweepAuthorityHead),
}

/// Independently queried current-head directory used only by the fake.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FakeArtifactAuthorityDirectory {
    heads: HashMap<String, AuthorityHead>,
}

impl FakeArtifactAuthorityDirectory {
    /// Installs or advances one independently obtained reference-owner head.
    pub fn set_reference_head(&mut self, head: ArtifactReferenceAuthorityHead) {
        let key = reference_authority_key(&head);
        self.heads.insert(key, AuthorityHead::Reference(head));
    }

    /// Installs or advances one independently obtained read-owner head.
    pub fn set_read_head(&mut self, head: ArtifactReadAuthorityHead) {
        let key = read_authority_key(&head);
        self.heads.insert(key, AuthorityHead::Read(head));
    }

    /// Installs or advances one independently obtained read-closure verifier head.
    pub fn set_read_closure_head(&mut self, head: ArtifactReadClosureEvidenceHead) {
        let key = read_closure_evidence_key(&head);
        self.heads.insert(key, AuthorityHead::ReadClosure(head));
    }

    /// Installs or advances one independently obtained sweep-owner head.
    pub fn set_sweep_head(&mut self, head: ArtifactSweepAuthorityHead) {
        let key = sweep_authority_key(&head);
        self.heads.insert(key, AuthorityHead::Sweep(head));
    }

    /// Convenience installation of the independently carried reference head.
    pub fn install_reference_pair(&mut self, pair: &ArtifactReferenceAuthorityPair) {
        self.set_reference_head(pair.current_head().clone());
    }

    /// Convenience installation of the independently carried read head.
    pub fn install_read_pair(&mut self, pair: &ArtifactReadAuthorityPair) {
        self.set_read_head(pair.current_head().clone());
    }

    /// Convenience installation of independently carried read-closure evidence.
    pub fn install_read_closure_pair(&mut self, pair: &ArtifactReadClosureEvidencePair) {
        self.set_read_closure_head(pair.current_head().clone());
    }

    /// Convenience installation of the independently carried sweep head.
    pub fn install_sweep_pair(&mut self, pair: &ArtifactSweepAuthorityPair) {
        self.set_sweep_head(pair.current_head().clone());
    }

    fn verify_reference(
        &self,
        pair: &ArtifactReferenceAuthorityPair,
    ) -> Result<(), ArtifactLifecycleError> {
        if pair.receipt().binding().runtime() != RuntimeKind::Fake {
            return Err(ArtifactLifecycleError::AuthorityRuntimeMismatch);
        }
        match self
            .heads
            .get(&reference_authority_key(pair.current_head()))
        {
            Some(AuthorityHead::Reference(head)) if head == pair.current_head() => Ok(()),
            Some(AuthorityHead::Reference(_)) => Err(ArtifactLifecycleError::AuthorityStale),
            Some(_) => Err(ArtifactLifecycleError::AuthorityScopeMismatch),
            None => Err(ArtifactLifecycleError::AuthorityMissing),
        }
    }

    fn verify_read(&self, pair: &ArtifactReadAuthorityPair) -> Result<(), ArtifactLifecycleError> {
        if pair.receipt().binding().runtime() != RuntimeKind::Fake {
            return Err(ArtifactLifecycleError::AuthorityRuntimeMismatch);
        }
        match self.heads.get(&read_authority_key(pair.current_head())) {
            Some(AuthorityHead::Read(head)) if head == pair.current_head() => Ok(()),
            Some(AuthorityHead::Read(_)) => Err(ArtifactLifecycleError::AuthorityStale),
            Some(_) => Err(ArtifactLifecycleError::AuthorityScopeMismatch),
            None => Err(ArtifactLifecycleError::AuthorityMissing),
        }
    }

    fn verify_read_closure(
        &self,
        pair: &ArtifactReadClosureEvidencePair,
    ) -> Result<(), ArtifactLifecycleError> {
        if pair.receipt().binding().runtime() != RuntimeKind::Fake {
            return Err(ArtifactLifecycleError::AuthorityRuntimeMismatch);
        }
        match self
            .heads
            .get(&read_closure_evidence_key(pair.current_head()))
        {
            Some(AuthorityHead::ReadClosure(head)) if head == pair.current_head() => Ok(()),
            Some(AuthorityHead::ReadClosure(_)) => Err(ArtifactLifecycleError::AuthorityStale),
            Some(_) => Err(ArtifactLifecycleError::AuthorityScopeMismatch),
            None => Err(ArtifactLifecycleError::AuthorityMissing),
        }
    }

    fn verify_sweep(
        &self,
        pair: &ArtifactSweepAuthorityPair,
    ) -> Result<(), ArtifactLifecycleError> {
        if pair.receipt().binding().runtime() != RuntimeKind::Fake {
            return Err(ArtifactLifecycleError::AuthorityRuntimeMismatch);
        }
        match self.heads.get(&sweep_authority_key(pair.current_head())) {
            Some(AuthorityHead::Sweep(head)) if head == pair.current_head() => Ok(()),
            Some(AuthorityHead::Sweep(_)) => Err(ArtifactLifecycleError::AuthorityStale),
            Some(_) => Err(ArtifactLifecycleError::AuthorityScopeMismatch),
            None => Err(ArtifactLifecycleError::AuthorityMissing),
        }
    }
}

/// Immutable, read-only delete plan bound to one exact full store head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactDeletePlan {
    object: ArtifactObjectIdentity,
    expected_head: ArtifactAuthorityHead,
    observed_at: String,
    grace_until: String,
    claim_token: String,
    plan_digest: ContentDigest,
}

impl ArtifactDeletePlan {
    /// Returns the exact generation selected by this plan.
    #[must_use]
    pub const fn object(&self) -> &ArtifactObjectIdentity {
        &self.object
    }

    /// Returns the independently obtained full head used by this plan.
    #[must_use]
    pub const fn expected_head(&self) -> &ArtifactAuthorityHead {
        &self.expected_head
    }

    /// Returns the explicit observation time.
    #[must_use]
    pub fn observed_at(&self) -> &str {
        &self.observed_at
    }

    /// Returns the exact grace deadline.
    #[must_use]
    pub fn grace_until(&self) -> &str {
        &self.grace_until
    }

    /// Returns the unique deterministic claim token.
    #[must_use]
    pub fn claim_token(&self) -> &str {
        &self.claim_token
    }

    /// Returns the delete-plan-specific digest.
    #[must_use]
    pub const fn plan_digest(&self) -> &ContentDigest {
        &self.plan_digest
    }
}

/// Explicit fake delete adapter outcome; ambiguous is never treated as success.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeDeleteOutcome {
    /// The exact claimed generation is verified absent.
    VerifiedDeleted,
    /// The attempted effect is verified not to have changed the exact bytes.
    VerifiedNoEffect,
    /// Transaction or adapter outcome is ambiguous.
    Unknown,
}

/// Exact reconciliation result derived from metadata plus owned-byte evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactReconciliationResult {
    /// Exact bytes remain present and verify against the claimed identity.
    VerifiedAvailable,
    /// Exact bytes are verified absent.
    VerifiedDeleted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredRead {
    head: ArtifactReadHead,
}

/// Root-composed evidence installed into every current object head.
///
/// Lifecycle mutation remains isolated and I/O-free. The root facade computes
/// authoritative quota and command heads, then atomically refreshes all
/// lifecycle receipts with this evidence before exposing a terminal result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactIntegratedHeadEvidence {
    object: ArtifactObjectIdentity,
    lifecycle_revision: ArtifactRevision,
    task_quota_head_digest: ContentDigest,
    project_quota_head_digest: ContentDigest,
    store_quota_head_digest: ContentDigest,
    staging_quota_head_digest: ContentDigest,
    command_high_water: ArtifactCounter,
    command_tail_digest: ContentDigest,
}

impl ArtifactIntegratedHeadEvidence {
    /// Constructs one exact object/revision-bound root projection.
    ///
    /// # Errors
    ///
    /// Rejects an empty command history. Remaining digest validation is
    /// performed by the shared `ArtifactObjectHead` contract during refresh.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        object: ArtifactObjectIdentity,
        lifecycle_revision: ArtifactRevision,
        task_quota_head_digest: ContentDigest,
        project_quota_head_digest: ContentDigest,
        store_quota_head_digest: ContentDigest,
        staging_quota_head_digest: ContentDigest,
        command_high_water: ArtifactCounter,
        command_tail_digest: ContentDigest,
    ) -> Result<Self, ArtifactLifecycleError> {
        if command_high_water.get() == 0 {
            return Err(ArtifactLifecycleError::IntegratedEvidenceMismatch);
        }
        Ok(Self {
            object,
            lifecycle_revision,
            task_quota_head_digest,
            project_quota_head_digest,
            store_quota_head_digest,
            staging_quota_head_digest,
            command_high_water,
            command_tail_digest,
        })
    }

    #[must_use]
    pub(crate) const fn object(&self) -> &ArtifactObjectIdentity {
        &self.object
    }

    #[must_use]
    pub(crate) const fn lifecycle_revision(&self) -> ArtifactRevision {
        self.lifecycle_revision
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObjectRecord {
    identity: ArtifactObjectIdentity,
    byte_length: u64,
    bundle_total_declared_bytes: u64,
    revision: u64,
    availability: ArtifactAvailability,
    delete_status: ArtifactDeleteStatus,
    delete_claim_token: Option<String>,
    references: HashMap<String, ArtifactReferenceHead>,
    reads: HashMap<String, StoredRead>,
    sweep_not_before: String,
    integrated_head_evidence: Option<ArtifactIntegratedHeadEvidence>,
    last_receipt: Option<ArtifactAuthorityReceipt>,
}

/// Deterministic in-memory semantic fake; it is not durable truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactLifecycleState {
    limits: ArtifactStoreLimits,
    objects: HashMap<String, ObjectRecord>,
    last_generations: HashMap<String, u64>,
    terminal_reference_ids: HashSet<String>,
    terminal_read_ids: HashSet<String>,
}

impl Default for ArtifactLifecycleState {
    fn default() -> Self {
        Self::new(ArtifactStoreLimits::hard_maximums())
    }
}

impl ArtifactLifecycleState {
    /// Constructs one fake with an immutable lower-or-equal limit snapshot.
    #[must_use]
    pub fn new(limits: ArtifactStoreLimits) -> Self {
        Self {
            limits,
            objects: HashMap::new(),
            last_generations: HashMap::new(),
            terminal_reference_ids: HashSet::new(),
            terminal_read_ids: HashSet::new(),
        }
    }

    /// Returns the number of currently represented logical object keys.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Returns this fake's immutable limit snapshot.
    #[must_use]
    pub const fn limits(&self) -> ArtifactStoreLimits {
        self.limits
    }

    /// Returns every represented current generation in canonical identity order.
    #[must_use]
    pub(crate) fn current_object_identities(&self) -> Vec<ArtifactObjectIdentity> {
        let mut rows = self
            .objects
            .values()
            .map(|record| (identity_token(&record.identity), record.identity.clone()))
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        rows.into_iter().map(|(_, identity)| identity).collect()
    }

    /// Discovers every exact project/task scope retained by references or reads.
    #[must_use]
    pub(crate) fn current_task_scopes(&self) -> Vec<(ProjectId, TaskId)> {
        let mut scopes = HashMap::<(String, String), (ProjectId, TaskId)>::new();
        for record in self.objects.values() {
            for reference in record.references.values() {
                let binding = reference.manifest().binding();
                scopes.insert(
                    (
                        binding.project_id().as_str().to_owned(),
                        binding.task_id().as_str().to_owned(),
                    ),
                    (binding.project_id().clone(), binding.task_id().clone()),
                );
            }
            for read in record.reads.values() {
                let binding = read.head.authority().receipt().binding();
                scopes.insert(
                    (
                        binding.project_id().as_str().to_owned(),
                        binding.task_id().as_str().to_owned(),
                    ),
                    (binding.project_id().clone(), binding.task_id().clone()),
                );
            }
        }
        let mut rows = scopes.into_iter().collect::<Vec<_>>();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        rows.into_iter().map(|(_, scope)| scope).collect()
    }

    /// Discovers every project namespace represented by current generations.
    #[must_use]
    pub(crate) fn current_project_scopes(&self) -> Vec<ProjectId> {
        let mut scopes = self
            .objects
            .values()
            .map(|record| {
                (
                    record.identity.key().project_id().as_str().to_owned(),
                    record.identity.key().project_id().clone(),
                )
            })
            .collect::<HashMap<_, _>>()
            .into_iter()
            .collect::<Vec<_>>();
        scopes.sort_by(|left, right| left.0.cmp(&right.0));
        scopes
            .into_iter()
            .map(|(_, project_id)| project_id)
            .collect()
    }

    /// Derives exact object quota records from verified lifecycle metadata.
    ///
    /// No count, Boolean, state, or byte metric is accepted from the caller.
    ///
    /// # Errors
    ///
    /// Rejects signed-BIGINT overflow or an internally invalid quota record.
    pub(crate) fn quota_object_records(
        &self,
    ) -> Result<Vec<ArtifactObjectQuotaRecord>, ArtifactLifecycleError> {
        let mut rows = Vec::with_capacity(self.objects.len());
        for record in self.objects.values() {
            let max_field_bytes = usize_to_i64(canonical_max_string_bytes(
                &object_record_canonical_value(record)?,
            ))?;
            let bundle_entries = record
                .references
                .values()
                .filter_map(|reference| reference.manifest().bundle())
                .map(|bundle| bundle.entry_count().get())
                .max()
                .unwrap_or(0);
            let bundle_depth = record
                .references
                .values()
                .filter_map(|reference| reference.manifest().bundle())
                .map(|bundle| bundle.max_depth().get())
                .max()
                .unwrap_or(0);
            let quota_record = ArtifactObjectQuotaRecord::new(
                record.identity.clone(),
                u64_to_i64(accounted_object_bytes(record)?)?,
                max_field_bytes,
                u64_to_i64(bundle_entries)?,
                u64_to_i64(bundle_depth)?,
                object_quota_state(record.availability),
            )
            .map_err(|_| ArtifactLifecycleError::InvalidContract)?;
            rows.push((identity_token(&record.identity), quota_record));
        }
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(rows.into_iter().map(|(_, record)| record).collect())
    }

    /// Derives exact immutable-reference quota records and canonical sizes.
    ///
    /// # Errors
    ///
    /// Rejects canonicalization, signed-BIGINT overflow, or invalid identity.
    pub(crate) fn quota_reference_records(
        &self,
    ) -> Result<Vec<ArtifactReferenceQuotaRecord>, ArtifactLifecycleError> {
        let mut rows = Vec::new();
        for record in self.objects.values() {
            for reference in record.references.values() {
                let manifest = reference.manifest();
                let identity = ArtifactReferenceIdentity::new(
                    manifest.binding().task_id().clone(),
                    record.identity.key().clone(),
                    manifest.reference_id(),
                )
                .map_err(|_| ArtifactLifecycleError::InvalidContract)?;
                let key = format!(
                    "{}:{}:{}",
                    identity.project_id().as_str(),
                    identity.task_id().as_str(),
                    identity.value()
                );
                let quota_record = ArtifactReferenceQuotaRecord::new(
                    identity,
                    record.identity.clone(),
                    u64_to_i64(artifact_manifest_canonical_len(manifest)?)?,
                    match reference.status() {
                        ArtifactReferenceStatus::Active => ArtifactReferenceQuotaState::Active,
                        ArtifactReferenceStatus::Released => ArtifactReferenceQuotaState::Released,
                    },
                )
                .map_err(|_| ArtifactLifecycleError::InvalidContract)?;
                rows.push((key, quota_record));
            }
        }
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(rows.into_iter().map(|(_, record)| record).collect())
    }

    /// Derives exact read-claim quota records from verified stored heads.
    ///
    /// # Errors
    ///
    /// Rejects an internally invalid quota identity.
    pub(crate) fn quota_read_records(
        &self,
    ) -> Result<Vec<ArtifactReadQuotaRecord>, ArtifactLifecycleError> {
        let mut rows = Vec::new();
        for record in self.objects.values() {
            for read in record.reads.values() {
                let binding = read.head.authority().receipt().binding();
                let identity = ArtifactReadIdentity::new(
                    binding.task_id().clone(),
                    record.identity.clone(),
                    binding.read_claim_id(),
                )
                .map_err(|_| ArtifactLifecycleError::InvalidContract)?;
                let key = format!(
                    "{}:{}:{}:{}",
                    identity.project_id().as_str(),
                    identity.task_id().as_str(),
                    identity.object().generation().get(),
                    identity.value()
                );
                let quota_record = ArtifactReadQuotaRecord::new(
                    identity,
                    record.identity.clone(),
                    match read.head.status() {
                        ArtifactReadStatus::Active => ArtifactReadQuotaState::Active,
                        ArtifactReadStatus::ExpiredSuspect => {
                            ArtifactReadQuotaState::ExpiredSuspect
                        }
                        ArtifactReadStatus::Released => ArtifactReadQuotaState::VerifiedClosed,
                    },
                )
                .with_max_field_bytes(usize_to_i64(canonical_max_string_bytes(
                    &read_head_canonical_value(&read.head),
                ))?)
                .map_err(|_| ArtifactLifecycleError::InvalidContract)?;
                rows.push((key, quota_record));
            }
        }
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(rows.into_iter().map(|(_, record)| record).collect())
    }

    /// Returns the complete canonical metadata state without artifact payloads.
    ///
    /// # Errors
    ///
    /// Rejects canonical ordering or nested metadata projection failures.
    pub(crate) fn canonical_metadata_state(
        &self,
    ) -> Result<CanonicalValue, ArtifactLifecycleError> {
        let mut objects = self
            .objects
            .values()
            .map(object_record_canonical_value)
            .collect::<Result<Vec<_>, _>>()?;
        sort_canonical_values(&mut objects)?;

        let mut generations = self
            .last_generations
            .iter()
            .map(|(key, generation)| {
                CanonicalValue::Object(vec![
                    string_field("object_key", key),
                    string_field("last_generation", generation.to_string()),
                ])
            })
            .collect::<Vec<_>>();
        sort_canonical_values(&mut generations)?;

        let mut terminal_references = self
            .terminal_reference_ids
            .iter()
            .cloned()
            .map(CanonicalValue::String)
            .collect::<Vec<_>>();
        sort_canonical_values(&mut terminal_references)?;
        let mut terminal_reads = self
            .terminal_read_ids
            .iter()
            .cloned()
            .map(CanonicalValue::String)
            .collect::<Vec<_>>();
        sort_canonical_values(&mut terminal_reads)?;

        let limits = CanonicalValue::Object(
            ArtifactLimitKind::ALL
                .into_iter()
                .map(|kind| string_field(kind.as_str(), self.limits.get(kind).to_string()))
                .collect(),
        );
        Ok(CanonicalValue::Object(vec![
            ("limits".to_owned(), limits),
            (
                "last_generations".to_owned(),
                CanonicalValue::Array(generations),
            ),
            ("objects".to_owned(), CanonicalValue::Array(objects)),
            (
                "terminal_read_ids".to_owned(),
                CanonicalValue::Array(terminal_reads),
            ),
            (
                "terminal_reference_ids".to_owned(),
                CanonicalValue::Array(terminal_references),
            ),
        ]))
    }

    /// Hashes the complete raw-byte-free lifecycle metadata state.
    ///
    /// # Errors
    ///
    /// Rejects canonicalization or digest construction failure.
    pub(crate) fn metadata_state_digest(&self) -> Result<ContentDigest, ArtifactLifecycleError> {
        canonical_digest(
            "lattice.artifact.lifecycle-metadata-state",
            self.canonical_metadata_state()?,
        )
    }

    /// Atomically installs root-composed quota/history evidence for every
    /// current object and rebuilds every current receipt.
    ///
    /// Existing nested reference/read projections are preserved. Updating an
    /// unrelated store quota head therefore makes prior object heads stale
    /// without incrementing their lifecycle revision.
    ///
    /// # Errors
    ///
    /// Rejects missing, extra, duplicate, wrong-object, wrong-revision, or
    /// contract-invalid evidence without mutating this state.
    pub(crate) fn refresh_integrated_heads(
        &mut self,
        evidence: Vec<ArtifactIntegratedHeadEvidence>,
    ) -> Result<(), ArtifactLifecycleError> {
        let mut next = self.clone();
        let expected = next.current_object_identities();
        if evidence.len() != expected.len() {
            return Err(ArtifactLifecycleError::IntegratedEvidenceMismatch);
        }
        let mut supplied = HashMap::with_capacity(evidence.len());
        for item in evidence {
            let key = identity_token(item.object());
            if supplied.insert(key, item).is_some() {
                return Err(ArtifactLifecycleError::IntegratedEvidenceMismatch);
            }
        }
        for object in &expected {
            let key = identity_token(object);
            let item = supplied
                .remove(&key)
                .ok_or(ArtifactLifecycleError::IntegratedEvidenceMismatch)?;
            let record = next.record_mut(object)?;
            if item.object() != &record.identity
                || item.lifecycle_revision().get() != record.revision
            {
                return Err(ArtifactLifecycleError::IntegratedEvidenceMismatch);
            }
            record.integrated_head_evidence = Some(item);
        }
        if !supplied.is_empty() {
            return Err(ArtifactLifecycleError::IntegratedEvidenceMismatch);
        }

        for object in &expected {
            let (reference, read) = {
                let prior = next
                    .record(object)?
                    .last_receipt
                    .as_ref()
                    .ok_or(ArtifactLifecycleError::IntegratedEvidenceMismatch)?;
                (prior.reference().cloned(), prior.read().cloned())
            };
            let receipt = next.build_receipt(object, reference, read, "integrated-head-refresh")?;
            next.set_last_receipt(object, receipt)?;
        }
        next.validate_quotas()?;
        *self = next;
        Ok(())
    }

    /// Returns the current full fixed-owner store head for an exact generation.
    ///
    /// # Errors
    ///
    /// Rejects a missing or non-current generation.
    pub fn current_head(
        &self,
        object: &ArtifactObjectIdentity,
    ) -> Result<ArtifactAuthorityHead, ArtifactLifecycleError> {
        let record = self.record(object)?;
        record
            .last_receipt
            .as_ref()
            .map(ArtifactAuthorityReceipt::head)
            .ok_or(ArtifactLifecycleError::ObjectNotFound)
    }

    /// Returns the exact current fixed-owner receipt after root integration.
    ///
    /// # Errors
    ///
    /// Rejects a missing/non-current generation or absent receipt.
    pub(crate) fn current_receipt(
        &self,
        object: &ArtifactObjectIdentity,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        self.record(object)?
            .last_receipt
            .clone()
            .ok_or(ArtifactLifecycleError::ObjectNotFound)
    }

    /// Returns the current object-state projection for an exact generation.
    ///
    /// # Errors
    ///
    /// Rejects missing, non-current, exhausted, or invalid projected state.
    pub fn object_head(
        &self,
        object: &ArtifactObjectIdentity,
    ) -> Result<ArtifactObjectHead, ArtifactLifecycleError> {
        self.build_object_head(self.record(object)?)
    }

    /// Returns one exact retained read head for aggregate command attribution.
    ///
    /// This crate-private query grants no transition authority.
    pub(crate) fn read_head(
        &self,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
    ) -> Result<ArtifactReadHead, ArtifactLifecycleError> {
        self.record(object)?
            .reads
            .get(read_claim_id)
            .map(|read| read.head.clone())
            .ok_or(ArtifactLifecycleError::ReadNotFound)
    }

    /// Publishes exact verified bytes with the immutable initial reference.
    ///
    /// Equal available bytes deduplicate only inside the same project and use
    /// `ADD_REFERENCE`; a deleted key accepts only the next generation and a
    /// new `PUBLISH_INITIAL_REFERENCE` authority. Metadata and fake bytes are
    /// clone-validated and swapped together.
    ///
    /// # Errors
    ///
    /// Rejects any byte, manifest, authority, generation, state, or quota drift.
    pub fn publish(
        &mut self,
        bytes_backend: &mut FakeArtifactBytes,
        manifest: ArtifactReferenceManifest,
        exact_bytes: &[u8],
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next_store = self.clone();
        let mut next_bytes = bytes_backend.clone();
        let receipt =
            next_store.publish_inner(&mut next_bytes, manifest, exact_bytes, authorities)?;
        *self = next_store;
        *bytes_backend = next_bytes;
        Ok(receipt)
    }

    /// Adds one immutable reference to the exact available generation.
    ///
    /// # Errors
    ///
    /// Rejects stale authority, terminal identity, unavailable state, or quota drift.
    pub fn add_reference(
        &mut self,
        manifest: ArtifactReferenceManifest,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next = self.clone();
        let receipt = next.add_reference_inner(manifest, authorities)?;
        *self = next;
        Ok(receipt)
    }

    /// Releases one exact reference terminally.
    ///
    /// # Errors
    ///
    /// Rejects stale authority, scope drift, missing references, or unavailable state.
    pub fn release_reference(
        &mut self,
        object: &ArtifactObjectIdentity,
        reference_id: &str,
        authority: ArtifactReferenceAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next = self.clone();
        let receipt = next.release_reference_inner(object, reference_id, authority, authorities)?;
        *self = next;
        Ok(receipt)
    }

    /// Acquires one exact, at-most-15-minute normal read claim.
    ///
    /// # Errors
    ///
    /// Rejects stale authority, scope drift, invalid lease, state, or quota.
    #[allow(clippy::too_many_arguments)]
    pub fn acquire_read(
        &mut self,
        object: &ArtifactObjectIdentity,
        holder_id: &str,
        acquired_at: &str,
        expires_at: &str,
        authority: ArtifactReadAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next = self.clone();
        let receipt = next.acquire_read_inner(
            object,
            holder_id,
            acquired_at,
            expires_at,
            authority,
            authorities,
        )?;
        *self = next;
        Ok(receipt)
    }

    /// Releases an active read through an exact typed owner transition.
    ///
    /// # Errors
    ///
    /// Rejects stale authority, scope drift, or non-active read state.
    pub fn release_read(
        &mut self,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        authority: ArtifactReadAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next = self.clone();
        let receipt =
            next.release_read_inner(object, read_claim_id, authority, authorities, None)?;
        *self = next;
        Ok(receipt)
    }

    /// Marks an elapsed active lease `EXPIRED_SUSPECT` without freeing quota.
    ///
    /// # Errors
    ///
    /// Rejects malformed time, early expiry, or non-active read state.
    pub fn mark_read_expired_suspect(
        &mut self,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        observed_at: &str,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next = self.clone();
        let receipt = next.mark_read_expired_suspect_inner(object, read_claim_id, observed_at)?;
        *self = next;
        Ok(receipt)
    }

    /// Releases an expired-suspect read only with exact closure evidence and
    /// the typed release-owner transition.
    ///
    /// # Errors
    ///
    /// Rejects stale authority or inexact holder-closure evidence.
    pub fn reconcile_expired_read(
        &mut self,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        authority: ArtifactReadAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
        evidence: &ArtifactReadClosureEvidencePair,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next = self.clone();
        let receipt = next.release_read_inner(
            object,
            read_claim_id,
            authority,
            authorities,
            Some(evidence),
        )?;
        *self = next;
        Ok(receipt)
    }

    /// Returns a copy only after canonical-time, active-claim, length, and
    /// digest verification.
    ///
    /// Reaching the exact lease deadline atomically advances the read to
    /// `EXPIRED_SUSPECT` before returning a typed denial.
    ///
    /// # Errors
    ///
    /// Rejects missing claims, unavailable state, or missing/corrupt bytes.
    pub fn read_verified(
        &mut self,
        bytes_backend: &FakeArtifactBytes,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        observed_at: &str,
    ) -> Result<Vec<u8>, ArtifactLifecycleError> {
        let observed = parse_canonical_time(observed_at)?;
        {
            let record = self.record(object)?;
            if record.availability != ArtifactAvailability::Available {
                return Err(ArtifactLifecycleError::ObjectUnavailable);
            }
            let read = record
                .reads
                .get(read_claim_id)
                .ok_or(ArtifactLifecycleError::ReadNotFound)?;
            if read.head.status() != ArtifactReadStatus::Active {
                return Err(ArtifactLifecycleError::ReadTerminal);
            }
            let acquired = parse_time(read.head.acquired_at())?;
            let expires = parse_time(read.head.expires_at())?;
            if observed < acquired {
                return Err(ArtifactLifecycleError::InvalidReadEvidence);
            }
            if observed >= expires {
                self.mark_read_expired_suspect(object, read_claim_id, observed_at)?;
                return Err(ArtifactLifecycleError::ReadExpiredSuspect);
            }
        }
        let record = self.record(object)?;
        let bytes = bytes_backend
            .get(object)
            .ok_or(ArtifactLifecycleError::MissingBytes)?;
        verify_exact_bytes(object, record.byte_length, bytes)
            .map_err(|_| ArtifactLifecycleError::CorruptBytes)?;
        Ok(bytes.to_vec())
    }

    /// Produces a read-only plan from an exact independently obtained full
    /// Artifact Store head. No metadata or byte state changes.
    ///
    /// # Errors
    ///
    /// Rejects stale heads, blockers, unavailable state, or active retention.
    pub fn plan_delete(
        &self,
        object: &ArtifactObjectIdentity,
        expected_head: &ArtifactAuthorityHead,
        observed_at: &str,
        grace_until: &str,
    ) -> Result<ArtifactDeletePlan, ArtifactLifecycleError> {
        let record = self.record(object)?;
        if record.availability != ArtifactAvailability::Available {
            return Err(ArtifactLifecycleError::ObjectUnavailable);
        }
        if &self.current_head(object)? != expected_head {
            return Err(ArtifactLifecycleError::StalePlan);
        }
        if active_reference_count(record) != 0 || blocking_read_count(record) != 0 {
            return Err(ArtifactLifecycleError::DeleteBlocked);
        }
        let observed = parse_time(observed_at)?;
        let grace = parse_time(grace_until)?;
        let retention = parse_time(&record.sweep_not_before)?;
        if observed < grace || observed < retention {
            return Err(ArtifactLifecycleError::RetentionActive);
        }
        let plan_digest = canonical_digest(
            "lattice.artifact.delete-plan",
            CanonicalValue::Object(vec![
                string_field("object", identity_token(object)),
                string_field(
                    "expected_receipt_digest",
                    expected_head.receipt_digest().as_str(),
                ),
                string_field("observed_at", observed_at),
                string_field("grace_until", grace_until),
            ]),
        )?;
        let claim_token_digest = canonical_digest(
            "lattice.artifact.delete-claim",
            CanonicalValue::Object(vec![
                string_field("plan_digest", plan_digest.as_str()),
                string_field("generation", object.generation().get().to_string()),
            ]),
        )?;
        let claim_token = claim_token_digest.as_str().to_owned();
        validate_configured_field_bytes(self.limits, &CanonicalValue::String(claim_token.clone()))?;
        Ok(ArtifactDeletePlan {
            object: object.clone(),
            expected_head: expected_head.clone(),
            observed_at: observed_at.to_owned(),
            grace_until: grace_until.to_owned(),
            claim_token,
            plan_digest,
        })
    }

    /// Claims one exact plan. An exact token retry returns the identical
    /// terminal receipt before current-head or time revalidation.
    ///
    /// # Errors
    ///
    /// Rejects stale plans, authority drift, blockers, or invalid state.
    pub fn claim_delete(
        &mut self,
        plan: &ArtifactDeletePlan,
        authority: &ArtifactSweepAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        if let Ok(record) = self.record(&plan.object)
            && record.availability == ArtifactAvailability::DeleteClaimed
            && record.delete_claim_token.as_deref() == Some(plan.claim_token())
        {
            return record
                .last_receipt
                .clone()
                .ok_or(ArtifactLifecycleError::ClaimTokenMismatch);
        }
        let mut next = self.clone();
        let receipt = next.claim_delete_inner(plan, authority, authorities)?;
        *self = next;
        Ok(receipt)
    }

    /// Applies one exact fake adapter outcome to a claimed generation.
    ///
    /// # Errors
    ///
    /// Rejects wrong claims, illegal state, or contradictory no-effect evidence.
    pub fn apply_delete_outcome(
        &mut self,
        bytes_backend: &mut FakeArtifactBytes,
        object: &ArtifactObjectIdentity,
        claim_token: &str,
        outcome: FakeDeleteOutcome,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next_store = self.clone();
        let mut next_bytes = bytes_backend.clone();
        let receipt =
            next_store.apply_delete_outcome_inner(&mut next_bytes, object, claim_token, outcome)?;
        *self = next_store;
        *bytes_backend = next_bytes;
        Ok(receipt)
    }

    /// Resolves `RECONCILIATION_REQUIRED` only from exact owned-byte evidence.
    ///
    /// # Errors
    ///
    /// Rejects wrong claims, illegal state, or contradictory byte evidence.
    pub fn reconcile_delete(
        &mut self,
        bytes_backend: &FakeArtifactBytes,
        object: &ArtifactObjectIdentity,
        claim_token: &str,
        result: ArtifactReconciliationResult,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let mut next = self.clone();
        let receipt = next.reconcile_delete_inner(bytes_backend, object, claim_token, result)?;
        *self = next;
        Ok(receipt)
    }

    fn publish_inner(
        &mut self,
        bytes_backend: &mut FakeArtifactBytes,
        manifest: ArtifactReferenceManifest,
        exact_bytes: &[u8],
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        self.validate_manifest(&manifest)?;
        verify_exact_bytes(manifest.object(), manifest.byte_length().get(), exact_bytes)?;
        self.validate_staging_bounds(manifest.binding().task_id().as_str(), exact_bytes.len())?;
        let key = object_key_token(manifest.object().key());
        let current_generation = self
            .last_generations
            .get(&key)
            .copied()
            .map(ArtifactGeneration::new)
            .transpose()
            .map_err(|_| ArtifactLifecycleError::CounterExhausted)?;
        let expected_generation = next_artifact_generation(current_generation)?.get();
        if let Some(existing) = self.objects.get(&key) {
            if existing.availability == ArtifactAvailability::Available
                && existing.identity == *manifest.object()
            {
                let stored = bytes_backend
                    .get(manifest.object())
                    .ok_or(ArtifactLifecycleError::MissingBytes)?;
                verify_exact_bytes(manifest.object(), existing.byte_length, stored)?;
                return self.add_reference_inner(manifest, authorities);
            }
            if existing.availability != ArtifactAvailability::Deleted {
                return Err(ArtifactLifecycleError::ObjectUnavailable);
            }
        }
        if manifest.object().generation().get() != expected_generation {
            return Err(ArtifactLifecycleError::GenerationMismatch);
        }
        Self::validate_reference_authority(
            &manifest,
            ArtifactReferenceAuthorityAction::PublishInitialReference,
            authorities,
        )?;
        let terminal = reference_terminal_token(&manifest);
        if self.terminal_reference_ids.contains(&terminal) {
            return Err(ArtifactLifecycleError::ReferenceTerminal);
        }
        let reference = build_reference_head(
            manifest.clone(),
            manifest.creation_authority().clone(),
            1,
            ArtifactReferenceStatus::Active,
        )?;
        let identity = manifest.object().clone();
        let mut references = HashMap::new();
        references.insert(manifest.reference_id().to_owned(), reference.clone());
        let record = ObjectRecord {
            identity: identity.clone(),
            byte_length: manifest.byte_length().get(),
            bundle_total_declared_bytes: manifest
                .bundle()
                .map_or(0, |bundle| bundle.total_declared_bytes().get()),
            revision: 1,
            availability: ArtifactAvailability::Available,
            delete_status: ArtifactDeleteStatus::NotClaimed,
            delete_claim_token: None,
            references,
            reads: HashMap::new(),
            sweep_not_before: manifest.retention_until().to_owned(),
            integrated_head_evidence: None,
            last_receipt: None,
        };
        self.objects.insert(key.clone(), record);
        self.last_generations
            .insert(key, identity.generation().get());
        self.terminal_reference_ids.insert(terminal);
        self.validate_quotas()?;
        let receipt = self.build_receipt(&identity, Some(reference), None, "publish")?;
        self.set_last_receipt(&identity, receipt.clone())?;
        bytes_backend.insert(&identity, exact_bytes);
        Ok(receipt)
    }

    fn add_reference_inner(
        &mut self,
        manifest: ArtifactReferenceManifest,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        self.validate_manifest(&manifest)?;
        Self::validate_reference_authority(
            &manifest,
            ArtifactReferenceAuthorityAction::AddReference,
            authorities,
        )?;
        let terminal = reference_terminal_token(&manifest);
        if self.terminal_reference_ids.contains(&terminal) {
            return Err(ArtifactLifecycleError::ReferenceTerminal);
        }
        let object = manifest.object().clone();
        let reference_id = manifest.reference_id().to_owned();
        let creation_authority = manifest.creation_authority().clone();
        {
            let record = self.record_mut(&object)?;
            if record.availability != ArtifactAvailability::Available {
                return Err(ArtifactLifecycleError::ObjectUnavailable);
            }
            if manifest.byte_length().get() != record.byte_length {
                return Err(ArtifactLifecycleError::LengthMismatch);
            }
            if manifest
                .bundle()
                .map_or(0, |bundle| bundle.total_declared_bytes().get())
                != record.bundle_total_declared_bytes
            {
                return Err(ArtifactLifecycleError::ManifestMismatch);
            }
            record.revision = checked_increment(record.revision)?;
            record.integrated_head_evidence = None;
            let reference = build_reference_head(
                manifest,
                creation_authority,
                record.revision,
                ArtifactReferenceStatus::Active,
            )?;
            if parse_time(reference.manifest().retention_until())?
                > parse_time(&record.sweep_not_before)?
            {
                reference
                    .manifest()
                    .retention_until()
                    .clone_into(&mut record.sweep_not_before);
            }
            record.references.insert(reference_id.clone(), reference);
        }
        self.terminal_reference_ids.insert(terminal);
        self.validate_quotas()?;
        let reference = self
            .record(&object)?
            .references
            .get(&reference_id)
            .cloned()
            .ok_or(ArtifactLifecycleError::ReferenceNotFound)?;
        let receipt = self.build_receipt(&object, Some(reference), None, "add-reference")?;
        self.set_last_receipt(&object, receipt.clone())?;
        Ok(receipt)
    }

    fn release_reference_inner(
        &mut self,
        object: &ArtifactObjectIdentity,
        reference_id: &str,
        authority: ArtifactReferenceAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        authorities.verify_reference(&authority)?;
        let binding = authority.receipt().binding();
        if binding.action() != ArtifactReferenceAuthorityAction::ReleaseReference {
            return Err(ArtifactLifecycleError::AuthorityActionMismatch);
        }
        if binding.object() != object || binding.reference_id() != reference_id {
            return Err(ArtifactLifecycleError::AuthorityScopeMismatch);
        }
        let released;
        {
            let record = self.record_mut(object)?;
            if record.availability != ArtifactAvailability::Available {
                return Err(ArtifactLifecycleError::ObjectUnavailable);
            }
            let current = record
                .references
                .get(reference_id)
                .ok_or(ArtifactLifecycleError::ReferenceNotFound)?;
            if current.status() != ArtifactReferenceStatus::Active {
                return Err(ArtifactLifecycleError::ReferenceTerminal);
            }
            if binding.project_id() != current.manifest().binding().project_id()
                || binding.task_id() != current.manifest().binding().task_id()
            {
                return Err(ArtifactLifecycleError::AuthorityScopeMismatch);
            }
            record.revision = checked_increment(record.revision)?;
            record.integrated_head_evidence = None;
            released = build_reference_head(
                current.manifest().clone(),
                authority,
                record.revision,
                ArtifactReferenceStatus::Released,
            )?;
            record
                .references
                .insert(reference_id.to_owned(), released.clone());
        }
        self.validate_quotas()?;
        let receipt = self.build_receipt(object, Some(released), None, "release-reference")?;
        self.set_last_receipt(object, receipt.clone())?;
        Ok(receipt)
    }

    #[allow(clippy::too_many_arguments)]
    fn acquire_read_inner(
        &mut self,
        object: &ArtifactObjectIdentity,
        holder_id: &str,
        acquired_at: &str,
        expires_at: &str,
        authority: ArtifactReadAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        authorities.verify_read(&authority)?;
        let binding = authority.receipt().binding();
        if binding.action() != ArtifactReadAuthorityAction::AcquireRead {
            return Err(ArtifactLifecycleError::AuthorityActionMismatch);
        }
        if binding.object() != object {
            return Err(ArtifactLifecycleError::AuthorityScopeMismatch);
        }
        let acquired = parse_time(acquired_at)?;
        let expires = parse_time(expires_at)?;
        let lease = expires - acquired;
        if lease <= Duration::ZERO || lease.whole_seconds() > MAX_READ_LEASE_SECONDS {
            return Err(ArtifactLifecycleError::InvalidReadLease);
        }
        let read_claim_id = binding.read_claim_id().to_owned();
        let terminal = read_terminal_token(object, &read_claim_id);
        if self.terminal_read_ids.contains(&terminal) {
            return Err(ArtifactLifecycleError::ReadTerminal);
        }
        let read_head;
        {
            let record = self.record_mut(object)?;
            if record.availability != ArtifactAvailability::Available {
                return Err(ArtifactLifecycleError::ObjectUnavailable);
            }
            record.revision = checked_increment(record.revision)?;
            record.integrated_head_evidence = None;
            read_head = build_read_head(
                authority,
                record.revision,
                ArtifactReadStatus::Active,
                holder_id,
                acquired_at,
                expires_at,
            )?;
            record.reads.insert(
                read_claim_id,
                StoredRead {
                    head: read_head.clone(),
                },
            );
        }
        self.terminal_read_ids.insert(terminal);
        self.validate_quotas()?;
        let receipt = self.build_receipt(object, None, Some(read_head), "acquire-read")?;
        self.set_last_receipt(object, receipt.clone())?;
        Ok(receipt)
    }

    fn release_read_inner(
        &mut self,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        authority: ArtifactReadAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
        evidence: Option<&ArtifactReadClosureEvidencePair>,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        authorities.verify_read(&authority)?;
        let binding = authority.receipt().binding();
        if binding.action() != ArtifactReadAuthorityAction::ReleaseRead {
            return Err(ArtifactLifecycleError::AuthorityActionMismatch);
        }
        if binding.object() != object || binding.read_claim_id() != read_claim_id {
            return Err(ArtifactLifecycleError::AuthorityScopeMismatch);
        }
        let released;
        {
            let record = self.record_mut(object)?;
            if record.availability != ArtifactAvailability::Available {
                return Err(ArtifactLifecycleError::ObjectUnavailable);
            }
            let current = record
                .reads
                .get(read_claim_id)
                .ok_or(ArtifactLifecycleError::ReadNotFound)?;
            let current_binding = current.head.authority().receipt().binding();
            if binding.project_id() != current_binding.project_id()
                || binding.task_id() != current_binding.task_id()
                || binding.owner_kind() != current_binding.owner_kind()
                || binding.producer_id() != current_binding.producer_id()
                || binding.producer_version() != current_binding.producer_version()
                || binding.runtime() != current_binding.runtime()
            {
                return Err(ArtifactLifecycleError::AuthorityScopeMismatch);
            }
            match (current.head.status(), evidence) {
                (ArtifactReadStatus::Active, None) => {}
                (ArtifactReadStatus::ExpiredSuspect, Some(exact)) => {
                    validate_read_closure_evidence(
                        object,
                        read_claim_id,
                        &current.head,
                        exact,
                        authorities,
                    )?;
                }
                (ArtifactReadStatus::Released, _) => {
                    return Err(ArtifactLifecycleError::ReadTerminal);
                }
                _ => return Err(ArtifactLifecycleError::InvalidReadEvidence),
            }
            record.revision = checked_increment(record.revision)?;
            record.integrated_head_evidence = None;
            released = build_read_head(
                authority,
                record.revision,
                ArtifactReadStatus::Released,
                current.head.holder_id(),
                current.head.acquired_at(),
                current.head.expires_at(),
            )?;
            record.reads.insert(
                read_claim_id.to_owned(),
                StoredRead {
                    head: released.clone(),
                },
            );
        }
        self.validate_quotas()?;
        let receipt = self.build_receipt(object, None, Some(released), "release-read")?;
        self.set_last_receipt(object, receipt.clone())?;
        Ok(receipt)
    }

    fn mark_read_expired_suspect_inner(
        &mut self,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        observed_at: &str,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let observed = parse_time(observed_at)?;
        let suspect;
        {
            let record = self.record_mut(object)?;
            if record.availability != ArtifactAvailability::Available {
                return Err(ArtifactLifecycleError::ObjectUnavailable);
            }
            let current = record
                .reads
                .get(read_claim_id)
                .ok_or(ArtifactLifecycleError::ReadNotFound)?;
            if current.head.status() != ArtifactReadStatus::Active {
                return Err(ArtifactLifecycleError::ReadTerminal);
            }
            if observed < parse_time(current.head.expires_at())? {
                return Err(ArtifactLifecycleError::InvalidReadEvidence);
            }
            record.revision = checked_increment(record.revision)?;
            record.integrated_head_evidence = None;
            suspect = build_read_head(
                current.head.authority().clone(),
                record.revision,
                ArtifactReadStatus::ExpiredSuspect,
                current.head.holder_id(),
                current.head.acquired_at(),
                current.head.expires_at(),
            )?;
            record.reads.insert(
                read_claim_id.to_owned(),
                StoredRead {
                    head: suspect.clone(),
                },
            );
        }
        self.validate_quotas()?;
        let receipt = self.build_receipt(object, None, Some(suspect), "expire-read-suspect")?;
        self.set_last_receipt(object, receipt.clone())?;
        Ok(receipt)
    }

    fn claim_delete_inner(
        &mut self,
        plan: &ArtifactDeletePlan,
        authority: &ArtifactSweepAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        validate_configured_field_bytes(
            self.limits,
            &CanonicalValue::String(plan.claim_token.clone()),
        )?;
        authorities.verify_sweep(authority)?;
        let binding = authority.receipt().binding();
        if binding.object() != &plan.object {
            return Err(ArtifactLifecycleError::AuthorityScopeMismatch);
        }
        let current = self.current_head(&plan.object)?;
        if current != plan.expected_head {
            return Err(ArtifactLifecycleError::StalePlan);
        }
        let object_head = self.object_head(&plan.object)?;
        if binding.zero_reference_set_digest() != object_head.active_reference_set_digest()
            || binding.zero_read_set_digest() != object_head.active_read_set_digest()
            || binding.quota_projection_digest() != object_head.project_quota_projection_digest()
            || binding.retention_observed_at() != plan.observed_at()
            || binding.grace_until() != plan.grace_until()
        {
            return Err(ArtifactLifecycleError::AuthorityScopeMismatch);
        }
        let record = self.record(&plan.object)?;
        if record.availability != ArtifactAvailability::Available {
            return Err(ArtifactLifecycleError::ObjectUnavailable);
        }
        if active_reference_count(record) != 0 || blocking_read_count(record) != 0 {
            return Err(ArtifactLifecycleError::DeleteBlocked);
        }
        {
            let record = self.record_mut(&plan.object)?;
            record.revision = checked_increment(record.revision)?;
            record.integrated_head_evidence = None;
            record.availability = ArtifactAvailability::DeleteClaimed;
            record.delete_status = ArtifactDeleteStatus::Claimed;
            record.delete_claim_token = Some(plan.claim_token.clone());
        }
        self.validate_quotas()?;
        let receipt = self.build_receipt(&plan.object, None, None, "claim-delete")?;
        self.set_last_receipt(&plan.object, receipt.clone())?;
        Ok(receipt)
    }

    fn apply_delete_outcome_inner(
        &mut self,
        bytes_backend: &mut FakeArtifactBytes,
        object: &ArtifactObjectIdentity,
        claim_token: &str,
        outcome: FakeDeleteOutcome,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let current = self.record(object)?;
        if current.availability != ArtifactAvailability::DeleteClaimed {
            return Err(ArtifactLifecycleError::InvalidDeleteOutcome);
        }
        if current.delete_claim_token.as_deref() != Some(claim_token) {
            return Err(ArtifactLifecycleError::ClaimTokenMismatch);
        }
        if outcome == FakeDeleteOutcome::VerifiedNoEffect {
            let exact = bytes_backend
                .get(object)
                .ok_or(ArtifactLifecycleError::ReconciliationMismatch)?;
            verify_exact_bytes(object, current.byte_length, exact)
                .map_err(|_| ArtifactLifecycleError::ReconciliationMismatch)?;
        }
        {
            let record = self.record_mut(object)?;
            record.revision = checked_increment(record.revision)?;
            record.integrated_head_evidence = None;
            match outcome {
                FakeDeleteOutcome::VerifiedDeleted => {
                    record.availability = ArtifactAvailability::Deleted;
                    record.delete_status = ArtifactDeleteStatus::VerifiedDeleted;
                    bytes_backend.remove(object);
                }
                FakeDeleteOutcome::VerifiedNoEffect => {
                    record.availability = ArtifactAvailability::Available;
                    record.delete_status = ArtifactDeleteStatus::VerifiedNoEffect;
                }
                FakeDeleteOutcome::Unknown => {
                    record.availability = ArtifactAvailability::ReconciliationRequired;
                    record.delete_status = ArtifactDeleteStatus::ReconciliationRequired;
                }
            }
        }
        self.validate_quotas()?;
        let operation = match outcome {
            FakeDeleteOutcome::VerifiedDeleted => "delete-result-deleted",
            FakeDeleteOutcome::VerifiedNoEffect => "delete-result-no-effect",
            FakeDeleteOutcome::Unknown => "delete-result-unknown",
        };
        let receipt = self.build_receipt(object, None, None, operation)?;
        self.set_last_receipt(object, receipt.clone())?;
        Ok(receipt)
    }

    fn reconcile_delete_inner(
        &mut self,
        bytes_backend: &FakeArtifactBytes,
        object: &ArtifactObjectIdentity,
        claim_token: &str,
        result: ArtifactReconciliationResult,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let current = self.record(object)?;
        if current.availability != ArtifactAvailability::ReconciliationRequired {
            return Err(ArtifactLifecycleError::InvalidDeleteOutcome);
        }
        if current.delete_claim_token.as_deref() != Some(claim_token) {
            return Err(ArtifactLifecycleError::ClaimTokenMismatch);
        }
        match result {
            ArtifactReconciliationResult::VerifiedAvailable => {
                let exact = bytes_backend
                    .get(object)
                    .ok_or(ArtifactLifecycleError::ReconciliationMismatch)?;
                verify_exact_bytes(object, current.byte_length, exact)
                    .map_err(|_| ArtifactLifecycleError::ReconciliationMismatch)?;
            }
            ArtifactReconciliationResult::VerifiedDeleted => {
                if bytes_backend.get(object).is_some() {
                    return Err(ArtifactLifecycleError::ReconciliationMismatch);
                }
            }
        }
        {
            let record = self.record_mut(object)?;
            record.revision = checked_increment(record.revision)?;
            record.integrated_head_evidence = None;
            match result {
                ArtifactReconciliationResult::VerifiedAvailable => {
                    record.availability = ArtifactAvailability::Available;
                    record.delete_status = ArtifactDeleteStatus::VerifiedNoEffect;
                }
                ArtifactReconciliationResult::VerifiedDeleted => {
                    record.availability = ArtifactAvailability::Deleted;
                    record.delete_status = ArtifactDeleteStatus::VerifiedDeleted;
                }
            }
        }
        self.validate_quotas()?;
        let operation = match result {
            ArtifactReconciliationResult::VerifiedAvailable => "reconcile-available",
            ArtifactReconciliationResult::VerifiedDeleted => "reconcile-deleted",
        };
        let receipt = self.build_receipt(object, None, None, operation)?;
        self.set_last_receipt(object, receipt.clone())?;
        Ok(receipt)
    }

    fn record(
        &self,
        object: &ArtifactObjectIdentity,
    ) -> Result<&ObjectRecord, ArtifactLifecycleError> {
        let record = self
            .objects
            .get(&object_key_token(object.key()))
            .ok_or(ArtifactLifecycleError::ObjectNotFound)?;
        if record.identity != *object {
            return Err(ArtifactLifecycleError::GenerationMismatch);
        }
        Ok(record)
    }

    fn record_mut(
        &mut self,
        object: &ArtifactObjectIdentity,
    ) -> Result<&mut ObjectRecord, ArtifactLifecycleError> {
        let record = self
            .objects
            .get_mut(&object_key_token(object.key()))
            .ok_or(ArtifactLifecycleError::ObjectNotFound)?;
        if record.identity != *object {
            return Err(ArtifactLifecycleError::GenerationMismatch);
        }
        Ok(record)
    }

    fn set_last_receipt(
        &mut self,
        object: &ArtifactObjectIdentity,
        receipt: ArtifactAuthorityReceipt,
    ) -> Result<(), ArtifactLifecycleError> {
        self.record_mut(object)?.last_receipt = Some(receipt);
        Ok(())
    }

    fn validate_manifest(
        &self,
        manifest: &ArtifactReferenceManifest,
    ) -> Result<(), ArtifactLifecycleError> {
        if manifest.byte_length().get() > self.limits.max_object_bytes() {
            return Err(ArtifactLifecycleError::LimitExceeded {
                field: "max_object_bytes",
            });
        }
        if artifact_manifest_canonical_len(manifest)? > self.limits.max_manifest_bytes() {
            return Err(ArtifactLifecycleError::LimitExceeded {
                field: "max_manifest_bytes",
            });
        }
        if artifact_manifest_digest(manifest)? != *manifest.manifest_digest() {
            return Err(ArtifactLifecycleError::ManifestMismatch);
        }
        let expected_limits = self
            .limits
            .limit_snapshot_digest()
            .map_err(|_| ArtifactLifecycleError::Canonicalization)?;
        if manifest.provenance().limit_snapshot_digest() != &expected_limits {
            return Err(ArtifactLifecycleError::ManifestMismatch);
        }
        if manifest.provenance().source_runtime() != RuntimeKind::Fake
            || manifest.creation_authority().receipt().binding().runtime() != RuntimeKind::Fake
        {
            return Err(ArtifactLifecycleError::AuthorityRuntimeMismatch);
        }
        if manifest.provenance().payload_digest() != manifest.object().key().content_digest() {
            return Err(ArtifactLifecycleError::DigestMismatch);
        }
        let binding = manifest.binding();
        let provenance = manifest.provenance();
        let authority = manifest.creation_authority().receipt().binding();
        let max_field_bytes = self.limits.get(ArtifactLimitKind::FieldBytes);
        for value in [
            binding.project_id().as_str(),
            binding.project_snapshot_id().as_str(),
            binding.task_id().as_str(),
            binding.task_revision(),
            manifest.attempt_id().as_str(),
            manifest.request_id().as_str(),
            manifest.reference_id(),
            manifest.media_type(),
            manifest.payload_schema_id(),
            manifest.payload_schema_version(),
            manifest.retention_until(),
            provenance.source_producer_id(),
            provenance.source_producer_version(),
            provenance.adapter_id(),
            provenance.adapter_version(),
            provenance.invocation_id(),
            provenance.correlation_id(),
            provenance.run_id(),
            provenance.produced_at(),
            provenance.capability_id(),
            provenance.effect_claim_id(),
            provenance.daemon_instance_id(),
            authority.producer_id(),
            authority.producer_version(),
            authority.owner_record_id(),
            authority.reference_id(),
        ] {
            if u64::try_from(value.len()).map_err(|_| ArtifactLifecycleError::CounterExhausted)?
                > max_field_bytes
            {
                return Err(ArtifactLifecycleError::LimitExceeded {
                    field: "max_field_bytes",
                });
            }
        }
        if let Some(bundle) = manifest.bundle() {
            if bundle.entry_count().get() > self.limits.max_bundle_entries() {
                return Err(ArtifactLifecycleError::LimitExceeded {
                    field: "max_bundle_entries",
                });
            }
            if bundle.max_depth().get() > self.limits.max_bundle_depth() {
                return Err(ArtifactLifecycleError::LimitExceeded {
                    field: "max_bundle_depth",
                });
            }
        }
        parse_time(manifest.retention_until())?;
        Ok(())
    }

    fn validate_reference_authority(
        manifest: &ArtifactReferenceManifest,
        action: ArtifactReferenceAuthorityAction,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<(), ArtifactLifecycleError> {
        let pair = manifest.creation_authority();
        authorities.verify_reference(pair)?;
        let binding = pair.receipt().binding();
        if binding.action() != action {
            return Err(ArtifactLifecycleError::AuthorityActionMismatch);
        }
        if binding.object() != manifest.object()
            || binding.project_id() != manifest.binding().project_id()
            || binding.task_id() != manifest.binding().task_id()
            || binding.reference_id() != manifest.reference_id()
        {
            return Err(ArtifactLifecycleError::AuthorityScopeMismatch);
        }
        Ok(())
    }

    fn validate_staging_bounds(
        &self,
        _task_id: &str,
        byte_length: usize,
    ) -> Result<(), ArtifactLifecycleError> {
        let byte_length =
            u64::try_from(byte_length).map_err(|_| ArtifactLifecycleError::CounterExhausted)?;
        for kind in [
            ArtifactLimitKind::StagingBytesPerTask,
            ArtifactLimitKind::StagingBytesPerStore,
        ] {
            if byte_length > self.limits.get(kind) {
                return Err(ArtifactLifecycleError::LimitExceeded {
                    field: kind.as_str(),
                });
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn validate_quotas(&self) -> Result<(), ArtifactLifecycleError> {
        let mut store_objects = 0_u64;
        let mut store_bytes = 0_u64;
        let mut store_references = 0_u64;
        let mut store_reads = 0_u64;
        let mut project_objects: HashMap<String, u64> = HashMap::new();
        let mut project_bytes: HashMap<String, u64> = HashMap::new();
        let mut project_references: HashMap<String, u64> = HashMap::new();
        let mut project_reads: HashMap<String, u64> = HashMap::new();
        let mut task_objects: HashSet<(String, String, String)> = HashSet::new();
        let mut task_active_objects: HashSet<(String, String, String)> = HashSet::new();
        let mut task_references: HashMap<(String, String), u64> = HashMap::new();
        let mut task_reads: HashMap<(String, String), u64> = HashMap::new();

        for record in self.objects.values() {
            let active_references = active_reference_count(record);
            let blocking_reads = blocking_read_count(record);
            check_limit(
                active_references,
                self.limits.max_active_references_per_object(),
                "max_active_references_per_object",
            )?;
            check_limit(
                blocking_reads,
                self.limits.max_active_reads_per_object(),
                "max_active_reads_per_object",
            )?;
            let project_id = record.identity.key().project_id().as_str().to_owned();
            if record.availability != ArtifactAvailability::Deleted {
                let accounted_bytes = accounted_object_bytes(record)?;
                checked_add_assign(&mut store_objects, 1)?;
                checked_add_assign(&mut store_bytes, accounted_bytes)?;
                checked_map_add(&mut project_objects, &project_id, 1)?;
                checked_map_add(&mut project_bytes, &project_id, accounted_bytes)?;
            }
            checked_add_assign(&mut store_references, active_references)?;
            checked_add_assign(&mut store_reads, blocking_reads)?;
            checked_map_add(&mut project_references, &project_id, active_references)?;
            checked_map_add(&mut project_reads, &project_id, blocking_reads)?;

            let mut active_object_tasks = HashSet::new();
            for reference in record.references.values() {
                let task_id = reference.manifest().binding().task_id().as_str();
                if reference.status() == ArtifactReferenceStatus::Active {
                    active_object_tasks.insert(task_id.to_owned());
                    checked_task_map_add(&mut task_references, &project_id, task_id, 1)?;
                }
            }
            if record.availability != ArtifactAvailability::Deleted {
                for task_id in active_object_tasks {
                    task_objects.insert((
                        project_id.clone(),
                        task_id.clone(),
                        identity_token(&record.identity),
                    ));
                    task_active_objects.insert((
                        project_id.clone(),
                        task_id,
                        identity_token(&record.identity),
                    ));
                }
            }
            for read in record.reads.values().filter(|read| {
                matches!(
                    read.head.status(),
                    ArtifactReadStatus::Active | ArtifactReadStatus::ExpiredSuspect
                )
            }) {
                checked_task_map_add(
                    &mut task_reads,
                    &project_id,
                    read.head.authority().receipt().binding().task_id().as_str(),
                    1,
                )?;
            }
        }

        check_limit(
            store_objects,
            self.limits.get(ArtifactLimitKind::ObjectsPerStore),
            ArtifactLimitKind::ObjectsPerStore.as_str(),
        )?;
        check_limit(
            store_bytes,
            self.limits.get(ArtifactLimitKind::UniqueBytesPerStore),
            ArtifactLimitKind::UniqueBytesPerStore.as_str(),
        )?;
        check_limit(
            store_references,
            self.limits.get(ArtifactLimitKind::ReferencesPerStore),
            ArtifactLimitKind::ReferencesPerStore.as_str(),
        )?;
        check_limit(
            store_reads,
            self.limits.get(ArtifactLimitKind::ReadsPerStore),
            ArtifactLimitKind::ReadsPerStore.as_str(),
        )?;
        check_map_limits(
            &project_objects,
            self.limits.get(ArtifactLimitKind::ObjectsPerProject),
            ArtifactLimitKind::ObjectsPerProject.as_str(),
        )?;
        check_map_limits(
            &project_bytes,
            self.limits.get(ArtifactLimitKind::UniqueBytesPerProject),
            ArtifactLimitKind::UniqueBytesPerProject.as_str(),
        )?;
        check_map_limits(
            &project_references,
            self.limits.get(ArtifactLimitKind::ReferencesPerProject),
            ArtifactLimitKind::ReferencesPerProject.as_str(),
        )?;
        check_map_limits(
            &project_reads,
            self.limits.get(ArtifactLimitKind::ReadsPerProject),
            ArtifactLimitKind::ReadsPerProject.as_str(),
        )?;
        check_task_map_limits(
            &task_references,
            self.limits.get(ArtifactLimitKind::ReferencesPerTask),
            ArtifactLimitKind::ReferencesPerTask.as_str(),
        )?;
        check_task_map_limits(
            &task_reads,
            self.limits.get(ArtifactLimitKind::ReadsPerTask),
            ArtifactLimitKind::ReadsPerTask.as_str(),
        )?;
        check_task_set_limit(
            &task_objects,
            self.limits.get(ArtifactLimitKind::ObjectsPerTask),
            ArtifactLimitKind::ObjectsPerTask.as_str(),
        )?;
        self.check_task_active_byte_limits(&task_active_objects)?;
        Ok(())
    }

    fn check_task_active_byte_limits(
        &self,
        task_objects: &HashSet<(String, String, String)>,
    ) -> Result<(), ArtifactLifecycleError> {
        let mut bytes_by_task: HashMap<(String, String), u64> = HashMap::new();
        for (project_id, task_id, identity) in task_objects {
            let byte_length = self
                .objects
                .values()
                .find(|record| identity_token(&record.identity) == *identity)
                .map(accounted_object_bytes)
                .transpose()?
                .ok_or(ArtifactLifecycleError::ObjectNotFound)?;
            checked_task_map_add(&mut bytes_by_task, project_id, task_id, byte_length)?;
        }
        check_task_map_limits(
            &bytes_by_task,
            self.limits.get(ArtifactLimitKind::ActiveBytesPerTask),
            ArtifactLimitKind::ActiveBytesPerTask.as_str(),
        )
    }

    fn build_object_head(
        &self,
        record: &ObjectRecord,
    ) -> Result<ArtifactObjectHead, ArtifactLifecycleError> {
        let reference_set = reference_set_digest(record)?;
        let read_set = read_set_digest(record)?;
        let (
            task_quota,
            project_quota,
            store_quota,
            staging_quota,
            command_high_water,
            command_tail,
        ) = if let Some(evidence) = record.integrated_head_evidence.as_ref().filter(|evidence| {
            evidence.object == record.identity
                && evidence.lifecycle_revision.get() == record.revision
        }) {
            (
                evidence.task_quota_head_digest.clone(),
                evidence.project_quota_head_digest.clone(),
                evidence.store_quota_head_digest.clone(),
                evidence.staging_quota_head_digest.clone(),
                evidence.command_high_water,
                evidence.command_tail_digest.clone(),
            )
        } else {
            let task_quota = self.quota_projection_digest("task", Some(record))?;
            let project_quota = self.quota_projection_digest("project", Some(record))?;
            let store_quota = self.quota_projection_digest("store", None)?;
            let staging_quota = canonical_digest(
                "lattice.artifact.quota.staging",
                CanonicalValue::Object(vec![
                    string_field("staging_bytes", "0"),
                    string_field("staging_streams", "0"),
                ]),
            )?;
            let command_tail = canonical_digest(
                "lattice.artifact.command-tail",
                CanonicalValue::Object(vec![
                    string_field("object", identity_token(&record.identity)),
                    string_field("high_water", record.revision.to_string()),
                ]),
            )?;
            (
                task_quota,
                project_quota,
                store_quota,
                staging_quota,
                artifact_counter(record.revision)?,
                command_tail,
            )
        };
        let transition = canonical_digest(
            "lattice.artifact.object-head",
            CanonicalValue::Object(vec![
                string_field("object", identity_token(&record.identity)),
                string_field("revision", record.revision.to_string()),
                string_field("availability", record.availability.as_str()),
                string_field("byte_length", record.byte_length.to_string()),
                string_field(
                    "active_reference_count",
                    active_reference_count(record).to_string(),
                ),
                string_field("reference_set_digest", reference_set.as_str()),
                string_field("sweep_not_before", &record.sweep_not_before),
                string_field("active_read_count", blocking_read_count(record).to_string()),
                string_field("read_set_digest", read_set.as_str()),
                string_field("delete_status", record.delete_status.as_str()),
                string_field(
                    "claim_token",
                    record.delete_claim_token.as_deref().unwrap_or("NONE"),
                ),
                string_field("task_quota", task_quota.as_str()),
                string_field("project_quota", project_quota.as_str()),
                string_field("store_quota", store_quota.as_str()),
                string_field("staging_quota", staging_quota.as_str()),
                string_field("command_high_water", command_high_water.get().to_string()),
                string_field("command_tail_digest", command_tail.as_str()),
            ]),
        )?;
        ArtifactObjectHead::new(
            record.identity.clone(),
            artifact_revision(record.revision)?,
            record.availability,
            artifact_byte_length(record.byte_length)?,
            artifact_counter(active_reference_count(record))?,
            reference_set,
            record.sweep_not_before.clone(),
            artifact_counter(blocking_read_count(record))?,
            read_set,
            record.delete_status,
            record.delete_claim_token.clone(),
            task_quota,
            project_quota,
            store_quota,
            staging_quota,
            command_high_water,
            command_tail,
            transition,
        )
        .map_err(|_| ArtifactLifecycleError::InvalidContract)
    }

    fn quota_projection_digest(
        &self,
        scope: &str,
        subject: Option<&ObjectRecord>,
    ) -> Result<ContentDigest, ArtifactLifecycleError> {
        if scope == "task" {
            let subject = subject.ok_or(ArtifactLifecycleError::Canonicalization)?;
            let project_id = subject.identity.key().project_id().as_str();
            let mut task_scopes = subject
                .references
                .values()
                .map(|reference| {
                    (
                        project_id.to_owned(),
                        reference.manifest().binding().task_id().as_str().to_owned(),
                    )
                })
                .chain(subject.reads.values().map(|read| {
                    (
                        project_id.to_owned(),
                        read.head
                            .authority()
                            .receipt()
                            .binding()
                            .task_id()
                            .as_str()
                            .to_owned(),
                    )
                }))
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            task_scopes.sort();
            let mut projections = Vec::with_capacity(task_scopes.len());
            for (project_id, task_id) in task_scopes {
                let mut rows = self
                    .objects
                    .values()
                    .filter(|record| {
                        record.identity.key().project_id().as_str() == project_id
                            && (record.references.values().any(|reference| {
                                reference.manifest().binding().task_id().as_str() == task_id
                            }) || record.reads.values().any(|read| {
                                read.head.authority().receipt().binding().task_id().as_str()
                                    == task_id
                            }))
                    })
                    .map(quota_object_row)
                    .collect::<Vec<_>>();
                sort_canonical_values(&mut rows)?;
                projections.push(CanonicalValue::Object(vec![
                    string_field("project_id", project_id),
                    string_field("task_id", task_id),
                    ("objects".to_owned(), CanonicalValue::Array(rows)),
                ]));
            }
            return canonical_digest(
                "lattice.artifact.quota.task-set",
                CanonicalValue::Array(projections),
            );
        }

        let project_filter = subject.map(|record| record.identity.key().project_id().as_str());
        let mut rows = self
            .objects
            .values()
            .filter(|record| {
                scope != "project"
                    || project_filter.is_some_and(|project| {
                        record.identity.key().project_id().as_str() == project
                    })
            })
            .map(quota_object_row)
            .collect::<Vec<_>>();
        sort_canonical_values(&mut rows)?;
        canonical_digest(
            &format!("lattice.artifact.quota.{scope}"),
            CanonicalValue::Array(rows),
        )
    }

    fn build_receipt(
        &self,
        object: &ArtifactObjectIdentity,
        reference: Option<ArtifactReferenceHead>,
        read: Option<ArtifactReadHead>,
        operation: &str,
    ) -> Result<ArtifactAuthorityReceipt, ArtifactLifecycleError> {
        let object_head = self.build_object_head(self.record(object)?)?;
        let observation = canonical_digest(
            "lattice.artifact.observation",
            CanonicalValue::Object(vec![
                string_field("operation", operation),
                string_field("object", identity_token(object)),
                string_field("transition", object_head.transition_digest().as_str()),
            ]),
        )?;
        let receipt_digest = canonical_digest(
            "lattice.artifact.receipt",
            CanonicalValue::Object(vec![
                string_field("operation", operation),
                string_field("observation_digest", observation.as_str()),
                string_field(
                    "object_head_digest",
                    object_head.transition_digest().as_str(),
                ),
                string_field(
                    "reference_head_digest",
                    reference
                        .as_ref()
                        .map_or("NONE", |head| head.transition_digest().as_str()),
                ),
                string_field(
                    "read_head_digest",
                    read.as_ref()
                        .map_or("NONE", |head| head.transition_digest().as_str()),
                ),
            ]),
        )?;
        let receipt = ArtifactAuthorityReceipt::new(
            CONTRACT_VERSION,
            ARTIFACT_STORE_PRODUCER_ID,
            ARTIFACT_STORE_PRODUCER_VERSION,
            RuntimeKind::Fake,
            object_head,
            reference,
            read,
            observation,
            receipt_digest,
        )
        .map_err(|_| ArtifactLifecycleError::InvalidContract)?;
        validate_configured_field_bytes(self.limits, &authority_receipt_canonical_value(&receipt))?;
        Ok(receipt)
    }
}

fn verify_exact_bytes(
    object: &ArtifactObjectIdentity,
    declared_length: u64,
    exact_bytes: &[u8],
) -> Result<(), ArtifactLifecycleError> {
    let observed_length =
        u64::try_from(exact_bytes.len()).map_err(|_| ArtifactLifecycleError::CounterExhausted)?;
    if observed_length != declared_length {
        return Err(ArtifactLifecycleError::LengthMismatch);
    }
    if sha256_content(exact_bytes)? != *object.key().content_digest() {
        return Err(ArtifactLifecycleError::DigestMismatch);
    }
    Ok(())
}

fn checked_increment(value: u64) -> Result<u64, ArtifactLifecycleError> {
    const MAX_SIGNED_BIGINT: u64 = 9_223_372_036_854_775_807;
    if value >= MAX_SIGNED_BIGINT {
        Err(ArtifactLifecycleError::CounterExhausted)
    } else {
        Ok(value + 1)
    }
}

fn checked_add_assign(target: &mut u64, delta: u64) -> Result<(), ArtifactLifecycleError> {
    *target = target
        .checked_add(delta)
        .ok_or(ArtifactLifecycleError::CounterExhausted)?;
    Ok(())
}

fn checked_map_add(
    values: &mut HashMap<String, u64>,
    key: &str,
    delta: u64,
) -> Result<(), ArtifactLifecycleError> {
    checked_add_assign(values.entry(key.to_owned()).or_default(), delta)
}

fn checked_task_map_add(
    values: &mut HashMap<(String, String), u64>,
    project_id: &str,
    task_id: &str,
    delta: u64,
) -> Result<(), ArtifactLifecycleError> {
    checked_add_assign(
        values
            .entry((project_id.to_owned(), task_id.to_owned()))
            .or_default(),
        delta,
    )
}

fn check_limit(value: u64, limit: u64, field: &'static str) -> Result<(), ArtifactLifecycleError> {
    if value > limit {
        Err(ArtifactLifecycleError::LimitExceeded { field })
    } else {
        Ok(())
    }
}

fn check_map_limits(
    values: &HashMap<String, u64>,
    limit: u64,
    field: &'static str,
) -> Result<(), ArtifactLifecycleError> {
    for value in values.values() {
        check_limit(*value, limit, field)?;
    }
    Ok(())
}

fn check_task_map_limits(
    values: &HashMap<(String, String), u64>,
    limit: u64,
    field: &'static str,
) -> Result<(), ArtifactLifecycleError> {
    for value in values.values() {
        check_limit(*value, limit, field)?;
    }
    Ok(())
}

fn check_task_set_limit(
    values: &HashSet<(String, String, String)>,
    limit: u64,
    field: &'static str,
) -> Result<(), ArtifactLifecycleError> {
    let mut counts = HashMap::new();
    for (project_id, task_id, _) in values {
        checked_task_map_add(&mut counts, project_id, task_id, 1)?;
    }
    check_task_map_limits(&counts, limit, field)
}

fn active_reference_count(record: &ObjectRecord) -> u64 {
    u64::try_from(
        record
            .references
            .values()
            .filter(|reference| reference.status() == ArtifactReferenceStatus::Active)
            .count(),
    )
    .unwrap_or(u64::MAX)
}

fn blocking_read_count(record: &ObjectRecord) -> u64 {
    u64::try_from(
        record
            .reads
            .values()
            .filter(|read| {
                matches!(
                    read.head.status(),
                    ArtifactReadStatus::Active | ArtifactReadStatus::ExpiredSuspect
                )
            })
            .count(),
    )
    .unwrap_or(u64::MAX)
}

fn artifact_revision(value: u64) -> Result<ArtifactRevision, ArtifactLifecycleError> {
    ArtifactRevision::new(value).map_err(|_| ArtifactLifecycleError::CounterExhausted)
}

fn artifact_byte_length(value: u64) -> Result<ArtifactByteLength, ArtifactLifecycleError> {
    ArtifactByteLength::new(value).map_err(|_| ArtifactLifecycleError::CounterExhausted)
}

fn artifact_counter(value: u64) -> Result<ArtifactCounter, ArtifactLifecycleError> {
    ArtifactCounter::new(value).map_err(|_| ArtifactLifecycleError::CounterExhausted)
}

fn accounted_object_bytes(record: &ObjectRecord) -> Result<u64, ArtifactLifecycleError> {
    record
        .byte_length
        .checked_add(record.bundle_total_declared_bytes)
        .filter(|value| i64::try_from(*value).is_ok())
        .ok_or(ArtifactLifecycleError::CounterExhausted)
}

fn quota_object_row(record: &ObjectRecord) -> CanonicalValue {
    CanonicalValue::Object(vec![
        string_field("object", identity_token(&record.identity)),
        string_field("availability", record.availability.as_str()),
        string_field("bytes", record.byte_length.to_string()),
        string_field(
            "bundle_total_declared_bytes",
            record.bundle_total_declared_bytes.to_string(),
        ),
        string_field(
            "active_references",
            active_reference_count(record).to_string(),
        ),
        string_field("blocking_reads", blocking_read_count(record).to_string()),
    ])
}

fn u64_to_i64(value: u64) -> Result<i64, ArtifactLifecycleError> {
    i64::try_from(value).map_err(|_| ArtifactLifecycleError::CounterExhausted)
}

const fn object_quota_state(availability: ArtifactAvailability) -> ArtifactObjectQuotaState {
    match availability {
        ArtifactAvailability::Available => ArtifactObjectQuotaState::Available,
        ArtifactAvailability::DeleteClaimed => ArtifactObjectQuotaState::DeleteClaimed,
        ArtifactAvailability::Deleted => ArtifactObjectQuotaState::VerifiedDeleted,
        ArtifactAvailability::ReconciliationRequired => {
            ArtifactObjectQuotaState::ReconciliationRequired
        }
    }
}

fn canonical_max_string_bytes(value: &CanonicalValue) -> usize {
    match value {
        CanonicalValue::Null | CanonicalValue::Bool(_) => 0,
        CanonicalValue::String(value) => value.len(),
        CanonicalValue::Array(values) => values
            .iter()
            .map(canonical_max_string_bytes)
            .max()
            .unwrap_or(0),
        CanonicalValue::Object(fields) => fields
            .iter()
            .map(|(_, value)| canonical_max_string_bytes(value))
            .max()
            .unwrap_or(0),
    }
}

fn validate_configured_field_bytes(
    limits: ArtifactStoreLimits,
    value: &CanonicalValue,
) -> Result<(), ArtifactLifecycleError> {
    let maximum = u64::try_from(canonical_max_string_bytes(value))
        .map_err(|_| ArtifactLifecycleError::CounterExhausted)?;
    if maximum > limits.get(ArtifactLimitKind::FieldBytes) {
        return Err(ArtifactLifecycleError::LimitExceeded {
            field: "max_field_bytes",
        });
    }
    Ok(())
}

fn usize_to_i64(value: usize) -> Result<i64, ArtifactLifecycleError> {
    i64::try_from(value).map_err(|_| ArtifactLifecycleError::CounterExhausted)
}

fn object_identity_canonical_value(object: &ArtifactObjectIdentity) -> CanonicalValue {
    CanonicalValue::Object(vec![
        string_field("project_id", object.key().project_id().as_str()),
        string_field("algorithm", object.key().algorithm()),
        string_field("content_digest", object.key().content_digest().as_str()),
        string_field("generation", object.generation().get().to_string()),
    ])
}

fn integrated_evidence_canonical_value(
    evidence: &ArtifactIntegratedHeadEvidence,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "object".to_owned(),
            object_identity_canonical_value(&evidence.object),
        ),
        string_field(
            "lifecycle_revision",
            evidence.lifecycle_revision.get().to_string(),
        ),
        string_field(
            "task_quota_head_digest",
            evidence.task_quota_head_digest.as_str(),
        ),
        string_field(
            "project_quota_head_digest",
            evidence.project_quota_head_digest.as_str(),
        ),
        string_field(
            "store_quota_head_digest",
            evidence.store_quota_head_digest.as_str(),
        ),
        string_field(
            "staging_quota_head_digest",
            evidence.staging_quota_head_digest.as_str(),
        ),
        string_field(
            "command_high_water",
            evidence.command_high_water.get().to_string(),
        ),
        string_field("command_tail_digest", evidence.command_tail_digest.as_str()),
    ])
}

fn reference_authority_pair_canonical_value(
    pair: &ArtifactReferenceAuthorityPair,
) -> CanonicalValue {
    let binding = pair.receipt().binding();
    CanonicalValue::Object(vec![
        string_field("receipt_version", pair.receipt().version().to_string()),
        string_field("owner_kind", binding.owner_kind().as_str()),
        string_field("producer_id", binding.producer_id()),
        string_field("producer_version", binding.producer_version()),
        string_field("runtime", runtime_text(binding.runtime())),
        string_field("owner_record_id", binding.owner_record_id()),
        string_field("owner_revision", binding.owner_revision().get().to_string()),
        string_field("status", binding.status().as_str()),
        string_field("action", binding.action().as_str()),
        string_field("project_id", binding.project_id().as_str()),
        string_field("task_id", binding.task_id().as_str()),
        (
            "object".to_owned(),
            object_identity_canonical_value(binding.object()),
        ),
        string_field("reference_id", binding.reference_id()),
        string_field("observation_digest", binding.observation_digest().as_str()),
        string_field(
            "authority_receipt_digest",
            pair.receipt().receipt_digest().as_str(),
        ),
        string_field(
            "current_head_version",
            pair.current_head().version().to_string(),
        ),
        string_field(
            "authority_current_head_receipt_digest",
            pair.current_head().receipt_digest().as_str(),
        ),
    ])
}

fn read_authority_pair_canonical_value(pair: &ArtifactReadAuthorityPair) -> CanonicalValue {
    let binding = pair.receipt().binding();
    CanonicalValue::Object(vec![
        string_field("receipt_version", pair.receipt().version().to_string()),
        string_field("owner_kind", binding.owner_kind().as_str()),
        string_field("producer_id", binding.producer_id()),
        string_field("producer_version", binding.producer_version()),
        string_field("runtime", runtime_text(binding.runtime())),
        string_field("owner_record_id", binding.owner_record_id()),
        string_field("owner_revision", binding.owner_revision().get().to_string()),
        string_field("status", binding.status().as_str()),
        string_field("action", binding.action().as_str()),
        string_field("project_id", binding.project_id().as_str()),
        string_field("task_id", binding.task_id().as_str()),
        (
            "object".to_owned(),
            object_identity_canonical_value(binding.object()),
        ),
        string_field("read_claim_id", binding.read_claim_id()),
        string_field("observation_digest", binding.observation_digest().as_str()),
        string_field(
            "authority_receipt_digest",
            pair.receipt().receipt_digest().as_str(),
        ),
        string_field(
            "current_head_version",
            pair.current_head().version().to_string(),
        ),
        string_field(
            "authority_current_head_receipt_digest",
            pair.current_head().receipt_digest().as_str(),
        ),
    ])
}

fn reference_head_canonical_value(head: &ArtifactReferenceHead) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "manifest".to_owned(),
            CanonicalValue::Object(vec![
                (
                    "payload".to_owned(),
                    manifest_canonical_value(head.manifest()),
                ),
                string_field(
                    "manifest_digest",
                    head.manifest().manifest_digest().as_str(),
                ),
                (
                    "creation_authority".to_owned(),
                    reference_authority_pair_canonical_value(head.manifest().creation_authority()),
                ),
            ]),
        ),
        (
            "transition_authority".to_owned(),
            reference_authority_pair_canonical_value(head.transition_authority()),
        ),
        string_field("revision", head.revision().get().to_string()),
        string_field("status", head.status().as_str()),
        string_field("transition_digest", head.transition_digest().as_str()),
    ])
}

fn read_head_canonical_value(head: &ArtifactReadHead) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "authority".to_owned(),
            read_authority_pair_canonical_value(head.authority()),
        ),
        string_field("revision", head.revision().get().to_string()),
        string_field("status", head.status().as_str()),
        string_field("holder_id", head.holder_id()),
        string_field("acquired_at", head.acquired_at()),
        string_field("expires_at", head.expires_at()),
        string_field("transition_digest", head.transition_digest().as_str()),
    ])
}

fn object_head_canonical_value(head: &ArtifactObjectHead) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "object".to_owned(),
            object_identity_canonical_value(head.object()),
        ),
        string_field("revision", head.revision().get().to_string()),
        string_field("availability", head.availability().as_str()),
        string_field("byte_length", head.byte_length().get().to_string()),
        string_field(
            "active_reference_count",
            head.active_reference_count().get().to_string(),
        ),
        string_field(
            "active_reference_set_digest",
            head.active_reference_set_digest().as_str(),
        ),
        string_field("sweep_not_before", head.sweep_not_before()),
        string_field(
            "active_read_count",
            head.active_read_count().get().to_string(),
        ),
        string_field(
            "active_read_set_digest",
            head.active_read_set_digest().as_str(),
        ),
        string_field("delete_status", head.delete_status().as_str()),
        string_field(
            "delete_claim_token",
            head.delete_claim_token().unwrap_or("NONE"),
        ),
        string_field(
            "task_quota_projection_digest",
            head.task_quota_projection_digest().as_str(),
        ),
        string_field(
            "project_quota_projection_digest",
            head.project_quota_projection_digest().as_str(),
        ),
        string_field(
            "store_quota_projection_digest",
            head.store_quota_projection_digest().as_str(),
        ),
        string_field(
            "staging_quota_projection_digest",
            head.staging_quota_projection_digest().as_str(),
        ),
        string_field(
            "command_high_water",
            head.command_high_water().get().to_string(),
        ),
        string_field("command_tail_digest", head.command_tail_digest().as_str()),
        string_field("transition_digest", head.transition_digest().as_str()),
    ])
}

pub(crate) fn authority_receipt_canonical_value(
    receipt: &ArtifactAuthorityReceipt,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        string_field("version", receipt.version().to_string()),
        string_field("producer_id", receipt.producer_id()),
        string_field("producer_version", receipt.producer_version()),
        string_field("runtime", runtime_text(receipt.runtime())),
        (
            "object".to_owned(),
            object_head_canonical_value(receipt.object()),
        ),
        (
            "reference".to_owned(),
            receipt
                .reference()
                .map_or(CanonicalValue::Null, reference_head_canonical_value),
        ),
        (
            "read".to_owned(),
            receipt
                .read()
                .map_or(CanonicalValue::Null, read_head_canonical_value),
        ),
        string_field("observation_digest", receipt.observation_digest().as_str()),
        string_field("receipt_digest", receipt.receipt_digest().as_str()),
    ])
}

fn object_record_canonical_value(
    record: &ObjectRecord,
) -> Result<CanonicalValue, ArtifactLifecycleError> {
    let mut references = record
        .references
        .iter()
        .map(|(reference_id, head)| {
            CanonicalValue::Object(vec![
                string_field("map_reference_id", reference_id),
                ("head".to_owned(), reference_head_canonical_value(head)),
            ])
        })
        .collect::<Vec<_>>();
    sort_canonical_values(&mut references)?;
    let mut reads = record
        .reads
        .iter()
        .map(|(read_claim_id, read)| {
            CanonicalValue::Object(vec![
                string_field("map_read_claim_id", read_claim_id),
                ("head".to_owned(), read_head_canonical_value(&read.head)),
            ])
        })
        .collect::<Vec<_>>();
    sort_canonical_values(&mut reads)?;

    Ok(CanonicalValue::Object(vec![
        (
            "identity".to_owned(),
            object_identity_canonical_value(&record.identity),
        ),
        string_field("byte_length", record.byte_length.to_string()),
        string_field(
            "bundle_total_declared_bytes",
            record.bundle_total_declared_bytes.to_string(),
        ),
        string_field("revision", record.revision.to_string()),
        string_field("availability", record.availability.as_str()),
        string_field("delete_status", record.delete_status.as_str()),
        string_field(
            "delete_claim_token",
            record.delete_claim_token.as_deref().unwrap_or("NONE"),
        ),
        ("references".to_owned(), CanonicalValue::Array(references)),
        ("reads".to_owned(), CanonicalValue::Array(reads)),
        string_field("sweep_not_before", &record.sweep_not_before),
        (
            "integrated_head_evidence".to_owned(),
            record
                .integrated_head_evidence
                .as_ref()
                .map_or(CanonicalValue::Null, integrated_evidence_canonical_value),
        ),
        (
            "last_receipt".to_owned(),
            record
                .last_receipt
                .as_ref()
                .map_or(CanonicalValue::Null, authority_receipt_canonical_value),
        ),
    ]))
}

fn build_reference_head(
    manifest: ArtifactReferenceManifest,
    authority: ArtifactReferenceAuthorityPair,
    revision: u64,
    status: ArtifactReferenceStatus,
) -> Result<ArtifactReferenceHead, ArtifactLifecycleError> {
    let transition = canonical_digest(
        "lattice.artifact.reference-head",
        CanonicalValue::Object(vec![
            string_field("manifest_digest", manifest.manifest_digest().as_str()),
            string_field(
                "authority_receipt_digest",
                authority.receipt().receipt_digest().as_str(),
            ),
            string_field("revision", revision.to_string()),
            string_field("status", status.as_str()),
        ]),
    )?;
    ArtifactReferenceHead::new(
        manifest,
        authority,
        artifact_revision(revision)?,
        status,
        transition,
    )
    .map_err(|_| ArtifactLifecycleError::InvalidContract)
}

#[allow(clippy::too_many_arguments)]
fn build_read_head(
    authority: ArtifactReadAuthorityPair,
    revision: u64,
    status: ArtifactReadStatus,
    holder_id: &str,
    acquired_at: &str,
    expires_at: &str,
) -> Result<ArtifactReadHead, ArtifactLifecycleError> {
    let transition = canonical_digest(
        "lattice.artifact.read-head",
        CanonicalValue::Object(vec![
            string_field(
                "authority_receipt_digest",
                authority.receipt().receipt_digest().as_str(),
            ),
            string_field("revision", revision.to_string()),
            string_field("status", status.as_str()),
            string_field("holder_id", holder_id),
            string_field("acquired_at", acquired_at),
            string_field("expires_at", expires_at),
        ]),
    )?;
    ArtifactReadHead::new(
        authority,
        artifact_revision(revision)?,
        status,
        holder_id,
        acquired_at,
        expires_at,
        transition,
    )
    .map_err(|_| ArtifactLifecycleError::InvalidContract)
}

fn validate_read_closure_evidence(
    object: &ArtifactObjectIdentity,
    read_claim_id: &str,
    current: &ArtifactReadHead,
    evidence: &ArtifactReadClosureEvidencePair,
    authorities: &FakeArtifactAuthorityDirectory,
) -> Result<(), ArtifactLifecycleError> {
    authorities.verify_read_closure(evidence)?;
    let binding = evidence.receipt().binding();
    let read_binding = current.authority().receipt().binding();
    if binding.object() != object
        || binding.project_id() != object.key().project_id()
        || binding.task_id() != read_binding.task_id()
        || binding.read_claim_id() != read_claim_id
        || binding.holder_id() != current.holder_id()
        || parse_canonical_time(binding.observed_at())? < parse_time(current.expires_at())?
    {
        return Err(ArtifactLifecycleError::InvalidReadEvidence);
    }
    Ok(())
}

fn reference_set_digest(record: &ObjectRecord) -> Result<ContentDigest, ArtifactLifecycleError> {
    let mut rows = record
        .references
        .iter()
        .filter(|(_, reference)| reference.status() == ArtifactReferenceStatus::Active)
        .map(|(reference_id, reference)| {
            CanonicalValue::Object(vec![
                string_field("reference_id", reference_id),
                string_field("transition_digest", reference.transition_digest().as_str()),
            ])
        })
        .collect::<Vec<_>>();
    sort_canonical_values(&mut rows)?;
    canonical_digest(
        "lattice.artifact.reference-set",
        CanonicalValue::Array(rows),
    )
}

fn read_set_digest(record: &ObjectRecord) -> Result<ContentDigest, ArtifactLifecycleError> {
    let mut rows = record
        .reads
        .iter()
        .filter(|(_, read)| {
            matches!(
                read.head.status(),
                ArtifactReadStatus::Active | ArtifactReadStatus::ExpiredSuspect
            )
        })
        .map(|(read_id, read)| {
            CanonicalValue::Object(vec![
                string_field("read_claim_id", read_id),
                string_field("status", read.head.status().as_str()),
                string_field("transition_digest", read.head.transition_digest().as_str()),
            ])
        })
        .collect::<Vec<_>>();
    sort_canonical_values(&mut rows)?;
    canonical_digest("lattice.artifact.read-set", CanonicalValue::Array(rows))
}

fn sort_canonical_values(values: &mut Vec<CanonicalValue>) -> Result<(), ArtifactLifecycleError> {
    let mut keyed = values
        .drain(..)
        .map(|value| {
            let key = canonicalize(&value)
                .map_err(|_| ArtifactLifecycleError::Canonicalization)?
                .as_slice()
                .to_vec();
            Ok((key, value))
        })
        .collect::<Result<Vec<_>, ArtifactLifecycleError>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    values.extend(keyed.into_iter().map(|(_, value)| value));
    Ok(())
}

fn reference_authority_key(head: &ArtifactReferenceAuthorityHead) -> String {
    let binding = head.binding();
    format!(
        "reference:{}:{}",
        binding.producer_id(),
        binding.owner_record_id()
    )
}

fn read_authority_key(head: &ArtifactReadAuthorityHead) -> String {
    let binding = head.binding();
    format!(
        "read:{}:{}",
        binding.producer_id(),
        binding.owner_record_id()
    )
}

fn read_closure_evidence_key(head: &ArtifactReadClosureEvidenceHead) -> String {
    let binding = head.binding();
    format!(
        "read-closure:{}:{}",
        binding.producer_id(),
        binding.evidence_record_id()
    )
}

fn sweep_authority_key(head: &ArtifactSweepAuthorityHead) -> String {
    let binding = head.binding();
    format!(
        "sweep:{}:{}",
        binding.producer_id(),
        binding.owner_record_id()
    )
}

fn object_key_token(key: &ArtifactObjectKey) -> String {
    format!(
        "{}:{}:{}",
        key.project_id().as_str(),
        key.algorithm(),
        key.content_digest().as_str()
    )
}

fn identity_token(object: &ArtifactObjectIdentity) -> String {
    format!(
        "{}:{}",
        object_key_token(object.key()),
        object.generation().get()
    )
}

fn reference_terminal_token(manifest: &ArtifactReferenceManifest) -> String {
    format!(
        "{}:{}:{}",
        manifest.binding().project_id().as_str(),
        manifest.object().key().content_digest().as_str(),
        manifest.reference_id()
    )
}

fn read_terminal_token(object: &ArtifactObjectIdentity, read_claim_id: &str) -> String {
    format!("{}:{read_claim_id}", identity_token(object))
}

fn runtime_text(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Fake => "FAKE",
        RuntimeKind::Live => "LIVE",
    }
}

fn sha256_content(bytes: &[u8]) -> Result<ContentDigest, ArtifactLifecycleError> {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut text, "{byte:02x}").map_err(|_| ArtifactLifecycleError::Canonicalization)?;
    }
    ContentDigest::from_sha256(text).map_err(|_| ArtifactLifecycleError::InvalidContract)
}

#[allow(clippy::needless_pass_by_value)]
fn canonical_digest(
    schema_id: &str,
    value: CanonicalValue,
) -> Result<ContentDigest, ArtifactLifecycleError> {
    let domain =
        HashDomain::new(schema_id, "1.0").map_err(|_| ArtifactLifecycleError::Canonicalization)?;
    let digest =
        canonical_sha256(&domain, &value).map_err(|_| ArtifactLifecycleError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| ArtifactLifecycleError::InvalidContract)
}

fn string_field(name: &str, value: impl Into<String>) -> (String, CanonicalValue) {
    (name.to_owned(), CanonicalValue::String(value.into()))
}

fn parse_time(value: &str) -> Result<OffsetDateTime, ArtifactLifecycleError> {
    OffsetDateTime::parse(value, &Rfc3339).map_err(|_| ArtifactLifecycleError::Canonicalization)
}

fn parse_canonical_time(value: &str) -> Result<OffsetDateTime, ArtifactLifecycleError> {
    let parsed = parse_time(value)?;
    let canonical = parsed
        .format(&Rfc3339)
        .map_err(|_| ArtifactLifecycleError::Canonicalization)?;
    if canonical != value {
        return Err(ArtifactLifecycleError::Canonicalization);
    }
    Ok(parsed)
}

/// Recomputes the immutable manifest digest over every semantic field except
/// the digest field itself.
///
/// # Errors
///
/// Returns a typed canonicalization or digest-construction failure.
pub fn artifact_manifest_digest(
    manifest: &ArtifactReferenceManifest,
) -> Result<ContentDigest, ArtifactLifecycleError> {
    canonical_digest(
        "lattice.artifact.reference-manifest",
        manifest_canonical_value(manifest),
    )
}

/// Returns the exact canonical manifest payload byte length used for bounds.
///
/// # Errors
///
/// Returns a typed canonicalization or length-overflow failure.
pub fn artifact_manifest_canonical_len(
    manifest: &ArtifactReferenceManifest,
) -> Result<u64, ArtifactLifecycleError> {
    let bytes = canonicalize(&manifest_canonical_value(manifest))
        .map_err(|_| ArtifactLifecycleError::Canonicalization)?;
    u64::try_from(bytes.as_slice().len()).map_err(|_| ArtifactLifecycleError::CounterExhausted)
}

/// Allocates the first or strictly next non-wrapping signed-BIGINT generation.
///
/// # Errors
///
/// Returns `CounterExhausted` at the signed `BIGINT` maximum.
pub fn next_artifact_generation(
    current: Option<ArtifactGeneration>,
) -> Result<ArtifactGeneration, ArtifactLifecycleError> {
    let next = match current {
        Some(value) => checked_increment(value.get())?,
        None => 1,
    };
    ArtifactGeneration::new(next).map_err(|_| ArtifactLifecycleError::CounterExhausted)
}

#[allow(clippy::too_many_lines)]
fn manifest_canonical_value(manifest: &ArtifactReferenceManifest) -> CanonicalValue {
    let binding = manifest.binding();
    let object = manifest.object();
    let provenance = manifest.provenance();
    let authority = manifest.creation_authority().receipt().binding();
    let bundle = manifest.bundle().map_or(CanonicalValue::Null, |bundle| {
        CanonicalValue::Object(vec![
            string_field("entry_count", bundle.entry_count().get().to_string()),
            string_field("max_depth", bundle.max_depth().get().to_string()),
            string_field(
                "total_declared_bytes",
                bundle.total_declared_bytes().get().to_string(),
            ),
        ])
    });
    let provenance_value = CanonicalValue::Object(vec![
        string_field("source_producer_id", provenance.source_producer_id()),
        string_field(
            "source_producer_version",
            provenance.source_producer_version(),
        ),
        string_field("source_runtime", runtime_text(provenance.source_runtime())),
        string_field(
            "producer_binary_digest",
            provenance.producer_binary_digest().as_str(),
        ),
        string_field("adapter_id", provenance.adapter_id()),
        string_field("adapter_version", provenance.adapter_version()),
        string_field(
            "adapter_binary_digest",
            provenance.adapter_binary_digest().as_str(),
        ),
        string_field("invocation_id", provenance.invocation_id()),
        string_field("correlation_id", provenance.correlation_id()),
        string_field("run_id", provenance.run_id()),
        string_field("sequence", provenance.sequence().get().to_string()),
        string_field("produced_at", provenance.produced_at()),
        string_field("payload_digest", provenance.payload_digest().as_str()),
        string_field("capability_id", provenance.capability_id()),
        string_field("input_set_digest", provenance.input_set_digest().as_str()),
        string_field(
            "configuration_digest",
            provenance.configuration_digest().as_str(),
        ),
        string_field("evidence_digest", provenance.evidence_digest().as_str()),
        string_field(
            "registry_authority_receipt_digest",
            provenance.registry_authority_receipt_digest().as_str(),
        ),
        string_field(
            "registry_current_head_digest",
            provenance.registry_current_head_digest().as_str(),
        ),
        string_field("effect_claim_id", provenance.effect_claim_id()),
        string_field(
            "effect_claim_digest",
            provenance.effect_claim_digest().as_str(),
        ),
        string_field("daemon_instance_id", provenance.daemon_instance_id()),
        string_field("daemon_epoch", provenance.daemon_epoch().get().to_string()),
        string_field("runtime_admission", provenance.runtime_admission().as_str()),
        string_field(
            "capability_owner_receipt_digest",
            provenance.capability_owner_receipt_digest().as_str(),
        ),
        string_field(
            "capability_owner_current_head_digest",
            provenance.capability_owner_current_head_digest().as_str(),
        ),
        string_field(
            "limit_snapshot_digest",
            provenance.limit_snapshot_digest().as_str(),
        ),
    ]);
    let authority_value = CanonicalValue::Object(vec![
        string_field("owner_kind", authority.owner_kind().as_str()),
        string_field("producer_id", authority.producer_id()),
        string_field("producer_version", authority.producer_version()),
        string_field("runtime", runtime_text(authority.runtime())),
        string_field("owner_record_id", authority.owner_record_id()),
        string_field(
            "owner_revision",
            authority.owner_revision().get().to_string(),
        ),
        string_field("status", authority.status().as_str()),
        string_field("action", authority.action().as_str()),
        string_field("project_id", authority.project_id().as_str()),
        string_field("task_id", authority.task_id().as_str()),
        string_field("reference_id", authority.reference_id()),
        string_field(
            "observation_digest",
            authority.observation_digest().as_str(),
        ),
        string_field(
            "receipt_digest",
            manifest
                .creation_authority()
                .receipt()
                .receipt_digest()
                .as_str(),
        ),
        string_field(
            "current_head_digest",
            manifest
                .creation_authority()
                .current_head()
                .receipt_digest()
                .as_str(),
        ),
    ]);
    CanonicalValue::Object(vec![
        string_field("project_id", binding.project_id().as_str()),
        string_field(
            "project_snapshot_id",
            binding.project_snapshot_id().as_str(),
        ),
        string_field("task_id", binding.task_id().as_str()),
        string_field("task_revision", binding.task_revision()),
        string_field("task_spec_digest", binding.task_spec_digest().as_str()),
        string_field("attempt_id", manifest.attempt_id().as_str()),
        string_field("request_id", manifest.request_id().as_str()),
        string_field("reference_id", manifest.reference_id()),
        string_field("algorithm", object.key().algorithm()),
        string_field("content_digest", object.key().content_digest().as_str()),
        string_field("generation", object.generation().get().to_string()),
        string_field("byte_length", manifest.byte_length().get().to_string()),
        string_field("media_type", manifest.media_type()),
        string_field("payload_schema_id", manifest.payload_schema_id()),
        string_field("payload_schema_version", manifest.payload_schema_version()),
        ("bundle".to_owned(), bundle),
        ("provenance".to_owned(), provenance_value),
        ("creation_authority".to_owned(), authority_value),
        string_field("purpose", manifest.purpose().as_str()),
        string_field("retention_until", manifest.retention_until()),
    ])
}
