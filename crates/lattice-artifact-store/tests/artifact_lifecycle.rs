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
use lattice_contracts::{
    ARTIFACT_STORE_PRODUCER_ID, ARTIFACT_STORE_PRODUCER_VERSION, ArtifactAuthorityStatus,
    ArtifactAvailability, ArtifactBundleBounds, ArtifactByteLength, ArtifactCounter,
    ArtifactGeneration, ArtifactObjectIdentity, ArtifactObjectKey, ArtifactProvenance,
    ArtifactPurpose, ArtifactReadAuthorityAction, ArtifactReadAuthorityBinding,
    ArtifactReadAuthorityPair, ArtifactReadAuthorityReceipt, ArtifactReadClosureEvidenceBinding,
    ArtifactReadClosureEvidenceKind, ArtifactReadClosureEvidencePair,
    ArtifactReadClosureEvidenceReceipt, ArtifactReferenceAuthorityAction,
    ArtifactReferenceAuthorityBinding, ArtifactReferenceAuthorityPair,
    ArtifactReferenceAuthorityReceipt, ArtifactReferenceManifest, ArtifactRevision,
    ArtifactSweepAuthorityAction, ArtifactSweepAuthorityBinding, ArtifactSweepAuthorityPair,
    ArtifactSweepAuthorityReceipt, AttemptId, ContentDigest, DaemonEpoch, ProjectId,
    ProjectSnapshotId, RequestId, RuntimeAdmissionMode, RuntimeKind, SubjectBinding, TaskId,
};
use semantics::{
    ArtifactLifecycleState as FakeArtifactStore, ArtifactReconciliationResult,
    FakeArtifactAuthorityDirectory, FakeArtifactBytes, FakeDeleteOutcome, next_artifact_generation,
};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

const RETENTION: &str = "2026-07-30T00:10:00Z";
const AFTER_RETENTION: &str = "2026-07-30T00:20:00Z";
const GRACE: &str = "2026-07-30T00:15:00Z";

fn marker(hex: char) -> ContentDigest {
    ContentDigest::from_sha256(std::iter::repeat_n(hex, 64).collect::<String>()).expect("digest")
}

fn content_digest(bytes: &[u8]) -> ContentDigest {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("writing to a string cannot fail");
    }
    ContentDigest::from_sha256(text).expect("content digest")
}

fn identity(project: &str, bytes: &[u8], generation: u64) -> ArtifactObjectIdentity {
    ArtifactObjectIdentity::new(
        ArtifactObjectKey::new(
            ProjectId::new(project).expect("project"),
            content_digest(bytes),
        ),
        ArtifactGeneration::new(generation).expect("generation"),
    )
}

fn reference_pair(
    object: &ArtifactObjectIdentity,
    task_id: &TaskId,
    reference_id: &str,
    owner_record_id: &str,
    revision: u64,
    action: ArtifactReferenceAuthorityAction,
    runtime: RuntimeKind,
) -> ArtifactReferenceAuthorityPair {
    let binding = ArtifactReferenceAuthorityBinding::new(
        lattice_contracts::ArtifactAuthorityOwnerKind::TaskLedger,
        runtime,
        owner_record_id,
        ArtifactRevision::new(revision).expect("revision"),
        ArtifactAuthorityStatus::Available,
        action,
        object.key().project_id().clone(),
        task_id.clone(),
        object.clone(),
        reference_id,
        marker('1'),
    )
    .expect("reference binding");
    let receipt =
        ArtifactReferenceAuthorityReceipt::new(1, binding, marker('2')).expect("reference receipt");
    ArtifactReferenceAuthorityPair::new(receipt.clone(), receipt.head()).expect("reference pair")
}

fn read_pair(
    object: &ArtifactObjectIdentity,
    task_id: &TaskId,
    read_claim_id: &str,
    owner_record_id: &str,
    revision: u64,
    action: ArtifactReadAuthorityAction,
) -> ArtifactReadAuthorityPair {
    read_pair_with_owner(
        lattice_contracts::ArtifactAuthorityOwnerKind::TaskLedger,
        object,
        task_id,
        read_claim_id,
        owner_record_id,
        revision,
        action,
    )
}

#[allow(clippy::too_many_arguments)]
fn read_pair_with_owner(
    owner_kind: lattice_contracts::ArtifactAuthorityOwnerKind,
    object: &ArtifactObjectIdentity,
    task_id: &TaskId,
    read_claim_id: &str,
    owner_record_id: &str,
    revision: u64,
    action: ArtifactReadAuthorityAction,
) -> ArtifactReadAuthorityPair {
    let binding = ArtifactReadAuthorityBinding::new(
        owner_kind,
        RuntimeKind::Fake,
        owner_record_id,
        ArtifactRevision::new(revision).expect("revision"),
        ArtifactAuthorityStatus::Available,
        action,
        object.key().project_id().clone(),
        task_id.clone(),
        object.clone(),
        read_claim_id,
        marker('3'),
    )
    .expect("read binding");
    let receipt = ArtifactReadAuthorityReceipt::new(1, binding, marker('4')).expect("read receipt");
    ArtifactReadAuthorityPair::new(receipt.clone(), receipt.head()).expect("read pair")
}

