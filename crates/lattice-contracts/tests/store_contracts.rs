use lattice_contracts::{
    ContentDigest, ContractError, DaemonEpoch, ProjectId, ProjectSnapshotId, RuntimeAdmissionMode,
    RuntimeKind, STORE_CONTRACT_VERSION, STORE_CONTRACT_VERSION_V1, STORE_IDENTIFIER_MAX_BYTES,
    STORE_PRODUCER_ID, STORE_PRODUCER_VERSION, STORE_PROJECT_SNAPSHOT_ID_MAX_BYTES,
    StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId, StoreDurability,
    StoreMutationCommitment, StorePersistenceEvidence, StorePhysicalHead, StoreReceiptDisposition,
    StoreRepositoryOwner, StoreRevision, StoreScope, StoreTransactionId, StoreTransactionReceipt,
    StoreTransactionRequest,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn scope(owner: StoreRepositoryOwner) -> StoreScope {
    StoreScope::new(
        ProjectId::new("project-1").expect("valid project"),
        ProjectSnapshotId::new("snapshot-1").expect("valid snapshot"),
        owner,
        digest('a'),
    )
    .expect("valid scope")
}

fn authority(runtime: RuntimeKind, admission: RuntimeAdmissionMode) -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        runtime,
        StoreDaemonInstanceId::new("daemon-1").expect("valid daemon"),
        DaemonEpoch::new(7).expect("valid epoch"),
        admission,
        StoreAuthorityRevision::new(3).expect("valid revision"),
        digest('b'),
        digest('c'),
    )
    .expect("valid authority")
}

fn head(runtime: RuntimeKind, scope: StoreScope, revision: u64, state: char) -> StorePhysicalHead {
    StorePhysicalHead::new(
        runtime,
        scope,
        StoreRevision::new(revision).expect("valid revision"),
        digest(state),
        digest(if revision == 0 { 'd' } else { 'e' }),
    )
    .expect("valid head")
}

fn mutation() -> StoreMutationCommitment {
    StoreMutationCommitment::new(
        digest('1'),
        digest('2'),
        digest('3'),
        digest('4'),
        Some(digest('5')),
        Some(digest('6')),
    )
    .expect("valid mutation")
}

fn request_fixture(runtime: RuntimeKind) -> StoreTransactionRequest {
    let scope = scope(StoreRepositoryOwner::TaskLedger);
    StoreTransactionRequest::new(
        STORE_CONTRACT_VERSION,
        StoreTransactionId::new("transaction-1").expect("valid transaction"),
        scope.clone(),
        authority(runtime, RuntimeAdmissionMode::Active),
        head(runtime, scope, 0, '7'),
        mutation(),
    )
    .expect("valid request")
}

fn persistence() -> StorePersistenceEvidence {
    StorePersistenceEvidence::new(digest('b'), 2, digest('c')).expect("valid persistence")
}

#[test]
fn store_identifiers_are_bounded_canonical_ascii() {
    let exact = "a".repeat(STORE_IDENTIFIER_MAX_BYTES);
    assert_eq!(
        StoreTransactionId::new(exact.clone())
            .expect("exact bound")
            .as_str(),
        exact
    );
    assert!(matches!(
        StoreTransactionId::new("a".repeat(STORE_IDENTIFIER_MAX_BYTES + 1)),
        Err(ContractError::InvalidStoreValue {
            field: "store_transaction_id"
        })
    ));
    for invalid in ["", " ", "Transaction-1", "transaction/1", "交易-1"] {
        assert!(StoreTransactionId::new(invalid).is_err(), "{invalid:?}");
    }
    assert!(StoreDaemonInstanceId::new("daemon-1").is_ok());
    assert!(StoreDaemonInstanceId::new("0daemon").is_ok());
    for compatible_v1_identifier in [".daemon", "_daemon", "-daemon", ":daemon"] {
        assert!(
            StoreDaemonInstanceId::new(compatible_v1_identifier).is_ok(),
            "v1-compatible daemon identity was rejected: {compatible_v1_identifier}"
        );
    }
    assert!(StoreTransactionId::new(".transaction").is_ok());
    assert!(StoreDaemonInstanceId::new("daemon\0one").is_err());
}

