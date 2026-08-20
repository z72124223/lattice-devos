use lattice_postgres_approval_verifier::{
    APPROVAL_EXTENSION_ID, APPROVAL_EXTENSION_PATH, APPROVAL_EXTENSION_SCHEMA_VERSION,
    verify_embedded_extension_manifest,
};

#[test]
fn embedded_extension_identity_is_exact_and_contains_no_protected_claim_surface() {
    let evidence = verify_embedded_extension_manifest().expect("exact embedded manifest");
    assert_eq!(evidence.extension_id(), APPROVAL_EXTENSION_ID);
    assert_eq!(evidence.path(), APPROVAL_EXTENSION_PATH);
    assert_eq!(evidence.schema_version(), APPROVAL_EXTENSION_SCHEMA_VERSION);
    assert!(evidence.byte_length() > 0);
    let sql = std::str::from_utf8(evidence.bytes()).expect("SQL is UTF-8");
    assert!(sql.contains("approval_verifier_load_for_update_v1"));
    assert!(sql.contains("approval_verifier_commit_plan_v1"));
    assert!(sql.contains("approval_verifier_load_current_v1"));
    assert!(sql.contains("approval_verifier_load_commands_v1"));
    assert!(sql.contains("approval_verifier_load_effects_v1"));
    assert!(sql.contains("current_setting('transaction_isolation') <> 'serializable'"));
    assert!(sql.contains("current_setting('transaction_isolation') <> 'repeatable read'"));
    assert!(sql.contains("current_setting('synchronous_commit') <> 'on'"));
    assert!(!sql.to_ascii_lowercase().contains("protected_claim"));
    assert!(!sql.to_ascii_lowercase().contains("claim_activation"));
}
