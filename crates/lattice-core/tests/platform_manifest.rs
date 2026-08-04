use std::collections::HashSet;

use lattice_core::{ComponentId, ComponentMode, bootstrap_manifest, platform_name};

#[test]
fn manifest_identifies_the_general_ai_platform() {
    assert_eq!(platform_name(), "LATTICE DevOS");

    let manifest = bootstrap_manifest();
    let ids: HashSet<_> = manifest.iter().map(|component| component.id).collect();

    assert_eq!(manifest.len(), 8);
    assert_eq!(ids.len(), manifest.len());
    assert_eq!(
        ids,
        HashSet::from([
            ComponentId::RustCore,
            ComponentId::OpenClaw,
            ComponentId::PostgreSql,
            ComponentId::Codex,
            ComponentId::Graphify,
            ComponentId::Hermes,
            ComponentId::CodebaseMemory,
            ComponentId::Guardian,
        ])
    );
}

#[test]
fn manifest_preserves_one_gateway_truth_writer_and_read_only_lanes() {
    let manifest = bootstrap_manifest();

    let mode = |id| {
        manifest
            .iter()
            .find(|component| component.id == id)
            .map(|component| component.mode)
    };

    assert_eq!(mode(ComponentId::OpenClaw), Some(ComponentMode::Gateway));
    assert_eq!(
        mode(ComponentId::RustCore),
        Some(ComponentMode::ControlCore)
    );
    assert_eq!(
        mode(ComponentId::PostgreSql),
        Some(ComponentMode::DurableTruth)
    );
    assert_eq!(mode(ComponentId::Codex), Some(ComponentMode::SoleWriter));
    assert_eq!(
        mode(ComponentId::Graphify),
        Some(ComponentMode::ReadOnlyEvidence)
    );
    assert_eq!(
        mode(ComponentId::Hermes),
        Some(ComponentMode::ReadOnlyEvidence)
    );
    assert_eq!(
        mode(ComponentId::CodebaseMemory),
        Some(ComponentMode::DurableMemory)
    );
    assert_eq!(
        mode(ComponentId::Guardian),
        Some(ComponentMode::ApprovalGated)
    );
}