#[test]
fn live_authority_requires_a_sql_compatible_daemon_prefix_without_breaking_v1_fake() {
    let legacy_daemon = StoreDaemonInstanceId::new(".daemon").expect("v1-compatible daemon");
    let fake_authority = StoreAuthorityHead::new(
        RuntimeKind::Fake,
        legacy_daemon.clone(),
        DaemonEpoch::new(7).expect("valid epoch"),
        RuntimeAdmissionMode::Active,
        StoreAuthorityRevision::new(3).expect("valid revision"),
        digest('b'),
        digest('c'),
    )
    .expect("v1 fake authority remains compatible");
    let v1_scope = scope(StoreRepositoryOwner::TaskLedger);
    assert!(
        StoreTransactionRequest::new(
            STORE_CONTRACT_VERSION_V1,
            StoreTransactionId::new("transaction-v1-legacy-daemon").expect("valid transaction"),
            v1_scope.clone(),
            fake_authority,
            head(RuntimeKind::Fake, v1_scope, 0, '7'),
            mutation(),
        )
        .is_ok()
    );

    assert!(matches!(
        StoreAuthorityHead::new(
            RuntimeKind::Live,
            legacy_daemon,
            DaemonEpoch::new(7).expect("valid epoch"),
            RuntimeAdmissionMode::Active,
            StoreAuthorityRevision::new(3).expect("valid revision"),
            digest('b'),
            digest('c'),
        ),
        Err(ContractError::InvalidStoreValue {
            field: "store_daemon_instance_id"
        })
    ));
}

#[test]
fn store_owner_and_scope_are_closed_and_project_scoped() {
    assert_eq!(
        StoreRepositoryOwner::ALL.map(StoreRepositoryOwner::as_str),
        [
            "PROJECT_REGISTRY",
            "TASK_LEDGER",
            "WRITER_LEASE",
            "APPROVAL_VERIFIER",
            "ARTIFACT_STORE",
        ]
    );
    let value = scope(StoreRepositoryOwner::ProjectRegistry);
    assert_eq!(value.project_id().as_str(), "project-1");
    assert_eq!(value.project_snapshot_id().as_str(), "snapshot-1");
    assert_eq!(value.owner(), StoreRepositoryOwner::ProjectRegistry);
    assert_eq!(value.aggregate_key_digest(), &digest('a'));
    let maximum_snapshot = "s".repeat(STORE_PROJECT_SNAPSHOT_ID_MAX_BYTES);
    assert_eq!(
        StoreScope::new(
            ProjectId::new("project-1").expect("valid project"),
            ProjectSnapshotId::new(maximum_snapshot.clone()).expect("maximum snapshot"),
            StoreRepositoryOwner::ProjectRegistry,
            digest('1'),
        )
        .expect("159-byte Registry snapshot")
        .project_snapshot_id()
        .as_str(),
        maximum_snapshot
    );
    assert!(matches!(
        StoreScope::new(
            ProjectId::new("project-1").expect("valid project"),
            ProjectSnapshotId::new("snapshot-1").expect("valid snapshot"),
            StoreRepositoryOwner::ProjectRegistry,
            digest('0'),
        ),
        Err(ContractError::InvalidStoreValue {
            field: "aggregate_key_digest"
        })
    ));
    for invalid_snapshot in [
        "Snapshot-1".to_owned(),
        "x".repeat(STORE_PROJECT_SNAPSHOT_ID_MAX_BYTES + 1),
    ] {
        assert!(matches!(
            StoreScope::new(
                ProjectId::new("project-1").expect("valid project"),
                ProjectSnapshotId::new(invalid_snapshot).expect("generic snapshot shape"),
                StoreRepositoryOwner::ProjectRegistry,
                digest('1'),
            ),
            Err(ContractError::InvalidStoreValue {
                field: "project_snapshot_id"
            })
        ));
    }
}

