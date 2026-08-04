use lattice_contracts::{
    ContentDigest, DaemonEpoch, ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind,
    STORE_CONTRACT_VERSION, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId,
    StoreMutationCommitment, StorePhysicalHead, StoreReceiptDisposition, StoreRepositoryOwner,
    StoreRevision, StoreScope, StoreTransactionId, StoreTransactionRequest,
};
use lattice_ports::{ControlStore, ControlStoreErrorKind};
use lattice_postgres_store::{
    FakePostgresStore, FakeReplayCorruption, FakeStoreFault, MAX_FAKE_TRANSACTIONS,
    MAX_SERIALIZATION_ATTEMPTS,
};

fn digest(byte: char) -> ContentDigest {
    ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("valid digest")
}

fn scope(
    project: &str,
    snapshot: &str,
    owner: StoreRepositoryOwner,
    aggregate: char,
) -> StoreScope {
    StoreScope::new(
        ProjectId::new(project).expect("valid project"),
        ProjectSnapshotId::new(snapshot).expect("valid snapshot"),
        owner,
        digest(aggregate),
    )
    .expect("valid scope")
}

fn authority(
    runtime: RuntimeKind,
    instance: &str,
    epoch: u64,
    admission: RuntimeAdmissionMode,
    revision: u64,
    observation: char,
    head: char,
) -> StoreAuthorityHead {
    StoreAuthorityHead::new(
        runtime,
        StoreDaemonInstanceId::new(instance).expect("valid daemon"),
        DaemonEpoch::new(epoch).expect("valid epoch"),
        admission,
        StoreAuthorityRevision::new(revision).expect("valid revision"),
        digest(observation),
        digest(head),
    )
    .expect("valid authority")
}

fn active_authority() -> StoreAuthorityHead {
    authority(
        RuntimeKind::Fake,
        "daemon-1",
        7,
        RuntimeAdmissionMode::Active,
        3,
        'a',
        'b',
    )
}

fn mutation(seed: u8) -> StoreMutationCommitment {
    let chars = b"123456789abcdef";
    let at = |offset: usize| char::from(chars[(usize::from(seed) + offset) % chars.len()]);
    StoreMutationCommitment::new(
        digest(at(0)),
        digest(at(1)),
        digest(at(2)),
        digest(at(3)),
        Some(digest(at(4))),
        Some(digest(at(5))),
    )
    .expect("valid mutation")
}

fn request(
    id: &str,
    scope: StoreScope,
    authority: StoreAuthorityHead,
    expected_head: StorePhysicalHead,
    mutation: StoreMutationCommitment,
) -> StoreTransactionRequest {
    StoreTransactionRequest::new(
        STORE_CONTRACT_VERSION,
        StoreTransactionId::new(id).expect("valid transaction"),
        scope,
        authority,
        expected_head,
        mutation,
    )
    .expect("valid request")
}

fn assert_kind<T: std::fmt::Debug>(
    result: Result<T, lattice_ports::ControlStoreError>,
    kind: ControlStoreErrorKind,
) {
    let error = result.expect_err("expected Store failure");
    assert_eq!(error.kind(), kind, "{}", error.code());
}

#[test]
fn genesis_heads_are_deterministic_fake_and_fully_project_isolated() {
    let mut store = FakePostgresStore::new(active_authority(), 8).expect("fake store");
    let first = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let same = store.current_head(&first).expect("genesis");
    assert_eq!(same, store.current_head(&first).expect("same genesis"));
    assert_eq!(same.runtime(), RuntimeKind::Fake);
    assert_eq!(same.revision().get(), 0);

    for other in [
        scope(
            "project-2",
            "snapshot-1",
            StoreRepositoryOwner::TaskLedger,
            '1',
        ),
        scope(
            "project-1",
            "snapshot-2",
            StoreRepositoryOwner::TaskLedger,
            '1',
        ),
        scope(
            "project-1",
            "snapshot-1",
            StoreRepositoryOwner::ProjectRegistry,
            '1',
        ),
        scope(
            "project-1",
            "snapshot-1",
            StoreRepositoryOwner::TaskLedger,
            '2',
        ),
    ] {
        assert_ne!(same, store.current_head(&other).expect("isolated genesis"));
    }
    assert_eq!(store.transaction_count(), 0);
    assert_eq!(store.materialized_scope_count(), 0);
}

