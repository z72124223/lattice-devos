use lattice_approval_verifier::{
    ApprovalCommand, ApprovalCommandOutcome, ApprovalCommandReceipt, ApprovalNormalClaimExecution,
    ApprovalNormalClaimReceipt, ApprovalNormalClaimRequest, ApprovalRepository,
    ApprovalRepositoryCommand, ApprovalRepositoryError, ApprovalRepositoryErrorKind,
    ApprovalVerifierCheckpoint, ApprovalVerifierError, UntrustedApprovalSnapshot,
    VerifiedApprovalAggregate, apply_normal_claim_plan, apply_plan, plan_command,
    plan_normal_claim, verify_snapshot_against_checkpoint,
};
use lattice_contracts::{ApprovalAuthorityHead, ContentDigest, DaemonEpoch, RuntimeAdmissionMode};
use postgres::error::SqlState;
use postgres::{Client, IsolationLevel, Row, Transaction};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::{ExtensionTarget, sha256_bytes, verify_embedded_extension_manifest};

const MAX_SERIALIZATION_RETRIES: usize = 3;
const LOAD_FOR_UPDATE_SQL: &str = "SELECT * FROM approval_verifier.approval_verifier_load_for_update_v1(\
        $1,$2,$3,$4,$5,$6,$7,$8,$9,$10)";
const COMMIT_PLAN_SQL: &str = "SELECT approval_verifier.approval_verifier_commit_plan_v1(\
        $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,\
        $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30)";
const LOAD_CURRENT_SQL: &str = "SELECT * FROM approval_verifier.approval_verifier_load_current_v1(\
        $1,$2,$3,$4,$5,$6)";
const LOAD_COMMANDS_SQL: &str =
    "SELECT * FROM approval_verifier.approval_verifier_load_commands_v1()";
const LOAD_EFFECTS_SQL: &str =
    "SELECT * FROM approval_verifier.approval_verifier_load_effects_v1()";

/// Live `PostgreSQL` implementation of the domain-owned Approval repository.
pub struct PostgresApprovalVerifier {
    client: Client,
    target: ExtensionTarget,
}

impl PostgresApprovalVerifier {
    /// Constructs a runtime adapter around an already provisioned connection.
    /// The connection string and credentials remain outside repository inputs.
    ///
    /// # Errors
    ///
    /// Rejects an unavailable connection or a database-name substitution.
    pub fn new(
        mut client: Client,
        target: ExtensionTarget,
    ) -> Result<Self, ApprovalRepositoryError> {
        let database_name: String = client
            .query_one("SELECT pg_catalog.current_database()::text", &[])
            .and_then(|row| row.try_get(0))
            .map_err(|_| repository_error(ApprovalRepositoryErrorKind::Unavailable))?;
        if database_name != target.database_name() {
            return Err(authority_repository_error());
        }
        verify_embedded_extension_manifest().map_err(|_| corrupt_repository_error())?;
        Ok(Self { client, target })
    }

    fn execute_once(
        &mut self,
        repository_command: &ApprovalRepositoryCommand,
    ) -> Result<ApprovalCommandReceipt, AdapterFailure> {
        let repository_request_bytes = repository_command
            .canonical_bytes()
            .map_err(domain_failure)?;
        let repository_request_sha256 = sha256_bytes(&repository_request_bytes);
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(database_failure)?;
        enter_runtime_writer(&mut transaction)?;
        let loaded = load_for_update(
            &mut transaction,
            &self.target,
            repository_command.command_id(),
        )?;
        verify_physical_history(&mut transaction, &loaded.aggregate)?;
        if let Some(receipt) = exact_command_retry(
            repository_command,
            &repository_request_bytes,
            &repository_request_sha256,
            &loaded,
        )? {
            transaction.commit().map_err(commit_failure)?;
            return Ok(receipt);
        }
        let expires_at = match repository_command {
            ApprovalRepositoryCommand::Issue(request) => {
                Some(expiry_after(&loaded.observed_at, request.ttl_seconds)?)
            }
            ApprovalRepositoryCommand::Verify(_) | ApprovalRepositoryCommand::Revoke(_) => None,
        };
        let command = repository_command
            .clone()
            .bind_observation(&loaded.observed_at, expires_at.as_deref())
            .map_err(domain_failure)?;
        let plan = plan_command(&loaded.aggregate, &command).map_err(domain_failure)?;
        if plan.is_exact_retry() {
            return Err(corrupt_failure());
        }
        let receipt = plan.receipt().clone();
        let next = apply_plan(&loaded.aggregate, plan).map_err(domain_failure)?;
        persist_plan(
            &mut transaction,
            &loaded,
            &next,
            &receipt,
            &repository_request_bytes,
            &repository_request_sha256,
            None,
        )?;
        transaction.commit().map_err(commit_failure)?;
        Ok(receipt)
    }

