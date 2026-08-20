use lattice_hermes_adapter::preparation::{
    HermesPreparationDisposition, OFFICIAL_HERMES_RUNTIME_GUEST_ROOT,
    materialize_official_preparation_bundle, official_preparation_bundle,
    verify_official_preparation_for_launch,
};
use lattice_hermes_adapter::{
    HERMES_RELEASE, HERMES_UPSTREAM_COMMIT, HermesOfflineRuntimeManifest,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct FixtureCleanup(PathBuf);

impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture_parent(name: &str) -> (PathBuf, FixtureCleanup) {
    let unique = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("task057-{name}-{}-{unique}", std::process::id()));
    fs::create_dir_all(&root).expect("create test-owned fixture parent");
    (root.clone(), FixtureCleanup(root))
}

fn file_names(root: &Path) -> Vec<String> {
    let mut names = fs::read_dir(root)
        .expect("read prepared root")
        .map(|entry| {
            entry
                .expect("prepared entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn official_preparation_bundle_is_canonical_secret_free_and_self_consistent() {
    let bundle = official_preparation_bundle().expect("compile-time bundle");
    assert_eq!(bundle.files().len(), 2);

    let manifest_bytes = bundle.offline_runtime_manifest_bytes();
    HermesOfflineRuntimeManifest::from_canonical_json(manifest_bytes)
        .expect("current parser round-trips exact canonical manifest bytes");
    assert_eq!(sha256(manifest_bytes), bundle.manifest_sha256());

    let prepared_bytes = bundle.prepared_assets_bytes();
    let prepared: Value = serde_json::from_slice(prepared_bytes).expect("prepared asset JSON");
    assert_eq!(serde_json::to_vec(&prepared).unwrap(), prepared_bytes);
    assert_eq!(prepared["schema"], "lattice.hermes.prepared-assets.v1");
    assert_eq!(prepared["hermes_release"], HERMES_RELEASE);
    assert_eq!(prepared["hermes_commit"], HERMES_UPSTREAM_COMMIT);
    assert_eq!(
        prepared["runtime_guest_root"],
        OFFICIAL_HERMES_RUNTIME_GUEST_ROOT
    );
    assert_eq!(
        prepared["runtime_manifest_sha256"],
        bundle.manifest_sha256()
    );
    assert_eq!(prepared["runtime_file_count"], 14_077);
    assert_eq!(prepared["runtime_byte_count"], 722_643_145_u64);
    assert_eq!(
        prepared["runtime_tree_sha256"],
        "cb0e331bcb2b4fe2fd0977401d246819aadb800b645ca31ec233ad4e25b96929"
    );

    let assets = prepared["source_assets"].as_array().expect("source assets");
    assert_eq!(assets.len(), 4);
    let names = assets
        .iter()
        .map(|asset| asset["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "codex-config-lock",
            "hermes-config-yaml",
            "inner-runner",
            "outer-runner"
        ]
    );
    for asset in assets {
        assert!(asset["byte_count"].as_u64().unwrap() > 0);
        assert_eq!(asset["sha256"].as_str().unwrap().len(), 64);
    }

    let text = String::from_utf8(prepared_bytes.to_vec()).unwrap();
    for forbidden in [
        "auth",
        "secret",
        "api_key",
        "created_at",
        "timestamp",
        "C:\\\\",
        "\\\\Users\\\\",
    ] {
        assert!(
            !text
                .to_ascii_lowercase()
                .contains(&forbidden.to_ascii_lowercase())
        );
    }

    let receipt = bundle.receipt();
    assert_eq!(receipt.file_count(), 2);
    assert_eq!(receipt.manifest_sha256(), bundle.manifest_sha256());
    assert_eq!(receipt.prepared_assets_sha256(), sha256(prepared_bytes));
    assert_eq!(receipt.bundle_sha256().len(), 64);
    let receipt_text = receipt.render_created();
    assert!(receipt_text.starts_with("LATTICE_HERMES_PREPARE_ASSETS_CREATED_UNVERIFIED:"));
    assert!(!receipt_text.contains('/') && !receipt_text.contains('\\'));
}

#[test]
fn materialization_is_exclusive_exact_and_idempotent_without_rewrite() {
    let (parent, _cleanup) = fixture_parent("materialize");
    let product_root = parent.join("product");
    fs::create_dir(&product_root).expect("create protected product root");
    let target = parent.join("prepared");

    let created = materialize_official_preparation_bundle(&target, &product_root)
        .expect("materialize fresh bundle");
    assert_eq!(created.disposition(), HermesPreparationDisposition::Created);
    assert_eq!(
        file_names(&target),
        ["offline-runtime-manifest.json", "prepared-assets.json"]
    );
    let manifest_before = fs::read(target.join("offline-runtime-manifest.json")).unwrap();
    let prepared_before = fs::read(target.join("prepared-assets.json")).unwrap();
    let manifest_modified = fs::metadata(target.join("offline-runtime-manifest.json"))
        .unwrap()
        .modified()
        .unwrap();
    let prepared_modified = fs::metadata(target.join("prepared-assets.json"))
        .unwrap()
        .modified()
        .unwrap();

    let present = materialize_official_preparation_bundle(&target, &product_root)
        .expect("verify existing exact bundle");
    assert_eq!(present.disposition(), HermesPreparationDisposition::Present);
    assert_eq!(created.receipt(), present.receipt());
    assert_eq!(
        fs::metadata(target.join("offline-runtime-manifest.json"))
            .unwrap()
            .modified()
            .unwrap(),
        manifest_modified
    );
    assert_eq!(
        fs::metadata(target.join("prepared-assets.json"))
            .unwrap()
            .modified()
            .unwrap(),
        prepared_modified
    );
    assert_eq!(
        fs::read(target.join("offline-runtime-manifest.json")).unwrap(),
        manifest_before
    );
    assert_eq!(
        fs::read(target.join("prepared-assets.json")).unwrap(),
        prepared_before
    );
    assert!(
        created
            .render()
            .starts_with("LATTICE_HERMES_PREPARE_ASSETS_CREATED_UNVERIFIED:")
    );
    assert!(
        present
            .render()
            .starts_with("LATTICE_HERMES_PREPARE_ASSETS_PRESENT_UNVERIFIED:")
    );
}

#[test]
fn launch_preparation_gate_rejects_missing_assets_or_receipt_digest() {
    let (parent, _cleanup) = fixture_parent("launch-gate-missing");
    let product_root = parent.join("product");
    fs::create_dir(&product_root).expect("create protected product root");
    let target = parent.join("prepared");

    let missing_assets = verify_official_preparation_for_launch(
        &target,
        &product_root,
        "0000000000000000000000000000000000000000000000000000000000000000",
    )
    .expect_err("missing prepared assets must fail closed");
    assert_eq!(
        missing_assets.code(),
        "HERMES_PREPARATION_LAUNCH_ASSETS_REQUIRED"
    );

    materialize_official_preparation_bundle(&target, &product_root)
        .expect("materialize exact test bundle");
    let missing_receipt = verify_official_preparation_for_launch(&target, &product_root, "")
        .expect_err("missing receipt digest must fail closed");
    assert_eq!(
        missing_receipt.code(),
        "HERMES_PREPARATION_LAUNCH_RECEIPT_REJECTED"
    );
}

#[test]
fn launch_preparation_gate_accepts_only_the_exact_receipt_without_launching() {
    let (parent, _cleanup) = fixture_parent("launch-gate-exact");
    let product_root = parent.join("product");
    fs::create_dir(&product_root).expect("create protected product root");
    let target = parent.join("prepared");
    let prepared = materialize_official_preparation_bundle(&target, &product_root)
        .expect("materialize exact test bundle");

    let mismatch = verify_official_preparation_for_launch(
        &target,
        &product_root,
        "0000000000000000000000000000000000000000000000000000000000000000",
    )
    .expect_err("a substituted receipt digest must fail closed");
    assert_eq!(mismatch.code(), "HERMES_PREPARATION_LAUNCH_DIGEST_MISMATCH");

    let verified = verify_official_preparation_for_launch(
        &target,
        &product_root,
        prepared.receipt().bundle_sha256(),
    )
    .expect("exact assets and receipt advance only past the preparation gate");
    assert_eq!(&verified, prepared.receipt());
}
