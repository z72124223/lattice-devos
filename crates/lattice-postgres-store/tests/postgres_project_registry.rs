use lattice_postgres_store::{POSTGRES_SCHEMA_VERSION, PostgresProjectRegistryErrorKind};

#[test]
fn typed_project_registry_adapter_retains_schema_v5_on_global_v6() {
    assert_eq!(POSTGRES_SCHEMA_VERSION, 6);
    assert!(
        include_str!("../src/project_registry.rs")
            .contains("const GLOBAL_REGISTRY_SCHEMA_VERSION: u16 = 5;")
    );
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
