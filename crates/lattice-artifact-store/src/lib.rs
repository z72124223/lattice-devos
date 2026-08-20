//! Pure project-scoped artifact semantics and deterministic fake composition.

mod aggregate;
mod history;
mod quota;
mod quota_owner;
mod repository;
mod semantics;
mod snapshot;
mod snapshot_contract;
mod snapshot_parse;
mod snapshot_quota;

pub use aggregate::*;
pub use history::*;
pub use quota::*;
pub use repository::*;
pub use semantics::*;
pub use snapshot::*;

use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::ContentDigest;

/// Absolute byte limit for one artifact object.
pub const HARD_MAX_OBJECT_BYTES: u64 = 1_073_741_824;
/// Absolute canonical-byte limit for one immutable reference manifest.
pub const HARD_MAX_MANIFEST_BYTES: u64 = 65_536;
/// Absolute active-reference count for one object generation.
pub const HARD_MAX_ACTIVE_REFERENCES_PER_OBJECT: u64 = 65_536;
/// Absolute active-read-claim count for one object generation.
pub const HARD_MAX_ACTIVE_READS_PER_OBJECT: u64 = 4_096;
/// Absolute terminal command-record count for one object aggregate.
pub const HARD_MAX_COMMANDS_PER_OBJECT: u64 = 1_000_000;
/// Absolute entry count for one bounded bundle descriptor.
pub const HARD_MAX_BUNDLE_ENTRIES: u64 = 100_000;
/// Absolute logical path depth for one bounded bundle descriptor.
pub const HARD_MAX_BUNDLE_DEPTH: u64 = 64;
/// Absolute UTF-8 byte length for one bounded metadata field.
pub const HARD_MAX_FIELD_BYTES: u64 = 256;
/// Absolute distinct object count attributed to one task.
pub const HARD_MAX_OBJECTS_PER_TASK: u64 = 100_000;
/// Absolute active reference count attributed to one task.
pub const HARD_MAX_REFERENCES_PER_TASK: u64 = 1_000_000;
/// Absolute active read-claim count attributed to one task.
pub const HARD_MAX_READS_PER_TASK: u64 = 65_536;
/// Absolute active referenced byte count attributed to one task.
pub const HARD_MAX_ACTIVE_BYTES_PER_TASK: u64 = 64 * 1_073_741_824;
/// Absolute concurrent staged byte count attributed to one task.
pub const HARD_MAX_STAGING_BYTES_PER_TASK: u64 = 4 * 1_073_741_824;
/// Absolute concurrent staging stream count attributed to one task.
pub const HARD_MAX_STAGING_STREAMS_PER_TASK: u64 = 8;
/// Absolute command record count attributed to one task.
pub const HARD_MAX_COMMANDS_PER_TASK: u64 = 5_000_000;
/// Absolute canonical command-history bytes attributed to one task.
pub const HARD_MAX_HISTORY_BYTES_PER_TASK: u64 = 1_073_741_824;
/// Absolute available object count attributed to one project.
pub const HARD_MAX_OBJECTS_PER_PROJECT: u64 = 1_000_000;
/// Absolute active reference count attributed to one project.
pub const HARD_MAX_REFERENCES_PER_PROJECT: u64 = 10_000_000;
/// Absolute active read-claim count attributed to one project.
pub const HARD_MAX_READS_PER_PROJECT: u64 = 1_000_000;
/// Absolute unique available object bytes attributed to one project.
pub const HARD_MAX_UNIQUE_BYTES_PER_PROJECT: u64 = 1_099_511_627_776;
/// Absolute command record count attributed to one project.
pub const HARD_MAX_COMMANDS_PER_PROJECT: u64 = 100_000_000;
/// Absolute canonical command-history bytes attributed to one project.
pub const HARD_MAX_HISTORY_BYTES_PER_PROJECT: u64 = 64 * 1_073_741_824;
/// Absolute available object count in one Artifact Store.
pub const HARD_MAX_OBJECTS_PER_STORE: u64 = 10_000_000;
/// Absolute active reference count in one Artifact Store.
pub const HARD_MAX_REFERENCES_PER_STORE: u64 = 50_000_000;
/// Absolute active read-claim count in one Artifact Store.
pub const HARD_MAX_READS_PER_STORE: u64 = 5_000_000;
/// Absolute unique available object bytes in one Artifact Store.
pub const HARD_MAX_UNIQUE_BYTES_PER_STORE: u64 = 8 * 1_099_511_627_776;
/// Absolute concurrent staged bytes in one Artifact Store.
pub const HARD_MAX_STAGING_BYTES_PER_STORE: u64 = 16 * 1_073_741_824;
/// Absolute concurrent staging streams in one Artifact Store.
pub const HARD_MAX_STAGING_STREAMS_PER_STORE: u64 = 64;
/// Absolute command records in one Artifact Store.
pub const HARD_MAX_COMMANDS_PER_STORE: u64 = 500_000_000;
/// Absolute canonical command-history bytes in one Artifact Store.
pub const HARD_MAX_HISTORY_BYTES_PER_STORE: u64 = 256 * 1_073_741_824;

