//! Rebuildable, secret-free Hermes preparation assets.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::broker::official_codex_config_lock_bytes;
use crate::containment::{OUTER_RUNNER_SOURCE, PRIVATE_RUNNER_SOURCE};
use crate::runtime::{
    HERMES_RUNTIME_PAYLOAD_BYTE_COUNT, HERMES_RUNTIME_PAYLOAD_FILE_COUNT,
    HERMES_RUNTIME_PAYLOAD_TREE_SHA256, HermesOfflineRuntimeManifest,
};
#[cfg(windows)]
use crate::windows_path_is_unsupported;
use crate::{
    HERMES_RELEASE, HERMES_UPSTREAM_COMMIT, HermesAdapterError, HermesAdapterErrorKind,
    HermesAdapterResult, metadata_is_reparse_point, path_is_within,
    reject_link_or_reparse_ancestors,
};

pub const OFFICIAL_HERMES_RUNTIME_GUEST_ROOT: &str = concat!(
    "/var/tmp/lattice-runtime-targets/",
    "hermes-v2026.8.3-cpython-3.12.13-pbs-20260804-errorfix-v1"
);

const OFFLINE_RUNTIME_MANIFEST_NAME: &str = "offline-runtime-manifest.json";
const PREPARED_ASSETS_NAME: &str = "prepared-assets.json";
pub(crate) const OFFICIAL_HERMES_CONFIG: &[u8] = br"_config_version: 33
model:
  provider: openai-api
  default: gpt-5.6-terra
  openai_runtime: codex_app_server
  api_mode: codex_app_server
  base_url: http://127.0.0.1:9/v1
platform_toolsets:
  api_server: []
plugins:
  enabled: []
mcp_servers: {}
";

