//! Strict metadata-only checkpoint, export, and replay for the fake root owner.

use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, normalize_nfc};
use lattice_contracts::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ContentDigest, RuntimeKind,
};

use crate::{ArtifactStoreAggregateError, ArtifactStoreIdentity, FakeArtifactStore};

const SNAPSHOT_VERSION: &str = "1.0";
const SNAPSHOT_DOMAIN: &str = "lattice.artifact.aggregate-snapshot";
const CHECKPOINT_DOMAIN: &str = "lattice.artifact.aggregate-checkpoint";
const REPLAY_DEPTH_SLACK: usize = 8;
const REPLAY_NODE_SLACK: usize = 1_024;
const REPLAY_COLLECTION_SLACK: usize = 32;
const REPLAY_BYTE_SLACK: usize = 1_048_576;

/// Independently retained compact trusted aggregate checkpoint.
///
/// This trust anchor contains only identity and digest commitments. It never
/// retains an owner, lifecycle row, history row, terminal receipt, or payload
/// byte. Successful replay reconstructs all owner metadata from the untrusted
/// snapshot before checking these commitments.
#[derive(Clone, Eq, PartialEq)]
pub struct ArtifactStoreCheckpoint {
    store_id: ArtifactStoreIdentity,
    limit_snapshot_digest: ContentDigest,
    trust_anchor_digest: ContentDigest,
    snapshot_digest: ContentDigest,
    checkpoint_digest: ContentDigest,
    replay_bounds: ReplayBounds,
}

impl ArtifactStoreCheckpoint {
    /// Frozen aggregate snapshot/checkpoint schema version.
    #[must_use]
    pub const fn version(&self) -> &'static str {
        SNAPSHOT_VERSION
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

    /// This checkpoint can restore only the visibly fake runtime.
    #[must_use]
    pub const fn runtime(&self) -> RuntimeKind {
        RuntimeKind::Fake
    }

    /// Exact fake-store identity bound by this checkpoint.
    #[must_use]
    pub const fn store_id(&self) -> &ArtifactStoreIdentity {
        &self.store_id
    }

    /// Immutable configured-limit commitment bound by this checkpoint.
    ///
    /// # Errors
    ///
    /// The digest was validated when the checkpoint was created.
    pub fn limit_snapshot_digest(&self) -> Result<ContentDigest, ArtifactStoreReplayError> {
        Ok(self.limit_snapshot_digest.clone())
    }

    /// Snapshot-domain digest of the complete raw-byte-free metadata document.
    #[must_use]
    pub const fn snapshot_digest(&self) -> &ContentDigest {
        &self.snapshot_digest
    }

    /// Separate checkpoint-domain digest binding owner identity and snapshot.
    #[must_use]
    pub const fn checkpoint_digest(&self) -> &ContentDigest {
        &self.checkpoint_digest
    }

    fn verify(&self) -> Result<(), ArtifactStoreReplayError> {
        let checkpoint_digest = checkpoint_digest(
            &self.store_id,
            &self.limit_snapshot_digest,
            &self.trust_anchor_digest,
            &self.snapshot_digest,
            &self.replay_bounds,
        )?;
        if checkpoint_digest != self.checkpoint_digest {
            return Err(ArtifactStoreReplayError::TrustedCheckpointInvalid);
        }
        Ok(())
    }
}

impl fmt::Debug for ArtifactStoreCheckpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactStoreCheckpoint")
            .field("version", &SNAPSHOT_VERSION)
            .field("producer_id", &ARTIFACT_STORE_PRODUCER_ID)
            .field("producer_version", &ARTIFACT_STORE_PRODUCER_VERSION)
            .field("runtime", &RuntimeKind::Fake)
            .field("store_id", &self.store_id)
            .field("limit_snapshot_digest", &self.limit_snapshot_digest)
            .field("trust_anchor", &self.trust_anchor_digest)
            .field("payload_bytes", &"[ABSENT]")
            .field("snapshot_digest", &self.snapshot_digest)
            .field("checkpoint_digest", &self.checkpoint_digest)
            .field("replay_bounds", &self.replay_bounds)
            .finish()
    }
}

