use lattice_contracts::{
    ContentDigest, ProjectId, StoreAuthorityHead, WriterLeaseAuthorityHead,
    WriterLeaseAuthorityReceipt,
};
use lattice_postgres_writer_lease::{
    ExtensionTarget, PostgresWriterLease, V3ExtensionTarget, V4ExtensionTarget, V5ExtensionTarget,
};
use lattice_writer_lease::{
    WriterLeaseAcquireRequest, WriterLeaseProjectEvidence, WriterLeaseReleaseRequest,
    WriterLeaseRepository, WriterLeaseRepositoryError,
};
use postgres::Client;

#[test]
fn concrete_adapter_implements_the_domain_owned_repository_port() {
    fn assert_repository<T: WriterLeaseRepository>() {}
    assert_repository::<PostgresWriterLease>();

    let _: for<'a> fn(
        Client,
        ExtensionTarget,
        &'a StoreAuthorityHead,
        u32,
    ) -> Result<
        PostgresWriterLease,
        lattice_writer_lease::WriterLeaseRepositoryError,
    > = PostgresWriterLease::new;
    let _: for<'a> fn(
        Client,
        &'a V3ExtensionTarget,
        &'a StoreAuthorityHead,
        u32,
    ) -> Result<
        PostgresWriterLease,
        lattice_writer_lease::WriterLeaseRepositoryError,
    > = PostgresWriterLease::new_v3;
    let _: for<'a> fn(
        Client,
        &'a V4ExtensionTarget,
        &'a StoreAuthorityHead,
        u32,
    ) -> Result<
        PostgresWriterLease,
        lattice_writer_lease::WriterLeaseRepositoryError,
    > = PostgresWriterLease::new_v4_v7;
    let _: for<'a> fn(
        Client,
        &'a V5ExtensionTarget,
        &'a StoreAuthorityHead,
        u32,
    ) -> Result<
        PostgresWriterLease,
        lattice_writer_lease::WriterLeaseRepositoryError,
    > = PostgresWriterLease::new_v5_v7;
    let _: fn(
        &mut PostgresWriterLease,
        &ProjectId,
    ) -> Result<Option<WriterLeaseProjectEvidence>, WriterLeaseRepositoryError> =
        PostgresWriterLease::inspect_project;
    let _: fn(
        &mut PostgresWriterLease,
        &ProjectId,
        &ContentDigest,
    ) -> Result<Option<WriterLeaseAuthorityReceipt>, WriterLeaseRepositoryError> =
        PostgresWriterLease::inspect_historical_authority;
    let _: fn(
        &mut PostgresWriterLease,
        WriterLeaseReleaseRequest,
        WriterLeaseAcquireRequest,
    ) -> Result<WriterLeaseAuthorityHead, WriterLeaseRepositoryError> =
        PostgresWriterLease::rotate_exact;
    let _: fn(
        &mut PostgresWriterLease,
        &ProjectId,
        &str,
    ) -> Result<Option<WriterLeaseReleaseRequest>, WriterLeaseRepositoryError> =
        PostgresWriterLease::replay_applied_release_request;
    let _: fn(
        &mut PostgresWriterLease,
        &ProjectId,
        &str,
    ) -> Result<Option<WriterLeaseAcquireRequest>, WriterLeaseRepositoryError> =
        PostgresWriterLease::replay_applied_acquire_request;
    let _: fn(
        String,
        ContentDigest,
        ContentDigest,
        ContentDigest,
    ) -> Result<ExtensionTarget, lattice_postgres_writer_lease::ExtensionSetupError> =
        ExtensionTarget::new;
    let _: fn(&mut PostgresWriterLease, &ProjectId) = |_adapter, _project| {};
}

#[test]
fn adapter_routes_only_the_two_ordinal_bound_calls_by_version() {
    let adapter = include_str!("../src/adapter.rs");
    assert!(adapter.contains("writer_lease_bind_runtime_v2"));
    assert!(adapter.contains("writer_lease_load_for_update_v2"));
    assert!(adapter.contains("writer_lease_bind_runtime_v3"));
    assert!(adapter.contains("writer_lease_load_for_update_v3"));
    assert!(adapter.contains("writer_lease_bind_runtime_v4"));
    assert!(adapter.contains("writer_lease_load_for_update_v4"));
    assert!(adapter.contains("writer_lease_bind_runtime_v5"));
    assert!(adapter.contains("writer_lease_load_for_update_v5"));
    assert!(!adapter.contains("new_v3_v7"));
    assert!(!adapter.contains("writer_lease_bind_runtime_v1"));
    assert!(!adapter.contains("writer_lease_load_for_update_v1"));
    for retained in [
        "writer_lease_commit_plan_v1",
        "writer_lease_load_commands_v1",
        "writer_lease_load_transitions_v1",
        "writer_lease_load_current_v1",
        "writer_lease_assert_current_v1",
    ] {
        assert!(
            adapter.contains(retained),
            "missing retained v1 call: {retained}"
        );
    }
}
