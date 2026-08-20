//! Independent `PostgreSQL` Codebase Memory extension and adapter.

use std::error::Error;
use std::fmt;

use lattice_contracts::ContentDigest;
pub use lattice_contracts::{
    CODEBASE_MEMORY_EXTENSION_ID, CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION,
};
use sha2::{Digest, Sha256};

mod adapter;
mod setup;

pub use adapter::PostgresCodebaseMemory;
pub use setup::{
    ExtensionApplyOutcome, ExtensionCatalogEvidence, ExtensionDatabaseRole, ExtensionSetupError,
    ExtensionSetupErrorKind, ExtensionTarget, apply_extension, verify_extension,
};

/// Repository-relative location of the frozen v1 extension profile.
pub const CODEBASE_MEMORY_V1_EXTENSION_PATH: &str = "db/extensions/codebase-memory/v1.sql";
/// Repository-relative location of the frozen v2 extension profile.
pub const CODEBASE_MEMORY_V2_EXTENSION_PATH: &str = "db/extensions/codebase-memory/v2.sql";
/// Repository-relative location of the current append-only v3 profile.
pub const CODEBASE_MEMORY_EXTENSION_PATH: &str = "db/extensions/codebase-memory/v3.sql";
const V1_EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/codebase-memory/v1.sql");
const V2_EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/codebase-memory/v2.sql");
const EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/codebase-memory/v3.sql");
const EXPECTED_V1_EXTENSION_SQL_BYTES: usize = 42_411;
const EXPECTED_V1_EXTENSION_SQL_SHA256: &str =
    "555eabce843417bcbcd111a3cec42d05f3e2aaff802aa168b54be2fbfb300a3f";
const EXPECTED_V1_EXTENSION_MANIFEST_SHA256: &str =
    "90942d378fce1e7a35356e537bd3724c505fe062cd581b5be956a2960f531600";
const EXPECTED_V2_EXTENSION_SQL_BYTES: usize = 76_866;
const EXPECTED_V2_EXTENSION_SQL_SHA256: &str =
    "9db54342b88f554ca76054c7a33ae72f04b412d2dfe21fae6eb4d8faf3e854e2";
const EXPECTED_V2_EXTENSION_MANIFEST_SHA256: &str =
    "0aedbd7d9ef7ca07fc2910d0da34c163cc83e3dd56f9b28292ae1f4f0c3c4d7e";
const EXPECTED_EXTENSION_SQL_BYTES: usize = 87_545;
const EXPECTED_EXTENSION_SQL_SHA256: &str =
    "7388f6bfe4c2d30a20306e4f9ebdff5862125bcab58f769ba286af542cb051c3";
const EXPECTED_EXTENSION_MANIFEST_SHA256: &str =
    "d4cc712d262ae1f7c96bd65526eab611c90e193363afd865af2126307b2903f0";
const EXTENSION_MANIFEST_DOMAIN: &str = "lattice.postgres-codebase-memory.extension-manifest.v1";

/// Exact embedded extension-manifest verification failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionManifestError {
    /// The embedded SQL bytes no longer match the frozen descriptor.
    SqlMismatch,
    /// The complete extension identity no longer matches its frozen digest.
    ManifestMismatch,
    /// A computed SHA-256 could not be represented by the shared contract.
    Contract,
}

impl fmt::Display for ExtensionManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SqlMismatch => "embedded Codebase Memory SQL mismatch",
            Self::ManifestMismatch => "Codebase Memory extension manifest mismatch",
            Self::Contract => "Codebase Memory extension digest rejected",
        })
    }
}

impl Error for ExtensionManifestError {}

/// Verified identity of the sole embedded extension profile.
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
        CODEBASE_MEMORY_EXTENSION_ID
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

/// Verifies the compile-time extension bytes and complete fixed identity.
///
/// # Errors
///
/// Returns a typed failure for byte/hash/identity drift.
pub fn verify_embedded_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        CODEBASE_MEMORY_EXTENSION_PATH,
        CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION,
        EXTENSION_SQL,
        EXPECTED_EXTENSION_SQL_BYTES,
        EXPECTED_EXTENSION_SQL_SHA256,
        EXPECTED_EXTENSION_MANIFEST_SHA256,
    )
}

/// Verifies the immutable v1 extension bytes and identity.
///
/// # Errors
///
/// Returns a typed failure for byte/hash/identity drift.
pub fn verify_embedded_v1_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        CODEBASE_MEMORY_V1_EXTENSION_PATH,
        1,
        V1_EXTENSION_SQL,
        EXPECTED_V1_EXTENSION_SQL_BYTES,
        EXPECTED_V1_EXTENSION_SQL_SHA256,
        EXPECTED_V1_EXTENSION_MANIFEST_SHA256,
    )
}

/// Verifies the immutable v2 extension bytes and identity.
///
/// # Errors
///
/// Returns a typed failure for byte/hash/identity drift.
pub fn verify_embedded_v2_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    verify_manifest(
        CODEBASE_MEMORY_V2_EXTENSION_PATH,
        2,
        V2_EXTENSION_SQL,
        EXPECTED_V2_EXTENSION_SQL_BYTES,
        EXPECTED_V2_EXTENSION_SQL_SHA256,
        EXPECTED_V2_EXTENSION_MANIFEST_SHA256,
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
        "{EXTENSION_MANIFEST_DOMAIN}\n{CODEBASE_MEMORY_EXTENSION_ID}\n{path}\n{schema_version}\n{}\n{sql_sha256}\n",
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
