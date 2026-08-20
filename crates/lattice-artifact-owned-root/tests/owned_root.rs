use std::fs::{self, OpenOptions};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lattice_artifact_owned_root::{
    OwnedArtifactRoot, OwnedRootErrorKind, PublishDisposition, owned_root_marker_bytes,
};
use lattice_contracts::{
    ArtifactGeneration, ArtifactObjectIdentity, ArtifactObjectKey, ContentDigest, ProjectId,
};
use sha2::{Digest, Sha256};

fn fixture(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "lattice-task025-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("fixture root");
    path
}

fn marker(root: &Path, root_id: &str) {
    let path = root.join(".lattice-artifact-owned-root");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .expect("marker");
    std::io::Write::write_all(&mut file, &owned_root_marker_bytes(root_id)).expect("marker bytes");
    file.sync_all().expect("marker flush");
}

fn object(project: &str, bytes: &[u8], generation: u64) -> ArtifactObjectIdentity {
    let digest = hex::encode(Sha256::digest(bytes));
    ArtifactObjectIdentity::new(
        ArtifactObjectKey::new(
            ProjectId::new(project).expect("project"),
            ContentDigest::from_sha256(digest).expect("digest"),
        ),
        ArtifactGeneration::new(generation).expect("generation"),
    )
}

#[test]
fn unverified_root_is_rejected_before_any_effect() {
    let root = fixture("unverified");
    let before = fs::read_dir(&root).expect("read root").count();
    let error = OwnedArtifactRoot::admit(&root, "root-a", &[]).expect_err("must reject");
    assert_eq!(error.kind(), OwnedRootErrorKind::UnverifiedRoot);
    assert_eq!(fs::read_dir(&root).expect("read root").count(), before);
    fs::remove_dir(&root).expect("empty fixture cleanup");
}

#[test]
fn product_root_overlap_is_rejected() {
    let root = fixture("overlap");
    marker(&root, "root-b");
    let product = root.join("product");
    fs::create_dir(&product).expect("product");
    let error = OwnedArtifactRoot::admit(&root, "root-b", std::slice::from_ref(&product))
        .expect_err("ancestor overlap");
    assert_eq!(error.kind(), OwnedRootErrorKind::ProductRootOverlap);
    fs::remove_dir(product).expect("product cleanup");
    fs::remove_file(root.join(".lattice-artifact-owned-root")).expect("marker cleanup");
    fs::remove_dir(root).expect("root cleanup");
}

#[cfg(windows)]
#[test]
fn reparse_or_symlink_root_is_rejected_before_canonicalization() {
    let parent = fixture("symlink-parent");
    let target = parent.join("target");
    fs::create_dir(&target).expect("target root");
    marker(&target, "root-symlink");
    let link = parent.join("link");
    let junction = std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .status()
        .expect("junction command");
    assert!(junction.success(), "junction fixture");

    let error =
        OwnedArtifactRoot::admit(&link, "root-symlink", &[]).expect_err("symlink root rejected");
    assert_eq!(error.kind(), OwnedRootErrorKind::UnverifiedRoot);
    fs::remove_dir(link).expect("symlink cleanup");
    fs::remove_file(target.join(".lattice-artifact-owned-root")).expect("marker cleanup");
    fs::remove_dir(target).expect("target cleanup");
    fs::remove_dir(parent).expect("parent cleanup");
}

#[test]
fn hardlinked_marker_is_rejected() {
    let root = fixture("hardlink-marker");
    marker(&root, "root-hardlink");
    let linked = root.join("marker-hardlink");
    fs::hard_link(root.join(".lattice-artifact-owned-root"), &linked).expect("hardlink");
    let error =
        OwnedArtifactRoot::admit(&root, "root-hardlink", &[]).expect_err("linked marker rejected");
    assert_eq!(error.kind(), OwnedRootErrorKind::UnverifiedRoot);
    fs::remove_file(linked).expect("link cleanup");
    fs::remove_file(root.join(".lattice-artifact-owned-root")).expect("marker cleanup");
    fs::remove_dir(root).expect("root cleanup");
}

#[test]
fn admitted_marker_identity_cannot_be_substituted() {
    let root = fixture("marker-substitution");
    marker(&root, "root-marker-substitution");
    let mut adapter =
        OwnedArtifactRoot::admit(&root, "root-marker-substitution", &[]).expect("admit");
    fs::remove_file(root.join(".lattice-artifact-owned-root")).expect("remove original marker");
    marker(&root, "root-marker-substitution");

    let error = adapter
        .cleanup_empty_fixture()
        .expect_err("substituted marker rejected");
    assert_eq!(error.kind(), OwnedRootErrorKind::UnverifiedRoot);
    fs::remove_file(root.join(".lattice-artifact-owned-root")).expect("marker cleanup");
    fs::remove_dir(root).expect("root cleanup");
}

#[cfg(windows)]
#[test]
fn alternate_data_stream_on_marker_is_rejected() {
    let root = fixture("marker-ads");
    marker(&root, "root-ads");
    let marker_path = root.join(".lattice-artifact-owned-root");
    let ads_path = PathBuf::from(format!("{}:forbidden", marker_path.display()));
    let mut ads = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(ads_path)
        .expect("create marker ADS");
    std::io::Write::write_all(&mut ads, b"forbidden").expect("write marker ADS");
    drop(ads);

    let error = OwnedArtifactRoot::admit(&root, "root-ads", &[]).expect_err("ADS rejected");
    assert_eq!(error.kind(), OwnedRootErrorKind::UnverifiedRoot);
    fs::remove_file(marker_path).expect("marker and ADS cleanup");
    fs::remove_dir(root).expect("root cleanup");
}

