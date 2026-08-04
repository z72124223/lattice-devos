//! Exact, fail-closed Artifact Store quota projection and staging accounting.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactObjectIdentity,
    ArtifactObjectKey, ArtifactRevision, ContentDigest, ProjectId, RuntimeKind, TaskId,
};

use crate::{ArtifactLimitKind, ArtifactStoreLimits};

const LIMIT_COUNT: usize = ArtifactLimitKind::ALL.len();

/// A quota input, transition, arithmetic, or hash failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactQuotaError {
    /// A quota-local exact identity field is empty or contains a NUL.
    InvalidIdentity {
        /// Stable field name.
        field: &'static str,
    },
    /// A signed-BIGINT-compatible input violates its required range.
    InvalidNumber {
        /// Stable field name.
        field: &'static str,
    },
    /// The same exact identity appeared more than once.
    DuplicateIdentity {
        /// Stable identity kind.
        kind: &'static str,
    },
    /// A relationship points to an object absent from the same snapshot.
    UnknownObject,
    /// A task-scoped relationship crosses the object's project boundary.
    ProjectMismatch,
    /// An identity-bound object key or generation differs from its record.
    ObjectIdentityMismatch,
    /// More than one retained generation exists for one project/object key.
    ConflictingRetainedGeneration,
    /// An active relationship points to a verified-deleted object.
    InconsistentObjectState,
    /// Signed `BIGINT` addition overflowed.
    Overflow {
        /// Limit whose projection overflowed.
        kind: ArtifactLimitKind,
        /// Value before the attempted change.
        current: i64,
        /// Attempted signed change.
        delta: i64,
    },
    /// A signed change would make an exact projection negative.
    Underflow {
        /// Limit whose projection would become negative.
        kind: ArtifactLimitKind,
        /// Value before the attempted change.
        current: i64,
        /// Attempted signed change.
        delta: i64,
    },
    /// An exact projected value exceeds the immutable configured limit.
    LimitExceeded {
        /// Limit that was exceeded.
        kind: ArtifactLimitKind,
        /// Exact attempted value.
        attempted: i64,
        /// Exact configured limit.
        limit: i64,
    },
    /// The requested exact quota scope is absent.
    MissingScope,
    /// A staging reservation attempted a non-monotonic transition.
    InvalidStagingTransition,
    /// Typed staging terminal evidence is malformed, stale, or scope-mismatched.
    StagingEvidenceMismatch,
    /// An authoritative quota-head chain changed scope or limit snapshot.
    QuotaHeadMismatch,
    /// A positive signed-BIGINT-compatible quota-head revision was exhausted.
    CounterExhausted,
    /// Canonical quota-head framing failed.
    Canonicalization,
    /// A locally computed SHA-256 violated the shared digest contract.
    InvalidDigest,
}

impl ArtifactQuotaError {
    /// Stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidIdentity { .. } => "ARTIFACT_QUOTA_INVALID_IDENTITY",
            Self::InvalidNumber { .. } => "ARTIFACT_QUOTA_INVALID_NUMBER",
            Self::DuplicateIdentity { .. } => "ARTIFACT_QUOTA_DUPLICATE_IDENTITY",
            Self::UnknownObject => "ARTIFACT_QUOTA_UNKNOWN_OBJECT",
            Self::ProjectMismatch => "ARTIFACT_QUOTA_PROJECT_MISMATCH",
            Self::ObjectIdentityMismatch => "ARTIFACT_QUOTA_OBJECT_IDENTITY_MISMATCH",
            Self::ConflictingRetainedGeneration => "ARTIFACT_QUOTA_CONFLICTING_RETAINED_GENERATION",
            Self::InconsistentObjectState => "ARTIFACT_QUOTA_INCONSISTENT_OBJECT_STATE",
            Self::Overflow { .. } => "ARTIFACT_QUOTA_OVERFLOW",
            Self::Underflow { .. } => "ARTIFACT_QUOTA_UNDERFLOW",
            Self::LimitExceeded { .. } => "ARTIFACT_QUOTA_LIMIT_EXCEEDED",
            Self::MissingScope => "ARTIFACT_QUOTA_MISSING_SCOPE",
            Self::InvalidStagingTransition => "ARTIFACT_QUOTA_INVALID_STAGING_TRANSITION",
            Self::StagingEvidenceMismatch => "ARTIFACT_QUOTA_STAGING_EVIDENCE_MISMATCH",
            Self::QuotaHeadMismatch => "ARTIFACT_QUOTA_HEAD_MISMATCH",
            Self::CounterExhausted => "ARTIFACT_QUOTA_COUNTER_EXHAUSTED",
            Self::Canonicalization => "ARTIFACT_QUOTA_CANONICALIZATION_FAILED",
            Self::InvalidDigest => "ARTIFACT_QUOTA_INVALID_DIGEST",
        }
    }
}

impl fmt::Display for ArtifactQuotaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentity { field } => write!(formatter, "invalid identity: {field}"),
            Self::InvalidNumber { field } => write!(formatter, "invalid quota number: {field}"),
            Self::DuplicateIdentity { kind } => {
                write!(formatter, "duplicate exact {kind} identity")
            }
            Self::UnknownObject => formatter.write_str("quota relationship references no object"),
            Self::ProjectMismatch => {
                formatter.write_str("quota relationship crosses a project boundary")
            }
            Self::ObjectIdentityMismatch => {
                formatter.write_str("quota relationship object binding does not match")
            }
            Self::ConflictingRetainedGeneration => {
                formatter.write_str("multiple retained generations share one object key")
            }
            Self::InconsistentObjectState => {
                formatter.write_str("active relationship targets a verified-deleted object")
            }
            Self::Overflow {
                kind,
                current,
                delta,
            } => write!(
                formatter,
                "{} overflowed: {current} + {delta}",
                kind.as_str()
            ),
            Self::Underflow {
                kind,
                current,
                delta,
            } => write!(
                formatter,
                "{} underflowed: {current} + {delta}",
                kind.as_str()
            ),
            Self::LimitExceeded {
                kind,
                attempted,
                limit,
            } => write!(
                formatter,
                "{} exceeded: {attempted} > {limit}",
                kind.as_str()
            ),
            Self::MissingScope => formatter.write_str("quota scope is absent"),
            Self::InvalidStagingTransition => {
                formatter.write_str("invalid staging reservation transition")
            }
            Self::StagingEvidenceMismatch => {
                formatter.write_str("staging terminal evidence is not exact and current")
            }
            Self::QuotaHeadMismatch => {
                formatter.write_str("quota-head scope or immutable limits changed")
            }
            Self::CounterExhausted => formatter.write_str("quota-head revision was exhausted"),
            Self::Canonicalization => formatter.write_str("quota-head canonicalization failed"),
            Self::InvalidDigest => formatter.write_str("quota-head digest was invalid"),
        }
    }
}

impl Error for ArtifactQuotaError {}

fn validate_identity(value: &str, field: &'static str) -> Result<(), ArtifactQuotaError> {
    let valid = !value.is_empty()
        && value.len() <= 256
        && value.trim() == value
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(ArtifactQuotaError::InvalidIdentity { field })
    }
}

fn validate_nonnegative(value: i64, field: &'static str) -> Result<(), ArtifactQuotaError> {
    if value < 0 {
        Err(ArtifactQuotaError::InvalidNumber { field })
    } else {
        Ok(())
    }
}

fn limit_as_i64(value: u64, field: &'static str) -> Result<i64, ArtifactQuotaError> {
    i64::try_from(value).map_err(|_| ArtifactQuotaError::InvalidNumber { field })
}

fn quota_digest(
    domain: &'static str,
    value: &CanonicalValue,
) -> Result<ContentDigest, ArtifactQuotaError> {
    let domain =
        HashDomain::new(domain, "1.0").map_err(|_| ArtifactQuotaError::Canonicalization)?;
    let hash =
        canonical_sha256(&domain, value).map_err(|_| ArtifactQuotaError::Canonicalization)?;
    ContentDigest::from_sha256(hash.to_hex()).map_err(|_| ArtifactQuotaError::InvalidDigest)
}

const fn runtime_label(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Fake => "FAKE",
        RuntimeKind::Live => "LIVE",
    }
}

/// Exact identity of one Artifact Store composition.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactStoreIdentity(String);

impl ArtifactStoreIdentity {
    /// Constructs a path-free ASCII identity of at most 256 bytes.
    ///
    /// # Errors
    ///
    /// Rejects empty, untrimmed, non-ASCII, path-bearing, or oversized text.
    pub fn new(value: impl Into<String>) -> Result<Self, ArtifactQuotaError> {
        let value = value.into();
        validate_identity(&value, "store_id")?;
        Ok(Self(value))
    }

    /// Returns the exact store identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Exact project/object-key/task/reference identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactReferenceIdentity {
    task_id: TaskId,
    object_key: ArtifactObjectKey,
    value: String,
}

impl ArtifactReferenceIdentity {
    /// Constructs a reference identity whose generation is deliberately absent.
    ///
    /// # Errors
    ///
    /// Rejects an invalid quota-local reference identifier.
    pub fn new(
        task_id: TaskId,
        object_key: ArtifactObjectKey,
        value: impl Into<String>,
    ) -> Result<Self, ArtifactQuotaError> {
        let value = value.into();
        validate_identity(&value, "reference_id")?;
        Ok(Self {
            task_id,
            object_key,
            value,
        })
    }

