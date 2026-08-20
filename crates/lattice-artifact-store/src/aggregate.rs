//! Atomic composition of lifecycle, history, quota, and fake byte mechanics.
//!
//! The lower-level modules are pure mechanisms. This module is the only
//! public fake owner allowed to turn those mechanisms into terminal Artifact
//! Store commands.

#[path = "aggregate/snapshot_restore.rs"]
mod snapshot_restore;

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;

use crate::history::{
    ArtifactCommandExecutionDisposition, ArtifactCommandHistory, ArtifactCommandKind,
    ArtifactCommandOutcome, ArtifactCommandReceipt, ArtifactCommandRequest,
    ArtifactCommandStorageKey, ArtifactCommandTerminalProjection, ArtifactHistoryError,
};
use crate::quota::{
    ArtifactCommandIdentity, ArtifactCommandQuotaRecord, ArtifactQuotaError, ArtifactQuotaHead,
    ArtifactQuotaScope, ArtifactQuotaSnapshot, ArtifactReadQuotaRecord,
    ArtifactReferenceQuotaRecord, ArtifactStagingIdentity, ArtifactStagingReservation,
    ArtifactStagingState, ArtifactStagingTerminalEvidence, ArtifactStoreIdentity,
    FakeArtifactStagingTerminalAuthority,
};
use crate::quota_owner::{ArtifactQuotaHeadSet, ArtifactQuotaOwnerError};
use crate::semantics::{
    ArtifactDeletePlan, ArtifactIntegratedHeadEvidence, ArtifactLifecycleError,
    ArtifactLifecycleState, ArtifactReconciliationResult, FakeArtifactAuthorityDirectory,
    FakeArtifactBytes, FakeDeleteOutcome, artifact_manifest_digest,
    authority_receipt_canonical_value,
};
use crate::{ArtifactLimitKind, ArtifactStoreLimits};
use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactAuthorityHead,
    ArtifactAuthorityReceipt, ArtifactAvailability, ArtifactObjectIdentity, ArtifactObjectKey,
    ArtifactReadAuthorityPair, ArtifactReadClosureEvidencePair, ArtifactReadHead,
    ArtifactReferenceAuthorityPair, ArtifactReferenceHead, ArtifactReferenceManifest,
    ArtifactSweepAuthorityPair, ContentDigest, RuntimeKind, TaskId,
};
use sha2::{Digest, Sha256};

const AGGREGATE_RECEIPT_DOMAIN: &str = "lattice.artifact.aggregate-command-receipt";
const AGGREGATE_STATE_DOMAIN: &str = "lattice.artifact.aggregate-command-state";
const ABSENT_STATE_DOMAIN: &str = "lattice.artifact.aggregate-absent-object";
const DENIAL_RESULT_DOMAIN: &str = "lattice.artifact.aggregate-denial-result";
const HASH_VERSION: &str = "1.0";

/// Immutable fixed-owner terminal result of one atomic fake command.
///
/// The history receipt binds the exact sanitized request and idempotency
/// chain. The optional lifecycle receipt is present only when a semantic
/// object transition applied. Quota and aggregate commitments bind the state
/// after the terminal command, including a zero-mutation denial whose history
/// row still consumes quota.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactStoreCommandReceipt {
    history: ArtifactCommandReceipt,
    lifecycle: Option<ArtifactAuthorityReceipt>,
    authority_input_digest: ContentDigest,
    quota_checkpoint_digest: ContentDigest,
    aggregate_state_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl ArtifactStoreCommandReceipt {
    fn new(
        history: ArtifactCommandReceipt,
        lifecycle: Option<ArtifactAuthorityReceipt>,
        authority_input_digest: ContentDigest,
        quota_checkpoint_digest: ContentDigest,
        aggregate_state_digest: ContentDigest,
    ) -> Result<Self, ArtifactStoreAggregateError> {
        let lifecycle_digest = lifecycle
            .as_ref()
            .map_or("NONE", |receipt| receipt.receipt_digest().as_str());
        let value = CanonicalValue::Object(vec![
            string("producer_id", ARTIFACT_STORE_PRODUCER_ID),
            string("producer_version", ARTIFACT_STORE_PRODUCER_VERSION),
            string("runtime", "FAKE"),
            string("history_receipt_digest", history.receipt_digest().as_str()),
            string("outcome", command_outcome_text(history.outcome())),
            string("lifecycle_receipt_digest", lifecycle_digest),
            string("authority_input_digest", authority_input_digest.as_str()),
            string("quota_checkpoint_digest", quota_checkpoint_digest.as_str()),
            string("aggregate_state_digest", aggregate_state_digest.as_str()),
        ]);
        let receipt_digest = digest(AGGREGATE_RECEIPT_DOMAIN, &value)?;
        Ok(Self {
            history,
            lifecycle,
            authority_input_digest,
            quota_checkpoint_digest,
            aggregate_state_digest,
            receipt_digest,
        })
    }

    /// Compile-time fixed Artifact Store producer.
    #[must_use]
    pub const fn producer_id(&self) -> &'static str {
        ARTIFACT_STORE_PRODUCER_ID
    }

    /// Compile-time fixed Artifact Store producer version.
    #[must_use]
    pub const fn producer_version(&self) -> &'static str {
        ARTIFACT_STORE_PRODUCER_VERSION
    }

    /// This TASK-016 composition is visibly fake.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        RuntimeKind::Fake
    }

    /// Exact request, outcome, and object-chain receipt.
    #[must_use]
    pub const fn history(&self) -> &ArtifactCommandReceipt {
        &self.history
    }

    /// Applied object transition, absent for terminal denials or staging-only
    /// transitions.
    #[must_use]
    pub const fn lifecycle(&self) -> Option<&ArtifactAuthorityReceipt> {
        self.lifecycle.as_ref()
    }

    /// Digest of the complete typed external authority/current-head input.
    #[must_use]
    pub const fn authority_input_digest(&self) -> &ContentDigest {
        &self.authority_input_digest
    }

    /// Commitment to all affected 30-field quota heads.
    #[must_use]
    pub const fn quota_checkpoint_digest(&self) -> &ContentDigest {
        &self.quota_checkpoint_digest
    }

    /// Commitment to lifecycle, staging, history, and quota state after the
    /// terminal command.
    #[must_use]
    pub const fn aggregate_state_digest(&self) -> &ContentDigest {
        &self.aggregate_state_digest
    }

    /// Final non-circular aggregate receipt digest.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

/// Returned disposition and immutable terminal receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactStoreCommandExecution {
    disposition: ArtifactCommandExecutionDisposition,
    receipt: ArtifactStoreCommandReceipt,
}

impl ArtifactStoreCommandExecution {
    /// Whether this call appended a terminal row or returned an exact prior
    /// result before evaluating currentness or time.
    #[must_use]
    pub const fn disposition(&self) -> ArtifactCommandExecutionDisposition {
        self.disposition
    }

    /// Byte-identical terminal aggregate receipt.
    #[must_use]
    pub const fn receipt(&self) -> &ArtifactStoreCommandReceipt {
        &self.receipt
    }
}

/// Fail-closed aggregate composition failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactStoreAggregateError {
    /// Canonical hashing failed.
    Canonicalization,
    /// An exact command key was reused with changed sanitized content.
    CommandIdReuse,
    /// A command identity or sanitized metadata shape is invalid.
    InvalidCommand,
    /// The internal lifecycle, history, quota, and terminal maps disagree.
    CorruptState,
    /// A checked signed-BIGINT-compatible counter was exhausted.
    CounterExhausted,
    /// A new terminal command cannot be retained inside command/history quota.
    QuotaExhausted,
    /// A read-only lifecycle query failed without recording a command.
    Lifecycle(ArtifactLifecycleError),
}

impl ArtifactStoreAggregateError {
    /// Stable non-secret diagnostic code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Canonicalization => "ARTIFACT_AGGREGATE_CANONICALIZATION",
            Self::CommandIdReuse => "ARTIFACT_AGGREGATE_COMMAND_ID_REUSE",
            Self::InvalidCommand => "ARTIFACT_AGGREGATE_INVALID_COMMAND",
            Self::CorruptState => "ARTIFACT_AGGREGATE_CORRUPT_STATE",
            Self::CounterExhausted => "ARTIFACT_AGGREGATE_COUNTER_EXHAUSTED",
            Self::QuotaExhausted => "ARTIFACT_AGGREGATE_QUOTA_EXHAUSTED",
            Self::Lifecycle(error) => error.code(),
        }
    }
}

impl fmt::Display for ArtifactStoreAggregateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ArtifactStoreAggregateError {}

/// Result of a digest-verified fake read or its atomic expiry transition.
#[derive(Clone, Eq, PartialEq)]
pub enum ArtifactVerifiedReadExecution {
    /// Exact bytes copied only after active-claim, time, length, and digest
    /// verification.
    Bytes(Vec<u8>),
    /// Exact expiry-command result. Consumers must inspect the embedded
    /// receipt outcome because an exact retry can be either applied or denied.
    ExpiryCommand(Box<ArtifactStoreCommandExecution>),
}

impl fmt::Debug for ArtifactVerifiedReadExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes(bytes) => formatter
                .debug_struct("Bytes")
                .field("byte_length", &bytes.len())
                .field("bytes", &"[ELIDED]")
                .finish(),
            Self::ExpiryCommand(execution) => formatter
                .debug_tuple("ExpiryCommand")
                .field(execution)
                .finish(),
        }
    }
}

/// Visibly non-durable, single-writer Artifact Store owner used by tests and
/// local composition before `PostgreSQL` and filesystem adapters exist.
#[derive(Clone, Eq, PartialEq)]
pub struct FakeArtifactStore {
    store_id: ArtifactStoreIdentity,
    limits: ArtifactStoreLimits,
    lifecycle: ArtifactLifecycleState,
    bytes: FakeArtifactBytes,
    history: ArtifactCommandHistory,
    staging: HashMap<ArtifactStagingIdentity, ArtifactStagingReservation>,
    command_tasks: HashMap<ArtifactCommandStorageKey, TaskId>,
    quota_head_set: Option<ArtifactQuotaHeadSet>,
    retired_quota_objects: HashSet<ArtifactObjectIdentity>,
    terminal_receipts: HashMap<ArtifactCommandStorageKey, ArtifactStoreCommandReceipt>,
}

impl fmt::Debug for FakeArtifactStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FakeArtifactStore")
            .field("store_id", &self.store_id)
            .field("limits", &self.limits)
            .field("object_count", &self.lifecycle.object_count())
            .field("bytes", &"[ELIDED]")
            .field("terminal_command_count", &self.terminal_receipts.len())
            .field("staging_reservation_count", &self.staging.len())
            .field(
                "quota_head_count",
                &self
                    .quota_head_set
                    .as_ref()
                    .map_or(0, |heads| heads.sorted_heads().len()),
            )
            .field(
                "retired_quota_object_count",
                &self.retired_quota_objects.len(),
            )
            .finish_non_exhaustive()
    }
}

fn digest(
    schema_id: &str,
    value: &CanonicalValue,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let domain = HashDomain::new(schema_id, HASH_VERSION)
        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?;
    let hash = canonical_sha256(&domain, value)
        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?;
    ContentDigest::from_sha256(hash.to_hex())
        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)
}

fn string(name: &str, value: impl Into<String>) -> (String, CanonicalValue) {
    (name.to_owned(), CanonicalValue::String(value.into()))
}

fn request_source_string<'a>(
    request: &'a ArtifactCommandRequest,
    field: &str,
) -> Result<&'a str, ArtifactStoreAggregateError> {
    let CanonicalValue::Object(fields) = request.source() else {
        return Err(ArtifactStoreAggregateError::CorruptState);
    };
    fields
        .iter()
        .find_map(|(name, value)| {
            (name == field).then_some(value).and_then(|value| {
                if let CanonicalValue::String(value) = value {
                    Some(value.as_str())
                } else {
                    None
                }
            })
        })
        .ok_or(ArtifactStoreAggregateError::CorruptState)
}

fn runtime_text(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Fake => "FAKE",
        RuntimeKind::Live => "LIVE",
    }
}

fn command_outcome_text(outcome: ArtifactCommandOutcome) -> &'static str {
    match outcome {
        ArtifactCommandOutcome::Applied => "APPLIED",
        ArtifactCommandOutcome::Denied => "DENIED",
    }
}

fn reference_authority_input_digest(
    pair: &ArtifactReferenceAuthorityPair,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let binding = pair.receipt().binding();
    digest(
        "lattice.artifact.aggregate.reference-authority-input",
        &CanonicalValue::Object(vec![
            string("version", pair.receipt().version().to_string()),
            string("owner_kind", binding.owner_kind().as_str()),
            string("producer_id", binding.producer_id()),
            string("producer_version", binding.producer_version()),
            string("runtime", runtime_text(binding.runtime())),
            string("owner_record_id", binding.owner_record_id()),
            string("owner_revision", binding.owner_revision().get().to_string()),
            string("status", binding.status().as_str()),
            string("action", binding.action().as_str()),
            string("project_id", binding.project_id().as_str()),
            string("task_id", binding.task_id().as_str()),
            string("algorithm", binding.object().key().algorithm()),
            string(
                "content_digest",
                binding.object().key().content_digest().as_str(),
            ),
            string(
                "object_generation",
                binding.object().generation().get().to_string(),
            ),
            string("reference_id", binding.reference_id()),
            string("observation_digest", binding.observation_digest().as_str()),
            string(
                "authority_receipt_digest",
                pair.receipt().receipt_digest().as_str(),
            ),
            string(
                "authority_current_head_digest",
                pair.current_head().receipt_digest().as_str(),
            ),
        ]),
    )
}

