pub use lattice_artifact_store::{
    ArtifactLimitKind, ArtifactObjectQuotaRecord, ArtifactObjectQuotaState, ArtifactReadIdentity,
    ArtifactReadQuotaRecord, ArtifactReadQuotaState, ArtifactReferenceIdentity,
    ArtifactReferenceQuotaRecord, ArtifactReferenceQuotaState, ArtifactStoreLimits,
};
#[allow(dead_code)]
#[path = "../src/semantics.rs"]
mod semantics;
pub use semantics::artifact_manifest_digest;
#[allow(dead_code)]
#[path = "../src/snapshot_contract.rs"]
mod snapshot_contract;
#[allow(dead_code)]
#[path = "../src/snapshot_parse.rs"]
mod snapshot_parse;

use std::fmt::Write as _;

use lattice_artifact_store::{ArtifactQuotaSnapshot, ArtifactStoreIdentity};
use lattice_cjson::canonicalize;
use lattice_contracts::{
    ArtifactAuthorityOwnerKind, ArtifactAuthorityStatus, ArtifactBundleBounds, ArtifactByteLength,
    ArtifactCounter, ArtifactGeneration, ArtifactObjectIdentity, ArtifactObjectKey,
    ArtifactProvenance, ArtifactPurpose, ArtifactReferenceAuthorityAction,
    ArtifactReferenceAuthorityBinding, ArtifactReferenceAuthorityPair,
    ArtifactReferenceAuthorityReceipt, ArtifactReferenceManifest, ArtifactRevision, AttemptId,
    ContentDigest, DaemonEpoch, ProjectId, ProjectSnapshotId, RequestId, RuntimeAdmissionMode,
    RuntimeKind, SubjectBinding, TaskId,
};
use semantics::{
    ArtifactIntegratedHeadEvidence, ArtifactLifecycleState, FakeArtifactAuthorityDirectory,
    FakeArtifactBytes, artifact_manifest_canonical_len,
};
use sha2::{Digest, Sha256};

const RETENTION: &str = "2026-07-30T00:10:00Z";

fn marker(hex: char) -> ContentDigest {
    ContentDigest::from_sha256(std::iter::repeat_n(hex, 64).collect::<String>()).expect("digest")
}

fn content_digest(bytes: &[u8]) -> ContentDigest {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("string formatting");
    }
    ContentDigest::from_sha256(text).expect("content digest")
}

fn object(project: &str, bytes: &[u8]) -> ArtifactObjectIdentity {
    ArtifactObjectIdentity::new(
        ArtifactObjectKey::new(
            ProjectId::new(project).expect("project"),
            content_digest(bytes),
        ),
        ArtifactGeneration::new(1).expect("generation"),
    )
}

fn reference_pair(
    object: &ArtifactObjectIdentity,
    task_id: &TaskId,
    reference_id: &str,
    owner_record_id: &str,
) -> ArtifactReferenceAuthorityPair {
    let binding = ArtifactReferenceAuthorityBinding::new(
        ArtifactAuthorityOwnerKind::TaskLedger,
        RuntimeKind::Fake,
        owner_record_id,
        ArtifactRevision::new(1).expect("revision"),
        ArtifactAuthorityStatus::Available,
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        object.key().project_id().clone(),
        task_id.clone(),
        object.clone(),
        reference_id,
        marker('1'),
    )
    .expect("binding");
    let receipt = ArtifactReferenceAuthorityReceipt::new(1, binding, marker('2')).expect("receipt");
    ArtifactReferenceAuthorityPair::new(receipt.clone(), receipt.head()).expect("pair")
}