#[derive(Debug, Serialize)]
struct PreparedSourceAsset<'a> {
    byte_count: u64,
    name: &'a str,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct PreparedAssets<'a> {
    hermes_commit: &'a str,
    hermes_release: &'a str,
    runtime_byte_count: u64,
    runtime_file_count: u64,
    runtime_guest_root: &'a str,
    runtime_manifest_sha256: String,
    runtime_tree_sha256: &'a str,
    schema: &'a str,
    source_assets: Vec<PreparedSourceAsset<'a>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HermesPreparationReceipt {
    bundle_sha256: String,
    manifest_sha256: String,
    prepared_assets_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HermesPreparationDisposition {
    Created,
    Present,
}

#[derive(Debug, Eq, PartialEq)]
pub struct HermesPreparationOutcome {
    disposition: HermesPreparationDisposition,
    receipt: HermesPreparationReceipt,
}

impl HermesPreparationOutcome {
    #[must_use]
    pub const fn disposition(&self) -> HermesPreparationDisposition {
        self.disposition
    }

    #[must_use]
    pub const fn receipt(&self) -> &HermesPreparationReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn render(&self) -> String {
        let state = match self.disposition {
            HermesPreparationDisposition::Created => "ASSETS_CREATED_UNVERIFIED",
            HermesPreparationDisposition::Present => "ASSETS_PRESENT_UNVERIFIED",
        };
        format!(
            "LATTICE_HERMES_PREPARE_{state}:{}",
            self.receipt.bundle_sha256()
        )
    }
}

impl HermesPreparationReceipt {
    #[must_use]
    pub const fn file_count(&self) -> usize {
        2
    }

    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    #[must_use]
    pub fn prepared_assets_sha256(&self) -> &str {
        &self.prepared_assets_sha256
    }

    #[must_use]
    pub fn bundle_sha256(&self) -> &str {
        &self.bundle_sha256
    }

    #[must_use]
    pub fn render_created(&self) -> String {
        format!(
            "LATTICE_HERMES_PREPARE_ASSETS_CREATED_UNVERIFIED:{}",
            self.bundle_sha256
        )
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct HermesPreparationBundle {
    manifest_bytes: Vec<u8>,
    prepared_assets_bytes: Vec<u8>,
    receipt: HermesPreparationReceipt,
}

impl HermesPreparationBundle {
    #[must_use]
    pub fn files(&self) -> [(&'static str, &[u8]); 2] {
        [
            (OFFLINE_RUNTIME_MANIFEST_NAME, &self.manifest_bytes),
            (PREPARED_ASSETS_NAME, &self.prepared_assets_bytes),
        ]
    }

    #[must_use]
    pub fn offline_runtime_manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }

    #[must_use]
    pub fn prepared_assets_bytes(&self) -> &[u8] {
        &self.prepared_assets_bytes
    }

    #[must_use]
    pub fn manifest_sha256(&self) -> &str {
        self.receipt.manifest_sha256()
    }

    #[must_use]
    pub const fn receipt(&self) -> &HermesPreparationReceipt {
        &self.receipt
    }
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn source_asset(name: &'static str, bytes: &'static [u8]) -> PreparedSourceAsset<'static> {
    PreparedSourceAsset {
        byte_count: bytes.len() as u64,
        name,
        sha256: sha256(bytes),
    }
}

fn official_source_assets() -> Vec<PreparedSourceAsset<'static>> {
    vec![
        source_asset("codex-config-lock", official_codex_config_lock_bytes()),
        source_asset("hermes-config-yaml", OFFICIAL_HERMES_CONFIG),
        source_asset("inner-runner", PRIVATE_RUNNER_SOURCE.as_bytes()),
        source_asset("outer-runner", OUTER_RUNNER_SOURCE.as_bytes()),
    ]
}

/// Builds the exact source-owned, secret-free preparation bundle in memory.
///
/// This does not inspect the host, read credentials, or grant launch authority.
pub fn official_preparation_bundle() -> HermesAdapterResult<HermesPreparationBundle> {
    let manifest_bytes = HermesOfflineRuntimeManifest::official_canonical_bytes()?;
    let manifest_sha256 = sha256(&manifest_bytes);
    let prepared = PreparedAssets {
        hermes_commit: HERMES_UPSTREAM_COMMIT,
        hermes_release: HERMES_RELEASE,
        runtime_byte_count: HERMES_RUNTIME_PAYLOAD_BYTE_COUNT,
        runtime_file_count: HERMES_RUNTIME_PAYLOAD_FILE_COUNT,
        runtime_guest_root: OFFICIAL_HERMES_RUNTIME_GUEST_ROOT,
        runtime_manifest_sha256: manifest_sha256.clone(),
        runtime_tree_sha256: HERMES_RUNTIME_PAYLOAD_TREE_SHA256,
        schema: "lattice.hermes.prepared-assets.v1",
        source_assets: official_source_assets(),
    };
    let prepared_assets_bytes = serde_json::to_vec(&prepared).map_err(|_| {
        HermesAdapterError::new(
            HermesAdapterErrorKind::Malformed,
            "HERMES_PREPARATION_CANONICALIZATION_FAILED",
        )
    })?;
    let prepared_assets_sha256 = sha256(&prepared_assets_bytes);
    let bundle_sha256 = sha256(
        format!(
            "lattice.hermes.preparation-bundle.v1\0{manifest_sha256}\0{prepared_assets_sha256}"
        )
        .as_bytes(),
    );
    Ok(HermesPreparationBundle {
        manifest_bytes,
        prepared_assets_bytes,
        receipt: HermesPreparationReceipt {
            bundle_sha256,
            manifest_sha256,
            prepared_assets_sha256,
        },
    })
}

fn preparation_error(kind: HermesAdapterErrorKind, code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(kind, code)
}

fn canonical_preparation_target(
    target_root: &Path,
    product_root: &Path,
) -> HermesAdapterResult<PathBuf> {
    if !target_root.is_absolute()
        || target_root.file_name().is_none()
        || !product_root.is_absolute()
        || !product_root.is_dir()
    {
        return Err(preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        ));
    }
    #[cfg(windows)]
    if windows_path_is_unsupported(target_root) || windows_path_is_unsupported(product_root) {
        return Err(preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        ));
    }
    let parent = target_root.parent().ok_or_else(|| {
        preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        )
    })?;
    if !parent.is_dir() {
        return Err(preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        ));
    }
    reject_link_or_reparse_ancestors(parent).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        )
    })?;
    reject_link_or_reparse_ancestors(product_root).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        )
    })?;
    let canonical_parent = fs::canonicalize(parent).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        )
    })?;
    let canonical_product = fs::canonicalize(product_root).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        )
    })?;
    let candidate = canonical_parent.join(target_root.file_name().ok_or_else(|| {
        preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        )
    })?);
    if path_is_within(&candidate, &canonical_product)
        || path_is_within(&canonical_product, &candidate)
    {
        return Err(preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_TARGET_REJECTED",
        ));
    }
    Ok(candidate)
}

