//! Independent `PostgreSQL` persistence adapter for the pure Writer Lease owner.

use std::error::Error;
use std::fmt;

use lattice_contracts::ContentDigest;
use sha2::{Digest, Sha256};

mod adapter;
mod setup;

pub use adapter::PostgresWriterLease;
pub use setup::{
    ExtensionApplyOutcome, ExtensionSetupError, ExtensionSetupErrorKind, ExtensionTarget,
    V3BootstrapProfile, V3ExtensionTarget, V4ExtensionTarget, apply_extension, apply_v3_extension,
    apply_v4_extension, inspect_v3_bootstrap_profile, rebind_existing_v3_extension,
    rebind_v3_extension, verify_extension,
};

/// Fixed extension identity.
pub const WRITER_LEASE_EXTENSION_ID: &str = "lattice-writer-lease";
/// Repository-relative location of the immutable v1 extension profile.
pub const WRITER_LEASE_V1_EXTENSION_PATH: &str = "db/extensions/writer-lease/v1.sql";
/// Repository-relative location of the current append-only v2 successor.
pub const WRITER_LEASE_EXTENSION_PATH: &str = "db/extensions/writer-lease/v2.sql";
/// Explicit alias for the frozen v2 successor path.
pub const WRITER_LEASE_V2_EXTENSION_PATH: &str = WRITER_LEASE_EXTENSION_PATH;
/// Repository-relative location of the immutable append-only v3 schema-v5/v6 bridge.
pub const WRITER_LEASE_V3_EXTENSION_PATH: &str = "db/extensions/writer-lease/v3.sql";
/// Repository-relative location of the fixed Writer-owned v3 rebind boundary.
pub const WRITER_LEASE_V3_REBIND_PATH: &str = "db/extensions/writer-lease/v3-rebind.sql";
/// Repository-relative location of the append-only v4 schema-v6/v7 bridge.
pub const WRITER_LEASE_V4_EXTENSION_PATH: &str = "db/extensions/writer-lease/v4.sql";
/// Repository-relative location of the fixed Writer-owned v4 rebind boundary.
pub const WRITER_LEASE_V4_REBIND_PATH: &str = "db/extensions/writer-lease/v4-rebind.sql";
/// Physical extension schema version.
pub const WRITER_LEASE_EXTENSION_SCHEMA_VERSION: u16 = 2;
/// Physical v3 bridge schema version.
pub const WRITER_LEASE_V3_EXTENSION_SCHEMA_VERSION: u16 = 3;
/// Physical v4 bridge schema version.
pub const WRITER_LEASE_V4_EXTENSION_SCHEMA_VERSION: u16 = 4;
const V1_EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v1.sql");
const EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v2.sql");
const V3_EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v3.sql");
const V3_REBIND_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v3-rebind.sql");
const V4_EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v4.sql");
const V4_REBIND_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v4-rebind.sql");
const EXPECTED_V1_EXTENSION_SQL_BYTES: usize = 44_366;
const EXPECTED_V1_EXTENSION_SQL_SHA256: &str =
    "63ffbf8f8b6c22bf35c3d393bd84e9462ca37e4ace94ceaedd6c27b729daa562";
const EXPECTED_V1_EXTENSION_MANIFEST_SHA256: &str =
    "0179e2a9b0976008902ab0d1cce6ab493a16047a649571f9ce4f13cc53cc6b33";
const EXPECTED_EXTENSION_SQL_BYTES: usize = 22_985;
const EXPECTED_EXTENSION_SQL_SHA256: &str =
    "8243fd39a3565c641423fde3f15cf801a4a48a12c8d238ae8e1657acdcdc56e3";
const EXPECTED_EXTENSION_MANIFEST_SHA256: &str =
    "5f54c182465c8e2dc8a6e6cc2ebd9a375f776adf500656586e59bfbc7dfd31a4";
const EXPECTED_V3_EXTENSION_SQL_BYTES: usize = 17_568;
const EXPECTED_V3_EXTENSION_SQL_SHA256: &str =
    "677c010a61e5945bcc6b96ca9f3d9e57830dc42f4cfbd46ea76d5e9d8b9262a0";
const EXPECTED_V3_EXTENSION_MANIFEST_SHA256: &str =
    "eab2812fa3d94cd3466d7c003386f805a973fd7def1f16aeb15b52f47dad78e4";