fn read_authority_input_digest(
    pair: &ArtifactReadAuthorityPair,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let binding = pair.receipt().binding();
    digest(
        "lattice.artifact.aggregate.read-authority-input",
        &CanonicalValue::Object(vec![
            string("version", pair.receipt().version().to_string()),
            string("owner_kind", binding.owner_kind().as_str()),
            string("producer_id", binding.producer_id()),
            string("producer_version", binding.producer_version()),
            string("runtime", runtime_text(binding.runtime())),
            string("owner_record_id", binding.owner_record_id()),
            string("owner_revision", binding.owner_revision().get().to_string()),
            string("status", binding.status().as_str()),
            string("action", binding.action().as_str()),
            string("project_id", binding.project_id().as_str()),
            string("task_id", binding.task_id().as_str()),
            string("algorithm", binding.object().key().algorithm()),
            string(
                "content_digest",
                binding.object().key().content_digest().as_str(),
            ),
            string(
                "object_generation",
                binding.object().generation().get().to_string(),
            ),
            string("read_claim_id", binding.read_claim_id()),
            string("observation_digest", binding.observation_digest().as_str()),
            string(
                "authority_receipt_digest",
                pair.receipt().receipt_digest().as_str(),
            ),
            string(
                "authority_current_head_digest",
                pair.current_head().receipt_digest().as_str(),
            ),
        ]),
    )
}

fn read_closure_input_digest(
    pair: &ArtifactReadClosureEvidencePair,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let binding = pair.receipt().binding();
    digest(
        "lattice.artifact.aggregate.read-closure-input",
        &CanonicalValue::Object(vec![
            string("version", pair.receipt().version().to_string()),
            string("producer_id", binding.producer_id()),
            string("producer_version", binding.producer_version()),
            string("runtime", runtime_text(binding.runtime())),
            string("evidence_record_id", binding.evidence_record_id()),
            string(
                "evidence_revision",
                binding.evidence_revision().get().to_string(),
            ),
            string("status", binding.status().as_str()),
            string("kind", binding.kind().as_str()),
            string("project_id", binding.project_id().as_str()),
            string("task_id", binding.task_id().as_str()),
            string("algorithm", binding.object().key().algorithm()),
            string(
                "content_digest",
                binding.object().key().content_digest().as_str(),
            ),
            string(
                "object_generation",
                binding.object().generation().get().to_string(),
            ),
            string("read_claim_id", binding.read_claim_id()),
            string("holder_id", binding.holder_id()),
            string("daemon_instance_id", binding.daemon_instance_id()),
            string("daemon_epoch", binding.daemon_epoch().get().to_string()),
            string("observed_at", binding.observed_at()),
            string("observation_digest", binding.observation_digest().as_str()),
            string(
                "closure_receipt_digest",
                pair.receipt().receipt_digest().as_str(),
            ),
            string(
                "closure_current_head_digest",
                pair.current_head().receipt_digest().as_str(),
            ),
        ]),
    )
}

fn sweep_authority_input_digest(
    pair: &ArtifactSweepAuthorityPair,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let binding = pair.receipt().binding();
    digest(
        "lattice.artifact.aggregate.sweep-authority-input",
        &CanonicalValue::Object(vec![
            string("version", pair.receipt().version().to_string()),
            string("producer_id", binding.producer_id()),
            string("producer_version", binding.producer_version()),
            string("runtime", runtime_text(binding.runtime())),
            string("owner_record_id", binding.owner_record_id()),
            string("owner_revision", binding.owner_revision().get().to_string()),
            string("status", binding.status().as_str()),
            string("action", binding.action().as_str()),
            string("project_id", binding.object().key().project_id().as_str()),
            string("algorithm", binding.object().key().algorithm()),
            string(
                "content_digest",
                binding.object().key().content_digest().as_str(),
            ),
            string(
                "object_generation",
                binding.object().generation().get().to_string(),
            ),
            string(
                "zero_reference_set_digest",
                binding.zero_reference_set_digest().as_str(),
            ),
            string(
                "zero_read_set_digest",
                binding.zero_read_set_digest().as_str(),
            ),
            string(
                "quota_projection_digest",
                binding.quota_projection_digest().as_str(),
            ),
            string("retention_observed_at", binding.retention_observed_at()),
            string("grace_until", binding.grace_until()),
            string(
                "root_identity_digest",
                binding.root_identity_digest().as_str(),
            ),
            string("daemon_instance_id", binding.daemon_instance_id()),
            string("daemon_epoch", binding.daemon_epoch().get().to_string()),
            string("runtime_admission", binding.runtime_admission().as_str()),
            string("observation_digest", binding.observation_digest().as_str()),
            string(
                "authority_receipt_digest",
                pair.receipt().receipt_digest().as_str(),
            ),
            string(
                "authority_current_head_digest",
                pair.current_head().receipt_digest().as_str(),
            ),
        ]),
    )
}

fn artifact_current_head_input_digest(
    head: &ArtifactAuthorityHead,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let object = head.object();
    let reference = head
        .reference()
        .map(reference_head_input_digest)
        .transpose()?;
    let read = head.read().map(read_head_input_digest).transpose()?;
    digest(
        "lattice.artifact.aggregate.current-head-input",
        &CanonicalValue::Object(vec![
            string("version", head.version().to_string()),
            string("producer_id", head.producer_id()),
            string("producer_version", head.producer_version()),
            string("runtime", runtime_text(head.runtime())),
            string("project_id", object.object().key().project_id().as_str()),
            string("algorithm", object.object().key().algorithm()),
            string(
                "content_digest",
                object.object().key().content_digest().as_str(),
            ),
            string(
                "object_generation",
                object.object().generation().get().to_string(),
            ),
            string("object_revision", object.revision().get().to_string()),
            string("availability", object.availability().as_str()),
            string("byte_length", object.byte_length().get().to_string()),
            string(
                "active_reference_count",
                object.active_reference_count().get().to_string(),
            ),
            string(
                "active_reference_set_digest",
                object.active_reference_set_digest().as_str(),
            ),
            string("sweep_not_before", object.sweep_not_before()),
            string(
                "active_read_count",
                object.active_read_count().get().to_string(),
            ),
            string(
                "active_read_set_digest",
                object.active_read_set_digest().as_str(),
            ),
            string("delete_status", object.delete_status().as_str()),
            string(
                "delete_claim_token",
                object.delete_claim_token().unwrap_or("NONE"),
            ),
            string(
                "task_quota_projection_digest",
                object.task_quota_projection_digest().as_str(),
            ),
            string(
                "project_quota_projection_digest",
                object.project_quota_projection_digest().as_str(),
            ),
            string(
                "store_quota_projection_digest",
                object.store_quota_projection_digest().as_str(),
            ),
            string(
                "staging_quota_projection_digest",
                object.staging_quota_projection_digest().as_str(),
            ),
            string(
                "command_high_water",
                object.command_high_water().get().to_string(),
            ),
            string("command_tail_digest", object.command_tail_digest().as_str()),
            string(
                "object_transition_digest",
                object.transition_digest().as_str(),
            ),
            string(
                "reference_head_digest",
                reference.as_ref().map_or("NONE", ContentDigest::as_str),
            ),
            string(
                "read_head_digest",
                read.as_ref().map_or("NONE", ContentDigest::as_str),
            ),
            string("observation_digest", head.observation_digest().as_str()),
            string("receipt_digest", head.receipt_digest().as_str()),
        ]),
    )
}

fn reference_head_input_digest(
    head: &ArtifactReferenceHead,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let recomputed_manifest = crate::semantics::artifact_manifest_digest(head.manifest())
        .map_err(|_| ArtifactStoreAggregateError::CorruptState)?;
    let authority = reference_authority_input_digest(head.transition_authority())?;
    digest(
        "lattice.artifact.aggregate.reference-head-input",
        &CanonicalValue::Object(vec![
            string("manifest_digest", recomputed_manifest.as_str()),
            string(
                "declared_manifest_digest",
                head.manifest().manifest_digest().as_str(),
            ),
            string("authority_input_digest", authority.as_str()),
            string("revision", head.revision().get().to_string()),
            string("status", head.status().as_str()),
            string("transition_digest", head.transition_digest().as_str()),
        ]),
    )
}

fn read_head_input_digest(
    head: &ArtifactReadHead,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let authority = read_authority_input_digest(head.authority())?;
    digest(
        "lattice.artifact.aggregate.read-head-input",
        &CanonicalValue::Object(vec![
            string("authority_input_digest", authority.as_str()),
            string("revision", head.revision().get().to_string()),
            string("status", head.status().as_str()),
            string("holder_id", head.holder_id()),
            string("acquired_at", head.acquired_at()),
            string("expires_at", head.expires_at()),
            string("transition_digest", head.transition_digest().as_str()),
        ]),
    )
}

fn semantic_receipt_state_digest(
    receipt: &ArtifactAuthorityReceipt,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    artifact_current_head_input_digest(&receipt.head())
}

fn sha256_content(bytes: &[u8]) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}")
            .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?;
    }
    ContentDigest::from_sha256(value).map_err(|_| ArtifactStoreAggregateError::Canonicalization)
}

fn staging_reservation_input_digest(
    reservation: &ArtifactStagingReservation,
    action: &str,
    evidence_digest: Option<&ContentDigest>,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let identity = reservation.identity();
    digest(
        "lattice.artifact.aggregate.staging-reservation-input",
        &CanonicalValue::Object(vec![
            string("project_id", identity.project_id().as_str()),
            string("algorithm", identity.object_key().algorithm()),
            string(
                "content_digest",
                identity.object_key().content_digest().as_str(),
            ),
            string("task_id", identity.task_id().as_str()),
            string("reservation_id", identity.value()),
            string("staging_bytes", reservation.bytes().to_string()),
            string("staging_streams", reservation.streams().to_string()),
            string("status", staging_state_text(reservation.state())),
            string("action", action),
            string(
                "evidence_digest",
                evidence_digest.map_or("NONE", ContentDigest::as_str),
            ),
        ]),
    )
}

fn staging_terminal_input_digest(
    evidence: &ArtifactStagingTerminalEvidence,
) -> Result<ContentDigest, ArtifactStoreAggregateError> {
    let receipt = evidence.receipt();
    let binding = receipt.binding();
    let identity = binding.identity();
    digest(
        "lattice.artifact.aggregate.staging-terminal-input",
        &CanonicalValue::Object(vec![
            string("producer_id", receipt.producer_id()),
            string("producer_version", receipt.producer_version()),
            string("runtime", runtime_text(receipt.runtime())),
            string("project_id", identity.project_id().as_str()),
            string("algorithm", identity.object_key().algorithm()),
            string(
                "content_digest",
                identity.object_key().content_digest().as_str(),
            ),
            string("task_id", identity.task_id().as_str()),
            string("reservation_id", identity.value()),
            string("staging_bytes", binding.bytes().to_string()),
            string("staging_streams", binding.streams().to_string()),
            string("from_status", staging_state_text(binding.from())),
            string("to_status", staging_state_text(binding.to())),
            string("observation_digest", receipt.observation_digest().as_str()),
            string("terminal_receipt_digest", receipt.receipt_digest().as_str()),
            string(
                "terminal_head_receipt_digest",
                evidence.current_head().receipt_digest().as_str(),
            ),
            string(
                "terminal_current_head_digest",
                evidence.current_head().head_digest().as_str(),
            ),
        ]),
    )
}

fn sweep_task_id() -> Result<TaskId, ArtifactStoreAggregateError> {
    TaskId::new("artifact-store-sweep").map_err(|_| ArtifactStoreAggregateError::InvalidCommand)
}

const fn delete_outcome_text(outcome: FakeDeleteOutcome) -> &'static str {
    match outcome {
        FakeDeleteOutcome::VerifiedDeleted => "VERIFIED_DELETED",
        FakeDeleteOutcome::VerifiedNoEffect => "VERIFIED_NO_EFFECT",
        FakeDeleteOutcome::Unknown => "UNKNOWN",
    }
}

const fn reconciliation_result_text(result: ArtifactReconciliationResult) -> &'static str {
    match result {
        ArtifactReconciliationResult::VerifiedAvailable => "VERIFIED_AVAILABLE",
        ArtifactReconciliationResult::VerifiedDeleted => "VERIFIED_DELETED",
    }
}

fn denial_code(value: &'static str) -> String {
    value.to_owned()
}

