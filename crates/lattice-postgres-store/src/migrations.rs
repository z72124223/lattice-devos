//! Exact-manifest `PostgreSQL` schema foundation.

use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::ops::RangeInclusive;
use std::path::Path;

use sha2::{Digest, Sha256};

/// Exact synchronous `PostgreSQL` driver version approved for Store 1.3.
pub const POSTGRES_DRIVER_VERSION: &str = "0.19.14";
/// Only `PostgreSQL` server major accepted by the Store 1.3 verifier.
pub const SUPPORTED_POSTGRES_MAJOR: u32 = 17;
/// Current global database schema contract version.
pub const POSTGRES_SCHEMA_VERSION: u16 = 8;
/// Immutable physical schema profile retained by every Store v2 receipt.
pub const STORE_V2_SCHEMA_VERSION: u16 = 2;
/// Immutable first-three-entry manifest retained by every Store v2 receipt.
pub const STORE_V2_MANIFEST_SHA256: &str =
    "4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129";

const BOOTSTRAP_BYTES: &[u8] = include_bytes!("../../../db/migrations/0001_bootstrap.sql");
const FOUNDATION_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0002_control_store_foundation.sql");
const LIVE_CONTROL_STORE_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0003_live_control_store.sql");
const TASK_LEDGER_REPOSITORY_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0004_task_ledger_repository.sql");
const PROJECT_REGISTRY_REPOSITORY_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0005_project_registry_repository.sql");
const TASK_AUTONOMY_RECEIPT_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0006_task_autonomy_receipt.sql");
const FOREMAN_COORDINATION_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0007_foreman_coordination.sql");
const TASK_SUBMISSION_ENVELOPE_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0008_task_submission_envelope.sql");
const EXTERNAL_VERIFIED_RESULT_ADOPTION_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0009_external_verified_result_adoption.sql");
const STORE_V8_RUNTIME_SUCCESSOR_BYTES: &[u8] =
    include_bytes!("../../../db/migrations/0010_store_v8_runtime_successor.sql");
const BOOTSTRAP_SHA256: &str = "7bff021fc17f738551309c906578c8015b2dd0307d27d239c21df1697c4d09c8";
const FOUNDATION_SHA256: &str = "e996dc64af3112a647e75ebf07df2a77b1e9b3a018ed443880150365184883f0";
const LIVE_CONTROL_STORE_SHA256: &str =
    "00ae3eedd76704f26b1df58955d9d594c98f0ba525be93b15d8c9ebb1f2115c1";
const TASK_LEDGER_REPOSITORY_SHA256: &str =
    "cd658ed2f4624cd0a829c818c1cf96d8ac3829264046976bdde3b2fc7feea6e5";
const PROJECT_REGISTRY_REPOSITORY_SHA256: &str =
    "b7af1f8a8ac370bbfc8a5312497461587cb8a86eb32ff97e5b865c7ae9bf0dcf";
const TASK_AUTONOMY_RECEIPT_SHA256: &str =
    "c50f2a51380950b5f6c757b736b35b550d903319a46d2bcd9319938e02106a61";
const FOREMAN_COORDINATION_SHA256: &str =
    "33a4e1c3ab8f29f763123ffe46c2929025a7a7256614f5c92011a1140c8300ad";
pub(crate) const CURRENT_V6_MANIFEST_SHA256: &str =
    "75189dea7cd2cb95b694bade467c2b5c40373436fb1b3d48e9017b50a9d206ae";
const TASK_SUBMISSION_ENVELOPE_SHA256: &str =
    "a9059c74722dcbff5345a2732bf1c44f8f2dd682a5eecb57bda2f0d820e9d4a0";
pub(crate) const CURRENT_V7_MANIFEST_SHA256: &str =
    "584a446464ab2f7ebd8b85543ba36a6d52b0a708502c39d2653b8814d84313f8";
const EXTERNAL_VERIFIED_RESULT_ADOPTION_SHA256: &str =
    "587aaff568e4a058055c608ad80aa3a598288bba8cb91905dc9978f7de4f8319";
const LEGACY_V8_MANIFEST_SHA256: &str =
    "01373ed5092e90bf6a9e383955cd70d0fd4e0ed821667f1905b69e313005ea82";