    fn claim_once(
        &mut self,
        request: &ApprovalNormalClaimRequest,
    ) -> Result<ApprovalNormalClaimExecution, AdapterFailure> {
        let repository_request_bytes = request.canonical_bytes().map_err(domain_failure)?;
        let repository_request_sha256 = sha256_bytes(&repository_request_bytes);
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(database_failure)?;
        enter_runtime_writer(&mut transaction)?;
        let loaded = load_for_update(&mut transaction, &self.target, request.command_id())?;
        verify_physical_history(&mut transaction, &loaded.aggregate)?;
        if let Some(execution) = exact_claim_retry(
            request,
            &repository_request_bytes,
            &repository_request_sha256,
            &loaded,
        )? {
            transaction.commit().map_err(commit_failure)?;
            return Ok(execution);
        }
        let daemon_instance_id = loaded
            .daemon_instance_id
            .as_deref()
            .ok_or_else(authority_failure)?;
        let daemon_epoch = loaded.daemon_epoch.ok_or_else(authority_failure)?;
        if loaded.admission != RuntimeAdmissionMode::Active {
            return Err(authority_failure());
        }
        let plan = plan_normal_claim(
            &loaded.aggregate,
            request.clone(),
            &loaded.observed_at,
            daemon_instance_id,
            daemon_epoch,
            loaded.admission,
        )
        .map_err(domain_failure)?;
        let execution = plan.execution().clone();
        let receipt = plan.approval_receipt().clone();
        let next = apply_normal_claim_plan(&loaded.aggregate, plan).map_err(domain_failure)?;
        persist_plan(
            &mut transaction,
            &loaded,
            &next,
            &receipt,
            &repository_request_bytes,
            &repository_request_sha256,
            Some(&execution),
        )?;
        transaction.commit().map_err(commit_failure)?;
        Ok(execution)
    }

    fn load_current(
        &mut self,
        approval_id: &str,
    ) -> Result<Option<(VerifiedApprovalAggregate, String)>, AdapterFailure> {
        let manifest = verify_embedded_extension_manifest().map_err(|_| corrupt_failure())?;
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(database_failure)?;
        enter_runtime_reader(&mut transaction)?;
        let rows = transaction
            .query(
                LOAD_CURRENT_SQL,
                &[
                    &approval_id,
                    &self.target.database_name(),
                    &self.target.database_identity_digest().as_str(),
                    &self.target.global_manifest_digest().as_str(),
                    &self.target.memory_manifest_digest().as_str(),
                    &manifest.manifest_sha256().as_str(),
                ],
            )
            .map_err(database_failure)?;
        let result = match rows.as_slice() {
            [] => None,
            [row] => {
                let aggregate = aggregate_from_row(row, 0)?;
                verify_physical_history(&mut transaction, &aggregate)?;
                let observed_at: String = row_value(row, 6)?;
                Some((aggregate, observed_at))
            }
            _ => return Err(corrupt_failure()),
        };
        transaction.commit().map_err(database_failure)?;
        Ok(result)
    }
}

impl ApprovalRepository for PostgresApprovalVerifier {
    fn execute(
        &mut self,
        command: ApprovalRepositoryCommand,
    ) -> Result<ApprovalCommandReceipt, ApprovalRepositoryError> {
        for attempt in 0..=MAX_SERIALIZATION_RETRIES {
            match self.execute_once(&command) {
                Ok(receipt) => return Ok(receipt),
                Err(error) if error.retryable && attempt < MAX_SERIALIZATION_RETRIES => {}
                Err(error) if error.retryable => {
                    return Err(repository_error(
                        ApprovalRepositoryErrorKind::SerializationExhausted,
                    ));
                }
                Err(error) => return Err(error.error),
            }
        }
        Err(repository_error(
            ApprovalRepositoryErrorKind::SerializationExhausted,
        ))
    }