#[test]
fn valid_transaction_atomically_advances_one_physical_head() {
    let current_authority = active_authority();
    let mut store = FakePostgresStore::new(current_authority.clone(), 8).expect("fake store");
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let before = store.current_head(&scope).expect("genesis");
    let mutation = mutation(0);
    let request = request(
        "transaction-1",
        scope.clone(),
        current_authority,
        before.clone(),
        mutation.clone(),
    );
    let receipt = store.transact(request.clone()).expect("applied");

    assert_eq!(receipt.disposition(), StoreReceiptDisposition::Applied);
    assert_eq!(receipt.request(), &request);
    assert_eq!(receipt.before_head(), &before);
    assert_eq!(receipt.after_head().revision().get(), 1);
    assert_eq!(
        receipt.after_head().state_digest(),
        mutation.next_state_digest()
    );
    assert_eq!(
        store.current_head(&scope).expect("current"),
        *receipt.after_head()
    );
    assert_eq!(store.transaction_count(), 1);
    assert_eq!(store.materialized_scope_count(), 1);
}

#[test]
fn exact_retry_precedes_mutable_checks_and_changed_reuse_never_leaks() {
    let original_authority = active_authority();
    let mut store = FakePostgresStore::new(original_authority.clone(), 8).expect("fake store");
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let first = request(
        "transaction-1",
        scope.clone(),
        original_authority.clone(),
        store.current_head(&scope).expect("genesis"),
        mutation(0),
    );
    let first_receipt = store.transact(first.clone()).expect("first apply");
    let second = request(
        "transaction-2",
        scope.clone(),
        original_authority.clone(),
        store.current_head(&scope).expect("head one"),
        mutation(1),
    );
    store.transact(second).expect("second apply");
    let head_after_second = store.current_head(&scope).expect("head two");

    let new_authority = authority(
        RuntimeKind::Fake,
        "daemon-2",
        8,
        RuntimeAdmissionMode::Draining,
        4,
        'c',
        'd',
    );
    store
        .set_current_authority(new_authority)
        .expect("fake authority update");
    assert_eq!(
        store.transact(first.clone()).expect("exact retry"),
        first_receipt
    );
    assert_eq!(
        store.current_head(&scope).expect("unchanged"),
        head_after_second
    );

    let changed = request(
        "transaction-1",
        scope,
        original_authority,
        first.expected_head().clone(),
        mutation(9),
    );
    assert_kind(
        store.transact(changed),
        ControlStoreErrorKind::CommandSubstitution,
    );
    assert_eq!(store.transaction_count(), 2);
}

#[test]
fn stale_head_is_terminal_non_mutating_and_exactly_replayable() {
    let authority = active_authority();
    let mut store = FakePostgresStore::new(authority.clone(), 8).expect("fake store");
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let genesis = store.current_head(&scope).expect("genesis");
    store
        .transact(request(
            "transaction-1",
            scope.clone(),
            authority.clone(),
            genesis.clone(),
            mutation(0),
        ))
        .expect("first apply");
    let current = store.current_head(&scope).expect("current");
    let stale = request(
        "transaction-2",
        scope.clone(),
        authority,
        genesis,
        mutation(1),
    );
    let receipt = store.transact(stale.clone()).expect("terminal stale");
    assert_eq!(
        receipt.disposition(),
        StoreReceiptDisposition::StalePhysicalHead
    );
    assert_eq!(receipt.before_head(), &current);
    assert_eq!(receipt.after_head(), &current);
    assert_eq!(store.current_head(&scope).expect("unchanged"), current);
    assert_eq!(store.transact(stale).expect("stale replay"), receipt);
}

#[test]
fn authority_head_substitution_is_rejected_without_mutation() {
    let current = active_authority();
    let mut store = FakePostgresStore::new(current.clone(), 8).expect("fake store");
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let head = store.current_head(&scope).expect("genesis");
    let substitutions = [
        authority(
            RuntimeKind::Fake,
            "daemon-2",
            7,
            RuntimeAdmissionMode::Active,
            3,
            'a',
            'b',
        ),
        authority(
            RuntimeKind::Fake,
            "daemon-1",
            8,
            RuntimeAdmissionMode::Active,
            3,
            'a',
            'b',
        ),
        authority(
            RuntimeKind::Fake,
            "daemon-1",
            7,
            RuntimeAdmissionMode::Active,
            4,
            'a',
            'b',
        ),
        authority(
            RuntimeKind::Fake,
            "daemon-1",
            7,
            RuntimeAdmissionMode::Active,
            3,
            'c',
            'b',
        ),
        authority(
            RuntimeKind::Fake,
            "daemon-1",
            7,
            RuntimeAdmissionMode::Active,
            3,
            'a',
            'c',
        ),
    ];
    for (index, substituted) in substitutions.into_iter().enumerate() {
        assert_kind(
            store.transact(request(
                &format!("transaction-{index}"),
                scope.clone(),
                substituted,
                head.clone(),
                mutation(u8::try_from(index).expect("small substitution matrix")),
            )),
            ControlStoreErrorKind::AuthorityMismatch,
        );
    }
    assert_eq!(store.transaction_count(), 0);
    assert_eq!(store.current_head(&scope).expect("unchanged"), head);
}

