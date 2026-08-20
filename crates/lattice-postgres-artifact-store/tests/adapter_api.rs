use lattice_artifact_store::ArtifactRepository;
use lattice_postgres_artifact_store::{
    ARTIFACT_EXTENSION_ID, ARTIFACT_EXTENSION_SQL, ExtensionTarget, PostgresArtifactStore,
    verify_embedded_extension_manifest,
};

#[test]
fn adapter_implements_only_domain_repository_contract() {
    fn assert_repository<T: ArtifactRepository>() {}
    assert_repository::<PostgresArtifactStore>();
}

#[test]
fn target_and_embedded_extension_have_closed_identity() {
    assert_eq!(ARTIFACT_EXTENSION_ID, "lattice-postgres-artifact-store");
    assert!(ARTIFACT_EXTENSION_SQL.contains("artifact_store_commit_snapshot_v1"));
    let manifest = verify_embedded_extension_manifest().expect("embedded manifest");
    assert_eq!(manifest.sql_bytes(), ARTIFACT_EXTENSION_SQL.len());
    assert_eq!(manifest.sql_sha256().as_str().len(), 64);
    assert_eq!(manifest.manifest_sha256().as_str().len(), 64);
    assert!(std::mem::size_of::<ExtensionTarget>() > 0);
}