    /// Returns the project namespace bound by the object key.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        self.object_key.project_id()
    }

    /// Returns the shared task identifier.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the generation-independent shared object key.
    #[must_use]
    pub const fn object_key(&self) -> &ArtifactObjectKey {
        &self.object_key
    }

    /// Returns the exact reference identifier.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Exact project/object-generation/task/read-claim identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactReadIdentity {
    task_id: TaskId,
    object: ArtifactObjectIdentity,
    value: String,
}

impl ArtifactReadIdentity {
    /// Constructs a read identity bound to one exact physical generation.
    ///
    /// # Errors
    ///
    /// Rejects an invalid quota-local read-claim identifier.
    pub fn new(
        task_id: TaskId,
        object: ArtifactObjectIdentity,
        value: impl Into<String>,
    ) -> Result<Self, ArtifactQuotaError> {
        let value = value.into();
        validate_identity(&value, "read_id")?;
        Ok(Self {
            task_id,
            object,
            value,
        })
    }

    /// Returns the project namespace bound by the exact object.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        self.object.key().project_id()
    }

    /// Returns the shared task identifier.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the exact object generation.
    #[must_use]
    pub const fn object(&self) -> &ArtifactObjectIdentity {
        &self.object
    }

    /// Returns the exact read-claim identifier.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Exact project/object-key/task/command identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactCommandIdentity {
    task_id: TaskId,
    object_key: ArtifactObjectKey,
    value: String,
}

impl ArtifactCommandIdentity {
    /// Constructs a generation-independent idempotency identity.
    ///
    /// # Errors
    ///
    /// Rejects an invalid quota-local command identifier.
    pub fn new(
        task_id: TaskId,
        object_key: ArtifactObjectKey,
        value: impl Into<String>,
    ) -> Result<Self, ArtifactQuotaError> {
        let value = value.into();
        validate_identity(&value, "command_id")?;
        Ok(Self {
            task_id,
            object_key,
            value,
        })
    }

    /// Returns the project namespace bound by the object key.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        self.object_key.project_id()
    }

    /// Returns the shared task identifier.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the generation-independent shared object key.
    #[must_use]
    pub const fn object_key(&self) -> &ArtifactObjectKey {
        &self.object_key
    }

    /// Returns the exact command identifier.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Exact project/task/reservation identity of one staging reservation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtifactStagingIdentity {
    object_key: ArtifactObjectKey,
    task_id: TaskId,
    value: String,
}

impl ArtifactStagingIdentity {
    /// Constructs an object-key/task-bound staging reservation identity.
    ///
    /// The logical object key binds project plus SHA-256 while deliberately
    /// omitting generation because staging precedes metadata publication.
    ///
    /// # Errors
    ///
    /// Rejects an invalid quota-local reservation identifier.
    pub fn new(
        object_key: ArtifactObjectKey,
        task_id: TaskId,
        value: impl Into<String>,
    ) -> Result<Self, ArtifactQuotaError> {
        let value = value.into();
        validate_identity(&value, "reservation_id")?;
        Ok(Self {
            object_key,
            task_id,
            value,
        })
    }

    /// Returns the shared project identifier.
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        self.object_key.project_id()
    }

    /// Returns the exact project-scoped target key without a generation.
    #[must_use]
    pub const fn object_key(&self) -> &ArtifactObjectKey {
        &self.object_key
    }

    /// Returns the shared task identifier.
    #[must_use]
    pub const fn task_id(&self) -> &TaskId {
        &self.task_id
    }

    /// Returns the exact reservation identifier.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Quota retention state of one shared object generation identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactObjectQuotaState {
    /// Available for normal reads and references.
    Available,
    /// Delete ownership is claimed; worst-case quota remains held.
    DeleteClaimed,
    /// Physical outcome is unknown; worst-case quota remains held.
    ReconciliationRequired,
    /// Unpublished sealed bytes remain; worst-case quota remains held.
    SealedOrphan,
    /// Physical deletion was independently verified.
    VerifiedDeleted,
}

impl ArtifactObjectQuotaState {
    const fn retains_quota(self) -> bool {
        !matches!(self, Self::VerifiedDeleted)
    }
}

/// Exact object-generation metrics used to recompute quota.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactObjectQuotaRecord {
    identity: ArtifactObjectIdentity,
    byte_length: i64,
    max_field_bytes: i64,
    bundle_entries: i64,
    bundle_depth: i64,
    state: ArtifactObjectQuotaState,
}

impl ArtifactObjectQuotaRecord {
    /// Constructs one exact object quota record.
    ///
    /// # Errors
    ///
    /// Rejects any negative signed-`BIGINT` metric.
    pub fn new(
        identity: ArtifactObjectIdentity,
        byte_length: i64,
        max_field_bytes: i64,
        bundle_entries: i64,
        bundle_depth: i64,
        state: ArtifactObjectQuotaState,
    ) -> Result<Self, ArtifactQuotaError> {
        validate_nonnegative(byte_length, "byte_length")?;
        validate_nonnegative(max_field_bytes, "max_field_bytes")?;
        validate_nonnegative(bundle_entries, "bundle_entries")?;
        validate_nonnegative(bundle_depth, "bundle_depth")?;
        Ok(Self {
            identity,
            byte_length,
            max_field_bytes,
            bundle_entries,
            bundle_depth,
            state,
        })
    }

    /// Returns the shared object identity.
    #[must_use]
    pub const fn identity(&self) -> &ArtifactObjectIdentity {
        &self.identity
    }

    /// Returns the exact signed-BIGINT-compatible byte length.
    #[must_use]
    pub const fn byte_length(&self) -> i64 {
        self.byte_length
    }

    /// Returns the quota-retention state.
    #[must_use]
    pub const fn state(&self) -> ArtifactObjectQuotaState {
        self.state
    }
}

/// Whether an immutable reference still contributes active quota.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactReferenceQuotaState {
    /// The reference is active.
    Active,
    /// Its terminal release is recorded.
    Released,
}

/// Exact reference metrics used to recompute quota.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReferenceQuotaRecord {
    identity: ArtifactReferenceIdentity,
    object: ArtifactObjectIdentity,
    manifest_bytes: i64,
    state: ArtifactReferenceQuotaState,
}

impl ArtifactReferenceQuotaRecord {
    /// Constructs one immutable-reference quota record.
    ///
    /// # Errors
    ///
    /// Rejects a negative manifest byte length.
    pub fn new(
        identity: ArtifactReferenceIdentity,
        object: ArtifactObjectIdentity,
        manifest_bytes: i64,
        state: ArtifactReferenceQuotaState,
    ) -> Result<Self, ArtifactQuotaError> {
        validate_nonnegative(manifest_bytes, "manifest_bytes")?;
        Ok(Self {
            identity,
            object,
            manifest_bytes,
            state,
        })
    }

    /// Returns the exact reference identity.
    #[must_use]
    pub const fn identity(&self) -> &ArtifactReferenceIdentity {
        &self.identity
    }

    /// Returns the referenced shared object identity.
    #[must_use]
    pub const fn object(&self) -> &ArtifactObjectIdentity {
        &self.object
    }
}

/// Read-claim state controlling quota retention.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactReadQuotaState {
    /// The bounded claim remains active.
    Active,
    /// TTL elapsed, but holder death or handle closure is unverified.
    ExpiredSuspect,
    /// Outcome is unknown and must retain quota.
    ReconciliationRequired,
    /// Holder death or handle closure was independently verified.
    VerifiedClosed,
}

impl ArtifactReadQuotaState {
    const fn retains_quota(self) -> bool {
        !matches!(self, Self::VerifiedClosed)
    }
}

/// Exact read-claim metrics used to recompute quota.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactReadQuotaRecord {
    identity: ArtifactReadIdentity,
    object: ArtifactObjectIdentity,
    max_field_bytes: i64,
    state: ArtifactReadQuotaState,
}

impl ArtifactReadQuotaRecord {
    /// Constructs one exact read-claim quota record.
    #[must_use]
    pub const fn new(
        identity: ArtifactReadIdentity,
        object: ArtifactObjectIdentity,
        state: ArtifactReadQuotaState,
    ) -> Self {
        Self {
            identity,
            object,
            max_field_bytes: 0,
            state,
        }
    }

    /// Binds the exact maximum retained lifecycle string width to this read.
    ///
    /// # Errors
    ///
    /// Rejects a negative signed-`BIGINT` metric.
    pub fn with_max_field_bytes(
        mut self,
        max_field_bytes: i64,
    ) -> Result<Self, ArtifactQuotaError> {
        validate_nonnegative(max_field_bytes, "max_field_bytes")?;
        self.max_field_bytes = max_field_bytes;
        Ok(self)
    }

    /// Returns the exact read identity.
    #[must_use]
    pub const fn identity(&self) -> &ArtifactReadIdentity {
        &self.identity
    }
}

/// One retained terminal command and its canonical history size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactCommandQuotaRecord {
    identity: ArtifactCommandIdentity,
    history_bytes: i64,
}