/// Fail-closed aggregate replay/checkpoint failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactStoreReplayError {
    /// A nested canonical value exceeded an iterative replay safety bound.
    ReplayLimit {
        /// Stable name of the exceeded bound.
        field: &'static str,
    },
    /// Canonical encoding or hash framing failed.
    Canonicalization,
    /// A locally produced digest violated the shared digest contract.
    InvalidDigest,
    /// Current or checkpointed metadata failed internal owner invariants.
    InvalidMetadata,
    /// The private trusted checkpoint no longer matches its own digest chain.
    TrustedCheckpointInvalid,
    /// Untrusted raw data was not the exact current trusted snapshot.
    SnapshotMismatch,
}

impl ArtifactStoreReplayError {
    /// Stable non-secret diagnostic code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ReplayLimit { .. } => "ARTIFACT_STORE_REPLAY_LIMIT",
            Self::Canonicalization => "ARTIFACT_STORE_REPLAY_CANONICALIZATION",
            Self::InvalidDigest => "ARTIFACT_STORE_REPLAY_INVALID_DIGEST",
            Self::InvalidMetadata => "ARTIFACT_STORE_REPLAY_INVALID_METADATA",
            Self::TrustedCheckpointInvalid => "ARTIFACT_STORE_REPLAY_TRUSTED_CHECKPOINT_INVALID",
            Self::SnapshotMismatch => "ARTIFACT_STORE_REPLAY_SNAPSHOT_MISMATCH",
        }
    }
}

impl fmt::Display for ArtifactStoreReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ArtifactStoreReplayError {}

impl FakeArtifactStore {
    /// Creates an independent metadata-only trusted checkpoint.
    ///
    /// # Errors
    ///
    /// Rejects any internally inconsistent lifecycle, history, quota,
    /// staging, command-task, retired-scope, or terminal-receipt row.
    pub fn checkpoint(&self) -> Result<ArtifactStoreCheckpoint, ArtifactStoreReplayError> {
        let metadata = self.snapshot_metadata_clone();
        let raw = metadata
            .snapshot_canonical_state()
            .map_err(|error| map_metadata_error(&error))?;
        let replay_bounds = ReplayBounds::for_snapshot(&raw)?;
        let snapshot_digest = replay_digest(SNAPSHOT_DOMAIN, &raw)?;
        let limit_snapshot_digest = metadata
            .limits()
            .limit_snapshot_digest()
            .map_err(|_| ArtifactStoreReplayError::Canonicalization)?;
        let trust_anchor_digest = metadata
            .snapshot_trust_anchor_digest()
            .map_err(|error| map_metadata_error(&error))?;
        let store_id = metadata.store_id().clone();
        let checkpoint_digest = checkpoint_digest(
            &store_id,
            &limit_snapshot_digest,
            &trust_anchor_digest,
            &snapshot_digest,
            &replay_bounds,
        )?;
        Ok(ArtifactStoreCheckpoint {
            store_id,
            limit_snapshot_digest,
            trust_anchor_digest,
            snapshot_digest,
            checkpoint_digest,
            replay_bounds,
        })
    }

    /// Exports the complete strict aggregate metadata snapshot.
    ///
    /// Artifact payload bytes are physically excluded. The returned
    /// [`CanonicalValue`] is untrusted storage material and grants no mutation
    /// authority.
    ///
    /// # Errors
    ///
    /// Rejects internally inconsistent metadata or canonicalization failure.
    pub fn export_untrusted(&self) -> Result<CanonicalValue, ArtifactStoreReplayError> {
        self.snapshot_canonical_state()
            .map_err(|error| map_metadata_error(&error))
    }