fn manifest(
    object: &ArtifactObjectIdentity,
    bytes: &[u8],
    task_id_text: &str,
    reference_id: &str,
) -> ArtifactReferenceManifest {
    let task_id = TaskId::new(task_id_text).expect("task");
    let limits = ArtifactStoreLimits::hard_maximums();
    let binding = SubjectBinding::new(
        object.key().project_id().clone(),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        task_id.clone(),
        "1",
        marker('3'),
    )
    .expect("subject");
    let authority = reference_pair(
        object,
        &task_id,
        reference_id,
        &format!("owner-{reference_id}"),
    );
    let provenance = ArtifactProvenance::new(
        "projection-test",
        "1.0",
        RuntimeKind::Fake,
        marker('4'),
        "projection-adapter",
        "1.0",
        marker('5'),
        "invocation-1",
        "correlation-1",
        "run-1",
        ArtifactCounter::new(1).expect("sequence"),
        "2026-07-30T00:00:00Z",
        object.key().content_digest().clone(),
        "capability-1",
        marker('6'),
        marker('7'),
        marker('8'),
        marker('9'),
        marker('a'),
        "effect-claim-1",
        marker('b'),
        "daemon-1",
        DaemonEpoch::new(1).expect("epoch"),
        RuntimeAdmissionMode::Active,
        marker('c'),
        marker('d'),
        limits.limit_snapshot_digest().expect("limits"),
    )
    .expect("provenance");
    let byte_length = u64::try_from(bytes.len()).expect("test length");
    let bundle = ArtifactBundleBounds::new(
        ArtifactCounter::new(1).expect("entries"),
        ArtifactCounter::new(1).expect("depth"),
        ArtifactByteLength::new(byte_length).expect("bundle bytes"),
    )
    .expect("bundle");
    let build = |digest| {
        ArtifactReferenceManifest::new(
            binding.clone(),
            AttemptId::new("attempt-1").expect("attempt"),
            RequestId::new(format!("request-{reference_id}")).expect("request"),
            reference_id,
            object.clone(),
            ArtifactByteLength::new(byte_length).expect("length"),
            "application/octet-stream",
            "lattice.test.projection",
            "1.0",
            Some(bundle),
            provenance.clone(),
            authority.clone(),
            ArtifactPurpose::TaskOutput,
            RETENTION,
            digest,
        )
        .expect("manifest")
    };
    let provisional = build(marker('e'));
    build(artifact_manifest_digest(&provisional).expect("manifest digest"))
}

fn publish(
    state: &mut ArtifactLifecycleState,
    bytes_backend: &mut FakeArtifactBytes,
    directory: &mut FakeArtifactAuthorityDirectory,
    project: &str,
    bytes: &[u8],
    task_id: &str,
    reference_id: &str,
) -> ArtifactObjectIdentity {
    let object = object(project, bytes);
    let manifest = manifest(&object, bytes, task_id, reference_id);
    directory.install_reference_pair(manifest.creation_authority());
    state
        .publish(bytes_backend, manifest, bytes, directory)
        .expect("publish");
    object
}

fn evidence(
    state: &ArtifactLifecycleState,
    object: &ArtifactObjectIdentity,
    store_digest: ContentDigest,
    high_water: u64,
    tail: ContentDigest,
) -> ArtifactIntegratedHeadEvidence {
    ArtifactIntegratedHeadEvidence::new(
        object.clone(),
        state.object_head(object).expect("head").revision(),
        marker('1'),
        marker('2'),
        store_digest,
        marker('4'),
        ArtifactCounter::new(high_water).expect("high water"),
        tail,
    )
    .expect("evidence")
}

fn finalize(state: &mut ArtifactLifecycleState, store_digest: &ContentDigest, high_water: u64) {
    let evidence = state
        .current_object_identities()
        .into_iter()
        .enumerate()
        .map(|(index, object)| {
            evidence(
                state,
                &object,
                store_digest.clone(),
                high_water + u64::try_from(index).expect("index"),
                if index == 0 { marker('5') } else { marker('6') },
            )
        })
        .collect();
    state
        .refresh_integrated_heads(evidence)
        .expect("integrated refresh");
}

#[test]
fn unrelated_store_quota_refresh_stales_prior_head_without_lifecycle_revision() {
    let mut state = ArtifactLifecycleState::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object_a = publish(
        &mut state,
        &mut bytes_backend,
        &mut directory,
        "project-a",
        b"alpha",
        "task-a",
        "reference-a",
    );
    finalize(&mut state, &marker('a'), 9);
    let before = state.current_head(&object_a).expect("before");
    let before_revision = before.object().revision();
    let nested_reference = before.reference().cloned();

    let _object_b = publish(
        &mut state,
        &mut bytes_backend,
        &mut directory,
        "project-a",
        b"beta",
        "task-b",
        "reference-b",
    );
    let before_invalid_refresh = state.clone();
    assert_eq!(
        state
            .refresh_integrated_heads(Vec::new())
            .expect_err("evidence must cover every current object")
            .code(),
        "ARTIFACT_INTEGRATED_EVIDENCE_MISMATCH"
    );
    assert_eq!(state, before_invalid_refresh);
    finalize(&mut state, &marker('b'), 11);
    let after = state.current_head(&object_a).expect("after");
    let current_receipt = state.current_receipt(&object_a).expect("current receipt");

    assert_ne!(before, after);
    assert_eq!(current_receipt.head(), after);
    assert_eq!(after.object().revision(), before_revision);
    assert_eq!(after.reference(), nested_reference.as_ref());
    assert_eq!(after.object().store_quota_projection_digest(), &marker('b'));
    assert_ne!(
        after.object().command_high_water().get(),
        after.object().revision().get()
    );
    assert_eq!(after.object().command_tail_digest(), &marker('5'));
}

