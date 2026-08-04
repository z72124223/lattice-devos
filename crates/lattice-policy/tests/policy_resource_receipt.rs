use lattice_contracts::{
    CONTRACT_VERSION, ContentDigest, ProjectId, ProjectSnapshotId, ResourceCounters,
    ResourceRequest, RuntimeKind, TASK_LEDGER_PRODUCER_ID, TASK_LEDGER_PRODUCER_VERSION, TaskId,
    TaskLedgerResourceReceipt, TaskLedgerStreamHead, TaskLedgerStreamIdentity,
};
use lattice_policy::{ResourceUsageFact, SubjectBinding};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
}

#[test]
fn policy_resource_fact_requires_owner_receipt_and_independent_current_head() {
    let identity = TaskLedgerStreamIdentity::new(
        ProjectId::new("project-1").expect("project"),
        ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
        TaskId::new("TASK-013").expect("task"),
        "1",
        digest('1'),
        "TWD",
    )
    .expect("identity");
    let stream_head = TaskLedgerStreamHead::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        RuntimeKind::Fake,
        identity,
        digest('2'),
        7,
        digest('3'),
        1,
        digest('4'),
        digest('5'),
    )
    .expect("head");
    let receipt = TaskLedgerResourceReceipt::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        RuntimeKind::Fake,
        stream_head,
        3,
        "effect-claim-1",
        digest('6'),
        ResourceCounters::new(1, 1, 10, 1, 2, "0").expect("counters"),
        ResourceRequest::new(0, 0, 0, 0, 0, Some("0")).expect("request"),
        "TWD",
        digest('7'),
        digest('8'),
    )
    .expect("receipt");
    let fact = ResourceUsageFact {
        binding: SubjectBinding::new(
            ProjectId::new("project-1").expect("project"),
            ProjectSnapshotId::new("snapshot-1").expect("snapshot"),
            TaskId::new("TASK-013").expect("task"),
            "1",
            digest('1'),
        )
        .expect("binding"),
        current_head: Some(receipt.head()),
        receipt,
    };

    assert_eq!(fact.current_head.as_ref(), Some(&fact.receipt.head()));
}
