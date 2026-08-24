use lattice_postgres_store::{
    FOREMAN_COORDINATION_EVENT_IDENTITY, FOREMAN_COORDINATION_MIGRATION_ID,
    FOREMAN_COORDINATION_MIGRATION_PATH, FOREMAN_COORDINATION_STREAM_IDENTITY,
    ForemanSchemaV6Candidate, ForemanSchemaV6CatalogAcl, POSTGRES_SCHEMA_VERSION,
    SchemaV6ProfileError, WriterLeaseV3Profile, migration_manifest,
    verify_foreman_schema_v6_profile,
};

const SQL: &[u8] = include_bytes!("../../../db/migrations/0007_foreman_coordination.sql");

fn candidate() -> ForemanSchemaV6Candidate {
    ForemanSchemaV6Candidate::from_migration_bytes(
        7,
        FOREMAN_COORDINATION_MIGRATION_ID,
        FOREMAN_COORDINATION_MIGRATION_PATH,
        6,
        6..=6,
        6..=6,
        FOREMAN_COORDINATION_STREAM_IDENTITY,
        FOREMAN_COORDINATION_EVENT_IDENTITY,
        SQL,
    )
    .expect("exact candidate")
}

fn catalog() -> ForemanSchemaV6CatalogAcl {
    ForemanSchemaV6CatalogAcl::exact_foreman_coordination()
}

#[test]
fn task079_appends_exact_0007_without_changing_the_v5_prefix() {
    assert_eq!(POSTGRES_SCHEMA_VERSION, 6);
    assert_eq!(migration_manifest().len(), 7);
    assert_eq!(
        migration_manifest()[6].id(),
        FOREMAN_COORDINATION_MIGRATION_ID
    );
    assert_eq!(migration_manifest()[6].bytes(), SQL);
    assert_eq!(
        candidate().manifest_sha256(),
        "4a004488543ce39266ec046607a938958da51567fe747cb22f2e731f30b36ed7"
    );
}

#[test]
fn schema_v6_accepts_only_exact_0007_stream_event_and_catalog_acl() {
    let candidate = candidate();
    let verified =
        verify_foreman_schema_v6_profile(&candidate, &catalog(), WriterLeaseV3Profile::Bridge)
            .expect("exact bridge profile");
    assert_eq!(verified.schema_version(), 6);
    assert_eq!(verified.migration_ordinal(), 7);
    assert_eq!(verified.stream_identity(), "FOREMAN_COORDINATION");
    assert_eq!(verified.event_identity(), "FOREMAN_SNAPSHOT_RECORDED");
    assert_eq!(verified.runtime_writer_functions(), 0);
}

#[test]
fn schema_v6_rejects_missing_skipped_unknown_or_substituted_migration_identity() {
    for (ordinal, id, path, schema, stream, event) in [
        (
            8,
            FOREMAN_COORDINATION_MIGRATION_ID,
            FOREMAN_COORDINATION_MIGRATION_PATH,
            6,
            FOREMAN_COORDINATION_STREAM_IDENTITY,
            FOREMAN_COORDINATION_EVENT_IDENTITY,
        ),
        (
            7,
            "0007_other",
            FOREMAN_COORDINATION_MIGRATION_PATH,
            6,
            FOREMAN_COORDINATION_STREAM_IDENTITY,
            FOREMAN_COORDINATION_EVENT_IDENTITY,
        ),
        (
            7,
            FOREMAN_COORDINATION_MIGRATION_ID,
            "db/migrations/0008_foreman_coordination.sql",
            6,
            FOREMAN_COORDINATION_STREAM_IDENTITY,
            FOREMAN_COORDINATION_EVENT_IDENTITY,
        ),
        (
            7,
            FOREMAN_COORDINATION_MIGRATION_ID,
            FOREMAN_COORDINATION_MIGRATION_PATH,
            7,
            FOREMAN_COORDINATION_STREAM_IDENTITY,
            FOREMAN_COORDINATION_EVENT_IDENTITY,
        ),
        (
            7,
            FOREMAN_COORDINATION_MIGRATION_ID,
            FOREMAN_COORDINATION_MIGRATION_PATH,
            6,
            "TASK",
            FOREMAN_COORDINATION_EVENT_IDENTITY,
        ),
        (
            7,
            FOREMAN_COORDINATION_MIGRATION_ID,
            FOREMAN_COORDINATION_MIGRATION_PATH,
            6,
            FOREMAN_COORDINATION_STREAM_IDENTITY,
            "DIAGNOSTIC",
        ),
    ] {
        let error = ForemanSchemaV6Candidate::from_migration_bytes(
            ordinal,
            id,
            path,
            schema,
            6..=6,
            6..=6,
            stream,
            event,
            SQL,
        )
        .expect_err("substitution must fail closed");
        assert_eq!(error, SchemaV6ProfileError::MigrationIdentity);
    }
    assert_eq!(
        ForemanSchemaV6Candidate::from_migration_bytes(
            7,
            FOREMAN_COORDINATION_MIGRATION_ID,
            FOREMAN_COORDINATION_MIGRATION_PATH,
            6,
            6..=6,
            6..=6,
            FOREMAN_COORDINATION_STREAM_IDENTITY,
            FOREMAN_COORDINATION_EVENT_IDENTITY,
            b"",
        )
        .expect_err("missing migration bytes"),
        SchemaV6ProfileError::MigrationMissing
    );
}

#[test]
fn schema_v6_rejects_acl_catalog_and_writer_phase_drift() {
    let exact = catalog();
    for drift in [
        exact.with_table_count(16),
        exact.with_retained_function_count(48),
        exact.with_runtime_function_count(20),
        exact.with_foreman_table(false),
        exact.with_record_function(false),
        exact.with_read_function(false),
        exact.with_runtime_record_execute(false),
        exact.with_runtime_read_execute(false),
        exact.with_direct_table_privilege(true),
        exact.with_unexpected_object_count(1),
        exact.with_writer_assertion_present(false),
        exact.with_writer_assertion_before_append(false),
        exact.with_atomic_foreman_finalize(false),
    ] {
        assert_eq!(
            verify_foreman_schema_v6_profile(&candidate(), &drift, WriterLeaseV3Profile::Bridge,)
                .expect_err("catalog/ACL drift must fail closed"),
            SchemaV6ProfileError::CatalogAcl
        );
    }
    assert_eq!(
        verify_foreman_schema_v6_profile(
            &candidate(),
            &catalog(),
            WriterLeaseV3Profile::V2Current,
        )
        .expect_err("v2 cannot be a schema-v6 bridge"),
        SchemaV6ProfileError::WriterProfile
    );
}

#[test]
fn current_profile_requires_exact_v3_rebind_and_seven_writer_functions() {
    let verified =
        verify_foreman_schema_v6_profile(&candidate(), &catalog(), WriterLeaseV3Profile::Current)
            .expect("exact current profile");
    assert_eq!(verified.runtime_writer_functions(), 7);

    for profile in [
        WriterLeaseV3Profile::Bridge,
        WriterLeaseV3Profile::BridgePending,
    ] {
        let verified = verify_foreman_schema_v6_profile(&candidate(), &catalog(), profile)
            .expect("closed migration profile");
        assert_eq!(verified.runtime_writer_functions(), 0);
    }
}
