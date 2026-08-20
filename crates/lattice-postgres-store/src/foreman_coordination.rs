//! Typed foreman coordination Port bound to the durable Task Ledger repository.

use lattice_contracts::{StoreAuthorityHead, WriterLeaseAuthorityHead};
use lattice_foreman_state::ForemanSnapshot;
use lattice_ports::{
    ForemanAppendReceipt, ForemanCoordinationError, ForemanCoordinationErrorKind,
    ForemanCoordinationPort, ForemanCoordinationResult,
};
use lattice_task_ledger::{
    CommandId, CorrelationId, ForemanAppendMetadata, foreman_coordination_identity,
    plan_foreman_snapshot_append,
};

use crate::{PostgresTaskLedger, PostgresTaskLedgerErrorKind};

/// Production Port adapter. The Task Ledger remains the only persisted truth;
/// this type owns no cache, dashboard state, or independent current snapshot.
pub struct PostgresForemanCoordination {
    ledger: PostgresTaskLedger,
    store_authority: StoreAuthorityHead,
}

impl PostgresForemanCoordination {
    #[must_use]
    pub const fn new(ledger: PostgresTaskLedger, store_authority: StoreAuthorityHead) -> Self {
        Self {
            ledger,
            store_authority,
        }
    }
}

impl ForemanCoordinationPort for PostgresForemanCoordination {
    fn append_snapshot(
        &mut self,
        command_id: &str,
        correlation_id: &str,
        occurred_at: &str,
        snapshot: ForemanSnapshot,
        writer: &WriterLeaseAuthorityHead,
    ) -> ForemanCoordinationResult<ForemanAppendReceipt> {
        let records = self.ledger.load_foreman_records().map_err(map_error)?;
        let identity = foreman_coordination_identity().map_err(|_| malformed())?;
        let loaded = self.ledger.load_stream(identity).map_err(map_error)?;
        let metadata = ForemanAppendMetadata::new(
            CommandId::new(command_id).map_err(|_| malformed())?,
            CorrelationId::new(correlation_id).map_err(|_| malformed())?,
            occurred_at,
        )
        .map_err(|_| malformed())?;
        let plan =
            plan_foreman_snapshot_append(loaded.stream(), &records, metadata, snapshot.clone())
                .map_err(|_| malformed())?;
        let event_digest = plan
            .ledger_plan()
            .receipt()
            .event_digest()
            .cloned()
            .ok_or_else(malformed)?;
        let checkpoint_digest = plan
            .ledger_plan()
            .next_checkpoint()
            .checkpoint_digest()
            .clone();
        let execution = self
            .ledger
            .execute_foreman(&plan, &self.store_authority, writer)
            .map_err(map_error)?;
        ForemanAppendReceipt::new(
            event_digest,
            checkpoint_digest,
            snapshot.generation(),
            execution.is_exact_retry(),
        )
    }

    fn load_snapshots(&mut self) -> ForemanCoordinationResult<Vec<ForemanSnapshot>> {
        self.ledger
            .load_foreman_records()
            .map(|records| {
                records
                    .into_iter()
                    .map(|record| record.snapshot().clone())
                    .collect()
            })
            .map_err(map_error)
    }
}

const fn malformed() -> ForemanCoordinationError {
    ForemanCoordinationError::new(
        ForemanCoordinationErrorKind::Malformed,
        "FOREMAN_COORDINATION_MALFORMED",
    )
}

fn map_error(error: crate::PostgresTaskLedgerError) -> ForemanCoordinationError {
    let kind = match error.kind() {
        PostgresTaskLedgerErrorKind::Malformed => ForemanCoordinationErrorKind::Malformed,
        PostgresTaskLedgerErrorKind::AuthorityMismatch
        | PostgresTaskLedgerErrorKind::AdmissionDenied => ForemanCoordinationErrorKind::StaleWriter,
        PostgresTaskLedgerErrorKind::CommandSubstitution
        | PostgresTaskLedgerErrorKind::SerializationExhausted => {
            ForemanCoordinationErrorKind::Conflict
        }
        PostgresTaskLedgerErrorKind::CheckpointCorrupt
        | PostgresTaskLedgerErrorKind::RetainedRowCorrupt
        | PostgresTaskLedgerErrorKind::PhysicalStateMismatch
        | PostgresTaskLedgerErrorKind::RevisionOverflow => ForemanCoordinationErrorKind::Corrupt,
        PostgresTaskLedgerErrorKind::CommitOutcomeUnknown => {
            ForemanCoordinationErrorKind::OutcomeUnknown
        }
        PostgresTaskLedgerErrorKind::TransactionFailed
        | PostgresTaskLedgerErrorKind::Unavailable => ForemanCoordinationErrorKind::Unavailable,
    };
    ForemanCoordinationError::new(kind, error.code())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PostgresTaskLedgerError, PostgresTaskLedgerErrorKind};

    #[test]
    fn error_mapping_is_closed_and_preserves_unknown_commit() {
        assert_eq!(
            map_error(PostgresTaskLedgerError::new(
                PostgresTaskLedgerErrorKind::AuthorityMismatch
            ))
            .kind(),
            ForemanCoordinationErrorKind::StaleWriter
        );
        assert_eq!(
            map_error(PostgresTaskLedgerError::new(
                PostgresTaskLedgerErrorKind::CommitOutcomeUnknown
            ))
            .kind(),
            ForemanCoordinationErrorKind::OutcomeUnknown
        );
    }

    #[test]
    fn adapter_satisfies_the_typed_port_without_exposing_a_second_store() {
        fn assert_port<T: ForemanCoordinationPort>() {}
        assert_port::<PostgresForemanCoordination>();
    }
}
