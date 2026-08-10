//! Live `PostgreSQL` physical `ControlStore` adapter.

use lattice_contracts::{
    ContentDigest, RuntimeKind, STORE_CONTRACT_VERSION, StorePersistenceEvidence,
    StorePhysicalHead, StoreReceiptDisposition, StoreRevision, StoreScope, StoreTransactionReceipt,
    StoreTransactionRequest,
};
use lattice_ports::{ControlStore, ControlStoreError, ControlStoreErrorKind, ControlStoreResult};
use postgres::types::FromSqlOwned;
use postgres::{Client, Error as PostgresError, IsolationLevel, Row, Transaction};

use crate::migrations::{
    POSTGRES_SCHEMA_VERSION, STORE_V2_MANIFEST_SHA256, STORE_V2_SCHEMA_VERSION,
    verify_embedded_manifest,
};
use crate::postgres_setup::verify_runtime_store_schema;
use crate::{
    MigrationTarget, PostgresStoreSetupError, PostgresStoreSetupErrorKind, build_live_receipt,
    genesis_head, physical_head, request_digest, validate_physical_head,
};

const MAX_LIVE_SERIALIZATION_RETRIES: u8 = 3;
const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";

const PREPARE_SQL: &str = "\
    SELECT prepare_status, database_uuid::text, database_identity_digest, \
           schema_version, manifest_sha256, head_found, before_revision, \
           before_state_digest, before_head_digest, after_revision, \
           after_state_digest, after_head_digest, terminal_disposition, \
           terminal_transaction_digest, terminal_receipt_digest, \
           global_schema_version, global_manifest_sha256 \
      FROM control.store_prepare_v4(\
           $1::smallint, $2::text, $3::smallint, $4::text, $5::text, $6::text, \
           $7::text, $8::bytea, $9::bytea, $10::text, $11::text, $12::bigint, \
           $13::text, $14::bigint, $15::bytea, $16::bytea, $17::text, \
           $18::bigint, $19::bytea, $20::bytea, $21::bytea, $22::bytea, \
           $23::bytea, $24::bytea, $25::bytea, $26::bytea, $27::bytea, $28::bytea)";

const FINALIZE_SQL: &str = "\
    SELECT control.store_finalize_v4(\
           $1::smallint, $2::text, $3::smallint, $4::text, $5::text, $6::text, \
           $7::text, $8::bytea, $9::bytea, $10::text, $11::text, $12::bigint, \
           $13::text, $14::bigint, $15::bytea, $16::bytea, $17::text, \
           $18::bigint, $19::bytea, $20::bytea, $21::bytea, $22::bytea, \
           $23::bytea, $24::bytea, $25::bytea, $26::bytea, $27::bytea, \
           $28::bytea, $29::text::uuid, $30::bytea, $31::smallint, $32::text, \
           $33::bigint, $34::bytea, $35::bytea, $36::bigint, $37::bytea, \
           $38::bytea, $39::text, $40::bytea, $41::bytea)";

const CURRENT_HEAD_SQL: &str = "\
    SELECT database_uuid::text, schema_version, manifest_sha256, head_found, \
           physical_revision, state_digest, head_digest, global_schema_version, \
           global_manifest_sha256 \
      FROM control.store_current_head_v4(\
           $1::smallint, $2::text, $3::text, $4::text, $5::text, $6::bytea)";

const WRITE_TRANSACTION_SETTINGS: &str = "\
    SET LOCAL search_path = pg_catalog; \
    SET LOCAL row_security = on; \
    SET LOCAL synchronous_commit = on; \
    SET LOCAL lock_timeout = '5s'; \
    SET LOCAL statement_timeout = '30s'";

const READ_TRANSACTION_SETTINGS: &str = "\
    SET LOCAL search_path = pg_catalog; \
    SET LOCAL row_security = on; \
    SET LOCAL lock_timeout = '5s'; \
    SET LOCAL statement_timeout = '30s'";

/// Synchronous live `PostgreSQL` implementation of the physical Store port.
///
/// The adapter owns one already-authenticated runtime connection. It exposes
/// neither that connection nor an arbitrary SQL escape hatch. After an unknown
/// commit outcome the instance is poisoned and must be replaced for exact
/// request reconciliation.
pub struct PostgresControlStore {
    client: Client,
    database_uuid: String,
    schema_version: i16,
    manifest_sha256: String,
    global_schema_version: i16,
    global_manifest_sha256: String,
    persistence: StorePersistenceEvidence,
    commit_outcome_unknown: bool,
}

