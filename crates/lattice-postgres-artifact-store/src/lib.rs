//! Fixed `PostgreSQL` persistence adapter for Artifact Store metadata.

mod adapter;
mod setup;

pub use adapter::*;
pub use setup::*;

use lattice_contracts::ContentDigest;
use sha2::{Digest, Sha256};

/// Fixed extension identity.
pub const ARTIFACT_EXTENSION_ID: &str = "lattice-postgres-artifact-store";
/// Repository-relative append-only v1 SQL profile.
pub const ARTIFACT_EXTENSION_SQL: &str =
    include_str!("../../../db/extensions/artifact-store/v1.sql");
const MANIFEST_DOMAIN: &[u8] = b"LATTICE_POSTGRES_ARTIFACT_EXTENSION_V1\0";

/// Exact embedded extension evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionManifestEvidence {
    sql_bytes: usize,
    sql_sha256: ContentDigest,
    manifest_sha256: ContentDigest,
}

impl ExtensionManifestEvidence {
    /// Exact embedded SQL byte length.
    #[must_use]
    pub const fn sql_bytes(&self) -> usize {
        self.sql_bytes
    }

    /// SHA-256 of exact embedded SQL bytes.
    #[must_use]
    pub const fn sql_sha256(&self) -> &ContentDigest {
        &self.sql_sha256
    }

    /// Domain-framed extension manifest digest.
    #[must_use]
    pub const fn manifest_sha256(&self) -> &ContentDigest {
        &self.manifest_sha256
    }
}

/// Verifies and returns the embedded append-only SQL identity.
///
/// # Errors
///
/// Rejects an impossible locally generated digest contract failure.
pub fn verify_embedded_extension_manifest() -> Result<ExtensionManifestEvidence, SetupError> {
    let sql_sha256 = content_digest(ARTIFACT_EXTENSION_SQL.as_bytes())?;
    let mut manifest = Sha256::new();
    manifest.update(MANIFEST_DOMAIN);
    manifest.update(
        u64::try_from(ARTIFACT_EXTENSION_SQL.len())
            .map_err(|_| SetupError::new(SetupErrorKind::EmbeddedManifest))?
            .to_be_bytes(),
    );
    manifest.update(ARTIFACT_EXTENSION_SQL.as_bytes());
    Ok(ExtensionManifestEvidence {
        sql_bytes: ARTIFACT_EXTENSION_SQL.len(),
        sql_sha256,
        manifest_sha256: ContentDigest::from_sha256(digest_hex(&manifest.finalize()))
            .map_err(|_| SetupError::new(SetupErrorKind::EmbeddedManifest))?,
    })
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

pub(crate) fn digest_bytes(digest: &ContentDigest) -> Result<Vec<u8>, SetupError> {
    decode_hex(digest.as_str()).ok_or_else(|| SetupError::new(SetupErrorKind::EmbeddedManifest))
}

fn content_digest(bytes: &[u8]) -> Result<ContentDigest, SetupError> {
    ContentDigest::from_sha256(digest_hex(&Sha256::digest(bytes)))
        .map_err(|_| SetupError::new(SetupErrorKind::EmbeddedManifest))
}

fn digest_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = decode_nibble(pair[0])?;
            let low = decode_nibble(pair[1])?;
            Some((high << 4) | low)
        })
        .collect()
}

const fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}
