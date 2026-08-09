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