#[allow(clippy::too_many_arguments)]
fn manifest(
    object: &ArtifactObjectIdentity,
    byte_length: u64,
    task_id_text: &str,
    reference_id: &str,
    owner_record_id: &str,
    action: ArtifactReferenceAuthorityAction,
    retention_until: &str,
    limits: ArtifactStoreLimits,
    source_runtime: RuntimeKind,
) -> ArtifactReferenceManifest {
    manifest_with_bundle_total(
        object,
        byte_length,
        byte_length,
        task_id_text,
        reference_id,
        owner_record_id,
        action,
        retention_until,
        limits,
        source_runtime,
    )
}

#[allow(clippy::too_many_arguments)]
fn manifest_with_bundle_total(
    object: &ArtifactObjectIdentity,
    byte_length: u64,
    bundle_total_declared_bytes: u64,
    task_id_text: &str,
    reference_id: &str,
    owner_record_id: &str,
    action: ArtifactReferenceAuthorityAction,
    retention_until: &str,
    limits: ArtifactStoreLimits,
    source_runtime: RuntimeKind,
) -> ArtifactReferenceManifest {
    let task_id = TaskId::new(task_id_text).expect("task id");
    let binding = SubjectBinding::new(
        object.key().project_id().clone(),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        task_id.clone(),
        "1",
        marker('5'),
    )
    .expect("subject");
    let authority = reference_pair(
        object,
        &task_id,
        reference_id,
        owner_record_id,
        1,
        action,
        RuntimeKind::Fake,
    );
    let provenance = ArtifactProvenance::new(
        "test-producer",
        "1.0",
        source_runtime,
        marker('6'),
        "test-adapter",
        "1.0",
        marker('7'),
        "invocation-1",
        "correlation-1",
        "run-1",
        ArtifactCounter::new(1).expect("sequence"),
        "2026-07-30T00:00:00Z",
        object.key().content_digest().clone(),
        "capability-1",
        marker('8'),
        marker('9'),
        marker('a'),
        marker('b'),
        marker('c'),
        "effect-claim-1",
        marker('d'),
        "daemon-1",
        DaemonEpoch::new(1).expect("epoch"),
        RuntimeAdmissionMode::Active,
        marker('e'),
        marker('f'),
        limits.limit_snapshot_digest().expect("limits digest"),
    )
    .expect("provenance");
    let bundle = ArtifactBundleBounds::new(
        ArtifactCounter::new(u64::from(byte_length > 0)).expect("entries"),
        ArtifactCounter::new(u64::from(byte_length > 0)).expect("depth"),
        ArtifactByteLength::new(bundle_total_declared_bytes).expect("bundle bytes"),
    )
    .expect("bundle");
    let build = |manifest_digest| {
        ArtifactReferenceManifest::new(
            binding.clone(),
            AttemptId::new("attempt-1").expect("attempt"),
            RequestId::new(format!("request-{reference_id}")).expect("request"),
            reference_id,
            object.clone(),
            ArtifactByteLength::new(byte_length).expect("length"),
            "application/octet-stream",
            "lattice.test.payload",
            "1.0",
            Some(bundle),
            provenance.clone(),
            authority.clone(),
            ArtifactPurpose::TaskOutput,
            retention_until,
            manifest_digest,
        )
        .expect("manifest")
    };
    let provisional = build(marker('1'));
    build(artifact_manifest_digest(&provisional).expect("manifest digest"))
}

fn install_manifest_authority(
    directory: &mut FakeArtifactAuthorityDirectory,
    manifest: &ArtifactReferenceManifest,
) {
    directory.install_reference_pair(manifest.creation_authority());
}

fn release_pair(
    object: &ArtifactObjectIdentity,
    task_id: &str,
    reference_id: &str,
    owner_record_id: &str,
) -> ArtifactReferenceAuthorityPair {
    reference_pair(
        object,
        &TaskId::new(task_id).expect("task"),
        reference_id,
        owner_record_id,
        2,
        ArtifactReferenceAuthorityAction::ReleaseReference,
        RuntimeKind::Fake,
    )
}

fn sweep_pair(
    store: &FakeArtifactStore,
    object: &ArtifactObjectIdentity,
    plan: &semantics::ArtifactDeletePlan,
    owner_record_id: &str,
) -> ArtifactSweepAuthorityPair {
    let head = store.object_head(object).expect("object head");
    let binding = ArtifactSweepAuthorityBinding::new(
        RuntimeKind::Fake,
        owner_record_id,
        ArtifactRevision::new(1).expect("revision"),
        ArtifactAuthorityStatus::Available,
        ArtifactSweepAuthorityAction::ClaimDelete,
        object.clone(),
        head.active_reference_set_digest().clone(),
        head.active_read_set_digest().clone(),
        head.project_quota_projection_digest().clone(),
        plan.observed_at(),
        plan.grace_until(),
        marker('8'),
        "daemon-1",
        DaemonEpoch::new(1).expect("epoch"),
        RuntimeAdmissionMode::Active,
        marker('9'),
    )
    .expect("sweep binding");
    let receipt =
        ArtifactSweepAuthorityReceipt::new(1, binding, marker('a')).expect("sweep receipt");
    ArtifactSweepAuthorityPair::new(receipt.clone(), receipt.head()).expect("sweep pair")
}