fn verify_exact_bundle(
    target_root: &Path,
    bundle: &HermesPreparationBundle,
) -> HermesAdapterResult<()> {
    let target_metadata = fs::symlink_metadata(target_root).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PREPARATION_ASSET_CONFLICT",
        )
    })?;
    if !target_metadata.is_dir()
        || target_metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&target_metadata)
    {
        return Err(preparation_error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PREPARATION_ASSET_CONFLICT",
        ));
    }
    reject_link_or_reparse_ancestors(target_root).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PREPARATION_ASSET_CONFLICT",
        )
    })?;

    let expected = bundle.files();
    let entries = fs::read_dir(target_root).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PREPARATION_ASSET_CONFLICT",
        )
    })?;
    let mut seen = [false; 2];
    for entry in entries {
        let entry = entry.map_err(|_| {
            preparation_error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PREPARATION_ASSET_CONFLICT",
            )
        })?;
        let name = entry.file_name();
        let Some(index) = expected
            .iter()
            .position(|(expected_name, _)| name == OsStr::new(expected_name))
        else {
            return Err(preparation_error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PREPARATION_ASSET_CONFLICT",
            ));
        };
        if seen[index] {
            return Err(preparation_error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PREPARATION_ASSET_CONFLICT",
            ));
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| {
            preparation_error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PREPARATION_ASSET_CONFLICT",
            )
        })?;
        let expected_bytes = expected[index].1;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&metadata)
            || metadata.len() != expected_bytes.len() as u64
        {
            return Err(preparation_error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PREPARATION_ASSET_CONFLICT",
            ));
        }
        let actual = fs::read(entry.path()).map_err(|_| {
            preparation_error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PREPARATION_ASSET_CONFLICT",
            )
        })?;
        if actual != expected_bytes {
            return Err(preparation_error(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_PREPARATION_ASSET_CONFLICT",
            ));
        }
        seen[index] = true;
    }
    if seen != [true; 2] {
        return Err(preparation_error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PREPARATION_ASSET_CONFLICT",
        ));
    }
    Ok(())
}

/// Materializes the exact two-file preparation bundle under a fresh root, or
/// verifies an already exact bundle without rewriting it.
///
/// This operation does not inspect credentials, launch processes, or grant
/// Hermes launch authority.
pub fn materialize_official_preparation_bundle(
    target_root: &Path,
    product_root: &Path,
) -> HermesAdapterResult<HermesPreparationOutcome> {
    let target_root = canonical_preparation_target(target_root, product_root)?;
    let bundle = official_preparation_bundle()?;
    if target_root.exists() {
        verify_exact_bundle(&target_root, &bundle)?;
        return Ok(HermesPreparationOutcome {
            disposition: HermesPreparationDisposition::Present,
            receipt: bundle.receipt.clone(),
        });
    }

    fs::create_dir(&target_root).map_err(|_| {
        let code = if target_root.exists() {
            "HERMES_PREPARATION_RECONCILIATION_REQUIRED"
        } else {
            "HERMES_PREPARATION_WRITE_REJECTED"
        };
        preparation_error(HermesAdapterErrorKind::Failed, code)
    })?;
    for (name, bytes) in bundle.files() {
        let path = target_root.join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| {
                preparation_error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_PREPARATION_RECONCILIATION_REQUIRED",
                )
            })?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| {
                preparation_error(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_PREPARATION_RECONCILIATION_REQUIRED",
                )
            })?;
    }
    verify_exact_bundle(&target_root, &bundle).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::Ambiguous,
            "HERMES_PREPARATION_RECONCILIATION_REQUIRED",
        )
    })?;
    Ok(HermesPreparationOutcome {
        disposition: HermesPreparationDisposition::Created,
        receipt: bundle.receipt.clone(),
    })
}

/// Revalidates the exact prepared bundle and binds it to the receipt digest
/// supplied by process-start configuration.
///
/// This gate only proves that the rebuildable, secret-free preparation assets
/// match the current binary. It does not inspect credentials, launch a process,
/// or grant Hermes launch authority by itself.
pub fn verify_official_preparation_for_launch(
    target_root: &Path,
    product_root: &Path,
    expected_receipt_sha256: &str,
) -> HermesAdapterResult<HermesPreparationReceipt> {
    let target_root = canonical_preparation_target(target_root, product_root)?;
    if expected_receipt_sha256.len() != 64
        || !expected_receipt_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(preparation_error(
            HermesAdapterErrorKind::Malformed,
            "HERMES_PREPARATION_LAUNCH_RECEIPT_REJECTED",
        ));
    }
    if !target_root.exists() {
        return Err(preparation_error(
            HermesAdapterErrorKind::Configuration,
            "HERMES_PREPARATION_LAUNCH_ASSETS_REQUIRED",
        ));
    }

    let bundle = official_preparation_bundle()?;
    verify_exact_bundle(&target_root, &bundle).map_err(|_| {
        preparation_error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PREPARATION_LAUNCH_DIGEST_MISMATCH",
        )
    })?;
    if bundle.receipt.bundle_sha256() != expected_receipt_sha256 {
        return Err(preparation_error(
            HermesAdapterErrorKind::CrossBinding,
            "HERMES_PREPARATION_LAUNCH_DIGEST_MISMATCH",
        ));
    }
    Ok(bundle.receipt.clone())
}