    fn claim_normal(
        &mut self,
        request: ApprovalNormalClaimRequest,
    ) -> Result<ApprovalNormalClaimExecution, ApprovalRepositoryError> {
        for attempt in 0..=MAX_SERIALIZATION_RETRIES {
            match self.claim_once(&request) {
                Ok(execution) => return Ok(execution),
                Err(error) if error.retryable && attempt < MAX_SERIALIZATION_RETRIES => {}
                Err(error) if error.retryable => {
                    return Err(repository_error(
                        ApprovalRepositoryErrorKind::SerializationExhausted,
                    ));
                }
                Err(error) => return Err(error.error),
            }
        }
        Err(repository_error(
            ApprovalRepositoryErrorKind::SerializationExhausted,
        ))
    }

    fn current_authority(
        &mut self,
        approval_id: &str,
    ) -> Result<Option<ApprovalAuthorityHead>, ApprovalRepositoryError> {
        let Some((aggregate, observed_at)) = self
            .load_current(approval_id)
            .map_err(|failure| failure.error)?
        else {
            return Ok(None);
        };
        aggregate
            .current_authority_at(approval_id, &observed_at)
            .map_err(ApprovalRepositoryError::from_domain)
    }
}

struct LoadedAggregate {
    row_version: i64,
    aggregate: VerifiedApprovalAggregate,
    checkpoint: ApprovalVerifierCheckpoint,
    existing_repository_request_bytes: Option<Vec<u8>>,
    existing_repository_request_sha256: Option<Vec<u8>>,
    existing_effect: Option<ExistingEffect>,
    observed_at: String,
    admission: RuntimeAdmissionMode,
    daemon_instance_id: Option<String>,
    daemon_epoch: Option<DaemonEpoch>,
}

