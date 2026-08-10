use lattice_postgres_store::{POSTGRES_SCHEMA_VERSION, PostgresProjectRegistryErrorKind};

#[test]
fn typed_project_registry_adapter_surface_is_schema_v4_and_closed() {
    assert_eq!(POSTGRES_SCHEMA_VERSION, 4);
    let mut codes =
        PostgresProjectRegistryErrorKind::ALL.map(PostgresProjectRegistryErrorKind::code);
    codes.sort_unstable();
    assert!(codes.windows(2).all(|pair| pair[0] != pair[1]));
    assert!(
        codes
            .iter()
            .all(|code| code.starts_with("POSTGRES_PROJECT_REGISTRY_"))
    );
}
