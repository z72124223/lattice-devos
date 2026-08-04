use lattice_postgres_store::{PostgresTaskLedgerError, PostgresTaskLedgerErrorKind};

#[test]
fn postgres_task_ledger_errors_are_static_and_exhaustive() {
    let expected = [
        "POSTGRES_TASK_LEDGER_MALFORMED",
        "POSTGRES_TASK_LEDGER_COMMAND_SUBSTITUTED",
        "POSTGRES_TASK_LEDGER_ADMISSION_DENIED",
        "POSTGRES_TASK_LEDGER_AUTHORITY_MISMATCH",
        "POSTGRES_TASK_LEDGER_PHYSICAL_STATE_MISMATCH",
        "POSTGRES_TASK_LEDGER_CHECKPOINT_CORRUPT",
        "POSTGRES_TASK_LEDGER_RETAINED_ROW_CORRUPT",
        "POSTGRES_TASK_LEDGER_REVISION_OVERFLOW",
        "POSTGRES_TASK_LEDGER_SERIALIZATION_EXHAUSTED",
        "POSTGRES_TASK_LEDGER_TRANSACTION_FAILED",
        "POSTGRES_TASK_LEDGER_UNAVAILABLE",
        "POSTGRES_TASK_LEDGER_COMMIT_OUTCOME_UNKNOWN",
    ];

    assert_eq!(PostgresTaskLedgerErrorKind::ALL.len(), expected.len());
    for (kind, code) in PostgresTaskLedgerErrorKind::ALL.into_iter().zip(expected) {
        let error = PostgresTaskLedgerError::new(kind);
        assert_eq!(error.kind(), kind);
        assert_eq!(error.code(), code);
        assert_eq!(error.to_string(), code);
        assert!(!format!("{error:?}").contains("postgres://"));
    }
}