/// One configurable hard-bounded Artifact Store limit.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ArtifactLimitKind {
    /// Exact bytes in one object.
    ObjectBytes,
    /// Canonical bytes in one reference manifest.
    ManifestBytes,
    /// UTF-8 bytes in one bounded metadata field.
    FieldBytes,
    /// Active references on one object generation.
    ActiveReferencesPerObject,
    /// Active read claims on one object generation.
    ActiveReadsPerObject,
    /// Terminal command rows on one object aggregate.
    CommandsPerObject,
    /// Entries in one bundle descriptor.
    BundleEntries,
    /// Logical path depth in one bundle descriptor.
    BundleDepth,
    /// Distinct objects attributed to one task.
    ObjectsPerTask,
    /// Active references attributed to one task.
    ReferencesPerTask,
    /// Active reads attributed to one task.
    ReadsPerTask,
    /// Active referenced bytes attributed to one task.
    ActiveBytesPerTask,
    /// Concurrent staged bytes attributed to one task.
    StagingBytesPerTask,
    /// Concurrent staging streams attributed to one task.
    StagingStreamsPerTask,
    /// Command rows attributed to one task.
    CommandsPerTask,
    /// Canonical command-history bytes attributed to one task.
    HistoryBytesPerTask,
    /// Available objects attributed to one project.
    ObjectsPerProject,
    /// Active references attributed to one project.
    ReferencesPerProject,
    /// Active reads attributed to one project.
    ReadsPerProject,
    /// Unique available bytes attributed to one project.
    UniqueBytesPerProject,
    /// Command rows attributed to one project.
    CommandsPerProject,
    /// Canonical command-history bytes attributed to one project.
    HistoryBytesPerProject,
    /// Available objects in one store.
    ObjectsPerStore,
    /// Active references in one store.
    ReferencesPerStore,
    /// Active reads in one store.
    ReadsPerStore,
    /// Unique available bytes in one store.
    UniqueBytesPerStore,
    /// Concurrent staged bytes in one store.
    StagingBytesPerStore,
    /// Concurrent staging streams in one store.
    StagingStreamsPerStore,
    /// Command rows in one store.
    CommandsPerStore,
    /// Canonical command-history bytes in one store.
    HistoryBytesPerStore,
}

impl ArtifactLimitKind {
    /// Complete closed limit set used by the limit-snapshot hash.
    pub const ALL: [Self; 30] = [
        Self::ObjectBytes,
        Self::ManifestBytes,
        Self::FieldBytes,
        Self::ActiveReferencesPerObject,
        Self::ActiveReadsPerObject,
        Self::CommandsPerObject,
        Self::BundleEntries,
        Self::BundleDepth,
        Self::ObjectsPerTask,
        Self::ReferencesPerTask,
        Self::ReadsPerTask,
        Self::ActiveBytesPerTask,
        Self::StagingBytesPerTask,
        Self::StagingStreamsPerTask,
        Self::CommandsPerTask,
        Self::HistoryBytesPerTask,
        Self::ObjectsPerProject,
        Self::ReferencesPerProject,
        Self::ReadsPerProject,
        Self::UniqueBytesPerProject,
        Self::CommandsPerProject,
        Self::HistoryBytesPerProject,
        Self::ObjectsPerStore,
        Self::ReferencesPerStore,
        Self::ReadsPerStore,
        Self::UniqueBytesPerStore,
        Self::StagingBytesPerStore,
        Self::StagingStreamsPerStore,
        Self::CommandsPerStore,
        Self::HistoryBytesPerStore,
    ];