impl ArtifactCommandQuotaRecord {
    /// Constructs one terminal command quota record.
    ///
    /// # Errors
    ///
    /// Rejects a negative canonical history byte length.
    pub fn new(
        identity: ArtifactCommandIdentity,
        history_bytes: i64,
    ) -> Result<Self, ArtifactQuotaError> {
        validate_nonnegative(history_bytes, "history_bytes")?;
        Ok(Self {
            identity,
            history_bytes,
        })
    }

    /// Returns the exact command identity.
    #[must_use]
    pub const fn identity(&self) -> &ArtifactCommandIdentity {
        &self.identity
    }
}

/// Monotonic staging reservation state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ArtifactStagingState {
    /// Bytes and one or more streams are actively reserved.
    Active,
    /// A sealed but unpublished orphan remains physically present.
    SealedOrphan,
    /// Outcome is unknown; worst-case quota remains held.
    ReconciliationRequired,
    /// Publication and staged-byte disposition were independently verified.
    VerifiedPublished,
    /// Physical cleanup was independently verified.
    VerifiedCleaned,
}

impl ArtifactStagingState {
    const fn retains_quota(self) -> bool {
        matches!(
            self,
            Self::Active | Self::SealedOrphan | Self::ReconciliationRequired
        )
    }

    const fn is_verified_terminal(self) -> bool {
        matches!(self, Self::VerifiedPublished | Self::VerifiedCleaned)
    }

    const fn can_fail_safe_transition_to(self, next: Self) -> bool {
        match self {
            Self::Active => matches!(
                next,
                Self::Active | Self::SealedOrphan | Self::ReconciliationRequired
            ),
            Self::SealedOrphan => {
                matches!(next, Self::SealedOrphan | Self::ReconciliationRequired)
            }
            Self::ReconciliationRequired => matches!(next, Self::ReconciliationRequired),
            Self::VerifiedPublished | Self::VerifiedCleaned => false,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::SealedOrphan => "SEALED_ORPHAN",
            Self::ReconciliationRequired => "RECONCILIATION_REQUIRED",
            Self::VerifiedPublished => "VERIFIED_PUBLISHED",
            Self::VerifiedCleaned => "VERIFIED_CLEANED",
        }
    }
}

/// Exact fixed-owner binding for one verified staging terminal transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactStagingTerminalBinding {
    identity: ArtifactStagingIdentity,
    bytes: i64,
    streams: i64,
    from: ArtifactStagingState,
    to: ArtifactStagingState,
    runtime: RuntimeKind,
}

impl ArtifactStagingTerminalBinding {
    fn from_reservation(
        reservation: &ArtifactStagingReservation,
        to: ArtifactStagingState,
    ) -> Result<Self, ArtifactQuotaError> {
        if reservation.state.is_verified_terminal() || !to.is_verified_terminal() {
            return Err(ArtifactQuotaError::InvalidStagingTransition);
        }
        Ok(Self {
            identity: reservation.identity.clone(),
            bytes: reservation.bytes,
            streams: reservation.streams,
            from: reservation.state,
            to,
            runtime: RuntimeKind::Fake,
        })
    }

    fn canonical_value(&self) -> CanonicalValue {
        CanonicalValue::Object(vec![
            (
                "algorithm".to_owned(),
                CanonicalValue::String(self.identity.object_key().algorithm().to_owned()),
            ),
            (
                "bytes".to_owned(),
                CanonicalValue::String(self.bytes.to_string()),
            ),
            (
                "content_digest".to_owned(),
                CanonicalValue::String(
                    self.identity
                        .object_key()
                        .content_digest()
                        .as_str()
                        .to_owned(),
                ),
            ),
            (
                "from".to_owned(),
                CanonicalValue::String(self.from.label().to_owned()),
            ),
            (
                "project_id".to_owned(),
                CanonicalValue::String(self.identity.project_id().as_str().to_owned()),
            ),
            (
                "reservation_id".to_owned(),
                CanonicalValue::String(self.identity.value().to_owned()),
            ),
            (
                "runtime".to_owned(),
                CanonicalValue::String(runtime_label(self.runtime).to_owned()),
            ),
            (
                "streams".to_owned(),
                CanonicalValue::String(self.streams.to_string()),
            ),
            (
                "task_id".to_owned(),
                CanonicalValue::String(self.identity.task_id().as_str().to_owned()),
            ),
            (
                "to".to_owned(),
                CanonicalValue::String(self.to.label().to_owned()),
            ),
        ])
    }

    /// Returns the exact object-key/task/reservation identity.
    #[must_use]
    pub const fn identity(&self) -> &ArtifactStagingIdentity {
        &self.identity
    }

    /// Returns the exact reserved bytes.
    #[must_use]
    pub const fn bytes(&self) -> i64 {
        self.bytes
    }

    /// Returns the exact reserved stream count.
    #[must_use]
    pub const fn streams(&self) -> i64 {
        self.streams
    }

    /// Returns the exact expected source state.
    #[must_use]
    pub const fn from(&self) -> ArtifactStagingState {
        self.from
    }

    /// Returns the verified terminal target state.
    #[must_use]
    pub const fn to(&self) -> ArtifactStagingState {
        self.to
    }

    /// Returns the visibly fake runtime marker.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        self.runtime
    }
}

/// Fixed-owner fake receipt for one verified staging terminal transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactStagingTerminalReceipt {
    binding: ArtifactStagingTerminalBinding,
    observation_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl ArtifactStagingTerminalReceipt {
    fn new(
        binding: ArtifactStagingTerminalBinding,
        observation_digest: ContentDigest,
    ) -> Result<Self, ArtifactQuotaError> {
        let receipt_digest = quota_digest(
            "lattice.artifact.staging-terminal-receipt",
            &CanonicalValue::Object(vec![
                ("binding".to_owned(), binding.canonical_value()),
                (
                    "observation_digest".to_owned(),
                    CanonicalValue::String(observation_digest.as_str().to_owned()),
                ),
                (
                    "producer_id".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_ID.to_owned()),
                ),
                (
                    "producer_version".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_VERSION.to_owned()),
                ),
                (
                    "runtime".to_owned(),
                    CanonicalValue::String(runtime_label(RuntimeKind::Fake).to_owned()),
                ),
            ]),
        )?;
        Ok(Self {
            binding,
            observation_digest,
            receipt_digest,
        })
    }

    fn verify_digest(&self) -> Result<(), ArtifactQuotaError> {
        let expected = Self::new(self.binding.clone(), self.observation_digest.clone())?;
        if expected.receipt_digest == self.receipt_digest {
            Ok(())
        } else {
            Err(ArtifactQuotaError::StagingEvidenceMismatch)
        }
    }

    /// Returns the only accepted semantic producer identity.
    #[must_use]
    pub const fn producer_id(&self) -> &'static str {
        ARTIFACT_STORE_PRODUCER_ID
    }

    /// Returns the only accepted semantic producer version.
    #[must_use]
    pub const fn producer_version(&self) -> &'static str {
        ARTIFACT_STORE_PRODUCER_VERSION
    }

    /// Returns the visibly fake runtime marker.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        RuntimeKind::Fake
    }

    /// Returns the exact transition binding.
    #[must_use]
    pub const fn binding(&self) -> &ArtifactStagingTerminalBinding {
        &self.binding
    }

    /// Returns the independently supplied terminal observation digest.
    #[must_use]
    pub const fn observation_digest(&self) -> &ContentDigest {
        &self.observation_digest
    }

    /// Returns the fixed-domain receipt digest.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

/// Independently queried current fake head for one staging terminal receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactStagingTerminalHead {
    binding: ArtifactStagingTerminalBinding,
    receipt_digest: ContentDigest,
    head_digest: ContentDigest,
}

impl ArtifactStagingTerminalHead {
    fn new(receipt: &ArtifactStagingTerminalReceipt) -> Result<Self, ArtifactQuotaError> {
        let head_digest = quota_digest(
            "lattice.artifact.staging-terminal-current-head",
            &CanonicalValue::Object(vec![
                ("binding".to_owned(), receipt.binding.canonical_value()),
                (
                    "producer_id".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_ID.to_owned()),
                ),
                (
                    "producer_version".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_VERSION.to_owned()),
                ),
                (
                    "receipt_digest".to_owned(),
                    CanonicalValue::String(receipt.receipt_digest.as_str().to_owned()),
                ),
                (
                    "runtime".to_owned(),
                    CanonicalValue::String(runtime_label(RuntimeKind::Fake).to_owned()),
                ),
            ]),
        )?;
        Ok(Self {
            binding: receipt.binding.clone(),
            receipt_digest: receipt.receipt_digest.clone(),
            head_digest,
        })
    }

    fn verify_digest(&self) -> Result<(), ArtifactQuotaError> {
        let synthetic = ArtifactStagingTerminalReceipt {
            binding: self.binding.clone(),
            observation_digest: self.receipt_digest.clone(),
            receipt_digest: self.receipt_digest.clone(),
        };
        let expected = Self::new(&synthetic)?;
        if expected.head_digest == self.head_digest {
            Ok(())
        } else {
            Err(ArtifactQuotaError::StagingEvidenceMismatch)
        }
    }

    /// Returns the exact transition binding.
    #[must_use]
    pub const fn binding(&self) -> &ArtifactStagingTerminalBinding {
        &self.binding
    }

    /// Returns the exact receipt digest mirrored by this head.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    /// Returns the fixed-domain current-head digest.
    #[must_use]
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }
}