    /// Restores a metadata-only fake owner after strict trusted replay.
    ///
    /// Iterative structural and byte bounds are checked before the trusted
    /// checkpoint. Replay reconstructs every metadata row from raw input,
    /// verifies all internal joins and digest chains, and then compares the
    /// reconstructed owner with the checkpoint's compact commitments. Changed,
    /// unknown, extra, reordered, truncated, cross-scope, fake-live, and
    /// coherent older-prefix inputs all fail without mutating either input.
    ///
    /// # Errors
    ///
    /// Returns a typed stable-code replay error for bounds, canonicalization,
    /// invalid trusted state, or any exact snapshot mismatch.
    pub fn replay_untrusted(
        raw: &CanonicalValue,
        trusted: &ArtifactStoreCheckpoint,
    ) -> Result<Self, ArtifactStoreReplayError> {
        preflight(raw, &trusted.replay_bounds)?;
        trusted.verify()?;
        let restored =
            Self::restore_snapshot(raw).map_err(|_| ArtifactStoreReplayError::SnapshotMismatch)?;
        let restored_snapshot_digest = replay_digest(SNAPSHOT_DOMAIN, raw)?;
        let restored_limit_snapshot_digest = restored
            .limits()
            .limit_snapshot_digest()
            .map_err(|_| ArtifactStoreReplayError::Canonicalization)?;
        let restored_trust_anchor_digest = restored
            .snapshot_trust_anchor_digest()
            .map_err(|_| ArtifactStoreReplayError::SnapshotMismatch)?;
        let restored_bounds = ReplayBounds::for_snapshot(raw)?;
        if restored.store_id() != &trusted.store_id
            || restored_limit_snapshot_digest != trusted.limit_snapshot_digest
            || restored_trust_anchor_digest != trusted.trust_anchor_digest
            || restored_snapshot_digest != trusted.snapshot_digest
            || restored_bounds != trusted.replay_bounds
        {
            return Err(ArtifactStoreReplayError::SnapshotMismatch);
        }
        Ok(restored)
    }
}

fn checkpoint_digest(
    store_id: &ArtifactStoreIdentity,
    limit_snapshot_digest: &ContentDigest,
    trust_anchor_digest: &ContentDigest,
    snapshot_digest: &ContentDigest,
    replay_bounds: &ReplayBounds,
) -> Result<ContentDigest, ArtifactStoreReplayError> {
    replay_digest(
        CHECKPOINT_DOMAIN,
        &CanonicalValue::Object(vec![
            string("version", SNAPSHOT_VERSION),
            string("producer_id", ARTIFACT_STORE_PRODUCER_ID),
            string("producer_version", ARTIFACT_STORE_PRODUCER_VERSION),
            string("runtime", "FAKE"),
            string("store_id", store_id.as_str()),
            string("limit_snapshot_digest", limit_snapshot_digest.as_str()),
            string("trust_anchor_digest", trust_anchor_digest.as_str()),
            string("snapshot_digest", snapshot_digest.as_str()),
            ("replay_bounds".to_owned(), replay_bounds.canonical_value()),
        ]),
    )
}

