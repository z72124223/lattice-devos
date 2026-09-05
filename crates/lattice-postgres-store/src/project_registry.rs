//! Live durable Project Registry repository backed by fixed `PostgreSQL` functions.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    ContentDigest, DaemonEpoch, GitRefIdentity, ProjectAuthorityHead, ProjectAuthorityReceipt,
    ProjectClass, ProjectId, ProjectLifecycle, ProjectSnapshotId, RuntimeAdmissionMode,
    RuntimeKind, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId,
};
use lattice_project_registry::{
    CommandId, IdentityDimension, IdentityDrift, ReconciliationDecision, RegistryCheckpoint,
    RegistryCommand, RegistryCommandOutcome, RegistryCommandRecord, RegistryDenial, RegistryError,
    RegistryIdentityReservation, RegistryProjectProjection, RegistryProjectRow,
    RegistryReservationStatus, RepositoryObservation, UntrustedRegistrySnapshot,
    VerifiedRegistryState, apply_command_plan, plan_command,
    verify_untrusted_registry_snapshot_against_checkpoint,
};
use postgres::types::{FromSqlOwned, ToSql};
use postgres::{Client, Error as PostgresError, GenericClient, IsolationLevel, Row, Transaction};

use crate::migrations::{CURRENT_V5_MANIFEST_SHA256, CURRENT_V7_MANIFEST_SHA256};
use crate::postgres_setup::verify_runtime_store_schema;
use crate::{MigrationTarget, PostgresStoreSetupErrorKind};

const FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION: u16 = 5;
const CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION: u16 = 7;
const EXTERNAL_ADOPTION_GLOBAL_REGISTRY_SCHEMA_VERSION: u16 = 8;
const REGISTRY_V4_MANIFEST_SHA256: &str =
    "df3f7ca3687afaa0d1f676158725e6d2f06670e0612df7482aa9d4d244b59f0f";
const REGISTRY_CATALOG_ID: &str = "PROJECT_REGISTRY_V1";
const REGISTRY_ADAPTER_PRODUCER_ID: &str = "lattice-postgres-store";
const REGISTRY_ADAPTER_PRODUCER_VERSION: &str = "1.4";
const MAX_LIVE_SERIALIZATION_RETRIES: u8 = 3;
const EXECUTION_DEADLINE: Duration = Duration::from_secs(45);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegistrySqlProfile {
    FrozenV5,
    CurrentV7,
    CurrentV8,
}

impl RegistrySqlProfile {
    fn from_schema_version(schema_version: u16) -> PostgresProjectRegistryResult<Self> {
        match schema_version {
            FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION => Ok(Self::FrozenV5),
            CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION => Ok(Self::CurrentV7),
            EXTERNAL_ADOPTION_GLOBAL_REGISTRY_SCHEMA_VERSION => Ok(Self::CurrentV8),
            _ => Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
        }
    }

    const fn schema_version(self) -> u16 {
        match self {
            Self::FrozenV5 => FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION,
            Self::CurrentV7 => CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION,
            Self::CurrentV8 => EXTERNAL_ADOPTION_GLOBAL_REGISTRY_SCHEMA_VERSION,
        }
    }
}

const WRITE_TRANSACTION_SETTINGS: &str = "\
    SET LOCAL search_path = pg_catalog; \
    SET LOCAL row_security = on; \
    SET LOCAL synchronous_commit = on; \
    SELECT pg_catalog.set_config('lock_timeout', \
        CASE WHEN pg_catalog.current_setting('lock_timeout') = '0' THEN '5000' \
             ELSE LEAST(pg_catalog.floor(EXTRACT(EPOCH FROM \
                 pg_catalog.current_setting('lock_timeout')::pg_catalog.interval) * 1000)::bigint, \
                 5000::bigint)::text END, true); \
    SELECT pg_catalog.set_config('statement_timeout', \
        CASE WHEN pg_catalog.current_setting('statement_timeout') = '0' THEN '30000' \
             ELSE LEAST(pg_catalog.floor(EXTRACT(EPOCH FROM \
                 pg_catalog.current_setting('statement_timeout')::pg_catalog.interval) * 1000)::bigint, \
                 30000::bigint)::text END, true); \
    SELECT pg_catalog.set_config('idle_in_transaction_session_timeout', \
        CASE WHEN pg_catalog.current_setting('idle_in_transaction_session_timeout') = '0' THEN '30000' \
             ELSE LEAST(pg_catalog.floor(EXTRACT(EPOCH FROM \
                 pg_catalog.current_setting('idle_in_transaction_session_timeout')::pg_catalog.interval) * 1000)::bigint, \
                 30000::bigint)::text END, true)";

const READ_TRANSACTION_SETTINGS: &str = "\
    SET LOCAL search_path = pg_catalog; \
    SET LOCAL row_security = on; \
    SELECT pg_catalog.set_config('lock_timeout', \
        CASE WHEN pg_catalog.current_setting('lock_timeout') = '0' THEN '5000' \
             ELSE LEAST(pg_catalog.floor(EXTRACT(EPOCH FROM \
                 pg_catalog.current_setting('lock_timeout')::pg_catalog.interval) * 1000)::bigint, \
                 5000::bigint)::text END, true); \
    SELECT pg_catalog.set_config('statement_timeout', \
        CASE WHEN pg_catalog.current_setting('statement_timeout') = '0' THEN '30000' \
             ELSE LEAST(pg_catalog.floor(EXTRACT(EPOCH FROM \
                 pg_catalog.current_setting('statement_timeout')::pg_catalog.interval) * 1000)::bigint, \
                 30000::bigint)::text END, true); \
    SELECT pg_catalog.set_config('idle_in_transaction_session_timeout', \
        CASE WHEN pg_catalog.current_setting('idle_in_transaction_session_timeout') = '0' THEN '30000' \
             ELSE LEAST(pg_catalog.floor(EXTRACT(EPOCH FROM \
                 pg_catalog.current_setting('idle_in_transaction_session_timeout')::pg_catalog.interval) * 1000)::bigint, \
                 30000::bigint)::text END, true)";

const PREPARE_SQL: &str = "\
    SELECT prepare_status, retained_request_digest, retained_result_digest, \
           retained_record_set_digest, retained_persistence_receipt_digest, \
           retained_base_checkpoint_digest, retained_result_checkpoint_digest, \
           current_ordinal, current_observation_count, current_project_count, \
           current_command_count, current_reservation_count, current_retained_bytes, \
           current_checkpoint_digest \
      FROM control.project_registry_prepare_v2(\
           $1::smallint,$2::text,$3::text,$4::bytea,$5::text,$6::text,$7::bigint,\
           $8::text,$9::bigint,$10::bytea,$11::bytea,$12::bytea)";

const READ_STATE_SQL: &str = "\
    SELECT runtime, command_ordinal, observation_count, project_count, command_count, \
           reservation_count, retained_bytes, checkpoint_digest, stage_command_id \
       FROM control.project_registry_read_state_v2($1::smallint,$2::text)";

const READ_OBSERVATIONS_SQL: &str = "\
    SELECT observation_digest, canonical_root, root_identity_digest, \
           repository_identity_digest, file_identity_digest, primary_ref, \
           primary_ref_storage_digest \
       FROM control.project_registry_read_observations_v2($1::smallint,$2::text)";

const READ_PROJECTS_SQL: &str = "\
    SELECT project_id, project_class, accepted_observation_digest, pending_observation_digest, \
           drift_canonical_root, drift_repository, drift_file, drift_primary_ref_name, \
           drift_primary_ref_storage, authority_contract_version, authority_producer_id, \
           authority_producer_version, authority_runtime, authority_snapshot_id, \
           authority_registry_revision, authority_lifecycle, authority_primary_ref, \
           authority_primary_ref_storage_digest, authority_observation_digest, \
           authority_receipt_digest \
       FROM control.project_registry_read_projects_v2($1::smallint,$2::text)";

const READ_COMMANDS_SQL: &str = "\
    SELECT ordinal, command_id, action, project_id, project_class, observation_digest, \
           before_present, before_producer_id, before_producer_version, before_runtime, \
           before_project_id, before_snapshot_id, before_registry_revision::text, \
           before_lifecycle, before_project_class, before_primary_ref, \
           before_primary_ref_storage_digest, before_observation_digest, before_receipt_digest, \
           decision, evidence_digest, request_digest, outcome, denial_reason, denial_dimension, \
           denial_existing_project_id, denial_lifecycle, denial_expected_decision, \
           denial_found_decision, semantic_before_receipt_digest, \
           semantic_after_receipt_digest, authority_receipt_digest, drift_canonical_root, \
           drift_repository, drift_file, drift_primary_ref_name, drift_primary_ref_storage, \
           result_digest, base_runtime, base_ordinal, base_observation_count, \
           base_project_count, base_command_count, base_reservation_count, base_retained_bytes, \
           base_checkpoint_digest, result_runtime, result_ordinal, result_observation_count, \
           result_project_count, result_command_count, result_reservation_count, \
           result_retained_bytes, result_checkpoint_digest, record_set_digest, \
           authority_runtime, daemon_instance_id, daemon_epoch, admission_mode, \
           daemon_authority_revision, daemon_observation_digest, daemon_head_digest, \
           transaction_digest, persistence_receipt_digest, \
           persistence_schema_version, persistence_manifest_sha256 \
      FROM control.project_registry_read_commands_v2($1::smallint,$2::text)";

const READ_RESERVATIONS_SQL: &str = "\
    SELECT dimension, identity_digest, reservation_status, project_id \
       FROM control.project_registry_read_reservations_v2($1::smallint,$2::text)";

const STAGE_COMMAND_SQL: &str = "\
    SELECT control.project_registry_stage_command_v2(\
      $1::smallint,$2::text,$3::smallint,$4::text,$5::bigint,$6::text,$7::text,\
      $8::text,$9::text,$10::bytea,$11::boolean,$12::text,$13::text,$14::text,\
      $15::text,$16::text,$17::text::numeric,$18::text,$19::text,$20::text,$21::bytea,\
      $22::bytea,$23::bytea,$24::text,$25::bytea,$26::bytea,$27::text,$28::text,\
      $29::text,$30::text,$31::text,$32::text,$33::text,$34::bytea,$35::bytea,\
      $36::bytea,$37::boolean,$38::boolean,$39::boolean,$40::boolean,$41::boolean,\
      $42::bytea,$43::text,$44::bigint,$45::bigint,$46::bigint,$47::bigint,\
      $48::bigint,$49::bigint,$50::bytea,$51::text,$52::bigint,$53::bigint,\
      $54::bigint,$55::bigint,$56::bigint,$57::bigint,$58::bytea,$59::bytea,\
      $60::text,$61::text,$62::bigint,$63::text,$64::bigint,$65::bytea,$66::bytea,\
      $67::bytea,$68::bytea,$69::boolean,$70::text,$71::bytea,$72::bytea,\
      $73::bytea,$74::text,$75::bytea)";

const STAGE_PROJECT_SQL: &str = "\
    SELECT control.project_registry_stage_project_v2(\
      $1::smallint,$2::text,$3::text,$4::text,$5::bytea,$6::bytea,$7::boolean,\
      $8::boolean,$9::boolean,$10::boolean,$11::boolean,$12::smallint,$13::text,\
      $14::text,$15::text,$16::text,$17::text::numeric,$18::text,$19::text,\
      $20::bytea,$21::bytea,$22::bytea)";