const EXPECTED_V3_REBIND_SQL_BYTES: usize = 10_286;
const EXPECTED_V3_REBIND_SQL_SHA256: &str =
    "ff04f37f4c1c008eff2d8f9117ae974f0d83efd3fd08d38ebae0682840bc0a09";
const EXPECTED_V3_REBIND_MANIFEST_SHA256: &str =
    "7a139f709e8c22d27eae5722588187c350338ac96301aee1e22697bc55143362";
const EXPECTED_V4_EXTENSION_SQL_BYTES: usize = 19_205;
const EXPECTED_V4_EXTENSION_SQL_SHA256: &str =
    "51996b50c9a7d3696f8319613d35acae6257c5802b63dc4a809873721a22da09";
const EXPECTED_V4_EXTENSION_MANIFEST_SHA256: &str =
    "73d3e435c5923797076d30cea337d84b94b2e760db6e9727033b68ace592a229";
const EXPECTED_V4_REBIND_SQL_BYTES: usize = 10_733;
const EXPECTED_V4_REBIND_SQL_SHA256: &str =
    "67e5f8830877f85ebc5e12a478ea5e5e807496c568da8332d76b6de9e05752b6";
const EXPECTED_V4_REBIND_MANIFEST_SHA256: &str =
    "21568a392427659285e8077609cf6685fd0b24a4662ea299d9465310905bd547";
const EXTENSION_MANIFEST_DOMAIN: &str = "lattice.postgres-writer-lease.extension-manifest.v1";

/// Exact embedded-extension identity failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionManifestError {
    SqlMismatch,
    ManifestMismatch,
    Contract,
}

impl fmt::Display for ExtensionManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SqlMismatch => "embedded Writer Lease SQL mismatch",
            Self::ManifestMismatch => "Writer Lease extension manifest mismatch",
            Self::Contract => "Writer Lease extension digest rejected",
        })
    }
}

impl Error for ExtensionManifestError {}

/// Verified identity and exact bytes of the embedded extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionManifestEvidence {
    path: &'static str,
    schema_version: u16,
    bytes: &'static [u8],
    sql_sha256: ContentDigest,
    manifest_sha256: ContentDigest,
}

impl ExtensionManifestEvidence {
    #[must_use]
    pub const fn extension_id(&self) -> &'static str {
        WRITER_LEASE_EXTENSION_ID
    }

    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn bytes(&self) -> &'static [u8] {
        self.bytes
    }

    #[must_use]
    pub const fn byte_length(&self) -> usize {
        self.bytes.len()
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

/// Verifies the compile-time SQL bytes and complete manifest identity.
///
/// # Errors
///
/// Returns a typed failure for any byte, hash, or identity drift.
pub fn verify_embedded_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        WRITER_LEASE_EXTENSION_PATH,
        WRITER_LEASE_EXTENSION_SCHEMA_VERSION,
        EXTENSION_SQL,
        EXPECTED_EXTENSION_SQL_BYTES,
        EXPECTED_EXTENSION_SQL_SHA256,
        EXPECTED_EXTENSION_MANIFEST_SHA256,
    )
}

/// Verifies the frozen v2 extension bytes explicitly.
///
/// # Errors
///
/// Returns a typed failure for any byte, hash, or identity drift.
pub fn verify_embedded_v2_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_embedded_extension_manifest()
}

/// Verifies the append-only v3 bridge bytes and complete identity.
///
/// # Errors
///
/// Returns a typed failure for any byte, hash, or identity drift.
pub fn verify_embedded_v3_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        WRITER_LEASE_V3_EXTENSION_PATH,
        WRITER_LEASE_V3_EXTENSION_SCHEMA_VERSION,
        V3_EXTENSION_SQL,
        EXPECTED_V3_EXTENSION_SQL_BYTES,
        EXPECTED_V3_EXTENSION_SQL_SHA256,
        EXPECTED_V3_EXTENSION_MANIFEST_SHA256,
    )
}

