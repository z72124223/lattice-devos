//! Bounded immutable evidence objects used by the managed foreman adapter.

use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize};
use lattice_contracts::{ContentDigest, ProjectId, task_ingress_text_contains_recognized_secret};
use sha2::{Digest, Sha256};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

/// Maximum exact bytes retained by one managed evidence object.
pub const MAX_MANAGED_EVIDENCE_BYTES: usize = 1_048_576;
/// Persistence schema for one managed evidence descriptor.
pub const MANAGED_EVIDENCE_RECORD_SCHEMA: &str = "lattice.artifact.managed-evidence/1.0";

/// Closed evidence classes retained by the Phase-4 foreman.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedEvidenceKind {
    WorkerLifecycle,
    GitSnapshot,
    VerificationResult,
    ReviewResult,
    ResourceObservation,
}

impl ManagedEvidenceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkerLifecycle => "WORKER_LIFECYCLE",
            Self::GitSnapshot => "GIT_SNAPSHOT",
            Self::VerificationResult => "VERIFICATION_RESULT",
            Self::ReviewResult => "REVIEW_RESULT",
            Self::ResourceObservation => "RESOURCE_OBSERVATION",
        }
    }

    /// Parses a persisted closed value.
    ///
    /// # Errors
    ///
    /// Unknown values fail closed.
    pub fn parse(value: &str) -> Result<Self, ManagedEvidenceError> {
        match value {
            "WORKER_LIFECYCLE" => Ok(Self::WorkerLifecycle),
            "GIT_SNAPSHOT" => Ok(Self::GitSnapshot),
            "VERIFICATION_RESULT" => Ok(Self::VerificationResult),
            "REVIEW_RESULT" => Ok(Self::ReviewResult),
            "RESOURCE_OBSERVATION" => Ok(Self::ResourceObservation),
            _ => Err(ManagedEvidenceError::MalformedField),
        }
    }
}

/// Fail-closed managed evidence validation errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedEvidenceError {
    MalformedField,
    BytesLimitExceeded,
    ForbiddenContent,
    DigestMismatch,
    Canonicalization,
}

impl fmt::Display for ManagedEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "MANAGED_EVIDENCE_{self:?}")
    }
}

impl Error for ManagedEvidenceError {}

/// Fully constructed, untrusted managed evidence input.
#[derive(Clone, Eq, PartialEq)]
pub struct ManagedEvidenceInput {
    project_id: ProjectId,
    task_ref: ContentDigest,
    attempt: u8,
    kind: ManagedEvidenceKind,
    media_type: String,
    payload_schema: String,
    producer_id: String,
    producer_version: String,
    producer_digest: ContentDigest,
    created_at: String,
    bytes: Vec<u8>,
}

impl fmt::Debug for ManagedEvidenceInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedEvidenceInput")
            .field("project_id", &self.project_id)
            .field("task_ref", &self.task_ref)
            .field("attempt", &self.attempt)
            .field("kind", &self.kind)
            .field("media_type", &self.media_type)
            .field("payload_schema", &self.payload_schema)
            .field("producer_id", &self.producer_id)
            .field("producer_version", &self.producer_version)
            .field("producer_digest", &self.producer_digest)
            .field("created_at", &self.created_at)
            .field("byte_length", &self.bytes.len())
            .finish()
    }
}

impl ManagedEvidenceInput {
    /// Constructs one bounded input without performing I/O.
    ///
    /// # Errors
    ///
    /// Rejects malformed metadata, non-canonical time, zero digests, oversized
    /// bytes, and common secret/credential-bearing text shapes.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_id: ProjectId,
        task_ref: ContentDigest,
        attempt: u8,
        kind: ManagedEvidenceKind,
        media_type: impl Into<String>,
        payload_schema: impl Into<String>,
        producer_id: impl Into<String>,
        producer_version: impl Into<String>,
        producer_digest: ContentDigest,
        created_at: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<Self, ManagedEvidenceError> {
        let input = Self {
            project_id,
            task_ref,
            attempt,
            kind,
            media_type: media_type.into(),
            payload_schema: payload_schema.into(),
            producer_id: producer_id.into(),
            producer_version: producer_version.into(),
            producer_digest,
            created_at: created_at.into(),
            bytes,
        };
        input.validate()?;
        Ok(input)
    }