impl PostgresControlStore {
    /// Constructs a live Store only after exact runtime schema verification.
    ///
    /// # Errors
    ///
    /// Returns a bounded Store error when the connection, target identity,
    /// manifest, catalog, roles, grants, or runtime-admission shape is not
    /// exactly the verified global schema-v4 contract and immutable Store-v2
    /// receipt profile.
    pub fn new(mut client: Client, target: &MigrationTarget) -> ControlStoreResult<Self> {
        let evidence = verify_runtime_store_schema(&mut client, target).map_err(map_setup_error)?;
        let global_manifest = verify_embedded_manifest().map_err(map_setup_error)?;
        if evidence.global_schema_version() != POSTGRES_SCHEMA_VERSION
            || evidence.global_schema_version() != global_manifest.schema_version()
            || evidence.global_manifest_sha256() != global_manifest.manifest_sha256()
            || evidence.schema_version() != STORE_V2_SCHEMA_VERSION
            || evidence.manifest_sha256().as_str() != STORE_V2_MANIFEST_SHA256
        {
            return Err(corrupt("STORE_PERSISTENCE_IDENTITY_MISMATCH"));
        }
        let database_identity_digest =
            ContentDigest::from_sha256(target.expected_database_identity_sha256().as_str())
                .map_err(|_| corrupt("STORE_DATABASE_IDENTITY_DIGEST_INVALID"))?;
        let manifest_digest = ContentDigest::from_sha256(evidence.manifest_sha256().as_str())
            .map_err(|_| corrupt("STORE_MANIFEST_DIGEST_INVALID"))?;
        let persistence = StorePersistenceEvidence::new(
            database_identity_digest,
            evidence.schema_version(),
            manifest_digest,
        )
        .map_err(|_| corrupt("STORE_PERSISTENCE_EVIDENCE_INVALID"))?;
        let schema_version = i16::try_from(evidence.schema_version())
            .map_err(|_| corrupt("STORE_SCHEMA_VERSION_INVALID"))?;
        let global_schema_version = i16::try_from(evidence.global_schema_version())
            .map_err(|_| corrupt("STORE_GLOBAL_SCHEMA_VERSION_INVALID"))?;

        Ok(Self {
            client,
            database_uuid: evidence.database_uuid().to_owned(),
            schema_version,
            manifest_sha256: evidence.manifest_sha256().as_str().to_owned(),
            global_schema_version,
            global_manifest_sha256: evidence.global_manifest_sha256().as_str().to_owned(),
            persistence,
            commit_outcome_unknown: false,
        })
    }

    fn attempt_context(
        &self,
        request: StoreTransactionRequest,
    ) -> ControlStoreResult<AttemptContext> {
        if request.version() != STORE_CONTRACT_VERSION {
            return Err(store_error(
                ControlStoreErrorKind::UnsupportedVersion,
                "STORE_LIVE_VERSION_UNSUPPORTED",
            ));
        }
        if request.expected_authority().runtime() != RuntimeKind::Live
            || request.expected_head().runtime() != RuntimeKind::Live
        {
            return Err(store_error(
                ControlStoreErrorKind::Malformed,
                "STORE_LIVE_RUNTIME_REQUIRED",
            ));
        }
        validate_physical_head(request.expected_head())?;
        let canonical_request_digest = request_digest(&request)?;
        let genesis = genesis_head(RuntimeKind::Live, request.scope().clone())?;
        let sql = SqlRequestValues::new(&request, &canonical_request_digest, &genesis)?;

        Ok(AttemptContext {
            request,
            request_digest: canonical_request_digest,
            genesis,
            database_uuid: self.database_uuid.clone(),
            schema_version: self.schema_version,
            manifest_sha256: self.manifest_sha256.clone(),
            global_schema_version: self.global_schema_version,
            global_manifest_sha256: self.global_manifest_sha256.clone(),
            persistence: self.persistence.clone(),
            sql,
        })
    }

    fn ensure_reconcilable(&self) -> ControlStoreResult<()> {
        if self.commit_outcome_unknown {
            Err(store_error(
                ControlStoreErrorKind::CommitOutcomeUnknown,
                "STORE_LIVE_RECONCILIATION_REQUIRED",
            ))
        } else {
            Ok(())
        }
    }
}

impl ControlStore for PostgresControlStore {
    fn transact(
        &mut self,
        request: StoreTransactionRequest,
    ) -> ControlStoreResult<StoreTransactionReceipt> {
        self.ensure_reconcilable()?;
        let context = self.attempt_context(request)?;

        for retry_count in 0..=MAX_LIVE_SERIALIZATION_RETRIES {
            match run_attempt(&mut self.client, &context) {
                Ok(receipt) => return Ok(receipt),
                Err(AttemptFailure::Retryable) if retry_count < MAX_LIVE_SERIALIZATION_RETRIES => {}
                Err(AttemptFailure::Retryable) => {
                    return Err(store_error(
                        ControlStoreErrorKind::SerializationExhausted,
                        "STORE_SERIALIZATION_RETRIES_EXHAUSTED",
                    ));
                }
                Err(AttemptFailure::CommitOutcomeUnknown) => {
                    self.commit_outcome_unknown = true;
                    return Err(store_error(
                        ControlStoreErrorKind::CommitOutcomeUnknown,
                        "STORE_COMMIT_OUTCOME_UNKNOWN",
                    ));
                }
                Err(AttemptFailure::Terminal(error)) => return Err(error),
            }
        }
        Err(corrupt("STORE_RETRY_LOOP_INVALID"))
    }