const STORE_V8_RUNTIME_SUCCESSOR_SHA256: &str =
    "0cc671a575879eaf28cbb4af31e449290b40a13ee87bd5ec037c5d340238153b";
pub(crate) const CURRENT_V8_MANIFEST_SHA256: &str =
    "2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60";
pub(crate) const CURRENT_V5_MANIFEST_SHA256: &str =
    "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
pub(crate) const REGISTRY_V4_MANIFEST_SHA256: &str =
    "df3f7ca3687afaa0d1f676158725e6d2f06670e0612df7482aa9d4d244b59f0f";
pub(crate) const LEGACY_V1_MANIFEST_SHA256: &str =
    "9b126a41e542b71d434b5786e35acb66575967d055a6733b9d6bf0b8c9f0eada";
pub(crate) const MANIFEST_HASH_DOMAIN: &[u8] = b"LATTICE_POSTGRES_MIGRATION_MANIFEST_V1\0";
const DATABASE_IDENTITY_DOMAIN: &[u8] = b"LATTICE_POSTGRES_DATABASE_IDENTITY_V1\0";

/// Closed manifest status; superseded bytes remain evidence but never execute.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MigrationStatus {
    /// Historical reviewed bytes retained as non-executable evidence.
    Superseded,
    /// SQL executed only by the explicit administrative runner.
    Executable,
}

impl MigrationStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Superseded => "SUPERSEDED",
            Self::Executable => "EXECUTABLE",
        }
    }
}

/// Closed transaction behavior for one manifest entry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MigrationTransactionMode {
    /// The entry is evidence only and must never execute.
    NotExecuted,
    /// The runner owns the surrounding `PostgreSQL` transaction.
    RunnerOwned,
}

impl MigrationTransactionMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotExecuted => "NOT_EXECUTED",
            Self::RunnerOwned => "RUNNER_OWNED",
        }
    }
}

/// One immutable compile-time migration manifest entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigrationDescriptor {
    ordinal: u16,
    id: &'static str,
    path: &'static str,
    bytes: &'static [u8],
    byte_length: usize,
    sha256: &'static str,
    status: MigrationStatus,
    transaction_mode: MigrationTransactionMode,
    schema_version: u16,
    min_reader: u16,
    max_reader: u16,
    min_writer: u16,
    max_writer: u16,
}

impl MigrationDescriptor {
    #[must_use]
    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    #[must_use]
    pub const fn bytes(&self) -> &'static [u8] {
        self.bytes
    }

    #[must_use]
    pub const fn byte_length(&self) -> usize {
        self.byte_length
    }

    #[must_use]
    pub const fn sha256(&self) -> &'static str {
        self.sha256
    }

    #[must_use]
    pub const fn status(&self) -> MigrationStatus {
        self.status
    }

    #[must_use]
    pub const fn transaction_mode(&self) -> MigrationTransactionMode {
        self.transaction_mode
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn reader_compatibility(&self) -> RangeInclusive<u16> {
        self.min_reader..=self.max_reader
    }

    #[must_use]
    pub const fn writer_compatibility(&self) -> RangeInclusive<u16> {
        self.min_writer..=self.max_writer
    }
}

pub(crate) struct MigrationMetadata<'a> {
    pub(crate) ordinal: u16,
    pub(crate) id: &'a str,
    pub(crate) path: &'a str,
    pub(crate) byte_length: u64,
    pub(crate) sha256: &'a str,
    pub(crate) status: MigrationStatus,
    pub(crate) transaction_mode: MigrationTransactionMode,
    pub(crate) schema_version: u16,
    pub(crate) min_reader: u16,
    pub(crate) max_reader: u16,
    pub(crate) min_writer: u16,
    pub(crate) max_writer: u16,
}