const FINALIZE_SQL: &str = "\
    SELECT control.project_registry_finalize_v2(\
      $1::smallint,$2::text,$3::text,$4::bigint,$5::text,$6::bigint,$7::bigint,\
      $8::bigint,$9::bigint,$10::bigint,$11::bigint,$12::bytea,$13::text,\
      $14::bigint,$15::bigint,$16::bigint,$17::bigint,$18::bigint,$19::bigint,\
      $20::bytea,$21::bytea,$22::bytea,$23::bytea,$24::boolean,$25::boolean,\
      $26::bigint,$27::bigint)";

/// Result returned by the live durable Project Registry adapter.
pub type PostgresProjectRegistryResult<T> = Result<T, PostgresProjectRegistryError>;

/// Closed, diagnostic-free Project Registry persistence failures.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PostgresProjectRegistryErrorKind {
    Malformed,
    CommandSubstitution,
    AdmissionDenied,
    AuthorityMismatch,
    CheckpointChanged,
    RetainedRowCorrupt,
    CapacityExceeded,
    RevisionOverflow,
    SerializationExhausted,
    TransactionFailed,
    Unavailable,
    CommitOutcomeUnknown,
}

impl PostgresProjectRegistryErrorKind {
    pub const ALL: [Self; 12] = [
        Self::Malformed,
        Self::CommandSubstitution,
        Self::AdmissionDenied,
        Self::AuthorityMismatch,
        Self::CheckpointChanged,
        Self::RetainedRowCorrupt,
        Self::CapacityExceeded,
        Self::RevisionOverflow,
        Self::SerializationExhausted,
        Self::TransactionFailed,
        Self::Unavailable,
        Self::CommitOutcomeUnknown,
    ];

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Malformed => "POSTGRES_PROJECT_REGISTRY_MALFORMED",
            Self::CommandSubstitution => "POSTGRES_PROJECT_REGISTRY_COMMAND_SUBSTITUTED",
            Self::AdmissionDenied => "POSTGRES_PROJECT_REGISTRY_ADMISSION_DENIED",
            Self::AuthorityMismatch => "POSTGRES_PROJECT_REGISTRY_AUTHORITY_MISMATCH",
            Self::CheckpointChanged => "POSTGRES_PROJECT_REGISTRY_CHECKPOINT_CHANGED",
            Self::RetainedRowCorrupt => "POSTGRES_PROJECT_REGISTRY_RETAINED_ROW_CORRUPT",
            Self::CapacityExceeded => "POSTGRES_PROJECT_REGISTRY_CAPACITY_EXCEEDED",
            Self::RevisionOverflow => "POSTGRES_PROJECT_REGISTRY_REVISION_OVERFLOW",
            Self::SerializationExhausted => "POSTGRES_PROJECT_REGISTRY_SERIALIZATION_EXHAUSTED",
            Self::TransactionFailed => "POSTGRES_PROJECT_REGISTRY_TRANSACTION_FAILED",
            Self::Unavailable => "POSTGRES_PROJECT_REGISTRY_UNAVAILABLE",
            Self::CommitOutcomeUnknown => "POSTGRES_PROJECT_REGISTRY_COMMIT_OUTCOME_UNKNOWN",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostgresProjectRegistryError {
    kind: PostgresProjectRegistryErrorKind,
}

impl PostgresProjectRegistryError {
    #[must_use]
    pub const fn new(kind: PostgresProjectRegistryErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> PostgresProjectRegistryErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }
}

impl fmt::Display for PostgresProjectRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for PostgresProjectRegistryError {}

/// Exact durable database/global-profile identity used by Registry receipts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresProjectRegistryPersistenceEvidence {
    database_identity_digest: ContentDigest,
    schema_version: u16,
    manifest_digest: ContentDigest,
}

impl PostgresProjectRegistryPersistenceEvidence {
    #[must_use]
    pub const fn database_identity_digest(&self) -> &ContentDigest {
        &self.database_identity_digest
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ContentDigest {
        &self.manifest_digest
    }
}

/// Immutable Registry-specific persistence receipt. This is not project authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresProjectRegistryPersistenceReceipt {
    command_id: CommandId,
    project_id: ProjectId,
    request_digest: ContentDigest,
    result_digest: ContentDigest,
    record_set_digest: ContentDigest,
    base_checkpoint: RegistryCheckpoint,
    result_checkpoint: RegistryCheckpoint,
    daemon_authority: StoreAuthorityHead,
    persistence: PostgresProjectRegistryPersistenceEvidence,
    transaction_digest: ContentDigest,
    receipt_digest: ContentDigest,
}

impl PostgresProjectRegistryPersistenceReceipt {
    #[must_use]
    pub const fn command_id(&self) -> &CommandId {
        &self.command_id
    }
    #[must_use]
    pub const fn project_id(&self) -> &ProjectId {
        &self.project_id
    }
    #[must_use]
    pub const fn request_digest(&self) -> &ContentDigest {
        &self.request_digest
    }
    #[must_use]
    pub const fn result_digest(&self) -> &ContentDigest {
        &self.result_digest
    }
    #[must_use]
    pub const fn record_set_digest(&self) -> &ContentDigest {
        &self.record_set_digest
    }
    #[must_use]
    pub const fn base_checkpoint(&self) -> &RegistryCheckpoint {
        &self.base_checkpoint
    }
    #[must_use]
    pub const fn result_checkpoint(&self) -> &RegistryCheckpoint {
        &self.result_checkpoint
    }
    #[must_use]
    pub const fn daemon_authority(&self) -> &StoreAuthorityHead {
        &self.daemon_authority
    }
    #[must_use]
    pub const fn persistence(&self) -> &PostgresProjectRegistryPersistenceEvidence {
        &self.persistence
    }
    #[must_use]
    pub const fn transaction_digest(&self) -> &ContentDigest {
        &self.transaction_digest
    }
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }
}

/// One fully verified current Registry load.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresProjectRegistryLoad {
    state: VerifiedRegistryState,
    retained_checkpoint: RegistryCheckpoint,
    persistence: PostgresProjectRegistryPersistenceEvidence,
}

impl PostgresProjectRegistryLoad {
    #[must_use]
    pub const fn state(&self) -> &VerifiedRegistryState {
        &self.state
    }
    #[must_use]
    pub const fn retained_checkpoint(&self) -> &RegistryCheckpoint {
        &self.retained_checkpoint
    }
    #[must_use]
    pub const fn persistence(&self) -> &PostgresProjectRegistryPersistenceEvidence {
        &self.persistence
    }
}

/// Durable result returned only after a known successful commit or exact replay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresProjectRegistryExecution {
    semantic_receipt: lattice_project_registry::RegistryCommandReceipt,
    result_checkpoint: RegistryCheckpoint,
    persistence_receipt: PostgresProjectRegistryPersistenceReceipt,
    exact_retry: bool,
}

impl PostgresProjectRegistryExecution {
    #[must_use]
    pub const fn semantic_receipt(&self) -> &lattice_project_registry::RegistryCommandReceipt {
        &self.semantic_receipt
    }
    #[must_use]
    pub const fn result_checkpoint(&self) -> &RegistryCheckpoint {
        &self.result_checkpoint
    }
    #[must_use]
    pub const fn persistence_receipt(&self) -> &PostgresProjectRegistryPersistenceReceipt {
        &self.persistence_receipt
    }
    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.exact_retry
    }
}

/// Synchronous live Project Registry adapter over one authenticated runtime client.
pub struct PostgresProjectRegistry {
    client: Client,
    persistence: PostgresProjectRegistryPersistenceEvidence,
    current_checkpoint: RegistryCheckpoint,
    commit_outcome_unknown: bool,
}

impl PostgresProjectRegistry {
    /// Verifies an exact supported runtime surface and current Registry state.
    ///
    /// # Errors
    ///
    /// Fails closed unless the supplied runtime client, disposable target,
    /// schema profile, catalog, and retained Registry state verify exactly.
    pub fn new(
        mut client: Client,
        target: &MigrationTarget,
    ) -> PostgresProjectRegistryResult<Self> {
        let evidence = verify_runtime_store_schema(&mut client, target).map_err(map_setup_error)?;
        let sql_profile =
            RegistrySqlProfile::from_schema_version(evidence.global_schema_version())?;
        let persistence = PostgresProjectRegistryPersistenceEvidence {
            database_identity_digest: digest(target.expected_database_identity_sha256().as_str())?,
            schema_version: sql_profile.schema_version(),
            manifest_digest: digest(evidence.global_manifest_sha256().as_str())?,
        };
        let mut adapter = Self {
            client,
            persistence,
            current_checkpoint: VerifiedRegistryState::vacant(RuntimeKind::Live)
                .map_err(map_registry_error)?
                .checkpoint()
                .clone(),
            commit_outcome_unknown: false,
        };
        let loaded = adapter.load_internal()?;
        adapter.current_checkpoint = loaded.retained_checkpoint.clone();
        Ok(adapter)
    }

    /// Loads and verifies the complete global Registry through fixed functions.
    ///
    /// # Errors
    ///
    /// Returns a typed unavailable, profile, row, or checkpoint failure without
    /// exposing a raw database diagnostic.
    pub fn load(&mut self) -> PostgresProjectRegistryResult<PostgresProjectRegistryLoad> {
        self.ensure_reconcilable()?;
        let loaded = self.load_internal()?;
        self.current_checkpoint = loaded.retained_checkpoint.clone();
        Ok(PostgresProjectRegistryLoad {
            state: loaded.state,
            retained_checkpoint: loaded.retained_checkpoint,
            persistence: self.persistence.clone(),
        })
    }

    /// Plans and durably commits one Registry-owned global command.
    ///
    /// # Errors
    ///
    /// Returns a closed failure for malformed work, changed command reuse,
    /// rejected authority, concurrency exhaustion, retained corruption, or an
    /// unknown commit outcome.
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute(
        &mut self,
        command: RegistryCommand,
        expected_authority: StoreAuthorityHead,
    ) -> PostgresProjectRegistryResult<PostgresProjectRegistryExecution> {
        self.ensure_reconcilable()?;
        let request_digest = command.request_digest().map_err(map_registry_error)?;
        for retry_count in 0..=MAX_LIVE_SERIALIZATION_RETRIES {
            match run_execute_attempt(
                &mut self.client,
                &self.persistence,
                &self.current_checkpoint,
                command.clone(),
                request_digest.clone(),
                expected_authority.clone(),
            ) {
                Ok((execution, checkpoint)) => {
                    self.current_checkpoint = checkpoint;
                    return Ok(execution);
                }
                Err(AttemptFailure::Retryable) if retry_count < MAX_LIVE_SERIALIZATION_RETRIES => {
                    let refreshed = self.load_internal()?;
                    self.current_checkpoint = refreshed.retained_checkpoint;
                }
                Err(AttemptFailure::Retryable) => {
                    return Err(error(
                        PostgresProjectRegistryErrorKind::SerializationExhausted,
                    ));
                }
                Err(AttemptFailure::CommitOutcomeUnknown) => {
                    self.commit_outcome_unknown = true;
                    return Err(error(
                        PostgresProjectRegistryErrorKind::CommitOutcomeUnknown,
                    ));
                }
                Err(AttemptFailure::Terminal(failure)) => return Err(failure),
            }
        }
        Err(error(PostgresProjectRegistryErrorKind::TransactionFailed))
    }