/// Typed fixed-owner receipt plus independently queried current fake head.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactStagingTerminalEvidence {
    receipt: ArtifactStagingTerminalReceipt,
    current_head: ArtifactStagingTerminalHead,
}

impl ArtifactStagingTerminalEvidence {
    fn new(
        receipt: ArtifactStagingTerminalReceipt,
        current_head: ArtifactStagingTerminalHead,
    ) -> Result<Self, ArtifactQuotaError> {
        receipt.verify_digest()?;
        current_head.verify_digest()?;
        if receipt.binding != current_head.binding
            || receipt.receipt_digest != current_head.receipt_digest
        {
            return Err(ArtifactQuotaError::StagingEvidenceMismatch);
        }
        Ok(Self {
            receipt,
            current_head,
        })
    }

    /// Returns the fixed-owner terminal receipt.
    #[must_use]
    pub const fn receipt(&self) -> &ArtifactStagingTerminalReceipt {
        &self.receipt
    }

    /// Returns the independently queried current fake head.
    #[must_use]
    pub const fn current_head(&self) -> &ArtifactStagingTerminalHead {
        &self.current_head
    }
}

/// Deterministic visibly fake staging-terminal authority directory.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FakeArtifactStagingTerminalAuthority {
    heads: HashMap<ArtifactStagingIdentity, ArtifactStagingTerminalHead>,
}

impl FakeArtifactStagingTerminalAuthority {
    /// Issues and records an exact fixed-owner fake terminal pair.
    ///
    /// # Errors
    ///
    /// Rejects terminal source state, a non-terminal target, or hash failure.
    pub fn issue(
        &mut self,
        reservation: &ArtifactStagingReservation,
        to: ArtifactStagingState,
        observation_digest: ContentDigest,
    ) -> Result<ArtifactStagingTerminalEvidence, ArtifactQuotaError> {
        let binding = ArtifactStagingTerminalBinding::from_reservation(reservation, to)?;
        let receipt = ArtifactStagingTerminalReceipt::new(binding, observation_digest)?;
        let head = ArtifactStagingTerminalHead::new(&receipt)?;
        let evidence = ArtifactStagingTerminalEvidence::new(receipt, head.clone())?;
        self.heads.insert(reservation.identity.clone(), head);
        Ok(evidence)
    }

    /// Returns an independently queried current head for one reservation.
    #[must_use]
    pub fn current_head(
        &self,
        identity: &ArtifactStagingIdentity,
    ) -> Option<&ArtifactStagingTerminalHead> {
        self.heads.get(identity)
    }

    fn verify(&self, evidence: &ArtifactStagingTerminalEvidence) -> Result<(), ArtifactQuotaError> {
        let binding = evidence.receipt.binding();
        evidence.receipt.verify_digest()?;
        evidence.current_head.verify_digest()?;
        if self.current_head(binding.identity()) != Some(evidence.current_head())
            || binding.runtime() != RuntimeKind::Fake
            || evidence.receipt.producer_id() != ARTIFACT_STORE_PRODUCER_ID
            || evidence.receipt.producer_version() != ARTIFACT_STORE_PRODUCER_VERSION
            || evidence.receipt.runtime() != RuntimeKind::Fake
        {
            return Err(ArtifactQuotaError::StagingEvidenceMismatch);
        }
        Ok(())
    }
}

/// One exact staging byte-and-stream reservation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactStagingReservation {
    identity: ArtifactStagingIdentity,
    bytes: i64,
    streams: i64,
    state: ArtifactStagingState,
}

impl ArtifactStagingReservation {
    /// Constructs one active reservation.
    ///
    /// # Errors
    ///
    /// Rejects negative bytes or a non-positive stream count.
    pub fn new(
        identity: ArtifactStagingIdentity,
        bytes: i64,
        streams: i64,
    ) -> Result<Self, ArtifactQuotaError> {
        validate_nonnegative(bytes, "staging_bytes")?;
        if streams <= 0 {
            return Err(ArtifactQuotaError::InvalidNumber {
                field: "staging_streams",
            });
        }
        Ok(Self {
            identity,
            bytes,
            streams,
            state: ArtifactStagingState::Active,
        })
    }

    /// Restores one exact persisted state after validating its immutable
    /// identity and metrics. This constructor is crate-sealed to aggregate
    /// closed-schema replay.
    pub(crate) fn restore_exact(
        identity: ArtifactStagingIdentity,
        bytes: i64,
        streams: i64,
        state: ArtifactStagingState,
    ) -> Result<Self, ArtifactQuotaError> {
        let mut reservation = Self::new(identity, bytes, streams)?;
        reservation.state = state;
        Ok(reservation)
    }

    /// Marks exact sealed bytes as an unpublished orphan without freeing quota.
    ///
    /// # Errors
    ///
    /// Rejects a backward transition or any change from a terminal state.
    pub fn mark_sealed_orphan(&mut self) -> Result<(), ArtifactQuotaError> {
        self.apply_fail_safe_transition(ArtifactStagingState::SealedOrphan)
    }

    /// Marks an unknown staging outcome without freeing quota.
    ///
    /// # Errors
    ///
    /// Rejects any change from a verified terminal state.
    pub fn mark_reconciliation_required(&mut self) -> Result<(), ArtifactQuotaError> {
        self.apply_fail_safe_transition(ArtifactStagingState::ReconciliationRequired)
    }

    /// Applies a verified terminal only from an exact current fake owner pair.
    ///
    /// # Errors
    ///
    /// Rejects stale, substituted, wrong-scope, wrong-metric, or non-current
    /// evidence without changing this reservation.
    pub fn apply_verified_terminal(
        &mut self,
        evidence: &ArtifactStagingTerminalEvidence,
        authority: &FakeArtifactStagingTerminalAuthority,
    ) -> Result<(), ArtifactQuotaError> {
        authority.verify(evidence)?;
        let binding = evidence.receipt().binding();
        if binding.identity() != &self.identity
            || binding.bytes() != self.bytes
            || binding.streams() != self.streams
            || binding.from() != self.state
            || !binding.to().is_verified_terminal()
        {
            return Err(ArtifactQuotaError::StagingEvidenceMismatch);
        }
        self.state = binding.to();
        Ok(())
    }

    fn apply_fail_safe_transition(
        &mut self,
        next: ArtifactStagingState,
    ) -> Result<(), ArtifactQuotaError> {
        if !self.state.can_fail_safe_transition_to(next) {
            return Err(ArtifactQuotaError::InvalidStagingTransition);
        }
        self.state = next;
        Ok(())
    }

    /// Returns the exact reservation identity.
    #[must_use]
    pub const fn identity(&self) -> &ArtifactStagingIdentity {
        &self.identity
    }

    /// Returns the current state.
    #[must_use]
    pub const fn state(&self) -> ArtifactStagingState {
        self.state
    }

    /// Returns the exact reserved byte count.
    #[must_use]
    pub const fn bytes(&self) -> i64 {
        self.bytes
    }

    /// Returns the exact reserved stream count.
    #[must_use]
    pub const fn streams(&self) -> i64 {
        self.streams
    }
}

/// Exact values for all 30 Artifact Store limits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactQuotaProjection {
    values: [i64; LIMIT_COUNT],
}

impl ArtifactQuotaProjection {
    /// Returns the all-zero exact projection.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            values: [0; LIMIT_COUNT],
        }
    }

    /// Returns one signed-BIGINT-compatible projected value.
    #[must_use]
    pub const fn get(&self, kind: ArtifactLimitKind) -> i64 {
        self.values[kind.index()]
    }

    /// Returns a copy with one exact non-negative value.
    ///
    /// # Errors
    ///
    /// Rejects a negative value.
    pub fn with_value(
        mut self,
        kind: ArtifactLimitKind,
        value: i64,
    ) -> Result<Self, ArtifactQuotaError> {
        validate_nonnegative(value, kind.as_str())?;
        self.values[kind.index()] = value;
        Ok(self)
    }

    /// Atomically applies a signed delta and validates all 30 fields.
    ///
    /// No field changes on overflow, underflow, or limit failure.
    ///
    /// # Errors
    ///
    /// Returns a typed arithmetic or limit error without mutating this value.
    pub fn checked_apply(
        &mut self,
        delta: &ArtifactQuotaDelta,
        limits: ArtifactStoreLimits,
    ) -> Result<(), ArtifactQuotaError> {
        let mut candidate = self.clone();
        for kind in ArtifactLimitKind::ALL {
            let current = candidate.get(kind);
            let change = delta.get(kind);
            let attempted = current
                .checked_add(change)
                .ok_or(ArtifactQuotaError::Overflow {
                    kind,
                    current,
                    delta: change,
                })?;
            if attempted < 0 {
                return Err(ArtifactQuotaError::Underflow {
                    kind,
                    current,
                    delta: change,
                });
            }
            candidate.values[kind.index()] = attempted;
        }
        candidate.validate(limits)?;
        *self = candidate;
        Ok(())
    }

    /// Atomically replaces this projection from exact identity sets.
    ///
    /// The prior projection remains unchanged on every recompute failure.
    ///
    /// # Errors
    ///
    /// Returns the snapshot's typed identity, arithmetic, or limit error.
    pub fn checked_recompute(
        &mut self,
        snapshot: &ArtifactQuotaSnapshot,
        limits: ArtifactStoreLimits,
    ) -> Result<(), ArtifactQuotaError> {
        let candidate = snapshot.recompute(limits)?.projection;
        *self = candidate;
        Ok(())
    }

    fn validate(&self, limits: ArtifactStoreLimits) -> Result<(), ArtifactQuotaError> {
        for kind in ArtifactLimitKind::ALL {
            let attempted = self.get(kind);
            let limit = limit_as_i64(limits.get(kind), kind.as_str())?;
            if attempted > limit {
                return Err(ArtifactQuotaError::LimitExceeded {
                    kind,
                    attempted,
                    limit,
                });
            }
        }
        Ok(())
    }

    fn checked_add(
        &mut self,
        kind: ArtifactLimitKind,
        delta: i64,
    ) -> Result<(), ArtifactQuotaError> {
        let current = self.get(kind);
        let attempted = current
            .checked_add(delta)
            .ok_or(ArtifactQuotaError::Overflow {
                kind,
                current,
                delta,
            })?;
        if attempted < 0 {
            return Err(ArtifactQuotaError::Underflow {
                kind,
                current,
                delta,
            });
        }
        self.values[kind.index()] = attempted;
        Ok(())
    }

    fn set_max(&mut self, kind: ArtifactLimitKind, value: i64) {
        let index = kind.index();
        self.values[index] = self.values[index].max(value);
    }

    fn merge_max(&mut self, other: &Self) {
        for kind in ArtifactLimitKind::ALL {
            self.set_max(kind, other.get(kind));
        }
    }

    fn canonical_value(&self) -> CanonicalValue {
        CanonicalValue::Object(
            ArtifactLimitKind::ALL
                .into_iter()
                .map(|kind| {
                    (
                        kind.as_str().to_owned(),
                        CanonicalValue::String(self.get(kind).to_string()),
                    )
                })
                .collect(),
        )
    }
}