    fn current_head(&mut self, scope: &StoreScope) -> ControlStoreResult<StorePhysicalHead> {
        self.ensure_reconcilable()?;
        let aggregate_key_digest = digest_bytes(scope.aggregate_key_digest())?;
        let database_uuid = self.database_uuid.clone();
        let schema_version = self.schema_version;
        let manifest_sha256 = self.manifest_sha256.clone();
        let global_schema_version = self.global_schema_version;
        let global_manifest_sha256 = self.global_manifest_sha256.clone();
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|error| map_current_error(&error))?;
        if let Err(error) = transaction.batch_execute(READ_TRANSACTION_SETTINGS) {
            return rollback_current(transaction, map_current_error(&error));
        }
        let row = match transaction.query_one(
            CURRENT_HEAD_SQL,
            &[
                &global_schema_version,
                &global_manifest_sha256,
                &scope.project_id().as_str(),
                &scope.project_snapshot_id().as_str(),
                &scope.owner().as_str(),
                &aggregate_key_digest,
            ],
        ) {
            Ok(row) => row,
            Err(error) => return rollback_current(transaction, map_current_error(&error)),
        };
        let head = match parse_current_head(
            &row,
            scope,
            &database_uuid,
            schema_version,
            &manifest_sha256,
            global_schema_version,
            &global_manifest_sha256,
        ) {
            Ok(head) => head,
            Err(error) => return rollback_current(transaction, error),
        };
        transaction
            .commit()
            .map_err(|error| map_current_error(&error))?;
        Ok(head)
    }
}

struct AttemptContext {
    request: StoreTransactionRequest,
    request_digest: ContentDigest,
    genesis: StorePhysicalHead,
    database_uuid: String,
    schema_version: i16,
    manifest_sha256: String,
    global_schema_version: i16,
    global_manifest_sha256: String,
    persistence: StorePersistenceEvidence,
    sql: SqlRequestValues,
}

struct SqlRequestValues {
    version: i16,
    transaction_id: String,
    project_id: String,
    project_snapshot_id: String,
    repository_owner: String,
    aggregate_key_digest: Vec<u8>,
    request_digest: Vec<u8>,
    authority_runtime: String,
    daemon_instance_id: String,
    daemon_epoch: i64,
    admission_mode: String,
    authority_revision: i64,
    authority_observation_digest: Vec<u8>,
    authority_head_digest: Vec<u8>,
    expected_head_runtime: String,
    expected_revision: i64,
    expected_state_digest: Vec<u8>,
    expected_head_digest: Vec<u8>,
    domain_command_digest: Vec<u8>,
    record_set_digest: Vec<u8>,
    next_state_digest: Vec<u8>,
    domain_receipt_digest: Vec<u8>,
    checkpoint_digest: Option<Vec<u8>>,
    outbox_intent_digest: Option<Vec<u8>>,
    genesis_state_digest: Vec<u8>,
    genesis_head_digest: Vec<u8>,
}

impl SqlRequestValues {
    fn new(
        request: &StoreTransactionRequest,
        canonical_request_digest: &ContentDigest,
        genesis: &StorePhysicalHead,
    ) -> ControlStoreResult<Self> {
        Ok(Self {
            version: i16::try_from(request.version())
                .map_err(|_| corrupt("STORE_REQUEST_VERSION_INVALID"))?,
            transaction_id: request.transaction_id().as_str().to_owned(),
            project_id: request.scope().project_id().as_str().to_owned(),
            project_snapshot_id: request.scope().project_snapshot_id().as_str().to_owned(),
            repository_owner: request.scope().owner().as_str().to_owned(),
            aggregate_key_digest: digest_bytes(request.scope().aggregate_key_digest())?,
            request_digest: digest_bytes(canonical_request_digest)?,
            authority_runtime: "LIVE".to_owned(),
            daemon_instance_id: request
                .expected_authority()
                .daemon_instance_id()
                .as_str()
                .to_owned(),
            daemon_epoch: signed_bigint(request.expected_authority().daemon_epoch().get())?,
            admission_mode: request.expected_authority().admission().as_str().to_owned(),
            authority_revision: signed_bigint(request.expected_authority().revision().get())?,
            authority_observation_digest: digest_bytes(
                request.expected_authority().observation_digest(),
            )?,
            authority_head_digest: digest_bytes(request.expected_authority().head_digest())?,
            expected_head_runtime: "LIVE".to_owned(),
            expected_revision: signed_bigint(request.expected_head().revision().get())?,
            expected_state_digest: digest_bytes(request.expected_head().state_digest())?,
            expected_head_digest: digest_bytes(request.expected_head().head_digest())?,
            domain_command_digest: digest_bytes(request.mutation().domain_command_digest())?,
            record_set_digest: digest_bytes(request.mutation().record_set_digest())?,
            next_state_digest: digest_bytes(request.mutation().next_state_digest())?,
            domain_receipt_digest: digest_bytes(request.mutation().domain_receipt_digest())?,
            checkpoint_digest: request
                .mutation()
                .checkpoint_digest()
                .map(digest_bytes)
                .transpose()?,
            outbox_intent_digest: request
                .mutation()
                .outbox_intent_digest()
                .map(digest_bytes)
                .transpose()?,
            genesis_state_digest: digest_bytes(genesis.state_digest())?,
            genesis_head_digest: digest_bytes(genesis.head_digest())?,
        })
    }
}

