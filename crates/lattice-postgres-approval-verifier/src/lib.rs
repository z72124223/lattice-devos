//! Independent `PostgreSQL` persistence adapter for Approval Verifier 1.1.

use std::error::Error;
use std::fmt;

use lattice_contracts::ContentDigest;
use sha2::{Digest, Sha256};

mod adapter;
mod setup;

pub use adapter::PostgresApprovalVerifier;
pub use setup::{
    ExtensionApplyOutcome, ExtensionSetupError, ExtensionSetupErrorKind, ExtensionTarget,
    apply_extension, verify_extension,
};

/// Fixed extension identity.
pub const APPROVAL_EXTENSION_ID: &str = "lattice-approval-verifier";
/// Repository-relative append-only v1 SQL profile.
pub const APPROVAL_EXTENSION_PATH: &str = "db/extensions/approval-verifier/v1.sql";
/// Physical extension schema version.
pub const APPROVAL_EXTENSION_SCHEMA_VERSION: u16 = 1;
const EXTENSION_SQL: &[u8] = include_bytes!("../../../db/extensions/approval-verifier/v1.sql");
const EXPECTED_EXTENSION_SQL_BYTES: usize = 30_180;
const EXPECTED_EXTENSION_SQL_SHA256: &str =
    "9adfe2a6f270f48ac35c42afb9a1d7ec55f394433fe2f4192f0f8285d85e2b74";
const EXPECTED_EXTENSION_MANIFEST_SHA256: &str =
    "6a0223e4f012ff8de51e332232a865dee4c9fd8a778670faa24f24d9e1fe69ad";
const EXTENSION_MANIFEST_DOMAIN: &str = "lattice.postgres-approval-verifier.extension-manifest.v1";

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
            Self::SqlMismatch => "embedded Approval SQL mismatch",
            Self::ManifestMismatch => "Approval extension manifest mismatch",
            Self::Contract => "Approval extension digest rejected",
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
        APPROVAL_EXTENSION_ID
    }

    #[must_use]
    pub const fn path(&self) -> &'static str {
        APPROVAL_EXTENSION_PATH
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        APPROVAL_EXTENSION_SCHEMA_VERSION
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

/// Verifies compile-time SQL bytes and the complete manifest identity.
///
/// # Errors
///
/// Returns a typed failure for byte, hash, or identity drift.
pub fn verify_embedded_extension_manifest()
-> Result<ExtensionManifestEvidence, ExtensionManifestError> {
    let sql_sha256 = sha256_hex(EXTENSION_SQL);
    if EXTENSION_SQL.len() != EXPECTED_EXTENSION_SQL_BYTES
        || sql_sha256 != EXPECTED_EXTENSION_SQL_SHA256
    {
        return Err(ExtensionManifestError::SqlMismatch);
    }
    let subject = format!(
        "{EXTENSION_MANIFEST_DOMAIN}\n{APPROVAL_EXTENSION_ID}\n{APPROVAL_EXTENSION_PATH}\n\
         {APPROVAL_EXTENSION_SCHEMA_VERSION}\n{}\n{sql_sha256}\n",
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
