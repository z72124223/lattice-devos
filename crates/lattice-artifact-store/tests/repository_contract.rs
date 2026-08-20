use lattice_artifact_store::{
    ArtifactRepository, ArtifactRepositoryError, ArtifactRepositorySnapshot,
    ArtifactStagingIdentity, ArtifactStagingReservation, ArtifactStoreIdentity,
    ArtifactStoreLimits, FakeArtifactStore,
};
use lattice_contracts::{ArtifactObjectKey, ContentDigest, ProjectId, TaskId};

struct ContractRepository;

impl ArtifactRepository for ContractRepository {
    fn load(
        &mut self,
        _store_id: &ArtifactStoreIdentity,
    ) -> Result<Option<ArtifactRepositorySnapshot>, ArtifactRepositoryError> {
        Ok(None)
    }

    fn compare_and_swap(
        &mut self,
        _expected_checkpoint_digest: &lattice_contracts::ContentDigest,
        next: &ArtifactRepositorySnapshot,
    ) -> Result<ArtifactRepositorySnapshot, ArtifactRepositoryError> {
        Ok(next.clone())
    }
}

#[test]
fn repository_snapshot_round_trips_only_canonical_metadata() {
    let mut contract = ContractRepository;
    let store = FakeArtifactStore::new(
        ArtifactStoreIdentity::new("repository-contract").expect("store id"),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("store");
    let snapshot = ArtifactRepositorySnapshot::capture(&store).expect("snapshot");
    let replayed = ArtifactRepositorySnapshot::from_canonical_bytes(
        snapshot.snapshot_bytes(),
        snapshot.checkpoint_bytes(),
    )
    .expect("strict replay");

    assert_eq!(replayed.store_id(), store.store_id());
    assert_eq!(replayed.snapshot_bytes(), snapshot.snapshot_bytes());
    assert_eq!(replayed.checkpoint_bytes(), snapshot.checkpoint_bytes());
    assert_eq!(replayed.checkpoint_digest(), snapshot.checkpoint_digest());
    assert_eq!(replayed.replay().expect("owner"), store);
    assert!(
        contract
            .load(store.store_id())
            .expect("contract load")
            .is_none()
    );
}

#[test]
fn repository_snapshot_rejects_noncanonical_or_substituted_bytes() {
    let first = FakeArtifactStore::new(
        ArtifactStoreIdentity::new("repository-first").expect("store id"),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("store");
    let second = FakeArtifactStore::new(
        ArtifactStoreIdentity::new("repository-second").expect("store id"),
        ArtifactStoreLimits::hard_maximums(),
    )
    .expect("store");
    let first = ArtifactRepositorySnapshot::capture(&first).expect("first");
    let second = ArtifactRepositorySnapshot::capture(&second).expect("second");

    let mut changed = first.snapshot_bytes().to_vec();
    changed.push(b' ');
    assert!(
        ArtifactRepositorySnapshot::from_canonical_bytes(&changed, first.checkpoint_bytes())
            .is_err()
    );
    assert!(
        ArtifactRepositorySnapshot::from_canonical_bytes(
            first.snapshot_bytes(),
            second.checkpoint_bytes()
        )
        .is_err()
    );
}

#[test]
fn repository_snapshot_accepts_only_vacant_initial_and_one_exact_successor() {
    let store_id = ArtifactStoreIdentity::new("repository-successor").expect("store id");
    let initial_store = FakeArtifactStore::new(store_id, ArtifactStoreLimits::hard_maximums())
        .expect("initial store");
    let initial = ArtifactRepositorySnapshot::capture(&initial_store).expect("initial snapshot");
    initial.verify_initial().expect("vacant initial");

    let mut left_store = initial_store.clone();
    left_store
        .reserve_staging(
            "left-command",
            ArtifactStagingReservation::new(staging_identity("left"), 11, 1)
                .expect("left reservation"),
        )
        .expect("left transition");
    let left = ArtifactRepositorySnapshot::capture(&left_store).expect("left snapshot");
    initial.verify_successor(&left).expect("one successor");
    assert!(left.verify_initial().is_err());

    let mut right_store = initial_store;
    right_store
        .reserve_staging(
            "right-command",
            ArtifactStagingReservation::new(staging_identity("right"), 13, 1)
                .expect("right reservation"),
        )
        .expect("right transition");
    let right = ArtifactRepositorySnapshot::capture(&right_store).expect("right snapshot");
    assert!(left.verify_successor(&right).is_err());
    assert!(initial.verify_successor(&initial).is_err());
}

fn staging_identity(suffix: &str) -> ArtifactStagingIdentity {
    ArtifactStagingIdentity::new(
        ArtifactObjectKey::new(
            ProjectId::new("repository-successor-project").expect("project"),
            ContentDigest::from_sha256(format!("{:0>64}", suffix.len())).expect("digest"),
        ),
        TaskId::new("repository-successor-task").expect("task"),
        format!("stream-{suffix}"),
    )
    .expect("staging identity")
}