struct PrepareRow {
    status: String,
    database_uuid: String,
    database_identity_digest: Option<Vec<u8>>,
    schema_version: i16,
    manifest_sha256: String,
    head_found: bool,
    before_revision: i64,
    before_state_digest: Vec<u8>,
    before_head_digest: Vec<u8>,
    after_revision: Option<i64>,
    after_state_digest: Option<Vec<u8>>,
    after_head_digest: Option<Vec<u8>>,
    disposition: Option<String>,
    transaction_digest: Option<Vec<u8>>,
    receipt_digest: Option<Vec<u8>>,
    global_schema_version: i16,
    global_manifest_sha256: String,
}

enum AttemptFailure {
    Retryable,
    CommitOutcomeUnknown,
    Terminal(ControlStoreError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommitFailureClass {
    Retryable,
    OutcomeUnknown,
    Terminal,
}

enum FinalizeFailure {
    Database(PostgresError),
    Invalid(ControlStoreError),
}

fn run_attempt(
    client: &mut Client,
    context: &AttemptContext,
) -> Result<StoreTransactionReceipt, AttemptFailure> {
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|error| classify_query_error(&error))?;
    if let Err(error) = transaction.batch_execute(WRITE_TRANSACTION_SETTINGS) {
        return rollback_attempt(transaction, classify_query_error(&error));
    }
    let row = match call_prepare(&mut transaction, context) {
        Ok(row) => row,
        Err(error) => return rollback_attempt(transaction, classify_query_error(&error)),
    };
    let prepared = match parse_prepare_row(&row) {
        Ok(prepared) => prepared,
        Err(error) => return rollback_attempt(transaction, AttemptFailure::Terminal(error)),
    };
    if let Err(error) = validate_prepare_identity(&prepared, context) {
        return rollback_attempt(transaction, AttemptFailure::Terminal(error));
    }

    let receipt = match prepared.status.as_str() {
        "REPLAY" => match reconstruct_replay(&prepared, context) {
            Ok(receipt) => receipt,
            Err(error) => return rollback_attempt(transaction, AttemptFailure::Terminal(error)),
        },
        "PREPARED" => {
            let receipt = match build_prepared_receipt(&prepared, context) {
                Ok(receipt) => receipt,
                Err(error) => {
                    return rollback_attempt(transaction, AttemptFailure::Terminal(error));
                }
            };
            let finalized = match call_finalize(&mut transaction, &context.sql, context, &receipt) {
                Ok(finalized) => finalized,
                Err(FinalizeFailure::Database(error)) => {
                    return rollback_attempt(transaction, classify_query_error(&error));
                }
                Err(FinalizeFailure::Invalid(error)) => {
                    return rollback_attempt(transaction, AttemptFailure::Terminal(error));
                }
            };
            if finalized != "FINALIZED" {
                return rollback_attempt(
                    transaction,
                    AttemptFailure::Terminal(corrupt("STORE_FINALIZE_STATUS_INVALID")),
                );
            }
            receipt
        }
        _ => {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(corrupt("STORE_PREPARE_STATUS_INVALID")),
            );
        }
    };

    transaction
        .commit()
        .map(|()| receipt)
        .map_err(|error| classify_commit_error(&error))
}

fn call_prepare(
    transaction: &mut Transaction<'_>,
    context: &AttemptContext,
) -> Result<Row, PostgresError> {
    let values = &context.sql;
    transaction.query_one(
        PREPARE_SQL,
        &[
            &context.global_schema_version,
            &context.global_manifest_sha256,
            &values.version,
            &values.transaction_id,
            &values.project_id,
            &values.project_snapshot_id,
            &values.repository_owner,
            &values.aggregate_key_digest,
            &values.request_digest,
            &values.authority_runtime,
            &values.daemon_instance_id,
            &values.daemon_epoch,
            &values.admission_mode,
            &values.authority_revision,
            &values.authority_observation_digest,
            &values.authority_head_digest,
            &values.expected_head_runtime,
            &values.expected_revision,
            &values.expected_state_digest,
            &values.expected_head_digest,
            &values.domain_command_digest,
            &values.record_set_digest,
            &values.next_state_digest,
            &values.domain_receipt_digest,
            &values.checkpoint_digest,
            &values.outbox_intent_digest,
            &values.genesis_state_digest,
            &values.genesis_head_digest,
        ],
    )
}