    fn load_internal(&mut self) -> PostgresProjectRegistryResult<LoadedRegistry> {
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|database| map_database_error(&database))?;
        if let Err(database) = transaction.batch_execute(READ_TRANSACTION_SETTINGS) {
            return rollback_load(transaction, map_database_error(&database));
        }
        let loaded = match load_verified_registry(&mut transaction, &self.persistence, None) {
            Ok(loaded) => loaded,
            Err(load_error) => return rollback_load(transaction, load_error),
        };
        transaction
            .commit()
            .map_err(|database| map_database_error(&database))?;
        Ok(loaded)
    }

    fn ensure_reconcilable(&self) -> PostgresProjectRegistryResult<()> {
        if self.commit_outcome_unknown {
            Err(error(
                PostgresProjectRegistryErrorKind::CommitOutcomeUnknown,
            ))
        } else {
            Ok(())
        }
    }
}

struct LoadedRegistry {
    state: VerifiedRegistryState,
    retained_checkpoint: RegistryCheckpoint,
    durable_receipts: BTreeMap<String, PostgresProjectRegistryPersistenceReceipt>,
}

struct PrepareRow {
    status: String,
    retained_request: Option<ContentDigest>,
    retained_result: Option<ContentDigest>,
    retained_record_set: Option<ContentDigest>,
    retained_persistence_receipt: Option<ContentDigest>,
    retained_base_checkpoint: Option<ContentDigest>,
    retained_result_checkpoint: Option<ContentDigest>,
    current_checkpoint: RegistryCheckpoint,
}

struct StoredCommand {
    ordinal: u64,
    command: RegistryCommand,
    request_digest: ContentDigest,
    outcome: String,
    denial_reason: Option<String>,
    denial_dimension: Option<String>,
    denial_existing_project_id: Option<String>,
    denial_lifecycle: Option<String>,
    denial_expected_decision: Option<String>,
    denial_found_decision: Option<String>,
    semantic_before_receipt_digest: Option<ContentDigest>,
    semantic_after_receipt_digest: Option<ContentDigest>,
    authority_receipt_digest: Option<ContentDigest>,
    drift: [bool; 5],
    result_digest: ContentDigest,
    base_checkpoint: RegistryCheckpoint,
    result_checkpoint: RegistryCheckpoint,
    record_set_digest: ContentDigest,
    daemon_authority: StoreAuthorityHead,
    persistence: PostgresProjectRegistryPersistenceEvidence,
    transaction_digest: ContentDigest,
    persistence_receipt_digest: ContentDigest,
}

enum AttemptFailure {
    Retryable,
    CommitOutcomeUnknown,
    Terminal(PostgresProjectRegistryError),
}

#[derive(Clone, Copy)]
enum CommitFailureClass {
    Retryable,
    OutcomeUnknown,
    Terminal,
}

#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
fn run_execute_attempt(
    client: &mut Client,
    persistence: &PostgresProjectRegistryPersistenceEvidence,
    expected_checkpoint: &RegistryCheckpoint,
    command: RegistryCommand,
    request_digest: ContentDigest,
    expected_authority: StoreAuthorityHead,
) -> Result<(PostgresProjectRegistryExecution, RegistryCheckpoint), AttemptFailure> {
    let started = Instant::now();
    let profile_version = profile_version(persistence).map_err(AttemptFailure::Terminal)?;
    let profile_manifest = persistence.manifest_digest().as_str().to_owned();
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|database| classify_query_error(&database))?;
    if let Err(database) = transaction.batch_execute(WRITE_TRANSACTION_SETTINGS) {
        return rollback_attempt(transaction, classify_query_error(&database));
    }
    if let Err(deadline) = ensure_deadline(&started) {
        return rollback_attempt(transaction, AttemptFailure::Terminal(deadline));
    }
    let prepare_values: Vec<Box<dyn ToSql + Sync>> = vec![
        Box::new(profile_version),
        Box::new(profile_manifest.clone()),
        Box::new(command.command_id().as_str().to_owned()),
        Box::new(digest_bytes(&request_digest).map_err(AttemptFailure::Terminal)?),
        Box::new(runtime_text(expected_authority.runtime()).to_owned()),
        Box::new(expected_authority.daemon_instance_id().as_str().to_owned()),
        Box::new(
            signed_i64(expected_authority.daemon_epoch().get())
                .map_err(AttemptFailure::Terminal)?,
        ),
        Box::new(expected_authority.admission().as_str().to_owned()),
        Box::new(
            signed_i64(expected_authority.revision().get()).map_err(AttemptFailure::Terminal)?,
        ),
        Box::new(
            digest_bytes(expected_authority.observation_digest())
                .map_err(AttemptFailure::Terminal)?,
        ),
        Box::new(digest_bytes(expected_authority.head_digest()).map_err(AttemptFailure::Terminal)?),
        Box::new(
            digest_bytes(expected_checkpoint.checkpoint_digest())
                .map_err(AttemptFailure::Terminal)?,
        ),
    ];
    let prepare_row = match query_one_boxed(&mut transaction, PREPARE_SQL, &prepare_values) {
        Ok(row) => row,
        Err(database) => return rollback_attempt(transaction, classify_query_error(&database)),
    };
    let prepare = match parse_prepare_row(&prepare_row) {
        Ok(prepare) => prepare,
        Err(failure) => return rollback_attempt(transaction, AttemptFailure::Terminal(failure)),
    };
    if prepare.status == "COMMAND_CONFLICT" {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::CommandSubstitution)),
        );
    }
    let loaded = match load_verified_registry(&mut transaction, persistence, Some(&started)) {
        Ok(loaded) => loaded,
        Err(failure) => return rollback_attempt(transaction, AttemptFailure::Terminal(failure)),
    };
    if prepare.current_checkpoint != loaded.retained_checkpoint {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
        );
    }
    let plan = match plan_command(&loaded.state, command.clone()) {
        Ok(plan) => plan,
        Err(registry) => {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(map_registry_error(registry)),
            );
        }
    };
    if plan.is_replay() {
        if prepare.status != "REPLAY" {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(
                    PostgresProjectRegistryErrorKind::RetainedRowCorrupt,
                )),
            );
        }
        let durable = match loaded.durable_receipts.get(command.command_id().as_str()) {
            Some(receipt) => receipt.clone(),
            None => {
                return rollback_attempt(
                    transaction,
                    AttemptFailure::Terminal(error(
                        PostgresProjectRegistryErrorKind::RetainedRowCorrupt,
                    )),
                );
            }
        };
        if !prepare_matches_replay(
            &prepare,
            plan.receipt(),
            plan.record_set().record_set_digest(),
            &durable,
        ) {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(
                    PostgresProjectRegistryErrorKind::RetainedRowCorrupt,
                )),
            );
        }
        let execution = PostgresProjectRegistryExecution {
            semantic_receipt: plan.receipt().clone(),
            result_checkpoint: durable.result_checkpoint().clone(),
            persistence_receipt: durable,
            exact_retry: true,
        };
        let checkpoint = loaded.retained_checkpoint;
        return transaction
            .rollback()
            .map(|()| (execution, checkpoint))
            .map_err(|_| {
                AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::TransactionFailed))
            });
    }
    if prepare.status != "NEW" || plan.base_checkpoint() != &loaded.retained_checkpoint {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
        );
    }
    let applied = match apply_command_plan(&loaded.state, &plan) {
        Ok(applied) => applied,
        Err(registry) => {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(map_registry_error(registry)),
            );
        }
    };
    let durable = match build_persistence_receipt(
        &command,
        plan.receipt(),
        plan.record_set().record_set_digest(),
        plan.base_checkpoint(),
        plan.result_checkpoint(),
        expected_authority,
        persistence,
    ) {
        Ok(receipt) => receipt,
        Err(failure) => return rollback_attempt(transaction, AttemptFailure::Terminal(failure)),
    };
    if let Err(deadline) = ensure_deadline(&started) {
        return rollback_attempt(transaction, AttemptFailure::Terminal(deadline));
    }
    let command_values =
        match stage_command_values(profile_version, &profile_manifest, &plan, &durable) {
            Ok(values) => values,
            Err(failure) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(failure));
            }
        };
    let status = match query_one_boxed(&mut transaction, STAGE_COMMAND_SQL, &command_values)
        .and_then(|row| row.try_get::<usize, String>(0))
    {
        Ok(status) => status,
        Err(database) => return rollback_attempt(transaction, classify_query_error(&database)),
    };
    if status != "STAGED" {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
        );
    }
    let stage_project = plan.record_set().project_replacement().is_some();
    let mut deleted_reservations = 0_u64;
    let mut inserted_reservations = 0_u64;
    if let Some((project_id, projection)) = plan.record_set().project_replacement() {
        deleted_reservations = u64::try_from(
            loaded
                .state
                .reservations()
                .iter()
                .filter(|row| row.project_id() == project_id)
                .count(),
        )
        .map_err(|_| {
            AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
        })?;
        inserted_reservations = u64::try_from(
            applied
                .state()
                .reservations()
                .iter()
                .filter(|row| row.project_id() == project_id)
                .count(),
        )
        .map_err(|_| {
            AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
        })?;
        let project_values = match stage_project_values(
            profile_version,
            &profile_manifest,
            project_id,
            projection,
        ) {
            Ok(values) => values,
            Err(failure) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(failure));
            }
        };
        let project_status =
            match query_one_boxed(&mut transaction, STAGE_PROJECT_SQL, &project_values)
                .and_then(|row| row.try_get::<usize, String>(0))
            {
                Ok(status) => status,
                Err(database) => {
                    return rollback_attempt(transaction, classify_query_error(&database));
                }
            };
        if project_status != "STAGED" {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(
                    PostgresProjectRegistryErrorKind::RetainedRowCorrupt,
                )),
            );
        }
    }
    if let Err(deadline) = ensure_deadline(&started) {
        return rollback_attempt(transaction, AttemptFailure::Terminal(deadline));
    }
    let finalize_values = match finalize_values(
        profile_version,
        &profile_manifest,
        &plan,
        &durable,
        stage_project,
        deleted_reservations,
        inserted_reservations,
    ) {
        Ok(values) => values,
        Err(failure) => return rollback_attempt(transaction, AttemptFailure::Terminal(failure)),
    };
    let final_status = match query_one_boxed(&mut transaction, FINALIZE_SQL, &finalize_values)
        .and_then(|row| row.try_get::<usize, String>(0))
    {
        Ok(status) => status,
        Err(database) => return rollback_attempt(transaction, classify_query_error(&database)),
    };
    if final_status != "FINALIZED" {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
        );
    }
    let reloaded = match load_verified_registry(&mut transaction, persistence, Some(&started)) {
        Ok(loaded) => loaded,
        Err(failure) => return rollback_attempt(transaction, AttemptFailure::Terminal(failure)),
    };
    if reloaded.state != *applied.state()
        || reloaded.retained_checkpoint != *plan.result_checkpoint()
        || reloaded.durable_receipts.get(command.command_id().as_str()) != Some(&durable)
    {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
        );
    }
    if let Err(deadline) = ensure_deadline(&started) {
        return rollback_attempt(transaction, AttemptFailure::Terminal(deadline));
    }
    let execution = PostgresProjectRegistryExecution {
        semantic_receipt: plan.receipt().clone(),
        result_checkpoint: plan.result_checkpoint().clone(),
        persistence_receipt: durable,
        exact_retry: false,
    };
    let checkpoint = plan.result_checkpoint().clone();
    transaction
        .commit()
        .map(|()| (execution, checkpoint))
        .map_err(|database| classify_commit_error(&database))
}