#[test]
fn empty_object_is_valid_and_over_limit_declaration_has_no_effect() {
    let root = fixture("empty");
    marker(&root, "root-empty");
    let mut adapter = OwnedArtifactRoot::admit(&root, "root-empty", &[]).expect("admit");
    let empty = object("project-empty", b"", 1);
    let sealed = adapter
        .stage(&empty, 0, Cursor::new([]))
        .expect("empty stage");
    assert_eq!(
        adapter.publish(sealed).expect("empty publish"),
        PublishDisposition::Published
    );
    assert!(
        adapter
            .read_verified(&empty, 0)
            .expect("empty read")
            .is_empty()
    );
    let before = fs::read_dir(root.join(".staging"))
        .expect("staging")
        .count();
    let error = adapter
        .stage(
            &empty,
            lattice_artifact_owned_root::MAX_OBJECT_BYTES + 1,
            Cursor::new([]),
        )
        .expect_err("over limit");
    assert_eq!(error.kind(), OwnedRootErrorKind::ByteLimit);
    assert_eq!(
        fs::read_dir(root.join(".staging"))
            .expect("staging")
            .count(),
        before
    );
    adapter
        .unlink_claimed(&empty, "claim-empty")
        .expect("empty unlink");
    adapter.cleanup_empty_fixture().expect("enumerated cleanup");
    fs::remove_file(root.join(".lattice-artifact-owned-root")).expect("marker cleanup");
    fs::remove_dir(root).expect("root cleanup");
}

#[test]
fn sealed_bytes_publish_no_clobber_read_and_exact_unlink() {
    let root = fixture("lifecycle");
    marker(&root, "root-c");
    let mut adapter = OwnedArtifactRoot::admit(&root, "root-c", &[]).expect("admit");
    let bytes = b"durable artifact bytes";
    let identity = object("project-one", bytes, 1);

    let first = adapter
        .stage(&identity, bytes.len() as u64, Cursor::new(bytes))
        .expect("stage first");
    let second = adapter
        .stage(&identity, bytes.len() as u64, Cursor::new(bytes))
        .expect("stage second");
    let competitor = OwnedArtifactRoot::admit(&root, "root-c", &[]).expect("competitor admit");
    let first_thread = std::thread::spawn(move || {
        let mut adapter = adapter;
        let outcome = adapter.publish(first).expect("publish first contender");
        (adapter, outcome)
    });
    let second_thread = std::thread::spawn(move || {
        let mut adapter = competitor;
        let outcome = adapter.publish(second).expect("publish second contender");
        (adapter, outcome)
    });
    let (mut adapter, first_outcome) = first_thread.join().expect("first join");
    let (mut competitor, second_outcome) = second_thread.join().expect("second join");
    assert_eq!(
        [first_outcome, second_outcome]
            .into_iter()
            .filter(|outcome| *outcome == PublishDisposition::Published)
            .count(),
        1
    );
    assert_eq!(
        [first_outcome, second_outcome]
            .into_iter()
            .filter(|outcome| *outcome == PublishDisposition::ReusedVerifiedWinner)
            .count(),
        1
    );
    assert_eq!(
        adapter
            .read_verified(&identity, bytes.len() as u64)
            .expect("read"),
        bytes
    );

    let error = adapter
        .unlink_claimed(&identity, "")
        .expect_err("empty token rejected");
    assert_eq!(error.kind(), OwnedRootErrorKind::InvalidClaim);
    adapter
        .unlink_claimed(&identity, "claim-token-1")
        .expect("exact unlink");
    assert_eq!(
        adapter
            .read_verified(&identity, bytes.len() as u64)
            .expect_err("deleted")
            .kind(),
        OwnedRootErrorKind::MissingObject
    );
    for _ in 0..8 {
        let competitor_clean = competitor.cleanup_empty_fixture().is_ok();
        let adapter_clean = adapter.cleanup_empty_fixture().is_ok();
        if competitor_clean && adapter_clean {
            break;
        }
    }
    competitor
        .cleanup_empty_fixture()
        .expect("competitor enumerated cleanup");
    adapter.cleanup_empty_fixture().expect("enumerated cleanup");
    fs::remove_file(root.join(".lattice-artifact-owned-root")).expect("marker cleanup");
    fs::remove_dir(root).expect("root cleanup");
}

#[test]
fn digest_mismatch_never_publishes_and_sealed_orphan_can_be_quarantined() {
    let root = fixture("failure");
    marker(&root, "root-d");
    let mut adapter = OwnedArtifactRoot::admit(&root, "root-d", &[]).expect("admit");
    let identity = object("project-two", b"expected", 1);
    let error = adapter
        .stage(&identity, 5, Cursor::new(b"wrong"))
        .expect_err("digest mismatch");
    assert_eq!(error.kind(), OwnedRootErrorKind::DigestMismatch);

    let sealed = adapter
        .stage(&identity, 8, Cursor::new(b"expected"))
        .expect("seal orphan");
    let quarantined = adapter.quarantine(sealed).expect("quarantine");
    assert!(quarantined.is_quarantined());
    adapter
        .discard_quarantined(quarantined)
        .expect("exact quarantine cleanup");
    adapter.cleanup_empty_fixture().expect("enumerated cleanup");
    fs::remove_file(root.join(".lattice-artifact-owned-root")).expect("marker cleanup");
    fs::remove_dir(root).expect("root cleanup");
}

mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(bytes.as_ref().len() * 2);
        for byte in bytes.as_ref() {
            output.push(char::from(DIGITS[usize::from(byte >> 4)]));
            output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
        }
        output
    }
}