#[test]
fn store_numeric_and_security_values_fail_closed() {
    assert_eq!(StoreRevision::new(0).expect("genesis").get(), 0);
    assert_eq!(
        StoreRevision::new(i64::MAX as u64)
            .expect("signed bigint maximum")
            .get(),
        i64::MAX as u64
    );
    assert!(StoreRevision::new((i64::MAX as u64) + 1).is_err());
    assert!(StoreAuthorityRevision::new(0).is_err());
    assert!(
        StoreAuthorityHead::new(
            RuntimeKind::Fake,
            StoreDaemonInstanceId::new("daemon-1").expect("valid daemon"),
            DaemonEpoch::new(1).expect("valid epoch"),
            RuntimeAdmissionMode::Active,
            StoreAuthorityRevision::new(1).expect("valid revision"),
            digest('0'),
            digest('c'),
        )
        .is_err()
    );
    assert!(
        StoreMutationCommitment::new(
            digest('1'),
            digest('2'),
            digest('0'),
            digest('4'),
            None,
            None,
        )
        .is_err()
    );
}

#[test]
fn transaction_request_binds_complete_runtime_and_scope() {
    let valid = request_fixture(RuntimeKind::Fake);
    assert_eq!(valid.version(), STORE_CONTRACT_VERSION);
    assert_eq!(valid.transaction_id().as_str(), "transaction-1");
    assert_eq!(valid.scope(), valid.expected_head().scope());
    assert_eq!(valid.expected_authority().runtime(), RuntimeKind::Fake);
    assert_eq!(valid.mutation().domain_command_digest(), &digest('1'));

    let other_scope = scope(StoreRepositoryOwner::ProjectRegistry);
    assert!(matches!(
        StoreTransactionRequest::new(
            STORE_CONTRACT_VERSION,
            StoreTransactionId::new("transaction-2").expect("valid transaction"),
            other_scope,
            authority(RuntimeKind::Fake, RuntimeAdmissionMode::Active),
            valid.expected_head().clone(),
            mutation(),
        ),
        Err(ContractError::StoreScopeMismatch)
    ));
    assert!(matches!(
        StoreTransactionRequest::new(
            STORE_CONTRACT_VERSION,
            StoreTransactionId::new("transaction-3").expect("valid transaction"),
            valid.scope().clone(),
            authority(RuntimeKind::Live, RuntimeAdmissionMode::Active),
            valid.expected_head().clone(),
            mutation(),
        ),
        Err(ContractError::StoreRuntimeMismatch)
    ));
    assert!(matches!(
        StoreTransactionRequest::new(
            STORE_CONTRACT_VERSION + 1,
            StoreTransactionId::new("transaction-4").expect("valid transaction"),
            valid.scope().clone(),
            valid.expected_authority().clone(),
            valid.expected_head().clone(),
            mutation(),
        ),
        Err(ContractError::UnsupportedStoreContractVersion)
    ));
}

#[test]
fn store_v1_remains_fake_only_while_v2_accepts_live_requests() {
    assert_eq!(STORE_CONTRACT_VERSION_V1, 1);
    assert_eq!(STORE_CONTRACT_VERSION, 2);

    let v1_scope = scope(StoreRepositoryOwner::TaskLedger);
    assert!(
        StoreTransactionRequest::new(
            STORE_CONTRACT_VERSION_V1,
            StoreTransactionId::new("transaction-v1-fake").expect("valid transaction"),
            v1_scope.clone(),
            authority(RuntimeKind::Fake, RuntimeAdmissionMode::Active),
            head(RuntimeKind::Fake, v1_scope, 0, '7'),
            mutation(),
        )
        .is_ok()
    );

    let live_scope = scope(StoreRepositoryOwner::TaskLedger);
    assert!(matches!(
        StoreTransactionRequest::new(
            STORE_CONTRACT_VERSION_V1,
            StoreTransactionId::new("transaction-v1-live").expect("valid transaction"),
            live_scope.clone(),
            authority(RuntimeKind::Live, RuntimeAdmissionMode::Active),
            head(RuntimeKind::Live, live_scope, 0, '7'),
            mutation(),
        ),
        Err(ContractError::UnsupportedStoreContractVersion)
    ));

    assert_eq!(request_fixture(RuntimeKind::Live).version(), 2);
}

#[test]
fn postgres_persistence_evidence_is_complete_and_nonzero() {
    let evidence = persistence();
    assert_eq!(evidence.database_identity_digest(), &digest('b'));
    assert_eq!(evidence.schema_version(), 2);
    assert_eq!(evidence.manifest_digest(), &digest('c'));

    for result in [
        StorePersistenceEvidence::new(digest('0'), 2, digest('c')),
        StorePersistenceEvidence::new(digest('b'), 0, digest('c')),
        StorePersistenceEvidence::new(digest('b'), 2, digest('0')),
    ] {
        assert!(result.is_err());
    }
}