/// Signed exact changes for all 30 quota fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactQuotaDelta {
    values: [i64; LIMIT_COUNT],
}

impl ArtifactQuotaDelta {
    /// Returns the all-zero delta.
    #[must_use]
    pub const fn zero() -> Self {
        Self {
            values: [0; LIMIT_COUNT],
        }
    }

    /// Returns a one-field signed delta.
    #[must_use]
    pub fn single(kind: ArtifactLimitKind, change: i64) -> Self {
        let mut delta = Self::zero();
        delta.values[kind.index()] = change;
        delta
    }

    /// Checked-adds one signed field change.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactQuotaError::Overflow`] if the delta cannot fit.
    pub fn with_change(
        mut self,
        kind: ArtifactLimitKind,
        change: i64,
    ) -> Result<Self, ArtifactQuotaError> {
        let current = self.values[kind.index()];
        self.values[kind.index()] =
            current
                .checked_add(change)
                .ok_or(ArtifactQuotaError::Overflow {
                    kind,
                    current,
                    delta: change,
                })?;
        Ok(self)
    }

    /// Returns one exact signed change.
    #[must_use]
    pub const fn get(&self, kind: ArtifactLimitKind) -> i64 {
        self.values[kind.index()]
    }
}

/// Immutable exact metadata used for a fail-closed non-authorizing recompute.
///
/// Public callers may use this calculator for validation and planning only.
/// A snapshot, report, projection, or digest produced here is never a current
/// owner head and cannot authorize a mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactQuotaSnapshot {
    store: ArtifactStoreIdentity,
    objects: Vec<ArtifactObjectQuotaRecord>,
    references: Vec<ArtifactReferenceQuotaRecord>,
    reads: Vec<ArtifactReadQuotaRecord>,
    commands: Vec<ArtifactCommandQuotaRecord>,
    staging: Vec<ArtifactStagingReservation>,
}

impl ArtifactQuotaSnapshot {
    /// Constructs a snapshot without any caller-supplied quota Boolean.
    #[must_use]
    pub fn new(
        store: ArtifactStoreIdentity,
        objects: Vec<ArtifactObjectQuotaRecord>,
        references: Vec<ArtifactReferenceQuotaRecord>,
        reads: Vec<ArtifactReadQuotaRecord>,
        commands: Vec<ArtifactCommandQuotaRecord>,
        staging: Vec<ArtifactStagingReservation>,
    ) -> Self {
        Self {
            store,
            objects,
            references,
            reads,
            commands,
            staging,
        }
    }

    /// Recomputes all 30 fields from exact identity sets and validates limits.
    ///
    /// The returned report is explicitly non-authorizing. Only the crate-owned
    /// root aggregate may seal an internally recomputed projection into an
    /// [`ArtifactQuotaHead`].
    ///
    /// # Errors
    ///
    /// Rejects malformed relationships, duplicate identities, arithmetic
    /// overflow, conflicting generations, or any exceeded limit.
    pub fn recompute(
        &self,
        limits: ArtifactStoreLimits,
    ) -> Result<ArtifactQuotaReport, ArtifactQuotaError> {
        Recompute::new(self).run(limits)
    }
}

/// One quota-head scope with a distinct hash domain.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ArtifactQuotaScope {
    /// One shared object generation identity.
    Object(ArtifactObjectIdentity),
    /// One shared project/task pair.
    Task {
        /// Shared project identifier.
        project_id: ProjectId,
        /// Shared task identifier.
        task_id: TaskId,
    },
    /// One shared project identifier.
    Project(ProjectId),
    /// One exact Artifact Store identity.
    Store(ArtifactStoreIdentity),
}

impl ArtifactQuotaScope {
    /// Returns the frozen scope-specific quota-head domain.
    #[must_use]
    pub const fn domain(&self) -> &'static str {
        match self {
            Self::Object(_) => "lattice.artifact.quota.object-head",
            Self::Task { .. } => "lattice.artifact.quota.task-head",
            Self::Project(_) => "lattice.artifact.quota.project-head",
            Self::Store(_) => "lattice.artifact.quota.store-head",
        }
    }

    fn canonical_value(&self) -> CanonicalValue {
        match self {
            Self::Object(identity) => CanonicalValue::Object(vec![
                (
                    "algorithm".to_owned(),
                    CanonicalValue::String(identity.key().algorithm().to_owned()),
                ),
                (
                    "digest".to_owned(),
                    CanonicalValue::String(identity.key().content_digest().as_str().to_owned()),
                ),
                (
                    "generation".to_owned(),
                    CanonicalValue::String(identity.generation().get().to_string()),
                ),
                (
                    "project_id".to_owned(),
                    CanonicalValue::String(identity.key().project_id().as_str().to_owned()),
                ),
                (
                    "scope_type".to_owned(),
                    CanonicalValue::String("object".to_owned()),
                ),
            ]),
            Self::Task {
                project_id,
                task_id,
            } => CanonicalValue::Object(vec![
                (
                    "project_id".to_owned(),
                    CanonicalValue::String(project_id.as_str().to_owned()),
                ),
                (
                    "scope_type".to_owned(),
                    CanonicalValue::String("task".to_owned()),
                ),
                (
                    "task_id".to_owned(),
                    CanonicalValue::String(task_id.as_str().to_owned()),
                ),
            ]),
            Self::Project(project_id) => CanonicalValue::Object(vec![
                (
                    "project_id".to_owned(),
                    CanonicalValue::String(project_id.as_str().to_owned()),
                ),
                (
                    "scope_type".to_owned(),
                    CanonicalValue::String("project".to_owned()),
                ),
            ]),
            Self::Store(store) => CanonicalValue::Object(vec![
                (
                    "scope_type".to_owned(),
                    CanonicalValue::String("store".to_owned()),
                ),
                (
                    "store_id".to_owned(),
                    CanonicalValue::String(store.as_str().to_owned()),
                ),
            ]),
        }
    }
}

/// Crate-sealed authoritative projection input.
///
/// Public recompute reports remain explicitly non-authorizing. Only the root
/// Artifact Store aggregate may convert one of its internally recomputed
/// projections into this sealed input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactQuotaAuthorityProjection {
    scope: ArtifactQuotaScope,
    projection: ArtifactQuotaProjection,
    limit_snapshot_digest: ContentDigest,
}

/// Immutable fixed-owner quota head and its scope-separated digest chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactQuotaHead {
    scope: ArtifactQuotaScope,
    revision: ArtifactRevision,
    projection: ArtifactQuotaProjection,
    limit_snapshot_digest: ContentDigest,
    predecessor_head_digest: ContentDigest,
    transition_tail_digest: ContentDigest,
    head_digest: ContentDigest,
}

