use lattice_postgres_artifact_store::ARTIFACT_EXTENSION_SQL;

#[test]
fn extension_is_fixed_function_metadata_only_and_closed_to_runtime_dml() {
    for required in [
        "CREATE SCHEMA artifact_store AUTHORIZATION lattice_migrator",
        "artifact_store_head",
        "artifact_store_transition",
        "artifact_store_load_for_update_v1",
        "artifact_store_commit_snapshot_v1",
        "artifact_store_load_current_v1",
        "current_setting('transaction_isolation') <> 'serializable'",
        "REVOKE ALL ON ALL TABLES IN SCHEMA artifact_store FROM lattice_runtime",
    ] {
        assert!(
            ARTIFACT_EXTENSION_SQL.contains(required),
            "missing {required}"
        );
    }
    for forbidden in [
        "artifact_bytes",
        "caller_path",
        "recursive_delete",
        "DROP SCHEMA",
        "GRANT INSERT ON",
        "GRANT UPDATE ON",
        "GRANT DELETE ON",
    ] {
        assert!(
            !ARTIFACT_EXTENSION_SQL.contains(forbidden),
            "forbidden {forbidden}"
        );
    }
}
