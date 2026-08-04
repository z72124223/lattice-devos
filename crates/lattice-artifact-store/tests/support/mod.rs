#![allow(dead_code)]

use std::fmt::Write as _;

use lattice_artifact_store::{
    ArtifactDeletePlan, ArtifactStoreLimits, FakeArtifactAuthorityDirectory,
    artifact_manifest_digest,
};
use lattice_contracts::{
    ArtifactAuthorityOwnerKind, ArtifactAuthorityStatus, ArtifactBundleBounds, ArtifactByteLength,
    ArtifactCounter, ArtifactGeneration, ArtifactObjectHead, ArtifactObjectIdentity,
    ArtifactObjectKey, ArtifactProvenance, ArtifactPurpose, ArtifactReadAuthorityAction,
    ArtifactReadAuthorityBinding, ArtifactReadAuthorityPair, ArtifactReadAuthorityReceipt,
    ArtifactReadClosureEvidenceBinding, ArtifactReadClosureEvidenceKind,
    ArtifactReadClosureEvidencePair, ArtifactReadClosureEvidenceReceipt,
    ArtifactReferenceAuthorityAction, ArtifactReferenceAuthorityBinding,
    ArtifactReferenceAuthorityPair, ArtifactReferenceAuthorityReceipt, ArtifactReferenceManifest,
    ArtifactRevision, ArtifactSweepAuthorityAction, ArtifactSweepAuthorityBinding,
    ArtifactSweepAuthorityPair, ArtifactSweepAuthorityReceipt, AttemptId, ContentDigest,
    DaemonEpoch, ProjectId, ProjectSnapshotId, RequestId, RuntimeAdmissionMode, RuntimeKind,
    SubjectBinding, TaskId,
};
use sha2::{Digest, Sha256};

pub(crate) const CREATED_AT: &str = "2026-07-30T00:00:00Z";
pub(crate) const RETENTION: &str = "2026-07-30T00:10:00Z";
pub(crate) const GRACE: &str = "2026-07-30T00:15:00Z";
pub(crate) const AFTER_RETENTION: &str = "2026-07-30T00:20:00Z";
pub(crate) const READ_ACQUIRED_AT: &str = "2026-07-30T00:01:00Z";
pub(crate) const READ_EXPIRES_AT: &str = "2026-07-30T00:11:00Z";
pub(crate) const READ_CLOSED_AT: &str = "2026-07-30T00:16:00Z";

pub(crate) fn marker(hex: char) -> ContentDigest {
    ContentDigest::from_sha256(std::iter::repeat_n(hex, 64).collect::<String>())
        .expect("fixture marker digest")
}

pub(crate) fn content_digest(bytes: &[u8]) -> ContentDigest {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut text, "{byte:02x}").expect("writing to a string cannot fail");
    }
    ContentDigest::from_sha256(text).expect("fixture content digest")
}

