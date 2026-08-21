use std::collections::HashSet;

use lattice_core::{
    ComponentFailurePolicy, ComponentHealth, ComponentId, ComponentMode, RecoveryAction,
    RuntimeState, bootstrap_manifest, platform_name, runtime_state,
};

#[test]
fn manifest_identifies_the_four_part_lattice_runtime() {
    assert_eq!(platform_name(), "LATTICE Runtime");

    let manifest = bootstrap_manifest();
    let ids: HashSet<_> = manifest.iter().map(|component| component.id).collect();

    assert_eq!(manifest.len(), 4);
    assert_eq!(ids.len(), manifest.len());
    assert_eq!(
        ids,
        HashSet::from([
            ComponentId::Lattice,
            ComponentId::PostgreSql,
            ComponentId::Graphify,
            ComponentId::Hermes,
        ])
    );
}

#[test]
fn manifest_preserves_one_truth_and_two_degradable_intelligence_lanes() {
    let manifest = bootstrap_manifest();

    let mode = |id| {
        manifest
            .iter()
            .find(|component| component.id == id)
            .map(|component| component.mode)
    };

    assert_eq!(mode(ComponentId::Lattice), Some(ComponentMode::ControlCore));
    assert_eq!(
        mode(ComponentId::PostgreSql),
        Some(ComponentMode::DurableTruth)
    );
    assert_eq!(
        mode(ComponentId::Graphify),
        Some(ComponentMode::DerivedRelationshipMemory)
    );
    assert_eq!(
        mode(ComponentId::Hermes),
        Some(ComponentMode::ReflectiveAdvisor)
    );
}

#[test]
fn postgresql_is_required_but_graphify_and_hermes_degrade_safely() {
    assert_eq!(
        runtime_state(
            ComponentHealth::Healthy,
            ComponentHealth::Healthy,
            ComponentHealth::Healthy,
            ComponentHealth::Healthy,
        ),
        RuntimeState::Ready
    );
    assert_eq!(
        runtime_state(
            ComponentHealth::Healthy,
            ComponentHealth::Healthy,
            ComponentHealth::Unavailable,
            ComponentHealth::Healthy,
        ),
        RuntimeState::Degraded
    );
    assert_eq!(
        runtime_state(
            ComponentHealth::Healthy,
            ComponentHealth::Unavailable,
            ComponentHealth::Healthy,
            ComponentHealth::Healthy,
        ),
        RuntimeState::Unavailable
    );
    assert_eq!(
        runtime_state(
            ComponentHealth::Unavailable,
            ComponentHealth::Healthy,
            ComponentHealth::Healthy,
            ComponentHealth::Healthy,
        ),
        RuntimeState::Unavailable
    );
}

#[test]
fn graphify_is_rebuilt_from_postgresql_after_a_degraded_failure() {
    let graphify = bootstrap_manifest()
        .iter()
        .find(|component| component.id == ComponentId::Graphify)
        .expect("graphify component");

    assert_eq!(graphify.failure_policy, ComponentFailurePolicy::Degraded);
    assert_eq!(
        graphify.recovery_action,
        RecoveryAction::RebuildFromPostgreSql
    );
}

#[test]
fn hermes_degrades_and_recomputes_suggestions_without_writing_truth() {
    let hermes = bootstrap_manifest()
        .iter()
        .find(|component| component.id == ComponentId::Hermes)
        .expect("hermes component");

    assert_eq!(hermes.failure_policy, ComponentFailurePolicy::Degraded);
    assert_eq!(
        hermes.recovery_action,
        RecoveryAction::RecomputeFromFactsAndGraph
    );
}
