use lattice_postgres_store::{POSTGRES_SCHEMA_VERSION, PostgresProjectRegistryErrorKind};

#[test]
fn typed_project_registry_adapter_selects_only_exact_v5_or_v7_profiles() {
    assert_eq!(POSTGRES_SCHEMA_VERSION, 8);
    let source = include_str!("../src/project_registry.rs");
    for contract in [
        "const FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION: u16 = 5;",
        "const CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION: u16 = 7;",
        "FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION => Ok(Self::FrozenV5)",
        "CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION => Ok(Self::CurrentV7)",
        "RegistrySqlProfile::from_schema_version(evidence.global_schema_version())",
    ] {
        assert!(
            source.contains(contract),
            "missing profile contract: {contract}"
        );
    }
    assert!(!source.contains("schema_version >="));
    assert!(!source.contains("schema_version >"));
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