#[test]
fn non_active_and_live_authority_fail_closed() {
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    for admission in [
        RuntimeAdmissionMode::Draining,
        RuntimeAdmissionMode::Canary,
        RuntimeAdmissionMode::Stopped,
        RuntimeAdmissionMode::ReconciliationRequired,
    ] {
        let authority = authority(RuntimeKind::Fake, "daemon-1", 7, admission, 3, 'a', 'b');
        let mut denied = FakePostgresStore::new(authority.clone(), 1).expect("fake store");
        let head = denied.current_head(&scope).expect("genesis");
        assert_kind(
            denied.transact(request(
                "transaction-denied",
                scope.clone(),
                authority,
                head,
                mutation(0),
            )),
            ControlStoreErrorKind::AdmissionDenied,
        );
        assert_eq!(denied.transaction_count(), 0);
    }

    assert_kind(
        FakePostgresStore::new(
            authority(
                RuntimeKind::Live,
                "daemon-1",
                7,
                RuntimeAdmissionMode::Active,
                3,
                'a',
                'b',
            ),
            1,
        ),
        ControlStoreErrorKind::Malformed,
    );
}

#[test]
fn transaction_capacity_is_bounded_but_existing_replay_remains_available() {
    assert_kind(
        FakePostgresStore::new(active_authority(), 0),
        ControlStoreErrorKind::Malformed,
    );
    assert_kind(
        FakePostgresStore::new(active_authority(), MAX_FAKE_TRANSACTIONS + 1),
        ControlStoreErrorKind::Malformed,
    );

    let authority = active_authority();
    let mut store = FakePostgresStore::new(authority.clone(), 1).expect("fake store");
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let first = request(
        "transaction-1",
        scope.clone(),
        authority.clone(),
        store.current_head(&scope).expect("genesis"),
        mutation(0),
    );
    let receipt = store.transact(first.clone()).expect("first apply");
    let second = request(
        "transaction-2",
        scope.clone(),
        authority,
        store.current_head(&scope).expect("current"),
        mutation(1),
    );
    assert_kind(
        store.transact(second),
        ControlStoreErrorKind::CapacityExceeded,
    );
    assert_eq!(store.transact(first).expect("retained replay"), receipt);
}

#[test]
fn revision_overflow_and_serialization_exhaustion_never_mutate() {
    let authority = active_authority();
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let max_head = FakePostgresStore::derive_head_for_fixture(
        scope.clone(),
        StoreRevision::new(i64::MAX as u64).expect("maximum revision"),
        digest('1'),
    )
    .expect("valid head");
    let mut overflow = FakePostgresStore::with_heads(authority.clone(), 2, [max_head.clone()])
        .expect("seeded fake");
    assert_kind(
        overflow.transact(request(
            "transaction-overflow",
            scope.clone(),
            authority.clone(),
            max_head.clone(),
            mutation(0),
        )),
        ControlStoreErrorKind::RevisionOverflow,
    );
    assert_eq!(overflow.current_head(&scope).expect("unchanged"), max_head);
    assert_eq!(overflow.transaction_count(), 0);

    let mut serialized = FakePostgresStore::new(authority.clone(), 2).expect("fake store");
    let genesis = serialized.current_head(&scope).expect("genesis");
    serialized.inject_next_fault(FakeStoreFault::SerializationConflicts(
        MAX_SERIALIZATION_ATTEMPTS - 1,
    ));
    serialized
        .transact(request(
            "transaction-retried",
            scope.clone(),
            authority.clone(),
            genesis,
            mutation(0),
        ))
        .expect("bounded retry applies");

    let current = serialized.current_head(&scope).expect("current");
    serialized.inject_next_fault(FakeStoreFault::SerializationConflicts(
        MAX_SERIALIZATION_ATTEMPTS,
    ));
    assert_kind(
        serialized.transact(request(
            "transaction-exhausted",
            scope.clone(),
            authority,
            current.clone(),
            mutation(1),
        )),
        ControlStoreErrorKind::SerializationExhausted,
    );
    assert_eq!(serialized.current_head(&scope).expect("unchanged"), current);
}