    fn validate(&self) -> Result<(), ManagedEvidenceError> {
        if !(1..=3).contains(&self.attempt)
            || [
                &self.media_type,
                &self.payload_schema,
                &self.producer_id,
                &self.producer_version,
            ]
            .into_iter()
            .any(|value| !valid_bounded_identifier(value))
            || is_zero_digest(&self.task_ref)
            || is_zero_digest(&self.producer_digest)
            || !canonical_utc(&self.created_at)
        {
            return Err(ManagedEvidenceError::MalformedField);
        }
        if self.bytes.len() > MAX_MANAGED_EVIDENCE_BYTES {
            return Err(ManagedEvidenceError::BytesLimitExceeded);
        }
        if [
            self.project_id.as_str(),
            self.media_type.as_str(),
            self.payload_schema.as_str(),
            self.producer_id.as_str(),
            self.producer_version.as_str(),
            self.created_at.as_str(),
        ]
        .into_iter()
        .any(task_ingress_text_contains_recognized_secret)
            || contains_forbidden_content(&self.bytes)
        {
            return Err(ManagedEvidenceError::ForbiddenContent);
        }
        Ok(())
    }
}

/// Verified immutable evidence bytes plus their domain-separated descriptor.
#[derive(Clone, Eq, PartialEq)]
pub struct VerifiedManagedEvidence {
    input: ManagedEvidenceInput,
    content_digest: ContentDigest,
    descriptor_digest: ContentDigest,
}

impl fmt::Debug for VerifiedManagedEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedManagedEvidence")
            .field("project_id", &self.input.project_id)
            .field("task_ref", &self.input.task_ref)
            .field("attempt", &self.input.attempt)
            .field("kind", &self.input.kind)
            .field("byte_length", &self.input.bytes.len())
            .field("content_digest", &self.content_digest)
            .field("descriptor_digest", &self.descriptor_digest)
            .finish()
    }
}

impl VerifiedManagedEvidence {
    /// Verifies exact bytes and constructs their descriptor.
    ///
    /// # Errors
    ///
    /// Returns the same validation failures as [`ManagedEvidenceInput`] or a
    /// canonical hashing failure.
    pub fn new(input: ManagedEvidenceInput) -> Result<Self, ManagedEvidenceError> {
        input.validate()?;
        let content_digest = sha256_digest(&input.bytes)?;
        let descriptor_digest = descriptor_digest(&input, &content_digest)?;
        Ok(Self {
            input,
            content_digest,
            descriptor_digest,
        })
    }

    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.input.project_id
    }

    #[must_use]
    pub const fn task_ref(&self) -> &ContentDigest {
        &self.input.task_ref
    }

    #[must_use]
    pub const fn attempt(&self) -> u8 {
        self.input.attempt
    }

    #[must_use]
    pub const fn kind(&self) -> ManagedEvidenceKind {
        self.input.kind
    }

    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.input.media_type
    }

    #[must_use]
    pub fn payload_schema(&self) -> &str {
        &self.input.payload_schema
    }

    #[must_use]
    pub fn producer_id(&self) -> &str {
        &self.input.producer_id
    }

    #[must_use]
    pub fn producer_version(&self) -> &str {
        &self.input.producer_version
    }

    #[must_use]
    pub const fn producer_digest(&self) -> &ContentDigest {
        &self.input.producer_digest
    }

    #[must_use]
    pub fn created_at(&self) -> &str {
        &self.input.created_at
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.input.bytes
    }

    #[must_use]
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    #[must_use]
    pub const fn descriptor_digest(&self) -> &ContentDigest {
        &self.descriptor_digest
    }

    /// Returns canonical descriptor bytes; raw evidence bytes are excluded.
    ///
    /// # Errors
    ///
    /// Fails only if canonical encoding rejects the frozen descriptor.
    pub fn canonical_descriptor_bytes(&self) -> Result<Vec<u8>, ManagedEvidenceError> {
        canonicalize(&descriptor_value(&self.input, &self.content_digest))
            .map(lattice_cjson::CanonicalBytes::into_vec)
            .map_err(|_| ManagedEvidenceError::Canonicalization)
    }

    #[must_use]
    pub fn to_untrusted(&self) -> UntrustedManagedEvidence {
        UntrustedManagedEvidence {
            record_schema: MANAGED_EVIDENCE_RECORD_SCHEMA.to_owned(),
            input: self.input.clone(),
            content_digest: self.content_digest.clone(),
            descriptor_digest: self.descriptor_digest.clone(),
        }
    }
}