#[allow(clippy::needless_pass_by_value)]
fn map_history_error(error: ArtifactHistoryError) -> ArtifactStoreAggregateError {
    match error {
        ArtifactHistoryError::CommandIdReuse => ArtifactStoreAggregateError::CommandIdReuse,
        ArtifactHistoryError::CounterExhausted => ArtifactStoreAggregateError::CounterExhausted,
        ArtifactHistoryError::Canonicalization => ArtifactStoreAggregateError::Canonicalization,
        ArtifactHistoryError::InvalidCommandId
        | ArtifactHistoryError::InvalidDenialCode
        | ArtifactHistoryError::InvalidRequestSource { .. }
        | ArtifactHistoryError::RequestSourceLimit { .. }
        | ArtifactHistoryError::ForbiddenRequestField => {
            ArtifactStoreAggregateError::InvalidCommand
        }
        ArtifactHistoryError::DeniedStateChanged
        | ArtifactHistoryError::InvalidDigest { .. }
        | ArtifactHistoryError::UnknownVersion
        | ArtifactHistoryError::UnknownKind
        | ArtifactHistoryError::UnknownField
        | ArtifactHistoryError::Malformed { .. }
        | ArtifactHistoryError::Tampered
        | ArtifactHistoryError::Reordered
        | ArtifactHistoryError::Truncated
        | ArtifactHistoryError::ReplayLimit { .. }
        | ArtifactHistoryError::DuplicateCommand
        | ArtifactHistoryError::ScopeSubstitution
        | ArtifactHistoryError::HeadMismatch
        | ArtifactHistoryError::DenialTailMismatch
        | ArtifactHistoryError::CheckpointMismatch => ArtifactStoreAggregateError::CorruptState,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum QuotaCommitFailure {
    Denied(&'static str),
    Hard(ArtifactStoreAggregateError),
}

#[allow(clippy::needless_pass_by_value)]
fn map_quota_recompute_error(error: ArtifactQuotaError) -> QuotaCommitFailure {
    match error {
        ArtifactQuotaError::LimitExceeded { .. } => QuotaCommitFailure::Denied(error.code()),
        ArtifactQuotaError::CounterExhausted
        | ArtifactQuotaError::Overflow { .. }
        | ArtifactQuotaError::Underflow { .. } => {
            QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CounterExhausted)
        }
        ArtifactQuotaError::Canonicalization | ArtifactQuotaError::InvalidDigest => {
            QuotaCommitFailure::Hard(ArtifactStoreAggregateError::Canonicalization)
        }
        ArtifactQuotaError::InvalidIdentity { .. }
        | ArtifactQuotaError::InvalidNumber { .. }
        | ArtifactQuotaError::DuplicateIdentity { .. }
        | ArtifactQuotaError::UnknownObject
        | ArtifactQuotaError::ProjectMismatch
        | ArtifactQuotaError::ObjectIdentityMismatch
        | ArtifactQuotaError::ConflictingRetainedGeneration
        | ArtifactQuotaError::InconsistentObjectState
        | ArtifactQuotaError::MissingScope
        | ArtifactQuotaError::InvalidStagingTransition
        | ArtifactQuotaError::StagingEvidenceMismatch
        | ArtifactQuotaError::QuotaHeadMismatch => {
            QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CorruptState)
        }
    }
}

fn map_quota_owner_error(error: ArtifactQuotaOwnerError) -> QuotaCommitFailure {
    match error {
        ArtifactQuotaOwnerError::Quota(error) => map_quota_recompute_error(error),
        ArtifactQuotaOwnerError::Canonicalization | ArtifactQuotaOwnerError::InvalidDigest => {
            QuotaCommitFailure::Hard(ArtifactStoreAggregateError::Canonicalization)
        }
        ArtifactQuotaOwnerError::InvalidScopeSet
        | ArtifactQuotaOwnerError::ScopeDrift
        | ArtifactQuotaOwnerError::MissingScope(_)
        | ArtifactQuotaOwnerError::MissingHead(_)
        | ArtifactQuotaOwnerError::LimitSnapshotDrift => {
            QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CorruptState)
        }
    }
}

impl FakeArtifactStore {
    /// Constructs one empty fake owner with an initial authoritative store
    /// quota head and immutable lower-or-equal limit snapshot.
    ///
    /// # Errors
    ///
    /// Rejects an internally invalid limit projection or canonical head.
    pub fn new(
        store_id: ArtifactStoreIdentity,
        limits: ArtifactStoreLimits,
    ) -> Result<Self, ArtifactStoreAggregateError> {
        let report = ArtifactQuotaSnapshot::new(
            store_id.clone(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .recompute(limits)
        .map_err(|failure| match map_quota_recompute_error(failure) {
            QuotaCommitFailure::Denied(_) => ArtifactStoreAggregateError::QuotaExhausted,
            QuotaCommitFailure::Hard(error) => error,
        })?;
        let quota_head_set = ArtifactQuotaHeadSet::from_report(
            &report,
            [ArtifactQuotaScope::Store(store_id.clone())],
        )
        .map_err(|failure| match map_quota_owner_error(failure) {
            QuotaCommitFailure::Denied(_) => ArtifactStoreAggregateError::QuotaExhausted,
            QuotaCommitFailure::Hard(error) => error,
        })?;
        Ok(Self {
            store_id,
            limits,
            lifecycle: ArtifactLifecycleState::new(limits),
            bytes: FakeArtifactBytes::default(),
            history: ArtifactCommandHistory::new(),
            staging: HashMap::new(),
            command_tasks: HashMap::new(),
            quota_head_set: Some(quota_head_set),
            retired_quota_objects: HashSet::new(),
            terminal_receipts: HashMap::new(),
        })
    }

    /// Returns the fixed fake-store identity.
    #[must_use]
    pub const fn store_id(&self) -> &ArtifactStoreIdentity {
        &self.store_id
    }

    /// Returns the immutable configured limit snapshot.
    #[must_use]
    pub const fn limits(&self) -> ArtifactStoreLimits {
        self.limits
    }

    /// Returns the complete current fixed-owner head for one quota scope.
    ///
    /// # Errors
    ///
    /// Rejects a scope that has never existed in this store.
    pub fn quota_head(
        &self,
        scope: &ArtifactQuotaScope,
    ) -> Result<&ArtifactQuotaHead, ArtifactStoreAggregateError> {
        self.quota_head_set
            .as_ref()
            .and_then(|heads| heads.head(scope))
            .ok_or(ArtifactStoreAggregateError::CorruptState)
    }

    /// Returns the checkpoint binding every current quota head.
    ///
    /// # Errors
    ///
    /// Rejects an internally absent owner head set.
    pub fn quota_checkpoint_digest(&self) -> Result<&ContentDigest, ArtifactStoreAggregateError> {
        self.quota_head_set
            .as_ref()
            .map(ArtifactQuotaHeadSet::checkpoint_digest)
            .ok_or(ArtifactStoreAggregateError::CorruptState)
    }

    /// Returns the number of represented logical object keys.
    #[must_use]
    pub fn object_count(&self) -> usize {
        self.lifecycle.object_count()
    }

    /// Returns the number of immutable applied or denied terminal commands.
    #[must_use]
    pub fn terminal_command_count(&self) -> usize {
        self.terminal_receipts.len()
    }

    /// Returns every retained staging identity, including fail-safe terminal
    /// reconciliation rows.
    #[must_use]
    pub fn staging_reservation_count(&self) -> usize {
        self.staging.len()
    }

    /// Verifies that this state is the exact empty owner constructed from its
    /// immutable identity and limits. Durable repositories use this instead
    /// of treating an arbitrary self-consistent snapshot as an initial row.
    pub(crate) fn validate_repository_initial(&self) -> Result<(), ArtifactStoreAggregateError> {
        self.validate_snapshot_metadata()?;
        let initial = Self::new(self.store_id.clone(), self.limits)?;
        if self != &initial {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        Ok(())
    }

    /// Verifies one exact semantic successor for durable compare-and-swap.
    ///
    /// A successor retains every immutable command/task/terminal row, adds
    /// exactly one terminal command, and proves that command's aggregate-state
    /// commitment against the complete next metadata state. This prevents a
    /// storage caller from replacing current state with an unrelated but
    /// internally self-consistent snapshot.
    pub(crate) fn validate_repository_successor(
        &self,
        next: &Self,
    ) -> Result<(), ArtifactStoreAggregateError> {
        self.validate_snapshot_metadata()?;
        next.validate_snapshot_metadata()?;
        if self.store_id != next.store_id
            || self.limits != next.limits
            || next.terminal_receipts.len() != self.terminal_receipts.len().saturating_add(1)
            || self.command_tasks.iter().any(|(key, task)| {
                next.command_tasks.get(key) != Some(task)
                    || next.terminal_receipts.get(key) != self.terminal_receipts.get(key)
            })
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }

        let mut added = next
            .terminal_receipts
            .iter()
            .filter(|(key, _)| !self.terminal_receipts.contains_key(*key));
        let (key, receipt) = added
            .next()
            .ok_or(ArtifactStoreAggregateError::CorruptState)?;
        if added.next().is_some()
            || next.command_tasks.len() != self.command_tasks.len().saturating_add(1)
            || !next.command_tasks.contains_key(key)
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }

        let object_key =
            ArtifactObjectKey::new(key.project_id().clone(), key.content_digest().clone());
        let object_generation =
            if receipt.history().request().kind() == ArtifactCommandKind::Staging {
                None
            } else {
                let generation =
                    request_source_string(receipt.history().request(), "object_generation")?
                        .parse::<u64>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or(ArtifactStoreAggregateError::CorruptState)?;
                if receipt.lifecycle().is_some_and(|lifecycle| {
                    lifecycle.object().object().generation().get() != generation
                }) {
                    return Err(ArtifactStoreAggregateError::CorruptState);
                }
                Some(generation)
            };
        let mut state_before_receipt = next.clone();
        state_before_receipt.terminal_receipts.remove(key);
        let aggregate_state_digest = state_before_receipt.command_state_digest(
            &object_key,
            object_generation,
            receipt.history().receipt_digest(),
            receipt.lifecycle(),
            receipt.quota_checkpoint_digest(),
        )?;
        if &aggregate_state_digest != receipt.aggregate_state_digest() {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        Ok(())
    }

    /// Returns the complete current fixed-owner head for one exact generation.
    ///
    /// # Errors
    ///
    /// Rejects an absent or non-current generation.
    pub fn current_head(
        &self,
        object: &ArtifactObjectIdentity,
    ) -> Result<ArtifactAuthorityHead, ArtifactLifecycleError> {
        self.lifecycle.current_head(object)
    }

    /// Resolves the sole current generation for one project-scoped object key.
    ///
    /// This query never crosses a project namespace and returns `None` for an
    /// unknown key without revealing whether equal bytes exist elsewhere.
    ///
    /// # Errors
    ///
    /// Rejects internally conflicting current generations or a malformed
    /// lifecycle head.
    pub fn current_head_for_key(
        &self,
        key: &ArtifactObjectKey,
    ) -> Result<Option<ArtifactAuthorityHead>, ArtifactStoreAggregateError> {
        let matches = self
            .lifecycle
            .current_object_identities()
            .into_iter()
            .filter(|object| object.key() == key)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => Ok(None),
            [object] => self
                .lifecycle
                .current_head(object)
                .map(Some)
                .map_err(|_| ArtifactStoreAggregateError::CorruptState),
            _ => Err(ArtifactStoreAggregateError::CorruptState),
        }
    }

    /// Publishes a new or explicitly expected deleted generation through one
    /// exact terminal command.
    ///
    /// `expected_head = None` means the project-scoped key must be absent.
    /// Reintroduction after deletion must carry the exact independently
    /// queried deleted head. An available generation must use
    /// [`Self::add_reference`] rather than implicit publish deduplication.
    ///
    /// # Errors
    ///
    /// Rejects malformed command metadata or changed command-ID reuse. Normal
    /// lifecycle/currentness/quota denials are retained and returned as a
    /// terminal denied receipt.
    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &mut self,
        command_id: impl Into<String>,
        manifest: ArtifactReferenceManifest,
        exact_bytes: &[u8],
        expected_head: Option<&ArtifactAuthorityHead>,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let object = manifest.object().clone();
        let task_id = manifest.binding().task_id().clone();
        let recomputed_manifest = artifact_manifest_digest(&manifest)
            .map_err(|_| ArtifactStoreAggregateError::InvalidCommand)?;
        let observed_content_digest = sha256_content(exact_bytes)?;
        let authority_input_digest =
            reference_authority_input_digest(manifest.creation_authority())?;
        let mut fields = vec![
            string("project_id", object.key().project_id().as_str()),
            string("task_id", task_id.as_str()),
            string("algorithm", object.key().algorithm()),
            string("content_digest", object.key().content_digest().as_str()),
            string("object_generation", object.generation().get().to_string()),
            string("reference_id", manifest.reference_id()),
            string("byte_length", manifest.byte_length().get().to_string()),
            string("observed_byte_length", exact_bytes.len().to_string()),
            string("manifest_digest", recomputed_manifest.as_str()),
            string(
                "declared_manifest_digest",
                manifest.manifest_digest().as_str(),
            ),
            string("observed_content_digest", observed_content_digest.as_str()),
            string("authority_input_digest", authority_input_digest.as_str()),
            string(
                "limit_snapshot_digest",
                self.limits
                    .limit_snapshot_digest()
                    .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                    .as_str(),
            ),
            string("runtime", "FAKE"),
            string(
                "expected_head_token",
                if expected_head.is_some() {
                    "PRESENT"
                } else {
                    "ABSENT"
                },
            ),
        ];
        if let Some(expected) = expected_head {
            fields.push(string(
                "expected_head_digest",
                artifact_current_head_input_digest(expected)?.as_str(),
            ));
        }
        let request =
            Self::command_request(command_id, &object, ArtifactCommandKind::Publish, fields)?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }

        let before = self.semantic_state_digest(&object)?;
        let mut next = self.clone();
        let transition = match next.validate_expected_publish_head(&object, expected_head) {
            Ok(()) => next
                .lifecycle
                .publish(&mut next.bytes, manifest, exact_bytes, authorities)
                .map_err(|error| error.code()),
            Err(code) => Err(code),
        };
        if transition.is_ok()
            && let Some(expected) = expected_head
            && expected.object().availability() == ArtifactAvailability::Deleted
        {
            next.retired_quota_objects
                .insert(expected.object().object().clone());
        }
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            &object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Adds one immutable reference only from the exact current Artifact Store
    /// head and exact current external reference-owner pair.
    ///
    /// # Errors
    ///
    /// Hard request/idempotency failures return an error; stale/semantic
    /// failures return a retained terminal denial.
    pub fn add_reference(
        &mut self,
        command_id: impl Into<String>,
        manifest: ArtifactReferenceManifest,
        expected_head: &ArtifactAuthorityHead,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let object = manifest.object().clone();
        let task_id = manifest.binding().task_id().clone();
        let recomputed_manifest = artifact_manifest_digest(&manifest)
            .map_err(|_| ArtifactStoreAggregateError::InvalidCommand)?;
        let authority_input_digest =
            reference_authority_input_digest(manifest.creation_authority())?;
        let request = Self::command_request(
            command_id,
            &object,
            ArtifactCommandKind::AddReference,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("reference_id", manifest.reference_id()),
                string("byte_length", manifest.byte_length().get().to_string()),
                string("manifest_digest", recomputed_manifest.as_str()),
                string(
                    "declared_manifest_digest",
                    manifest.manifest_digest().as_str(),
                ),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(&object)?;
        let mut next = self.clone();
        let transition = match next.require_current_head(&object, expected_head) {
            Ok(()) => next
                .lifecycle
                .add_reference(manifest, authorities)
                .map_err(|error| error.code()),
            Err(code) => Err(code),
        };
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            &object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Terminally releases one exact reference from matching store and owner
    /// current heads.
    ///
    /// # Errors
    ///
    /// Hard request/idempotency failures return an error; stale/semantic
    /// failures return a retained terminal denial.
    pub fn release_reference(
        &mut self,
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        reference_id: &str,
        authority: ArtifactReferenceAuthorityPair,
        expected_head: &ArtifactAuthorityHead,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let task_id = authority.receipt().binding().task_id().clone();
        let authority_input_digest = reference_authority_input_digest(&authority)?;
        let request = Self::command_request(
            command_id,
            object,
            ArtifactCommandKind::ReleaseReference,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("reference_id", reference_id),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(object)?;
        let mut next = self.clone();
        let transition = match next.require_current_head(object, expected_head) {
            Ok(()) => next
                .lifecycle
                .release_reference(object, reference_id, authority, authorities)
                .map_err(|error| error.code()),
            Err(code) => Err(code),
        };
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Acquires one exact bounded read claim from matching store and
    /// read-owner current heads.
    ///
    /// # Errors
    ///
    /// Hard request/idempotency failures return an error; stale, authority,
    /// lease, state, and quota failures become immutable terminal denials.
    #[allow(clippy::too_many_arguments)]
    pub fn acquire_read(
        &mut self,
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        holder_id: &str,
        acquired_at: &str,
        expires_at: &str,
        authority: ArtifactReadAuthorityPair,
        expected_head: &ArtifactAuthorityHead,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let task_id = authority.receipt().binding().task_id().clone();
        let read_claim_id = authority.receipt().binding().read_claim_id().to_owned();
        let authority_input_digest = read_authority_input_digest(&authority)?;
        let request = Self::command_request(
            command_id,
            object,
            ArtifactCommandKind::AcquireRead,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("read_claim_id", &read_claim_id),
                string("holder_id", holder_id),
                string("acquired_at", acquired_at),
                string("expires_at", expires_at),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(object)?;
        let mut next = self.clone();
        let transition = match next.require_current_head(object, expected_head) {
            Ok(()) => next
                .lifecycle
                .acquire_read(
                    object,
                    holder_id,
                    acquired_at,
                    expires_at,
                    authority,
                    authorities,
                )
                .map_err(|error| error.code()),
            Err(code) => Err(code),
        };
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Terminally releases one active read claim.
    ///
    /// # Errors
    ///
    /// Hard request/idempotency failures return an error; normal semantic
    /// failures become immutable terminal denials.
    #[allow(clippy::too_many_arguments)]
    pub fn release_read(
        &mut self,
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        authority: ArtifactReadAuthorityPair,
        expected_head: &ArtifactAuthorityHead,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let task_id = authority.receipt().binding().task_id().clone();
        let authority_input_digest = read_authority_input_digest(&authority)?;
        let request = Self::command_request(
            command_id,
            object,
            ArtifactCommandKind::ReleaseRead,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("read_claim_id", read_claim_id),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(object)?;
        let mut next = self.clone();
        let transition = match next.require_current_head(object, expected_head) {
            Ok(()) => next
                .lifecycle
                .release_read(object, read_claim_id, authority, authorities)
                .map_err(|error| error.code()),
            Err(code) => Err(code),
        };
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Marks an elapsed active claim `EXPIRED_SUSPECT` without freeing quota.
    ///
    /// # Errors
    ///
    /// Hard request/idempotency failures return an error; early, stale, or
    /// terminal expiry attempts become immutable terminal denials.
    pub fn expire_read(
        &mut self,
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        observed_at: &str,
        expected_head: &ArtifactAuthorityHead,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let (task_id, authority_input_digest, request) = self.expire_read_request(
            command_id,
            object,
            read_claim_id,
            observed_at,
            expected_head,
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(object)?;
        let mut next = self.clone();
        let transition = match next.require_current_head(object, expected_head) {
            Ok(()) => next
                .lifecycle
                .mark_read_expired_suspect(object, read_claim_id, observed_at)
                .map_err(|error| error.code()),
            Err(code) => Err(code),
        };
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Reconciles one expired-suspect read from exact release authority and
    /// independently current closure evidence.
    ///
    /// # Errors
    ///
    /// Hard request/idempotency failures return an error; stale or inexact
    /// evidence becomes an immutable terminal denial.
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile_read(
        &mut self,
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        authority: ArtifactReadAuthorityPair,
        closure: &ArtifactReadClosureEvidencePair,
        expected_head: &ArtifactAuthorityHead,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let task_id = authority.receipt().binding().task_id().clone();
        let read_authority_digest = read_authority_input_digest(&authority)?;
        let closure_digest = read_closure_input_digest(closure)?;
        let authority_input_digest = digest(
            "lattice.artifact.aggregate.read-reconcile-authority-set",
            &CanonicalValue::Object(vec![
                string("read_authority_digest", read_authority_digest.as_str()),
                string("closure_evidence_digest", closure_digest.as_str()),
            ]),
        )?;
        let request = Self::command_request(
            command_id,
            object,
            ArtifactCommandKind::ReconcileRead,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("read_claim_id", read_claim_id),
                string("read_authority_digest", read_authority_digest.as_str()),
                string("closure_evidence_digest", closure_digest.as_str()),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(object)?;
        let mut next = self.clone();
        let transition = match next.require_current_head(object, expected_head) {
            Ok(()) => next
                .lifecycle
                .reconcile_expired_read(object, read_claim_id, authority, authorities, closure)
                .map_err(|error| error.code()),
            Err(code) => Err(code),
        };
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Verifies one active read and returns exact bytes, or atomically records
    /// expiry through the same command/history/quota owner.
    ///
    /// # Errors
    ///
    /// Rejects stale query heads or missing/corrupt bytes without recording a
    /// terminal command.
    pub fn read_verified(
        &mut self,
        expiry_command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        observed_at: &str,
        expected_head: &ArtifactAuthorityHead,
    ) -> Result<ArtifactVerifiedReadExecution, ArtifactStoreAggregateError> {
        let expiry_command_id = expiry_command_id.into();
        let (_, _, request) = self.expire_read_request(
            expiry_command_id.clone(),
            object,
            read_claim_id,
            observed_at,
            expected_head,
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(ArtifactVerifiedReadExecution::ExpiryCommand(Box::new(
                exact,
            )));
        }
        self.require_current_head(object, expected_head)
            .map_err(|_| {
                ArtifactStoreAggregateError::Lifecycle(ArtifactLifecycleError::StalePlan)
            })?;
        let mut probe = self.clone();
        let result = {
            let lifecycle = &mut probe.lifecycle;
            let bytes = &probe.bytes;
            lifecycle.read_verified(bytes, object, read_claim_id, observed_at)
        };
        match result {
            Ok(bytes) => Ok(ArtifactVerifiedReadExecution::Bytes(bytes)),
            Err(ArtifactLifecycleError::ReadExpiredSuspect) => self
                .expire_read(
                    expiry_command_id,
                    object,
                    read_claim_id,
                    observed_at,
                    expected_head,
                )
                .map(Box::new)
                .map(ArtifactVerifiedReadExecution::ExpiryCommand),
            Err(error) => Err(ArtifactStoreAggregateError::Lifecycle(error)),
        }
    }

    fn expire_read_request(
        &self,
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        read_claim_id: &str,
        observed_at: &str,
        expected_head: &ArtifactAuthorityHead,
    ) -> Result<(TaskId, ContentDigest, ArtifactCommandRequest), ArtifactStoreAggregateError> {
        let command_id = command_id.into();
        let key = ArtifactCommandStorageKey::new(
            object.key().project_id().clone(),
            object.key().content_digest().clone(),
            command_id.clone(),
        )
        .map_err(map_history_error)?;
        let (task_id, source_authority_digest) =
            if let Some(stored) = self.terminal_receipts.get(&key) {
                let stored_request = stored.history().request();
                if stored_request.key() != &key {
                    return Err(ArtifactStoreAggregateError::CorruptState);
                }
                if stored_request.kind() != ArtifactCommandKind::ExpireRead {
                    return Err(ArtifactStoreAggregateError::CommandIdReuse);
                }
                let task_id = TaskId::new(request_source_string(stored_request, "task_id")?)
                    .map_err(|_| ArtifactStoreAggregateError::CorruptState)?;
                if self.command_tasks.get(&key) != Some(&task_id) {
                    return Err(ArtifactStoreAggregateError::CorruptState);
                }
                let source_authority_digest = ContentDigest::from_sha256(request_source_string(
                    stored_request,
                    "source_authority_digest",
                )?)
                .map_err(|_| ArtifactStoreAggregateError::CorruptState)?;
                (task_id, source_authority_digest)
            } else if self.command_tasks.contains_key(&key) {
                return Err(ArtifactStoreAggregateError::CorruptState);
            } else {
                let read = self
                    .lifecycle
                    .read_head(object, read_claim_id)
                    .map_err(ArtifactStoreAggregateError::Lifecycle)?;
                (
                    read.authority().receipt().binding().task_id().clone(),
                    read_authority_input_digest(read.authority())?,
                )
            };
        let authority_input_digest = digest(
            "lattice.artifact.aggregate.read-expiry-input",
            &CanonicalValue::Object(vec![
                string("source_authority_digest", source_authority_digest.as_str()),
                string("observed_at", observed_at),
                string("read_claim_id", read_claim_id),
            ]),
        )?;
        let request = Self::command_request(
            command_id,
            object,
            ArtifactCommandKind::ExpireRead,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("read_claim_id", read_claim_id),
                string("observed_at", observed_at),
                string("source_authority_digest", source_authority_digest.as_str()),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        Ok((task_id, authority_input_digest, request))
    }

    /// Returns one retained staging reservation, including verified terminal
    /// audit rows.
    #[must_use]
    pub fn staging_reservation(
        &self,
        identity: &ArtifactStagingIdentity,
    ) -> Option<&ArtifactStagingReservation> {
        self.staging.get(identity)
    }

    /// Atomically reserves exact staging bytes and streams.
    ///
    /// # Errors
    ///
    /// Changed command reuse is rejected. Duplicate identities and quota
    /// exhaustion become immutable terminal denials when the denial record
    /// itself still fits command/history quota.
    pub fn reserve_staging(
        &mut self,
        command_id: impl Into<String>,
        reservation: ArtifactStagingReservation,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let identity = reservation.identity().clone();
        let task_id = identity.task_id().clone();
        let authority_input_digest =
            staging_reservation_input_digest(&reservation, "RESERVE", None)?;
        let request = self.staging_command_request(
            command_id,
            &identity,
            vec![
                string("staging_bytes", reservation.bytes().to_string()),
                string("staging_streams", reservation.streams().to_string()),
                string("status", staging_state_text(reservation.state())),
                string("action", "RESERVE"),
                string("authority_input_digest", authority_input_digest.as_str()),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.staging_reservation_state_digest(&identity)?;
        let mut next = self.clone();
        let transition = if next.staging.contains_key(&identity) {
            Err("ARTIFACT_STAGING_IDENTITY_EXISTS")
        } else {
            next.staging.insert(identity.clone(), reservation);
            Ok(())
        };
        self.finish_staging_command(
            next,
            &request,
            task_id,
            &identity,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Marks exact staged bytes as a sealed unpublished orphan without
    /// releasing quota.
    ///
    /// # Errors
    ///
    /// Invalid or terminal transitions become immutable denials.
    pub fn mark_staging_sealed_orphan(
        &mut self,
        command_id: impl Into<String>,
        identity: &ArtifactStagingIdentity,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        self.apply_fail_safe_staging_transition(
            command_id,
            identity,
            "MARK_SEALED_ORPHAN",
            ArtifactStagingState::SealedOrphan,
        )
    }

    /// Marks one staging outcome reconciliation-required without releasing
    /// quota.
    ///
    /// # Errors
    ///
    /// Invalid or terminal transitions become immutable denials.
    pub fn mark_staging_reconciliation_required(
        &mut self,
        command_id: impl Into<String>,
        identity: &ArtifactStagingIdentity,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        self.apply_fail_safe_staging_transition(
            command_id,
            identity,
            "MARK_RECONCILIATION_REQUIRED",
            ArtifactStagingState::ReconciliationRequired,
        )
    }

    /// Applies a quota-releasing terminal staging transition only from exact
    /// fixed-owner evidence and its independently current fake head.
    ///
    /// # Errors
    ///
    /// Stale, substituted, wrong-metric, or non-current evidence becomes an
    /// immutable terminal denial.
    pub fn apply_verified_staging_terminal(
        &mut self,
        command_id: impl Into<String>,
        identity: &ArtifactStagingIdentity,
        evidence: &ArtifactStagingTerminalEvidence,
        authority: &FakeArtifactStagingTerminalAuthority,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let binding = evidence.receipt().binding();
        let task_id = identity.task_id().clone();
        let authority_input_digest = staging_terminal_input_digest(evidence)?;
        let request = self.staging_command_request(
            command_id,
            identity,
            vec![
                string("staging_bytes", binding.bytes().to_string()),
                string("staging_streams", binding.streams().to_string()),
                string("status", staging_state_text(binding.from())),
                string("kind", staging_state_text(binding.to())),
                string(
                    "terminal_receipt_digest",
                    evidence.receipt().receipt_digest().as_str(),
                ),
                string(
                    "terminal_current_head_digest",
                    evidence.current_head().head_digest().as_str(),
                ),
                string("authority_input_digest", authority_input_digest.as_str()),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.staging_reservation_state_digest(identity)?;
        let mut next = self.clone();
        let transition = next
            .staging
            .get_mut(identity)
            .ok_or(ArtifactQuotaError::StagingEvidenceMismatch)
            .and_then(|reservation| reservation.apply_verified_terminal(evidence, authority))
            .map_err(|error| error.code());
        self.finish_staging_command(
            next,
            &request,
            task_id,
            identity,
            before,
            authority_input_digest,
            transition,
        )
    }

    fn apply_fail_safe_staging_transition(
        &mut self,
        command_id: impl Into<String>,
        identity: &ArtifactStagingIdentity,
        action: &'static str,
        target: ArtifactStagingState,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let task_id = identity.task_id().clone();
        let authority_input_digest = digest(
            "lattice.artifact.aggregate.staging-fail-safe-input",
            &CanonicalValue::Object(vec![
                string("project_id", identity.project_id().as_str()),
                string("algorithm", identity.object_key().algorithm()),
                string(
                    "content_digest",
                    identity.object_key().content_digest().as_str(),
                ),
                string("task_id", identity.task_id().as_str()),
                string("reservation_id", identity.value()),
                string("action", action),
                string("status", staging_state_text(target)),
            ]),
        )?;
        let request = self.staging_command_request(
            command_id,
            identity,
            vec![
                string("action", action),
                string("status", staging_state_text(target)),
                string("authority_input_digest", authority_input_digest.as_str()),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.staging_reservation_state_digest(identity)?;
        let mut next = self.clone();
        let transition = match next.staging.get_mut(identity) {
            Some(reservation) if target == ArtifactStagingState::SealedOrphan => {
                reservation.mark_sealed_orphan()
            }
            Some(reservation) if target == ArtifactStagingState::ReconciliationRequired => {
                reservation.mark_reconciliation_required()
            }
            Some(_) | None => Err(ArtifactQuotaError::InvalidStagingTransition),
        }
        .map_err(|error| error.code());
        self.finish_staging_command(
            next,
            &request,
            task_id,
            identity,
            before,
            authority_input_digest,
            transition,
        )
    }

    fn staging_command_request(
        &self,
        command_id: impl Into<String>,
        identity: &ArtifactStagingIdentity,
        mut fields: Vec<(String, CanonicalValue)>,
    ) -> Result<ArtifactCommandRequest, ArtifactStoreAggregateError> {
        fields.extend([
            string("project_id", identity.project_id().as_str()),
            string("task_id", identity.task_id().as_str()),
            string("algorithm", identity.object_key().algorithm()),
            string(
                "content_digest",
                identity.object_key().content_digest().as_str(),
            ),
            string("reservation_id", identity.value()),
            string(
                "limit_snapshot_digest",
                self.limits
                    .limit_snapshot_digest()
                    .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                    .as_str(),
            ),
            string("runtime", "FAKE"),
        ]);
        Self::command_request_for_key(
            command_id,
            identity.object_key(),
            ArtifactCommandKind::Staging,
            fields,
        )
    }

    fn staging_reservation_state_digest(
        &self,
        identity: &ArtifactStagingIdentity,
    ) -> Result<ContentDigest, ArtifactStoreAggregateError> {
        let (bytes, streams, status) = self.staging.get(identity).map_or_else(
            || ("0".to_owned(), "0".to_owned(), "ABSENT"),
            |reservation| {
                (
                    reservation.bytes().to_string(),
                    reservation.streams().to_string(),
                    staging_state_text(reservation.state()),
                )
            },
        );
        digest(
            "lattice.artifact.aggregate.staging-reservation-state",
            &CanonicalValue::Object(vec![
                string("project_id", identity.project_id().as_str()),
                string("algorithm", identity.object_key().algorithm()),
                string(
                    "content_digest",
                    identity.object_key().content_digest().as_str(),
                ),
                string("task_id", identity.task_id().as_str()),
                string("reservation_id", identity.value()),
                string("staging_bytes", bytes),
                string("staging_streams", streams),
                string("status", status),
            ]),
        )
    }

    /// Produces a read-only exact delete plan from the independently queried
    /// integrated Artifact Store head.
    ///
    /// # Errors
    ///
    /// Rejects stale heads, active reference/read blockers, unavailable state,
    /// or unelapsed retention/grace.
    pub fn plan_delete(
        &self,
        object: &ArtifactObjectIdentity,
        expected_head: &ArtifactAuthorityHead,
        observed_at: &str,
        grace_until: &str,
    ) -> Result<ArtifactDeletePlan, ArtifactLifecycleError> {
        self.lifecycle
            .plan_delete(object, expected_head, observed_at, grace_until)
    }

    /// Claims one exact delete plan from typed sweep authority.
    ///
    /// # Errors
    ///
    /// Changed command reuse is rejected. Stale plans, blockers, or inexact
    /// authority become immutable terminal denials.
    pub fn claim_delete(
        &mut self,
        command_id: impl Into<String>,
        plan: &ArtifactDeletePlan,
        authority: &ArtifactSweepAuthorityPair,
        authorities: &FakeArtifactAuthorityDirectory,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let object = plan.object();
        let task_id = sweep_task_id()?;
        let authority_input_digest = sweep_authority_input_digest(authority)?;
        let request = Self::command_request(
            command_id,
            object,
            ArtifactCommandKind::DeleteClaim,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("plan_digest", plan.plan_digest().as_str()),
                string("claim_token", plan.claim_token()),
                string("observed_at", plan.observed_at()),
                string("grace_until", plan.grace_until()),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(plan.expected_head())?.as_str(),
                ),
                string("sweep_authority_digest", authority_input_digest.as_str()),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(object)?;
        let mut next = self.clone();
        let transition = next
            .lifecycle
            .claim_delete(plan, authority, authorities)
            .map_err(|error| error.code());
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Records an exact fake delete-adapter outcome for one claimed object.
    ///
    /// # Errors
    ///
    /// Changed command reuse is rejected. Stale heads, wrong tokens, or
    /// illegal outcomes become immutable terminal denials.
    pub fn apply_delete_outcome(
        &mut self,
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        claim_token: &str,
        outcome: FakeDeleteOutcome,
        expected_head: &ArtifactAuthorityHead,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let task_id = sweep_task_id()?;
        let authority_input_digest = digest(
            "lattice.artifact.aggregate.delete-result-input",
            &CanonicalValue::Object(vec![
                string("project_id", object.key().project_id().as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("claim_token", claim_token),
                string("status", delete_outcome_text(outcome)),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
            ]),
        )?;
        let request = Self::command_request(
            command_id,
            object,
            ArtifactCommandKind::DeleteResult,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("claim_token", claim_token),
                string("status", delete_outcome_text(outcome)),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(object)?;
        let mut next = self.clone();
        let transition = match next.require_current_head(object, expected_head) {
            Ok(()) => {
                let lifecycle = &mut next.lifecycle;
                let bytes = &mut next.bytes;
                lifecycle
                    .apply_delete_outcome(bytes, object, claim_token, outcome)
                    .map_err(|error| error.code())
            }
            Err(code) => Err(code),
        };
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Reconciles an ambiguous delete result from exact owned-byte evidence.
    ///
    /// # Errors
    ///
    /// Changed command reuse is rejected. Stale heads, wrong tokens, or
    /// contradictory byte evidence become immutable terminal denials.
    pub fn reconcile_delete(
        &mut self,
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        claim_token: &str,
        result: ArtifactReconciliationResult,
        expected_head: &ArtifactAuthorityHead,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let task_id = sweep_task_id()?;
        let authority_input_digest = digest(
            "lattice.artifact.aggregate.delete-reconcile-input",
            &CanonicalValue::Object(vec![
                string("project_id", object.key().project_id().as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("claim_token", claim_token),
                string("status", reconciliation_result_text(result)),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
            ]),
        )?;
        let request = Self::command_request(
            command_id,
            object,
            ArtifactCommandKind::DeleteReconcile,
            vec![
                string("project_id", object.key().project_id().as_str()),
                string("task_id", task_id.as_str()),
                string("algorithm", object.key().algorithm()),
                string("content_digest", object.key().content_digest().as_str()),
                string("object_generation", object.generation().get().to_string()),
                string("claim_token", claim_token),
                string("status", reconciliation_result_text(result)),
                string(
                    "expected_head_digest",
                    artifact_current_head_input_digest(expected_head)?.as_str(),
                ),
                string("authority_input_digest", authority_input_digest.as_str()),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string("runtime", "FAKE"),
            ],
        )?;
        if let Some(exact) = self.exact_retry(&request)? {
            return Ok(exact);
        }
        let before = self.semantic_state_digest(object)?;
        let mut next = self.clone();
        let transition = match next.require_current_head(object, expected_head) {
            Ok(()) => {
                let lifecycle = &mut next.lifecycle;
                let bytes = &next.bytes;
                lifecycle
                    .reconcile_delete(bytes, object, claim_token, result)
                    .map_err(|error| error.code())
            }
            Err(code) => Err(code),
        };
        self.finish_lifecycle_command(
            next,
            &request,
            task_id,
            object,
            before,
            authority_input_digest,
            transition,
        )
    }

    /// Removes one exact fake byte row to simulate independently verified
    /// physical absence in reconciliation tests.
    ///
    /// This is a visibly fake fault-injection hook; it grants no filesystem
    /// deletion capability.
    #[doc(hidden)]
    pub fn remove_bytes_for_test(&mut self, object: &ArtifactObjectIdentity) {
        self.bytes.remove(object);
    }

    fn command_request(
        command_id: impl Into<String>,
        object: &ArtifactObjectIdentity,
        kind: ArtifactCommandKind,
        source: Vec<(String, CanonicalValue)>,
    ) -> Result<ArtifactCommandRequest, ArtifactStoreAggregateError> {
        Self::command_request_for_key(command_id, object.key(), kind, source)
    }

    fn command_request_for_key(
        command_id: impl Into<String>,
        object_key: &ArtifactObjectKey,
        kind: ArtifactCommandKind,
        source: Vec<(String, CanonicalValue)>,
    ) -> Result<ArtifactCommandRequest, ArtifactStoreAggregateError> {
        let key = ArtifactCommandStorageKey::new(
            object_key.project_id().clone(),
            object_key.content_digest().clone(),
            command_id,
        )
        .map_err(map_history_error)?;
        ArtifactCommandRequest::new(key, kind, CanonicalValue::Object(source))
            .map_err(map_history_error)
    }

    fn exact_retry(
        &self,
        request: &ArtifactCommandRequest,
    ) -> Result<Option<ArtifactStoreCommandExecution>, ArtifactStoreAggregateError> {
        let Some(history) = self
            .history
            .lookup_request(request)
            .map_err(map_history_error)?
        else {
            return Ok(None);
        };
        let stored = self
            .terminal_receipts
            .get(request.key())
            .filter(|receipt| receipt.history() == &history)
            .cloned()
            .ok_or(ArtifactStoreAggregateError::CorruptState)?;
        Ok(Some(ArtifactStoreCommandExecution {
            disposition: ArtifactCommandExecutionDisposition::ExactRetry,
            receipt: stored,
        }))
    }

    fn validate_expected_publish_head(
        &self,
        object: &ArtifactObjectIdentity,
        expected: Option<&ArtifactAuthorityHead>,
    ) -> Result<(), &'static str> {
        match (self.current_head_for_key(object.key()), expected) {
            (Ok(None), None) => Ok(()),
            (Ok(Some(current)), Some(expected))
                if &current == expected
                    && current.object().availability() == ArtifactAvailability::Deleted =>
            {
                Ok(())
            }
            _ => Err("ARTIFACT_STALE_PUBLISH_HEAD"),
        }
    }

    fn require_current_head(
        &self,
        object: &ArtifactObjectIdentity,
        expected: &ArtifactAuthorityHead,
    ) -> Result<(), &'static str> {
        match self.lifecycle.current_head(object) {
            Ok(current) if &current == expected => Ok(()),
            _ => Err("ARTIFACT_STALE_CURRENT_HEAD"),
        }
    }

    fn commit_quota_and_refresh_heads(&mut self) -> Result<ContentDigest, QuotaCommitFailure> {
        if self.lifecycle.limits() != self.limits {
            return Err(QuotaCommitFailure::Hard(
                ArtifactStoreAggregateError::CorruptState,
            ));
        }
        let objects = self
            .lifecycle
            .quota_object_records()
            .map_err(|_| QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CorruptState))?;
        let references = self
            .lifecycle
            .quota_reference_records()
            .map_err(|_| QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CorruptState))?;
        let reads = self
            .lifecycle
            .quota_read_records()
            .map_err(|_| QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CorruptState))?;
        let commands = self.command_quota_records()?;
        let staging = self.staging.values().cloned().collect::<Vec<_>>();
        let report = ArtifactQuotaSnapshot::new(
            self.store_id.clone(),
            objects,
            references.clone(),
            reads.clone(),
            commands,
            staging,
        )
        .recompute(self.limits)
        .map_err(map_quota_recompute_error)?;
        let scopes = self.quota_scopes();
        let retired_scopes = self
            .retired_quota_objects
            .iter()
            .cloned()
            .map(ArtifactQuotaScope::Object)
            .collect::<Vec<_>>();
        match &mut self.quota_head_set {
            Some(heads) => heads
                .apply_report_with_retired(&report, scopes, retired_scopes)
                .map_err(map_quota_owner_error)?,
            None => {
                self.quota_head_set = Some(
                    ArtifactQuotaHeadSet::from_report(&report, scopes)
                        .map_err(map_quota_owner_error)?,
                );
            }
        }
        self.refresh_integrated_heads(&references, &reads)?;
        self.quota_head_set
            .as_ref()
            .map(|heads| heads.checkpoint_digest().clone())
            .ok_or(QuotaCommitFailure::Hard(
                ArtifactStoreAggregateError::CorruptState,
            ))
    }

    fn command_quota_records(&self) -> Result<Vec<ArtifactCommandQuotaRecord>, QuotaCommitFailure> {
        let mut records = Vec::with_capacity(self.command_tasks.len());
        for receipt in self.history.sorted_receipts() {
            let key = receipt.request().key();
            let task_id = self
                .command_tasks
                .get(key)
                .cloned()
                .ok_or(QuotaCommitFailure::Hard(
                    ArtifactStoreAggregateError::CorruptState,
                ))?;
            let identity = ArtifactCommandIdentity::new(
                task_id,
                ArtifactObjectKey::new(key.project_id().clone(), key.content_digest().clone()),
                key.command_id(),
            )
            .map_err(map_quota_recompute_error)?;
            let history_bytes = i64::try_from(
                receipt
                    .canonical_bytes()
                    .map_err(map_history_error)
                    .map_err(QuotaCommitFailure::Hard)?
                    .len(),
            )
            .map_err(|_| QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CounterExhausted))?;
            records.push(
                ArtifactCommandQuotaRecord::new(identity, history_bytes)
                    .map_err(map_quota_recompute_error)?,
            );
        }
        if records.len() != self.command_tasks.len() {
            return Err(QuotaCommitFailure::Hard(
                ArtifactStoreAggregateError::CorruptState,
            ));
        }
        Ok(records)
    }

    fn quota_scopes(&self) -> Vec<ArtifactQuotaScope> {
        let mut scopes = HashSet::new();
        scopes.insert(ArtifactQuotaScope::Store(self.store_id.clone()));
        for object in self.lifecycle.current_object_identities() {
            scopes.insert(ArtifactQuotaScope::Object(object));
        }
        for project_id in self.lifecycle.current_project_scopes() {
            scopes.insert(ArtifactQuotaScope::Project(project_id));
        }
        for (project_id, task_id) in self.lifecycle.current_task_scopes() {
            scopes.insert(ArtifactQuotaScope::Project(project_id.clone()));
            scopes.insert(ArtifactQuotaScope::Task {
                project_id,
                task_id,
            });
        }
        for (key, task_id) in &self.command_tasks {
            scopes.insert(ArtifactQuotaScope::Project(key.project_id().clone()));
            scopes.insert(ArtifactQuotaScope::Task {
                project_id: key.project_id().clone(),
                task_id: task_id.clone(),
            });
        }
        for identity in self.staging.keys() {
            scopes.insert(ArtifactQuotaScope::Project(identity.project_id().clone()));
            scopes.insert(ArtifactQuotaScope::Task {
                project_id: identity.project_id().clone(),
                task_id: identity.task_id().clone(),
            });
        }
        scopes.into_iter().collect()
    }

    #[allow(clippy::too_many_lines)]
    fn refresh_integrated_heads(
        &mut self,
        references: &[ArtifactReferenceQuotaRecord],
        reads: &[ArtifactReadQuotaRecord],
    ) -> Result<(), QuotaCommitFailure> {
        let heads = self
            .quota_head_set
            .as_ref()
            .ok_or(QuotaCommitFailure::Hard(
                ArtifactStoreAggregateError::CorruptState,
            ))?;
        let all_tasks = self
            .quota_scopes()
            .into_iter()
            .filter_map(|scope| match scope {
                ArtifactQuotaScope::Task {
                    project_id,
                    task_id,
                } => Some((project_id, task_id)),
                ArtifactQuotaScope::Object(_)
                | ArtifactQuotaScope::Project(_)
                | ArtifactQuotaScope::Store(_) => None,
            })
            .collect::<Vec<_>>();
        let staging_quota_head_digest = heads
            .staging_quota_digest(&all_tasks)
            .map_err(map_quota_owner_error)?;
        let store_quota_head_digest = heads
            .store_head_digest(&self.store_id)
            .map_err(map_quota_owner_error)?
            .clone();
        let mut evidence = Vec::new();
        for object in self.lifecycle.current_object_identities() {
            let mut task_ids = HashMap::<String, TaskId>::new();
            for reference in references {
                if reference.object() == &object {
                    let task_id = reference.identity().task_id().clone();
                    task_ids.insert(task_id.as_str().to_owned(), task_id);
                }
            }
            for read in reads {
                if read.identity().object() == &object {
                    let task_id = read.identity().task_id().clone();
                    task_ids.insert(task_id.as_str().to_owned(), task_id);
                }
            }
            for (key, task_id) in &self.command_tasks {
                if key.project_id() == object.key().project_id()
                    && key.content_digest() == object.key().content_digest()
                {
                    task_ids.insert(task_id.as_str().to_owned(), task_id.clone());
                }
            }
            for identity in self.staging.keys() {
                if identity.object_key() == object.key() {
                    task_ids.insert(
                        identity.task_id().as_str().to_owned(),
                        identity.task_id().clone(),
                    );
                }
            }
            let task_ids = task_ids.into_values().collect::<Vec<_>>();
            let task_quota_head_digest = heads
                .combined_task_head_digest(&object, &task_ids)
                .map_err(map_quota_owner_error)?;
            let project_quota_head_digest = heads
                .project_head_digest(object.key().project_id())
                .map_err(map_quota_owner_error)?
                .clone();
            let _object_quota_head_digest = heads
                .object_head_digest(&object)
                .map_err(map_quota_owner_error)?;
            let history_head = self
                .history
                .head(&crate::history::ArtifactCommandObjectScope::new(
                    object.key().project_id().clone(),
                    object.key().content_digest().clone(),
                ))
                .map_err(map_history_error)
                .map_err(QuotaCommitFailure::Hard)?;
            let command_tail_digest =
                history_head
                    .tail_digest()
                    .cloned()
                    .ok_or(QuotaCommitFailure::Hard(
                        ArtifactStoreAggregateError::CorruptState,
                    ))?;
            let lifecycle_revision = self
                .lifecycle
                .object_head(&object)
                .map_err(|_| QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CorruptState))?
                .revision();
            evidence.push(
                ArtifactIntegratedHeadEvidence::new(
                    object,
                    lifecycle_revision,
                    task_quota_head_digest,
                    project_quota_head_digest,
                    store_quota_head_digest.clone(),
                    staging_quota_head_digest.clone(),
                    history_head.high_water(),
                    command_tail_digest,
                )
                .map_err(|_| QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CorruptState))?,
            );
        }
        self.lifecycle
            .refresh_integrated_heads(evidence)
            .map_err(|_| QuotaCommitFailure::Hard(ArtifactStoreAggregateError::CorruptState))
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_staging_command(
        &mut self,
        mut next: Self,
        request: &ArtifactCommandRequest,
        task_id: TaskId,
        identity: &ArtifactStagingIdentity,
        before: ContentDigest,
        authority_input_digest: ContentDigest,
        transition: Result<(), &'static str>,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let terminal = match transition {
            Ok(()) => {
                let after = next.staging_reservation_state_digest(identity)?;
                let result = digest(
                    "lattice.artifact.aggregate.staging-result",
                    &CanonicalValue::Object(vec![
                        string("before_state_digest", before.as_str()),
                        string("after_state_digest", after.as_str()),
                        string("authority_input_digest", authority_input_digest.as_str()),
                    ]),
                )?;
                ArtifactCommandTerminalProjection::applied(before.clone(), after, result)
                    .map_err(map_history_error)?
            }
            Err(code) => {
                return self.finish_staging_denied_command(
                    request,
                    task_id,
                    identity,
                    before,
                    authority_input_digest,
                    code,
                );
            }
        };
        let history = next
            .history
            .execute(request.clone(), || Ok(terminal))
            .map_err(map_history_error)?;
        if history.disposition() != ArtifactCommandExecutionDisposition::Recorded
            || next
                .command_tasks
                .insert(request.key().clone(), task_id.clone())
                .is_some()
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        match next.commit_quota_and_refresh_heads() {
            Ok(quota_checkpoint_digest) => self.commit_recorded_command(
                next,
                request,
                identity.object_key(),
                None,
                history.receipt().clone(),
                None,
                authority_input_digest,
                quota_checkpoint_digest,
            ),
            Err(QuotaCommitFailure::Denied(code)) => self.finish_staging_denied_command(
                request,
                task_id,
                identity,
                before,
                authority_input_digest,
                code,
            ),
            Err(QuotaCommitFailure::Hard(error)) => Err(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_staging_denied_command(
        &mut self,
        request: &ArtifactCommandRequest,
        task_id: TaskId,
        identity: &ArtifactStagingIdentity,
        before: ContentDigest,
        authority_input_digest: ContentDigest,
        code: &'static str,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let result = digest(
            DENIAL_RESULT_DOMAIN,
            &CanonicalValue::Object(vec![
                string("denial_code", code),
                string("before_state_digest", before.as_str()),
            ]),
        )?;
        let terminal = ArtifactCommandTerminalProjection::denied(
            denial_code(code),
            before.clone(),
            before,
            result,
        )
        .map_err(map_history_error)?;
        let mut denied = self.clone();
        let history = denied
            .history
            .execute(request.clone(), || Ok(terminal))
            .map_err(map_history_error)?;
        if history.disposition() != ArtifactCommandExecutionDisposition::Recorded
            || denied
                .command_tasks
                .insert(request.key().clone(), task_id)
                .is_some()
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        let quota_checkpoint_digest = match denied.commit_quota_and_refresh_heads() {
            Ok(checkpoint) => checkpoint,
            Err(QuotaCommitFailure::Denied(_)) => {
                return Err(ArtifactStoreAggregateError::QuotaExhausted);
            }
            Err(QuotaCommitFailure::Hard(error)) => return Err(error),
        };
        self.commit_recorded_command(
            denied,
            request,
            identity.object_key(),
            None,
            history.receipt().clone(),
            None,
            authority_input_digest,
            quota_checkpoint_digest,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_lifecycle_command(
        &mut self,
        next: Self,
        request: &ArtifactCommandRequest,
        task_id: TaskId,
        object: &ArtifactObjectIdentity,
        before: ContentDigest,
        authority_input_digest: ContentDigest,
        transition: Result<ArtifactAuthorityReceipt, &'static str>,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let raw_receipt = match transition {
            Ok(receipt) => receipt,
            Err(code) => {
                return self.finish_denied_command(
                    request,
                    task_id,
                    object,
                    before,
                    authority_input_digest,
                    code,
                );
            }
        };
        let after = semantic_receipt_state_digest(&raw_receipt)?;
        let result = raw_receipt.receipt_digest().clone();
        let terminal = ArtifactCommandTerminalProjection::applied(before.clone(), after, result)
            .map_err(map_history_error)?;
        let mut applied = next;
        let history = applied
            .history
            .execute(request.clone(), || Ok(terminal))
            .map_err(map_history_error)?;
        if history.disposition() != ArtifactCommandExecutionDisposition::Recorded
            || applied
                .command_tasks
                .insert(request.key().clone(), task_id.clone())
                .is_some()
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        match applied.commit_quota_and_refresh_heads() {
            Ok(quota_checkpoint_digest) => {
                let lifecycle = applied
                    .lifecycle
                    .current_receipt(object)
                    .map_err(|_| ArtifactStoreAggregateError::CorruptState)?;
                self.commit_recorded_command(
                    applied,
                    request,
                    object.key(),
                    Some(object.generation().get()),
                    history.receipt().clone(),
                    Some(lifecycle),
                    authority_input_digest,
                    quota_checkpoint_digest,
                )
            }
            Err(QuotaCommitFailure::Denied(code)) => self.finish_denied_command(
                request,
                task_id,
                object,
                before,
                authority_input_digest,
                code,
            ),
            Err(QuotaCommitFailure::Hard(error)) => Err(error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_denied_command(
        &mut self,
        request: &ArtifactCommandRequest,
        task_id: TaskId,
        object: &ArtifactObjectIdentity,
        before: ContentDigest,
        authority_input_digest: ContentDigest,
        code: &'static str,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let result = digest(
            DENIAL_RESULT_DOMAIN,
            &CanonicalValue::Object(vec![
                string("denial_code", code),
                string("before_state_digest", before.as_str()),
            ]),
        )?;
        let terminal = ArtifactCommandTerminalProjection::denied(
            denial_code(code),
            before.clone(),
            before,
            result,
        )
        .map_err(map_history_error)?;
        let mut denied = self.clone();
        let history = denied
            .history
            .execute(request.clone(), || Ok(terminal))
            .map_err(map_history_error)?;
        if history.disposition() != ArtifactCommandExecutionDisposition::Recorded
            || denied
                .command_tasks
                .insert(request.key().clone(), task_id)
                .is_some()
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        let quota_checkpoint_digest = match denied.commit_quota_and_refresh_heads() {
            Ok(checkpoint) => checkpoint,
            Err(QuotaCommitFailure::Denied(_)) => {
                return Err(ArtifactStoreAggregateError::QuotaExhausted);
            }
            Err(QuotaCommitFailure::Hard(error)) => return Err(error),
        };
        self.commit_recorded_command(
            denied,
            request,
            object.key(),
            Some(object.generation().get()),
            history.receipt().clone(),
            None,
            authority_input_digest,
            quota_checkpoint_digest,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn commit_recorded_command(
        &mut self,
        mut next: Self,
        request: &ArtifactCommandRequest,
        object_key: &ArtifactObjectKey,
        object_generation: Option<u64>,
        history: ArtifactCommandReceipt,
        lifecycle: Option<ArtifactAuthorityReceipt>,
        authority_input_digest: ContentDigest,
        quota_checkpoint_digest: ContentDigest,
    ) -> Result<ArtifactStoreCommandExecution, ArtifactStoreAggregateError> {
        let aggregate_state_digest = next.command_state_digest(
            object_key,
            object_generation,
            history.receipt_digest(),
            lifecycle.as_ref(),
            &quota_checkpoint_digest,
        )?;
        let receipt = ArtifactStoreCommandReceipt::new(
            history,
            lifecycle,
            authority_input_digest,
            quota_checkpoint_digest,
            aggregate_state_digest,
        )?;
        if next
            .terminal_receipts
            .insert(request.key().clone(), receipt.clone())
            .is_some()
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        *self = next;
        Ok(ArtifactStoreCommandExecution {
            disposition: ArtifactCommandExecutionDisposition::Recorded,
            receipt,
        })
    }

    fn semantic_state_digest(
        &self,
        object: &ArtifactObjectIdentity,
    ) -> Result<ContentDigest, ArtifactStoreAggregateError> {
        match self.lifecycle.current_head(object) {
            Ok(head) => artifact_current_head_input_digest(&head),
            Err(
                ArtifactLifecycleError::ObjectNotFound | ArtifactLifecycleError::GenerationMismatch,
            ) => match self.current_head_for_key(object.key())? {
                Some(head) => artifact_current_head_input_digest(&head),
                None => digest(
                    ABSENT_STATE_DOMAIN,
                    &CanonicalValue::Object(vec![
                        string("project_id", object.key().project_id().as_str()),
                        string("algorithm", object.key().algorithm()),
                        string("content_digest", object.key().content_digest().as_str()),
                    ]),
                ),
            },
            Err(_) => Err(ArtifactStoreAggregateError::CorruptState),
        }
    }

    fn command_state_digest(
        &self,
        object_key: &ArtifactObjectKey,
        object_generation: Option<u64>,
        command_receipt_digest: &ContentDigest,
        lifecycle: Option<&ArtifactAuthorityReceipt>,
        quota_checkpoint_digest: &ContentDigest,
    ) -> Result<ContentDigest, ArtifactStoreAggregateError> {
        let lifecycle_metadata_digest = self
            .lifecycle
            .metadata_state_digest()
            .map_err(|_| ArtifactStoreAggregateError::CorruptState)?;
        let history_checkpoint_set_digest = self.history_checkpoint_set_digest()?;
        let staging_state_digest = self.staging_state_digest()?;
        let command_task_state_digest = self.command_task_state_digest()?;
        let prior_terminal_state_digest = self.prior_terminal_state_digest()?;
        digest(
            AGGREGATE_STATE_DOMAIN,
            &CanonicalValue::Object(vec![
                string("store_id", self.store_id.as_str()),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string(
                    "lifecycle_metadata_digest",
                    lifecycle_metadata_digest.as_str(),
                ),
                string(
                    "history_checkpoint_set_digest",
                    history_checkpoint_set_digest.as_str(),
                ),
                string("staging_state_digest", staging_state_digest.as_str()),
                string(
                    "command_task_state_digest",
                    command_task_state_digest.as_str(),
                ),
                string(
                    "prior_terminal_state_digest",
                    prior_terminal_state_digest.as_str(),
                ),
                string("project_id", object_key.project_id().as_str()),
                string("algorithm", object_key.algorithm()),
                string("content_digest", object_key.content_digest().as_str()),
                string(
                    "object_generation",
                    object_generation.map_or_else(|| "NONE".to_owned(), |value| value.to_string()),
                ),
                string("command_receipt_digest", command_receipt_digest.as_str()),
                string(
                    "lifecycle_receipt_digest",
                    lifecycle.map_or("NONE", |receipt| receipt.receipt_digest().as_str()),
                ),
                string("quota_checkpoint_digest", quota_checkpoint_digest.as_str()),
                string(
                    "terminal_command_count",
                    self.history
                        .head(&crate::history::ArtifactCommandObjectScope::new(
                            object_key.project_id().clone(),
                            object_key.content_digest().clone(),
                        ))
                        .map_err(map_history_error)?
                        .high_water()
                        .get()
                        .to_string(),
                ),
            ]),
        )
    }

    fn history_checkpoint_set_digest(&self) -> Result<ContentDigest, ArtifactStoreAggregateError> {
        let mut scopes = self
            .command_tasks
            .keys()
            .map(|key| {
                crate::history::ArtifactCommandObjectScope::new(
                    key.project_id().clone(),
                    key.content_digest().clone(),
                )
            })
            .collect::<Vec<_>>();
        scopes.sort_by(|left, right| {
            (
                left.project_id().as_str(),
                left.algorithm(),
                left.content_digest().as_str(),
            )
                .cmp(&(
                    right.project_id().as_str(),
                    right.algorithm(),
                    right.content_digest().as_str(),
                ))
        });
        scopes.dedup();
        let rows = scopes
            .into_iter()
            .map(|scope| {
                let checkpoint = self.history.checkpoint(&scope).map_err(map_history_error)?;
                Ok(CanonicalValue::Object(vec![
                    string("project_id", scope.project_id().as_str()),
                    string("algorithm", scope.algorithm()),
                    string("content_digest", scope.content_digest().as_str()),
                    string(
                        "high_water",
                        checkpoint.head().high_water().get().to_string(),
                    ),
                    string("head_digest", checkpoint.head().head_digest().as_str()),
                    string("checkpoint_digest", checkpoint.checkpoint_digest().as_str()),
                ]))
            })
            .collect::<Result<Vec<_>, ArtifactStoreAggregateError>>()?;
        digest(
            "lattice.artifact.aggregate-history-checkpoint-set",
            &CanonicalValue::Array(rows),
        )
    }

    fn staging_state_digest(&self) -> Result<ContentDigest, ArtifactStoreAggregateError> {
        let mut rows = self
            .staging
            .values()
            .map(|reservation| {
                let identity = reservation.identity();
                CanonicalValue::Object(vec![
                    string("project_id", identity.project_id().as_str()),
                    string("algorithm", identity.object_key().algorithm()),
                    string(
                        "content_digest",
                        identity.object_key().content_digest().as_str(),
                    ),
                    string("task_id", identity.task_id().as_str()),
                    string("reservation_id", identity.value()),
                    string("staging_bytes", reservation.bytes().to_string()),
                    string("staging_streams", reservation.streams().to_string()),
                    string("status", staging_state_text(reservation.state())),
                ])
            })
            .collect::<Vec<_>>();
        rows.sort_by(compare_canonical_values);
        digest(
            "lattice.artifact.aggregate-staging-state",
            &CanonicalValue::Array(rows),
        )
    }

    fn command_task_state_digest(&self) -> Result<ContentDigest, ArtifactStoreAggregateError> {
        let mut rows = self
            .command_tasks
            .iter()
            .map(|(key, task_id)| {
                CanonicalValue::Object(vec![
                    string("project_id", key.project_id().as_str()),
                    string("algorithm", key.algorithm()),
                    string("content_digest", key.content_digest().as_str()),
                    string("command_id", key.command_id()),
                    string("task_id", task_id.as_str()),
                ])
            })
            .collect::<Vec<_>>();
        rows.sort_by(compare_canonical_values);
        digest(
            "lattice.artifact.aggregate-command-task-state",
            &CanonicalValue::Array(rows),
        )
    }

    fn prior_terminal_state_digest(&self) -> Result<ContentDigest, ArtifactStoreAggregateError> {
        let mut rows = self
            .terminal_receipts
            .iter()
            .map(|(key, receipt)| {
                CanonicalValue::Object(vec![
                    string("project_id", key.project_id().as_str()),
                    string("algorithm", key.algorithm()),
                    string("content_digest", key.content_digest().as_str()),
                    string("command_id", key.command_id()),
                    string(
                        "history_receipt_digest",
                        receipt.history().receipt_digest().as_str(),
                    ),
                    string(
                        "authority_input_digest",
                        receipt.authority_input_digest().as_str(),
                    ),
                    string(
                        "quota_checkpoint_digest",
                        receipt.quota_checkpoint_digest().as_str(),
                    ),
                    string(
                        "aggregate_state_digest",
                        receipt.aggregate_state_digest().as_str(),
                    ),
                    string("receipt_digest", receipt.receipt_digest().as_str()),
                ])
            })
            .collect::<Vec<_>>();
        rows.sort_by(compare_canonical_values);
        digest(
            "lattice.artifact.aggregate-prior-terminal-state",
            &CanonicalValue::Array(rows),
        )
    }

    /// Clones every replay-authoritative metadata row while deliberately
    /// replacing the physically separate fake payload backend with an empty
    /// backend.
    #[must_use]
    pub(crate) fn snapshot_metadata_clone(&self) -> Self {
        Self {
            store_id: self.store_id.clone(),
            limits: self.limits,
            lifecycle: self.lifecycle.clone(),
            bytes: FakeArtifactBytes::default(),
            history: self.history.clone(),
            staging: self.staging.clone(),
            command_tasks: self.command_tasks.clone(),
            quota_head_set: self.quota_head_set.clone(),
            retired_quota_objects: self.retired_quota_objects.clone(),
            terminal_receipts: self.terminal_receipts.clone(),
        }
    }

    /// Hashes the compact rollback-sensitive roots retained by an independent
    /// aggregate checkpoint. The anchor includes delete/reconciliation state,
    /// every strict history head, quota heads, staging, command attribution,
    /// terminal receipts, and retired-generation membership without retaining
    /// a second owner.
    pub(crate) fn snapshot_trust_anchor_digest(
        &self,
    ) -> Result<ContentDigest, ArtifactStoreAggregateError> {
        let lifecycle_metadata_digest = self
            .lifecycle
            .metadata_state_digest()
            .map_err(|_| ArtifactStoreAggregateError::CorruptState)?;
        let quota_checkpoint_digest = self
            .quota_head_set
            .as_ref()
            .map(ArtifactQuotaHeadSet::checkpoint_digest)
            .ok_or(ArtifactStoreAggregateError::CorruptState)?;
        let retired_scope_digest = digest(
            "lattice.artifact.aggregate-retired-object-scope-set",
            &CanonicalValue::Array(self.snapshot_retired_object_rows()),
        )?;
        digest(
            "lattice.artifact.aggregate-trust-anchor",
            &CanonicalValue::Object(vec![
                string("store_id", self.store_id.as_str()),
                string(
                    "limit_snapshot_digest",
                    self.limits
                        .limit_snapshot_digest()
                        .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?
                        .as_str(),
                ),
                string(
                    "lifecycle_metadata_digest",
                    lifecycle_metadata_digest.as_str(),
                ),
                string(
                    "history_checkpoint_set_digest",
                    self.history_checkpoint_set_digest()?.as_str(),
                ),
                string(
                    "staging_state_digest",
                    self.staging_state_digest()?.as_str(),
                ),
                string(
                    "command_task_state_digest",
                    self.command_task_state_digest()?.as_str(),
                ),
                string(
                    "prior_terminal_state_digest",
                    self.prior_terminal_state_digest()?.as_str(),
                ),
                string("quota_checkpoint_digest", quota_checkpoint_digest.as_str()),
                string("retired_scope_digest", retired_scope_digest.as_str()),
            ]),
        )
    }

    /// Validates cross-owner metadata invariants without mutating this owner.
    ///
    /// This replays every strict history stream, checks all command/terminal
    /// joins, rebuilds every aggregate receipt digest, and performs an
    /// idempotent quota/lifecycle root recompute on a private clone.
    pub(crate) fn validate_snapshot_metadata(&self) -> Result<(), ArtifactStoreAggregateError> {
        if self.lifecycle.limits() != self.limits
            || self.history.sorted_receipts().len() != self.command_tasks.len()
            || self.history.sorted_receipts().len() != self.terminal_receipts.len()
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }

        for history_receipt in self.history.sorted_receipts() {
            let key = history_receipt.request().key();
            if !self.command_tasks.contains_key(key) {
                return Err(ArtifactStoreAggregateError::CorruptState);
            }
            let terminal = self
                .terminal_receipts
                .get(key)
                .ok_or(ArtifactStoreAggregateError::CorruptState)?;
            if terminal.history() != history_receipt {
                return Err(ArtifactStoreAggregateError::CorruptState);
            }
            let rebuilt = ArtifactStoreCommandReceipt::new(
                terminal.history().clone(),
                terminal.lifecycle().cloned(),
                terminal.authority_input_digest().clone(),
                terminal.quota_checkpoint_digest().clone(),
                terminal.aggregate_state_digest().clone(),
            )?;
            if &rebuilt != terminal {
                return Err(ArtifactStoreAggregateError::CorruptState);
            }
        }

        for scope in self.snapshot_history_scopes() {
            let raw = self
                .history
                .export_untrusted(&scope)
                .map_err(map_history_error)?;
            let checkpoint = self.history.checkpoint(&scope).map_err(map_history_error)?;
            let replayed = ArtifactCommandHistory::replay_untrusted(&raw, &checkpoint)
                .map_err(map_history_error)?;
            if replayed
                .export_untrusted(&scope)
                .map_err(map_history_error)?
                != raw
            {
                return Err(ArtifactStoreAggregateError::CorruptState);
            }
        }

        let limit_snapshot_digest = self
            .limits
            .limit_snapshot_digest()
            .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?;
        let heads = self
            .quota_head_set
            .as_ref()
            .ok_or(ArtifactStoreAggregateError::CorruptState)?;
        if heads.limit_snapshot_digest() != &limit_snapshot_digest {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        for (scope, head) in heads.sorted_heads() {
            if head.scope() != scope
                || head.producer_id() != ARTIFACT_STORE_PRODUCER_ID
                || head.producer_version() != ARTIFACT_STORE_PRODUCER_VERSION
                || head.runtime() != RuntimeKind::Fake
                || head.limit_snapshot_digest() != &limit_snapshot_digest
            {
                return Err(ArtifactStoreAggregateError::CorruptState);
            }
        }

        let mut recomputed = self.snapshot_metadata_clone();
        let recomputed_checkpoint = match recomputed.commit_quota_and_refresh_heads() {
            Ok(checkpoint) => checkpoint,
            Err(QuotaCommitFailure::Denied(_) | QuotaCommitFailure::Hard(_)) => {
                return Err(ArtifactStoreAggregateError::CorruptState);
            }
        };
        if recomputed.quota_head_set != self.quota_head_set
            || recomputed.lifecycle != self.lifecycle
            || &recomputed_checkpoint != heads.checkpoint_digest()
        {
            return Err(ArtifactStoreAggregateError::CorruptState);
        }
        Ok(())
    }

    /// Returns the complete strict raw aggregate snapshot with payload bytes
    /// excluded by construction.
    pub(crate) fn snapshot_canonical_state(
        &self,
    ) -> Result<CanonicalValue, ArtifactStoreAggregateError> {
        self.validate_snapshot_metadata()?;

        let limit_snapshot_digest = self
            .limits
            .limit_snapshot_digest()
            .map_err(|_| ArtifactStoreAggregateError::Canonicalization)?;
        let limits = CanonicalValue::Object(
            ArtifactLimitKind::ALL
                .into_iter()
                .map(|kind| string(kind.as_str(), self.limits.get(kind).to_string()))
                .collect(),
        );
        let lifecycle = self
            .lifecycle
            .canonical_metadata_state()
            .map_err(|_| ArtifactStoreAggregateError::CorruptState)?;
        let histories = self.snapshot_history_rows()?;
        let quota = self.snapshot_quota_state()?;

        Ok(CanonicalValue::Object(vec![
            string("version", HASH_VERSION),
            string("producer_id", ARTIFACT_STORE_PRODUCER_ID),
            string("producer_version", ARTIFACT_STORE_PRODUCER_VERSION),
            string("runtime", "FAKE"),
            string("store_id", self.store_id.as_str()),
            ("limits".to_owned(), limits),
            string("limit_snapshot_digest", limit_snapshot_digest.as_str()),
            ("lifecycle".to_owned(), lifecycle),
            ("histories".to_owned(), CanonicalValue::Array(histories)),
            ("quota".to_owned(), quota),
            (
                "staging".to_owned(),
                CanonicalValue::Array(self.snapshot_staging_rows()),
            ),
            (
                "command_tasks".to_owned(),
                CanonicalValue::Array(self.snapshot_command_task_rows()),
            ),
            (
                "retired_object_scopes".to_owned(),
                CanonicalValue::Array(self.snapshot_retired_object_rows()),
            ),
            (
                "terminal_receipts".to_owned(),
                CanonicalValue::Array(self.snapshot_terminal_receipt_rows()),
            ),
        ]))
    }

    fn snapshot_history_scopes(&self) -> Vec<crate::history::ArtifactCommandObjectScope> {
        let mut scopes = self
            .history
            .sorted_receipts()
            .into_iter()
            .map(|receipt| {
                let key = receipt.request().key();
                crate::history::ArtifactCommandObjectScope::new(
                    key.project_id().clone(),
                    key.content_digest().clone(),
                )
            })
            .collect::<Vec<_>>();
        scopes.sort_by(|left, right| {
            (
                left.project_id().as_str(),
                left.algorithm(),
                left.content_digest().as_str(),
            )
                .cmp(&(
                    right.project_id().as_str(),
                    right.algorithm(),
                    right.content_digest().as_str(),
                ))
        });
        scopes.dedup();
        scopes
    }

    fn snapshot_history_rows(&self) -> Result<Vec<CanonicalValue>, ArtifactStoreAggregateError> {
        self.snapshot_history_scopes()
            .into_iter()
            .map(|scope| {
                let checkpoint = self.history.checkpoint(&scope).map_err(map_history_error)?;
                let head = checkpoint.head();
                let raw = self
                    .history
                    .export_untrusted(&scope)
                    .map_err(map_history_error)?;
                Ok(CanonicalValue::Object(vec![
                    string("project_id", scope.project_id().as_str()),
                    string("algorithm", scope.algorithm()),
                    string("content_digest", scope.content_digest().as_str()),
                    (
                        "checkpoint".to_owned(),
                        CanonicalValue::Object(vec![
                            string("high_water", head.high_water().get().to_string()),
                            (
                                "tail_digest".to_owned(),
                                optional_snapshot_digest(head.tail_digest()),
                            ),
                            string("denial_count", head.denial_count().get().to_string()),
                            (
                                "denial_tail_digest".to_owned(),
                                optional_snapshot_digest(head.denial_tail_digest()),
                            ),
                            string("head_digest", head.head_digest().as_str()),
                            string("checkpoint_digest", checkpoint.checkpoint_digest().as_str()),
                        ]),
                    ),
                    ("strict_history".to_owned(), raw),
                ]))
            })
            .collect()
    }

    fn snapshot_quota_state(&self) -> Result<CanonicalValue, ArtifactStoreAggregateError> {
        let heads = self
            .quota_head_set
            .as_ref()
            .ok_or(ArtifactStoreAggregateError::CorruptState)?;
        let rows = heads
            .sorted_heads()
            .into_iter()
            .map(|(scope, head)| {
                let projection = CanonicalValue::Object(
                    ArtifactLimitKind::ALL
                        .into_iter()
                        .map(|kind| string(kind.as_str(), head.projection().get(kind).to_string()))
                        .collect(),
                );
                CanonicalValue::Object(vec![
                    ("scope".to_owned(), snapshot_quota_scope(scope)),
                    string("producer_id", head.producer_id()),
                    string("producer_version", head.producer_version()),
                    string("runtime", runtime_text(head.runtime())),
                    string("revision", head.revision().get().to_string()),
                    ("projection".to_owned(), projection),
                    string(
                        "limit_snapshot_digest",
                        head.limit_snapshot_digest().as_str(),
                    ),
                    string(
                        "predecessor_head_digest",
                        head.predecessor_head_digest().as_str(),
                    ),
                    string(
                        "transition_tail_digest",
                        head.transition_tail_digest().as_str(),
                    ),
                    string("head_digest", head.head_digest().as_str()),
                ])
            })
            .collect();
        Ok(CanonicalValue::Object(vec![
            string(
                "limit_snapshot_digest",
                heads.limit_snapshot_digest().as_str(),
            ),
            string("checkpoint_digest", heads.checkpoint_digest().as_str()),
            ("heads".to_owned(), CanonicalValue::Array(rows)),
        ]))
    }

    fn snapshot_staging_rows(&self) -> Vec<CanonicalValue> {
        let mut rows = self
            .staging
            .values()
            .map(|reservation| {
                let identity = reservation.identity();
                CanonicalValue::Object(vec![
                    string("project_id", identity.project_id().as_str()),
                    string("algorithm", identity.object_key().algorithm()),
                    string(
                        "content_digest",
                        identity.object_key().content_digest().as_str(),
                    ),
                    string("task_id", identity.task_id().as_str()),
                    string("reservation_id", identity.value()),
                    string("staging_bytes", reservation.bytes().to_string()),
                    string("staging_streams", reservation.streams().to_string()),
                    string("status", staging_state_text(reservation.state())),
                ])
            })
            .collect::<Vec<_>>();
        rows.sort_by(compare_canonical_values);
        rows
    }

    fn snapshot_command_task_rows(&self) -> Vec<CanonicalValue> {
        let mut rows = self
            .command_tasks
            .iter()
            .map(|(key, task_id)| {
                CanonicalValue::Object(vec![
                    string("project_id", key.project_id().as_str()),
                    string("algorithm", key.algorithm()),
                    string("content_digest", key.content_digest().as_str()),
                    string("command_id", key.command_id()),
                    string("task_id", task_id.as_str()),
                ])
            })
            .collect::<Vec<_>>();
        rows.sort_by(compare_canonical_values);
        rows
    }

    fn snapshot_retired_object_rows(&self) -> Vec<CanonicalValue> {
        let mut rows = self
            .retired_quota_objects
            .iter()
            .map(snapshot_object_identity)
            .collect::<Vec<_>>();
        rows.sort_by(compare_canonical_values);
        rows
    }

    fn snapshot_terminal_receipt_rows(&self) -> Vec<CanonicalValue> {
        let mut rows = self
            .terminal_receipts
            .iter()
            .map(|(key, receipt)| {
                let history = receipt.history();
                CanonicalValue::Object(vec![
                    string("project_id", key.project_id().as_str()),
                    string("algorithm", key.algorithm()),
                    string("content_digest", key.content_digest().as_str()),
                    string("command_id", key.command_id()),
                    string("producer_id", receipt.producer_id()),
                    string("producer_version", receipt.producer_version()),
                    string("runtime", runtime_text(receipt.runtime())),
                    string(
                        "history_request_digest",
                        history.request().request_digest().as_str(),
                    ),
                    string("history_ordinal", history.ordinal().get().to_string()),
                    (
                        "history_predecessor_digest".to_owned(),
                        optional_snapshot_digest(history.predecessor_digest()),
                    ),
                    string("history_outcome", command_outcome_text(history.outcome())),
                    (
                        "history_denial_code".to_owned(),
                        optional_snapshot_string(history.denial_code()),
                    ),
                    string(
                        "history_before_state_digest",
                        history.before_state_digest().as_str(),
                    ),
                    string(
                        "history_after_state_digest",
                        history.after_state_digest().as_str(),
                    ),
                    string("history_result_digest", history.result_digest().as_str()),
                    string("history_record_digest", history.record_digest().as_str()),
                    string("history_receipt_digest", history.receipt_digest().as_str()),
                    (
                        "lifecycle_receipt".to_owned(),
                        receipt.lifecycle().map_or(CanonicalValue::Null, |value| {
                            authority_receipt_canonical_value(value)
                        }),
                    ),
                    (
                        "lifecycle_receipt_digest".to_owned(),
                        receipt.lifecycle().map_or(CanonicalValue::Null, |value| {
                            CanonicalValue::String(value.receipt_digest().as_str().to_owned())
                        }),
                    ),
                    string(
                        "authority_input_digest",
                        receipt.authority_input_digest().as_str(),
                    ),
                    string(
                        "quota_checkpoint_digest",
                        receipt.quota_checkpoint_digest().as_str(),
                    ),
                    string(
                        "aggregate_state_digest",
                        receipt.aggregate_state_digest().as_str(),
                    ),
                    string("receipt_digest", receipt.receipt_digest().as_str()),
                ])
            })
            .collect::<Vec<_>>();
        rows.sort_by(compare_canonical_values);
        rows
    }
}

fn optional_snapshot_digest(value: Option<&ContentDigest>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |digest| {
        CanonicalValue::String(digest.as_str().to_owned())
    })
}

fn optional_snapshot_string(value: Option<&str>) -> CanonicalValue {
    value.map_or(CanonicalValue::Null, |text| {
        CanonicalValue::String(text.to_owned())
    })
}

fn snapshot_object_identity(object: &ArtifactObjectIdentity) -> CanonicalValue {
    CanonicalValue::Object(vec![
        string("project_id", object.key().project_id().as_str()),
        string("algorithm", object.key().algorithm()),
        string("content_digest", object.key().content_digest().as_str()),
        string("generation", object.generation().get().to_string()),
    ])
}

fn snapshot_quota_scope(scope: &ArtifactQuotaScope) -> CanonicalValue {
    match scope {
        ArtifactQuotaScope::Object(object) => CanonicalValue::Object(vec![
            string("scope_type", "OBJECT"),
            ("object".to_owned(), snapshot_object_identity(object)),
        ]),
        ArtifactQuotaScope::Task {
            project_id,
            task_id,
        } => CanonicalValue::Object(vec![
            string("scope_type", "TASK"),
            string("project_id", project_id.as_str()),
            string("task_id", task_id.as_str()),
        ]),
        ArtifactQuotaScope::Project(project_id) => CanonicalValue::Object(vec![
            string("scope_type", "PROJECT"),
            string("project_id", project_id.as_str()),
        ]),
        ArtifactQuotaScope::Store(store) => CanonicalValue::Object(vec![
            string("scope_type", "STORE"),
            string("store_id", store.as_str()),
        ]),
    }
}

fn staging_state_text(state: ArtifactStagingState) -> &'static str {
    match state {
        ArtifactStagingState::Active => "ACTIVE",
        ArtifactStagingState::SealedOrphan => "SEALED_ORPHAN",
        ArtifactStagingState::ReconciliationRequired => "RECONCILIATION_REQUIRED",
        ArtifactStagingState::VerifiedPublished => "VERIFIED_PUBLISHED",
        ArtifactStagingState::VerifiedCleaned => "VERIFIED_CLEANED",
    }
}

fn compare_canonical_values(left: &CanonicalValue, right: &CanonicalValue) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    fn rank(value: &CanonicalValue) -> u8 {
        match value {
            CanonicalValue::Null => 0,
            CanonicalValue::Bool(_) => 1,
            CanonicalValue::String(_) => 2,
            CanonicalValue::Array(_) => 3,
            CanonicalValue::Object(_) => 4,
        }
    }

    match (left, right) {
        (CanonicalValue::Null, CanonicalValue::Null) => Ordering::Equal,
        (CanonicalValue::Bool(left), CanonicalValue::Bool(right)) => left.cmp(right),
        (CanonicalValue::String(left), CanonicalValue::String(right)) => left.cmp(right),
        (CanonicalValue::Array(left), CanonicalValue::Array(right)) => {
            for (left, right) in left.iter().zip(right) {
                let ordering = compare_canonical_values(left, right);
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            left.len().cmp(&right.len())
        }
        (CanonicalValue::Object(left), CanonicalValue::Object(right)) => {
            for ((left_key, left_value), (right_key, right_value)) in left.iter().zip(right) {
                let key_ordering = left_key.cmp(right_key);
                if key_ordering != Ordering::Equal {
                    return key_ordering;
                }
                let value_ordering = compare_canonical_values(left_value, right_value);
                if value_ordering != Ordering::Equal {
                    return value_ordering;
                }
            }
            left.len().cmp(&right.len())
        }
        _ => rank(left).cmp(&rank(right)),
    }
}