static MIGRATION_MANIFEST: [MigrationDescriptor; 10] = [
    MigrationDescriptor {
        ordinal: 1,
        id: "0001_bootstrap_draft",
        path: "db/migrations/0001_bootstrap.sql",
        bytes: BOOTSTRAP_BYTES,
        byte_length: 312,
        sha256: BOOTSTRAP_SHA256,
        status: MigrationStatus::Superseded,
        transaction_mode: MigrationTransactionMode::NotExecuted,
        schema_version: 0,
        min_reader: 0,
        max_reader: 0,
        min_writer: 0,
        max_writer: 0,
    },
    MigrationDescriptor {
        ordinal: 2,
        id: "0002_control_store_foundation",
        path: "db/migrations/0002_control_store_foundation.sql",
        bytes: FOUNDATION_BYTES,
        byte_length: 14_259,
        sha256: FOUNDATION_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: 1,
        min_reader: 1,
        max_reader: 1,
        min_writer: 1,
        max_writer: 1,
    },
    MigrationDescriptor {
        ordinal: 3,
        id: "0003_live_control_store",
        path: "db/migrations/0003_live_control_store.sql",
        bytes: LIVE_CONTROL_STORE_BYTES,
        byte_length: 29_518,
        sha256: LIVE_CONTROL_STORE_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: STORE_V2_SCHEMA_VERSION,
        min_reader: 2,
        max_reader: 2,
        min_writer: 2,
        max_writer: 2,
    },
    MigrationDescriptor {
        ordinal: 4,
        id: "0004_task_ledger_repository",
        path: "db/migrations/0004_task_ledger_repository.sql",
        bytes: TASK_LEDGER_REPOSITORY_BYTES,
        byte_length: 111_742,
        sha256: TASK_LEDGER_REPOSITORY_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: 3,
        min_reader: 3,
        max_reader: 3,
        min_writer: 3,
        max_writer: 3,
    },
    MigrationDescriptor {
        ordinal: 5,
        id: "0005_project_registry_repository",
        path: "db/migrations/0005_project_registry_repository.sql",
        bytes: PROJECT_REGISTRY_REPOSITORY_BYTES,
        byte_length: 200_547,
        sha256: PROJECT_REGISTRY_REPOSITORY_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: 4,
        min_reader: 4,
        max_reader: 4,
        min_writer: 4,
        max_writer: 4,
    },
    MigrationDescriptor {
        ordinal: 6,
        id: "0006_task_autonomy_receipt",
        path: "db/migrations/0006_task_autonomy_receipt.sql",
        bytes: TASK_AUTONOMY_RECEIPT_BYTES,
        byte_length: 207_133,
        sha256: TASK_AUTONOMY_RECEIPT_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: 5,
        min_reader: 5,
        max_reader: 5,
        min_writer: 5,
        max_writer: 5,
    },
    MigrationDescriptor {
        ordinal: 7,
        id: "0007_foreman_coordination",
        path: "db/migrations/0007_foreman_coordination.sql",
        bytes: FOREMAN_COORDINATION_BYTES,
        byte_length: 217_170,
        sha256: FOREMAN_COORDINATION_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: 6,
        min_reader: 6,
        max_reader: 6,
        min_writer: 6,
        max_writer: 6,
    },
    MigrationDescriptor {
        ordinal: 8,
        id: "0008_task_submission_envelope",
        path: "db/migrations/0008_task_submission_envelope.sql",
        bytes: TASK_SUBMISSION_ENVELOPE_BYTES,
        byte_length: 334_756,
        sha256: TASK_SUBMISSION_ENVELOPE_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: 7,
        min_reader: 7,
        max_reader: 7,
        min_writer: 7,
        max_writer: 7,
    },
    MigrationDescriptor {
        ordinal: 9,
        id: "0009_external_verified_result_adoption",
        path: "db/migrations/0009_external_verified_result_adoption.sql",
        bytes: EXTERNAL_VERIFIED_RESULT_ADOPTION_BYTES,
        byte_length: 12_438,
        sha256: EXTERNAL_VERIFIED_RESULT_ADOPTION_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: POSTGRES_SCHEMA_VERSION,
        min_reader: 8,
        max_reader: 8,
        min_writer: 8,
        max_writer: 8,
    },
    MigrationDescriptor {
        ordinal: 10,
        id: "0010_store_v8_runtime_successor",
        path: "db/migrations/0010_store_v8_runtime_successor.sql",
        bytes: STORE_V8_RUNTIME_SUCCESSOR_BYTES,
        byte_length: 195_721,
        sha256: STORE_V8_RUNTIME_SUCCESSOR_SHA256,
        status: MigrationStatus::Executable,
        transaction_mode: MigrationTransactionMode::RunnerOwned,
        schema_version: POSTGRES_SCHEMA_VERSION,
        min_reader: 8,
        max_reader: 8,
        min_writer: 8,
        max_writer: 8,
    },
];