    /// Stable canonical field name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.field()
    }

    const fn field(self) -> &'static str {
        match self {
            Self::ObjectBytes => "max_object_bytes",
            Self::ManifestBytes => "max_manifest_bytes",
            Self::FieldBytes => "max_field_bytes",
            Self::ActiveReferencesPerObject => "max_active_references_per_object",
            Self::ActiveReadsPerObject => "max_active_reads_per_object",
            Self::CommandsPerObject => "max_commands_per_object",
            Self::BundleEntries => "max_bundle_entries",
            Self::BundleDepth => "max_bundle_depth",
            Self::ObjectsPerTask => "max_objects_per_task",
            Self::ReferencesPerTask => "max_references_per_task",
            Self::ReadsPerTask => "max_reads_per_task",
            Self::ActiveBytesPerTask => "max_active_bytes_per_task",
            Self::StagingBytesPerTask => "max_staging_bytes_per_task",
            Self::StagingStreamsPerTask => "max_staging_streams_per_task",
            Self::CommandsPerTask => "max_commands_per_task",
            Self::HistoryBytesPerTask => "max_history_bytes_per_task",
            Self::ObjectsPerProject => "max_objects_per_project",
            Self::ReferencesPerProject => "max_references_per_project",
            Self::ReadsPerProject => "max_reads_per_project",
            Self::UniqueBytesPerProject => "max_unique_bytes_per_project",
            Self::CommandsPerProject => "max_commands_per_project",
            Self::HistoryBytesPerProject => "max_history_bytes_per_project",
            Self::ObjectsPerStore => "max_objects_per_store",
            Self::ReferencesPerStore => "max_references_per_store",
            Self::ReadsPerStore => "max_reads_per_store",
            Self::UniqueBytesPerStore => "max_unique_bytes_per_store",
            Self::StagingBytesPerStore => "max_staging_bytes_per_store",
            Self::StagingStreamsPerStore => "max_staging_streams_per_store",
            Self::CommandsPerStore => "max_commands_per_store",
            Self::HistoryBytesPerStore => "max_history_bytes_per_store",
        }
    }

    const fn hard_maximum(self) -> u64 {
        match self {
            Self::ObjectBytes => HARD_MAX_OBJECT_BYTES,
            Self::ManifestBytes => HARD_MAX_MANIFEST_BYTES,
            Self::FieldBytes => HARD_MAX_FIELD_BYTES,
            Self::ActiveReferencesPerObject => HARD_MAX_ACTIVE_REFERENCES_PER_OBJECT,
            Self::ActiveReadsPerObject => HARD_MAX_ACTIVE_READS_PER_OBJECT,
            Self::CommandsPerObject => HARD_MAX_COMMANDS_PER_OBJECT,
            Self::BundleEntries => HARD_MAX_BUNDLE_ENTRIES,
            Self::BundleDepth => HARD_MAX_BUNDLE_DEPTH,
            Self::ObjectsPerTask => HARD_MAX_OBJECTS_PER_TASK,
            Self::ReferencesPerTask => HARD_MAX_REFERENCES_PER_TASK,
            Self::ReadsPerTask => HARD_MAX_READS_PER_TASK,
            Self::ActiveBytesPerTask => HARD_MAX_ACTIVE_BYTES_PER_TASK,
            Self::StagingBytesPerTask => HARD_MAX_STAGING_BYTES_PER_TASK,
            Self::StagingStreamsPerTask => HARD_MAX_STAGING_STREAMS_PER_TASK,
            Self::CommandsPerTask => HARD_MAX_COMMANDS_PER_TASK,
            Self::HistoryBytesPerTask => HARD_MAX_HISTORY_BYTES_PER_TASK,
            Self::ObjectsPerProject => HARD_MAX_OBJECTS_PER_PROJECT,
            Self::ReferencesPerProject => HARD_MAX_REFERENCES_PER_PROJECT,
            Self::ReadsPerProject => HARD_MAX_READS_PER_PROJECT,
            Self::UniqueBytesPerProject => HARD_MAX_UNIQUE_BYTES_PER_PROJECT,
            Self::CommandsPerProject => HARD_MAX_COMMANDS_PER_PROJECT,
            Self::HistoryBytesPerProject => HARD_MAX_HISTORY_BYTES_PER_PROJECT,
            Self::ObjectsPerStore => HARD_MAX_OBJECTS_PER_STORE,
            Self::ReferencesPerStore => HARD_MAX_REFERENCES_PER_STORE,
            Self::ReadsPerStore => HARD_MAX_READS_PER_STORE,
            Self::UniqueBytesPerStore => HARD_MAX_UNIQUE_BYTES_PER_STORE,
            Self::StagingBytesPerStore => HARD_MAX_STAGING_BYTES_PER_STORE,
            Self::StagingStreamsPerStore => HARD_MAX_STAGING_STREAMS_PER_STORE,
            Self::CommandsPerStore => HARD_MAX_COMMANDS_PER_STORE,
            Self::HistoryBytesPerStore => HARD_MAX_HISTORY_BYTES_PER_STORE,
        }
    }

    /// Returns the stable array index for the closed 1.0 limit set.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::ObjectBytes => 0,
            Self::ManifestBytes => 1,
            Self::FieldBytes => 2,
            Self::ActiveReferencesPerObject => 3,
            Self::ActiveReadsPerObject => 4,
            Self::CommandsPerObject => 5,
            Self::BundleEntries => 6,
            Self::BundleDepth => 7,
            Self::ObjectsPerTask => 8,
            Self::ReferencesPerTask => 9,
            Self::ReadsPerTask => 10,
            Self::ActiveBytesPerTask => 11,
            Self::StagingBytesPerTask => 12,
            Self::StagingStreamsPerTask => 13,
            Self::CommandsPerTask => 14,
            Self::HistoryBytesPerTask => 15,
            Self::ObjectsPerProject => 16,
            Self::ReferencesPerProject => 17,
            Self::ReadsPerProject => 18,
            Self::UniqueBytesPerProject => 19,
            Self::CommandsPerProject => 20,
            Self::HistoryBytesPerProject => 21,
            Self::ObjectsPerStore => 22,
            Self::ReferencesPerStore => 23,
            Self::ReadsPerStore => 24,
            Self::UniqueBytesPerStore => 25,
            Self::StagingBytesPerStore => 26,
            Self::StagingStreamsPerStore => 27,
            Self::CommandsPerStore => 28,
            Self::HistoryBytesPerStore => 29,
        }
    }
}