impl ArtifactQuotaHead {
    #[allow(dead_code)]
    pub(crate) fn initial(
        authoritative: ArtifactQuotaAuthorityProjection,
    ) -> Result<Self, ArtifactQuotaError> {
        let predecessor_head_digest = quota_digest(
            "lattice.artifact.quota-head-genesis",
            &CanonicalValue::Object(vec![
                (
                    "limit_snapshot_digest".to_owned(),
                    CanonicalValue::String(authoritative.limit_snapshot_digest.as_str().to_owned()),
                ),
                (
                    "producer_id".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_ID.to_owned()),
                ),
                (
                    "producer_version".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_VERSION.to_owned()),
                ),
                (
                    "runtime".to_owned(),
                    CanonicalValue::String(runtime_label(RuntimeKind::Fake).to_owned()),
                ),
                ("scope".to_owned(), authoritative.scope.canonical_value()),
            ]),
        )?;
        let revision =
            ArtifactRevision::new(1).map_err(|_| ArtifactQuotaError::CounterExhausted)?;
        Self::build(authoritative, revision, predecessor_head_digest)
    }

    #[allow(dead_code)]
    pub(crate) fn successor(
        &self,
        authoritative: ArtifactQuotaAuthorityProjection,
    ) -> Result<Self, ArtifactQuotaError> {
        if authoritative.scope != self.scope
            || authoritative.limit_snapshot_digest != self.limit_snapshot_digest
        {
            return Err(ArtifactQuotaError::QuotaHeadMismatch);
        }
        let revision = self
            .revision
            .get()
            .checked_add(1)
            .ok_or(ArtifactQuotaError::CounterExhausted)
            .and_then(|value| {
                ArtifactRevision::new(value).map_err(|_| ArtifactQuotaError::CounterExhausted)
            })?;
        Self::build(authoritative, revision, self.head_digest.clone())
    }

    fn build(
        authoritative: ArtifactQuotaAuthorityProjection,
        revision: ArtifactRevision,
        predecessor_head_digest: ContentDigest,
    ) -> Result<Self, ArtifactQuotaError> {
        let transition_tail_digest = quota_digest(
            "lattice.artifact.quota-transition-tail",
            &CanonicalValue::Object(vec![
                (
                    "limit_snapshot_digest".to_owned(),
                    CanonicalValue::String(authoritative.limit_snapshot_digest.as_str().to_owned()),
                ),
                (
                    "predecessor_head_digest".to_owned(),
                    CanonicalValue::String(predecessor_head_digest.as_str().to_owned()),
                ),
                (
                    "producer_id".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_ID.to_owned()),
                ),
                (
                    "producer_version".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_VERSION.to_owned()),
                ),
                (
                    "projection".to_owned(),
                    authoritative.projection.canonical_value(),
                ),
                (
                    "revision".to_owned(),
                    CanonicalValue::String(revision.get().to_string()),
                ),
                (
                    "runtime".to_owned(),
                    CanonicalValue::String(runtime_label(RuntimeKind::Fake).to_owned()),
                ),
                ("scope".to_owned(), authoritative.scope.canonical_value()),
            ]),
        )?;
        let head_digest = quota_digest(
            authoritative.scope.domain(),
            &CanonicalValue::Object(vec![
                (
                    "limit_snapshot_digest".to_owned(),
                    CanonicalValue::String(authoritative.limit_snapshot_digest.as_str().to_owned()),
                ),
                (
                    "predecessor_head_digest".to_owned(),
                    CanonicalValue::String(predecessor_head_digest.as_str().to_owned()),
                ),
                (
                    "producer_id".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_ID.to_owned()),
                ),
                (
                    "producer_version".to_owned(),
                    CanonicalValue::String(ARTIFACT_STORE_PRODUCER_VERSION.to_owned()),
                ),
                (
                    "projection".to_owned(),
                    authoritative.projection.canonical_value(),
                ),
                (
                    "revision".to_owned(),
                    CanonicalValue::String(revision.get().to_string()),
                ),
                (
                    "runtime".to_owned(),
                    CanonicalValue::String(runtime_label(RuntimeKind::Fake).to_owned()),
                ),
                ("scope".to_owned(), authoritative.scope.canonical_value()),
                (
                    "transition_tail_digest".to_owned(),
                    CanonicalValue::String(transition_tail_digest.as_str().to_owned()),
                ),
            ]),
        )?;
        Ok(Self {
            scope: authoritative.scope,
            revision,
            projection: authoritative.projection,
            limit_snapshot_digest: authoritative.limit_snapshot_digest,
            predecessor_head_digest,
            transition_tail_digest,
            head_digest,
        })
    }

    /// Restores one exact current head from untrusted storage while
    /// recomputing both head hash domains from its complete closed projection.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn restore_exact(
        scope: ArtifactQuotaScope,
        revision: ArtifactRevision,
        projection: ArtifactQuotaProjection,
        limit_snapshot_digest: ContentDigest,
        predecessor_head_digest: ContentDigest,
        transition_tail_digest: &ContentDigest,
        head_digest: &ContentDigest,
        limits: ArtifactStoreLimits,
    ) -> Result<Self, ArtifactQuotaError> {
        projection.validate(limits)?;
        let authoritative = ArtifactQuotaAuthorityProjection {
            scope,
            projection,
            limit_snapshot_digest,
        };
        if revision.get() == 1 {
            let initial = Self::initial(authoritative.clone())?;
            if initial.predecessor_head_digest != predecessor_head_digest {
                return Err(ArtifactQuotaError::QuotaHeadMismatch);
            }
        }
        let rebuilt = Self::build(authoritative, revision, predecessor_head_digest)?;
        if &rebuilt.transition_tail_digest != transition_tail_digest
            || &rebuilt.head_digest != head_digest
        {
            return Err(ArtifactQuotaError::QuotaHeadMismatch);
        }
        Ok(rebuilt)
    }

    /// Returns the only accepted semantic producer identity.
    #[must_use]
    pub const fn producer_id(&self) -> &'static str {
        ARTIFACT_STORE_PRODUCER_ID
    }

    /// Returns the only accepted semantic producer version.
    #[must_use]
    pub const fn producer_version(&self) -> &'static str {
        ARTIFACT_STORE_PRODUCER_VERSION
    }

    /// Returns the visibly fake runtime marker.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        RuntimeKind::Fake
    }

    /// Returns the exact scope.
    #[must_use]
    pub const fn scope(&self) -> &ArtifactQuotaScope {
        &self.scope
    }

    /// Returns the positive owner-managed monotonic revision.
    #[must_use]
    pub const fn revision(&self) -> ArtifactRevision {
        self.revision
    }

    /// Returns the exact 30-field projection hashed by this head.
    #[must_use]
    pub const fn projection(&self) -> &ArtifactQuotaProjection {
        &self.projection
    }

    /// Returns the immutable configured limit snapshot digest.
    #[must_use]
    pub const fn limit_snapshot_digest(&self) -> &ContentDigest {
        &self.limit_snapshot_digest
    }

    /// Returns the previous authoritative head or fixed genesis digest.
    #[must_use]
    pub const fn predecessor_head_digest(&self) -> &ContentDigest {
        &self.predecessor_head_digest
    }

    /// Returns the owner-computed transition tail digest.
    #[must_use]
    pub const fn transition_tail_digest(&self) -> &ContentDigest {
        &self.transition_tail_digest
    }

    /// Returns the complete scope-domain-separated head digest.
    #[must_use]
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TaskKey {
    project_id: ProjectId,
    task_id: TaskId,
}

impl TaskKey {
    fn new(project_id: &ProjectId, task_id: &TaskId) -> Self {
        Self {
            project_id: project_id.clone(),
            task_id: task_id.clone(),
        }
    }
}

/// Exact non-authorizing projections from one public recompute.
///
/// This report proves calculator consistency only. It is not a fixed-owner
/// receipt, current head, checkpoint, or permission to mutate state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactQuotaReport {
    store: ArtifactStoreIdentity,
    limit_snapshot_digest: ContentDigest,
    projection: ArtifactQuotaProjection,
    objects: HashMap<ArtifactObjectIdentity, ArtifactQuotaProjection>,
    object_keys: HashMap<ArtifactObjectKey, ArtifactQuotaProjection>,
    tasks: HashMap<TaskKey, ArtifactQuotaProjection>,
    projects: HashMap<ProjectId, ArtifactQuotaProjection>,
    store_projection: ArtifactQuotaProjection,
}

impl ArtifactQuotaReport {
    /// Returns the immutable configured limit snapshot used for recompute.
    #[must_use]
    pub const fn limit_snapshot_digest(&self) -> &ContentDigest {
        &self.limit_snapshot_digest
    }

    /// Returns the global maxima/totals used for complete validation.
    #[must_use]
    pub const fn projection(&self) -> &ArtifactQuotaProjection {
        &self.projection
    }

    /// Returns one exact per-task projection.
    #[must_use]
    pub fn task_projection(
        &self,
        project_id: &ProjectId,
        task_id: &TaskId,
    ) -> Option<&ArtifactQuotaProjection> {
        self.tasks.get(&TaskKey::new(project_id, task_id))
    }

    /// Returns one non-authorizing logical-object-key projection.
    ///
    /// This scope exists before publication and deliberately has no generation.
    #[must_use]
    pub fn object_key_projection(
        &self,
        object_key: &ArtifactObjectKey,
    ) -> Option<&ArtifactQuotaProjection> {
        self.object_keys.get(object_key)
    }

    #[allow(dead_code)]
    pub(crate) fn authority_projection(
        &self,
        scope: ArtifactQuotaScope,
    ) -> Result<ArtifactQuotaAuthorityProjection, ArtifactQuotaError> {
        let projection = match &scope {
            ArtifactQuotaScope::Object(identity) => self.objects.get(identity),
            ArtifactQuotaScope::Task {
                project_id,
                task_id,
            } => self.tasks.get(&TaskKey::new(project_id, task_id)),
            ArtifactQuotaScope::Project(project_id) => self.projects.get(project_id),
            ArtifactQuotaScope::Store(store) if store == &self.store => {
                Some(&self.store_projection)
            }
            ArtifactQuotaScope::Store(_) => None,
        }
        .ok_or(ArtifactQuotaError::MissingScope)?
        .clone();
        Ok(ArtifactQuotaAuthorityProjection {
            scope,
            projection,
            limit_snapshot_digest: self.limit_snapshot_digest.clone(),
        })
    }
}

