use resource_aware_scheduler_phase0::{
    Observation, ObservationFault, ObservationSource, ObservationState, SampleFailure,
};

#[test]
fn observed_values_are_fresh_then_stale_without_becoming_unknown() {
    let sample = Observation::observed(ObservationSource::WindowsPerfCounter, 100, 110, 42_u64)
        .expect("valid sample");

    assert_eq!(sample.state_at(109), Ok(ObservationState::Fresh));
    assert_eq!(sample.state_at(110), Ok(ObservationState::Fresh));
    assert_eq!(sample.state_at(111), Ok(ObservationState::Stale));
    assert_eq!(sample.value(), Some(&42));
}

#[test]
fn query_failure_and_unknown_are_explicit_and_never_substituted_with_zero() {
    let failed = Observation::<u64>::failed(
        ObservationSource::WindowsCim,
        100,
        SampleFailure::QueryFailed,
    );
    let unknown = Observation::<u64>::unknown(ObservationSource::CodexRateLimits);

    assert_eq!(
        failed.state_at(101),
        Ok(ObservationState::Failed(SampleFailure::QueryFailed))
    );
    assert_eq!(failed.value(), None);
    assert_eq!(unknown.state_at(101), Ok(ObservationState::Unknown));
    assert_eq!(unknown.value(), None);
}

#[test]
fn future_or_inverted_observation_windows_fail_closed() {
    assert_eq!(
        Observation::observed(ObservationSource::NvidiaSmi, 110, 109, 1_u64),
        Err(ObservationFault::InvalidValidityWindow)
    );

    let sample =
        Observation::observed(ObservationSource::NvidiaSmi, 110, 120, 1_u64).expect("valid sample");
    assert_eq!(
        sample.state_at(109),
        Err(ObservationFault::FutureObservation)
    );
}