pub(crate) fn object_identity(
    project_id: &str,
    bytes: &[u8],
    generation: u64,
) -> ArtifactObjectIdentity {
    ArtifactObjectIdentity::new(
        ArtifactObjectKey::new(
            ProjectId::new(project_id).expect("fixture project id"),
            content_digest(bytes),
        ),
        ArtifactGeneration::new(generation).expect("fixture artifact generation"),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reference_pair(
    object: &ArtifactObjectIdentity,
    task_id: &TaskId,
    reference_id: &str,
    owner_record_id: &str,
    revision: u64,
    action: ArtifactReferenceAuthorityAction,
    runtime: RuntimeKind,
) -> ArtifactReferenceAuthorityPair {
    let binding = ArtifactReferenceAuthorityBinding::new(
        ArtifactAuthorityOwnerKind::TaskLedger,
        runtime,
        owner_record_id,
        ArtifactRevision::new(revision).expect("fixture reference authority revision"),
        ArtifactAuthorityStatus::Available,
        action,
        object.key().project_id().clone(),
        task_id.clone(),
        object.clone(),
        reference_id,
        marker('1'),
    )
    .expect("fixture reference authority binding");
    let receipt = ArtifactReferenceAuthorityReceipt::new(1, binding, marker('2'))
        .expect("fixture reference authority receipt");
    ArtifactReferenceAuthorityPair::new(receipt.clone(), receipt.head())
        .expect("fixture reference authority pair")
}

pub(crate) fn read_pair(
    object: &ArtifactObjectIdentity,
    task_id: &TaskId,
    read_claim_id: &str,
    owner_record_id: &str,
    revision: u64,
    action: ArtifactReadAuthorityAction,
) -> ArtifactReadAuthorityPair {
    read_pair_with_owner(
        ArtifactAuthorityOwnerKind::TaskLedger,
        object,
        task_id,
        read_claim_id,
        owner_record_id,
        revision,
        action,
        RuntimeKind::Fake,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_pair_with_owner(
    owner_kind: ArtifactAuthorityOwnerKind,
    object: &ArtifactObjectIdentity,
    task_id: &TaskId,
    read_claim_id: &str,
    owner_record_id: &str,
    revision: u64,
    action: ArtifactReadAuthorityAction,
    runtime: RuntimeKind,
) -> ArtifactReadAuthorityPair {
    let binding = ArtifactReadAuthorityBinding::new(
        owner_kind,
        runtime,
        owner_record_id,
        ArtifactRevision::new(revision).expect("fixture read authority revision"),
        ArtifactAuthorityStatus::Available,
        action,
        object.key().project_id().clone(),
        task_id.clone(),
        object.clone(),
        read_claim_id,
        marker('3'),
    )
    .expect("fixture read authority binding");
    let receipt = ArtifactReadAuthorityReceipt::new(1, binding, marker('4'))
        .expect("fixture read authority receipt");
    ArtifactReadAuthorityPair::new(receipt.clone(), receipt.head())
        .expect("fixture read authority pair")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_closure_pair(
    object: &ArtifactObjectIdentity,
    task_id: &TaskId,
    read_claim_id: &str,
    holder_id: &str,
    evidence_record_id: &str,
    revision: u64,
    kind: ArtifactReadClosureEvidenceKind,
    runtime: RuntimeKind,
    observed_at: &str,
) -> ArtifactReadClosureEvidencePair {
    let binding = ArtifactReadClosureEvidenceBinding::new(
        runtime,
        evidence_record_id,
        ArtifactRevision::new(revision).expect("fixture closure evidence revision"),
        ArtifactAuthorityStatus::Available,
        kind,
        object.clone(),
        task_id.clone(),
        read_claim_id,
        holder_id,
        "daemon-1",
        DaemonEpoch::new(1).expect("fixture daemon epoch"),
        observed_at,
        marker('a'),
    )
    .expect("fixture closure evidence binding");
    let receipt = ArtifactReadClosureEvidenceReceipt::new(1, binding, marker('b'))
        .expect("fixture closure evidence receipt");
    ArtifactReadClosureEvidencePair::new(receipt.clone(), receipt.head())
        .expect("fixture closure evidence pair")
}

pub(crate) fn sweep_pair(
    object_head: &ArtifactObjectHead,
    plan: &ArtifactDeletePlan,
    owner_record_id: &str,
) -> ArtifactSweepAuthorityPair {
    assert_eq!(
        object_head.object(),
        plan.object(),
        "fixture sweep head and plan must identify the same object"
    );
    let binding = ArtifactSweepAuthorityBinding::new(
        RuntimeKind::Fake,
        owner_record_id,
        ArtifactRevision::new(1).expect("fixture sweep authority revision"),
        ArtifactAuthorityStatus::Available,
        ArtifactSweepAuthorityAction::ClaimDelete,
        plan.object().clone(),
        object_head.active_reference_set_digest().clone(),
        object_head.active_read_set_digest().clone(),
        object_head.project_quota_projection_digest().clone(),
        plan.observed_at(),
        plan.grace_until(),
        marker('8'),
        "daemon-1",
        DaemonEpoch::new(1).expect("fixture daemon epoch"),
        RuntimeAdmissionMode::Active,
        marker('9'),
    )
    .expect("fixture sweep authority binding");
    let receipt = ArtifactSweepAuthorityReceipt::new(1, binding, marker('a'))
        .expect("fixture sweep authority receipt");
    ArtifactSweepAuthorityPair::new(receipt.clone(), receipt.head())
        .expect("fixture sweep authority pair")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn manifest(
    object: &ArtifactObjectIdentity,
    byte_length: u64,
    task_id: &str,
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
        task_id,
        reference_id,
        owner_record_id,
        action,
        retention_until,
        limits,
        source_runtime,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn manifest_with_bundle_total(
    object: &ArtifactObjectIdentity,
    byte_length: u64,
    bundle_total_declared_bytes: u64,
    task_id: &str,
    reference_id: &str,
    owner_record_id: &str,
    action: ArtifactReferenceAuthorityAction,
    retention_until: &str,
    limits: ArtifactStoreLimits,
    source_runtime: RuntimeKind,
) -> ArtifactReferenceManifest {
    let task_id = TaskId::new(task_id).expect("fixture task id");
    let subject = SubjectBinding::new(
        object.key().project_id().clone(),
        ProjectSnapshotId::new("snapshot-1").expect("fixture project snapshot id"),
        task_id.clone(),
        "1",
        marker('5'),
    )
    .expect("fixture subject binding");
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
        ArtifactCounter::new(1).expect("fixture provenance sequence"),
        CREATED_AT,
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
        DaemonEpoch::new(1).expect("fixture daemon epoch"),
        RuntimeAdmissionMode::Active,
        marker('e'),
        marker('f'),
        limits
            .limit_snapshot_digest()
            .expect("fixture limit snapshot digest"),
    )
    .expect("fixture artifact provenance");
    let bundle = ArtifactBundleBounds::new(
        ArtifactCounter::new(u64::from(byte_length > 0)).expect("fixture bundle entries"),
        ArtifactCounter::new(u64::from(byte_length > 0)).expect("fixture bundle depth"),
        ArtifactByteLength::new(bundle_total_declared_bytes).expect("fixture bundle bytes"),
    )
    .expect("fixture bundle bounds");
    let build = |manifest_digest| {
        ArtifactReferenceManifest::new(
            subject.clone(),
            AttemptId::new("attempt-1").expect("fixture attempt id"),
            RequestId::new(format!("request-{reference_id}")).expect("fixture request id"),
            reference_id,
            object.clone(),
            ArtifactByteLength::new(byte_length).expect("fixture byte length"),
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
        .expect("fixture artifact manifest")
    };
    let provisional = build(marker('1'));
    build(artifact_manifest_digest(&provisional).expect("fixture manifest digest"))
}

pub(crate) fn release_reference_pair(
    object: &ArtifactObjectIdentity,
    task_id: &str,
    reference_id: &str,
    owner_record_id: &str,
) -> ArtifactReferenceAuthorityPair {
    reference_pair(
        object,
        &TaskId::new(task_id).expect("fixture task id"),
        reference_id,
        owner_record_id,
        2,
        ArtifactReferenceAuthorityAction::ReleaseReference,
        RuntimeKind::Fake,
    )
}

pub(crate) fn install_manifest_authority(
    directory: &mut FakeArtifactAuthorityDirectory,
    manifest: &ArtifactReferenceManifest,
) {
    directory.install_reference_pair(manifest.creation_authority());
}

pub(crate) fn install_reference_authority(
    directory: &mut FakeArtifactAuthorityDirectory,
    pair: &ArtifactReferenceAuthorityPair,
) {
    directory.install_reference_pair(pair);
}

pub(crate) fn install_read_authority(
    directory: &mut FakeArtifactAuthorityDirectory,
    pair: &ArtifactReadAuthorityPair,
) {
    directory.install_read_pair(pair);
}

pub(crate) fn install_read_closure_authority(
    directory: &mut FakeArtifactAuthorityDirectory,
    pair: &ArtifactReadClosureEvidencePair,
) {
    directory.install_read_closure_pair(pair);
}

pub(crate) fn install_sweep_authority(
    directory: &mut FakeArtifactAuthorityDirectory,
    pair: &ArtifactSweepAuthorityPair,
) {
    directory.install_sweep_pair(pair);
}