/// Persistence-shaped evidence that must be reverified after loading.
#[derive(Clone, Eq, PartialEq)]
pub struct UntrustedManagedEvidence {
    record_schema: String,
    input: ManagedEvidenceInput,
    content_digest: ContentDigest,
    descriptor_digest: ContentDigest,
}

impl fmt::Debug for UntrustedManagedEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UntrustedManagedEvidence")
            .field("record_schema", &self.record_schema)
            .field("input", &self.input)
            .field("content_digest", &self.content_digest)
            .field("descriptor_digest", &self.descriptor_digest)
            .finish()
    }
}

impl UntrustedManagedEvidence {
    /// Rehydrates one explicitly untrusted persistence row. Call
    /// [`verify_untrusted_managed_evidence`] before using it as evidence.
    #[must_use]
    pub fn new(
        record_schema: impl Into<String>,
        input: ManagedEvidenceInput,
        content_digest: ContentDigest,
        descriptor_digest: ContentDigest,
    ) -> Self {
        Self {
            record_schema: record_schema.into(),
            input,
            content_digest,
            descriptor_digest,
        }
    }

    #[must_use]
    pub fn with_bytes(mut self, bytes: Vec<u8>) -> Self {
        self.input.bytes = bytes;
        self
    }

    #[must_use]
    pub fn record_schema(&self) -> &str {
        &self.record_schema
    }

    #[must_use]
    pub const fn input(&self) -> &ManagedEvidenceInput {
        &self.input
    }

    #[must_use]
    pub const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    #[must_use]
    pub const fn descriptor_digest(&self) -> &ContentDigest {
        &self.descriptor_digest
    }
}

/// Recomputes both byte and descriptor digests for a loaded row.
///
/// # Errors
///
/// Unknown schema, invalid bytes/metadata, or either digest substitution fails
/// closed.
pub fn verify_untrusted_managed_evidence(
    value: &UntrustedManagedEvidence,
) -> Result<VerifiedManagedEvidence, ManagedEvidenceError> {
    if value.record_schema != MANAGED_EVIDENCE_RECORD_SCHEMA {
        return Err(ManagedEvidenceError::MalformedField);
    }
    let verified = VerifiedManagedEvidence::new(value.input.clone())?;
    if verified.content_digest != value.content_digest
        || verified.descriptor_digest != value.descriptor_digest
    {
        return Err(ManagedEvidenceError::DigestMismatch);
    }
    Ok(verified)
}

fn descriptor_digest(
    input: &ManagedEvidenceInput,
    content_digest: &ContentDigest,
) -> Result<ContentDigest, ManagedEvidenceError> {
    let domain = HashDomain::new("lattice.artifact.managed-evidence", "1.0")
        .map_err(|_| ManagedEvidenceError::Canonicalization)?;
    let digest = canonical_sha256(&domain, &descriptor_value(input, content_digest))
        .map_err(|_| ManagedEvidenceError::Canonicalization)?;
    ContentDigest::from_sha256(digest.to_hex()).map_err(|_| ManagedEvidenceError::Canonicalization)
}

