//! Typed foreman coordination Port bound to the durable Task Ledger repository.

use lattice_contracts::{StoreAuthorityHead, WriterLeaseAuthorityHead};
use lattice_foreman_state::{ForemanCheckpointIntent, ForemanSnapshot, reconstruct};
use lattice_ports::{
    ForemanAppendReceipt, ForemanCheckpointReplay, ForemanCoordinationError,
    ForemanCoordinationErrorKind, ForemanCoordinationPort, ForemanCoordinationResult,
    ForemanRuntimeStatus,
};
use lattice_task_ledger::{
    CommandId, CorrelationId, ForemanAppendMetadata, LedgerError, plan_foreman_snapshot_append,
    preflight_foreman_checkpoint,
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
    fn replay_checkpoint(
        &mut self,
        intent: &ForemanCheckpointIntent,
    ) -> ForemanCoordinationResult<Option<ForemanCheckpointReplay>> {
        let command_id = CommandId::new(intent.checkpoint_id()).map_err(|_| malformed())?;
        let replay = self.ledger.load_foreman_replay().map_err(map_error)?;
        let exact_retry =
            preflight_foreman_checkpoint(replay.ledger().stream(), replay.records(), intent)
                .map_err(|error| map_ledger_error(&error))?;
        if !exact_retry {
            return Ok(None);
        }
        let record = replay
            .records()
            .iter()
            .find(|record| record.command_id() == &command_id)
            .ok_or_else(corrupt)?;
        replay
            .ledger()
            .stream()
            .events()
            .iter()
            .find(|event| event.command_id() == &command_id)
            .ok_or_else(corrupt)?;
        let command = replay
            .ledger()
            .stream()
            .commands()
            .iter()
            .find(|command| command.request().command_id() == &command_id)
            .ok_or_else(corrupt)?;
        let receipt = ForemanAppendReceipt::new(
            record.event_digest().clone(),
            command.result_checkpoint().checkpoint_digest().clone(),
            record.snapshot().generation(),
            true,
        )?;
        let authority_digest = record
            .snapshot()
            .authority()
            .strip_prefix("authority:sha256:")
            .ok_or_else(corrupt)
            .and_then(|digest| {
                lattice_contracts::ContentDigest::from_sha256(digest).map_err(|_| corrupt())
            })?;
        Ok(Some(ForemanCheckpointReplay::new(
            receipt,
            authority_digest,
        )))
    }

    fn append_snapshot(
        &mut self,
        command_id: &str,
        correlation_id: &str,
        occurred_at: &str,
        snapshot: ForemanSnapshot,
        writer: &WriterLeaseAuthorityHead,
    ) -> ForemanCoordinationResult<ForemanAppendReceipt> {
        let replay = self.ledger.load_foreman_replay().map_err(map_error)?;
        let metadata = ForemanAppendMetadata::new(
            CommandId::new(command_id).map_err(|_| malformed())?,
            CorrelationId::new(correlation_id).map_err(|_| malformed())?,
            occurred_at,
        )
        .map_err(|_| malformed())?;
        let plan = plan_foreman_snapshot_append(
            replay.ledger().stream(),
            replay.records(),
            metadata,
            snapshot.clone(),
        )
        .map_err(|error| map_ledger_error(&error))?;
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

    fn load_runtime_status(&mut self) -> ForemanCoordinationResult<ForemanRuntimeStatus> {
        let replay = self.ledger.load_foreman_replay().map_err(map_error)?;
        let projection = reconstruct(
            replay
                .records()
                .iter()
                .map(|record| record.snapshot().clone()),
        )
        .map_err(|_| corrupt())?;
        Ok(ForemanRuntimeStatus::new(
            replay.ledger().stream().head().head_digest().clone(),
            replay
                .ledger()
                .retained_checkpoint()
                .checkpoint_digest()
                .clone(),
            projection.latest_generation(),
            projection.active().len(),
            projection.blocked().len(),
            projection.completed().len(),
            projection.runtime_next_action(),
        ))
    }
}

const fn malformed() -> ForemanCoordinationError {
    ForemanCoordinationError::new(
        ForemanCoordinationErrorKind::Malformed,
        "FOREMAN_COORDINATION_MALFORMED",
    )
}

const fn corrupt() -> ForemanCoordinationError {
    ForemanCoordinationError::new(
        ForemanCoordinationErrorKind::Corrupt,
        "FOREMAN_REPLAY_CORRUPT",
    )
}

fn map_error(error: crate::PostgresTaskLedgerError) -> ForemanCoordinationError {
    if error.kind() == PostgresTaskLedgerErrorKind::CommandSubstitution {
        return ForemanCoordinationError::new(
            ForemanCoordinationErrorKind::Conflict,
            "FOREMAN_CHECKPOINT_ID_REUSE",
        );
    }
    let kind = match error.kind() {
        PostgresTaskLedgerErrorKind::Malformed => ForemanCoordinationErrorKind::Malformed,
        PostgresTaskLedgerErrorKind::CommandSubstitution => ForemanCoordinationErrorKind::Conflict,
        PostgresTaskLedgerErrorKind::AuthorityMismatch
        | PostgresTaskLedgerErrorKind::AdmissionDenied => ForemanCoordinationErrorKind::StaleWriter,
        PostgresTaskLedgerErrorKind::SerializationExhausted => {
            ForemanCoordinationErrorKind::Conflict
        }
        PostgresTaskLedgerErrorKind::CheckpointCorrupt
        | PostgresTaskLedgerErrorKind::RetainedRowCorrupt
        | PostgresTaskLedgerErrorKind::PhysicalStateMismatch
        | PostgresTaskLedgerErrorKind::RevisionOverflow => ForemanCoordinationErrorKind::Corrupt,
        PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema => {
            return ForemanCoordinationError::new(
                ForemanCoordinationErrorKind::Corrupt,
                "FOREMAN_REPLAY_UNSUPPORTED",
            );
        }
        PostgresTaskLedgerErrorKind::CommitOutcomeUnknown => {
            ForemanCoordinationErrorKind::OutcomeUnknown
        }
        PostgresTaskLedgerErrorKind::TransactionFailed
        | PostgresTaskLedgerErrorKind::Unavailable => {
            return ForemanCoordinationError::new(
                ForemanCoordinationErrorKind::Unavailable,
                "FOREMAN_REPLAY_UNAVAILABLE",
            );
        }
    };
    let code = if kind == ForemanCoordinationErrorKind::Corrupt {
        "FOREMAN_REPLAY_CORRUPT"
    } else {
        error.code()
    };
    ForemanCoordinationError::new(kind, code)
}

fn map_ledger_error(error: &LedgerError) -> ForemanCoordinationError {
    match error {
        LedgerError::CommandIdReuse => ForemanCoordinationError::new(
            ForemanCoordinationErrorKind::Conflict,
            "FOREMAN_CHECKPOINT_ID_REUSE",
        ),
        LedgerError::ForemanGenerationRollback => ForemanCoordinationError::new(
            ForemanCoordinationErrorKind::Conflict,
            "FOREMAN_GENERATION_INVALID",
        ),
        LedgerError::UnknownForemanSnapshotVersion => ForemanCoordinationError::new(
            ForemanCoordinationErrorKind::Corrupt,
            "FOREMAN_REPLAY_UNSUPPORTED",
        ),
        LedgerError::InvalidForemanSnapshot => corrupt(),
        _ => malformed(),
    }
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
        assert_eq!(
            map_error(PostgresTaskLedgerError::new(
                PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema
            ))
            .code(),
            "FOREMAN_REPLAY_UNSUPPORTED"
        );
        assert_eq!(
            map_error(PostgresTaskLedgerError::new(
                PostgresTaskLedgerErrorKind::RetainedRowCorrupt
            ))
            .code(),
            "FOREMAN_REPLAY_CORRUPT"
        );
        assert_eq!(
            map_error(PostgresTaskLedgerError::new(
                PostgresTaskLedgerErrorKind::Unavailable
            ))
            .code(),
            "FOREMAN_REPLAY_UNAVAILABLE"
        );
    }

    #[test]
    fn adapter_satisfies_the_typed_port_without_exposing_a_second_store() {
        fn assert_port<T: ForemanCoordinationPort>() {}
        assert_port::<PostgresForemanCoordination>();
    }

    #[test]
    fn concurrent_same_id_substitution_maps_to_stable_conflict() {
        let error = map_ledger_error(&LedgerError::CommandIdReuse);
        assert_eq!(error.kind(), ForemanCoordinationErrorKind::Conflict);
        assert_eq!(error.code(), "FOREMAN_CHECKPOINT_ID_REUSE");
    }
}
