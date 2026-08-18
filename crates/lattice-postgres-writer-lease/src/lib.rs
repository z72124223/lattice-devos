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
/// Repository-relative location of the immutable v1 extension profile.
pub const WRITER_LEASE_V1_EXTENSION_PATH: &str = "db/extensions/writer-lease/v1.sql";
/// Repository-relative location of the current append-only v2 successor.
pub const WRITER_LEASE_EXTENSION_PATH: &str = "db/extensions/writer-lease/v2.sql";
/// Physical extension schema version.
pub const WRITER_LEASE_EXTENSION_SCHEMA_VERSION: u16 = 2;
const V1_EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v1.sql");
const EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/writer-lease/v2.sql");
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
