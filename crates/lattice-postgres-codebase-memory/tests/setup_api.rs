use lattice_postgres_codebase_memory::{
    ExtensionApplyOutcome, ExtensionCatalogEvidence, ExtensionDatabaseRole, ExtensionSetupError,
    ExtensionTarget, apply_extension, verify_extension,
};
use postgres::Client;

#[test]
fn explicit_admin_and_closed_verifier_api_are_the_only_setup_surface() {
    let _: fn(&mut Client, &ExtensionTarget) -> Result<ExtensionApplyOutcome, ExtensionSetupError> =
        apply_extension;
    let _: fn(
        &mut Client,
        &ExtensionTarget,
        ExtensionDatabaseRole,
    ) -> Result<ExtensionCatalogEvidence, ExtensionSetupError> = verify_extension;

    let run_id = "1".repeat(32);
    let target =
        ExtensionTarget::new("lattice_task019_12345678_base", run_id).expect("marker-owned target");
    assert_eq!(target.database_name(), "lattice_task019_12345678_base");
    assert_eq!(target.expected_database_uuid().len(), 36);
    assert_eq!(
        target.expected_database_identity_digest().as_str().len(),
        64
    );
    assert!(ExtensionTarget::new("postgres", "1".repeat(32)).is_err());
    assert!(ExtensionTarget::new("lattice_task019_12345678_base", "not-a-run-id").is_err());
}

#[test]
fn memory_runner_uses_global_then_extension_lock_order() {
    let source = include_str!("../src/setup.rs");
    let global = source
        .find("&GLOBAL_MIGRATION_ADVISORY_LOCK")
        .expect("global migration lock");
    let extension = source
        .find("&EXTENSION_ADVISORY_LOCK")
        .expect("memory extension lock");
    let classify = source
        .find("let pre_state = classify_pre_state")
        .expect("classification");
    assert!(global < extension && extension < classify);
}

#[test]
fn exact_catalog_signatures_cover_same_count_definition_and_acl_drift() {
    let source = include_str!("../src/setup.rs");
    for required in [
        "RELATION_SIGNATURE_SQL",
        "COLUMN_SIGNATURE_SQL",
        "pg_catalog.pg_get_constraintdef",
        "pg_catalog.pg_get_indexdef",
        "pg_catalog.pg_get_functiondef",
        "p.prosrc",
        "TABLE_ACL_SIGNATURE_SQL",
        "FUNCTION_ACL_SIGNATURE_SQL",
        "if columns != 143 || constraints != 59",
        "SCHEMA_ACL_SIGNATURE_SQL",
        "ExactCatalogProfile::V2",
        "ExactCatalogProfile::V3",
    ] {
        assert!(
            source.contains(required),
            "missing exact catalog field {required}"
        );
    }
    assert!(
        source.matches("verify_exact_catalog_profile(").count() >= 3,
        "v2 source and both v3 apply/read-only paths must verify exact signatures"
    );
}
