use lattice_contracts::{ContentDigest, ProjectId, StoreAuthorityHead};
use lattice_postgres_writer_lease::{ExtensionTarget, PostgresWriterLease};
use lattice_writer_lease::{
    WriterLeaseProjectEvidence, WriterLeaseRepository, WriterLeaseRepositoryError,
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
    let _: fn(
        &mut PostgresWriterLease,
        &ProjectId,
    ) -> Result<Option<WriterLeaseProjectEvidence>, WriterLeaseRepositoryError> =
        PostgresWriterLease::inspect_project;
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
fn adapter_switches_only_the_two_ordinal_bound_calls_to_v2() {
    let adapter = include_str!("../src/adapter.rs");
    assert!(adapter.contains("writer_lease_bind_runtime_v2"));
    assert!(adapter.contains("writer_lease_load_for_update_v2"));
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