fn replay_digest(
    schema_id: &str,
    value: &CanonicalValue,
) -> Result<ContentDigest, ArtifactStoreReplayError> {
    let domain = HashDomain::new(schema_id, SNAPSHOT_VERSION)
        .map_err(|_| ArtifactStoreReplayError::Canonicalization)?;
    let hash =
        canonical_sha256(&domain, value).map_err(|_| ArtifactStoreReplayError::Canonicalization)?;
    ContentDigest::from_sha256(hash.to_hex()).map_err(|_| ArtifactStoreReplayError::InvalidDigest)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReplayBounds {
    depth: usize,
    nodes: usize,
    collection_entries: usize,
    string_bytes: usize,
    canonical_bytes: usize,
}

impl ReplayBounds {
    fn for_snapshot(raw: &CanonicalValue) -> Result<Self, ArtifactStoreReplayError> {
        let shape = measure(raw)?;
        Ok(Self {
            depth: shape.max_depth.saturating_add(REPLAY_DEPTH_SLACK),
            nodes: shape.nodes.saturating_add(REPLAY_NODE_SLACK),
            collection_entries: shape
                .max_collection_entries
                .saturating_add(REPLAY_COLLECTION_SLACK),
            string_bytes: shape.string_bytes.saturating_add(REPLAY_BYTE_SLACK),
            canonical_bytes: shape.canonical_bytes.saturating_add(REPLAY_BYTE_SLACK),
        })
    }

    fn canonical_value(&self) -> CanonicalValue {
        CanonicalValue::Object(vec![
            string("max_depth", self.depth.to_string()),
            string("max_nodes", self.nodes.to_string()),
            string(
                "max_collection_entries",
                self.collection_entries.to_string(),
            ),
            string("max_string_bytes", self.string_bytes.to_string()),
            string("max_canonical_bytes", self.canonical_bytes.to_string()),
        ])
    }
}

struct ReplayShape {
    max_depth: usize,
    nodes: usize,
    max_collection_entries: usize,
    string_bytes: usize,
    canonical_bytes: usize,
}

fn measure(raw: &CanonicalValue) -> Result<ReplayShape, ArtifactStoreReplayError> {
    let mut stack = vec![(raw, 1_usize)];
    let mut nodes = 0_usize;
    let mut string_bytes = 0_usize;
    let mut max_depth = 0_usize;
    let mut max_collection_entries = 0_usize;

    while let Some((value, depth)) = stack.pop() {
        max_depth = max_depth.max(depth);
        nodes = nodes
            .checked_add(1)
            .ok_or(ArtifactStoreReplayError::ReplayLimit { field: "nodes" })?;
        match value {
            CanonicalValue::Null | CanonicalValue::Bool(_) => {}
            CanonicalValue::String(value) => {
                checked_add_string_bytes(&mut string_bytes, value.len())?;
            }
            CanonicalValue::Array(values) => {
                max_collection_entries = max_collection_entries.max(values.len());
                for value in values.iter().rev() {
                    stack.push((value, depth + 1));
                }
            }
            CanonicalValue::Object(entries) => {
                max_collection_entries = max_collection_entries.max(entries.len());
                for (key, value) in entries.iter().rev() {
                    checked_add_string_bytes(&mut string_bytes, key.len())?;
                    stack.push((value, depth + 1));
                }
            }
        }
    }

    let canonical_bytes = encoded_canonical_len(raw, None)?;
    Ok(ReplayShape {
        max_depth,
        nodes,
        max_collection_entries,
        string_bytes,
        canonical_bytes,
    })
}

fn preflight(raw: &CanonicalValue, bounds: &ReplayBounds) -> Result<(), ArtifactStoreReplayError> {
    let mut stack = vec![(raw, 1_usize)];
    let mut nodes = 0_usize;
    let mut string_bytes = 0_usize;

    while let Some((value, depth)) = stack.pop() {
        if depth > bounds.depth {
            return Err(ArtifactStoreReplayError::ReplayLimit { field: "depth" });
        }
        nodes = nodes
            .checked_add(1)
            .ok_or(ArtifactStoreReplayError::ReplayLimit { field: "nodes" })?;
        if nodes > bounds.nodes {
            return Err(ArtifactStoreReplayError::ReplayLimit { field: "nodes" });
        }
        match value {
            CanonicalValue::Null | CanonicalValue::Bool(_) => {}
            CanonicalValue::String(value) => {
                add_string_bytes(&mut string_bytes, value.len(), bounds)?;
            }
            CanonicalValue::Array(values) => {
                if values.len() > bounds.collection_entries {
                    return Err(ArtifactStoreReplayError::ReplayLimit {
                        field: "array_entries",
                    });
                }
                for value in values.iter().rev() {
                    stack.push((value, depth + 1));
                }
            }
            CanonicalValue::Object(entries) => {
                if entries.len() > bounds.collection_entries {
                    return Err(ArtifactStoreReplayError::ReplayLimit {
                        field: "object_entries",
                    });
                }
                for (key, value) in entries.iter().rev() {
                    add_string_bytes(&mut string_bytes, key.len(), bounds)?;
                    stack.push((value, depth + 1));
                }
            }
        }
    }

    encoded_canonical_len(raw, Some(bounds.canonical_bytes))?;
    Ok(())
}

/// Computes the exact `lattice-cjson-1` encoded length without allocating the
/// encoded document. This must run before canonicalization of untrusted input:
/// JSON control characters can expand from one UTF-8 byte to six encoded
/// bytes.
fn encoded_canonical_len(
    raw: &CanonicalValue,
    limit: Option<usize>,
) -> Result<usize, ArtifactStoreReplayError> {
    let mut stack = vec![raw];
    let mut total = 0_usize;
    while let Some(value) = stack.pop() {
        match value {
            CanonicalValue::Null => add_canonical_bytes(&mut total, 4, limit)?,
            CanonicalValue::Bool(value) => {
                add_canonical_bytes(&mut total, if *value { 4 } else { 5 }, limit)?;
            }
            CanonicalValue::String(value) => {
                add_canonical_bytes(&mut total, encoded_string_len(value)?, limit)?;
            }
            CanonicalValue::Array(values) => {
                add_canonical_bytes(
                    &mut total,
                    2_usize.saturating_add(values.len().saturating_sub(1)),
                    limit,
                )?;
                stack.extend(values.iter().rev());
            }
            CanonicalValue::Object(entries) => {
                let punctuation = 2_usize
                    .checked_add(entries.len())
                    .and_then(|value| value.checked_add(entries.len().saturating_sub(1)))
                    .ok_or(ArtifactStoreReplayError::ReplayLimit {
                        field: "canonical_bytes",
                    })?;
                add_canonical_bytes(&mut total, punctuation, limit)?;
                for (key, value) in entries.iter().rev() {
                    add_canonical_bytes(&mut total, encoded_string_len(key)?, limit)?;
                    stack.push(value);
                }
            }
        }
    }
    Ok(total)
}

fn encoded_string_len(value: &str) -> Result<usize, ArtifactStoreReplayError> {
    let normalized = normalize_nfc(value);
    let mut total = 2_usize;
    for character in normalized.chars() {
        let encoded = match character {
            '"' | '\\' | '\u{8}' | '\t' | '\n' | '\u{c}' | '\r' => 2,
            control if control <= '\u{1f}' => 6,
            other => other.len_utf8(),
        };
        total = total
            .checked_add(encoded)
            .ok_or(ArtifactStoreReplayError::ReplayLimit {
                field: "canonical_bytes",
            })?;
    }
    Ok(total)
}

fn add_canonical_bytes(
    total: &mut usize,
    added: usize,
    limit: Option<usize>,
) -> Result<(), ArtifactStoreReplayError> {
    *total = total
        .checked_add(added)
        .ok_or(ArtifactStoreReplayError::ReplayLimit {
            field: "canonical_bytes",
        })?;
    if limit.is_some_and(|limit| *total > limit) {
        return Err(ArtifactStoreReplayError::ReplayLimit {
            field: "canonical_bytes",
        });
    }
    Ok(())
}

fn checked_add_string_bytes(
    total: &mut usize,
    added: usize,
) -> Result<(), ArtifactStoreReplayError> {
    *total = total
        .checked_add(added)
        .ok_or(ArtifactStoreReplayError::ReplayLimit {
            field: "string_bytes",
        })?;
    Ok(())
}

fn add_string_bytes(
    total: &mut usize,
    added: usize,
    bounds: &ReplayBounds,
) -> Result<(), ArtifactStoreReplayError> {
    checked_add_string_bytes(total, added)?;
    if *total > bounds.string_bytes {
        return Err(ArtifactStoreReplayError::ReplayLimit {
            field: "string_bytes",
        });
    }
    Ok(())
}

fn map_metadata_error(error: &ArtifactStoreAggregateError) -> ArtifactStoreReplayError {
    match error {
        ArtifactStoreAggregateError::Canonicalization => ArtifactStoreReplayError::Canonicalization,
        ArtifactStoreAggregateError::CommandIdReuse
        | ArtifactStoreAggregateError::InvalidCommand
        | ArtifactStoreAggregateError::CorruptState
        | ArtifactStoreAggregateError::CounterExhausted
        | ArtifactStoreAggregateError::QuotaExhausted
        | ArtifactStoreAggregateError::Lifecycle(_) => ArtifactStoreReplayError::InvalidMetadata,
    }
}

fn string(name: &str, value: impl Into<String>) -> (String, CanonicalValue) {
    (name.to_owned(), CanonicalValue::String(value.into()))
}
