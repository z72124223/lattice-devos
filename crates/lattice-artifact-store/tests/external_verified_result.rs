use lattice_artifact_store::{ExternalVerifiedResultEvidence, ExternalVerifiedResultEvidenceError};
use lattice_contracts::{
    ContentDigest, ProjectId, ProjectSnapshotId, TaskId, TaskLedgerStreamIdentity,
};
use lattice_task_ledger::{ExternalVerifiedResultAdoption, TaskSubmissionEnvelope};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).unwrap()
}

fn adoption() -> ExternalVerifiedResultAdoption {
    let identity = TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new("project-adoption").unwrap(),
        ProjectSnapshotId::new("project-adoption:registry:1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
        TaskId::new("TASK-ADOPTION-1").unwrap(),
        "1",
        digest('a'),
    )
    .unwrap();
    let submission = TaskSubmissionEnvelope::new(
        "lattice_task_submit.v1",
        "adopt-1",
        "closed",
        "project",
        identity,
        digest('b'),
    )
    .unwrap();
    ExternalVerifiedResultAdoption::new(
        submission.task_ref().clone(),
        "adopt-1",
        digest('c'),
        "1".repeat(40),
        "2".repeat(40),
        format!("evidence:sha256:{}", "3".repeat(64)),
        format!("evidence:sha256:{}", "4".repeat(64)),
        format!("evidence:sha256:{}", "5".repeat(64)),
        format!("evidence:sha256:{}", "6".repeat(64)),
        vec![format!("evidence:sha256:{}", "7".repeat(64))],
    )
    .unwrap()
}

#[test]
fn evidence_is_digest_bound_to_verified_target_and_nonforce_receipt() {
    let adoption = adoption();
    let evidence = ExternalVerifiedResultEvidence::new(
        ProjectId::new("project-adoption").unwrap(),
        ProjectSnapshotId::new("project-adoption:registry:1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
        &adoption, "2".repeat(40), digest('8'), digest('9'), "independent-reviewer-1", true,
    )
    .unwrap();
    assert_eq!(evidence.adoption_digest(), adoption.result_digest());
    assert_ne!(evidence.descriptor_digest(), &digest('0'));
    assert_eq!(
        ExternalVerifiedResultEvidence::new(
            ProjectId::new("project-adoption").unwrap(),
            ProjectSnapshotId::new("project-adoption:registry:1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            &adoption, "f".repeat(40), digest('8'), digest('9'), "independent-reviewer-1", true,
        ),
        Err(ExternalVerifiedResultEvidenceError::Mismatch)
    );
    assert_eq!(
        ExternalVerifiedResultEvidence::new(
            ProjectId::new("project-adoption").unwrap(),
            ProjectSnapshotId::new("project-adoption:registry:1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap(),
            &adoption, "2".repeat(40), digest('8'), digest('9'), "independent-reviewer-1", false,
        ),
        Err(ExternalVerifiedResultEvidenceError::Mismatch)
    );
}