#[test]
fn lifecycle_derives_quota_records_and_sorted_scopes_without_caller_counts() {
    let raw = b"RAW_SECRET_ALPHA";
    let mut state = ArtifactLifecycleState::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object = publish(
        &mut state,
        &mut bytes_backend,
        &mut directory,
        "project-quota",
        raw,
        "task-quota",
        "reference-quota",
    );

    let objects = state.quota_object_records().expect("objects");
    let references = state.quota_reference_records().expect("references");
    let reads = state.quota_read_records().expect("reads");
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].identity(), &object);
    assert_eq!(
        objects[0].byte_length(),
        i64::try_from(raw.len() * 2).expect("accounted bytes")
    );
    assert_eq!(references.len(), 1);
    assert_eq!(references[0].object(), &object);
    assert_eq!(references[0].identity().task_id().as_str(), "task-quota");
    assert_eq!(references[0].identity().value(), "reference-quota");
    assert!(reads.is_empty());
    assert_eq!(state.current_task_scopes()[0].1.as_str(), "task-quota");
    assert_eq!(state.current_project_scopes()[0].as_str(), "project-quota");
    let expected_manifest_bytes = artifact_manifest_canonical_len(
        state
            .current_head(&object)
            .expect("head")
            .reference()
            .expect("reference")
            .manifest(),
    )
    .expect("manifest length");
    let report = ArtifactQuotaSnapshot::new(
        ArtifactStoreIdentity::new("projection-store").expect("store"),
        objects,
        references,
        reads,
        Vec::new(),
        Vec::new(),
    )
    .recompute(ArtifactStoreLimits::hard_maximums())
    .expect("quota recompute");
    assert_eq!(
        report.projection().get(ArtifactLimitKind::ManifestBytes),
        i64::try_from(expected_manifest_bytes).expect("manifest bytes")
    );
}

#[test]
fn metadata_digest_is_insertion_order_deterministic_and_contains_no_raw_bytes() {
    fn populated(reverse: bool) -> ArtifactLifecycleState {
        let mut state = ArtifactLifecycleState::default();
        let mut bytes_backend = FakeArtifactBytes::default();
        let mut directory = FakeArtifactAuthorityDirectory::default();
        let rows = if reverse {
            vec![
                (
                    "project-order",
                    b"RAW_SECRET_BETA".as_slice(),
                    "task-b",
                    "reference-b",
                ),
                (
                    "project-order",
                    b"RAW_SECRET_ALPHA".as_slice(),
                    "task-a",
                    "reference-a",
                ),
            ]
        } else {
            vec![
                (
                    "project-order",
                    b"RAW_SECRET_ALPHA".as_slice(),
                    "task-a",
                    "reference-a",
                ),
                (
                    "project-order",
                    b"RAW_SECRET_BETA".as_slice(),
                    "task-b",
                    "reference-b",
                ),
            ]
        };
        for (project, bytes, task, reference) in rows {
            publish(
                &mut state,
                &mut bytes_backend,
                &mut directory,
                project,
                bytes,
                task,
                reference,
            );
        }
        finalize(&mut state, &marker('f'), 20);
        state
    }

    let forward = populated(false);
    let reverse = populated(true);
    assert_eq!(
        forward.metadata_state_digest().expect("forward digest"),
        reverse.metadata_state_digest().expect("reverse digest")
    );

    let canonical = canonicalize(
        &forward
            .canonical_metadata_state()
            .expect("canonical metadata"),
    )
    .expect("canonical bytes");
    assert!(
        !canonical
            .as_slice()
            .windows(b"RAW_SECRET_ALPHA".len())
            .any(|window| window == b"RAW_SECRET_ALPHA")
    );
    assert!(
        !canonical
            .as_slice()
            .windows(b"RAW_SECRET_BETA".len())
            .any(|window| window == b"RAW_SECRET_BETA")
    );
}
