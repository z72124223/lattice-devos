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