/// Returns the complete compile-time manifest. No directory is inspected.
#[must_use]
pub const fn migration_manifest() -> &'static [MigrationDescriptor] {
    &MIGRATION_MANIFEST
}

/// Closed least-privilege database roles expected by Store 1.3.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DatabaseRole {
    Migrator,
    Runtime,
    Guardian,
    ReadOnly,
}

impl DatabaseRole {
    pub const ALL: [Self; 4] = [
        Self::Migrator,
        Self::Runtime,
        Self::Guardian,
        Self::ReadOnly,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Migrator => "lattice_migrator",
            Self::Runtime => "lattice_runtime",
            Self::Guardian => "lattice_guardian",
            Self::ReadOnly => "lattice_readonly",
        }
    }

    /// Fixed externally provisioned LOGIN principal for this capability role.
    #[must_use]
    pub const fn login_role(self) -> &'static str {
        match self {
            Self::Migrator => "lattice_migrator_login",
            Self::Runtime => "lattice_runtime_login",
            Self::Guardian => "lattice_guardian_login",
            Self::ReadOnly => "lattice_readonly_login",
        }
    }
}

/// Exact disposable database target and pre-provisioned run sentinel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationTarget {
    database_name: String,
    run_id: String,
    expected_database_uuid: String,
    expected_database_identity_sha256: Sha256Hex,
}

impl MigrationTarget {
    /// Constructs one exact non-default marker-owned disposable database target.
    ///
    /// # Errors
    ///
    /// Rejects default, malformed, uppercase, oversized, or unscoped names and
    /// any run identity other than 32 lowercase hexadecimal bytes.
    pub fn new(
        database_name: impl Into<String>,
        run_id: impl Into<String>,
    ) -> Result<Self, PostgresStoreSetupError> {
        let database_name = database_name.into();
        let run_id = run_id.into();
        let suffix = database_name.strip_prefix("lattice_task019_");
        let valid_database = database_name.len() <= 63
            && suffix.is_some_and(|value| {
                !value.is_empty()
                    && value.len() <= 32
                    && value.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            });
        let valid_run = run_id.len() == 32
            && run_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid_database || !valid_run {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::TargetMismatch,
            ));
        }
        let (expected_database_uuid, expected_database_identity_sha256) =
            derive_database_identity(&database_name, &run_id);
        Ok(Self {
            database_name,
            run_id,
            expected_database_uuid,
            expected_database_identity_sha256,
        })
    }

    #[must_use]
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    #[must_use]
    pub fn database_comment(&self) -> String {
        format!("LATTICE_DEVOS_DISPOSABLE_V1:{}", self.run_id)
    }

    /// Immutable database identity derived from the exact target and run marker.
    #[must_use]
    pub fn expected_database_uuid(&self) -> &str {
        &self.expected_database_uuid
    }

    /// Complete domain-separated target identity commitment retained by live receipts.
    #[must_use]
    pub const fn expected_database_identity_sha256(&self) -> &Sha256Hex {
        &self.expected_database_identity_sha256
    }
}

fn derive_database_identity(database_name: &str, run_id: &str) -> (String, Sha256Hex) {
    let mut hasher = Sha256::new();
    hasher.update(DATABASE_IDENTITY_DOMAIN);
    hasher.update(
        u64::try_from(database_name.len())
            .expect("validated database name length fits u64")
            .to_be_bytes(),
    );
    hasher.update(database_name.as_bytes());
    hasher.update(
        u64::try_from(run_id.len())
            .expect("validated run id length fits u64")
            .to_be_bytes(),
    );
    hasher.update(run_id.as_bytes());
    let digest = hasher.finalize();
    let identity_sha256 = Sha256Hex::from_digest(&digest);
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    (format_uuid(bytes), identity_sha256)
}

fn format_uuid(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(36);
    for (index, byte) in bytes.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            output.push('-');
        }
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

