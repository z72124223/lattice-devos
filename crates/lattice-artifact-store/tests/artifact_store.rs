use lattice_artifact_store::{
    ArtifactLimitKind, ArtifactStoreLimits, HARD_MAX_ACTIVE_READS_PER_OBJECT,
    HARD_MAX_ACTIVE_REFERENCES_PER_OBJECT, HARD_MAX_BUNDLE_DEPTH, HARD_MAX_BUNDLE_ENTRIES,
    HARD_MAX_COMMANDS_PER_OBJECT, HARD_MAX_MANIFEST_BYTES, HARD_MAX_OBJECT_BYTES,
    HARD_MAX_UNIQUE_BYTES_PER_STORE,
};

#[test]
fn hard_limits_are_finite_and_lower_config_can_only_tighten_them() {
    assert_eq!(HARD_MAX_OBJECT_BYTES, 1_073_741_824);
    assert_eq!(HARD_MAX_MANIFEST_BYTES, 65_536);
    assert_eq!(HARD_MAX_ACTIVE_REFERENCES_PER_OBJECT, 65_536);
    assert_eq!(HARD_MAX_ACTIVE_READS_PER_OBJECT, 4_096);
    assert_eq!(HARD_MAX_COMMANDS_PER_OBJECT, 1_000_000);
    assert_eq!(HARD_MAX_BUNDLE_ENTRIES, 100_000);
    assert_eq!(HARD_MAX_BUNDLE_DEPTH, 64);

    let defaults = ArtifactStoreLimits::hard_maximums();
    assert_eq!(defaults.max_object_bytes(), HARD_MAX_OBJECT_BYTES);
    assert_eq!(defaults.max_manifest_bytes(), HARD_MAX_MANIFEST_BYTES);

    let tightened = ArtifactStoreLimits::new(1_024, 4_096, 8, 4, 100, 32, 4).expect("lower limits");
    assert_eq!(tightened.max_object_bytes(), 1_024);
    assert_eq!(tightened.max_manifest_bytes(), 4_096);

    assert!(
        ArtifactStoreLimits::new(
            HARD_MAX_OBJECT_BYTES + 1,
            HARD_MAX_MANIFEST_BYTES,
            HARD_MAX_ACTIVE_REFERENCES_PER_OBJECT,
            HARD_MAX_ACTIVE_READS_PER_OBJECT,
            HARD_MAX_COMMANDS_PER_OBJECT,
            HARD_MAX_BUNDLE_ENTRIES,
            HARD_MAX_BUNDLE_DEPTH,
        )
        .is_err()
    );

    let store_tightened = defaults
        .tighten(ArtifactLimitKind::UniqueBytesPerStore, 8_192)
        .expect("tighten store bytes");
    assert_eq!(
        store_tightened.get(ArtifactLimitKind::UniqueBytesPerStore),
        8_192
    );
    assert_eq!(
        defaults.get(ArtifactLimitKind::UniqueBytesPerStore),
        HARD_MAX_UNIQUE_BYTES_PER_STORE
    );
    assert!(
        defaults
            .tighten(
                ArtifactLimitKind::UniqueBytesPerStore,
                HARD_MAX_UNIQUE_BYTES_PER_STORE + 1,
            )
            .is_err()
    );
}

#[test]
fn limit_snapshot_digest_is_deterministic_and_binds_every_change() {
    let defaults = ArtifactStoreLimits::hard_maximums();
    let same = ArtifactStoreLimits::hard_maximums();
    let tightened = defaults
        .tighten(ArtifactLimitKind::ReferencesPerTask, 99)
        .expect("tightened");

    assert_eq!(
        defaults.limit_snapshot_digest().expect("default digest"),
        same.limit_snapshot_digest().expect("same digest")
    );
    assert_ne!(
        defaults.limit_snapshot_digest().expect("default digest"),
        tightened.limit_snapshot_digest().expect("tight digest")
    );
}

#[test]
fn a_tightened_limit_cannot_be_raised_again() {
    let tightened = ArtifactStoreLimits::hard_maximums()
        .tighten(ArtifactLimitKind::ReferencesPerTask, 99)
        .expect("first tightening");

    let error = tightened
        .tighten(ArtifactLimitKind::ReferencesPerTask, 100)
        .expect_err("a configured limit must never be raised");

    assert_eq!(error.code(), "ARTIFACT_INVALID_LIMIT");
}

#[test]
fn every_limit_rejects_zero_and_values_above_its_hard_maximum() {
    let defaults = ArtifactStoreLimits::hard_maximums();

    for kind in ArtifactLimitKind::ALL {
        let hard_maximum = defaults.get(kind);
        assert!(
            defaults.tighten(kind, hard_maximum).is_ok(),
            "{} must accept its exact hard maximum",
            kind.as_str()
        );
        assert!(
            defaults.tighten(kind, 0).is_err(),
            "{} must reject zero",
            kind.as_str()
        );
        assert!(
            defaults
                .tighten(kind, hard_maximum.saturating_add(1))
                .is_err(),
            "{} must reject values above its hard maximum",
            kind.as_str()
        );
    }
}