/// Verifies the append-only Writer-owned v3 administrative rebind boundary.
///
/// # Errors
///
/// Returns a typed failure for any byte, hash, path, or identity drift.
pub fn verify_embedded_v3_rebind_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        WRITER_LEASE_V3_REBIND_PATH,
        WRITER_LEASE_V3_EXTENSION_SCHEMA_VERSION,
        V3_REBIND_SQL,
        EXPECTED_V3_REBIND_SQL_BYTES,
        EXPECTED_V3_REBIND_SQL_SHA256,
        EXPECTED_V3_REBIND_MANIFEST_SHA256,
    )
}

/// Verifies the append-only v4 schema-v6/v7 bridge bytes and identity.
///
/// # Errors
///
/// Returns a typed failure for any byte, hash, path, or identity drift.
pub fn verify_embedded_v4_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        WRITER_LEASE_V4_EXTENSION_PATH,
        WRITER_LEASE_V4_EXTENSION_SCHEMA_VERSION,
        V4_EXTENSION_SQL,
        EXPECTED_V4_EXTENSION_SQL_BYTES,
        EXPECTED_V4_EXTENSION_SQL_SHA256,
        EXPECTED_V4_EXTENSION_MANIFEST_SHA256,
    )
}

/// Verifies the append-only Writer-owned v4 administrative rebind boundary.
///
/// # Errors
///
/// Returns a typed failure for any byte, hash, path, or identity drift.
pub fn verify_embedded_v4_rebind_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        WRITER_LEASE_V4_REBIND_PATH,
        WRITER_LEASE_V4_EXTENSION_SCHEMA_VERSION,
        V4_REBIND_SQL,
        EXPECTED_V4_REBIND_SQL_BYTES,
        EXPECTED_V4_REBIND_SQL_SHA256,
        EXPECTED_V4_REBIND_MANIFEST_SHA256,
    )
}

/// Verifies the immutable v1 extension bytes and complete identity.
///
/// # Errors
///
/// Returns a typed failure for byte, hash, or identity drift.
pub fn verify_embedded_v1_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        WRITER_LEASE_V1_EXTENSION_PATH,
        1,
        V1_EXTENSION_SQL,
        EXPECTED_V1_EXTENSION_SQL_BYTES,
        EXPECTED_V1_EXTENSION_SQL_SHA256,
        EXPECTED_V1_EXTENSION_MANIFEST_SHA256,
    )
}

fn verify_manifest(
    path: &'static str,
    schema_version: u16,
    bytes: &'static [u8],
    expected_bytes: usize,
    expected_sql_sha256: &str,
    expected_manifest_sha256: &str,
) -> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    let sql_sha256 = sha256_hex(bytes);
    if bytes.len() != expected_bytes || sql_sha256 != expected_sql_sha256 {
        return Err(ExtensionManifestError::SqlMismatch);
    }
    let subject = format!(
        "{EXTENSION_MANIFEST_DOMAIN}\n{WRITER_LEASE_EXTENSION_ID}\n{path}\n{schema_version}\n{}\n{sql_sha256}\n",
        bytes.len()
    );
    let manifest_sha256 = sha256_hex(subject.as_bytes());
    if manifest_sha256 != expected_manifest_sha256 {
        return Err(ExtensionManifestError::ManifestMismatch);
    }
    Ok(ExtensionManifestEvidence {
        path,
        schema_version,
        bytes,
        sql_sha256: ContentDigest::from_sha256(sql_sha256)
            .map_err(|_| ExtensionManifestError::Contract)?,
        manifest_sha256: ContentDigest::from_sha256(manifest_sha256)
            .map_err(|_| ExtensionManifestError::Contract)?,
    })
}

/// Closed offline state used to characterize the immutable v2-to-v3 and
/// schema-v5-to-v6 compatibility transitions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterLeaseV3BridgeState {
    V2Current,
    Bridge,
    BridgePending,
    Current,
}