struct ExistingEffect {
    kind: String,
    id: String,
    digest: ContentDigest,
    request_bytes: Vec<u8>,
    observed_at: String,
    daemon_instance_id: String,
    daemon_epoch: DaemonEpoch,
    admission: RuntimeAdmissionMode,
    claim_digest: ContentDigest,
    approval_receipt_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

fn load_for_update(
    transaction: &mut Transaction<'_>,
    target: &ExtensionTarget,
    command_id: &str,
) -> Result<LoadedAggregate, AdapterFailure> {
    let manifest = verify_embedded_extension_manifest().map_err(|_| corrupt_failure())?;
    let vacant = VerifiedApprovalAggregate::empty();
    let vacant_bytes = vacant
        .export_untrusted()
        .canonical_bytes()
        .map_err(domain_failure)?;
    let vacant_checkpoint = vacant.checkpoint().map_err(domain_failure)?;
    let row = transaction
        .query_one(
            LOAD_FOR_UPDATE_SQL,
            &[
                &command_id,
                &vacant_bytes,
                &sha256_bytes(&vacant_bytes),
                &digest_bytes(vacant_checkpoint.nonce_bindings_digest())?,
                &digest_bytes(vacant_checkpoint.snapshot_digest())?,
                &target.database_name(),
                &target.database_identity_digest().as_str(),
                &target.global_manifest_digest().as_str(),
                &target.memory_manifest_digest().as_str(),
                &manifest.manifest_sha256().as_str(),
            ],
        )
        .map_err(database_failure)?;
    LoadedAggregate::from_locked_row(&row)
}

impl LoadedAggregate {
    fn from_locked_row(row: &Row) -> Result<Self, AdapterFailure> {
        let row_version: i64 = row_value(row, 0)?;
        let aggregate = aggregate_from_row(row, 1)?;
        let checkpoint = checkpoint_from_row(row, 3)?;
        if row_version < 0
            || u64::try_from(row_version).ok() != Some(checkpoint.command_high_water())
        {
            return Err(corrupt_failure());
        }
        let existing_repository_request_bytes: Option<Vec<u8>> = row_value(row, 7)?;
        let existing_repository_request_sha256: Option<Vec<u8>> = row_value(row, 8)?;
        match (
            existing_repository_request_bytes.as_ref(),
            existing_repository_request_sha256.as_ref(),
        ) {
            (None, None) => {}
            (Some(bytes), Some(digest)) if sha256_bytes(bytes) == *digest => {}
            _ => return Err(corrupt_failure()),
        }
        let effect_parts = (
            row_value::<Option<String>>(row, 9)?,
            row_value::<Option<String>>(row, 10)?,
            row_optional_digest(row, 11)?,
            row_value::<Option<Vec<u8>>>(row, 12)?,
            row_value::<Option<String>>(row, 13)?,
            row_value::<Option<String>>(row, 14)?,
            row_value::<Option<i64>>(row, 15)?,
            row_value::<Option<String>>(row, 16)?,
            row_optional_digest(row, 17)?,
            row_optional_digest(row, 18)?,
            row_optional_digest(row, 19)?,
        );
        let existing_effect = match effect_parts {
            (
                Some(kind),
                Some(id),
                Some(digest),
                Some(request_bytes),
                Some(observed_at),
                Some(daemon_instance_id),
                Some(daemon_epoch),
                Some(admission),
                Some(claim_digest),
                Some(approval_receipt_digest),
                Some(receipt_digest),
            ) => Some(ExistingEffect {
                kind,
                id,
                digest,
                request_bytes,
                observed_at,
                daemon_instance_id,
                daemon_epoch: u64::try_from(daemon_epoch)
                    .ok()
                    .and_then(|value| DaemonEpoch::new(value).ok())
                    .ok_or_else(corrupt_failure)?,
                admission: parse_admission(&admission)?,
                claim_digest,
                approval_receipt_digest,
                receipt_digest,
            }),
            (None, None, None, None, None, None, None, None, None, None, None) => None,
            _ => return Err(corrupt_failure()),
        };
        if existing_effect.is_some() && existing_repository_request_bytes.is_none() {
            return Err(corrupt_failure());
        }
        let observed_at: String = row_value(row, 20)?;
        let admission = parse_admission(&row_value::<String>(row, 21)?)?;
        let daemon_instance_id: Option<String> = row_value(row, 22)?;
        let daemon_epoch = row_value::<Option<i64>>(row, 23)?
            .map(|value| {
                u64::try_from(value)
                    .ok()
                    .and_then(|value| DaemonEpoch::new(value).ok())
                    .ok_or_else(corrupt_failure)
            })
            .transpose()?;
        Ok(Self {
            row_version,
            aggregate,
            checkpoint,
            existing_repository_request_bytes,
            existing_repository_request_sha256,
            existing_effect,
            observed_at,
            admission,
            daemon_instance_id,
            daemon_epoch,
        })
    }
}

fn aggregate_from_row(
    row: &Row,
    snapshot_offset: usize,
) -> Result<VerifiedApprovalAggregate, AdapterFailure> {
    let snapshot_bytes: Vec<u8> = row_value(row, snapshot_offset)?;
    let snapshot_bytes_sha256: Vec<u8> = row_value(row, snapshot_offset + 1)?;
    if sha256_bytes(&snapshot_bytes) != snapshot_bytes_sha256 {
        return Err(corrupt_failure());
    }
    let checkpoint = checkpoint_from_row(row, snapshot_offset + 2)?;
    let snapshot = UntrustedApprovalSnapshot::from_canonical_bytes(&snapshot_bytes)
        .map_err(|_| corrupt_failure())?;
    verify_snapshot_against_checkpoint(&snapshot, &checkpoint).map_err(|_| corrupt_failure())
}

fn checkpoint_from_row(
    row: &Row,
    offset: usize,
) -> Result<ApprovalVerifierCheckpoint, AdapterFailure> {
    let high_water = row_value::<i64>(row, offset)?;
    ApprovalVerifierCheckpoint::new(
        u64::try_from(high_water).map_err(|_| corrupt_failure())?,
        row_optional_digest(row, offset + 1)?,
        row_digest(row, offset + 2)?,
        row_digest(row, offset + 3)?,
    )
    .map_err(|_| corrupt_failure())
}

fn exact_command_retry(
    request: &ApprovalRepositoryCommand,
    request_bytes: &[u8],
    request_sha256: &[u8],
    loaded: &LoadedAggregate,
) -> Result<Option<ApprovalCommandReceipt>, AdapterFailure> {
    let existing = loaded
        .aggregate
        .command_receipts()
        .iter()
        .find(|receipt| receipt.request.command_id() == request.command_id());
    match (
        existing,
        loaded.existing_repository_request_bytes.as_deref(),
        loaded.existing_repository_request_sha256.as_deref(),
    ) {
        (None, None, None) => Ok(None),
        (Some(receipt), Some(stored_bytes), Some(stored_sha256)) => {
            if stored_bytes != request_bytes || stored_sha256 != request_sha256 {
                return Err(domain_failure(ApprovalVerifierError::CommandIdReuse));
            }
            if loaded.existing_effect.is_some() {
                return Err(corrupt_failure());
            }
            let plan = plan_command(&loaded.aggregate, &receipt.request).map_err(domain_failure)?;
            if !plan.is_exact_retry() || plan.receipt() != receipt {
                return Err(corrupt_failure());
            }
            Ok(Some(receipt.clone()))
        }
        _ => Err(corrupt_failure()),
    }
}

fn exact_claim_retry(
    request: &ApprovalNormalClaimRequest,
    request_bytes: &[u8],
    request_sha256: &[u8],
    loaded: &LoadedAggregate,
) -> Result<Option<ApprovalNormalClaimExecution>, AdapterFailure> {
    let existing = loaded
        .aggregate
        .command_receipts()
        .iter()
        .find(|receipt| receipt.request.command_id() == request.command_id());
    match (
        existing,
        loaded.existing_repository_request_bytes.as_deref(),
        loaded.existing_repository_request_sha256.as_deref(),
    ) {
        (None, None, None) => Ok(None),
        (Some(receipt), Some(stored_bytes), Some(stored_sha256)) => {
            if stored_bytes != request_bytes || stored_sha256 != request_sha256 {
                return Err(domain_failure(ApprovalVerifierError::CommandIdReuse));
            }
            let plan = plan_command(&loaded.aggregate, &receipt.request).map_err(domain_failure)?;
            if !plan.is_exact_retry() || plan.receipt() != receipt {
                return Err(corrupt_failure());
            }
            match receipt.outcome {
                ApprovalCommandOutcome::Denied(_) => {
                    if loaded.existing_effect.is_some() {
                        return Err(corrupt_failure());
                    }
                    Ok(Some(ApprovalNormalClaimExecution::Denied(receipt.clone())))
                }
                ApprovalCommandOutcome::Applied => {
                    let effect = loaded
                        .existing_effect
                        .as_ref()
                        .ok_or_else(corrupt_failure)?;
                    if effect.request_bytes != request_bytes
                        || effect.kind != request.effect().effect_kind()
                        || effect.id != request.effect().effect_id()
                        || &effect.digest != request.effect().effect_digest()
                        || effect.approval_receipt_digest != receipt.receipt_digest
                        || effect.admission != RuntimeAdmissionMode::Active
                    {
                        return Err(corrupt_failure());
                    }
                    let rebuilt = ApprovalNormalClaimReceipt::from_verified_parts(
                        request.clone(),
                        receipt.clone(),
                        effect.observed_at.clone(),
                        effect.daemon_instance_id.clone(),
                        effect.daemon_epoch,
                        effect.admission,
                        effect.claim_digest.clone(),
                    )
                    .map_err(|_| corrupt_failure())?;
                    if rebuilt.receipt_digest() != &effect.receipt_digest {
                        return Err(corrupt_failure());
                    }
                    Ok(Some(ApprovalNormalClaimExecution::Claimed(rebuilt)))
                }
            }
        }
        _ => Err(corrupt_failure()),
    }
}

#[allow(clippy::too_many_lines)]
fn verify_physical_history(
    transaction: &mut Transaction<'_>,
    aggregate: &VerifiedApprovalAggregate,
) -> Result<(), AdapterFailure> {
    let command_rows = transaction
        .query(LOAD_COMMANDS_SQL, &[])
        .map_err(database_failure)?;
    let receipts = aggregate.command_receipts();
    if command_rows.len() != receipts.len() {
        return Err(corrupt_failure());
    }
    for (row, receipt) in command_rows.iter().zip(receipts) {
        let ordinal = u64::try_from(row_value::<i64>(row, 0)?).map_err(|_| corrupt_failure())?;
        let command_id: String = row_value(row, 1)?;
        let approval_id: String = row_value(row, 2)?;
        let repository_request_bytes: Vec<u8> = row_value(row, 3)?;
        let repository_request_sha256: Vec<u8> = row_value(row, 4)?;
        let command_bytes: Vec<u8> = row_value(row, 5)?;
        let command_bytes_sha256: Vec<u8> = row_value(row, 6)?;
        let receipt_bytes: Vec<u8> = row_value(row, 7)?;
        let receipt_bytes_sha256: Vec<u8> = row_value(row, 8)?;
        let receipt_digest = row_digest(row, 9)?;
        let outcome: String = row_value(row, 10)?;
        let denial_reason: Option<String> = row_value(row, 11)?;
        let (expected_outcome, expected_denial) = match receipt.outcome {
            ApprovalCommandOutcome::Applied => ("APPLIED", None),
            ApprovalCommandOutcome::Denied(denial) => ("DENIED", Some(denial.code())),
        };
        if ordinal != receipt.ordinal
            || command_id != receipt.request.command_id()
            || approval_id != receipt.request.approval_id()
            || repository_request_sha256 != sha256_bytes(&repository_request_bytes)
            || command_bytes_sha256 != sha256_bytes(&command_bytes)
            || command_bytes != receipt.request.canonical_bytes().map_err(domain_failure)?
            || receipt_bytes_sha256 != sha256_bytes(&receipt_bytes)
            || receipt_bytes != receipt.canonical_bytes().map_err(domain_failure)?
            || receipt_digest != receipt.receipt_digest
            || outcome != expected_outcome
            || denial_reason.as_deref() != expected_denial
        {
            return Err(corrupt_failure());
        }
    }

    let effect_rows = transaction
        .query(LOAD_EFFECTS_SQL, &[])
        .map_err(database_failure)?;
    let expected_effect_count = receipts
        .iter()
        .filter(|receipt| {
            matches!(receipt.request, ApprovalCommand::ConsumeNormal(_))
                && receipt.outcome == ApprovalCommandOutcome::Applied
        })
        .count();
    if effect_rows.len() != expected_effect_count {
        return Err(corrupt_failure());
    }
    for row in effect_rows {
        let command_id: String = row_value(&row, 0)?;
        let approval_id: String = row_value(&row, 1)?;
        let effect_kind: String = row_value(&row, 2)?;
        let effect_id: String = row_value(&row, 3)?;
        let effect_digest = row_digest(&row, 4)?;
        let request_bytes: Vec<u8> = row_value(&row, 5)?;
        let request_sha256: Vec<u8> = row_value(&row, 6)?;
        let observed_at: String = row_value(&row, 7)?;
        let daemon_instance_id: String = row_value(&row, 8)?;
        let daemon_epoch = u64::try_from(row_value::<i64>(&row, 9)?)
            .ok()
            .and_then(|value| DaemonEpoch::new(value).ok())
            .ok_or_else(corrupt_failure)?;
        let admission = parse_admission(&row_value::<String>(&row, 10)?)?;
        let claim_digest = row_digest(&row, 11)?;
        let approval_receipt_digest = row_digest(&row, 12)?;
        let effect_receipt_digest = row_digest(&row, 13)?;
        if request_sha256 != sha256_bytes(&request_bytes) {
            return Err(corrupt_failure());
        }
        let request = ApprovalNormalClaimRequest::from_canonical_bytes(&request_bytes)
            .map_err(|_| corrupt_failure())?;
        let receipt = receipts
            .iter()
            .find(|receipt| receipt.request.command_id() == command_id)
            .ok_or_else(corrupt_failure)?;
        let ApprovalCommand::ConsumeNormal(command) = &receipt.request else {
            return Err(corrupt_failure());
        };
        if receipt.outcome != ApprovalCommandOutcome::Applied
            || approval_id != request.approval_id()
            || request.command_id() != command_id
            || request.expected_head() != &command.expected_head
            || request.effect().effect_kind() != effect_kind
            || request.effect().effect_id() != effect_id
            || request.effect().effect_digest() != &effect_digest
            || command.approval_id != approval_id
            || command.observed_at != observed_at
            || command.claim_digest != claim_digest
            || receipt.receipt_digest != approval_receipt_digest
            || admission != RuntimeAdmissionMode::Active
        {
            return Err(corrupt_failure());
        }
        let rebuilt = ApprovalNormalClaimReceipt::from_verified_parts(
            request,
            receipt.clone(),
            observed_at,
            daemon_instance_id,
            daemon_epoch,
            admission,
            claim_digest,
        )
        .map_err(|_| corrupt_failure())?;
        if rebuilt.receipt_digest() != &effect_receipt_digest {
            return Err(corrupt_failure());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn persist_plan(
    transaction: &mut Transaction<'_>,
    loaded: &LoadedAggregate,
    next: &VerifiedApprovalAggregate,
    receipt: &ApprovalCommandReceipt,
    repository_request_bytes: &[u8],
    repository_request_sha256: &[u8],
    claim_execution: Option<&ApprovalNormalClaimExecution>,
) -> Result<(), AdapterFailure> {
    let next_snapshot_bytes = next
        .export_untrusted()
        .canonical_bytes()
        .map_err(domain_failure)?;
    let next_checkpoint = next.checkpoint().map_err(domain_failure)?;
    let next_tail = next_checkpoint
        .command_tail_digest()
        .map(digest_bytes)
        .transpose()?;
    let command_bytes = receipt.request.canonical_bytes().map_err(domain_failure)?;
    let receipt_bytes = receipt.canonical_bytes().map_err(domain_failure)?;
    let receipt_digest = digest_bytes(&receipt.receipt_digest)?;
    let (outcome, denial_reason) = match receipt.outcome {
        ApprovalCommandOutcome::Applied => ("APPLIED", None),
        ApprovalCommandOutcome::Denied(denial) => ("DENIED", Some(denial.code())),
    };
    let mut effect_kind: Option<String> = None;
    let mut effect_id: Option<String> = None;
    let mut effect_digest: Option<Vec<u8>> = None;
    let mut effect_request_bytes: Option<Vec<u8>> = None;
    let mut effect_request_sha256: Option<Vec<u8>> = None;
    let mut claim_digest: Option<Vec<u8>> = None;
    let mut effect_receipt_digest: Option<Vec<u8>> = None;
    match claim_execution {
        None | Some(ApprovalNormalClaimExecution::Denied(_)) => {}
        Some(ApprovalNormalClaimExecution::Claimed(claimed)) => {
            effect_kind = Some(claimed.request().effect().effect_kind().to_owned());
            effect_id = Some(claimed.request().effect().effect_id().to_owned());
            effect_digest = Some(digest_bytes(claimed.request().effect().effect_digest())?);
            let bytes = claimed
                .request()
                .canonical_bytes()
                .map_err(domain_failure)?;
            effect_request_sha256 = Some(sha256_bytes(&bytes));
            effect_request_bytes = Some(bytes);
            claim_digest = Some(digest_bytes(claimed.claim_digest())?);
            effect_receipt_digest = Some(digest_bytes(claimed.receipt_digest())?);
        }
    }
    let daemon_epoch = loaded
        .daemon_epoch
        .map(|value| to_i64(value.get()))
        .transpose()?;
    let committed: bool = transaction
        .query_one(
            COMMIT_PLAN_SQL,
            &[
                &loaded.row_version,
                &digest_bytes(loaded.checkpoint.snapshot_digest())?,
                &loaded.observed_at,
                &loaded.admission.as_str(),
                &loaded.daemon_instance_id,
                &daemon_epoch,
                &next_snapshot_bytes,
                &sha256_bytes(&next_snapshot_bytes),
                &to_i64(next_checkpoint.command_high_water())?,
                &next_tail,
                &digest_bytes(next_checkpoint.nonce_bindings_digest())?,
                &digest_bytes(next_checkpoint.snapshot_digest())?,
                &receipt.request.command_id(),
                &receipt.request.approval_id(),
                &repository_request_bytes,
                &repository_request_sha256,
                &command_bytes,
                &sha256_bytes(&command_bytes),
                &receipt_bytes,
                &sha256_bytes(&receipt_bytes),
                &receipt_digest,
                &outcome,
                &denial_reason,
                &effect_kind,
                &effect_id,
                &effect_digest,
                &effect_request_bytes,
                &effect_request_sha256,
                &claim_digest,
                &effect_receipt_digest,
            ],
        )
        .and_then(|row| row.try_get(0))
        .map_err(database_failure)?;
    if !committed {
        return Err(authority_failure());
    }
    Ok(())
}

fn expiry_after(observed_at: &str, seconds: u32) -> Result<String, AdapterFailure> {
    let observed = OffsetDateTime::parse(observed_at, &Rfc3339).map_err(|_| corrupt_failure())?;
    observed
        .checked_add(Duration::seconds(i64::from(seconds)))
        .ok_or_else(authority_failure)?
        .format(&Rfc3339)
        .map_err(|_| corrupt_failure())
}

fn parse_admission(value: &str) -> Result<RuntimeAdmissionMode, AdapterFailure> {
    RuntimeAdmissionMode::ALL
        .into_iter()
        .find(|mode| mode.as_str() == value)
        .ok_or_else(corrupt_failure)
}

fn row_value<T>(row: &Row, index: usize) -> Result<T, AdapterFailure>
where
    T: postgres::types::FromSqlOwned,
{
    row.try_get(index).map_err(|_| corrupt_failure())
}

fn row_digest(row: &Row, index: usize) -> Result<ContentDigest, AdapterFailure> {
    let bytes: Vec<u8> = row_value(row, index)?;
    bytes_digest(&bytes)
}

fn row_optional_digest(row: &Row, index: usize) -> Result<Option<ContentDigest>, AdapterFailure> {
    row_value::<Option<Vec<u8>>>(row, index)?
        .map(|bytes| bytes_digest(&bytes))
        .transpose()
}

fn digest_bytes(digest: &ContentDigest) -> Result<Vec<u8>, AdapterFailure> {
    let bytes = digest.as_str().as_bytes();
    let mut output = Vec::with_capacity(32);
    for pair in bytes.chunks_exact(2) {
        let high = hex_value(pair[0]).ok_or_else(corrupt_failure)?;
        let low = hex_value(pair[1]).ok_or_else(corrupt_failure)?;
        output.push((high << 4) | low);
    }
    if output.len() != 32 {
        return Err(corrupt_failure());
    }
    Ok(output)
}

fn bytes_digest(bytes: &[u8]) -> Result<ContentDigest, AdapterFailure> {
    if bytes.len() != 32 {
        return Err(corrupt_failure());
    }
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    ContentDigest::from_sha256(output).map_err(|_| corrupt_failure())
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn to_i64(value: u64) -> Result<i64, AdapterFailure> {
    i64::try_from(value).map_err(|_| corrupt_failure())
}

fn enter_runtime_writer(transaction: &mut Transaction<'_>) -> Result<(), AdapterFailure> {
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_runtime; \
             SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; \
             SET LOCAL synchronous_commit = on; \
             SET LOCAL lock_timeout = '5s'; \
             SET LOCAL statement_timeout = '30s'; \
             SET LOCAL idle_in_transaction_session_timeout = '30s';",
        )
        .map_err(database_failure)
}

fn enter_runtime_reader(transaction: &mut Transaction<'_>) -> Result<(), AdapterFailure> {
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_runtime; \
             SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; \
             SET LOCAL lock_timeout = '5s'; \
             SET LOCAL statement_timeout = '30s'; \
             SET LOCAL idle_in_transaction_session_timeout = '30s';",
        )
        .map_err(database_failure)
}

#[derive(Clone, Copy, Debug)]
struct AdapterFailure {
    error: ApprovalRepositoryError,
    retryable: bool,
}

const fn repository_error(kind: ApprovalRepositoryErrorKind) -> ApprovalRepositoryError {
    ApprovalRepositoryError::new(kind)
}

const fn corrupt_repository_error() -> ApprovalRepositoryError {
    repository_error(ApprovalRepositoryErrorKind::Corrupt)
}

const fn authority_repository_error() -> ApprovalRepositoryError {
    repository_error(ApprovalRepositoryErrorKind::AuthorityMismatch)
}

const fn corrupt_failure() -> AdapterFailure {
    AdapterFailure {
        error: corrupt_repository_error(),
        retryable: false,
    }
}

const fn authority_failure() -> AdapterFailure {
    AdapterFailure {
        error: authority_repository_error(),
        retryable: false,
    }
}

fn domain_failure(error: ApprovalVerifierError) -> AdapterFailure {
    AdapterFailure {
        error: ApprovalRepositoryError::from_domain(error),
        retryable: false,
    }
}

#[allow(clippy::needless_pass_by_value)]
fn database_failure(error: postgres::Error) -> AdapterFailure {
    let retryable = error.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE);
    let kind = if retryable {
        ApprovalRepositoryErrorKind::Unavailable
    } else if matches!(
        error.code().map(SqlState::code),
        Some("LAV01" | "LAV02" | "LAV04")
    ) {
        ApprovalRepositoryErrorKind::Corrupt
    } else if matches!(error.code().map(SqlState::code), Some("LAV03" | "LAV05")) {
        ApprovalRepositoryErrorKind::AuthorityMismatch
    } else {
        ApprovalRepositoryErrorKind::Unavailable
    };
    AdapterFailure {
        error: repository_error(kind),
        retryable,
    }
}

fn commit_failure(error: postgres::Error) -> AdapterFailure {
    if error.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE) {
        database_failure(error)
    } else {
        AdapterFailure {
            error: repository_error(ApprovalRepositoryErrorKind::CommitOutcomeUnknown),
            retryable: false,
        }
    }
}