struct Recompute<'a> {
    snapshot: &'a ArtifactQuotaSnapshot,
    objects: HashMap<ArtifactObjectIdentity, ArtifactQuotaProjection>,
    object_keys: HashMap<ArtifactObjectKey, ArtifactQuotaProjection>,
    tasks: HashMap<TaskKey, ArtifactQuotaProjection>,
    projects: HashMap<ProjectId, ArtifactQuotaProjection>,
    store: ArtifactQuotaProjection,
    object_records: HashMap<ArtifactObjectIdentity, &'a ArtifactObjectQuotaRecord>,
    task_objects: HashSet<(TaskKey, ArtifactObjectIdentity)>,
}

impl<'a> Recompute<'a> {
    fn new(snapshot: &'a ArtifactQuotaSnapshot) -> Self {
        let mut store = ArtifactQuotaProjection::zero();
        store.set_max(
            ArtifactLimitKind::FieldBytes,
            usize_to_i64(snapshot.store.as_str().len()),
        );
        Self {
            snapshot,
            objects: HashMap::new(),
            object_keys: HashMap::new(),
            tasks: HashMap::new(),
            projects: HashMap::new(),
            store,
            object_records: HashMap::new(),
            task_objects: HashSet::new(),
        }
    }

    fn run(
        mut self,
        limits: ArtifactStoreLimits,
    ) -> Result<ArtifactQuotaReport, ArtifactQuotaError> {
        self.count_objects()?;
        self.count_references()?;
        self.count_task_objects()?;
        self.count_reads()?;
        self.count_commands()?;
        self.count_staging()?;

        let mut projection = ArtifactQuotaProjection::zero();
        for scoped in self
            .objects
            .values()
            .chain(self.object_keys.values())
            .chain(self.tasks.values())
            .chain(self.projects.values())
            .chain(std::iter::once(&self.store))
        {
            projection.merge_max(scoped);
        }
        projection.validate(limits)?;
        let limit_snapshot_digest = limits
            .limit_snapshot_digest()
            .map_err(|_| ArtifactQuotaError::Canonicalization)?;
        Ok(ArtifactQuotaReport {
            store: self.snapshot.store.clone(),
            limit_snapshot_digest,
            projection,
            objects: self.objects,
            object_keys: self.object_keys,
            tasks: self.tasks,
            projects: self.projects,
            store_projection: self.store,
        })
    }

    fn count_objects(&mut self) -> Result<(), ArtifactQuotaError> {
        let mut retained_keys = HashSet::new();
        for record in &self.snapshot.objects {
            if self
                .object_records
                .insert(record.identity.clone(), record)
                .is_some()
            {
                return Err(ArtifactQuotaError::DuplicateIdentity { kind: "object" });
            }
            let object = self
                .objects
                .entry(record.identity.clone())
                .or_insert_with(ArtifactQuotaProjection::zero);
            object.set_max(
                ArtifactLimitKind::FieldBytes,
                max_object_identity_field(record),
            );
            object.set_max(ArtifactLimitKind::FieldBytes, record.max_field_bytes);
            let project = self
                .projects
                .entry(record.identity.key().project_id().clone())
                .or_insert_with(ArtifactQuotaProjection::zero);
            project.set_max(
                ArtifactLimitKind::FieldBytes,
                usize_to_i64(record.identity.key().project_id().as_str().len()),
            );
            project.set_max(ArtifactLimitKind::FieldBytes, record.max_field_bytes);
            self.store
                .set_max(ArtifactLimitKind::FieldBytes, record.max_field_bytes);
            if !record.state.retains_quota() {
                continue;
            }
            if !retained_keys.insert(record.identity.key().clone()) {
                return Err(ArtifactQuotaError::ConflictingRetainedGeneration);
            }
            object.set_max(ArtifactLimitKind::ObjectBytes, record.byte_length);
            object.set_max(ArtifactLimitKind::BundleEntries, record.bundle_entries);
            object.set_max(ArtifactLimitKind::BundleDepth, record.bundle_depth);

            project.checked_add(ArtifactLimitKind::ObjectsPerProject, 1)?;
            project.checked_add(ArtifactLimitKind::UniqueBytesPerProject, record.byte_length)?;
            self.store
                .checked_add(ArtifactLimitKind::ObjectsPerStore, 1)?;
            self.store
                .checked_add(ArtifactLimitKind::UniqueBytesPerStore, record.byte_length)?;
        }
        Ok(())
    }

    fn count_references(&mut self) -> Result<(), ArtifactQuotaError> {
        let mut identities = HashSet::new();
        for record in &self.snapshot.references {
            if !identities.insert(record.identity.clone()) {
                return Err(ArtifactQuotaError::DuplicateIdentity { kind: "reference" });
            }
            if record.identity.object_key() != record.object.key() {
                return Err(ArtifactQuotaError::ObjectIdentityMismatch);
            }
            let retained = self
                .related_object(&record.object, record.identity.project_id())?
                .state
                .retains_quota();
            {
                let object = self
                    .objects
                    .get_mut(&record.object)
                    .ok_or(ArtifactQuotaError::UnknownObject)?;
                object.set_max(ArtifactLimitKind::ManifestBytes, record.manifest_bytes);
                object.set_max(
                    ArtifactLimitKind::FieldBytes,
                    max_scoped_identity_field(&record.identity),
                );
            }
            let task_key = TaskKey::new(record.identity.project_id(), record.identity.task_id());
            self.task_mut(&task_key).set_max(
                ArtifactLimitKind::FieldBytes,
                max_scoped_identity_field(&record.identity),
            );
            if !matches!(record.state, ArtifactReferenceQuotaState::Active) {
                continue;
            }
            if !retained {
                return Err(ArtifactQuotaError::InconsistentObjectState);
            }
            self.objects
                .get_mut(&record.object)
                .ok_or(ArtifactQuotaError::UnknownObject)?
                .checked_add(ArtifactLimitKind::ActiveReferencesPerObject, 1)?;
            self.task_mut(&task_key)
                .checked_add(ArtifactLimitKind::ReferencesPerTask, 1)?;
            self.project_mut(record.identity.project_id())
                .checked_add(ArtifactLimitKind::ReferencesPerProject, 1)?;
            self.store
                .checked_add(ArtifactLimitKind::ReferencesPerStore, 1)?;
            self.task_objects.insert((task_key, record.object.clone()));
        }
        Ok(())
    }

    fn count_task_objects(&mut self) -> Result<(), ArtifactQuotaError> {
        for (task_key, object_identity) in self.task_objects.clone() {
            self.task_mut(&task_key)
                .checked_add(ArtifactLimitKind::ObjectsPerTask, 1)?;
            let bytes = self
                .object_records
                .get(&object_identity)
                .ok_or(ArtifactQuotaError::UnknownObject)?
                .byte_length;
            self.task_mut(&task_key)
                .checked_add(ArtifactLimitKind::ActiveBytesPerTask, bytes)?;
        }
        Ok(())
    }

    fn count_reads(&mut self) -> Result<(), ArtifactQuotaError> {
        let mut identities = HashSet::new();
        for record in &self.snapshot.reads {
            if !identities.insert(record.identity.clone()) {
                return Err(ArtifactQuotaError::DuplicateIdentity { kind: "read" });
            }
            if record.identity.object() != &record.object {
                return Err(ArtifactQuotaError::ObjectIdentityMismatch);
            }
            let retained = self
                .related_object(&record.object, record.identity.project_id())?
                .state
                .retains_quota();
            let max_field_bytes =
                max_scoped_identity_field(&record.identity).max(record.max_field_bytes);
            self.objects
                .get_mut(&record.object)
                .ok_or(ArtifactQuotaError::UnknownObject)?
                .set_max(ArtifactLimitKind::FieldBytes, max_field_bytes);
            let task_key = TaskKey::new(record.identity.project_id(), record.identity.task_id());
            self.task_mut(&task_key)
                .set_max(ArtifactLimitKind::FieldBytes, max_field_bytes);
            self.project_mut(record.identity.project_id())
                .set_max(ArtifactLimitKind::FieldBytes, max_field_bytes);
            self.store
                .set_max(ArtifactLimitKind::FieldBytes, max_field_bytes);
            if !record.state.retains_quota() {
                continue;
            }
            if !retained {
                return Err(ArtifactQuotaError::InconsistentObjectState);
            }
            self.objects
                .get_mut(&record.object)
                .ok_or(ArtifactQuotaError::UnknownObject)?
                .checked_add(ArtifactLimitKind::ActiveReadsPerObject, 1)?;
            self.task_mut(&task_key)
                .checked_add(ArtifactLimitKind::ReadsPerTask, 1)?;
            self.project_mut(record.identity.project_id())
                .checked_add(ArtifactLimitKind::ReadsPerProject, 1)?;
            self.store
                .checked_add(ArtifactLimitKind::ReadsPerStore, 1)?;
        }
        Ok(())
    }