fn load_verified_registry<C: GenericClient>(
    client: &mut C,
    persistence: &PostgresProjectRegistryPersistenceEvidence,
    started: Option<&Instant>,
) -> PostgresProjectRegistryResult<LoadedRegistry> {
    let version = profile_version(persistence)?;
    let manifest = persistence.manifest_digest().as_str().to_owned();
    let params: [&(dyn ToSql + Sync); 2] = [&version, &manifest];
    let state_row = client
        .query_one(READ_STATE_SQL, &params)
        .map_err(|db| map_database_error(&db))?;
    let retained_checkpoint = parse_state_row(&state_row)?;
    check_optional_deadline(started)?;
    let observation_rows = client
        .query(READ_OBSERVATIONS_SQL, &params)
        .map_err(|db| map_database_error(&db))?;
    let observations = parse_observations(&observation_rows)?;
    check_optional_deadline(started)?;
    let project_rows = client
        .query(READ_PROJECTS_SQL, &params)
        .map_err(|db| map_database_error(&db))?;
    let projects = parse_projects(&project_rows, &observations)?;
    check_optional_deadline(started)?;
    let command_rows = client
        .query(READ_COMMANDS_SQL, &params)
        .map_err(|db| map_database_error(&db))?;
    let stored_commands = parse_commands(&command_rows, &observations, persistence)?;
    check_optional_deadline(started)?;
    let reservation_rows = client
        .query(READ_RESERVATIONS_SQL, &params)
        .map_err(|db| map_database_error(&db))?;
    let reservations = parse_reservations(&reservation_rows)?;
    check_optional_deadline(started)?;

    let mut replayed =
        VerifiedRegistryState::vacant(RuntimeKind::Live).map_err(map_registry_error)?;
    let mut commands = Vec::with_capacity(stored_commands.len());
    let mut durable_receipts = BTreeMap::new();
    for stored in stored_commands {
        let plan =
            plan_command(&replayed, stored.command.clone()).map_err(map_retained_registry_error)?;
        if plan.is_replay() || !stored_matches_plan(&stored, &plan) {
            return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
        }
        let durable = build_persistence_receipt(
            &stored.command,
            plan.receipt(),
            plan.record_set().record_set_digest(),
            plan.base_checkpoint(),
            plan.result_checkpoint(),
            stored.daemon_authority.clone(),
            &stored.persistence,
        )?;
        if durable.transaction_digest != stored.transaction_digest
            || durable.receipt_digest != stored.persistence_receipt_digest
        {
            return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
        }
        let record = RegistryCommandRecord::from_retained(
            stored.ordinal,
            stored.command.clone(),
            plan.receipt().clone(),
            plan.base_checkpoint().clone(),
            plan.result_checkpoint().clone(),
            plan.record_set().record_set_digest().clone(),
        );
        durable_receipts.insert(stored.command.command_id().as_str().to_owned(), durable);
        commands.push(record);
        replayed = apply_command_plan(&replayed, &plan)
            .map_err(map_retained_registry_error)?
            .state()
            .clone();
    }
    let snapshot = UntrustedRegistrySnapshot::from_retained(
        retained_checkpoint.clone(),
        observations.values().cloned().collect(),
        projects,
        commands,
        reservations,
    );
    let state =
        verify_untrusted_registry_snapshot_against_checkpoint(&snapshot, &retained_checkpoint)
            .map_err(map_retained_registry_error)?;
    Ok(LoadedRegistry {
        state,
        retained_checkpoint,
        durable_receipts,
    })
}

fn parse_state_row(row: &Row) -> PostgresProjectRegistryResult<RegistryCheckpoint> {
    if row.len() != 9 || row_value::<Option<String>>(row, 8)?.is_some() {
        return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
    }
    checkpoint_from_row(row, 0)
}

fn checkpoint_from_row(
    row: &Row,
    start: usize,
) -> PostgresProjectRegistryResult<RegistryCheckpoint> {
    Ok(RegistryCheckpoint::from_retained(
        parse_runtime(&row_value::<String>(row, start)?)?,
        nonnegative_i64(row_value(row, start + 1)?)?,
        nonnegative_i64(row_value(row, start + 2)?)?,
        nonnegative_i64(row_value(row, start + 3)?)?,
        nonnegative_i64(row_value(row, start + 4)?)?,
        nonnegative_i64(row_value(row, start + 5)?)?,
        nonnegative_i64(row_value(row, start + 6)?)?,
        row_digest(row, start + 7)?,
    ))
}

fn parse_observations(
    rows: &[Row],
) -> PostgresProjectRegistryResult<BTreeMap<String, RepositoryObservation>> {
    let mut observations = BTreeMap::new();
    for row in rows {
        if row.len() != 7 {
            return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
        }
        let retained_digest = row_digest(row, 0)?;
        let observation = RepositoryObservation::new(
            row_value::<String>(row, 1)?,
            row_digest(row, 2)?,
            row_digest(row, 3)?,
            row_digest(row, 4)?,
            GitRefIdentity::new(row_value::<String>(row, 5)?, row_digest(row, 6)?)
                .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
        )
        .map_err(map_retained_registry_error)?;
        if observation.digest() != &retained_digest
            || observations
                .insert(retained_digest.as_str().to_owned(), observation)
                .is_some()
        {
            return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
        }
    }
    Ok(observations)
}

