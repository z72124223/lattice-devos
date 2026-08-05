//! Exact, offline Hermes runtime identity.

use serde::{Deserialize, Serialize};

use crate::{
    HERMES_RELEASE, HERMES_UPSTREAM_COMMIT, HermesAdapterError, HermesAdapterErrorKind,
    HermesAdapterResult,
};

/// Linux `CPython` release required by the reviewed Hermes runtime.
pub const HERMES_CPYTHON_VERSION: &str = "3.12.13";
/// python-build-standalone release which produced the pinned artifact.
pub const HERMES_CPYTHON_BUILD_RELEASE: &str = "20260804";
/// Upstream binary provenance. This is Astral, not a PSF binary distribution.
pub const HERMES_CPYTHON_PROVENANCE: &str = "astral-sh/python-build-standalone";
/// Exact byte length of the pinned `CPython` archive.
pub const HERMES_CPYTHON_ARCHIVE_BYTES: u64 = 111_375_313;
/// SHA-256 of the official uv-managed python-build-standalone artifact.
pub const HERMES_CPYTHON_ARCHIVE_SHA256: &str =
    "a140c0868258075d160fa0da51ddffd423efbc9dd350695abd33e7ce3ce94352";
/// SHA-256 of the upstream release `SHA256SUMS` evidence file.
pub const HERMES_CPYTHON_SHA256SUMS_SHA256: &str =
    "eccfdcc61c9fe48b7fe61db8812925ce30f23943d16c60861001004a4ae8f55c";
/// SHA-256 of the official Hermes archive for the pinned upstream commit.
pub const HERMES_RUNTIME_ARCHIVE_SHA256: &str =
    "a9a84a25999a23a859a9d17ef3134ea1c3371d8bf1984313eab839e939528152";
/// SHA-256 of `pyproject.toml` inside the pinned Hermes archive.
pub const HERMES_PYPROJECT_SHA256: &str =
    "64d1085ee1c23caf0ae0d9e65c73e280f466362ed43fdda1531f18f3af1d9869";
/// SHA-256 of `uv.lock` inside the pinned Hermes archive.
pub const HERMES_UV_LOCK_SHA256: &str =
    "aab3c83f71b683507a590b6315b23bdc0abd6b63b76b2349eae15bf00dfbaf2b";

/// Compile-time identity of the only Hermes source accepted by this adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HermesRuntimePin {
    pub(crate) cpython_version: &'static str,
    pub(crate) cpython_build_release: &'static str,
    pub(crate) cpython_provenance: &'static str,
    pub(crate) cpython_archive_bytes: u64,
    pub(crate) cpython_archive_sha256: &'static str,
    pub(crate) cpython_sha256sums_sha256: &'static str,
    pub(crate) archive_sha256: &'static str,
    pub(crate) pyproject_sha256: &'static str,
    pub(crate) uv_lock_sha256: &'static str,
}

impl HermesRuntimePin {
    pub(crate) const fn official() -> Self {
        Self {
            cpython_version: HERMES_CPYTHON_VERSION,
            cpython_build_release: HERMES_CPYTHON_BUILD_RELEASE,
            cpython_provenance: HERMES_CPYTHON_PROVENANCE,
            cpython_archive_bytes: HERMES_CPYTHON_ARCHIVE_BYTES,
            cpython_archive_sha256: HERMES_CPYTHON_ARCHIVE_SHA256,
            cpython_sha256sums_sha256: HERMES_CPYTHON_SHA256SUMS_SHA256,
            archive_sha256: HERMES_RUNTIME_ARCHIVE_SHA256,
            pyproject_sha256: HERMES_PYPROJECT_SHA256,
            uv_lock_sha256: HERMES_UV_LOCK_SHA256,
        }
    }
}

/// Canonical manifest for a complete, pre-built Linux runtime closure.
///
/// The manifest describes artifacts already present under a LATTICE-owned
/// runtime root. It contains no installer or network configuration; payload
/// traversal and hashing are performed before execution by the containment
/// backend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HermesOfflineRuntimeManifest {
    cpython_archive_bytes: u64,
    cpython_archive_sha256: String,
    cpython_build_release: String,
    cpython_provenance: String,
    cpython_sha256sums_sha256: String,
    cpython_version: String,
    hermes_archive_sha256: String,
    hermes_commit: String,
    hermes_release: String,
    payload_byte_count: u64,
    payload_file_count: u64,
    payload_manifest_sha256: String,
    platform: String,
    pyproject_sha256: String,
    schema: String,
    uv_lock_sha256: String,
}

impl HermesOfflineRuntimeManifest {
    /// Parses one byte-exact canonical offline manifest and validates every
    /// upstream identity field against compile-time pins.
    ///
    /// # Errors
    ///
    /// Rejects malformed JSON, duplicate or unknown fields, non-canonical
    /// bytes, empty payloads, invalid digests, and any identity drift.
    pub fn from_canonical_json(bytes: &[u8]) -> HermesAdapterResult<Self> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(|failure| {
            let code = if failure.to_string().contains("unknown field") {
                "HERMES_RUNTIME_MANIFEST_UNKNOWN_FIELD"
            } else {
                "HERMES_RUNTIME_MANIFEST_MALFORMED"
            };
            HermesAdapterError::new(HermesAdapterErrorKind::Malformed, code)
        })?;
        let canonical = serde_json::to_vec(&manifest).map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Malformed,
                "HERMES_RUNTIME_MANIFEST_CANONICALIZATION_FAILED",
            )
        })?;
        if canonical != bytes {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Malformed,
                "HERMES_RUNTIME_MANIFEST_NON_CANONICAL",
            ));
        }
        manifest.validate()?;
        Ok(manifest)
    }

    #[must_use]
    pub const fn payload_file_count(&self) -> u64 {
        self.payload_file_count
    }

    #[must_use]
    pub const fn payload_byte_count(&self) -> u64 {
        self.payload_byte_count
    }

    #[must_use]
    pub fn payload_manifest_sha256(&self) -> &str {
        &self.payload_manifest_sha256
    }

    fn validate(&self) -> HermesAdapterResult<()> {
        let pin = HermesRuntimePin::official();
        if self.schema != "lattice.hermes.offline-runtime.v1"
            || self.platform != "x86_64-unknown-linux-gnu"
            || self.cpython_version != pin.cpython_version
            || self.cpython_build_release != pin.cpython_build_release
            || self.cpython_provenance != pin.cpython_provenance
            || self.cpython_archive_bytes != pin.cpython_archive_bytes
            || self.cpython_archive_sha256 != pin.cpython_archive_sha256
            || self.cpython_sha256sums_sha256 != pin.cpython_sha256sums_sha256
            || self.hermes_release != HERMES_RELEASE
            || self.hermes_commit != HERMES_UPSTREAM_COMMIT
            || self.hermes_archive_sha256 != pin.archive_sha256
            || self.pyproject_sha256 != pin.pyproject_sha256
            || self.uv_lock_sha256 != pin.uv_lock_sha256
        {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Identity,
                "HERMES_RUNTIME_MANIFEST_IDENTITY_MISMATCH",
            ));
        }
        if self.payload_file_count == 0
            || self.payload_byte_count == 0
            || !is_lowercase_sha256(&self.payload_manifest_sha256)
        {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Identity,
                "HERMES_RUNTIME_MANIFEST_PAYLOAD_REJECTED",
            ));
        }
        Ok(())
    }
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