#[allow(clippy::too_many_lines)]
fn call_finalize(
    transaction: &mut Transaction<'_>,
    values: &SqlRequestValues,
    context: &AttemptContext,
    receipt: &StoreTransactionReceipt,
) -> Result<String, FinalizeFailure> {
    let persistence = receipt
        .persistence()
        .ok_or_else(|| FinalizeFailure::Invalid(corrupt("STORE_PERSISTENCE_EVIDENCE_MISSING")))?;
    let database_identity_digest =
        digest_bytes(persistence.database_identity_digest()).map_err(FinalizeFailure::Invalid)?;
    let before_revision =
        signed_bigint(receipt.before_head().revision().get()).map_err(FinalizeFailure::Invalid)?;
    let before_state_digest =
        digest_bytes(receipt.before_head().state_digest()).map_err(FinalizeFailure::Invalid)?;
    let before_head_digest =
        digest_bytes(receipt.before_head().head_digest()).map_err(FinalizeFailure::Invalid)?;
    let after_revision =
        signed_bigint(receipt.after_head().revision().get()).map_err(FinalizeFailure::Invalid)?;
    let after_state_digest =
        digest_bytes(receipt.after_head().state_digest()).map_err(FinalizeFailure::Invalid)?;
    let after_head_digest =
        digest_bytes(receipt.after_head().head_digest()).map_err(FinalizeFailure::Invalid)?;
    let disposition = receipt.disposition().as_str();
    let transaction_digest =
        digest_bytes(receipt.transaction_digest()).map_err(FinalizeFailure::Invalid)?;
    let receipt_digest =
        digest_bytes(receipt.receipt_digest()).map_err(FinalizeFailure::Invalid)?;

    let row = transaction
        .query_one(
            FINALIZE_SQL,
            &[
                &context.global_schema_version,
                &context.global_manifest_sha256,
                &values.version,
                &values.transaction_id,
                &values.project_id,
                &values.project_snapshot_id,
                &values.repository_owner,
                &values.aggregate_key_digest,
                &values.request_digest,
                &values.authority_runtime,
                &values.daemon_instance_id,
                &values.daemon_epoch,
                &values.admission_mode,
                &values.authority_revision,
                &values.authority_observation_digest,
                &values.authority_head_digest,
                &values.expected_head_runtime,
                &values.expected_revision,
                &values.expected_state_digest,
                &values.expected_head_digest,
                &values.domain_command_digest,
                &values.record_set_digest,
                &values.next_state_digest,
                &values.domain_receipt_digest,
                &values.checkpoint_digest,
                &values.outbox_intent_digest,
                &values.genesis_state_digest,
                &values.genesis_head_digest,
                &context.database_uuid,
                &database_identity_digest,
                &context.schema_version,
                &context.manifest_sha256,
                &before_revision,
                &before_state_digest,
                &before_head_digest,
                &after_revision,
                &after_state_digest,
                &after_head_digest,
                &disposition,
                &transaction_digest,
                &receipt_digest,
            ],
        )
        .map_err(FinalizeFailure::Database)?;
    row.try_get(0).map_err(FinalizeFailure::Database)
}

fn parse_prepare_row(row: &Row) -> ControlStoreResult<PrepareRow> {
    if row.len() != 17 {
        return Err(corrupt("STORE_DATABASE_ROW_INVALID"));
    }
    Ok(PrepareRow {
        status: row_value(row, 0)?,
        database_uuid: row_value(row, 1)?,
        database_identity_digest: row_value(row, 2)?,
        schema_version: row_value(row, 3)?,
        manifest_sha256: row_value(row, 4)?,
        head_found: row_value(row, 5)?,
        before_revision: row_value(row, 6)?,
        before_state_digest: row_value(row, 7)?,
        before_head_digest: row_value(row, 8)?,
        after_revision: row_value(row, 9)?,
        after_state_digest: row_value(row, 10)?,
        after_head_digest: row_value(row, 11)?,
        disposition: row_value(row, 12)?,
        transaction_digest: row_value(row, 13)?,
        receipt_digest: row_value(row, 14)?,
        global_schema_version: row_value(row, 15)?,
        global_manifest_sha256: row_value(row, 16)?,
    })
}

fn validate_prepare_identity(
    prepared: &PrepareRow,
    context: &AttemptContext,
) -> ControlStoreResult<()> {
    if prepared.database_uuid != context.database_uuid
        || prepared.schema_version != context.schema_version
        || prepared.manifest_sha256 != context.manifest_sha256
        || !global_persistence_matches(
            prepared.global_schema_version,
            &prepared.global_manifest_sha256,
            context.global_schema_version,
            &context.global_manifest_sha256,
        )
    {
        return Err(corrupt("STORE_PERSISTENCE_IDENTITY_MISMATCH"));
    }
    Ok(())
}