    fn count_commands(&mut self) -> Result<(), ArtifactQuotaError> {
        let mut identities = HashSet::new();
        for record in &self.snapshot.commands {
            let storage_key = (
                record.identity.object_key().clone(),
                record.identity.value().to_owned(),
            );
            if !identities.insert(storage_key) {
                return Err(ArtifactQuotaError::DuplicateIdentity { kind: "command" });
            }
            let object_key = record.identity.object_key().clone();
            let logical_object = self
                .object_keys
                .entry(object_key)
                .or_insert_with(ArtifactQuotaProjection::zero);
            logical_object.set_max(
                ArtifactLimitKind::FieldBytes,
                max_scoped_identity_field(&record.identity),
            );
            logical_object.checked_add(ArtifactLimitKind::CommandsPerObject, 1)?;
            let task_key = TaskKey::new(record.identity.project_id(), record.identity.task_id());
            let task = self.task_mut(&task_key);
            task.set_max(
                ArtifactLimitKind::FieldBytes,
                max_scoped_identity_field(&record.identity),
            );
            task.checked_add(ArtifactLimitKind::CommandsPerTask, 1)?;
            task.checked_add(ArtifactLimitKind::HistoryBytesPerTask, record.history_bytes)?;
            let project = self.project_mut(record.identity.project_id());
            project.checked_add(ArtifactLimitKind::CommandsPerProject, 1)?;
            project.checked_add(
                ArtifactLimitKind::HistoryBytesPerProject,
                record.history_bytes,
            )?;
            self.store
                .checked_add(ArtifactLimitKind::CommandsPerStore, 1)?;
            self.store.checked_add(
                ArtifactLimitKind::HistoryBytesPerStore,
                record.history_bytes,
            )?;
        }
        for (object_key, logical) in &self.object_keys {
            for (identity, projection) in &mut self.objects {
                if identity.key() == object_key {
                    projection.set_max(
                        ArtifactLimitKind::CommandsPerObject,
                        logical.get(ArtifactLimitKind::CommandsPerObject),
                    );
                    projection.set_max(
                        ArtifactLimitKind::FieldBytes,
                        logical.get(ArtifactLimitKind::FieldBytes),
                    );
                }
            }
        }
        Ok(())
    }

    fn count_staging(&mut self) -> Result<(), ArtifactQuotaError> {
        let mut identities = HashSet::new();
        for reservation in &self.snapshot.staging {
            if !identities.insert(reservation.identity.clone()) {
                return Err(ArtifactQuotaError::DuplicateIdentity { kind: "staging" });
            }
            let task_key = TaskKey::new(
                reservation.identity.project_id(),
                reservation.identity.task_id(),
            );
            let task = self.task_mut(&task_key);
            task.set_max(
                ArtifactLimitKind::FieldBytes,
                max_scoped_identity_field(&reservation.identity),
            );
            if !reservation.state.retains_quota() {
                continue;
            }
            task.checked_add(ArtifactLimitKind::StagingBytesPerTask, reservation.bytes)?;
            task.checked_add(
                ArtifactLimitKind::StagingStreamsPerTask,
                reservation.streams,
            )?;
            self.store
                .checked_add(ArtifactLimitKind::StagingBytesPerStore, reservation.bytes)?;
            self.store.checked_add(
                ArtifactLimitKind::StagingStreamsPerStore,
                reservation.streams,
            )?;
        }
        Ok(())
    }

    fn related_object(
        &self,
        identity: &ArtifactObjectIdentity,
        project_id: &ProjectId,
    ) -> Result<&ArtifactObjectQuotaRecord, ArtifactQuotaError> {
        let record = self
            .object_records
            .get(identity)
            .copied()
            .ok_or(ArtifactQuotaError::UnknownObject)?;
        if project_id != identity.key().project_id() {
            return Err(ArtifactQuotaError::ProjectMismatch);
        }
        Ok(record)
    }

    fn task_mut(&mut self, key: &TaskKey) -> &mut ArtifactQuotaProjection {
        let projection = self
            .tasks
            .entry(key.clone())
            .or_insert_with(ArtifactQuotaProjection::zero);
        projection.set_max(
            ArtifactLimitKind::FieldBytes,
            usize_to_i64(key.project_id.as_str().len()),
        );
        projection.set_max(
            ArtifactLimitKind::FieldBytes,
            usize_to_i64(key.task_id.as_str().len()),
        );
        projection
    }

    fn project_mut(&mut self, project_id: &ProjectId) -> &mut ArtifactQuotaProjection {
        let projection = self
            .projects
            .entry(project_id.clone())
            .or_insert_with(ArtifactQuotaProjection::zero);
        projection.set_max(
            ArtifactLimitKind::FieldBytes,
            usize_to_i64(project_id.as_str().len()),
        );
        projection
    }
}

fn usize_to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn max_object_identity_field(record: &ArtifactObjectQuotaRecord) -> i64 {
    [
        record.identity.key().project_id().as_str().len(),
        record.identity.key().algorithm().len(),
        record.identity.key().content_digest().as_str().len(),
    ]
    .into_iter()
    .map(usize_to_i64)
    .max()
    .unwrap_or(0)
}

fn max_scoped_identity_field<T>(identity: &T) -> i64
where
    T: ScopedIdentityFields,
{
    [
        identity.project_id().as_str().len(),
        identity.task_id().as_str().len(),
        identity.local_value().len(),
    ]
    .into_iter()
    .map(usize_to_i64)
    .max()
    .unwrap_or(0)
}

trait ScopedIdentityFields {
    fn project_id(&self) -> &ProjectId;
    fn task_id(&self) -> &TaskId;
    fn local_value(&self) -> &str;
}

macro_rules! scoped_identity_fields {
    ($($name:ident),+ $(,)?) => {
        $(
            impl ScopedIdentityFields for $name {
                fn project_id(&self) -> &ProjectId {
                    self.project_id()
                }

                fn task_id(&self) -> &TaskId {
                    self.task_id()
                }

                fn local_value(&self) -> &str {
                    self.value()
                }
            }
        )+
    };
}

scoped_identity_fields!(
    ArtifactReferenceIdentity,
    ArtifactReadIdentity,
    ArtifactCommandIdentity,
    ArtifactStagingIdentity,
);

#[cfg(test)]
mod quota_head_tests {
    use super::*;

    fn authoritative(
        scope: ArtifactQuotaScope,
        projection: ArtifactQuotaProjection,
        limits: ArtifactStoreLimits,
    ) -> ArtifactQuotaAuthorityProjection {
        ArtifactQuotaAuthorityProjection {
            scope,
            projection,
            limit_snapshot_digest: limits.limit_snapshot_digest().expect("limit snapshot"),
        }
    }

    #[test]
    fn fixed_owner_quota_head_is_monotonic_predecessor_and_limit_bound() {
        let scope =
            ArtifactQuotaScope::Store(ArtifactStoreIdentity::new("store-head").expect("store"));
        let limits = ArtifactStoreLimits::hard_maximums();
        let first_projection = ArtifactQuotaProjection::zero()
            .with_value(ArtifactLimitKind::CommandsPerStore, 1)
            .expect("projection");
        let first =
            ArtifactQuotaHead::initial(authoritative(scope.clone(), first_projection, limits))
                .expect("first head");
        let same = ArtifactQuotaHead::initial(authoritative(
            scope.clone(),
            first.projection().clone(),
            limits,
        ))
        .expect("deterministic first head");

        assert_eq!(first, same);
        assert_eq!(first.producer_id(), ARTIFACT_STORE_PRODUCER_ID);
        assert_eq!(first.producer_version(), ARTIFACT_STORE_PRODUCER_VERSION);
        assert_eq!(first.runtime(), RuntimeKind::Fake);
        assert_eq!(first.revision().get(), 1);
        assert_eq!(
            first.limit_snapshot_digest(),
            &limits.limit_snapshot_digest().expect("limit snapshot")
        );
        assert_ne!(
            first.predecessor_head_digest(),
            first.transition_tail_digest()
        );
        assert_ne!(first.transition_tail_digest(), first.head_digest());

        let next_projection = first
            .projection()
            .clone()
            .with_value(ArtifactLimitKind::CommandsPerStore, 2)
            .expect("next projection");
        let next = first
            .successor(authoritative(scope.clone(), next_projection, limits))
            .expect("next head");
        assert_eq!(next.revision().get(), 2);
        assert_eq!(next.predecessor_head_digest(), first.head_digest());
        assert_ne!(
            next.transition_tail_digest(),
            first.transition_tail_digest()
        );
        assert_ne!(next.head_digest(), first.head_digest());

        let other_scope =
            ArtifactQuotaScope::Store(ArtifactStoreIdentity::new("store-other").expect("store"));
        assert_eq!(
            next.successor(authoritative(
                other_scope,
                next.projection().clone(),
                limits,
            )),
            Err(ArtifactQuotaError::QuotaHeadMismatch)
        );

        let tighter = limits
            .tighten(ArtifactLimitKind::CommandsPerStore, 10)
            .expect("tighter limits");
        assert_eq!(
            next.successor(authoritative(scope, next.projection().clone(), tighter)),
            Err(ArtifactQuotaError::QuotaHeadMismatch)
        );
    }
}