fn parse_projects(
    rows: &[Row],
    observations: &BTreeMap<String, RepositoryObservation>,
) -> PostgresProjectRegistryResult<Vec<RegistryProjectRow>> {
    let mut projects = Vec::with_capacity(rows.len());
    for row in rows {
        if row.len() != 20 {
            return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
        }
        let project_id = ProjectId::new(row_value::<String>(row, 0)?)
            .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
        let project_class = parse_project_class(&row_value::<String>(row, 1)?)?;
        let accepted_digest = row_digest(row, 2)?;
        let accepted = observations
            .get(accepted_digest.as_str())
            .cloned()
            .ok_or_else(|| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
        let pending = optional_row_digest(row, 3)?
            .map(|digest| {
                observations
                    .get(digest.as_str())
                    .cloned()
                    .ok_or_else(|| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
            })
            .transpose()?;
        let drift = drift_from_flags([
            row_value(row, 4)?,
            row_value(row, 5)?,
            row_value(row, 6)?,
            row_value(row, 7)?,
            row_value(row, 8)?,
        ]);
        let authority = ProjectAuthorityReceipt::new(
            positive_i16(row_value(row, 9)?)?,
            row_value::<String>(row, 10)?,
            row_value::<String>(row, 11)?,
            parse_runtime(&row_value::<String>(row, 12)?)?,
            project_id.clone(),
            ProjectSnapshotId::new(row_value::<String>(row, 13)?)
                .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
            parse_u64_text(&row_value::<String>(row, 14)?)?,
            parse_project_lifecycle(&row_value::<String>(row, 15)?)?,
            project_class,
            GitRefIdentity::new(row_value::<String>(row, 16)?, row_digest(row, 17)?)
                .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
            row_digest(row, 18)?,
            row_digest(row, 19)?,
        )
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
        projects.push(RegistryProjectRow::from_retained(
            project_id,
            RegistryProjectProjection::from_retained(
                project_class,
                accepted,
                pending,
                drift,
                authority,
            ),
        ));
    }
    Ok(projects)
}

fn parse_commands(
    rows: &[Row],
    observations: &BTreeMap<String, RepositoryObservation>,
    current_persistence: &PostgresProjectRegistryPersistenceEvidence,
) -> PostgresProjectRegistryResult<Vec<StoredCommand>> {
    rows.iter()
        .map(|row| parse_command(row, observations, current_persistence))
        .collect()
}

fn parse_command(
    row: &Row,
    observations: &BTreeMap<String, RepositoryObservation>,
    current_persistence: &PostgresProjectRegistryPersistenceEvidence,
) -> PostgresProjectRegistryResult<StoredCommand> {
    if row.len() != 66 {
        return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
    }
    let ordinal = positive_i64(row_value(row, 0)?)?;
    let command_id =
        CommandId::new(row_value::<String>(row, 1)?).map_err(map_retained_registry_error)?;
    let action: String = row_value(row, 2)?;
    let project_id = ProjectId::new(row_value::<String>(row, 3)?)
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
    let project_class = row_value::<Option<String>>(row, 4)?
        .map(|value| parse_project_class(&value))
        .transpose()?;
    let observation = optional_row_digest(row, 5)?
        .map(|digest| {
            observations
                .get(digest.as_str())
                .cloned()
                .ok_or_else(|| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
        })
        .transpose()?;
    let expected_head = parse_optional_head(row, 6)?;
    let decision = row_value::<Option<String>>(row, 19)?
        .map(|value| parse_decision(&value))
        .transpose()?;
    let evidence = optional_row_digest(row, 20)?;
    let command = match action.as_str() {
        "REGISTER" => RegistryCommand::register(
            command_id,
            project_id,
            required(project_class)?,
            required(observation)?,
        ),
        "OBSERVE" => RegistryCommand::observe(
            command_id,
            project_id,
            required(expected_head)?,
            required(observation)?,
        ),
        "SUSPEND" => RegistryCommand::suspend(
            command_id,
            project_id,
            required(expected_head)?,
            required(evidence)?,
        ),
        "RECONCILE" => RegistryCommand::reconcile(
            command_id,
            project_id,
            required(expected_head)?,
            required(observation)?,
            required(decision)?,
            required(evidence)?,
        ),
        _ => return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
    };
    let daemon_authority = StoreAuthorityHead::new(
        parse_runtime(&row_value::<String>(row, 55)?)?,
        StoreDaemonInstanceId::new(row_value::<String>(row, 56)?)
            .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
        DaemonEpoch::new(positive_i64(row_value(row, 57)?)?)
            .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
        parse_admission(&row_value::<String>(row, 58)?)?,
        StoreAuthorityRevision::new(positive_i64(row_value(row, 59)?)?)
            .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
        row_digest(row, 60)?,
        row_digest(row, 61)?,
    )
    .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
    let persistence = retained_command_persistence(row, current_persistence)?;
    Ok(StoredCommand {
        ordinal,
        command,
        request_digest: row_digest(row, 21)?,
        outcome: row_value(row, 22)?,
        denial_reason: row_value(row, 23)?,
        denial_dimension: row_value(row, 24)?,
        denial_existing_project_id: row_value(row, 25)?,
        denial_lifecycle: row_value(row, 26)?,
        denial_expected_decision: row_value(row, 27)?,
        denial_found_decision: row_value(row, 28)?,
        semantic_before_receipt_digest: optional_row_digest(row, 29)?,
        semantic_after_receipt_digest: optional_row_digest(row, 30)?,
        authority_receipt_digest: optional_row_digest(row, 31)?,
        drift: [
            row_value(row, 32)?,
            row_value(row, 33)?,
            row_value(row, 34)?,
            row_value(row, 35)?,
            row_value(row, 36)?,
        ],
        result_digest: row_digest(row, 37)?,
        base_checkpoint: checkpoint_from_row(row, 38)?,
        result_checkpoint: checkpoint_from_row(row, 46)?,
        record_set_digest: row_digest(row, 54)?,
        daemon_authority,
        persistence,
        transaction_digest: row_digest(row, 62)?,
        persistence_receipt_digest: row_digest(row, 63)?,
    })
}

fn parse_optional_head(
    row: &Row,
    start: usize,
) -> PostgresProjectRegistryResult<Option<ProjectAuthorityHead>> {
    let present: bool = row_value(row, start)?;
    if !present {
        for index in (start + 1)..=(start + 12) {
            if !row_is_null(row, index)? {
                return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
            }
        }
        return Ok(None);
    }
    let project_id = ProjectId::new(required(row_value::<Option<String>>(row, start + 4)?)?)
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
    let project_class =
        parse_project_class(&required(row_value::<Option<String>>(row, start + 8)?)?)?;
    let receipt = ProjectAuthorityReceipt::new(
        1,
        required(row_value::<Option<String>>(row, start + 1)?)?,
        required(row_value::<Option<String>>(row, start + 2)?)?,
        parse_runtime(&required(row_value::<Option<String>>(row, start + 3)?)?)?,
        project_id,
        ProjectSnapshotId::new(required(row_value::<Option<String>>(row, start + 5)?)?)
            .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
        parse_u64_text(&required(row_value::<Option<String>>(row, start + 6)?)?)?,
        parse_project_lifecycle(&required(row_value::<Option<String>>(row, start + 7)?)?)?,
        project_class,
        GitRefIdentity::new(
            required(row_value::<Option<String>>(row, start + 9)?)?,
            required(optional_row_digest(row, start + 10)?)?,
        )
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
        required(optional_row_digest(row, start + 11)?)?,
        required(optional_row_digest(row, start + 12)?)?,
    )
    .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
    Ok(Some(receipt.head()))
}

fn parse_reservations(
    rows: &[Row],
) -> PostgresProjectRegistryResult<Vec<RegistryIdentityReservation>> {
    let mut reservations = rows
        .iter()
        .map(|row| {
            if row.len() != 4 {
                return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
            }
            Ok(RegistryIdentityReservation::from_retained(
                parse_dimension(&row_value::<String>(row, 0)?)?,
                row_digest(row, 1)?,
                parse_reservation_status(&row_value::<String>(row, 2)?)?,
                ProjectId::new(row_value::<String>(row, 3)?)
                    .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
            ))
        })
        .collect::<PostgresProjectRegistryResult<Vec<_>>>()?;
    reservations.sort();
    Ok(reservations)
}

fn parse_prepare_row(row: &Row) -> PostgresProjectRegistryResult<PrepareRow> {
    if row.len() != 14 {
        return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
    }
    Ok(PrepareRow {
        status: row_value(row, 0)?,
        retained_request: optional_row_digest(row, 1)?,
        retained_result: optional_row_digest(row, 2)?,
        retained_record_set: optional_row_digest(row, 3)?,
        retained_persistence_receipt: optional_row_digest(row, 4)?,
        retained_base_checkpoint: optional_row_digest(row, 5)?,
        retained_result_checkpoint: optional_row_digest(row, 6)?,
        current_checkpoint: RegistryCheckpoint::from_retained(
            RuntimeKind::Live,
            nonnegative_i64(row_value(row, 7)?)?,
            nonnegative_i64(row_value(row, 8)?)?,
            nonnegative_i64(row_value(row, 9)?)?,
            nonnegative_i64(row_value(row, 10)?)?,
            nonnegative_i64(row_value(row, 11)?)?,
            nonnegative_i64(row_value(row, 12)?)?,
            row_digest(row, 13)?,
        ),
    })
}

fn stored_matches_plan(
    stored: &StoredCommand,
    plan: &lattice_project_registry::RegistryCommandPlan,
) -> bool {
    let receipt = plan.receipt();
    let denial = denial_projection(&receipt.outcome());
    stored.ordinal == plan.record_set().ordinal()
        && stored.request_digest == *receipt.request_digest()
        && stored.outcome == denial.outcome
        && stored.denial_reason == denial.reason
        && stored.denial_dimension == denial.dimension
        && stored.denial_existing_project_id == denial.existing_project_id
        && stored.denial_lifecycle == denial.lifecycle
        && stored.denial_expected_decision == denial.expected_decision
        && stored.denial_found_decision == denial.found_decision
        && stored.semantic_before_receipt_digest.as_ref()
            == receipt.before().map(ProjectAuthorityHead::receipt_digest)
        && stored.semantic_after_receipt_digest.as_ref()
            == receipt.after().map(ProjectAuthorityHead::receipt_digest)
        && stored.authority_receipt_digest.as_ref()
            == receipt
                .authority()
                .map(ProjectAuthorityReceipt::receipt_digest)
        && stored.drift == drift_flags(receipt.drift())
        && stored.result_digest == *receipt.result_digest()
        && stored.base_checkpoint == *plan.base_checkpoint()
        && stored.result_checkpoint == *plan.result_checkpoint()
        && stored.record_set_digest == *plan.record_set().record_set_digest()
}

struct DenialProjection {
    outcome: String,
    reason: Option<String>,
    dimension: Option<String>,
    existing_project_id: Option<String>,
    lifecycle: Option<String>,
    expected_decision: Option<String>,
    found_decision: Option<String>,
}

fn denial_projection(outcome: &RegistryCommandOutcome) -> DenialProjection {
    let mut projection = DenialProjection {
        outcome: "APPLIED".to_owned(),
        reason: None,
        dimension: None,
        existing_project_id: None,
        lifecycle: None,
        expected_decision: None,
        found_decision: None,
    };
    let denial = match outcome {
        RegistryCommandOutcome::Applied => return projection,
        RegistryCommandOutcome::Denied(denial) => {
            "DENIED".clone_into(&mut projection.outcome);
            denial
        }
        RegistryCommandOutcome::Blocked(denial) => {
            "BLOCKED".clone_into(&mut projection.outcome);
            denial
        }
    };
    match denial {
        RegistryDenial::DuplicateIdentity {
            dimension,
            existing_project_id,
        } => {
            projection.reason = Some("DUPLICATE_IDENTITY".to_owned());
            projection.dimension = Some(dimension.as_str().to_owned());
            projection.existing_project_id = Some(existing_project_id.as_str().to_owned());
        }
        RegistryDenial::UnknownProject => projection.reason = Some("UNKNOWN_PROJECT".to_owned()),
        RegistryDenial::StaleHead => projection.reason = Some("STALE_HEAD".to_owned()),
        RegistryDenial::LifecycleBlocked { lifecycle } => {
            projection.reason = Some("LIFECYCLE_BLOCKED".to_owned());
            projection.lifecycle = Some(lifecycle.as_str().to_owned());
        }
        RegistryDenial::ReconciliationDecisionMismatch { expected, found } => {
            projection.reason = Some("RECONCILIATION_DECISION_MISMATCH".to_owned());
            projection.expected_decision = Some(expected.as_str().to_owned());
            projection.found_decision = Some(found.as_str().to_owned());
        }
        RegistryDenial::PendingObservationMismatch => {
            projection.reason = Some("PENDING_OBSERVATION_MISMATCH".to_owned());
        }
        RegistryDenial::RevisionOverflow => {
            projection.reason = Some("REVISION_OVERFLOW".to_owned());
        }
    }
    projection
}

fn prepare_matches_replay(
    prepare: &PrepareRow,
    semantic: &lattice_project_registry::RegistryCommandReceipt,
    record_set: &ContentDigest,
    durable: &PostgresProjectRegistryPersistenceReceipt,
) -> bool {
    prepare.retained_request.as_ref() == Some(semantic.request_digest())
        && prepare.retained_result.as_ref() == Some(semantic.result_digest())
        && prepare.retained_record_set.as_ref() == Some(record_set)
        && prepare.retained_persistence_receipt.as_ref() == Some(durable.receipt_digest())
        && prepare.retained_base_checkpoint.as_ref()
            == Some(durable.base_checkpoint().checkpoint_digest())
        && prepare.retained_result_checkpoint.as_ref()
            == Some(durable.result_checkpoint().checkpoint_digest())
}

fn build_persistence_receipt(
    command: &RegistryCommand,
    semantic: &lattice_project_registry::RegistryCommandReceipt,
    record_set_digest: &ContentDigest,
    base_checkpoint: &RegistryCheckpoint,
    result_checkpoint: &RegistryCheckpoint,
    daemon_authority: StoreAuthorityHead,
    persistence: &PostgresProjectRegistryPersistenceEvidence,
) -> PostgresProjectRegistryResult<PostgresProjectRegistryPersistenceReceipt> {
    let project_id = command_project_id(command).clone();
    let transaction_digest = registry_hash(
        "lattice.postgres-project-registry.transaction",
        &object(vec![
            ("command_id", string(command.command_id().as_str())),
            ("project_id", string(project_id.as_str())),
            ("request_digest", string(semantic.request_digest().as_str())),
            ("result_digest", string(semantic.result_digest().as_str())),
            ("record_set_digest", string(record_set_digest.as_str())),
            ("base_checkpoint", checkpoint_value(base_checkpoint)),
            ("result_checkpoint", checkpoint_value(result_checkpoint)),
            (
                "daemon_authority",
                daemon_authority_value(&daemon_authority),
            ),
        ]),
    )?;
    let receipt_digest = registry_hash(
        "lattice.postgres-project-registry.receipt",
        &object(vec![
            ("producer_id", string(REGISTRY_ADAPTER_PRODUCER_ID)),
            (
                "producer_version",
                string(REGISTRY_ADAPTER_PRODUCER_VERSION),
            ),
            ("runtime", string("LIVE")),
            ("durability", string("DURABLE_POSTGRES")),
            ("registry_catalog", string(REGISTRY_CATALOG_ID)),
            ("command_id", string(command.command_id().as_str())),
            ("project_id", string(project_id.as_str())),
            ("request_digest", string(semantic.request_digest().as_str())),
            ("result_digest", string(semantic.result_digest().as_str())),
            ("record_set_digest", string(record_set_digest.as_str())),
            ("base_checkpoint", checkpoint_value(base_checkpoint)),
            ("result_checkpoint", checkpoint_value(result_checkpoint)),
            (
                "daemon_authority",
                daemon_authority_value(&daemon_authority),
            ),
            (
                "database_identity_digest",
                string(persistence.database_identity_digest().as_str()),
            ),
            (
                "schema_version",
                string(persistence.schema_version().to_string()),
            ),
            (
                "manifest_digest",
                string(persistence.manifest_digest().as_str()),
            ),
            ("transaction_digest", string(transaction_digest.as_str())),
        ]),
    )?;
    Ok(PostgresProjectRegistryPersistenceReceipt {
        command_id: command.command_id().clone(),
        project_id,
        request_digest: semantic.request_digest().clone(),
        result_digest: semantic.result_digest().clone(),
        record_set_digest: record_set_digest.clone(),
        base_checkpoint: base_checkpoint.clone(),
        result_checkpoint: result_checkpoint.clone(),
        daemon_authority,
        persistence: persistence.clone(),
        transaction_digest,
        receipt_digest,
    })
}

#[allow(clippy::too_many_lines)]
fn stage_command_values(
    profile_version: i16,
    manifest: &str,
    plan: &lattice_project_registry::RegistryCommandPlan,
    durable: &PostgresProjectRegistryPersistenceReceipt,
) -> PostgresProjectRegistryResult<Vec<Box<dyn ToSql + Sync>>> {
    let command = plan.record_set().command();
    let (action, project_id, project_class, observation, expected_head, decision, evidence) =
        command_parts(command);
    let before = head_parts(expected_head)?;
    let denial = denial_projection(&plan.receipt().outcome());
    let base = checkpoint_parts(plan.base_checkpoint())?;
    let result = checkpoint_parts(plan.result_checkpoint())?;
    let daemon = durable.daemon_authority();
    let new_observation = plan.record_set().new_observation();
    let drift = drift_flags(plan.receipt().drift());
    Ok(vec![
        Box::new(profile_version),
        Box::new(manifest.to_owned()),
        Box::new(profile_version),
        Box::new(manifest.to_owned()),
        Box::new(base.ordinal + 1),
        Box::new(command.command_id().as_str().to_owned()),
        Box::new(action.to_owned()),
        Box::new(project_id.as_str().to_owned()),
        Box::new(project_class.map(|v| v.as_str().to_owned())),
        Box::new(optional_digest_bytes(
            observation.map(RepositoryObservation::digest),
        )?),
        Box::new(before.present),
        Box::new(before.producer_id),
        Box::new(before.producer_version),
        Box::new(before.runtime),
        Box::new(before.project_id),
        Box::new(before.snapshot_id),
        Box::new(before.registry_revision),
        Box::new(before.lifecycle),
        Box::new(before.project_class),
        Box::new(before.primary_ref),
        Box::new(before.primary_ref_storage),
        Box::new(before.observation_digest),
        Box::new(before.receipt_digest),
        Box::new(decision.map(|v| v.as_str().to_owned())),
        Box::new(optional_digest_bytes(evidence)?),
        Box::new(digest_bytes(plan.receipt().request_digest())?),
        Box::new(denial.outcome),
        Box::new(denial.reason),
        Box::new(denial.dimension),
        Box::new(denial.existing_project_id),
        Box::new(denial.lifecycle),
        Box::new(denial.expected_decision),
        Box::new(denial.found_decision),
        Box::new(optional_digest_bytes(
            plan.receipt()
                .before()
                .map(ProjectAuthorityHead::receipt_digest),
        )?),
        Box::new(optional_digest_bytes(
            plan.receipt()
                .after()
                .map(ProjectAuthorityHead::receipt_digest),
        )?),
        Box::new(optional_digest_bytes(
            plan.receipt()
                .authority()
                .map(ProjectAuthorityReceipt::receipt_digest),
        )?),
        Box::new(drift[0]),
        Box::new(drift[1]),
        Box::new(drift[2]),
        Box::new(drift[3]),
        Box::new(drift[4]),
        Box::new(digest_bytes(plan.receipt().result_digest())?),
        Box::new(base.runtime),
        Box::new(base.ordinal),
        Box::new(base.observations),
        Box::new(base.projects),
        Box::new(base.commands),
        Box::new(base.reservations),
        Box::new(base.retained_bytes),
        Box::new(base.digest),
        Box::new(result.runtime),
        Box::new(result.ordinal),
        Box::new(result.observations),
        Box::new(result.projects),
        Box::new(result.commands),
        Box::new(result.reservations),
        Box::new(result.retained_bytes),
        Box::new(result.digest),
        Box::new(digest_bytes(plan.record_set().record_set_digest())?),
        Box::new("LIVE".to_owned()),
        Box::new(daemon.daemon_instance_id().as_str().to_owned()),
        Box::new(signed_i64(daemon.daemon_epoch().get())?),
        Box::new(daemon.admission().as_str().to_owned()),
        Box::new(signed_i64(daemon.revision().get())?),
        Box::new(digest_bytes(daemon.observation_digest())?),
        Box::new(digest_bytes(daemon.head_digest())?),
        Box::new(digest_bytes(durable.transaction_digest())?),
        Box::new(digest_bytes(durable.receipt_digest())?),
        Box::new(new_observation.is_some()),
        Box::new(
            new_observation
                .map(RepositoryObservation::canonical_root)
                .map(str::to_owned),
        ),
        Box::new(optional_digest_bytes(
            new_observation.map(RepositoryObservation::canonical_root_identity_digest),
        )?),
        Box::new(optional_digest_bytes(
            new_observation.map(RepositoryObservation::repository_identity_digest),
        )?),
        Box::new(optional_digest_bytes(
            new_observation.map(RepositoryObservation::file_identity_digest),
        )?),
        Box::new(new_observation.map(|value| value.primary_branch().reference().to_owned())),
        Box::new(optional_digest_bytes(
            new_observation.map(|value| value.primary_branch().storage_identity_digest()),
        )?),
    ])
}

fn stage_project_values(
    profile_version: i16,
    manifest: &str,
    project_id: &ProjectId,
    projection: &RegistryProjectProjection,
) -> PostgresProjectRegistryResult<Vec<Box<dyn ToSql + Sync>>> {
    let authority = projection.authority();
    let drift = drift_flags(projection.drift());
    Ok(vec![
        Box::new(profile_version),
        Box::new(manifest.to_owned()),
        Box::new(project_id.as_str().to_owned()),
        Box::new(projection.project_class().as_str().to_owned()),
        Box::new(digest_bytes(projection.observation().digest())?),
        Box::new(optional_digest_bytes(
            projection
                .pending_observation()
                .map(RepositoryObservation::digest),
        )?),
        Box::new(drift[0]),
        Box::new(drift[1]),
        Box::new(drift[2]),
        Box::new(drift[3]),
        Box::new(drift[4]),
        Box::new(
            i16::try_from(authority.version())
                .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?,
        ),
        Box::new(authority.producer_id().to_owned()),
        Box::new(authority.producer_version().to_owned()),
        Box::new(runtime_text(authority.runtime()).to_owned()),
        Box::new(authority.project_snapshot_id().as_str().to_owned()),
        Box::new(authority.registry_revision().to_string()),
        Box::new(authority.lifecycle().as_str().to_owned()),
        Box::new(authority.primary_branch().reference().to_owned()),
        Box::new(digest_bytes(
            authority.primary_branch().storage_identity_digest(),
        )?),
        Box::new(digest_bytes(authority.observation_digest())?),
        Box::new(digest_bytes(authority.receipt_digest())?),
    ])
}

fn finalize_values(
    profile_version: i16,
    manifest: &str,
    plan: &lattice_project_registry::RegistryCommandPlan,
    durable: &PostgresProjectRegistryPersistenceReceipt,
    stage_project: bool,
    deleted_reservations: u64,
    inserted_reservations: u64,
) -> PostgresProjectRegistryResult<Vec<Box<dyn ToSql + Sync>>> {
    let base = checkpoint_parts(plan.base_checkpoint())?;
    let result = checkpoint_parts(plan.result_checkpoint())?;
    Ok(vec![
        Box::new(profile_version),
        Box::new(manifest.to_owned()),
        Box::new(plan.record_set().command().command_id().as_str().to_owned()),
        Box::new(result.ordinal),
        Box::new(base.runtime),
        Box::new(base.ordinal),
        Box::new(base.observations),
        Box::new(base.projects),
        Box::new(base.commands),
        Box::new(base.reservations),
        Box::new(base.retained_bytes),
        Box::new(base.digest),
        Box::new(result.runtime),
        Box::new(result.ordinal),
        Box::new(result.observations),
        Box::new(result.projects),
        Box::new(result.commands),
        Box::new(result.reservations),
        Box::new(result.retained_bytes),
        Box::new(result.digest),
        Box::new(digest_bytes(plan.record_set().record_set_digest())?),
        Box::new(digest_bytes(durable.transaction_digest())?),
        Box::new(digest_bytes(durable.receipt_digest())?),
        Box::new(plan.record_set().new_observation().is_some()),
        Box::new(stage_project),
        Box::new(signed_i64(deleted_reservations)?),
        Box::new(signed_i64(inserted_reservations)?),
    ])
}

struct HeadParts {
    present: bool,
    producer_id: Option<String>,
    producer_version: Option<String>,
    runtime: Option<String>,
    project_id: Option<String>,
    snapshot_id: Option<String>,
    registry_revision: Option<String>,
    lifecycle: Option<String>,
    project_class: Option<String>,
    primary_ref: Option<String>,
    primary_ref_storage: Option<Vec<u8>>,
    observation_digest: Option<Vec<u8>>,
    receipt_digest: Option<Vec<u8>>,
}

fn head_parts(head: Option<&ProjectAuthorityHead>) -> PostgresProjectRegistryResult<HeadParts> {
    let Some(head) = head else {
        return Ok(HeadParts {
            present: false,
            producer_id: None,
            producer_version: None,
            runtime: None,
            project_id: None,
            snapshot_id: None,
            registry_revision: None,
            lifecycle: None,
            project_class: None,
            primary_ref: None,
            primary_ref_storage: None,
            observation_digest: None,
            receipt_digest: None,
        });
    };
    Ok(HeadParts {
        present: true,
        producer_id: Some(head.producer_id().to_owned()),
        producer_version: Some(head.producer_version().to_owned()),
        runtime: Some(runtime_text(head.runtime()).to_owned()),
        project_id: Some(head.project_id().as_str().to_owned()),
        snapshot_id: Some(head.project_snapshot_id().as_str().to_owned()),
        registry_revision: Some(head.registry_revision().to_string()),
        lifecycle: Some(head.lifecycle().as_str().to_owned()),
        project_class: Some(head.project_class().as_str().to_owned()),
        primary_ref: Some(head.primary_branch().reference().to_owned()),
        primary_ref_storage: Some(digest_bytes(
            head.primary_branch().storage_identity_digest(),
        )?),
        observation_digest: Some(digest_bytes(head.observation_digest())?),
        receipt_digest: Some(digest_bytes(head.receipt_digest())?),
    })
}

struct CheckpointParts {
    runtime: String,
    ordinal: i64,
    observations: i64,
    projects: i64,
    commands: i64,
    reservations: i64,
    retained_bytes: i64,
    digest: Vec<u8>,
}

fn checkpoint_parts(
    checkpoint: &RegistryCheckpoint,
) -> PostgresProjectRegistryResult<CheckpointParts> {
    Ok(CheckpointParts {
        runtime: runtime_text(checkpoint.runtime()).to_owned(),
        ordinal: signed_i64(checkpoint.command_ordinal())?,
        observations: signed_i64(checkpoint.observation_count())?,
        projects: signed_i64(checkpoint.project_count())?,
        commands: signed_i64(checkpoint.command_count())?,
        reservations: signed_i64(checkpoint.reservation_count())?,
        retained_bytes: signed_i64(checkpoint.retained_bytes())?,
        digest: digest_bytes(checkpoint.checkpoint_digest())?,
    })
}

type CommandParts<'a> = (
    &'static str,
    &'a ProjectId,
    Option<ProjectClass>,
    Option<&'a RepositoryObservation>,
    Option<&'a ProjectAuthorityHead>,
    Option<ReconciliationDecision>,
    Option<&'a ContentDigest>,
);

fn command_parts(command: &RegistryCommand) -> CommandParts<'_> {
    match command {
        RegistryCommand::Register {
            project_id,
            project_class,
            observation,
            ..
        } => (
            "REGISTER",
            project_id,
            Some(*project_class),
            Some(observation),
            None,
            None,
            None,
        ),
        RegistryCommand::Observe {
            project_id,
            expected_head,
            observation,
            ..
        } => (
            "OBSERVE",
            project_id,
            None,
            Some(observation),
            Some(expected_head),
            None,
            None,
        ),
        RegistryCommand::Suspend {
            project_id,
            expected_head,
            evidence_digest,
            ..
        } => (
            "SUSPEND",
            project_id,
            None,
            None,
            Some(expected_head),
            None,
            Some(evidence_digest),
        ),
        RegistryCommand::Reconcile {
            project_id,
            expected_head,
            observation,
            decision,
            evidence_digest,
            ..
        } => (
            "RECONCILE",
            project_id,
            None,
            Some(observation),
            Some(expected_head),
            Some(*decision),
            Some(evidence_digest),
        ),
    }
}

fn command_project_id(command: &RegistryCommand) -> &ProjectId {
    match command {
        RegistryCommand::Register { project_id, .. }
        | RegistryCommand::Observe { project_id, .. }
        | RegistryCommand::Suspend { project_id, .. }
        | RegistryCommand::Reconcile { project_id, .. } => project_id,
    }
}

fn drift_flags(drift: &[IdentityDrift]) -> [bool; 5] {
    [
        IdentityDrift::CanonicalRoot,
        IdentityDrift::Repository,
        IdentityDrift::File,
        IdentityDrift::PrimaryRefName,
        IdentityDrift::PrimaryRefStorage,
    ]
    .map(|dimension| drift.contains(&dimension))
}

fn drift_from_flags(flags: [bool; 5]) -> Vec<IdentityDrift> {
    [
        IdentityDrift::CanonicalRoot,
        IdentityDrift::Repository,
        IdentityDrift::File,
        IdentityDrift::PrimaryRefName,
        IdentityDrift::PrimaryRefStorage,
    ]
    .into_iter()
    .zip(flags)
    .filter_map(|(dimension, present)| present.then_some(dimension))
    .collect()
}

fn parse_dimension(value: &str) -> PostgresProjectRegistryResult<IdentityDimension> {
    match value {
        "CANONICAL_ROOT" => Ok(IdentityDimension::CanonicalRoot),
        "REPOSITORY" => Ok(IdentityDimension::Repository),
        "FILE" => Ok(IdentityDimension::File),
        _ => Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
    }
}

fn parse_reservation_status(
    value: &str,
) -> PostgresProjectRegistryResult<RegistryReservationStatus> {
    match value {
        "ACCEPTED" => Ok(RegistryReservationStatus::Accepted),
        "PENDING" => Ok(RegistryReservationStatus::Pending),
        _ => Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
    }
}

fn parse_decision(value: &str) -> PostgresProjectRegistryResult<ReconciliationDecision> {
    match value {
        "ACCEPT_MOVE" => Ok(ReconciliationDecision::AcceptMove),
        "ACCEPT_IDENTITY_CHANGE" => Ok(ReconciliationDecision::AcceptIdentityChange),
        "REACTIVATE" => Ok(ReconciliationDecision::Reactivate),
        _ => Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
    }
}

fn parse_project_class(value: &str) -> PostgresProjectRegistryResult<ProjectClass> {
    match value {
        "USER_PROJECT" => Ok(ProjectClass::UserProject),
        "LATTICE_SYSTEM" => Ok(ProjectClass::LatticeSystem),
        _ => Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
    }
}

fn parse_project_lifecycle(value: &str) -> PostgresProjectRegistryResult<ProjectLifecycle> {
    match value {
        "ACTIVE" => Ok(ProjectLifecycle::Active),
        "SUSPENDED" => Ok(ProjectLifecycle::Suspended),
        "RECONCILIATION_REQUIRED" => Ok(ProjectLifecycle::ReconciliationRequired),
        _ => Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
    }
}

fn parse_runtime(value: &str) -> PostgresProjectRegistryResult<RuntimeKind> {
    match value {
        "LIVE" => Ok(RuntimeKind::Live),
        "FAKE" => Ok(RuntimeKind::Fake),
        _ => Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
    }
}

fn runtime_text(runtime: RuntimeKind) -> &'static str {
    match runtime {
        RuntimeKind::Live => "LIVE",
        RuntimeKind::Fake => "FAKE",
    }
}