fn reconstruct_replay(
    prepared: &PrepareRow,
    context: &AttemptContext,
) -> ControlStoreResult<StoreTransactionReceipt> {
    if !prepared.head_found {
        return Err(corrupt("STORE_REPLAY_SHAPE_INVALID"));
    }
    let retained_identity = prepared
        .database_identity_digest
        .as_ref()
        .ok_or_else(|| corrupt("STORE_REPLAY_SHAPE_INVALID"))?;
    if bytes_digest(retained_identity)? != *context.persistence.database_identity_digest() {
        return Err(corrupt("STORE_DATABASE_IDENTITY_SUBSTITUTED"));
    }
    let after_revision = prepared
        .after_revision
        .ok_or_else(|| corrupt("STORE_REPLAY_SHAPE_INVALID"))?;
    let after_state = prepared
        .after_state_digest
        .as_ref()
        .ok_or_else(|| corrupt("STORE_REPLAY_SHAPE_INVALID"))?;
    let after_head = prepared
        .after_head_digest
        .as_ref()
        .ok_or_else(|| corrupt("STORE_REPLAY_SHAPE_INVALID"))?;
    let disposition = match prepared.disposition.as_deref() {
        Some("APPLIED") => StoreReceiptDisposition::Applied,
        Some("STALE_PHYSICAL_HEAD") => StoreReceiptDisposition::StalePhysicalHead,
        _ => return Err(corrupt("STORE_REPLAY_DISPOSITION_INVALID")),
    };
    let before = stored_head(
        context.request.scope(),
        prepared.before_revision,
        &prepared.before_state_digest,
        &prepared.before_head_digest,
    )?;
    let after = stored_head(
        context.request.scope(),
        after_revision,
        after_state,
        after_head,
    )?;
    let receipt = build_live_receipt(
        context.request.clone(),
        context.persistence.clone(),
        context.request_digest.clone(),
        before,
        after,
        disposition,
    )?;
    let retained_transaction = prepared
        .transaction_digest
        .as_ref()
        .ok_or_else(|| corrupt("STORE_REPLAY_SHAPE_INVALID"))?;
    let retained_receipt = prepared
        .receipt_digest
        .as_ref()
        .ok_or_else(|| corrupt("STORE_REPLAY_SHAPE_INVALID"))?;
    if bytes_digest(retained_transaction)? != *receipt.transaction_digest()
        || bytes_digest(retained_receipt)? != *receipt.receipt_digest()
    {
        return Err(corrupt("STORE_REPLAY_DIGEST_CORRUPT"));
    }
    Ok(receipt)
}

fn build_prepared_receipt(
    prepared: &PrepareRow,
    context: &AttemptContext,
) -> ControlStoreResult<StoreTransactionReceipt> {
    if prepared.database_identity_digest.is_some()
        || prepared.after_revision.is_some()
        || prepared.after_state_digest.is_some()
        || prepared.after_head_digest.is_some()
        || prepared.disposition.is_some()
        || prepared.transaction_digest.is_some()
        || prepared.receipt_digest.is_some()
    {
        return Err(corrupt("STORE_PREPARED_SHAPE_INVALID"));
    }
    let before = stored_head(
        context.request.scope(),
        prepared.before_revision,
        &prepared.before_state_digest,
        &prepared.before_head_digest,
    )?;
    if !prepared.head_found && before != context.genesis {
        return Err(corrupt("STORE_GENESIS_SUBSTITUTED"));
    }

    let (after, disposition) = if before == *context.request.expected_head() {
        let next_revision = before
            .revision()
            .get()
            .checked_add(1)
            .and_then(|revision| StoreRevision::new(revision).ok())
            .ok_or_else(|| {
                store_error(
                    ControlStoreErrorKind::RevisionOverflow,
                    "STORE_REVISION_OVERFLOW",
                )
            })?;
        (
            physical_head(
                RuntimeKind::Live,
                context.request.scope().clone(),
                next_revision,
                context.request.mutation().next_state_digest().clone(),
            )?,
            StoreReceiptDisposition::Applied,
        )
    } else {
        (before.clone(), StoreReceiptDisposition::StalePhysicalHead)
    };

    build_live_receipt(
        context.request.clone(),
        context.persistence.clone(),
        context.request_digest.clone(),
        before,
        after,
        disposition,
    )
}

fn parse_current_head(
    row: &Row,
    scope: &StoreScope,
    expected_database_uuid: &str,
    expected_schema_version: i16,
    expected_manifest_sha256: &str,
    expected_global_schema_version: i16,
    expected_global_manifest_sha256: &str,
) -> ControlStoreResult<StorePhysicalHead> {
    if row.len() != 9 {
        return Err(corrupt("STORE_DATABASE_ROW_INVALID"));
    }
    let database_uuid: String = row_value(row, 0)?;
    let schema_version: i16 = row_value(row, 1)?;
    let manifest_sha256: String = row_value(row, 2)?;
    let found: bool = row_value(row, 3)?;
    let revision: Option<i64> = row_value(row, 4)?;
    let state_digest: Option<Vec<u8>> = row_value(row, 5)?;
    let head_digest: Option<Vec<u8>> = row_value(row, 6)?;
    let global_schema_version: i16 = row_value(row, 7)?;
    let global_manifest_sha256: String = row_value(row, 8)?;

    if database_uuid != expected_database_uuid
        || schema_version != expected_schema_version
        || manifest_sha256 != expected_manifest_sha256
        || !global_persistence_matches(
            global_schema_version,
            &global_manifest_sha256,
            expected_global_schema_version,
            expected_global_manifest_sha256,
        )
    {
        return Err(corrupt("STORE_PERSISTENCE_IDENTITY_MISMATCH"));
    }
    match (found, revision, state_digest, head_digest) {
        (false, None, None, None) => genesis_head(RuntimeKind::Live, scope.clone()),
        (true, Some(revision), Some(state), Some(head)) => {
            stored_head(scope, revision, &state, &head)
        }
        _ => Err(corrupt("STORE_CURRENT_HEAD_SHAPE_INVALID")),
    }
}