fn publish_one(
    store: &mut FakeArtifactStore,
    bytes_backend: &mut FakeArtifactBytes,
    directory: &mut FakeArtifactAuthorityDirectory,
    project: &str,
    bytes: &[u8],
    generation: u64,
    reference_id: &str,
) -> (ArtifactObjectIdentity, ArtifactReferenceManifest) {
    let object = identity(project, bytes, generation);
    let manifest = manifest(
        &object,
        bytes.len() as u64,
        "task-1",
        reference_id,
        &format!("owner-{reference_id}"),
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RETENTION,
        store.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(directory, &manifest);
    store
        .publish(bytes_backend, manifest.clone(), bytes, directory)
        .expect("publish");
    (object, manifest)
}

#[test]
fn lifecycle_fake_and_byte_backend_are_separate_and_redacted() {
    let mut store = FakeArtifactStore::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let secret = b"do-not-print-this-secret";
    let (object, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p1",
        secret,
        1,
        "ref-1",
    );

    assert_eq!(store.object_count(), 1);
    assert_eq!(bytes_backend.object_count(), 1);
    assert!(!format!("{bytes_backend:?}").contains("do-not-print"));
    assert!(!format!("{store:?}").contains("do-not-print"));
    assert_eq!(
        store.current_head(&object).expect("head").producer_id(),
        ARTIFACT_STORE_PRODUCER_ID
    );
    assert_eq!(
        store
            .current_head(&object)
            .expect("head")
            .producer_version(),
        ARTIFACT_STORE_PRODUCER_VERSION
    );
}

#[test]
fn empty_nonempty_dedupe_and_cross_project_isolation_are_exact() {
    let mut store = FakeArtifactStore::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let (empty_p1, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p1",
        b"",
        1,
        "empty-1",
    );
    assert_eq!(
        store
            .object_head(&empty_p1)
            .expect("head")
            .byte_length()
            .get(),
        0
    );

    let dedup_manifest = manifest(
        &empty_p1,
        0,
        "task-1",
        "empty-2",
        "owner-empty-2",
        ArtifactReferenceAuthorityAction::AddReference,
        RETENTION,
        store.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &dedup_manifest);
    store
        .publish(&mut bytes_backend, dedup_manifest, b"", &directory)
        .expect("same-project dedupe");
    assert_eq!(bytes_backend.object_count(), 1);
    assert_eq!(
        store
            .object_head(&empty_p1)
            .expect("head")
            .active_reference_count()
            .get(),
        2
    );

    let (empty_p2, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p2",
        b"",
        1,
        "empty-1",
    );
    let (nonempty_p1, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p1",
        b"x",
        1,
        "nonempty-1",
    );
    assert_ne!(empty_p1.key().project_id(), empty_p2.key().project_id());
    assert_ne!(empty_p1.key(), empty_p2.key());
    assert_ne!(empty_p1.key(), nonempty_p1.key());
    assert_eq!(bytes_backend.object_count(), 3);
}

#[test]
fn digest_length_manifest_limit_and_runtime_denials_are_atomic() {
    let limits = ArtifactStoreLimits::new(4, 65_536, 10, 10, 100, 10, 10).expect("limits");
    let mut store = FakeArtifactStore::new(limits);
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object = identity("p1", b"four", 1);
    let exact = manifest(
        &object,
        4,
        "task-1",
        "ref-exact",
        "owner-exact",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RETENTION,
        limits,
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &exact);
    store
        .publish(&mut bytes_backend, exact, b"four", &directory)
        .expect("exact configured limit");

    for (candidate, raw, expected) in [
        (
            manifest(
                &identity("p1", b"five!", 1),
                5,
                "task-1",
                "ref-over",
                "owner-over",
                ArtifactReferenceAuthorityAction::PublishInitialReference,
                RETENTION,
                limits,
                RuntimeKind::Fake,
            ),
            b"five!".as_slice(),
            "ARTIFACT_LIMIT_EXCEEDED",
        ),
        (
            manifest(
                &identity("p1", b"abc", 1),
                2,
                "task-1",
                "ref-length",
                "owner-length",
                ArtifactReferenceAuthorityAction::PublishInitialReference,
                RETENTION,
                limits,
                RuntimeKind::Fake,
            ),
            b"abc".as_slice(),
            "ARTIFACT_LENGTH_MISMATCH",
        ),
        (
            manifest(
                &identity("p1", b"other", 1),
                4,
                "task-1",
                "ref-digest",
                "owner-digest",
                ArtifactReferenceAuthorityAction::PublishInitialReference,
                RETENTION,
                limits,
                RuntimeKind::Fake,
            ),
            b"four".as_slice(),
            "ARTIFACT_DIGEST_MISMATCH",
        ),
        (
            manifest(
                &identity("p1", b"live", 1),
                4,
                "task-1",
                "ref-live",
                "owner-live",
                ArtifactReferenceAuthorityAction::PublishInitialReference,
                RETENTION,
                limits,
                RuntimeKind::Live,
            ),
            b"live".as_slice(),
            "ARTIFACT_AUTHORITY_RUNTIME_MISMATCH",
        ),
    ] {
        install_manifest_authority(&mut directory, &candidate);
        let before_store = store.clone();
        let before_bytes = bytes_backend.clone();
        let error = store
            .publish(&mut bytes_backend, candidate, raw, &directory)
            .expect_err("denied");
        assert_eq!(error.code(), expected);
        assert_eq!(store, before_store);
        assert_eq!(bytes_backend, before_bytes);
    }
}

#[test]
fn lowered_field_limit_binds_manifest_and_provenance_text() {
    let limits = ArtifactStoreLimits::hard_maximums()
        .tighten(ArtifactLimitKind::FieldBytes, 20)
        .expect("field limit");
    let mut store = FakeArtifactStore::new(limits);
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object = identity("p1", b"x", 1);
    let candidate = manifest(
        &object,
        1,
        "task-1",
        "ref-1",
        "owner-1",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RETENTION,
        limits,
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &candidate);
    let before = store.clone();
    assert_eq!(
        store
            .publish(&mut bytes_backend, candidate, b"x", &directory)
            .expect_err("media/schema field exceeds lowered limit")
            .code(),
        "ARTIFACT_LIMIT_EXCEEDED"
    );
    assert_eq!(store, before);
    assert_eq!(bytes_backend.object_count(), 0);
}

#[test]
fn bundle_total_may_exceed_manifest_bytes_and_consumes_all_byte_scopes() {
    let raw = b"small";
    let accounted_bytes = 25;
    let exact_limits = ArtifactStoreLimits::hard_maximums()
        .tighten(ArtifactLimitKind::ActiveBytesPerTask, accounted_bytes)
        .expect("task bytes")
        .tighten(ArtifactLimitKind::UniqueBytesPerProject, accounted_bytes)
        .expect("project bytes")
        .tighten(ArtifactLimitKind::UniqueBytesPerStore, accounted_bytes)
        .expect("store bytes");
    let mut store = FakeArtifactStore::new(exact_limits);
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object = identity("p1", raw, 1);
    let candidate = manifest_with_bundle_total(
        &object,
        raw.len() as u64,
        20,
        "task-1",
        "ref-1",
        "owner-1",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RETENTION,
        exact_limits,
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &candidate);
    store
        .publish(&mut bytes_backend, candidate, raw, &directory)
        .expect("exact raw plus declared bundle bytes");

    for kind in [
        ArtifactLimitKind::ActiveBytesPerTask,
        ArtifactLimitKind::UniqueBytesPerProject,
        ArtifactLimitKind::UniqueBytesPerStore,
    ] {
        let limits = ArtifactStoreLimits::hard_maximums()
            .tighten(kind, accounted_bytes - 1)
            .expect("lower aggregate");
        let mut denied_store = FakeArtifactStore::new(limits);
        let mut denied_bytes = FakeArtifactBytes::default();
        let mut denied_directory = FakeArtifactAuthorityDirectory::default();
        let denied_object = identity("p1", raw, 1);
        let denied = manifest_with_bundle_total(
            &denied_object,
            raw.len() as u64,
            20,
            "task-1",
            "ref-1",
            "owner-1",
            ArtifactReferenceAuthorityAction::PublishInitialReference,
            RETENTION,
            limits,
            RuntimeKind::Fake,
        );
        install_manifest_authority(&mut denied_directory, &denied);
        let before = denied_store.clone();
        assert_eq!(
            denied_store
                .publish(&mut denied_bytes, denied, raw, &denied_directory)
                .expect_err("bundle bytes exceed exact aggregate")
                .code(),
            "ARTIFACT_LIMIT_EXCEEDED"
        );
        assert_eq!(denied_store, before);
        assert_eq!(denied_bytes.object_count(), 0);
    }
}

#[test]
fn equal_task_ids_in_different_projects_have_independent_quota_scope() {
    let limits = ArtifactStoreLimits::hard_maximums()
        .tighten(ArtifactLimitKind::ReferencesPerTask, 1)
        .expect("reference limit")
        .tighten(ArtifactLimitKind::ObjectsPerTask, 1)
        .expect("object limit");
    let mut store = FakeArtifactStore::new(limits);
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();

    for (project, raw) in [("p1", b"a".as_slice()), ("p2", b"b".as_slice())] {
        let object = identity(project, raw, 1);
        let candidate = manifest(
            &object,
            1,
            "same-task",
            "ref-1",
            &format!("owner-{project}"),
            ArtifactReferenceAuthorityAction::PublishInitialReference,
            RETENTION,
            limits,
            RuntimeKind::Fake,
        );
        install_manifest_authority(&mut directory, &candidate);
        store
            .publish(&mut bytes_backend, candidate, raw, &directory)
            .expect("project-task quota is isolated");
    }
    assert_eq!(store.object_count(), 2);
}

#[test]
fn independent_authority_currentness_and_terminal_reference_ids_are_enforced() {
    let mut store = FakeArtifactStore::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let object_a = identity("p1", b"a", 1);
    let missing = manifest(
        &object_a,
        1,
        "task-1",
        "shared-ref",
        "owner-shared",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RETENTION,
        store.limits(),
        RuntimeKind::Fake,
    );
    assert_eq!(
        store
            .publish(&mut bytes_backend, missing.clone(), b"a", &directory)
            .expect_err("independent head is required")
            .code(),
        "ARTIFACT_AUTHORITY_MISSING"
    );
    let stale_replacement = reference_pair(
        &object_a,
        &TaskId::new("task-1").expect("task"),
        "shared-ref",
        "owner-shared",
        2,
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RuntimeKind::Fake,
    );
    directory.install_reference_pair(&stale_replacement);
    assert_eq!(
        store
            .publish(&mut bytes_backend, missing.clone(), b"a", &directory)
            .expect_err("historical receipt is stale")
            .code(),
        "ARTIFACT_AUTHORITY_STALE"
    );
    install_manifest_authority(&mut directory, &missing);
    store
        .publish(&mut bytes_backend, missing, b"a", &directory)
        .expect("publish A");

    let object_b = identity("p1", b"b", 1);
    let same_id_other_object = manifest(
        &object_b,
        1,
        "task-1",
        "shared-ref",
        "owner-other",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RETENTION,
        store.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &same_id_other_object);
    store
        .publish(&mut bytes_backend, same_id_other_object, b"b", &directory)
        .expect("reference ID is object scoped");

    let release = release_pair(&object_a, "task-1", "shared-ref", "owner-release");
    directory.install_reference_pair(&release);
    store
        .release_reference(&object_a, "shared-ref", release, &directory)
        .expect("release");
    let rebound = manifest(
        &object_a,
        1,
        "task-1",
        "shared-ref",
        "owner-rebind",
        ArtifactReferenceAuthorityAction::AddReference,
        RETENTION,
        store.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &rebound);
    assert_eq!(
        store
            .add_reference(rebound, &directory)
            .expect_err("terminal ID")
            .code(),
        "ARTIFACT_REFERENCE_TERMINAL"
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn read_claims_expire_suspect_verify_bytes_and_require_exact_reconciliation() {
    let mut store = FakeArtifactStore::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let (object, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p1",
        b"read-me",
        1,
        "ref-1",
    );
    let acquire = read_pair(
        &object,
        &TaskId::new("task-1").expect("task"),
        "read-1",
        "read-owner-1",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    directory.install_read_pair(&acquire);
    store
        .acquire_read(
            &object,
            "holder-1",
            "2026-07-30T00:00:00Z",
            "2026-07-30T00:15:00Z",
            acquire,
            &directory,
        )
        .expect("15-minute claim");
    assert_eq!(
        store
            .read_verified(&bytes_backend, &object, "read-1", "2026-07-30T00:01:00Z",)
            .expect("verified read"),
        b"read-me"
    );

    let metadata_before_fault = store.clone();
    bytes_backend.replace_for_test(&object, b"corrupt".to_vec());
    assert_eq!(
        store
            .read_verified(&bytes_backend, &object, "read-1", "2026-07-30T00:01:00Z",)
            .expect_err("corrupt")
            .code(),
        "ARTIFACT_BYTES_CORRUPT"
    );
    assert_eq!(store, metadata_before_fault);
    bytes_backend.remove_for_test(&object);
    assert_eq!(
        store
            .read_verified(&bytes_backend, &object, "read-1", "2026-07-30T00:01:00Z",)
            .expect_err("missing")
            .code(),
        "ARTIFACT_BYTES_MISSING"
    );
    assert_eq!(store, metadata_before_fault);
    bytes_backend.replace_for_test(&object, b"read-me".to_vec());

    store
        .mark_read_expired_suspect(&object, "read-1", "2026-07-30T00:15:00Z")
        .expect("suspect");
    let suspect_head = store.current_head(&object).expect("head");
    assert_eq!(
        suspect_head.read().expect("read").status().as_str(),
        "EXPIRED_SUSPECT"
    );
    assert_eq!(
        store
            .plan_delete(&object, &suspect_head, AFTER_RETENTION, GRACE)
            .expect_err("suspect blocks delete")
            .code(),
        "ARTIFACT_DELETE_BLOCKED"
    );

    let release = read_pair(
        &object,
        &TaskId::new("task-1").expect("task"),
        "read-1",
        "read-owner-release",
        2,
        ArtifactReadAuthorityAction::ReleaseRead,
    );
    directory.install_read_pair(&release);
    let closure_binding = ArtifactReadClosureEvidenceBinding::new(
        RuntimeKind::Fake,
        "closure-record-1",
        ArtifactRevision::new(1).expect("revision"),
        ArtifactAuthorityStatus::Available,
        ArtifactReadClosureEvidenceKind::HandleClosed,
        object.clone(),
        TaskId::new("task-1").expect("task"),
        "read-1",
        "holder-1",
        "daemon-1",
        DaemonEpoch::new(1).expect("epoch"),
        "2026-07-30T00:16:00Z",
        marker('a'),
    )
    .expect("closure binding");
    let closure_receipt = ArtifactReadClosureEvidenceReceipt::new(1, closure_binding, marker('b'))
        .expect("closure receipt");
    let evidence =
        ArtifactReadClosureEvidencePair::new(closure_receipt.clone(), closure_receipt.head())
            .expect("closure pair");
    let live_binding = ArtifactReadClosureEvidenceBinding::new(
        RuntimeKind::Live,
        "closure-record-live",
        ArtifactRevision::new(1).expect("revision"),
        ArtifactAuthorityStatus::Available,
        ArtifactReadClosureEvidenceKind::HolderDeath,
        object.clone(),
        TaskId::new("task-1").expect("task"),
        "read-1",
        "holder-1",
        "daemon-1",
        DaemonEpoch::new(1).expect("epoch"),
        "2026-07-30T00:16:00Z",
        marker('c'),
    )
    .expect("live binding representation");
    let live_receipt = ArtifactReadClosureEvidenceReceipt::new(1, live_binding, marker('d'))
        .expect("live receipt");
    let live_evidence =
        ArtifactReadClosureEvidencePair::new(live_receipt.clone(), live_receipt.head())
            .expect("live pair");
    let before_live = store.clone();
    assert_eq!(
        store
            .reconcile_expired_read(
                &object,
                "read-1",
                release.clone(),
                &directory,
                &live_evidence,
            )
            .expect_err("TASK-016 fake cannot accept live closure evidence")
            .code(),
        "ARTIFACT_AUTHORITY_RUNTIME_MISMATCH"
    );
    assert_eq!(store, before_live);
    let before_missing_head = store.clone();
    assert_eq!(
        store
            .reconcile_expired_read(&object, "read-1", release.clone(), &directory, &evidence,)
            .expect_err("independent closure verifier head is required")
            .code(),
        "ARTIFACT_AUTHORITY_MISSING"
    );
    assert_eq!(store, before_missing_head);
    let advanced_binding = ArtifactReadClosureEvidenceBinding::new(
        RuntimeKind::Fake,
        "closure-record-1",
        ArtifactRevision::new(2).expect("revision"),
        ArtifactAuthorityStatus::Available,
        ArtifactReadClosureEvidenceKind::HandleClosed,
        object.clone(),
        TaskId::new("task-1").expect("task"),
        "read-1",
        "holder-1",
        "daemon-1",
        DaemonEpoch::new(1).expect("epoch"),
        "2026-07-30T00:17:00Z",
        marker('e'),
    )
    .expect("advanced closure binding");
    let advanced_receipt =
        ArtifactReadClosureEvidenceReceipt::new(1, advanced_binding, marker('f'))
            .expect("advanced closure receipt");
    let advanced =
        ArtifactReadClosureEvidencePair::new(advanced_receipt.clone(), advanced_receipt.head())
            .expect("advanced closure pair");
    directory.install_read_closure_pair(&advanced);
    let before_stale = store.clone();
    assert_eq!(
        store
            .reconcile_expired_read(&object, "read-1", release.clone(), &directory, &evidence,)
            .expect_err("historical closure evidence is stale")
            .code(),
        "ARTIFACT_AUTHORITY_STALE"
    );
    assert_eq!(store, before_stale);
    directory.install_read_closure_pair(&evidence);
    store
        .reconcile_expired_read(&object, "read-1", release, &directory, &evidence)
        .expect("verified closure");
    assert_eq!(
        store
            .object_head(&object)
            .expect("head")
            .active_read_count()
            .get(),
        0
    );
}

#[test]
fn read_release_rejects_cross_task_owner_scope_without_mutation() {
    let mut store = FakeArtifactStore::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let (object, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p1",
        b"task-scoped-read",
        1,
        "ref-1",
    );
    let acquire = read_pair(
        &object,
        &TaskId::new("task-1").expect("task"),
        "read-1",
        "read-owner-1",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    directory.install_read_pair(&acquire);
    store
        .acquire_read(
            &object,
            "holder-1",
            "2026-07-30T00:00:00Z",
            "2026-07-30T00:15:00Z",
            acquire,
            &directory,
        )
        .expect("acquire");

    let cross_task_release = read_pair(
        &object,
        &TaskId::new("task-2").expect("task"),
        "read-1",
        "read-owner-cross-task",
        1,
        ArtifactReadAuthorityAction::ReleaseRead,
    );
    directory.install_read_pair(&cross_task_release);
    let before = store.clone();
    assert_eq!(
        store
            .release_read(&object, "read-1", cross_task_release, &directory)
            .expect_err("another task cannot release this read")
            .code(),
        "ARTIFACT_AUTHORITY_SCOPE_MISMATCH"
    );
    assert_eq!(store, before);

    let cross_owner_release = read_pair_with_owner(
        lattice_contracts::ArtifactAuthorityOwnerKind::ReviewRuntime,
        &object,
        &TaskId::new("task-1").expect("task"),
        "read-1",
        "read-owner-cross-owner",
        1,
        ArtifactReadAuthorityAction::ReleaseRead,
    );
    directory.install_read_pair(&cross_owner_release);
    assert_eq!(
        store
            .release_read(&object, "read-1", cross_owner_release, &directory)
            .expect_err("another owner family cannot release this read")
            .code(),
        "ARTIFACT_AUTHORITY_SCOPE_MISMATCH"
    );
    assert_eq!(store, before);
}

#[test]
fn verified_read_at_expiry_atomically_marks_suspect_and_denies_bytes() {
    let mut store = FakeArtifactStore::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let (object, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p1",
        b"expiring-read",
        1,
        "ref-1",
    );
    let acquire = read_pair(
        &object,
        &TaskId::new("task-1").expect("task"),
        "read-1",
        "read-owner-1",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    directory.install_read_pair(&acquire);
    store
        .acquire_read(
            &object,
            "holder-1",
            "2026-07-30T00:00:00Z",
            "2026-07-30T00:15:00Z",
            acquire,
            &directory,
        )
        .expect("acquire");

    let before_noncanonical = store.clone();
    assert_eq!(
        store
            .read_verified(
                &bytes_backend,
                &object,
                "read-1",
                "2026-07-30T00:01:00+00:00",
            )
            .expect_err("observation must use canonical UTC seconds")
            .code(),
        "ARTIFACT_CANONICALIZATION_FAILED"
    );
    assert_eq!(store, before_noncanonical);
    assert_eq!(
        store
            .read_verified(&bytes_backend, &object, "read-1", "2026-07-30T00:15:00Z",)
            .expect_err("expiry must deny and mark suspect")
            .code(),
        "ARTIFACT_READ_EXPIRED_SUSPECT"
    );
    assert_eq!(
        store
            .current_head(&object)
            .expect("current")
            .read()
            .expect("read")
            .status(),
        lattice_contracts::ArtifactReadStatus::ExpiredSuspect
    );
}

#[test]
#[allow(clippy::too_many_lines)]
fn delete_claim_is_exact_blocks_use_and_unknown_requires_reconciliation() {
    let mut store = FakeArtifactStore::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let (object, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p1",
        b"delete-me",
        1,
        "ref-1",
    );
    let release = release_pair(&object, "task-1", "ref-1", "owner-release");
    directory.install_reference_pair(&release);
    store
        .release_reference(&object, "ref-1", release, &directory)
        .expect("release");
    let expected_head = store.current_head(&object).expect("head");
    assert_eq!(
        store
            .plan_delete(&object, &expected_head, "2026-07-30T00:05:00Z", GRACE,)
            .expect_err("retention")
            .code(),
        "ARTIFACT_RETENTION_ACTIVE"
    );
    let plan = store
        .plan_delete(&object, &expected_head, AFTER_RETENTION, GRACE)
        .expect("plan");
    assert_eq!(
        store.current_head(&object).expect("unchanged"),
        expected_head
    );
    let sweep = sweep_pair(&store, &object, &plan, "sweep-1");
    directory.install_sweep_pair(&sweep);
    let first = store
        .claim_delete(&plan, &sweep, &directory)
        .expect("claim");
    let retry = store
        .claim_delete(&plan, &sweep, &directory)
        .expect("exact retry");
    assert_eq!(retry, first);
    assert_eq!(
        store.object_head(&object).expect("claimed").availability(),
        ArtifactAvailability::DeleteClaimed
    );

    let add = manifest(
        &object,
        9,
        "task-1",
        "ref-2",
        "owner-ref-2",
        ArtifactReferenceAuthorityAction::AddReference,
        RETENTION,
        store.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &add);
    assert_eq!(
        store
            .add_reference(add, &directory)
            .expect_err("claim blocks retain")
            .code(),
        "ARTIFACT_OBJECT_UNAVAILABLE"
    );

    let blocked_read = read_pair(
        &object,
        &TaskId::new("task-1").expect("task"),
        "blocked-read",
        "blocked-read-owner",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    directory.install_read_pair(&blocked_read);
    assert_eq!(
        store
            .acquire_read(
                &object,
                "holder-blocked",
                "2026-07-30T00:20:00Z",
                "2026-07-30T00:21:00Z",
                blocked_read,
                &directory,
            )
            .expect_err("claim blocks reads")
            .code(),
        "ARTIFACT_OBJECT_UNAVAILABLE"
    );

    store
        .apply_delete_outcome(
            &mut bytes_backend,
            &object,
            plan.claim_token(),
            FakeDeleteOutcome::VerifiedNoEffect,
        )
        .expect("verified no effect");
    assert_eq!(
        store
            .object_head(&object)
            .expect("no effect")
            .availability(),
        ArtifactAvailability::Available
    );
    assert_eq!(bytes_backend.object_count(), 1);

    let retry_head = store.current_head(&object).expect("no-effect head");
    let retry_plan = store
        .plan_delete(&object, &retry_head, AFTER_RETENTION, GRACE)
        .expect("retry plan");
    let retry_sweep = sweep_pair(&store, &object, &retry_plan, "sweep-2");
    directory.install_sweep_pair(&retry_sweep);
    store
        .claim_delete(&retry_plan, &retry_sweep, &directory)
        .expect("retry claim");
    store
        .apply_delete_outcome(
            &mut bytes_backend,
            &object,
            retry_plan.claim_token(),
            FakeDeleteOutcome::Unknown,
        )
        .expect("unknown");
    assert_eq!(
        store.object_head(&object).expect("unknown").availability(),
        ArtifactAvailability::ReconciliationRequired
    );
    assert_eq!(bytes_backend.object_count(), 1);
    store
        .reconcile_delete(
            &bytes_backend,
            &object,
            retry_plan.claim_token(),
            ArtifactReconciliationResult::VerifiedAvailable,
        )
        .expect("verified present");
    assert_eq!(
        store
            .object_head(&object)
            .expect("available")
            .availability(),
        ArtifactAvailability::Available
    );
}

#[test]
fn verified_deletion_releases_bytes_and_reintroduction_requires_higher_generation() {
    let mut store = FakeArtifactStore::default();
    let mut bytes_backend = FakeArtifactBytes::default();
    let mut directory = FakeArtifactAuthorityDirectory::default();
    let raw = b"generation";
    let (generation_one, _) = publish_one(
        &mut store,
        &mut bytes_backend,
        &mut directory,
        "p1",
        raw,
        1,
        "ref-1",
    );
    let release = release_pair(&generation_one, "task-1", "ref-1", "owner-release");
    directory.install_reference_pair(&release);
    store
        .release_reference(&generation_one, "ref-1", release, &directory)
        .expect("release");
    let old_head = store.current_head(&generation_one).expect("head");
    let old_plan = store
        .plan_delete(&generation_one, &old_head, AFTER_RETENTION, GRACE)
        .expect("plan");
    let old_sweep = sweep_pair(&store, &generation_one, &old_plan, "sweep-1");
    directory.install_sweep_pair(&old_sweep);
    store
        .claim_delete(&old_plan, &old_sweep, &directory)
        .expect("claim");
    store
        .apply_delete_outcome(
            &mut bytes_backend,
            &generation_one,
            old_plan.claim_token(),
            FakeDeleteOutcome::VerifiedDeleted,
        )
        .expect("deleted");
    assert_eq!(bytes_backend.object_count(), 0);

    let generation_two = identity("p1", raw, 2);
    let reintroduced = manifest(
        &generation_two,
        raw.len() as u64,
        "task-1",
        "ref-2",
        "owner-ref-2",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        RETENTION,
        store.limits(),
        RuntimeKind::Fake,
    );
    install_manifest_authority(&mut directory, &reintroduced);
    store
        .publish(&mut bytes_backend, reintroduced, raw, &directory)
        .expect("higher generation");
    assert_eq!(
        store
            .current_head(&generation_one)
            .expect_err("old generation")
            .code(),
        "ARTIFACT_GENERATION_MISMATCH"
    );
    assert_eq!(
        store
            .claim_delete(&old_plan, &old_sweep, &directory)
            .expect_err("old evidence rejected")
            .code(),
        "ARTIFACT_GENERATION_MISMATCH"
    );
}

#[test]
fn generation_allocation_fails_closed_at_signed_bigint_maximum() {
    let maximum = ArtifactGeneration::new(9_223_372_036_854_775_807).expect("maximum");
    assert_eq!(
        next_artifact_generation(Some(maximum))
            .expect_err("must not wrap or saturate")
            .code(),
        "ARTIFACT_COUNTER_EXHAUSTED"
    );
    assert_eq!(next_artifact_generation(None).expect("first").get(), 1);
}

#[test]
fn reference_and_quota_digests_are_independent_of_hashmap_insertion_order() {
    fn build(order: [&[u8]; 2]) -> (FakeArtifactStore, ArtifactObjectIdentity) {
        let mut store = FakeArtifactStore::default();
        let mut bytes_backend = FakeArtifactBytes::default();
        let mut directory = FakeArtifactAuthorityDirectory::default();
        for raw in order {
            let reference_id = if raw == b"alpha" { "ref-a" } else { "ref-b" };
            publish_one(
                &mut store,
                &mut bytes_backend,
                &mut directory,
                "p1",
                raw,
                1,
                reference_id,
            );
        }
        (store, identity("p1", b"alpha", 1))
    }

    let (forward, object) = build([b"alpha".as_slice(), b"beta".as_slice()]);
    let (reverse, _) = build([b"beta".as_slice(), b"alpha".as_slice()]);
    let forward_head = forward.object_head(&object).expect("forward");
    let reverse_head = reverse.object_head(&object).expect("reverse");
    assert_eq!(
        forward_head.active_reference_set_digest(),
        reverse_head.active_reference_set_digest()
    );
    assert_eq!(
        forward_head.task_quota_projection_digest(),
        reverse_head.task_quota_projection_digest()
    );
    assert_eq!(
        forward_head.project_quota_projection_digest(),
        reverse_head.project_quota_projection_digest()
    );
    assert_eq!(
        forward_head.store_quota_projection_digest(),
        reverse_head.store_quota_projection_digest()
    );
    assert_eq!(
        forward_head.transition_digest(),
        reverse_head.transition_digest()
    );
}