fn parse_admission(value: &str) -> PostgresProjectRegistryResult<RuntimeAdmissionMode> {
    match value {
        "ACTIVE" => Ok(RuntimeAdmissionMode::Active),
        "DRAINING" => Ok(RuntimeAdmissionMode::Draining),
        "CANARY" => Ok(RuntimeAdmissionMode::Canary),
        "STOPPED" => Ok(RuntimeAdmissionMode::Stopped),
        "RECONCILIATION_REQUIRED" => Ok(RuntimeAdmissionMode::ReconciliationRequired),
        _ => Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)),
    }
}

fn checkpoint_value(checkpoint: &RegistryCheckpoint) -> CanonicalValue {
    object(vec![
        ("runtime", string(runtime_text(checkpoint.runtime()))),
        ("ordinal", string(checkpoint.command_ordinal().to_string())),
        (
            "observation_count",
            string(checkpoint.observation_count().to_string()),
        ),
        (
            "project_count",
            string(checkpoint.project_count().to_string()),
        ),
        (
            "command_count",
            string(checkpoint.command_count().to_string()),
        ),
        (
            "reservation_count",
            string(checkpoint.reservation_count().to_string()),
        ),
        (
            "retained_bytes",
            string(checkpoint.retained_bytes().to_string()),
        ),
        ("digest", string(checkpoint.checkpoint_digest().as_str())),
    ])
}