#[test]
fn live_receipts_require_v2_postgres_durability_and_persistence() {
    let request = request_fixture(RuntimeKind::Live);
    let before = request.expected_head().clone();
    let after = head(RuntimeKind::Live, request.scope().clone(), 1, '3');
    let receipt = StoreTransactionReceipt::new_durable_postgres(
        request.clone(),
        persistence(),
        digest('8'),
        before.clone(),
        after.clone(),
        StoreReceiptDisposition::Applied,
        digest('9'),
        digest('a'),
    )
    .expect("valid live receipt");

    assert_eq!(receipt.runtime(), RuntimeKind::Live);
    assert_eq!(receipt.durability(), StoreDurability::DurablePostgres);
    assert_eq!(receipt.persistence(), Some(&persistence()));
    assert_eq!(receipt.before_head(), &before);
    assert_eq!(receipt.after_head(), &after);

    let fake_request = request_fixture(RuntimeKind::Fake);
    assert!(matches!(
        StoreTransactionReceipt::new_durable_postgres(
            fake_request.clone(),
            persistence(),
            digest('8'),
            fake_request.expected_head().clone(),
            head(RuntimeKind::Fake, fake_request.scope().clone(), 1, '3'),
            StoreReceiptDisposition::Applied,
            digest('9'),
            digest('a'),
        ),
        Err(ContractError::StoreRuntimeMismatch)
    ));
}

#[test]
fn fake_receipts_are_complete_and_cannot_claim_durability() {
    let request = request_fixture(RuntimeKind::Fake);
    let before = request.expected_head().clone();
    let after = head(RuntimeKind::Fake, request.scope().clone(), 1, '3');
    let receipt = StoreTransactionReceipt::new_non_durable_fake(
        request.clone(),
        digest('8'),
        before.clone(),
        after.clone(),
        StoreReceiptDisposition::Applied,
        digest('9'),
        digest('a'),
    )
    .expect("valid fake receipt");

    assert_eq!(receipt.producer_id(), STORE_PRODUCER_ID);
    assert_eq!(receipt.producer_version(), STORE_PRODUCER_VERSION);
    assert_eq!(receipt.runtime(), RuntimeKind::Fake);
    assert_eq!(receipt.durability(), StoreDurability::NonDurableFake);
    assert_eq!(receipt.persistence(), None);
    assert_eq!(receipt.request(), &request);
    assert_eq!(receipt.before_head(), &before);
    assert_eq!(receipt.after_head(), &after);
    assert_eq!(receipt.disposition(), StoreReceiptDisposition::Applied);

    let live_request = request_fixture(RuntimeKind::Live);
    assert!(matches!(
        StoreTransactionReceipt::new_non_durable_fake(
            live_request.clone(),
            digest('8'),
            live_request.expected_head().clone(),
            head(RuntimeKind::Live, live_request.scope().clone(), 1, '3'),
            StoreReceiptDisposition::Applied,
            digest('9'),
            digest('a'),
        ),
        Err(ContractError::StoreRuntimeMismatch)
    ));
}

#[test]
fn stale_receipt_is_terminal_but_cannot_mutate_the_head() {
    let request = request_fixture(RuntimeKind::Fake);
    let current = head(RuntimeKind::Fake, request.scope().clone(), 2, 'e');
    let receipt = StoreTransactionReceipt::new_non_durable_fake(
        request,
        digest('8'),
        current.clone(),
        current.clone(),
        StoreReceiptDisposition::StalePhysicalHead,
        digest('9'),
        digest('a'),
    )
    .expect("valid stale receipt");
    assert_eq!(receipt.before_head(), receipt.after_head());

    let request = request_fixture(RuntimeKind::Fake);
    assert!(
        StoreTransactionReceipt::new_non_durable_fake(
            request.clone(),
            digest('8'),
            request.expected_head().clone(),
            request.expected_head().clone(),
            StoreReceiptDisposition::StalePhysicalHead,
            digest('9'),
            digest('a'),
        )
        .is_err()
    );
}