#[test]
fn seeded_heads_must_match_the_store_hash_domain() {
    let seed_scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let tampered = StorePhysicalHead::new(
        RuntimeKind::Fake,
        seed_scope,
        StoreRevision::new(4).expect("revision"),
        digest('1'),
        digest('2'),
    )
    .expect("structurally valid but unhashed head");
    assert_kind(
        FakePostgresStore::with_heads(active_authority(), 2, [tampered]),
        ControlStoreErrorKind::CorruptState,
    );

    let genesis_override = FakePostgresStore::derive_head_for_fixture(
        scope(
            "project-1",
            "snapshot-1",
            StoreRepositoryOwner::TaskLedger,
            '2',
        ),
        StoreRevision::new(0).expect("genesis revision"),
        digest('9'),
    );
    assert_kind(genesis_override, ControlStoreErrorKind::CorruptState);
}

#[test]
fn before_and_after_apply_faults_preserve_explicit_commit_uncertainty() {
    let authority = active_authority();
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let mut before = FakePostgresStore::new(authority.clone(), 2).expect("fake store");
    let genesis = before.current_head(&scope).expect("genesis");
    let command = request(
        "transaction-before",
        scope.clone(),
        authority.clone(),
        genesis.clone(),
        mutation(0),
    );
    before.inject_next_fault(FakeStoreFault::BeforeApplyUnavailable);
    assert_kind(before.transact(command), ControlStoreErrorKind::Unavailable);
    assert_eq!(before.current_head(&scope).expect("unchanged"), genesis);
    assert_eq!(before.transaction_count(), 0);

    let mut after = FakePostgresStore::new(authority.clone(), 2).expect("fake store");
    let genesis = after.current_head(&scope).expect("genesis");
    let command = request(
        "transaction-after",
        scope.clone(),
        authority,
        genesis.clone(),
        mutation(0),
    );
    after.inject_next_fault(FakeStoreFault::AfterApplyOutcomeUnknown);
    assert_kind(
        after.transact(command.clone()),
        ControlStoreErrorKind::CommitOutcomeUnknown,
    );
    assert_eq!(after.transaction_count(), 1);
    assert_eq!(
        after
            .current_head(&scope)
            .expect("applied")
            .revision()
            .get(),
        1
    );
    assert_eq!(
        after
            .transact(command)
            .expect("reconciled retry")
            .disposition(),
        StoreReceiptDisposition::Applied
    );
}

fn matrix_fixture() -> (
    FakePostgresStore,
    StoreAuthorityHead,
    StoreScope,
    StorePhysicalHead,
) {
    let current_authority = active_authority();
    let mut store = FakePostgresStore::new(current_authority.clone(), 8).expect("fake store");
    let base_scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let base_head = store.current_head(&base_scope).expect("genesis");
    let base = request(
        "transaction-matrix",
        base_scope.clone(),
        current_authority.clone(),
        base_head.clone(),
        mutation(0),
    );
    store.transact(base).expect("base apply");
    (store, current_authority, base_scope, base_head)
}

