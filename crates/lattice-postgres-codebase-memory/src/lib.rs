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

/// Repository-relative location of the sole embedded extension profile.
pub const CODEBASE_MEMORY_EXTENSION_PATH: &str = "db/extensions/codebase-memory/v1.sql";
const EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/codebase-memory/v1.sql");
const EXPECTED_EXTENSION_SQL_BYTES: usize = 42_411;
const EXPECTED_EXTENSION_SQL_SHA256: &str =
    "555eabce843417bcbcd111a3cec42d05f3e2aaff802aa168b54be2fbfb300a3f";
const EXPECTED_EXTENSION_MANIFEST_SHA256: &str =
    "90942d378fce1e7a35356e537bd3724c505fe062cd581b5be956a2960f531600";
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
        CODEBASE_MEMORY_EXTENSION_PATH
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION
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

/// Verifies the compile-time extension bytes and complete fixed identity.
///
/// # Errors
///
/// Returns a typed failure for byte/hash/identity drift.
pub fn verify_embedded_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    let sql_sha256 = sha256_hex(EXTENSION_SQL);
    if EXTENSION_SQL.len() != EXPECTED_EXTENSION_SQL_BYTES
        || sql_sha256 != EXPECTED_EXTENSION_SQL_SHA256
    {
        return Err(ExtensionManifestError::SqlMismatch);
    }
    let subject = format!(
        "{EXTENSION_MANIFEST_DOMAIN}\n{CODEBASE_MEMORY_EXTENSION_ID}\n{CODEBASE_MEMORY_EXTENSION_PATH}\n{CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION}\n{}\n{sql_sha256}\n",
        EXTENSION_SQL.len()
    );
    let manifest_sha256 = sha256_hex(subject.as_bytes());
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