fn daemon_authority_value(authority: &StoreAuthorityHead) -> CanonicalValue {
    object(vec![
        ("runtime", string(runtime_text(authority.runtime()))),
        (
            "daemon_instance_id",
            string(authority.daemon_instance_id().as_str()),
        ),
        (
            "daemon_epoch",
            string(authority.daemon_epoch().get().to_string()),
        ),
        ("admission", string(authority.admission().as_str())),
        ("revision", string(authority.revision().get().to_string())),
        (
            "observation_digest",
            string(authority.observation_digest().as_str()),
        ),
        ("head_digest", string(authority.head_digest().as_str())),
    ])
}

fn registry_hash(
    schema: &str,
    value: &CanonicalValue,
) -> PostgresProjectRegistryResult<ContentDigest> {
    let domain = HashDomain::new(schema, "1")
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
    let digest = canonical_sha256(&domain, value)
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
}

fn object(entries: Vec<(&str, CanonicalValue)>) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn string(value: impl Into<String>) -> CanonicalValue {
    CanonicalValue::String(value.into())
}

fn query_one_boxed<C: GenericClient>(
    client: &mut C,
    sql: &str,
    values: &[Box<dyn ToSql + Sync>],
) -> Result<Row, PostgresError> {
    let params = values
        .iter()
        .map(|value| &**value as &(dyn ToSql + Sync))
        .collect::<Vec<_>>();
    client.query_one(sql, &params)
}

fn row_value<T: FromSqlOwned>(row: &Row, index: usize) -> PostgresProjectRegistryResult<T> {
    row.try_get(index)
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
}

fn row_is_null(row: &Row, index: usize) -> PostgresProjectRegistryResult<bool> {
    row.try_get::<usize, Option<String>>(index)
        .map(|value| value.is_none())
        .or_else(|_| {
            row.try_get::<usize, Option<Vec<u8>>>(index)
                .map(|value| value.is_none())
        })
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
}

fn row_digest(row: &Row, index: usize) -> PostgresProjectRegistryResult<ContentDigest> {
    bytes_digest(&row_value::<Vec<u8>>(row, index)?)
}

fn optional_row_digest(
    row: &Row,
    index: usize,
) -> PostgresProjectRegistryResult<Option<ContentDigest>> {
    row_value::<Option<Vec<u8>>>(row, index)?
        .as_deref()
        .map(bytes_digest)
        .transpose()
}

fn bytes_digest(bytes: &[u8]) -> PostgresProjectRegistryResult<ContentDigest> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    if bytes.len() != 32 {
        return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
    }
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    digest(&output)
}

fn digest(value: &str) -> PostgresProjectRegistryResult<ContentDigest> {
    ContentDigest::from_sha256(value.to_owned())
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
}

fn digest_bytes(value: &ContentDigest) -> PostgresProjectRegistryResult<Vec<u8>> {
    let bytes = value.as_str().as_bytes();
    if bytes.len() != 64 {
        return Err(error(PostgresProjectRegistryErrorKind::Malformed));
    }
    let mut output = Vec::with_capacity(32);
    for pair in bytes.chunks_exact(2) {
        let high = hex_nibble(pair[0])
            .ok_or_else(|| error(PostgresProjectRegistryErrorKind::Malformed))?;
        let low = hex_nibble(pair[1])
            .ok_or_else(|| error(PostgresProjectRegistryErrorKind::Malformed))?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn optional_digest_bytes(
    value: Option<&ContentDigest>,
) -> PostgresProjectRegistryResult<Option<Vec<u8>>> {
    value.map(digest_bytes).transpose()
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn required<T>(value: Option<T>) -> PostgresProjectRegistryResult<T> {
    value.ok_or_else(|| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
}

fn nonnegative_i64(value: i64) -> PostgresProjectRegistryResult<u64> {
    u64::try_from(value).map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
}

fn positive_i64(value: i64) -> PostgresProjectRegistryResult<u64> {
    let value = nonnegative_i64(value)?;
    if value == 0 {
        Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
    } else {
        Ok(value)
    }
}

fn positive_i16(value: i16) -> PostgresProjectRegistryResult<u16> {
    u16::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
}

fn signed_i64(value: u64) -> PostgresProjectRegistryResult<i64> {
    i64::try_from(value).map_err(|_| error(PostgresProjectRegistryErrorKind::RevisionOverflow))
}

fn parse_u64_text(value: &str) -> PostgresProjectRegistryResult<u64> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))?;
    if parsed.to_string() != value || parsed == 0 {
        Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
    } else {
        Ok(parsed)
    }
}

fn profile_version(
    persistence: &PostgresProjectRegistryPersistenceEvidence,
) -> PostgresProjectRegistryResult<i16> {
    i16::try_from(persistence.schema_version())
        .map_err(|_| error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
}

fn retained_command_persistence(
    row: &Row,
    current: &PostgresProjectRegistryPersistenceEvidence,
) -> PostgresProjectRegistryResult<PostgresProjectRegistryPersistenceEvidence> {
    let schema_version = positive_i16(row_value(row, 64)?)?;
    let manifest = row_value::<String>(row, 65)?;
    retained_command_persistence_from_parts(schema_version, manifest.trim(), current)
}

fn retained_command_persistence_from_parts(
    schema_version: u16,
    manifest_sha256: &str,
    current: &PostgresProjectRegistryPersistenceEvidence,
) -> PostgresProjectRegistryResult<PostgresProjectRegistryPersistenceEvidence> {
    let profile_is_frozen_v4 =
        schema_version == 4 && manifest_sha256 == REGISTRY_V4_MANIFEST_SHA256;
    let profile_is_frozen_v5 = schema_version == FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION
        && manifest_sha256 == CURRENT_V5_MANIFEST_SHA256;
    // Upgrading the live schema does not rewrite historical command receipts.
    let profile_is_frozen_v7 = current.schema_version()
        == EXTERNAL_ADOPTION_GLOBAL_REGISTRY_SCHEMA_VERSION
        && schema_version == CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION
        && manifest_sha256 == CURRENT_V7_MANIFEST_SHA256;
    let profile_is_current = schema_version == current.schema_version()
        && matches!(
            current.schema_version(),
            FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION
                | CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION
                | EXTERNAL_ADOPTION_GLOBAL_REGISTRY_SCHEMA_VERSION
        )
        && manifest_sha256 == current.manifest_digest().as_str();
    if !profile_is_frozen_v4
        && !profile_is_frozen_v5
        && !profile_is_frozen_v7
        && !profile_is_current
    {
        return Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt));
    }
    Ok(PostgresProjectRegistryPersistenceEvidence {
        database_identity_digest: current.database_identity_digest().clone(),
        schema_version,
        manifest_digest: digest(manifest_sha256)?,
    })
}

fn check_optional_deadline(started: Option<&Instant>) -> PostgresProjectRegistryResult<()> {
    started.map_or(Ok(()), ensure_deadline)
}

fn ensure_deadline(started: &Instant) -> PostgresProjectRegistryResult<()> {
    if started.elapsed() >= EXECUTION_DEADLINE {
        Err(error(PostgresProjectRegistryErrorKind::Unavailable))
    } else {
        Ok(())
    }
}

fn classify_query_error(database: &PostgresError) -> AttemptFailure {
    if database
        .as_db_error()
        .is_some_and(|db| retryable_sqlstate(db.code().code(), db.constraint()))
    {
        AttemptFailure::Retryable
    } else {
        AttemptFailure::Terminal(map_database_error(database))
    }
}