fn descriptor_value(
    input: &ManagedEvidenceInput,
    content_digest: &ContentDigest,
) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "record_schema".to_owned(),
            CanonicalValue::String(MANAGED_EVIDENCE_RECORD_SCHEMA.to_owned()),
        ),
        (
            "project_id".to_owned(),
            CanonicalValue::String(input.project_id.as_str().to_owned()),
        ),
        (
            "task_ref".to_owned(),
            CanonicalValue::String(input.task_ref.as_str().to_owned()),
        ),
        (
            "attempt".to_owned(),
            CanonicalValue::String(input.attempt.to_string()),
        ),
        (
            "kind".to_owned(),
            CanonicalValue::String(input.kind.as_str().to_owned()),
        ),
        (
            "media_type".to_owned(),
            CanonicalValue::String(input.media_type.clone()),
        ),
        (
            "payload_schema".to_owned(),
            CanonicalValue::String(input.payload_schema.clone()),
        ),
        (
            "producer_id".to_owned(),
            CanonicalValue::String(input.producer_id.clone()),
        ),
        (
            "producer_version".to_owned(),
            CanonicalValue::String(input.producer_version.clone()),
        ),
        (
            "producer_digest".to_owned(),
            CanonicalValue::String(input.producer_digest.as_str().to_owned()),
        ),
        (
            "created_at".to_owned(),
            CanonicalValue::String(input.created_at.clone()),
        ),
        (
            "byte_length".to_owned(),
            CanonicalValue::String(input.bytes.len().to_string()),
        ),
        (
            "content_digest".to_owned(),
            CanonicalValue::String(content_digest.as_str().to_owned()),
        ),
    ])
}

fn sha256_digest(bytes: &[u8]) -> Result<ContentDigest, ManagedEvidenceError> {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let mut output = String::with_capacity(64);
    for byte in hasher.finalize() {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| ManagedEvidenceError::Canonicalization)?;
    }
    ContentDigest::from_sha256(output).map_err(|_| ManagedEvidenceError::Canonicalization)
}

fn valid_bounded_identifier(value: &str) -> bool {
    (1..=256).contains(&value.len())
        && value.trim() == value
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'+' | b'-')
        })
}

fn canonical_utc(value: &str) -> bool {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .filter(|parsed| parsed.offset() == UtcOffset::UTC)
        .and_then(|parsed| parsed.format(&Rfc3339).ok())
        .is_some_and(|formatted| formatted == value)
}

fn is_zero_digest(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}

fn contains_forbidden_content(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    if task_ingress_text_contains_recognized_secret(text.as_ref()) {
        return true;
    }

    // Keep a raw-byte scan as a second boundary: evidence can be binary and
    // must not rely on successful UTF-8 decoding to reject credential URLs or
    // the long-standing assignment/private-key shapes.
    let folded = bytes.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>();
    let contains = |needle: &[u8]| folded.windows(needle.len()).any(|window| window == needle);
    if [
        b"authorization: bearer ".as_slice(),
        b"bearer ".as_slice(),
        b"password=".as_slice(),
        b"password:".as_slice(),
        b"token=".as_slice(),
        b"token:".as_slice(),
        b"api_key".as_slice(),
        b"api-key".as_slice(),
        b"private_key".as_slice(),
        b"private-key".as_slice(),
    ]
    .into_iter()
    .any(contains)
        || (contains(b"-----begin") && contains(b"private key-----"))
    {
        return true;
    }

    folded.windows(3).enumerate().any(|(start, window)| {
        window == b"://"
            && folded[start + 3..]
                .iter()
                .take_while(|byte| !matches!(**byte, b'/' | b'?' | b'#' | b'\n' | b'\r'))
                .any(|byte| *byte == b'@')
    })
}