/// Stable fail-closed `PostgreSQL` setup and compatibility failure categories.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PostgresStoreSetupErrorKind {
    ManifestInvalid,
    ChecksumMismatch,
    TargetMismatch,
    TargetUnowned,
    SchemaCollision,
    HistoryMismatch,
    CompatibilityMismatch,
    PermissionDenied,
    ServerUnsupported,
    UnsupportedFutureSchema,
    UnsafeSetting,
    NetworkBoundary,
    TransactionFailed,
    CommitOutcomeUnknown,
    PostApplyVerificationFailed,
    CorruptCatalog,
}

impl PostgresStoreSetupErrorKind {
    pub const ALL: [Self; 16] = [
        Self::ManifestInvalid,
        Self::ChecksumMismatch,
        Self::TargetMismatch,
        Self::TargetUnowned,
        Self::SchemaCollision,
        Self::HistoryMismatch,
        Self::CompatibilityMismatch,
        Self::PermissionDenied,
        Self::ServerUnsupported,
        Self::UnsupportedFutureSchema,
        Self::UnsafeSetting,
        Self::NetworkBoundary,
        Self::TransactionFailed,
        Self::CommitOutcomeUnknown,
        Self::PostApplyVerificationFailed,
        Self::CorruptCatalog,
    ];

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::ManifestInvalid => "STORE_MANIFEST_INVALID",
            Self::ChecksumMismatch => "STORE_MIGRATION_CHECKSUM_MISMATCH",
            Self::TargetMismatch => "STORE_DATABASE_TARGET_MISMATCH",
            Self::TargetUnowned => "STORE_DATABASE_TARGET_UNOWNED",
            Self::SchemaCollision => "STORE_SCHEMA_COLLISION",
            Self::HistoryMismatch => "STORE_MIGRATION_HISTORY_MISMATCH",
            Self::CompatibilityMismatch => "STORE_SCHEMA_COMPATIBILITY_MISMATCH",
            Self::PermissionDenied => "STORE_DATABASE_PERMISSION_DENIED",
            Self::ServerUnsupported => "STORE_POSTGRES_SERVER_UNSUPPORTED",
            Self::UnsupportedFutureSchema => "STORE_SCHEMA_UNSUPPORTED_FUTURE",
            Self::UnsafeSetting => "STORE_POSTGRES_SETTING_UNSAFE",
            Self::NetworkBoundary => "STORE_POSTGRES_NETWORK_BOUNDARY_INVALID",
            Self::TransactionFailed => "STORE_MIGRATION_TRANSACTION_FAILED",
            Self::CommitOutcomeUnknown => "STORE_MIGRATION_OUTCOME_UNKNOWN",
            Self::PostApplyVerificationFailed => "STORE_MIGRATION_COMMITTED_UNVERIFIED",
            Self::CorruptCatalog => "STORE_POSTGRES_CATALOG_CORRUPT",
        }
    }
}

/// Bounded static setup failure that never retains driver diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostgresStoreSetupError {
    kind: PostgresStoreSetupErrorKind,
}

impl PostgresStoreSetupError {
    #[must_use]
    pub const fn new(kind: PostgresStoreSetupErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> PostgresStoreSetupErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }
}

impl fmt::Display for PostgresStoreSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for PostgresStoreSetupError {}

/// Canonical lowercase SHA-256 evidence value.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Sha256Hex(String);

