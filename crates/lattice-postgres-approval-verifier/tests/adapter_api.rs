use lattice_approval_verifier::ApprovalRepository;
use lattice_contracts::ContentDigest;
use lattice_postgres_approval_verifier::{
    ExtensionApplyOutcome, ExtensionSetupErrorKind, ExtensionTarget, PostgresApprovalVerifier,
};
use postgres::Client;

fn digest(character: char) -> ContentDigest {
    ContentDigest::from_sha256(character.to_string().repeat(64)).expect("valid digest")
}

#[test]
fn runtime_adapter_implements_the_domain_owned_repository_port() {
    fn assert_repository<T: ApprovalRepository>() {}
    assert_repository::<PostgresApprovalVerifier>();
    let _: fn(
        Client,
        ExtensionTarget,
    ) -> Result<
        PostgresApprovalVerifier,
        lattice_approval_verifier::ApprovalRepositoryError,
    > = PostgresApprovalVerifier::new;
}

#[test]
fn setup_contract_accepts_only_a_bounded_prevalidated_target() {
    let target = ExtensionTarget::new(
        "lattice_task024_fixture".to_owned(),
        digest('a'),
        digest('b'),
        digest('c'),
    )
    .expect("bounded target");

    assert_eq!(target.database_name(), "lattice_task024_fixture");
    assert_eq!(target.database_identity_digest(), &digest('a'));
    assert_eq!(target.global_manifest_digest(), &digest('b'));
    assert_eq!(target.memory_manifest_digest(), &digest('c'));
    assert_eq!(
        ExtensionApplyOutcome::Installed,
        ExtensionApplyOutcome::Installed
    );
    assert_eq!(
        ExtensionApplyOutcome::AlreadyCurrent,
        ExtensionApplyOutcome::AlreadyCurrent
    );

    let error = ExtensionTarget::new(
        "postgres://credential@host/db".to_owned(),
        digest('a'),
        digest('b'),
        digest('c'),
    )
    .expect_err("DSN-like target must fail closed");
    assert_eq!(error.kind(), ExtensionSetupErrorKind::InvalidTarget);
    assert_eq!(error.code(), "APPROVAL_EXTENSION_INVALID_TARGET");
}
