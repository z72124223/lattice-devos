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
    apply_extension, verify_extension,
};

/// Fixed extension identity.
pub const WRITER_LEASE_EXTENSION_ID: &str = "lattice-writer-lease";
/// Repository-relative location of the sole embedded extension profile.
pub const WRITER_LEASE_EXTENSION_PATH: &str = "db/extensions/writer-lease/v1.sql";
/// Physical extension schema version.
pub const WRITER_LEASE_EXTENSION_SCHEMA_VERSION: u16 = 1;
const EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v1.sql");
const EXPECTED_EXTENSION_SQL_BYTES: usize = 44_366;
const EXPECTED_EXTENSION_SQL_SHA256: &str =
    "63ffbf8f8b6c22bf35c3d393bd84e9462ca37e4ace94ceaedd6c27b729daa562";
const EXPECTED_EXTENSION_MANIFEST_SHA256: &str =
    "0179e2a9b0976008902ab0d1cce6ab493a16047a649571f9ce4f13cc53cc6b33";
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
        WRITER_LEASE_EXTENSION_PATH
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        WRITER_LEASE_EXTENSION_SCHEMA_VERSION
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

/// Verifies the compile-time SQL bytes and complete manifest identity.
///
/// # Errors
///
/// Returns a typed failure for any byte, hash, or identity drift.
pub fn verify_embedded_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    let sql_sha256 = sha256_hex(EXTENSION_SQL);
    if EXTENSION_SQL.len() != EXPECTED_EXTENSION_SQL_BYTES
        || sql_sha256 != EXPECTED_EXTENSION_SQL_SHA256
    {
        return Err(ExtensionManifestError::SqlMismatch);
    }
    let subject = format!(
        "{EXTENSION_MANIFEST_DOMAIN}\n{WRITER_LEASE_EXTENSION_ID}\n{WRITER_LEASE_EXTENSION_PATH}\n{WRITER_LEASE_EXTENSION_SCHEMA_VERSION}\n{}\n{sql_sha256}\n",
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
