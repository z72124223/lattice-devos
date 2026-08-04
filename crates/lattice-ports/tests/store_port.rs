use lattice_contracts::{
    ContentDigest, DaemonEpoch, ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind,
    STORE_CONTRACT_VERSION, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId,
    StoreMutationCommitment, StorePhysicalHead, StoreRepositoryOwner, StoreRevision, StoreScope,
    StoreTransactionId, StoreTransactionReceipt, StoreTransactionRequest,
};
use lattice_ports::{ControlStore, ControlStoreError, ControlStoreErrorKind, ControlStoreResult};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn scope() -> StoreScope {
    StoreScope::new(
        ProjectId::new("project-1").expect("valid project"),
        ProjectSnapshotId::new("snapshot-1").expect("valid snapshot"),
        StoreRepositoryOwner::TaskLedger,
        digest('1'),
    )
    .expect("valid scope")
}

fn request() -> StoreTransactionRequest {
    let scope = scope();
    StoreTransactionRequest::new(
        STORE_CONTRACT_VERSION,
        StoreTransactionId::new("transaction-1").expect("valid transaction"),
        scope.clone(),
        StoreAuthorityHead::new(
            RuntimeKind::Fake,
            StoreDaemonInstanceId::new("daemon-1").expect("valid daemon"),
            DaemonEpoch::new(1).expect("valid epoch"),
            RuntimeAdmissionMode::Active,
            StoreAuthorityRevision::new(1).expect("valid authority revision"),
            digest('2'),
            digest('3'),
        )
        .expect("valid authority"),
        StorePhysicalHead::new(
            RuntimeKind::Fake,
            scope,
            StoreRevision::new(0).expect("genesis"),
            digest('4'),
            digest('5'),
        )
        .expect("valid head"),
        StoreMutationCommitment::new(
            digest('6'),
            digest('7'),
            digest('8'),
            digest('9'),
            None,
            None,
        )
        .expect("valid mutation"),
    )
    .expect("valid request")
}

struct TypedStore {
    head: StorePhysicalHead,
}

impl ControlStore for TypedStore {
    fn transact(
        &mut self,
        _request: StoreTransactionRequest,
    ) -> ControlStoreResult<StoreTransactionReceipt> {
        Err(ControlStoreError::new(
            ControlStoreErrorKind::CommitOutcomeUnknown,
            "STORE_COMMIT_OUTCOME_UNKNOWN",
        ))
    }

    fn current_head(&mut self, _scope: &StoreScope) -> ControlStoreResult<StorePhysicalHead> {
        Ok(self.head.clone())
    }
}

#[test]
fn control_store_uses_complete_typed_transaction_and_head_contracts() {
    let request = request();
    let mut store = TypedStore {
        head: request.expected_head().clone(),
    };
    assert_eq!(
        store.current_head(request.scope()).expect("head"),
        store.head
    );
    let error = store.transact(request).expect_err("unknown outcome");
    assert_eq!(error.kind(), ControlStoreErrorKind::CommitOutcomeUnknown);
    assert_eq!(error.code(), "STORE_COMMIT_OUTCOME_UNKNOWN");
}

#[test]
fn control_store_error_kinds_are_closed_and_do_not_imply_success() {
    assert_eq!(
        ControlStoreErrorKind::ALL,
        [
            ControlStoreErrorKind::Malformed,
            ControlStoreErrorKind::UnsupportedVersion,
            ControlStoreErrorKind::CommandSubstitution,
            ControlStoreErrorKind::AuthorityMismatch,
            ControlStoreErrorKind::AdmissionDenied,
            ControlStoreErrorKind::RevisionOverflow,
            ControlStoreErrorKind::CapacityExceeded,
            ControlStoreErrorKind::Unavailable,
            ControlStoreErrorKind::SerializationExhausted,
            ControlStoreErrorKind::CommitOutcomeUnknown,
            ControlStoreErrorKind::CorruptState,
        ]
    );
}