fn stored_head(
    scope: &StoreScope,
    revision: i64,
    state_digest: &[u8],
    retained_head_digest: &[u8],
) -> ControlStoreResult<StorePhysicalHead> {
    let revision = u64::try_from(revision)
        .ok()
        .and_then(|revision| StoreRevision::new(revision).ok())
        .ok_or_else(|| corrupt("STORE_PHYSICAL_REVISION_INVALID"))?;
    let state_digest = bytes_digest(state_digest)?;
    let retained_head_digest = bytes_digest(retained_head_digest)?;
    let canonical = physical_head(RuntimeKind::Live, scope.clone(), revision, state_digest)?;
    validate_physical_head(&canonical)?;
    if canonical.head_digest() != &retained_head_digest {
        return Err(corrupt("STORE_PHYSICAL_HEAD_CORRUPT"));
    }
    Ok(canonical)
}

fn rollback_attempt(
    transaction: Transaction<'_>,
    failure: AttemptFailure,
) -> Result<StoreTransactionReceipt, AttemptFailure> {
    match transaction.rollback() {
        Ok(()) => Err(failure),
        Err(_) => Err(AttemptFailure::Terminal(store_error(
            ControlStoreErrorKind::Unavailable,
            "STORE_ROLLBACK_UNAVAILABLE",
        ))),
    }
}

fn rollback_current<T>(
    transaction: Transaction<'_>,
    error: ControlStoreError,
) -> ControlStoreResult<T> {
    match transaction.rollback() {
        Ok(()) => Err(error),
        Err(_) => Err(store_error(
            ControlStoreErrorKind::Unavailable,
            "STORE_ROLLBACK_UNAVAILABLE",
        )),
    }
}

fn classify_query_error(error: &PostgresError) -> AttemptFailure {
    if is_retryable(error) {
        AttemptFailure::Retryable
    } else {
        AttemptFailure::Terminal(map_database_error(error))
    }
}

fn classify_commit_error(error: &PostgresError) -> AttemptFailure {
    let database = error.as_db_error();
    match commit_failure_class(
        database.map(|value| value.code().code()),
        database.and_then(|value| value.constraint()),
    ) {
        CommitFailureClass::Retryable => AttemptFailure::Retryable,
        CommitFailureClass::OutcomeUnknown => AttemptFailure::CommitOutcomeUnknown,
        CommitFailureClass::Terminal => AttemptFailure::Terminal(map_database_error(error)),
    }
}

fn commit_failure_class(code: Option<&str>, constraint: Option<&str>) -> CommitFailureClass {
    match code {
        Some(code) if retryable_sqlstate(code, constraint) => CommitFailureClass::Retryable,
        Some(_) => CommitFailureClass::Terminal,
        None => CommitFailureClass::OutcomeUnknown,
    }
}

fn is_retryable(error: &PostgresError) -> bool {
    error
        .as_db_error()
        .is_some_and(|database| retryable_sqlstate(database.code().code(), database.constraint()))
}

fn retryable_sqlstate(code: &str, constraint: Option<&str>) -> bool {
    matches!(code, "40001" | "40P01")
        || (code == "23505"
            && matches!(
                constraint,
                Some("physical_heads_pkey" | "terminal_transactions_pkey")
            ))
}

fn map_database_error(error: &PostgresError) -> ControlStoreError {
    let Some(code) = error.as_db_error().map(|db_error| db_error.code().code()) else {
        return store_error(ControlStoreErrorKind::Unavailable, "STORE_LIVE_UNAVAILABLE");
    };
    map_database_sqlstate(code)
}

fn map_database_sqlstate(code: &str) -> ControlStoreError {
    match code {
        "LTX01" => store_error(
            ControlStoreErrorKind::CommandSubstitution,
            "STORE_TRANSACTION_SUBSTITUTED",
        ),
        "LAD01" => store_error(
            ControlStoreErrorKind::AdmissionDenied,
            "STORE_ADMISSION_DENIED",
        ),
        "LAU01" => store_error(
            ControlStoreErrorKind::AuthorityMismatch,
            "STORE_AUTHORITY_MISMATCH",
        ),
        "LRV01" => store_error(
            ControlStoreErrorKind::RevisionOverflow,
            "STORE_REVISION_OVERFLOW",
        ),
        "42501" => store_error(
            ControlStoreErrorKind::Unavailable,
            "STORE_LIVE_PERMISSION_DENIED",
        ),
        "55P03" | "57014" => store_error(ControlStoreErrorKind::Unavailable, "STORE_LIVE_TIMEOUT"),
        "LST01" | "LST02" | "LCR01" => corrupt("STORE_LIVE_STATE_CORRUPT"),
        _ => corrupt("STORE_LIVE_DATABASE_REJECTED"),
    }
}

fn map_current_error(error: &PostgresError) -> ControlStoreError {
    map_database_error(error)
}

fn map_setup_error(error: PostgresStoreSetupError) -> ControlStoreError {
    let kind = match error.kind() {
        PostgresStoreSetupErrorKind::TransactionFailed
        | PostgresStoreSetupErrorKind::NetworkBoundary => ControlStoreErrorKind::Unavailable,
        PostgresStoreSetupErrorKind::CommitOutcomeUnknown => {
            ControlStoreErrorKind::CommitOutcomeUnknown
        }
        _ => ControlStoreErrorKind::CorruptState,
    };
    store_error(kind, error.code())
}

