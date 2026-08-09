use lattice_postgres_writer_lease::{
    WRITER_LEASE_EXTENSION_ID, WRITER_LEASE_EXTENSION_PATH, WRITER_LEASE_EXTENSION_SCHEMA_VERSION,
    verify_embedded_extension_manifest,
};

#[test]
fn embedded_extension_is_exact_closed_and_contains_no_advisory_lock() {
    let manifest = verify_embedded_extension_manifest().expect("frozen manifest");
    assert_eq!(manifest.extension_id(), WRITER_LEASE_EXTENSION_ID);
    assert_eq!(manifest.path(), WRITER_LEASE_EXTENSION_PATH);
    assert_eq!(
        manifest.schema_version(),
        WRITER_LEASE_EXTENSION_SCHEMA_VERSION
    );
    assert_eq!(manifest.sql_sha256().as_str().len(), 64);
    assert_eq!(manifest.manifest_sha256().as_str().len(), 64);

    let sql = std::str::from_utf8(manifest.bytes()).expect("UTF-8 SQL");
    for required in [
        "CREATE TABLE writer_lease.writer_lease_extension_identity",
        "CREATE TABLE writer_lease.writer_lease_extension_ledger",
        "CREATE TABLE writer_lease.writer_lease_heads",
        "CREATE TABLE writer_lease.writer_lease_commands",
        "CREATE TABLE writer_lease.writer_lease_transitions",
        "CREATE FUNCTION writer_lease.writer_lease_bind_runtime_v1",
        "CREATE FUNCTION writer_lease.writer_lease_load_for_update_v1",
        "CREATE FUNCTION writer_lease.writer_lease_commit_plan_v1",
        "CREATE FUNCTION writer_lease.writer_lease_load_commands_v1",
        "CREATE FUNCTION writer_lease.writer_lease_load_transitions_v1",
        "CREATE FUNCTION writer_lease.writer_lease_load_current_v1",
        "CREATE FUNCTION writer_lease.writer_lease_assert_current_v1",
        "repository_request_bytes bytea NOT NULL",
        "repository_request_sha256 bytea NOT NULL",
        "p_repository_request_sha256 <> pg_catalog.sha256(p_repository_request_bytes)",
        "p_next_fencing_high_water < v_head.fencing_high_water",
        "p_next_lease_revision < v_head.lease_revision",
        "current_fencing_token = fencing_high_water",
        "current_project_snapshot_id = p_project_snapshot_id",
        "current_task_spec_digest = p_task_spec_digest",
        "current_holder_process_start_identity = p_holder_process_start_identity",
        "v_command_count IS DISTINCT FROM v_command_high_water",
        "v_physical_bytes > 67108864",
        "FOR UPDATE OF h",
        "SECURITY DEFINER",
        "REVOKE ALL ON ALL TABLES IN SCHEMA writer_lease",
    ] {
        assert!(sql.contains(required), "missing SQL boundary: {required}");
    }
    assert!(!sql.to_ascii_lowercase().contains("advisory"));
    assert!(!sql.contains("GRANT SELECT ON"));
    assert!(!sql.contains("GRANT INSERT ON"));
    assert!(!sql.contains("GRANT UPDATE ON"));
    assert!(!sql.contains("GRANT DELETE ON"));
}