#[test]
fn request_digest_binds_project_snapshot_and_runtime_scope() {
    let (mut store, current_authority, base_scope, base_head) = matrix_fixture();
    let alternate_scope = scope(
        "project-2",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let alternate_head = store.current_head(&alternate_scope).expect("other genesis");
    let live_scope = base_scope.clone();
    let live_authority = authority(
        RuntimeKind::Live,
        "daemon-1",
        7,
        RuntimeAdmissionMode::Active,
        3,
        'a',
        'b',
    );
    let live_head = StorePhysicalHead::new(
        RuntimeKind::Live,
        live_scope.clone(),
        StoreRevision::new(0).expect("genesis"),
        base_head.state_digest().clone(),
        base_head.head_digest().clone(),
    )
    .expect("live-shaped head");
    let snapshot_scope = scope(
        "project-1",
        "snapshot-2",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let snapshot_head = store
        .current_head(&snapshot_scope)
        .expect("snapshot genesis");
    let variants = [
        request(
            "transaction-matrix",
            alternate_scope,
            current_authority.clone(),
            alternate_head,
            mutation(0),
        ),
        request(
            "transaction-matrix",
            snapshot_scope,
            current_authority.clone(),
            snapshot_head,
            mutation(0),
        ),
        request(
            "transaction-matrix",
            live_scope,
            live_authority,
            live_head,
            mutation(0),
        ),
    ];
    for variant in variants {
        assert_kind(
            store.transact(variant),
            ControlStoreErrorKind::CommandSubstitution,
        );
    }
    assert_eq!(store.transaction_count(), 1);
}

#[test]
fn request_digest_binds_authority_physical_head_and_mutation() {
    let (mut store, current_authority, base_scope, base_head) = matrix_fixture();
    let variants = [
        request(
            "transaction-matrix",
            base_scope.clone(),
            authority(
                RuntimeKind::Fake,
                "daemon-2",
                7,
                RuntimeAdmissionMode::Active,
                3,
                'a',
                'b',
            ),
            base_head.clone(),
            mutation(0),
        ),
        request(
            "transaction-matrix",
            base_scope.clone(),
            current_authority.clone(),
            StorePhysicalHead::new(
                RuntimeKind::Fake,
                base_scope.clone(),
                StoreRevision::new(0).expect("genesis"),
                digest('e'),
                base_head.head_digest().clone(),
            )
            .expect("changed state head"),
            mutation(0),
        ),
        request(
            "transaction-matrix",
            base_scope,
            current_authority,
            base_head,
            mutation(7),
        ),
    ];
    for variant in variants {
        assert_kind(
            store.transact(variant),
            ControlStoreErrorKind::CommandSubstitution,
        );
    }
    assert_eq!(store.transaction_count(), 1);
}

#[test]
fn corrupted_retained_replay_fails_closed_instead_of_returning_receipt() {
    let authority = active_authority();
    let mut store = FakePostgresStore::new(authority.clone(), 2).expect("fake store");
    let scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let command = request(
        "transaction-corrupt",
        scope.clone(),
        authority,
        store.current_head(&scope).expect("genesis"),
        mutation(0),
    );
    store.transact(command.clone()).expect("apply");

    let mut request_corrupt = store.clone();
    assert!(request_corrupt.inject_replay_corruption(
        command.transaction_id(),
        FakeReplayCorruption::RequestDigest
    ));
    assert_kind(
        request_corrupt.transact(command.clone()),
        ControlStoreErrorKind::CorruptState,
    );

    let mut receipt_corrupt = store.clone();
    assert!(receipt_corrupt.inject_replay_corruption(
        command.transaction_id(),
        FakeReplayCorruption::ReceiptDigest
    ));
    assert_kind(
        receipt_corrupt.transact(command.clone()),
        ControlStoreErrorKind::CorruptState,
    );

    assert!(store.inject_head_corruption(&scope));
    assert_kind(
        store.current_head(&scope),
        ControlStoreErrorKind::CorruptState,
    );
    assert_kind(store.transact(command), ControlStoreErrorKind::CorruptState);
}

#[test]
fn changed_id_reuse_precedes_corrupt_substituted_scope_observation() {
    let authority = active_authority();
    let mut store = FakePostgresStore::new(authority.clone(), 4).expect("fake store");
    let original_scope = scope(
        "project-1",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '1',
    );
    let substituted_scope = scope(
        "project-2",
        "snapshot-1",
        StoreRepositoryOwner::TaskLedger,
        '2',
    );
    let original = request(
        "transaction-probe",
        original_scope.clone(),
        authority.clone(),
        store
            .current_head(&original_scope)
            .expect("original genesis"),
        mutation(0),
    );
    store.transact(original).expect("original apply");

    let substituted_genesis = store
        .current_head(&substituted_scope)
        .expect("substituted genesis");
    store
        .transact(request(
            "transaction-materialize",
            substituted_scope.clone(),
            authority.clone(),
            substituted_genesis,
            mutation(1),
        ))
        .expect("materialize substituted scope");
    let changed_reuse = request(
        "transaction-probe",
        substituted_scope.clone(),
        authority,
        store
            .current_head(&substituted_scope)
            .expect("substituted current"),
        mutation(2),
    );
    assert!(store.inject_head_corruption(&substituted_scope));
    assert_kind(
        store.transact(changed_reuse),
        ControlStoreErrorKind::CommandSubstitution,
    );
}