fn row_value<T: FromSqlOwned>(row: &Row, index: usize) -> ControlStoreResult<T> {
    row.try_get(index)
        .map_err(|_| corrupt("STORE_DATABASE_ROW_INVALID"))
}

fn global_persistence_matches(
    observed_schema_version: i16,
    observed_manifest_sha256: &str,
    expected_schema_version: i16,
    expected_manifest_sha256: &str,
) -> bool {
    observed_schema_version == expected_schema_version
        && observed_manifest_sha256 == expected_manifest_sha256
}

fn signed_bigint(value: u64) -> ControlStoreResult<i64> {
    i64::try_from(value).map_err(|_| corrupt("STORE_SIGNED_BIGINT_INVALID"))
}

fn digest_bytes(digest: &ContentDigest) -> ControlStoreResult<Vec<u8>> {
    let bytes = digest.as_str().as_bytes();
    if bytes.len() != 64 {
        return Err(corrupt("STORE_DIGEST_INVALID"));
    }
    let mut output = Vec::with_capacity(32);
    for pair in bytes.chunks_exact(2) {
        let high = hex_nibble(pair[0]).ok_or_else(|| corrupt("STORE_DIGEST_INVALID"))?;
        let low = hex_nibble(pair[1]).ok_or_else(|| corrupt("STORE_DIGEST_INVALID"))?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn bytes_digest(bytes: &[u8]) -> ControlStoreResult<ContentDigest> {
    if bytes.len() != 32 {
        return Err(corrupt("STORE_DATABASE_DIGEST_INVALID"));
    }
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(LOWER_HEX[usize::from(byte >> 4)]));
        output.push(char::from(LOWER_HEX[usize::from(byte & 0x0f)]));
    }
    ContentDigest::from_sha256(output).map_err(|_| corrupt("STORE_DATABASE_DIGEST_INVALID"))
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

const fn store_error(kind: ControlStoreErrorKind, code: &'static str) -> ControlStoreError {
    ControlStoreError::new(kind, code)
}

const fn corrupt(code: &'static str) -> ControlStoreError {
    store_error(ControlStoreErrorKind::CorruptState, code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_transactions_have_fixed_lock_and_statement_timeouts() {
        for settings in [WRITE_TRANSACTION_SETTINGS, READ_TRANSACTION_SETTINGS] {
            assert!(settings.contains("SET LOCAL lock_timeout = '5s'"));
            assert!(settings.contains("SET LOCAL statement_timeout = '30s'"));
        }
    }

    #[test]
    fn runtime_queries_append_global_identity_without_changing_store_profile_columns() {
        assert!(
            PREPARE_SQL
                .contains("terminal_receipt_digest, global_schema_version, global_manifest_sha256")
        );
        assert!(
            CURRENT_HEAD_SQL.contains("head_digest, global_schema_version, global_manifest_sha256")
        );
        assert!(PREPARE_SQL.contains("schema_version, manifest_sha256, head_found"));
    }

    #[test]
    fn global_persistence_comparison_is_exact() {
        assert!(global_persistence_matches(4, "manifest-a", 4, "manifest-a"));
        assert!(!global_persistence_matches(
            4,
            "manifest-a",
            3,
            "manifest-a"
        ));
        assert!(!global_persistence_matches(
            4,
            "manifest-b",
            4,
            "manifest-a"
        ));
    }

    #[test]
    fn retryable_sqlstate_is_limited_to_reconcilable_store_constraints() {
        assert!(retryable_sqlstate("40001", None));
        assert!(retryable_sqlstate("40P01", None));
        assert!(retryable_sqlstate("23505", Some("physical_heads_pkey")));
        assert!(retryable_sqlstate(
            "23505",
            Some("terminal_transactions_pkey")
        ));
        for constraint in [None, Some("other_unique"), Some("task_ledger_streams_pkey")] {
            assert!(!retryable_sqlstate("23505", constraint));
        }
    }

    #[test]
    fn commit_classification_only_poisons_when_database_did_not_respond() {
        assert_eq!(
            commit_failure_class(None, None),
            CommitFailureClass::OutcomeUnknown
        );
        assert_eq!(
            commit_failure_class(Some("40001"), None),
            CommitFailureClass::Retryable
        );
        assert_eq!(
            commit_failure_class(Some("23505"), Some("other_unique")),
            CommitFailureClass::Terminal
        );
        assert_eq!(
            commit_failure_class(Some("57014"), None),
            CommitFailureClass::Terminal
        );
    }

    #[test]
    fn timeout_sqlstates_map_to_terminal_unavailable() {
        for code in ["55P03", "57014"] {
            let mapped = map_database_sqlstate(code);
            assert_eq!(mapped.kind(), ControlStoreErrorKind::Unavailable);
            assert_eq!(mapped.code(), "STORE_LIVE_TIMEOUT");
        }
    }
}