impl WriterLeaseV3BridgeState {
    #[must_use]
    pub const fn runtime_function_count(self) -> u8 {
        match self {
            Self::V2Current | Self::Current => 7,
            Self::Bridge | Self::BridgePending => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterLeaseV3BridgeError {
    UnsupportedGeneration,
    HistoryMismatch,
}

/// Verifies one exact v3 bridge/rebind state transition or exact retry.
///
/// # Errors
///
/// Returns a typed failure for unknown generations, reordered history, skipped
/// transitions, or cross-generation replay.
pub fn verify_writer_lease_v3_transition(
    state: WriterLeaseV3BridgeState,
    global_schema_version: u16,
    ledger_shape: &str,
) -> Result<WriterLeaseV3BridgeState, WriterLeaseV3BridgeError> {
    const V2_CURRENT: [&str; 2] = ["1:INSTALLED", "1:INSTALLED,2:UPGRADED,3:REBOUND"];
    const V3_BRIDGE: [&str; 2] = [
        "1:INSTALLED,2:UPGRADED",
        "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED",
    ];
    const V3_CURRENT: [&str; 3] = [
        "1:INSTALLED",
        "1:INSTALLED,2:UPGRADED,3:REBOUND",
        "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED,5:REBOUND",
    ];
    match (state, global_schema_version) {
        (WriterLeaseV3BridgeState::V2Current, 5) if V2_CURRENT.contains(&ledger_shape) => {
            Ok(WriterLeaseV3BridgeState::Bridge)
        }
        (WriterLeaseV3BridgeState::Bridge, 5) if V3_BRIDGE.contains(&ledger_shape) => {
            Ok(WriterLeaseV3BridgeState::Bridge)
        }
        (WriterLeaseV3BridgeState::Bridge | WriterLeaseV3BridgeState::BridgePending, 6)
            if V3_BRIDGE.contains(&ledger_shape) =>
        {
            Ok(WriterLeaseV3BridgeState::BridgePending)
        }
        (WriterLeaseV3BridgeState::BridgePending | WriterLeaseV3BridgeState::Current, 6)
            if V3_CURRENT.contains(&ledger_shape) =>
        {
            Ok(WriterLeaseV3BridgeState::Current)
        }
        (_, 5..=6) => Err(WriterLeaseV3BridgeError::HistoryMismatch),
        _ => Err(WriterLeaseV3BridgeError::UnsupportedGeneration),
    }
}

/// Closed offline state for the immutable Writer-v3/schema-v6 predecessor and
/// append-only Writer-v4/schema-v7 successor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriterLeaseV4BridgeState {
    V3Current,
    Bridge,
    Current,
}

impl WriterLeaseV4BridgeState {
    #[must_use]
    pub const fn runtime_function_count(self) -> u8 {
        match self {
            Self::V3Current | Self::Current => 7,
            Self::Bridge => 0,
        }
    }
}

/// Verifies one exact Writer-v4 bridge/rebind transition or retry.
///
/// # Errors
///
/// Rejects future generations, skipped/reordered history, and any attempt to
/// relabel the frozen v3 identity as schema v7.
pub fn verify_writer_lease_v4_transition(
    state: WriterLeaseV4BridgeState,
    global_schema_version: u16,
    ledger_shape: &str,
) -> Result<WriterLeaseV4BridgeState, WriterLeaseV3BridgeError> {
    const V3_CURRENT: [&str; 3] = [
        "1:INSTALLED",
        "1:INSTALLED,2:UPGRADED,3:REBOUND",
        "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED,5:REBOUND",
    ];
    const V4_BRIDGE: [&str; 3] = [
        "1:INSTALLED,2:UPGRADED",
        "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED",
        "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED,5:REBOUND,6:UPGRADED",
    ];
    const V4_CURRENT: [&str; 3] = [
        "1:INSTALLED,2:UPGRADED,3:REBOUND",
        "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED,5:REBOUND",
        "1:INSTALLED,2:UPGRADED,3:REBOUND,4:UPGRADED,5:REBOUND,6:UPGRADED,7:REBOUND",
    ];
    match (state, global_schema_version) {
        (WriterLeaseV4BridgeState::V3Current, 6) if V3_CURRENT.contains(&ledger_shape) => {
            Ok(WriterLeaseV4BridgeState::Bridge)
        }
        (WriterLeaseV4BridgeState::Bridge, 6) if V4_BRIDGE.contains(&ledger_shape) => {
            Ok(WriterLeaseV4BridgeState::Bridge)
        }
        (WriterLeaseV4BridgeState::Bridge | WriterLeaseV4BridgeState::Current, 7)
            if V4_CURRENT.contains(&ledger_shape) =>
        {
            Ok(WriterLeaseV4BridgeState::Current)
        }
        (_, 6..=7) => Err(WriterLeaseV3BridgeError::HistoryMismatch),
        _ => Err(WriterLeaseV3BridgeError::UnsupportedGeneration),
    }
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