/// Invalid Artifact Store configuration or semantic input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactStoreError {
    /// A configured limit is zero or exceeds its constitutional hard maximum.
    InvalidLimit {
        /// Stable limit field name.
        field: &'static str,
    },
    /// An internal canonical value could not be framed or hashed.
    Canonicalization,
    /// An internally produced SHA-256 hex value violated the shared contract.
    InvalidDigest,
}

impl ArtifactStoreError {
    /// Stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidLimit { .. } => "ARTIFACT_INVALID_LIMIT",
            Self::Canonicalization => "ARTIFACT_CANONICALIZATION_FAILED",
            Self::InvalidDigest => "ARTIFACT_INVALID_DIGEST",
        }
    }
}

impl fmt::Display for ArtifactStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit { field } => {
                write!(formatter, "{field} must be within its hard limit")
            }
            Self::Canonicalization => formatter.write_str("artifact canonicalization failed"),
            Self::InvalidDigest => formatter.write_str("artifact hash produced an invalid digest"),
        }
    }
}

impl Error for ArtifactStoreError {}

/// Immutable lower-or-equal limits used by one Artifact Store composition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactStoreLimits {
    values: [u64; ArtifactLimitKind::ALL.len()],
}

impl ArtifactStoreLimits {
    /// Returns the constitutional hard maximums.
    #[must_use]
    pub const fn hard_maximums() -> Self {
        Self {
            values: [
                HARD_MAX_OBJECT_BYTES,
                HARD_MAX_MANIFEST_BYTES,
                HARD_MAX_FIELD_BYTES,
                HARD_MAX_ACTIVE_REFERENCES_PER_OBJECT,
                HARD_MAX_ACTIVE_READS_PER_OBJECT,
                HARD_MAX_COMMANDS_PER_OBJECT,
                HARD_MAX_BUNDLE_ENTRIES,
                HARD_MAX_BUNDLE_DEPTH,
                HARD_MAX_OBJECTS_PER_TASK,
                HARD_MAX_REFERENCES_PER_TASK,
                HARD_MAX_READS_PER_TASK,
                HARD_MAX_ACTIVE_BYTES_PER_TASK,
                HARD_MAX_STAGING_BYTES_PER_TASK,
                HARD_MAX_STAGING_STREAMS_PER_TASK,
                HARD_MAX_COMMANDS_PER_TASK,
                HARD_MAX_HISTORY_BYTES_PER_TASK,
                HARD_MAX_OBJECTS_PER_PROJECT,
                HARD_MAX_REFERENCES_PER_PROJECT,
                HARD_MAX_READS_PER_PROJECT,
                HARD_MAX_UNIQUE_BYTES_PER_PROJECT,
                HARD_MAX_COMMANDS_PER_PROJECT,
                HARD_MAX_HISTORY_BYTES_PER_PROJECT,
                HARD_MAX_OBJECTS_PER_STORE,
                HARD_MAX_REFERENCES_PER_STORE,
                HARD_MAX_READS_PER_STORE,
                HARD_MAX_UNIQUE_BYTES_PER_STORE,
                HARD_MAX_STAGING_BYTES_PER_STORE,
                HARD_MAX_STAGING_STREAMS_PER_STORE,
                HARD_MAX_COMMANDS_PER_STORE,
                HARD_MAX_HISTORY_BYTES_PER_STORE,
            ],
        }
    }