fn classify_commit_error(database: &PostgresError) -> AttemptFailure {
    let db = database.as_db_error();
    match commit_failure_class(
        db.map(|value| value.code().code()),
        db.and_then(|value| value.constraint()),
    ) {
        CommitFailureClass::Retryable => AttemptFailure::Retryable,
        CommitFailureClass::OutcomeUnknown => AttemptFailure::CommitOutcomeUnknown,
        CommitFailureClass::Terminal => AttemptFailure::Terminal(map_database_error(database)),
    }
}

fn commit_failure_class(code: Option<&str>, constraint: Option<&str>) -> CommitFailureClass {
    match code {
        Some(code) if retryable_sqlstate(code, constraint) => CommitFailureClass::Retryable,
        Some(_) => CommitFailureClass::Terminal,
        None => CommitFailureClass::OutcomeUnknown,
    }
}

fn retryable_sqlstate(code: &str, constraint: Option<&str>) -> bool {
    matches!(code, "40001" | "40P01" | "LCP01")
        || (code == "23505"
            && matches!(
                constraint,
                Some(
                    "project_registry_commands_pkey"
                        | "project_registry_commands_command_id_key"
                        | "project_registry_identity_reservations_pkey"
                )
            ))
}

fn map_database_error(database: &PostgresError) -> PostgresProjectRegistryError {
    let Some(db) = database.as_db_error() else {
        return error(PostgresProjectRegistryErrorKind::Unavailable);
    };
    match db.code().code() {
        "LPR01" => error(PostgresProjectRegistryErrorKind::Malformed),
        "LAM01" => error(PostgresProjectRegistryErrorKind::AdmissionDenied),
        "LAR01" | "LAD01" => error(PostgresProjectRegistryErrorKind::AuthorityMismatch),
        "LCP01" => error(PostgresProjectRegistryErrorKind::CheckpointChanged),
        "LCR01" | "LST01" | "LST02" => error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt),
        "55P03" | "57014" | "42501" => error(PostgresProjectRegistryErrorKind::Unavailable),
        _ => error(PostgresProjectRegistryErrorKind::TransactionFailed),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn map_registry_error(value: RegistryError) -> PostgresProjectRegistryError {
    match value {
        RegistryError::CommandIdReuse => {
            error(PostgresProjectRegistryErrorKind::CommandSubstitution)
        }
        RegistryError::CheckpointMismatch => {
            error(PostgresProjectRegistryErrorKind::CheckpointChanged)
        }
        RegistryError::CapacityExceeded => {
            error(PostgresProjectRegistryErrorKind::CapacityExceeded)
        }
        RegistryError::CommandOrdinalOverflow => {
            error(PostgresProjectRegistryErrorKind::RevisionOverflow)
        }
        RegistryError::CorruptSnapshot => {
            error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)
        }
        _ => error(PostgresProjectRegistryErrorKind::Malformed),
    }
}

fn map_retained_registry_error(_: RegistryError) -> PostgresProjectRegistryError {
    error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt)
}

fn map_setup_error(value: crate::PostgresStoreSetupError) -> PostgresProjectRegistryError {
    match value.kind() {
        PostgresStoreSetupErrorKind::NetworkBoundary => {
            error(PostgresProjectRegistryErrorKind::Unavailable)
        }
        PostgresStoreSetupErrorKind::TransactionFailed => {
            error(PostgresProjectRegistryErrorKind::TransactionFailed)
        }
        PostgresStoreSetupErrorKind::CommitOutcomeUnknown => {
            error(PostgresProjectRegistryErrorKind::CommitOutcomeUnknown)
        }
        PostgresStoreSetupErrorKind::TargetMismatch => {
            error(PostgresProjectRegistryErrorKind::Malformed)
        }
        _ => error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt),
    }
}

fn rollback_attempt(
    transaction: Transaction<'_>,
    failure: AttemptFailure,
) -> Result<(PostgresProjectRegistryExecution, RegistryCheckpoint), AttemptFailure> {
    match transaction.rollback() {
        Ok(()) => Err(failure),
        Err(_) => Err(AttemptFailure::Terminal(error(
            PostgresProjectRegistryErrorKind::TransactionFailed,
        ))),
    }
}

fn rollback_load<T>(
    transaction: Transaction<'_>,
    failure: PostgresProjectRegistryError,
) -> PostgresProjectRegistryResult<T> {
    match transaction.rollback() {
        Ok(()) => Err(failure),
        Err(_) => Err(error(PostgresProjectRegistryErrorKind::TransactionFailed)),
    }
}

const fn error(kind: PostgresProjectRegistryErrorKind) -> PostgresProjectRegistryError {
    PostgresProjectRegistryError::new(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_transactions_never_extend_a_shorter_session_deadline() {
        for settings in [WRITE_TRANSACTION_SETTINGS, READ_TRANSACTION_SETTINGS] {
            assert!(settings.contains("pg_catalog.set_config('lock_timeout'"));
            assert!(settings.contains("pg_catalog.set_config('statement_timeout'"));
            assert!(settings.contains("LEAST("));
            assert!(!settings.contains("SET LOCAL lock_timeout = '5s'"));
            assert!(!settings.contains("SET LOCAL statement_timeout = '30s'"));
        }
    }

    #[test]
    fn retry_and_commit_failure_classification_is_closed() {
        assert!(retryable_sqlstate("40001", None));
        assert!(retryable_sqlstate("40P01", None));
        assert!(retryable_sqlstate("LCP01", None));
        assert!(retryable_sqlstate(
            "23505",
            Some("project_registry_commands_pkey")
        ));
        assert!(!retryable_sqlstate("23505", Some("unrelated_unique")));
        assert_eq!(
            commit_failure_class(None, None) as u8,
            CommitFailureClass::OutcomeUnknown as u8
        );
    }

    #[test]
    fn error_codes_are_unique_and_static() {
        let mut codes =
            PostgresProjectRegistryErrorKind::ALL.map(PostgresProjectRegistryErrorKind::code);
        codes.sort_unstable();
        assert!(codes.windows(2).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn registry_sql_profiles_accept_exact_v5_v7_and_v8() {
        assert_eq!(
            RegistrySqlProfile::from_schema_version(5),
            Ok(RegistrySqlProfile::FrozenV5)
        );
        assert_eq!(
            RegistrySqlProfile::from_schema_version(7),
            Ok(RegistrySqlProfile::CurrentV7)
        );
        assert_eq!(
            RegistrySqlProfile::from_schema_version(8),
            Ok(RegistrySqlProfile::CurrentV8)
        );
        for unsupported in [0, 1, 3, 4, 6, u16::MAX] {
            assert_eq!(
                RegistrySqlProfile::from_schema_version(unsupported),
                Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
            );
        }
    }

    #[test]
    fn retained_registry_commands_bind_only_frozen_v4_v5_or_exact_current_profile() {
        let current_v5 = PostgresProjectRegistryPersistenceEvidence {
            database_identity_digest: digest(&"a".repeat(64)).expect("database identity"),
            schema_version: FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION,
            manifest_digest: digest(CURRENT_V5_MANIFEST_SHA256).expect("v5 manifest"),
        };
        let historical =
            retained_command_persistence_from_parts(4, REGISTRY_V4_MANIFEST_SHA256, &current_v5)
                .expect("frozen v4 profile");
        assert_eq!(historical.schema_version(), 4);
        assert_eq!(
            historical.manifest_digest().as_str(),
            REGISTRY_V4_MANIFEST_SHA256
        );
        assert_eq!(
            retained_command_persistence_from_parts(
                FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION,
                CURRENT_V5_MANIFEST_SHA256,
                &current_v5,
            ),
            Ok(current_v5.clone())
        );
        let current_v7 = PostgresProjectRegistryPersistenceEvidence {
            database_identity_digest: current_v5.database_identity_digest().clone(),
            schema_version: CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION,
            manifest_digest: digest(&"b".repeat(64)).expect("v7 manifest"),
        };
        let frozen_v5 = retained_command_persistence_from_parts(
            FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION,
            CURRENT_V5_MANIFEST_SHA256,
            &current_v7,
        )
        .expect("frozen v5 profile under v7");
        assert_eq!(
            frozen_v5.schema_version(),
            FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION
        );
        assert_eq!(
            retained_command_persistence_from_parts(
                CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION,
                current_v7.manifest_digest().as_str(),
                &current_v7,
            ),
            Ok(current_v7.clone())
        );
        for (version, manifest) in [
            (4, current_v7.manifest_digest().as_str()),
            (
                FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION,
                current_v7.manifest_digest().as_str(),
            ),
            (
                CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION,
                CURRENT_V5_MANIFEST_SHA256,
            ),
            (6, current_v7.manifest_digest().as_str()),
            (3, REGISTRY_V4_MANIFEST_SHA256),
            (8, current_v7.manifest_digest().as_str()),
        ] {
            assert_eq!(
                retained_command_persistence_from_parts(version, manifest, &current_v7),
                Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
            );
        }
    }

    #[test]
    fn retained_registry_v7_commands_replay_under_v8_with_the_exact_frozen_manifest() {
        let current_v8 = PostgresProjectRegistryPersistenceEvidence {
            database_identity_digest: digest(&"a".repeat(64)).expect("database identity"),
            schema_version: EXTERNAL_ADOPTION_GLOBAL_REGISTRY_SCHEMA_VERSION,
            manifest_digest: digest(crate::migrations::CURRENT_V8_MANIFEST_SHA256)
                .expect("v8 manifest"),
        };
        let historical = retained_command_persistence_from_parts(
            CURRENT_GLOBAL_REGISTRY_SCHEMA_VERSION,
            crate::migrations::CURRENT_V7_MANIFEST_SHA256,
            &current_v8,
        )
        .expect("frozen v7 command survives the v8 migration");
        assert_eq!(historical.schema_version(), 7);
        assert_eq!(
            historical.manifest_digest().as_str(),
            crate::migrations::CURRENT_V7_MANIFEST_SHA256
        );
        assert_eq!(
            historical.database_identity_digest(),
            current_v8.database_identity_digest()
        );
        assert_eq!(
            retained_command_persistence_from_parts(
                8,
                current_v8.manifest_digest().as_str(),
                &current_v8,
            ),
            Ok(current_v8.clone())
        );
        for (version, manifest) in [
            (7, current_v8.manifest_digest().as_str()),
            (7, CURRENT_V5_MANIFEST_SHA256),
            (6, crate::migrations::CURRENT_V7_MANIFEST_SHA256),
            (9, crate::migrations::CURRENT_V7_MANIFEST_SHA256),
            (8, crate::migrations::CURRENT_V7_MANIFEST_SHA256),
        ] {
            assert_eq!(
                retained_command_persistence_from_parts(version, manifest, &current_v8),
                Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
            );
        }
        let current_v5 = PostgresProjectRegistryPersistenceEvidence {
            database_identity_digest: current_v8.database_identity_digest().clone(),
            schema_version: FROZEN_GLOBAL_REGISTRY_SCHEMA_VERSION,
            manifest_digest: digest(CURRENT_V5_MANIFEST_SHA256).expect("v5 manifest"),
        };
        assert_eq!(
            retained_command_persistence_from_parts(
                7,
                crate::migrations::CURRENT_V7_MANIFEST_SHA256,
                &current_v5,
            ),
            Err(error(PostgresProjectRegistryErrorKind::RetainedRowCorrupt))
        );
    }

    #[test]
    fn stage_command_keeps_text_transport_for_numeric_registry_revision() {
        assert!(STAGE_COMMAND_SQL.contains("$17::text::numeric"));
        assert!(!STAGE_COMMAND_SQL.contains("$17::numeric"));
    }
}
