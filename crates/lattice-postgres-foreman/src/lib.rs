//! Same-database append-only persistence for managed foreman child evidence.

use std::error::Error;
use std::fmt;

use lattice_contracts::ContentDigest;
use sha2::{Digest, Sha256};

mod adapter;
mod setup;

pub use adapter::{
    ActiveTaskRef, ActiveTaskRestartKind, AdapterDatabaseStage, AdapterError, AdapterErrorKind,
    AppendDisposition, AttemptClosure, ClaimDisposition, ClaimOutcome, ClaimReservationDisposition,
    CredentialAuthorityKind, ExecutionEnvironmentDescriptor, ExecutionEnvironmentKind,
    ExecutionEnvironmentRef, ExecutionFileIdentity, ExecutionProcessFenceKind,
    ExecutionToolIdentity, ManagedPreparationObservation, ManagedPreparationObservationKind,
    ManagedPromotionIntent, ManagedPromotionSource, NATIVE_WINDOWS_EXECUTION_ENVIRONMENT_REF,
    PendingWorkerAttempt, PersistedApprovalReferenceLink, PersistedArtifactReferenceLink,
    PersistedExecutionEnvironment, PersistedReferenceLinks, PersistedTaskRuntimeRows,
    PostgresForeman, ProviderDispatchClaim, ProviderDispatchKind, ReplayRecord, ReplayRecordState,
    RestartTaskCursor, RestartTaskKind, RestartTaskRef, StagedArtifactReference, TaskReplay,
};
pub use lattice_approval_verifier::{
    ExecutionAuthoritySource, ExecutionCapability, UntrustedExecutionAuthority,
    VerifiedExecutionAuthority,
};
pub use lattice_artifact_store::{
    ManagedEvidenceKind, UntrustedManagedEvidence, VerifiedManagedEvidence,
};
pub use lattice_foreman_state::{ExternalCostBudget, WorkerBudget};
pub use lattice_task_ledger::{
    ModelReason, ReasoningEffort, TaskRuntimeEventLink, UntrustedTaskExecutionBinding,
    UntrustedTaskVerificationRow, UntrustedWorkerAttemptRow, UntrustedWorkerObservationRow,
    VerificationOutcome, VerifiedTaskExecutionBinding, VerifiedTaskVerificationRecord,
    VerifiedWorkerAttemptRecord, VerifiedWorkerObservationRecord, WorkerModel,
    WorkerObservationKind,
};
pub use setup::{
    ExtensionApplyOutcome, ExtensionCatalogEvidence, ExtensionDatabaseRole, ExtensionSetupError,
    ExtensionSetupErrorKind, ExtensionTarget, apply_extension, verify_extension,
};

/// Fixed extension producer identity.
pub const FOREMAN_EXTENSION_ID: &str = "lattice-postgres-foreman";
/// Sole extension schema profile.
pub const FOREMAN_EXTENSION_SCHEMA_VERSION: u16 = 1;
/// Repository-relative embedded profile path.
pub const FOREMAN_EXTENSION_PATH: &str = "db/extensions/foreman-execution/v1.sql";
/// Exact compatible global Store schema.
pub const REQUIRED_GLOBAL_SCHEMA_VERSION: u16 = 7;
/// Exact compatible global Store-v7 manifest.
pub const REQUIRED_GLOBAL_MANIFEST_SHA256: &str =
    "584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8";
/// Closed global active-attempt capacity.
pub const MAX_GLOBAL_ACTIVE_ATTEMPTS: u8 = 4;
/// Closed per-task active-attempt capacity.
pub const MAX_TASK_ACTIVE_ATTEMPTS: u8 = 1;
/// One initial attempt plus two repair attempts.
pub const MAX_EXTENSION_ATTEMPTS: u8 = 3;
/// Maximum retained evidence objects for one worker attempt.
pub const MAX_ARTIFACTS_PER_ATTEMPT: u16 = 64;
/// Maximum retained evidence bytes for one worker attempt.
pub const MAX_ARTIFACT_BYTES_PER_ATTEMPT: u64 = 8_388_608;
/// Maximum retained evidence objects for one task across three attempts.
pub const MAX_ARTIFACTS_PER_TASK: u16 = 192;
/// Maximum retained evidence bytes for one task across three attempts.
pub const MAX_ARTIFACT_BYTES_PER_TASK: u64 = 25_165_824;
/// Bounded maximum for one restart-discovery query.
pub const MAX_ACTIVE_TASK_REPLAY_ROWS: u16 = 256;

const EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/foreman-execution/v1.sql");
const EXPECTED_EXTENSION_SQL_BYTES: usize = 349_546;
const EXPECTED_EXTENSION_SQL_SHA256: &str =
    "46e186d54b65fbd55f7d5f48c693707287e0d723bd10c3077412d484c19ead6e";
const EXPECTED_EXTENSION_MANIFEST_SHA256: &str =
    "2a487f0f32c45542d0ee02a37881f55466ca892f530967d95f661a27594279dd";
const EXTENSION_MANIFEST_DOMAIN: &[u8] = b"LATTICE_POSTGRES_FOREMAN_EXTENSION_MANIFEST_V1\0";

/// Frozen embedded-profile verification failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionManifestError {
    SqlMismatch,
    ManifestMismatch,
    Contract,
}

impl fmt::Display for ExtensionManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SqlMismatch => "embedded foreman extension SQL mismatch",
            Self::ManifestMismatch => "foreman extension manifest mismatch",
            Self::Contract => "foreman extension digest rejected",
        })
    }
}

impl Error for ExtensionManifestError {}

/// Verified identity and exact bytes of extension v1.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionManifestEvidence {
    sql_sha256: ContentDigest,
    manifest_sha256: ContentDigest,
}

impl ExtensionManifestEvidence {
    #[must_use]
    pub const fn extension_id(&self) -> &'static str {
        FOREMAN_EXTENSION_ID
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        FOREMAN_EXTENSION_SCHEMA_VERSION
    }

    #[must_use]
    pub const fn path(&self) -> &'static str {
        FOREMAN_EXTENSION_PATH
    }

    #[must_use]
    pub const fn bytes(&self) -> &'static [u8] {
        EXTENSION_SQL
    }

    #[must_use]
    pub const fn byte_length(&self) -> usize {
        EXTENSION_SQL.len()
    }

    #[must_use]
    pub const fn sql_sha256(&self) -> &ContentDigest {
        &self.sql_sha256
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> &ContentDigest {
        &self.manifest_sha256
    }
}

/// Verifies exact embedded SQL bytes and their complete Store-v7-bound profile.
///
/// # Errors
///
/// Fails closed on any byte, checksum, profile, or shared-contract drift.
pub fn verify_embedded_extension() -> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    let sql_sha256 = sha256_hex(EXTENSION_SQL);
    if EXTENSION_SQL.len() != EXPECTED_EXTENSION_SQL_BYTES
        || sql_sha256 != EXPECTED_EXTENSION_SQL_SHA256
    {
        return Err(ExtensionManifestError::SqlMismatch);
    }
    let manifest_sha256 = extension_manifest_sha256(&sql_sha256);
    if manifest_sha256 != EXPECTED_EXTENSION_MANIFEST_SHA256 {
        return Err(ExtensionManifestError::ManifestMismatch);
    }
    Ok(ExtensionManifestEvidence {
        sql_sha256: ContentDigest::from_sha256(sql_sha256)
            .map_err(|_| ExtensionManifestError::Contract)?,
        manifest_sha256: ContentDigest::from_sha256(manifest_sha256)
            .map_err(|_| ExtensionManifestError::Contract)?,
    })
}

fn extension_manifest_sha256(sql_sha256: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(EXTENSION_MANIFEST_DOMAIN);
    let extension_version = FOREMAN_EXTENSION_SCHEMA_VERSION.to_string();
    let extension_bytes = EXTENSION_SQL.len().to_string();
    let global_version = REQUIRED_GLOBAL_SCHEMA_VERSION.to_string();
    for value in [
        FOREMAN_EXTENSION_ID,
        &extension_version,
        FOREMAN_EXTENSION_PATH,
        &extension_bytes,
        sql_sha256,
        &global_version,
        REQUIRED_GLOBAL_MANIFEST_SHA256,
    ] {
        update_framed(&mut hasher, value.as_bytes());
    }
    bytes_to_hex(&hasher.finalize())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    bytes_to_hex(&hasher.finalize())
}

fn update_framed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u64::try_from(value.len())
            .expect("bounded profile field")
            .to_be_bytes(),
    );
    hasher.update(value);
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "prints a replacement only while freezing reviewed SQL bytes"]
    fn measure_manifest_digest() {
        println!("{}", extension_manifest_sha256(&sha256_hex(EXTENSION_SQL)));
    }
}
