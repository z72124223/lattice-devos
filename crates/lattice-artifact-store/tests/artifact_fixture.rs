mod support;

use lattice_artifact_store::{ArtifactStoreLimits, FakeArtifactAuthorityDirectory};
use lattice_contracts::{
    ArtifactReadAuthorityAction, ArtifactReadClosureEvidenceKind, ArtifactReferenceAuthorityAction,
    RuntimeKind, TaskId,
};

#[test]
fn reusable_fixture_builds_complete_contracts_through_public_apis() {
    let bytes = b"fixture-payload";
    let object = support::object_identity("project-fixture", bytes, 1);
    let manifest = support::manifest(
        &object,
        bytes.len() as u64,
        "task-fixture",
        "reference-fixture",
        "reference-owner-fixture",
        ArtifactReferenceAuthorityAction::PublishInitialReference,
        support::RETENTION,
        ArtifactStoreLimits::hard_maximums(),
        RuntimeKind::Fake,
    );
    let task_id = TaskId::new("task-fixture").expect("fixture task id");
    let read = support::read_pair(
        &object,
        &task_id,
        "read-fixture",
        "read-owner-fixture",
        1,
        ArtifactReadAuthorityAction::AcquireRead,
    );
    let closure = support::read_closure_pair(
        &object,
        &task_id,
        "read-fixture",
        "holder-fixture",
        "closure-fixture",
        1,
        ArtifactReadClosureEvidenceKind::HandleClosed,
        RuntimeKind::Fake,
        support::READ_CLOSED_AT,
    );
    let mut directory = FakeArtifactAuthorityDirectory::default();
    support::install_manifest_authority(&mut directory, &manifest);
    support::install_read_authority(&mut directory, &read);
    support::install_read_closure_authority(&mut directory, &closure);

    assert_eq!(manifest.object(), &object);
    assert_eq!(
        manifest.object().key().content_digest(),
        &support::content_digest(bytes)
    );
}