    /// Constructs a limit snapshot that can only tighten hard maximums.
    ///
    /// # Errors
    ///
    /// Rejects zero or any value above its constitutional hard maximum.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_object_bytes: u64,
        max_manifest_bytes: u64,
        max_active_references_per_object: u64,
        max_active_reads_per_object: u64,
        max_commands_per_object: u64,
        max_bundle_entries: u64,
        max_bundle_depth: u64,
    ) -> Result<Self, ArtifactStoreError> {
        validate_limit(max_object_bytes, HARD_MAX_OBJECT_BYTES, "max_object_bytes")?;
        validate_limit(
            max_manifest_bytes,
            HARD_MAX_MANIFEST_BYTES,
            "max_manifest_bytes",
        )?;
        validate_limit(
            max_active_references_per_object,
            HARD_MAX_ACTIVE_REFERENCES_PER_OBJECT,
            "max_active_references_per_object",
        )?;
        validate_limit(
            max_active_reads_per_object,
            HARD_MAX_ACTIVE_READS_PER_OBJECT,
            "max_active_reads_per_object",
        )?;
        validate_limit(
            max_commands_per_object,
            HARD_MAX_COMMANDS_PER_OBJECT,
            "max_commands_per_object",
        )?;
        validate_limit(
            max_bundle_entries,
            HARD_MAX_BUNDLE_ENTRIES,
            "max_bundle_entries",
        )?;
        validate_limit(max_bundle_depth, HARD_MAX_BUNDLE_DEPTH, "max_bundle_depth")?;
        let mut limits = Self::hard_maximums();
        limits.values[ArtifactLimitKind::ObjectBytes.index()] = max_object_bytes;
        limits.values[ArtifactLimitKind::ManifestBytes.index()] = max_manifest_bytes;
        limits.values[ArtifactLimitKind::ActiveReferencesPerObject.index()] =
            max_active_references_per_object;
        limits.values[ArtifactLimitKind::ActiveReadsPerObject.index()] =
            max_active_reads_per_object;
        limits.values[ArtifactLimitKind::CommandsPerObject.index()] = max_commands_per_object;
        limits.values[ArtifactLimitKind::BundleEntries.index()] = max_bundle_entries;
        limits.values[ArtifactLimitKind::BundleDepth.index()] = max_bundle_depth;
        Ok(limits)
    }

    /// Maximum exact bytes accepted for one object.
    #[must_use]
    pub const fn max_object_bytes(self) -> u64 {
        self.get(ArtifactLimitKind::ObjectBytes)
    }

    /// Maximum canonical bytes accepted for one reference manifest.
    #[must_use]
    pub const fn max_manifest_bytes(self) -> u64 {
        self.get(ArtifactLimitKind::ManifestBytes)
    }

    /// Maximum active references on one object generation.
    #[must_use]
    pub const fn max_active_references_per_object(self) -> u64 {
        self.get(ArtifactLimitKind::ActiveReferencesPerObject)
    }

    /// Maximum active read claims on one object generation.
    #[must_use]
    pub const fn max_active_reads_per_object(self) -> u64 {
        self.get(ArtifactLimitKind::ActiveReadsPerObject)
    }

    /// Maximum terminal command records in one object aggregate.
    #[must_use]
    pub const fn max_commands_per_object(self) -> u64 {
        self.get(ArtifactLimitKind::CommandsPerObject)
    }

    /// Maximum bundle entries described by one reference.
    #[must_use]
    pub const fn max_bundle_entries(self) -> u64 {
        self.get(ArtifactLimitKind::BundleEntries)
    }

    /// Maximum bundle logical path depth.
    #[must_use]
    pub const fn max_bundle_depth(self) -> u64 {
        self.get(ArtifactLimitKind::BundleDepth)
    }

    /// Returns one configured limit from the complete immutable snapshot.
    #[must_use]
    pub const fn get(self, kind: ArtifactLimitKind) -> u64 {
        self.values[kind.index()]
    }

    /// Returns a copy with one lower-or-equal limit.
    ///
    /// # Errors
    ///
    /// Rejects zero or a value above the constitutional hard maximum.
    pub fn tighten(
        mut self,
        kind: ArtifactLimitKind,
        value: u64,
    ) -> Result<Self, ArtifactStoreError> {
        validate_limit(value, kind.hard_maximum(), kind.field())?;
        if value > self.get(kind) {
            return Err(ArtifactStoreError::InvalidLimit {
                field: kind.field(),
            });
        }
        self.values[kind.index()] = value;
        Ok(self)
    }

    /// Hashes the complete immutable limit snapshot under its own domain.
    ///
    /// # Errors
    ///
    /// Returns a typed internal error if canonical framing or digest
    /// construction fails.
    pub fn limit_snapshot_digest(self) -> Result<ContentDigest, ArtifactStoreError> {
        let fields = ArtifactLimitKind::ALL
            .into_iter()
            .map(|kind| {
                (
                    kind.as_str().to_owned(),
                    CanonicalValue::String(self.get(kind).to_string()),
                )
            })
            .collect();
        let domain = HashDomain::new("lattice.artifact.limit-snapshot", "1.0")
            .map_err(|_| ArtifactStoreError::Canonicalization)?;
        let digest = canonical_sha256(&domain, &CanonicalValue::Object(fields))
            .map_err(|_| ArtifactStoreError::Canonicalization)?;
        ContentDigest::from_sha256(digest.to_hex()).map_err(|_| ArtifactStoreError::InvalidDigest)
    }
}

fn validate_limit(
    value: u64,
    hard_maximum: u64,
    field: &'static str,
) -> Result<(), ArtifactStoreError> {
    if value == 0 || value > hard_maximum {
        Err(ArtifactStoreError::InvalidLimit { field })
    } else {
        Ok(())
    }
}
