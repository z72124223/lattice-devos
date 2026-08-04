use lattice_postgres_store::{
    BootstrapAdmission, DatabaseRole, MigrationApplyOutcome, MigrationTarget,
    PostgresSchemaEvidence, PostgresStoreSetupError, apply_migrations, verify_postgres_schema,
};
use postgres::Client;

#[test]
fn setup_api_requires_a_caller_owned_client_and_exact_target() {
    let _: fn(
        &mut Client,
        &MigrationTarget,
    ) -> Result<MigrationApplyOutcome, PostgresStoreSetupError> = apply_migrations;
    let _: fn(
        &mut Client,
        &MigrationTarget,
        DatabaseRole,
    ) -> Result<PostgresSchemaEvidence, PostgresStoreSetupError> = verify_postgres_schema;
}

#[test]
fn apply_outcomes_do_not_claim_a_store_transaction_or_domain_receipt() {
    let applied = MigrationApplyOutcome::Applied {
        executable_count: 1,
    };
    assert_eq!(applied.executable_count(), 1);
    assert!(!applied.was_current());

    let current = MigrationApplyOutcome::AlreadyCurrent;
    assert_eq!(current.executable_count(), 0);
    assert!(current.was_current());
}

#[test]
fn schema_evidence_can_only_represent_stopped_without_a_leader_in_task019() {
    assert_eq!(
        BootstrapAdmission::ALL,
        [BootstrapAdmission::StoppedNoLeader]
    );
    assert_eq!(
        BootstrapAdmission::StoppedNoLeader.as_str(),
        "STOPPED_NO_LEADER"
    );
}