impl Sha256Hex {
    fn from_digest(bytes: &[u8]) -> Self {
        Self(bytes_to_hex(bytes))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Verified identity of the complete embedded migration manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestEvidence {
    entry_count: usize,
    executable_count: usize,
    schema_version: u16,
    manifest_sha256: Sha256Hex,
}

impl ManifestEvidence {
    #[must_use]
    pub const fn entry_count(&self) -> usize {
        self.entry_count
    }

    #[must_use]
    pub const fn executable_count(&self) -> usize {
        self.executable_count
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> &Sha256Hex {
        &self.manifest_sha256
    }
}

/// Verifies every included migration byte and all frozen manifest metadata.
///
/// # Errors
///
/// Returns a static fail-closed error for any checksum, ordering, status,
/// compatibility, or structural mismatch.
pub fn verify_embedded_manifest() -> Result<ManifestEvidence, PostgresStoreSetupError> {
    verify_manifest(migration_manifest())
}

fn verify_manifest(
    manifest: &[MigrationDescriptor],
) -> Result<ManifestEvidence, PostgresStoreSetupError> {
    if manifest.len() != MIGRATION_MANIFEST.len() {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }

    let evidence = verify_manifest_entries(manifest)?;
    if manifest != MIGRATION_MANIFEST
        || evidence.executable_count != 9
        || evidence.manifest_sha256.as_str() != CURRENT_V8_MANIFEST_SHA256
        || manifest[0].status != MigrationStatus::Superseded
        || manifest[0].schema_version != 0
        || manifest[1].status != MigrationStatus::Executable
        || manifest[1].schema_version != 1
        || manifest[1].reader_compatibility() != (1..=1)
        || manifest[1].writer_compatibility() != (1..=1)
        || manifest[2].status != MigrationStatus::Executable
        || manifest[2].schema_version != STORE_V2_SCHEMA_VERSION
        || manifest[2].reader_compatibility() != (2..=2)
        || manifest[2].writer_compatibility() != (2..=2)
        || manifest[3].status != MigrationStatus::Executable
        || manifest[3].schema_version != 3
        || manifest[3].reader_compatibility() != (3..=3)
        || manifest[3].writer_compatibility() != (3..=3)
        || manifest[4].status != MigrationStatus::Executable
        || manifest[4].schema_version != 4
        || manifest[4].reader_compatibility() != (4..=4)
        || manifest[4].writer_compatibility() != (4..=4)
        || manifest[5].status != MigrationStatus::Executable
        || manifest[5].schema_version != 5
        || manifest[5].reader_compatibility() != (5..=5)
        || manifest[5].writer_compatibility() != (5..=5)
        || manifest[6].status != MigrationStatus::Executable
        || manifest[6].schema_version != 6
        || manifest[6].reader_compatibility() != (6..=6)
        || manifest[6].writer_compatibility() != (6..=6)
        || manifest[7].status != MigrationStatus::Executable
        || manifest[7].schema_version != 7
        || manifest[7].reader_compatibility() != (7..=7)
        || manifest[7].writer_compatibility() != (7..=7)
        || manifest[8].status != MigrationStatus::Executable
        || manifest[8].schema_version != POSTGRES_SCHEMA_VERSION
        || manifest[8].reader_compatibility() != (8..=8)
        || manifest[8].writer_compatibility() != (8..=8)
        || manifest[9].status != MigrationStatus::Executable
        || manifest[9].schema_version != POSTGRES_SCHEMA_VERSION
        || manifest[9].reader_compatibility() != (8..=8)
        || manifest[9].writer_compatibility() != (8..=8)
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

pub(crate) fn verify_v1_manifest_prefix() -> Result<ManifestEvidence, PostgresStoreSetupError> {
    let prefix = &MIGRATION_MANIFEST[..2];
    let evidence = verify_manifest_entries(prefix)?;
    if evidence.entry_count != 2
        || evidence.executable_count != 1
        || evidence.schema_version != 1
        || evidence.manifest_sha256.as_str() != LEGACY_V1_MANIFEST_SHA256
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

pub(crate) fn verify_v2_manifest_prefix() -> Result<ManifestEvidence, PostgresStoreSetupError> {
    let prefix = &MIGRATION_MANIFEST[..3];
    let evidence = verify_manifest_entries(prefix)?;
    if evidence.entry_count != 3
        || evidence.executable_count != 2
        || evidence.schema_version != STORE_V2_SCHEMA_VERSION
        || evidence.manifest_sha256.as_str() != STORE_V2_MANIFEST_SHA256
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

pub(crate) fn verify_v3_manifest_prefix() -> Result<ManifestEvidence, PostgresStoreSetupError> {
    let prefix = &MIGRATION_MANIFEST[..4];
    let evidence = verify_manifest_entries(prefix)?;
    if evidence.entry_count != 4 || evidence.executable_count != 3 || evidence.schema_version != 3 {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

pub(crate) fn verify_v4_manifest_prefix() -> Result<ManifestEvidence, PostgresStoreSetupError> {
    let prefix = &MIGRATION_MANIFEST[..5];
    let evidence = verify_manifest_entries(prefix)?;
    if evidence.entry_count != 5
        || evidence.executable_count != 4
        || evidence.schema_version != 4
        || evidence.manifest_sha256.as_str() != REGISTRY_V4_MANIFEST_SHA256
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

pub(crate) fn verify_v5_manifest_prefix() -> Result<ManifestEvidence, PostgresStoreSetupError> {
    let prefix = &MIGRATION_MANIFEST[..6];
    let evidence = verify_manifest_entries(prefix)?;
    if evidence.entry_count != 6
        || evidence.executable_count != 5
        || evidence.schema_version != 5
        || evidence.manifest_sha256.as_str() != CURRENT_V5_MANIFEST_SHA256
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

pub(crate) fn verify_v6_manifest_prefix() -> Result<ManifestEvidence, PostgresStoreSetupError> {
    let prefix = &MIGRATION_MANIFEST[..7];
    let evidence = verify_manifest_entries(prefix)?;
    if evidence.entry_count != 7
        || evidence.executable_count != 6
        || evidence.schema_version != 6
        || evidence.manifest_sha256.as_str() != CURRENT_V6_MANIFEST_SHA256
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

pub(crate) fn verify_v7_manifest_prefix() -> Result<ManifestEvidence, PostgresStoreSetupError> {
    let prefix = &MIGRATION_MANIFEST[..8];
    let evidence = verify_manifest_entries(prefix)?;
    if evidence.entry_count != 8
        || evidence.executable_count != 7
        || evidence.schema_version != 7
        || evidence.manifest_sha256.as_str() != CURRENT_V7_MANIFEST_SHA256
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

pub(crate) fn verify_v8_legacy_manifest_prefix() -> Result<ManifestEvidence, PostgresStoreSetupError>
{
    let prefix = &MIGRATION_MANIFEST[..9];
    let evidence = verify_manifest_entries(prefix)?;
    if evidence.entry_count != 9
        || evidence.executable_count != 8
        || evidence.schema_version != POSTGRES_SCHEMA_VERSION
        || evidence.manifest_sha256.as_str() != LEGACY_V8_MANIFEST_SHA256
    {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }
    Ok(evidence)
}

fn verify_manifest_entries(
    manifest: &[MigrationDescriptor],
) -> Result<ManifestEvidence, PostgresStoreSetupError> {
    if manifest.is_empty() {
        return Err(PostgresStoreSetupError::new(
            PostgresStoreSetupErrorKind::ManifestInvalid,
        ));
    }

    let mut executable_count = 0usize;
    let mut metadata = Vec::with_capacity(manifest.len());
    for (index, entry) in manifest.iter().enumerate() {
        if usize::from(entry.ordinal) != index + 1
            || entry.id.is_empty()
            || !entry
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
            || !entry.path.starts_with("db/migrations/")
            || Path::new(entry.path).extension() != Some(OsStr::new("sql"))
            || entry.byte_length != entry.bytes.len()
            || entry.min_reader > entry.max_reader
            || entry.min_writer > entry.max_writer
        {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::ManifestInvalid,
            ));
        }
        let actual = sha256_hex(entry.bytes);
        if actual != entry.sha256 {
            return Err(PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::ChecksumMismatch,
            ));
        }
        match (entry.status, entry.transaction_mode) {
            (MigrationStatus::Superseded, MigrationTransactionMode::NotExecuted) => {}
            (MigrationStatus::Executable, MigrationTransactionMode::RunnerOwned) => {
                executable_count += 1;
            }
            _ => {
                return Err(PostgresStoreSetupError::new(
                    PostgresStoreSetupErrorKind::ManifestInvalid,
                ));
            }
        }
        metadata.push(MigrationMetadata {
            ordinal: entry.ordinal,
            id: entry.id,
            path: entry.path,
            byte_length: u64::try_from(entry.byte_length).map_err(|_| {
                PostgresStoreSetupError::new(PostgresStoreSetupErrorKind::ManifestInvalid)
            })?,
            sha256: entry.sha256,
            status: entry.status,
            transaction_mode: entry.transaction_mode,
            schema_version: entry.schema_version,
            min_reader: entry.min_reader,
            max_reader: entry.max_reader,
            min_writer: entry.min_writer,
            max_writer: entry.max_writer,
        });
    }

    Ok(ManifestEvidence {
        entry_count: manifest.len(),
        executable_count,
        schema_version: manifest.last().map_or(0, |entry| entry.schema_version),
        manifest_sha256: Sha256Hex(migration_metadata_sha256(&metadata)),
    })
}

pub(crate) fn migration_metadata_sha256(entries: &[MigrationMetadata<'_>]) -> String {
    let mut manifest_hasher = Sha256::new();
    manifest_hasher.update(MANIFEST_HASH_DOMAIN);
    for entry in entries {
        update_manifest_field(&mut manifest_hasher, &entry.ordinal.to_be_bytes());
        update_manifest_field(&mut manifest_hasher, entry.id.as_bytes());
        update_manifest_field(&mut manifest_hasher, entry.path.as_bytes());
        update_manifest_field(&mut manifest_hasher, &entry.byte_length.to_be_bytes());
        update_manifest_field(&mut manifest_hasher, entry.sha256.as_bytes());
        update_manifest_field(&mut manifest_hasher, entry.status.as_str().as_bytes());
        update_manifest_field(
            &mut manifest_hasher,
            entry.transaction_mode.as_str().as_bytes(),
        );
        for value in [
            entry.schema_version,
            entry.min_reader,
            entry.max_reader,
            entry.min_writer,
            entry.max_writer,
        ] {
            update_manifest_field(&mut manifest_hasher, &value.to_be_bytes());
        }
    }
    bytes_to_hex(manifest_hasher.finalize().as_ref())
}

fn update_manifest_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    bytes_to_hex(digest.as_ref())
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for &byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_manifest_error(
        manifest: &[MigrationDescriptor],
        expected: PostgresStoreSetupErrorKind,
    ) {
        let error = verify_manifest(manifest).expect_err("mutated manifest must fail closed");
        assert_eq!(error.kind(), expected);
    }

    #[test]
    fn mutation_of_embedded_bytes_is_a_checksum_failure() {
        let mut manifest = MIGRATION_MANIFEST;
        manifest[1].bytes = b"x";
        manifest[1].byte_length = 1;
        assert_manifest_error(&manifest, PostgresStoreSetupErrorKind::ChecksumMismatch);
    }

    #[test]
    fn mutation_of_length_or_order_is_a_structural_failure() {
        let mut bad_length = MIGRATION_MANIFEST;
        bad_length[1].byte_length += 1;
        assert_manifest_error(&bad_length, PostgresStoreSetupErrorKind::ManifestInvalid);

        let mut bad_order = MIGRATION_MANIFEST;
        bad_order[1].ordinal = 9;
        assert_manifest_error(&bad_order, PostgresStoreSetupErrorKind::ManifestInvalid);
    }

    #[test]
    fn mutation_of_status_transaction_pair_is_a_structural_failure() {
        let mut manifest = MIGRATION_MANIFEST;
        manifest[0].status = MigrationStatus::Executable;
        assert_manifest_error(&manifest, PostgresStoreSetupErrorKind::ManifestInvalid);
    }

    #[test]
    fn legacy_prefix_digests_are_frozen_and_partial_manifest_is_rejected() {
        let legacy = verify_v1_manifest_prefix().expect("exact v1 prefix");
        assert_eq!(legacy.entry_count(), 2);
        assert_eq!(legacy.executable_count(), 1);
        assert_eq!(legacy.schema_version(), 1);
        assert_eq!(legacy.manifest_sha256().as_str(), LEGACY_V1_MANIFEST_SHA256);

        let store_v2 = verify_v2_manifest_prefix().expect("exact v2 prefix");
        assert_eq!(store_v2.entry_count(), 3);
        assert_eq!(store_v2.executable_count(), 2);
        assert_eq!(store_v2.schema_version(), STORE_V2_SCHEMA_VERSION);
        assert_eq!(
            store_v2.manifest_sha256().as_str(),
            STORE_V2_MANIFEST_SHA256
        );

        assert_manifest_error(
            &MIGRATION_MANIFEST[..2],
            PostgresStoreSetupErrorKind::ManifestInvalid,
        );
    }

    #[test]
    fn live_migration_checksum_mutation_fails_closed() {
        let mut manifest = MIGRATION_MANIFEST;
        manifest[2].bytes = b"x";
        manifest[2].byte_length = 1;
        assert_manifest_error(&manifest, PostgresStoreSetupErrorKind::ChecksumMismatch);
    }
}
