use lattice_contracts::{
    ContentDigest, DaemonEpoch, ProjectId, RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead,
    WriterLeaseAuthorityHead, WriterLeaseAuthorityReceipt,
};
use lattice_writer_lease::{
    AcquireClaim, AcquireCommand, CommandOutcome, HeartbeatCommand, LeaseObservation,
    MarkSuspectCommand, ProcessHandoffCommand, ReleaseCommand, RevokeCommand,
    UntrustedWriterLeaseSnapshot, VerifiedWriterLeaseAggregate, WriterLeaseAcquireRequest,
    WriterLeaseCheckpoint, WriterLeaseCommand, WriterLeaseCommandReceipt,
    WriterLeaseCurrentAuthority, WriterLeaseProjectEvidence, WriterLeaseReleaseRequest,
    WriterLeaseRepository, WriterLeaseRepositoryCommand, WriterLeaseRepositoryError,
    WriterLeaseRepositoryErrorKind, apply_plan, plan_command, verify_snapshot_against_checkpoint,
};
use postgres::error::SqlState;
use postgres::{Client, IsolationLevel, Row, Transaction};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

use crate::setup::{ExtensionTarget, V3ExtensionTarget, V4ExtensionTarget, V5ExtensionTarget};
use crate::{
    sha256_bytes, verify_embedded_extension_manifest, verify_embedded_v3_extension_manifest,
    verify_embedded_v4_extension_manifest, verify_embedded_v5_extension_manifest,
};

const MAX_SERIALIZATION_RETRIES: usize = 3;
const BIND_RUNTIME_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_bind_runtime_v2($1,$2,$3,$4,$5,$6,$7,$8)";
const LOAD_FOR_UPDATE_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_load_for_update_v2($1,$2,$3,$4,$5)";
const BIND_RUNTIME_V3_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_bind_runtime_v3($1,$2,$3,$4,$5,$6,$7,$8)";
const LOAD_FOR_UPDATE_V3_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_load_for_update_v3($1,$2,$3,$4,$5)";
const BIND_RUNTIME_V4_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_bind_runtime_v4($1,$2,$3,$4,$5,$6,$7,$8)";
const LOAD_FOR_UPDATE_V4_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_load_for_update_v4($1,$2,$3,$4,$5)";
const BIND_RUNTIME_V5_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_bind_runtime_v5($1,$2,$3,$4,$5,$6,$7,$8)";
const LOAD_FOR_UPDATE_V5_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_load_for_update_v5($1,$2,$3,$4,$5)";
const LOAD_COMMANDS_SQL: &str = "SELECT * FROM writer_lease.writer_lease_load_commands_v1($1)";
const LOAD_TRANSITIONS_SQL: &str =
    "SELECT * FROM writer_lease.writer_lease_load_transitions_v1($1)";
const LOAD_CURRENT_SQL: &str = "SELECT * FROM writer_lease.writer_lease_load_current_v1($1)";
const ASSERT_CURRENT_SQL: &str = "SELECT writer_lease.writer_lease_assert_current_v1(\
        $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)";
const COMMIT_PLAN_SQL: &str = "SELECT writer_lease.writer_lease_commit_plan_v1(\
        $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,\
        $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30,$31,$32,$33,$34,$35,$36,$37,\
        $38,$39,$40,$41,$42,$43,$44,$45,$46,$47,$48)";

/// Live `PostgreSQL` implementation of the domain-owned repository port.
pub struct PostgresWriterLease {
    client: Client,
    target: ExtensionTarget,
    procedure_profile: RuntimeProcedureProfile,
    lease_ttl_seconds: u32,
    bound_daemon_instance_id: String,
    bound_daemon_epoch: DaemonEpoch,
}

#[derive(Clone, Copy)]
enum RuntimeProcedureProfile {
    V2,
    V3,
    V4,
    V5,
}

impl RuntimeProcedureProfile {
    const fn bind_runtime_sql(self) -> &'static str {
        match self {
            Self::V2 => BIND_RUNTIME_SQL,
            Self::V3 => BIND_RUNTIME_V3_SQL,
            Self::V4 => BIND_RUNTIME_V4_SQL,
            Self::V5 => BIND_RUNTIME_V5_SQL,
        }
    }

    const fn load_for_update_sql(self) -> &'static str {
        match self {
            Self::V2 => LOAD_FOR_UPDATE_SQL,
            Self::V3 => LOAD_FOR_UPDATE_V3_SQL,
            Self::V4 => LOAD_FOR_UPDATE_V4_SQL,
            Self::V5 => LOAD_FOR_UPDATE_V5_SQL,
        }
    }
}

impl PostgresWriterLease {
    /// Constructs one runtime adapter around an already provisioned runtime
    /// connection. The connection string and credentials remain outside this
    /// type and never enter repository commands.
    ///
    /// # Errors
    ///
    /// Rejects a database mismatch, a TTL outside 1..=3600 seconds, an
    /// unverified process-start Store authority, or an unavailable connection.
    /// Shared credentials alone cannot borrow the current daemon identity.
    pub fn new(
        client: Client,
        target: ExtensionTarget,
        store_authority: &StoreAuthorityHead,
        lease_ttl_seconds: u32,
    ) -> Result<Self, WriterLeaseRepositoryError> {
        let manifest = verify_embedded_extension_manifest()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        Self::new_with_profile(
            client,
            target,
            store_authority,
            lease_ttl_seconds,
            RuntimeProcedureProfile::V2,
            manifest.sql_sha256(),
            manifest.manifest_sha256(),
        )
    }

    /// Constructs the current schema-v6 adapter through Writer-owned v3
    /// procedures. Historical v2 callers retain [`Self::new`] and cannot
    /// silently cross the versioned procedure boundary.
    ///
    /// # Errors
    ///
    /// Rejects any target, authority, manifest, or database profile mismatch.
    pub fn new_v3(
        client: Client,
        target: &V3ExtensionTarget,
        store_authority: &StoreAuthorityHead,
        lease_ttl_seconds: u32,
    ) -> Result<Self, WriterLeaseRepositoryError> {
        let runtime_target = target
            .successor()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        let manifest = verify_embedded_v3_extension_manifest()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        Self::new_with_profile(
            client,
            runtime_target,
            store_authority,
            lease_ttl_seconds,
            RuntimeProcedureProfile::V3,
            manifest.sql_sha256(),
            manifest.manifest_sha256(),
        )
    }

    /// Constructs the exact schema-v7 adapter through the append-only
    /// Writer-owned v4 procedure surface. Frozen v3 callers remain version-
    /// closed to schema v6.
    ///
    /// # Errors
    ///
    /// Rejects any target, authority, manifest, or database profile mismatch.
    pub fn new_v4_v7(
        client: Client,
        target: &V4ExtensionTarget,
        store_authority: &StoreAuthorityHead,
        lease_ttl_seconds: u32,
    ) -> Result<Self, WriterLeaseRepositoryError> {
        let runtime_target = target
            .successor()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        let manifest = verify_embedded_v4_extension_manifest()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        Self::new_with_profile(
            client,
            runtime_target,
            store_authority,
            lease_ttl_seconds,
            RuntimeProcedureProfile::V4,
            manifest.sql_sha256(),
            manifest.manifest_sha256(),
        )
    }

    /// Constructs the exact schema-v7 adapter through the append-only Writer
    /// v5 process-handoff procedure surface.
    ///
    /// # Errors
    ///
    /// Rejects any target, authority, manifest, or database profile mismatch.
    pub fn new_v5_v7(
        client: Client,
        target: &V5ExtensionTarget,
        store_authority: &StoreAuthorityHead,
        lease_ttl_seconds: u32,
    ) -> Result<Self, WriterLeaseRepositoryError> {
        let runtime_target = target
            .successor()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        let manifest = verify_embedded_v5_extension_manifest()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        Self::new_with_profile(
            client,
            runtime_target,
            store_authority,
            lease_ttl_seconds,
            RuntimeProcedureProfile::V5,
            manifest.sql_sha256(),
            manifest.manifest_sha256(),
        )
    }

    fn new_with_profile(
        mut client: Client,
        target: ExtensionTarget,
        store_authority: &StoreAuthorityHead,
        lease_ttl_seconds: u32,
        procedure_profile: RuntimeProcedureProfile,
        extension_sql_sha256: &ContentDigest,
        extension_manifest_sha256: &ContentDigest,
    ) -> Result<Self, WriterLeaseRepositoryError> {
        if !(1..=3600).contains(&lease_ttl_seconds) {
            return Err(repository_error(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        if store_authority.runtime() != RuntimeKind::Live
            || store_authority.admission() != RuntimeAdmissionMode::Active
        {
            return Err(repository_error(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        let database_name: String = client
            .query_one("SELECT pg_catalog.current_database()::text", &[])
            .and_then(|row| row.try_get(0))
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        if database_name != target.database_name() {
            return Err(repository_error(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        let expected_daemon_epoch =
            to_i64(store_authority.daemon_epoch().get()).map_err(|failure| failure.error)?;
        let expected_admission_digest =
            digest_bytes(store_authority.observation_digest()).map_err(|failure| failure.error)?;
        let mut transaction = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        enter_runtime_reader(&mut transaction).map_err(|failure| failure.error)?;
        let binding = transaction
            .query_one(
                procedure_profile.bind_runtime_sql(),
                &[
                    &store_authority.daemon_instance_id().as_str(),
                    &expected_daemon_epoch,
                    &expected_admission_digest,
                    &target.database_identity_digest().as_str(),
                    &target.global_manifest_digest().as_str(),
                    &target.memory_manifest_digest().as_str(),
                    &extension_sql_sha256.as_str(),
                    &extension_manifest_sha256.as_str(),
                ],
            )
            .map_err(|error| database_failure(error).error)?;
        let bound_daemon_instance_id: String = binding
            .try_get(0)
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        let bound_daemon_epoch = binding
            .try_get::<_, i64>(1)
            .ok()
            .and_then(|value| u64::try_from(value).ok())
            .and_then(|value| DaemonEpoch::new(value).ok())
            .ok_or_else(|| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        let admission_digest: Vec<u8> = binding
            .try_get(2)
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Corrupt))?;
        if bound_daemon_instance_id.is_empty() || admission_digest.len() != 32 {
            return Err(repository_error(WriterLeaseRepositoryErrorKind::Corrupt));
        }
        if bound_daemon_instance_id != store_authority.daemon_instance_id().as_str()
            || bound_daemon_epoch != store_authority.daemon_epoch()
            || admission_digest != expected_admission_digest
        {
            return Err(repository_error(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        transaction
            .commit()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        Ok(Self {
            client,
            target,
            procedure_profile,
            lease_ttl_seconds,
            bound_daemon_instance_id,
            bound_daemon_epoch,
        })
    }

    /// Replays one existing project's complete durable state without acquiring,
    /// renewing, releasing, or otherwise mutating a Writer Lease.
    ///
    /// `Ok(None)` means the project has no durable Writer Lease head/history.
    /// `Ok(Some(_))` also represents a released aggregate: in that case the
    /// evidence retains its monotonic high-water marks while
    /// `current_authority()` is `None`.
    ///
    /// # Errors
    ///
    /// Fails closed if the snapshot/checkpoint/current projection or any
    /// physical command/transition row cannot be replayed byte-exactly.
    pub fn inspect_project(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<Option<WriterLeaseProjectEvidence>, WriterLeaseRepositoryError> {
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        enter_runtime_reader(&mut transaction).map_err(|failure| failure.error)?;
        let evidence = Self::load_current_in(&mut transaction, project_id)
            .map_err(|failure| failure.error)?
            .as_ref()
            .map(|loaded| WriterLeaseProjectEvidence::from_verified_aggregate(&loaded.aggregate))
            .transpose()?;
        transaction
            .commit()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        Ok(evidence)
    }

    /// Replays the complete owner history and returns one exact historical
    /// authority receipt by digest, even when the aggregate is now released.
    ///
    /// # Errors
    ///
    /// Physical/snapshot tamper, duplicate authority evidence, a zero digest,
    /// or unavailable owner storage fails closed.
    pub fn inspect_historical_authority(
        &mut self,
        project_id: &ProjectId,
        receipt_digest: &ContentDigest,
    ) -> Result<Option<WriterLeaseAuthorityReceipt>, WriterLeaseRepositoryError> {
        validate_historical_receipt_digest(receipt_digest)?;
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        enter_runtime_reader(&mut transaction).map_err(|failure| failure.error)?;
        let receipt = match Self::load_current_in(&mut transaction, project_id)
            .map_err(|failure| failure.error)?
        {
            None => None,
            Some(loaded) => loaded
                .aggregate
                .historical_authority_receipt(receipt_digest)
                .map_err(|error| domain_failure(error).error)?,
        };
        transaction
            .commit()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        Ok(receipt)
    }

    /// Reconstructs one exact applied release intent from replay-verified
    /// Writer history. This is used only to finish a predecessor release that
    /// committed before a process crash; absence is distinct from corruption.
    ///
    /// # Errors
    ///
    /// Duplicate, denied, non-release, malformed, or physically inconsistent
    /// history fails closed.
    pub fn replay_applied_release_request(
        &mut self,
        project_id: &ProjectId,
        command_id: &str,
    ) -> Result<Option<WriterLeaseReleaseRequest>, WriterLeaseRepositoryError> {
        if command_id.is_empty() || command_id.len() > 128 {
            return Err(repository_error(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        enter_runtime_reader(&mut transaction).map_err(|failure| failure.error)?;
        let loaded =
            Self::load_current_in(&mut transaction, project_id).map_err(|failure| failure.error)?;
        let request = match loaded {
            None => None,
            Some(loaded) => {
                let matches = loaded
                    .aggregate
                    .command_receipts()
                    .iter()
                    .filter(|receipt| receipt.request.command_id() == command_id)
                    .collect::<Vec<_>>();
                match matches.as_slice() {
                    [] => None,
                    [receipt] => {
                        let lattice_writer_lease::WriterLeaseCommand::Release(release) =
                            &receipt.request
                        else {
                            return Err(repository_error(WriterLeaseRepositoryErrorKind::Corrupt));
                        };
                        if receipt.outcome != CommandOutcome::Applied
                            || receipt.before.as_ref() != Some(&release.expected_head)
                            || receipt.after.is_some()
                            || &release.project_id != project_id
                        {
                            return Err(repository_error(WriterLeaseRepositoryErrorKind::Corrupt));
                        }
                        Some(WriterLeaseReleaseRequest {
                            command_id: command_id.to_owned(),
                            project_id: project_id.clone(),
                            expected_head: release.expected_head.clone(),
                        })
                    }
                    _ => {
                        return Err(repository_error(WriterLeaseRepositoryErrorKind::Corrupt));
                    }
                }
            }
        };
        transaction
            .commit()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        Ok(request)
    }

    /// Reconstructs one exact applied acquire intent from replay-verified
    /// Writer history so a fresh process can retry the original holder
    /// identity instead of substituting its new PID/start observation.
    ///
    /// # Errors
    ///
    /// Duplicate, denied, non-acquire, malformed, or physically inconsistent
    /// history fails closed.
    pub fn replay_applied_acquire_request(
        &mut self,
        project_id: &ProjectId,
        command_id: &str,
    ) -> Result<Option<WriterLeaseAcquireRequest>, WriterLeaseRepositoryError> {
        if command_id.is_empty() || command_id.len() > 128 {
            return Err(repository_error(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        enter_runtime_reader(&mut transaction).map_err(|failure| failure.error)?;
        let loaded =
            Self::load_current_in(&mut transaction, project_id).map_err(|failure| failure.error)?;
        let request = match loaded {
            None => None,
            Some(loaded) => {
                let matches = loaded
                    .aggregate
                    .command_receipts()
                    .iter()
                    .filter(|receipt| receipt.request.command_id() == command_id)
                    .collect::<Vec<_>>();
                match matches.as_slice() {
                    [] => None,
                    [receipt] => {
                        let lattice_writer_lease::WriterLeaseCommand::Acquire(acquire) =
                            &receipt.request
                        else {
                            return Err(repository_error(WriterLeaseRepositoryErrorKind::Corrupt));
                        };
                        if receipt.outcome != CommandOutcome::Applied
                            || receipt.before != acquire.expected_head
                            || receipt.after.is_none()
                            || &acquire.claim.project_id != project_id
                        {
                            return Err(repository_error(WriterLeaseRepositoryErrorKind::Corrupt));
                        }
                        Some(WriterLeaseAcquireRequest {
                            command_id: command_id.to_owned(),
                            expected_head: acquire.expected_head.clone(),
                            project_id: acquire.claim.project_id.clone(),
                            project_snapshot_id: acquire.claim.project_snapshot_id.clone(),
                            task_id: acquire.claim.task_id.clone(),
                            task_revision: acquire.claim.task_revision.clone(),
                            task_spec_digest: acquire.claim.task_spec_digest.clone(),
                            attempt_id: acquire.claim.attempt_id.clone(),
                            lease_id: acquire.claim.lease_id.clone(),
                            lease_holder_id: acquire.claim.lease_holder_id.clone(),
                            worktree_id: acquire.claim.worktree_id.clone(),
                            holder_process_id: acquire.claim.holder_process_id,
                            holder_process_start_identity: acquire
                                .claim
                                .holder_process_start_identity
                                .clone(),
                        })
                    }
                    _ => {
                        return Err(repository_error(WriterLeaseRepositoryErrorKind::Corrupt));
                    }
                }
            }
        };
        transaction
            .commit()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        Ok(request)
    }

    /// Atomically releases one exact retained Writer head and acquires its exact
    /// successor. Both existing domain commands are planned and persisted in a
    /// single serializable transaction, so no competing acquire can consume the
    /// successor fence between the two operations. Exact retries reconcile both
    /// command receipts before returning the retained successor head.
    ///
    /// # Errors
    ///
    /// Cross-project requests, a denied/substituted command, serialization
    /// exhaustion, corrupt history, or an unknown commit outcome fail closed.
    pub fn rotate_exact(
        &mut self,
        release: WriterLeaseReleaseRequest,
        acquire: WriterLeaseAcquireRequest,
    ) -> Result<WriterLeaseAuthorityHead, WriterLeaseRepositoryError> {
        let release_command = WriterLeaseRepositoryCommand::Release(release);
        let acquire_command = WriterLeaseRepositoryCommand::Acquire(acquire);
        if release_command.project_id() != acquire_command.project_id() {
            return Err(repository_error(
                WriterLeaseRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        for attempt in 0..=MAX_SERIALIZATION_RETRIES {
            match self.rotate_once(&release_command, &acquire_command) {
                Ok(head) => return Ok(head),
                Err(error) if error.retryable && attempt < MAX_SERIALIZATION_RETRIES => {}
                Err(error) if error.retryable => {
                    return Err(repository_error(
                        WriterLeaseRepositoryErrorKind::SerializationExhausted,
                    ));
                }
                Err(error) => return Err(error.error),
            }
        }
        Err(repository_error(
            WriterLeaseRepositoryErrorKind::SerializationExhausted,
        ))
    }

    fn rotate_once(
        &mut self,
        release: &WriterLeaseRepositoryCommand,
        acquire: &WriterLeaseRepositoryCommand,
    ) -> Result<WriterLeaseAuthorityHead, AdapterFailure> {
        let procedure_profile = self.procedure_profile;
        let lease_ttl_seconds = self.lease_ttl_seconds;
        let bound_daemon_instance_id = self.bound_daemon_instance_id.clone();
        let bound_daemon_epoch = self.bound_daemon_epoch;
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(database_failure)?;
        enter_runtime_writer(&mut transaction)?;
        let released = execute_in_transaction(
            &mut transaction,
            procedure_profile,
            lease_ttl_seconds,
            &bound_daemon_instance_id,
            bound_daemon_epoch,
            release,
        )?;
        if released.outcome != CommandOutcome::Applied || released.after.is_some() {
            return Err(authority_failure());
        }
        let acquired = execute_in_transaction(
            &mut transaction,
            procedure_profile,
            lease_ttl_seconds,
            &bound_daemon_instance_id,
            bound_daemon_epoch,
            acquire,
        )?;
        let head = acquired
            .after
            .filter(|_| acquired.outcome == CommandOutcome::Applied)
            .ok_or_else(authority_failure)?;
        transaction.commit().map_err(commit_failure)?;
        Ok(head)
    }

    fn execute_once(
        &mut self,
        repository_command: &WriterLeaseRepositoryCommand,
    ) -> Result<WriterLeaseCommandReceipt, AdapterFailure> {
        let procedure_profile = self.procedure_profile;
        let lease_ttl_seconds = self.lease_ttl_seconds;
        let bound_daemon_instance_id = self.bound_daemon_instance_id.clone();
        let bound_daemon_epoch = self.bound_daemon_epoch;
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(database_failure)?;
        enter_runtime_writer(&mut transaction)?;
        let receipt = execute_in_transaction(
            &mut transaction,
            procedure_profile,
            lease_ttl_seconds,
            &bound_daemon_instance_id,
            bound_daemon_epoch,
            repository_command,
        )?;
        transaction.commit().map_err(commit_failure)?;
        Ok(receipt)
    }

    fn load_current_in(
        transaction: &mut Transaction<'_>,
        project_id: &ProjectId,
    ) -> Result<Option<LoadedAggregate>, AdapterFailure> {
        let rows = transaction
            .query(LOAD_CURRENT_SQL, &[&project_id.as_str()])
            .map_err(database_failure)?;
        match rows.as_slice() {
            [] => Ok(None),
            [row] => {
                let loaded = LoadedAggregate::from_current_row(row, project_id)?;
                verify_physical_history(transaction, project_id, &loaded.aggregate)?;
                Ok(Some(loaded))
            }
            _ => Err(corrupt_failure()),
        }
    }
}

fn execute_in_transaction(
    transaction: &mut Transaction<'_>,
    procedure_profile: RuntimeProcedureProfile,
    lease_ttl_seconds: u32,
    bound_daemon_instance_id: &str,
    bound_daemon_epoch: DaemonEpoch,
    repository_command: &WriterLeaseRepositoryCommand,
) -> Result<WriterLeaseCommandReceipt, AdapterFailure> {
    let project_id = repository_command.project_id();
    let repository_request_bytes = repository_command
        .canonical_bytes()
        .map_err(domain_failure)?;
    let repository_request_sha256 = sha256_bytes(&repository_request_bytes);
    let vacant = VerifiedWriterLeaseAggregate::vacant(project_id.clone());
    let vacant_bytes = vacant.export_canonical_bytes().map_err(domain_failure)?;
    let vacant_checkpoint = vacant.checkpoint().map_err(domain_failure)?;
    let vacant_snapshot_digest = digest_bytes(vacant_checkpoint.snapshot_digest())?;
    let vacant_bytes_sha256 = sha256_bytes(&vacant_bytes);
    let row = transaction
        .query_one(
            procedure_profile.load_for_update_sql(),
            &[
                &project_id.as_str(),
                &vacant_bytes,
                &vacant_bytes_sha256,
                &vacant_snapshot_digest,
                &repository_command.command_id(),
            ],
        )
        .map_err(database_failure)?;
    let loaded = LoadedAggregate::from_locked_row(&row, project_id)?;
    verify_physical_history(transaction, project_id, &loaded.aggregate)?;
    if let Some(receipt) = exact_repository_retry(
        repository_command,
        &repository_request_bytes,
        &repository_request_sha256,
        &loaded,
    )? {
        return Ok(receipt);
    }
    loaded.assert_bound_daemon(bound_daemon_instance_id, bound_daemon_epoch)?;
    let command = bind_live_command(repository_command.clone(), &loaded, lease_ttl_seconds)?;
    let plan = plan_command(&loaded.aggregate, &command).map_err(domain_failure)?;
    let receipt = plan.receipt().clone();
    if plan.is_exact_retry() {
        return Ok(receipt);
    }
    let next = apply_plan(&loaded.aggregate, plan).map_err(domain_failure)?;
    persist_plan(
        transaction,
        &loaded,
        &next,
        &receipt,
        &repository_request_bytes,
        &repository_request_sha256,
    )?;
    Ok(receipt)
}

impl WriterLeaseRepository for PostgresWriterLease {
    fn execute(
        &mut self,
        command: WriterLeaseRepositoryCommand,
    ) -> Result<WriterLeaseCommandReceipt, WriterLeaseRepositoryError> {
        // Touch the immutable target identity on every operation so a future
        // reconnect implementation cannot silently switch databases.
        let _identity = (
            self.target.database_identity_digest(),
            self.target.global_manifest_digest(),
            self.target.memory_manifest_digest(),
        );
        for attempt in 0..=MAX_SERIALIZATION_RETRIES {
            match self.execute_once(&command) {
                Ok(receipt) => return Ok(receipt),
                Err(error) if error.retryable && attempt < MAX_SERIALIZATION_RETRIES => {}
                Err(error) if error.retryable => {
                    return Err(repository_error(
                        WriterLeaseRepositoryErrorKind::SerializationExhausted,
                    ));
                }
                Err(error) => return Err(error.error),
            }
        }
        Err(repository_error(
            WriterLeaseRepositoryErrorKind::SerializationExhausted,
        ))
    }

    fn current_authority(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<Option<WriterLeaseCurrentAuthority>, WriterLeaseRepositoryError> {
        Ok(self
            .inspect_project(project_id)?
            .and_then(|evidence| evidence.current_authority().cloned()))
    }

    fn assert_current(
        &mut self,
        expected: &WriterLeaseAuthorityHead,
    ) -> Result<(), WriterLeaseRepositoryError> {
        if expected.identity().daemon_instance_id() != self.bound_daemon_instance_id
            || expected.identity().daemon_epoch() != self.bound_daemon_epoch
        {
            return Err(authority_repository_error());
        }
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        enter_runtime_reader(&mut transaction).map_err(|failure| failure.error)?;
        let project_id = expected.identity().project_id();
        let loaded = Self::load_current_in(&mut transaction, project_id)
            .map_err(|failure| failure.error)?
            .ok_or_else(authority_repository_error)?;
        if loaded.aggregate.current_head().as_ref() != Some(expected) {
            return Err(authority_repository_error());
        }
        let identity = expected.identity();
        let receipt_digest =
            digest_bytes(expected.receipt_digest()).map_err(|failure| failure.error)?;
        let task_spec_digest =
            digest_bytes(identity.task_spec_digest()).map_err(|failure| failure.error)?;
        let holder_process_start_identity = digest_bytes(identity.holder_process_start_identity())
            .map_err(|failure| failure.error)?;
        let holder_process_id =
            to_i64(identity.holder_process_id().get()).map_err(|failure| failure.error)?;
        let daemon_epoch =
            to_i64(identity.daemon_epoch().get()).map_err(|failure| failure.error)?;
        let fencing_token =
            to_i64(identity.fencing_token().get()).map_err(|failure| failure.error)?;
        let asserted: bool = transaction
            .query_one(
                ASSERT_CURRENT_SQL,
                &[
                    &project_id.as_str(),
                    &identity.project_snapshot_id().as_str(),
                    &identity.task_id().as_str(),
                    &identity.task_revision(),
                    &task_spec_digest,
                    &identity.attempt_id().as_str(),
                    &identity.lease_id(),
                    &identity.lease_holder_id(),
                    &identity.worktree_id(),
                    &holder_process_id,
                    &holder_process_start_identity,
                    &identity.daemon_instance_id(),
                    &daemon_epoch,
                    &fencing_token,
                    &receipt_digest,
                ],
            )
            .and_then(|row| row.try_get(0))
            .map_err(|error| map_assert_error(&error))?;
        if !asserted {
            return Err(authority_repository_error());
        }
        transaction
            .commit()
            .map_err(|_| repository_error(WriterLeaseRepositoryErrorKind::Unavailable))?;
        Ok(())
    }
}

struct LoadedAggregate {
    row_version: i64,
    aggregate: VerifiedWriterLeaseAggregate,
    checkpoint: WriterLeaseCheckpoint,
    observed_at: Option<String>,
    time_observation_digest: Option<ContentDigest>,
    admission: Option<RuntimeAdmissionMode>,
    daemon_instance_id: Option<String>,
    daemon_epoch: Option<DaemonEpoch>,
    admission_observation_digest: Option<ContentDigest>,
    existing_repository_request_bytes: Option<Vec<u8>>,
    existing_repository_request_sha256: Option<Vec<u8>>,
}

impl LoadedAggregate {
    fn from_locked_row(row: &Row, project_id: &ProjectId) -> Result<Self, AdapterFailure> {
        let mut loaded = Self::from_row(row, 0, project_id)?;
        loaded.observed_at = Some(row_value(row, 24)?);
        loaded.time_observation_digest = Some(row_digest(row, 25)?);
        loaded.admission = Some(parse_admission(&row_value::<String>(row, 26)?)?);
        loaded.daemon_instance_id = row_value::<Option<String>>(row, 27)?;
        loaded.daemon_epoch = row_value::<Option<i64>>(row, 28)?
            .map(|value| {
                u64::try_from(value)
                    .ok()
                    .and_then(|value| DaemonEpoch::new(value).ok())
                    .ok_or_else(corrupt_failure)
            })
            .transpose()?;
        loaded.admission_observation_digest = row_optional_digest(row, 29)?;
        loaded.existing_repository_request_bytes = row_value(row, 30)?;
        loaded.existing_repository_request_sha256 = row_value(row, 31)?;
        match (
            loaded.existing_repository_request_bytes.as_ref(),
            loaded.existing_repository_request_sha256.as_ref(),
        ) {
            (None, None) => {}
            (Some(bytes), Some(digest)) if sha256_bytes(bytes) == *digest => {}
            _ => return Err(corrupt_failure()),
        }
        Ok(loaded)
    }

    fn from_current_row(row: &Row, project_id: &ProjectId) -> Result<Self, AdapterFailure> {
        Self::from_row(row, 0, project_id)
    }

    fn from_row(
        row: &Row,
        offset: usize,
        expected_project_id: &ProjectId,
    ) -> Result<Self, AdapterFailure> {
        let row_version: i64 = row_value(row, offset)?;
        let snapshot_bytes: Vec<u8> = row_value(row, offset + 1)?;
        let snapshot_bytes_sha256: Vec<u8> = row_value(row, offset + 2)?;
        if snapshot_bytes_sha256 != sha256_bytes(&snapshot_bytes) {
            return Err(corrupt_failure());
        }
        let snapshot_digest = row_digest(row, offset + 3)?;
        let fencing_high_water = nonnegative_u64(row_value(row, offset + 4)?)?;
        let lease_revision = nonnegative_u64(row_value(row, offset + 5)?)?;
        let command_high_water = nonnegative_u64(row_value(row, offset + 6)?)?;
        let command_tail_digest = row_optional_digest(row, offset + 7)?;
        let snapshot = UntrustedWriterLeaseSnapshot::from_canonical_bytes(&snapshot_bytes)
            .map_err(domain_failure)?;
        let checkpoint = WriterLeaseCheckpoint::new(
            expected_project_id.clone(),
            command_high_water,
            command_tail_digest,
            snapshot_digest,
        )
        .map_err(domain_failure)?;
        let aggregate =
            verify_snapshot_against_checkpoint(&snapshot, &checkpoint).map_err(domain_failure)?;
        if aggregate.project_id() != expected_project_id
            || aggregate.fencing_high_water() != fencing_high_water
            || aggregate.revision() != lease_revision
        {
            return Err(corrupt_failure());
        }
        verify_current_projection(row, offset + 8, &aggregate)?;
        Ok(Self {
            row_version,
            aggregate,
            checkpoint,
            observed_at: None,
            time_observation_digest: None,
            admission: None,
            daemon_instance_id: None,
            daemon_epoch: None,
            admission_observation_digest: None,
            existing_repository_request_bytes: None,
            existing_repository_request_sha256: None,
        })
    }

    fn observation(&self) -> Result<LeaseObservation, AdapterFailure> {
        Ok(LeaseObservation {
            runtime: RuntimeKind::Live,
            admission: self.admission.ok_or_else(authority_failure)?,
            observed_at: self.observed_at.clone().ok_or_else(authority_failure)?,
            time_observation_digest: self
                .time_observation_digest
                .clone()
                .ok_or_else(authority_failure)?,
            admission_observation_digest: self
                .admission_observation_digest
                .clone()
                .ok_or_else(authority_failure)?,
        })
    }

    fn assert_bound_daemon(
        &self,
        daemon_instance_id: &str,
        daemon_epoch: DaemonEpoch,
    ) -> Result<(), AdapterFailure> {
        if self.daemon_instance_id.as_deref() != Some(daemon_instance_id)
            || self.daemon_epoch != Some(daemon_epoch)
        {
            return Err(authority_failure());
        }
        Ok(())
    }
}

fn exact_repository_retry(
    request: &WriterLeaseRepositoryCommand,
    repository_request_bytes: &[u8],
    repository_request_sha256: &[u8],
    loaded: &LoadedAggregate,
) -> Result<Option<WriterLeaseCommandReceipt>, AdapterFailure> {
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
        (Some(receipt), Some(existing_bytes), Some(existing_sha256)) => {
            if existing_bytes != repository_request_bytes
                || existing_sha256 != repository_request_sha256
            {
                return Err(domain_failure(
                    lattice_writer_lease::WriterLeaseError::CommandIdReuse,
                ));
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

fn bind_live_command(
    request: WriterLeaseRepositoryCommand,
    loaded: &LoadedAggregate,
    lease_ttl_seconds: u32,
) -> Result<WriterLeaseCommand, AdapterFailure> {
    let observation = loaded.observation()?;
    let daemon_instance_id = loaded
        .daemon_instance_id
        .clone()
        .ok_or_else(authority_failure)?;
    let daemon_epoch = loaded.daemon_epoch.ok_or_else(authority_failure)?;
    let expiry = expiry_after(&observation.observed_at, lease_ttl_seconds)?;
    Ok(match request {
        WriterLeaseRepositoryCommand::Acquire(request) => {
            WriterLeaseCommand::Acquire(AcquireCommand {
                command_id: request.command_id,
                expected_head: request.expected_head,
                claim: AcquireClaim {
                    project_id: request.project_id,
                    project_snapshot_id: request.project_snapshot_id,
                    task_id: request.task_id,
                    task_revision: request.task_revision,
                    task_spec_digest: request.task_spec_digest,
                    attempt_id: request.attempt_id,
                    lease_id: request.lease_id,
                    lease_holder_id: request.lease_holder_id,
                    worktree_id: request.worktree_id,
                    holder_process_id: request.holder_process_id,
                    holder_process_start_identity: request.holder_process_start_identity,
                    daemon_instance_id,
                    daemon_epoch,
                },
                observation,
                expires_at: expiry,
            })
        }
        WriterLeaseRepositoryCommand::Heartbeat(request) => {
            WriterLeaseCommand::Heartbeat(HeartbeatCommand {
                command_id: request.command_id,
                project_id: request.project_id,
                expected_head: request.expected_head,
                observation,
                expires_at: expiry,
            })
        }
        WriterLeaseRepositoryCommand::MarkSuspect(request) => {
            WriterLeaseCommand::MarkSuspect(MarkSuspectCommand {
                command_id: request.command_id,
                project_id: request.project_id,
                expected_head: request.expected_head,
                observation,
            })
        }
        WriterLeaseRepositoryCommand::ProcessHandoff(request) => {
            WriterLeaseCommand::ProcessHandoff(ProcessHandoffCommand {
                command_id: request.command_id,
                project_id: request.project_id,
                expected_head: request.expected_head,
                successor_holder_process_id: request.successor_holder_process_id,
                successor_holder_process_start_identity: request
                    .successor_holder_process_start_identity,
                successor_daemon_instance_id: daemon_instance_id,
                successor_daemon_epoch: daemon_epoch,
                observation,
                expires_at: expiry,
                evidence: request.evidence,
            })
        }
        WriterLeaseRepositoryCommand::Release(request) => {
            WriterLeaseCommand::Release(ReleaseCommand {
                command_id: request.command_id,
                project_id: request.project_id,
                expected_head: request.expected_head,
                observation,
            })
        }
        WriterLeaseRepositoryCommand::Revoke(request) => {
            WriterLeaseCommand::Revoke(RevokeCommand {
                command_id: request.command_id,
                project_id: request.project_id,
                expected_head: request.expected_head,
                observation,
                evidence: request.evidence,
            })
        }
    })
}

#[allow(clippy::too_many_lines)]
fn persist_plan(
    transaction: &mut Transaction<'_>,
    loaded: &LoadedAggregate,
    next: &VerifiedWriterLeaseAggregate,
    receipt: &WriterLeaseCommandReceipt,
    repository_request_bytes: &[u8],
    repository_request_sha256: &[u8],
) -> Result<(), AdapterFailure> {
    let snapshot = next.export_untrusted();
    let next_snapshot_bytes = snapshot.canonical_bytes().map_err(domain_failure)?;
    let next_snapshot_bytes_sha256 = sha256_bytes(&next_snapshot_bytes);
    let next_checkpoint = next.checkpoint().map_err(domain_failure)?;
    let expected_snapshot_digest = digest_bytes(loaded.checkpoint.snapshot_digest())?;
    let expected_tail = loaded
        .checkpoint
        .command_tail_digest()
        .map(digest_bytes)
        .transpose()?;
    let next_snapshot_digest = digest_bytes(next_checkpoint.snapshot_digest())?;
    let next_tail = next_checkpoint
        .command_tail_digest()
        .map(digest_bytes)
        .transpose()?;
    let receipt_bytes = receipt.canonical_bytes().map_err(domain_failure)?;
    let request_bytes = receipt.request.canonical_bytes().map_err(domain_failure)?;
    let transition = match receipt.transition_digest.as_ref() {
        None => None,
        Some(expected_digest) => {
            let transition = next.transitions().last().ok_or_else(corrupt_failure)?;
            if transition.ordinal != receipt.ordinal
                || transition.command_id != receipt.request.command_id()
                || &transition.transition_digest != expected_digest
            {
                return Err(corrupt_failure());
            }
            Some(transition)
        }
    };
    let transition_bytes = transition
        .map(|value| value.canonical_bytes().map_err(domain_failure))
        .transpose()?;
    let transition_kind = transition.map(|value| value.kind.as_str());

    let (outcome, denial_reason) = match receipt.outcome {
        CommandOutcome::Applied => ("APPLIED", None),
        CommandOutcome::Denied(denial) => ("DENIED", Some(denial.as_str())),
    };
    let current = next.current_receipt();
    let current_status = current.map(|value| value.status().as_str());
    let current_receipt_digest = current
        .map(|value| digest_bytes(value.receipt_digest()))
        .transpose()?;
    let current_project_snapshot_id =
        current.map(|value| value.identity().project_snapshot_id().as_str());
    let current_task_id = current.map(|value| value.identity().task_id().as_str());
    let current_task_revision = current.map(|value| value.identity().task_revision());
    let current_task_spec_digest = current
        .map(|value| digest_bytes(value.identity().task_spec_digest()))
        .transpose()?;
    let current_attempt_id = current.map(|value| value.identity().attempt_id().as_str());
    let current_lease_id = current.map(|value| value.identity().lease_id());
    let current_lease_holder_id = current.map(|value| value.identity().lease_holder_id());
    let current_worktree_id = current.map(|value| value.identity().worktree_id());
    let current_holder_process_id = current
        .map(|value| to_i64(value.identity().holder_process_id().get()))
        .transpose()?;
    let current_holder_process_start_identity = current
        .map(|value| digest_bytes(value.identity().holder_process_start_identity()))
        .transpose()?;
    let current_daemon_instance_id = current.map(|value| value.identity().daemon_instance_id());
    let current_daemon_epoch = current
        .map(|value| to_i64(value.identity().daemon_epoch().get()))
        .transpose()?;
    let current_fencing_token = current
        .map(|value| to_i64(value.identity().fencing_token().get()))
        .transpose()?;
    let current_expires_at =
        current.map(lattice_contracts::WriterLeaseAuthorityReceipt::expires_at);
    let request_digest = digest_bytes(&receipt.request_digest)?;
    let previous_receipt_digest = receipt
        .previous_receipt_digest
        .as_ref()
        .map(digest_bytes)
        .transpose()?;
    let transition_digest = receipt
        .transition_digest
        .as_ref()
        .map(digest_bytes)
        .transpose()?;
    let receipt_digest = digest_bytes(&receipt.receipt_digest)?;
    let observed_at = loaded
        .observed_at
        .as_deref()
        .ok_or_else(authority_failure)?;
    let time_observation_digest = digest_bytes(
        loaded
            .time_observation_digest
            .as_ref()
            .ok_or_else(authority_failure)?,
    )?;
    let admission = loaded.admission.ok_or_else(authority_failure)?;
    let daemon_instance_id = loaded
        .daemon_instance_id
        .as_deref()
        .ok_or_else(authority_failure)?;
    let daemon_epoch = to_i64(loaded.daemon_epoch.ok_or_else(authority_failure)?.get())?;
    let admission_observation_digest = digest_bytes(
        loaded
            .admission_observation_digest
            .as_ref()
            .ok_or_else(authority_failure)?,
    )?;

    let decision: String = transaction
        .query_one(
            COMMIT_PLAN_SQL,
            &[
                &next.project_id().as_str(),
                &loaded.row_version,
                &expected_snapshot_digest,
                &to_i64(loaded.checkpoint.command_high_water())?,
                &expected_tail,
                &observed_at,
                &time_observation_digest,
                &admission.as_str(),
                &daemon_instance_id,
                &daemon_epoch,
                &admission_observation_digest,
                &next_snapshot_bytes,
                &next_snapshot_bytes_sha256,
                &next_snapshot_digest,
                &to_i64(next.fencing_high_water())?,
                &to_i64(next.revision())?,
                &to_i64(next_checkpoint.command_high_water())?,
                &next_tail,
                &current_status,
                &current_receipt_digest,
                &current_project_snapshot_id,
                &current_task_id,
                &current_task_revision,
                &current_task_spec_digest,
                &current_attempt_id,
                &current_lease_id,
                &current_lease_holder_id,
                &current_worktree_id,
                &current_holder_process_id,
                &current_holder_process_start_identity,
                &current_daemon_instance_id,
                &current_daemon_epoch,
                &current_fencing_token,
                &current_expires_at,
                &to_i64(receipt.ordinal)?,
                &receipt.request.command_id(),
                &repository_request_bytes,
                &repository_request_sha256,
                &request_bytes,
                &request_digest,
                &previous_receipt_digest,
                &outcome,
                &denial_reason,
                &transition_digest,
                &receipt_bytes,
                &receipt_digest,
                &transition_kind,
                &transition_bytes,
            ],
        )
        .and_then(|row| row.try_get(0))
        .map_err(database_failure)?;
    if decision != "APPLIED" {
        return Err(AdapterFailure {
            error: authority_repository_error(),
            retryable: decision == "STALE",
        });
    }
    Ok(())
}

fn verify_current_projection(
    row: &Row,
    offset: usize,
    aggregate: &VerifiedWriterLeaseAggregate,
) -> Result<(), AdapterFailure> {
    let status: Option<String> = row_value(row, offset)?;
    let receipt_digest = row_optional_digest(row, offset + 1)?;
    let project_snapshot_id: Option<String> = row_value(row, offset + 2)?;
    let task_id: Option<String> = row_value(row, offset + 3)?;
    let task_revision: Option<String> = row_value(row, offset + 4)?;
    let task_spec_digest = row_optional_digest(row, offset + 5)?;
    let attempt_id: Option<String> = row_value(row, offset + 6)?;
    let lease_id: Option<String> = row_value(row, offset + 7)?;
    let lease_holder_id: Option<String> = row_value(row, offset + 8)?;
    let worktree_id: Option<String> = row_value(row, offset + 9)?;
    let holder_process_id: Option<i64> = row_value(row, offset + 10)?;
    let holder_process_start_identity = row_optional_digest(row, offset + 11)?;
    let daemon_instance_id: Option<String> = row_value(row, offset + 12)?;
    let daemon_epoch: Option<i64> = row_value(row, offset + 13)?;
    let fencing_token: Option<i64> = row_value(row, offset + 14)?;
    let expires_at: Option<String> = row_value(row, offset + 15)?;
    match aggregate.current_receipt() {
        None => {
            if status.is_some()
                || receipt_digest.is_some()
                || project_snapshot_id.is_some()
                || task_id.is_some()
                || task_revision.is_some()
                || task_spec_digest.is_some()
                || attempt_id.is_some()
                || lease_id.is_some()
                || lease_holder_id.is_some()
                || worktree_id.is_some()
                || holder_process_id.is_some()
                || holder_process_start_identity.is_some()
                || daemon_instance_id.is_some()
                || daemon_epoch.is_some()
                || fencing_token.is_some()
                || expires_at.is_some()
            {
                return Err(corrupt_failure());
            }
        }
        Some(receipt) => {
            let identity = receipt.identity();
            if status.as_deref() != Some(receipt.status().as_str())
                || receipt_digest.as_ref() != Some(receipt.receipt_digest())
                || project_snapshot_id.as_deref() != Some(identity.project_snapshot_id().as_str())
                || task_id.as_deref() != Some(identity.task_id().as_str())
                || task_revision.as_deref() != Some(identity.task_revision())
                || task_spec_digest.as_ref() != Some(identity.task_spec_digest())
                || attempt_id.as_deref() != Some(identity.attempt_id().as_str())
                || lease_id.as_deref() != Some(identity.lease_id())
                || lease_holder_id.as_deref() != Some(identity.lease_holder_id())
                || worktree_id.as_deref() != Some(identity.worktree_id())
                || holder_process_id != Some(to_i64(identity.holder_process_id().get())?)
                || holder_process_start_identity.as_ref()
                    != Some(identity.holder_process_start_identity())
                || daemon_instance_id.as_deref() != Some(identity.daemon_instance_id())
                || daemon_epoch != Some(to_i64(identity.daemon_epoch().get())?)
                || fencing_token != Some(to_i64(identity.fencing_token().get())?)
                || expires_at.as_deref() != Some(receipt.expires_at())
            {
                return Err(corrupt_failure());
            }
        }
    }
    Ok(())
}

fn verify_physical_history(
    transaction: &mut Transaction<'_>,
    project_id: &ProjectId,
    aggregate: &VerifiedWriterLeaseAggregate,
) -> Result<(), AdapterFailure> {
    let command_rows = transaction
        .query(LOAD_COMMANDS_SQL, &[&project_id.as_str()])
        .map_err(database_failure)?;
    let receipts = aggregate.command_receipts();
    if command_rows.len() != receipts.len() {
        return Err(corrupt_failure());
    }
    for (row, receipt) in command_rows.iter().zip(receipts) {
        let ordinal = nonnegative_u64(row_value::<i64>(row, 0)?)?;
        let command_id: String = row_value(row, 1)?;
        let repository_request_bytes: Vec<u8> = row_value(row, 2)?;
        let repository_request_sha256: Vec<u8> = row_value(row, 3)?;
        let request_bytes: Vec<u8> = row_value(row, 4)?;
        let request_digest = row_digest(row, 5)?;
        let previous_receipt_digest = row_optional_digest(row, 6)?;
        let outcome: String = row_value(row, 7)?;
        let denial_reason: Option<String> = row_value(row, 8)?;
        let transition_digest = row_optional_digest(row, 9)?;
        let receipt_bytes: Vec<u8> = row_value(row, 10)?;
        let receipt_digest = row_digest(row, 11)?;

        let expected_repository_request_bytes = receipt
            .request
            .repository_intent_canonical_bytes()
            .map_err(domain_failure)?;
        let expected_request_bytes = receipt.request.canonical_bytes().map_err(domain_failure)?;
        let expected_receipt_bytes = receipt.canonical_bytes().map_err(domain_failure)?;
        let (expected_outcome, expected_denial) = match receipt.outcome {
            CommandOutcome::Applied => ("APPLIED", None),
            CommandOutcome::Denied(denial) => ("DENIED", Some(denial.as_str())),
        };
        if ordinal != receipt.ordinal
            || command_id != receipt.request.command_id()
            || repository_request_sha256 != sha256_bytes(&repository_request_bytes)
            || repository_request_bytes != expected_repository_request_bytes
            || request_bytes != expected_request_bytes
            || request_digest != receipt.request_digest
            || previous_receipt_digest != receipt.previous_receipt_digest
            || outcome != expected_outcome
            || denial_reason.as_deref() != expected_denial
            || transition_digest != receipt.transition_digest
            || receipt_bytes != expected_receipt_bytes
            || receipt_digest != receipt.receipt_digest
        {
            return Err(corrupt_failure());
        }
    }

    let transition_rows = transaction
        .query(LOAD_TRANSITIONS_SQL, &[&project_id.as_str()])
        .map_err(database_failure)?;
    let transitions = aggregate.transitions();
    if transition_rows.len() != transitions.len() {
        return Err(corrupt_failure());
    }
    for (row, transition) in transition_rows.iter().zip(transitions) {
        let ordinal = nonnegative_u64(row_value::<i64>(row, 0)?)?;
        let command_id: String = row_value(row, 1)?;
        let transition_kind: String = row_value(row, 2)?;
        let transition_bytes: Vec<u8> = row_value(row, 3)?;
        let transition_digest = row_digest(row, 4)?;
        let expected_transition_bytes = transition.canonical_bytes().map_err(domain_failure)?;
        if ordinal != transition.ordinal
            || command_id != transition.command_id
            || transition_kind != transition.kind.as_str()
            || transition_bytes != expected_transition_bytes
            || transition_digest != transition.transition_digest
        {
            return Err(corrupt_failure());
        }
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

fn nonnegative_u64(value: i64) -> Result<u64, AdapterFailure> {
    u64::try_from(value).map_err(|_| corrupt_failure())
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
                          30000::bigint)::text END, true);",
        )
        .map_err(database_failure)
}

fn enter_runtime_reader(transaction: &mut Transaction<'_>) -> Result<(), AdapterFailure> {
    transaction
        .batch_execute(
            "SET LOCAL ROLE lattice_runtime; \
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
                          30000::bigint)::text END, true);",
        )
        .map_err(database_failure)
}

#[derive(Clone, Copy, Debug)]
struct AdapterFailure {
    error: WriterLeaseRepositoryError,
    retryable: bool,
}

fn repository_error(kind: WriterLeaseRepositoryErrorKind) -> WriterLeaseRepositoryError {
    WriterLeaseRepositoryError::new(kind)
}

fn corrupt_repository_error() -> WriterLeaseRepositoryError {
    repository_error(WriterLeaseRepositoryErrorKind::Corrupt)
}

fn authority_repository_error() -> WriterLeaseRepositoryError {
    repository_error(WriterLeaseRepositoryErrorKind::AuthorityMismatch)
}

fn corrupt_failure() -> AdapterFailure {
    AdapterFailure {
        error: corrupt_repository_error(),
        retryable: false,
    }
}

fn authority_failure() -> AdapterFailure {
    AdapterFailure {
        error: authority_repository_error(),
        retryable: false,
    }
}

fn domain_failure(error: lattice_writer_lease::WriterLeaseError) -> AdapterFailure {
    AdapterFailure {
        error: WriterLeaseRepositoryError::from_domain(error),
        retryable: false,
    }
}

fn validate_historical_receipt_digest(
    receipt_digest: &ContentDigest,
) -> Result<(), WriterLeaseRepositoryError> {
    if receipt_digest.as_str().bytes().all(|byte| byte == b'0') {
        return Err(WriterLeaseRepositoryError::from_domain(
            lattice_writer_lease::WriterLeaseError::ZeroEvidenceDigest,
        ));
    }
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn database_failure(error: postgres::Error) -> AdapterFailure {
    let retryable = error.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE);
    let kind = if retryable {
        WriterLeaseRepositoryErrorKind::Unavailable
    } else if matches!(error.code().map(SqlState::code), Some("LWL02" | "LWL04")) {
        WriterLeaseRepositoryErrorKind::Corrupt
    } else if matches!(
        error.code().map(SqlState::code),
        Some("LWL03" | "LWL05" | "LWL06")
    ) {
        WriterLeaseRepositoryErrorKind::AuthorityMismatch
    } else {
        WriterLeaseRepositoryErrorKind::Unavailable
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
            error: repository_error(WriterLeaseRepositoryErrorKind::CommitOutcomeUnknown),
            retryable: false,
        }
    }
}

fn map_assert_error(error: &postgres::Error) -> WriterLeaseRepositoryError {
    if error.code().map(SqlState::code) == Some("LWL05") {
        authority_repository_error()
    } else {
        repository_error(WriterLeaseRepositoryErrorKind::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_transactions_never_extend_a_shorter_session_deadline() {
        let source = include_str!("adapter.rs");
        for function in ["enter_runtime_writer", "enter_runtime_reader"] {
            let settings = source
                .split(&format!("fn {function}("))
                .nth(1)
                .expect("runtime transaction entry")
                .split(".map_err(database_failure)")
                .next()
                .expect("transaction settings boundary");
            assert!(settings.contains("pg_catalog.set_config('lock_timeout'"));
            assert!(settings.contains("pg_catalog.set_config('statement_timeout'"));
            assert!(settings.contains("LEAST("));
            assert!(!settings.contains("SET LOCAL lock_timeout = '5s'"));
            assert!(!settings.contains("SET LOCAL statement_timeout = '30s'"));
        }
    }

    #[test]
    fn historical_authority_rejects_zero_digest_before_storage_lookup() {
        let zero = ContentDigest::from_sha256("0".repeat(64)).expect("shape-valid zero digest");
        let error = validate_historical_receipt_digest(&zero)
            .expect_err("zero historical receipt digest must fail before any database access");

        assert_eq!(error.kind(), WriterLeaseRepositoryErrorKind::Domain);
        assert_eq!(
            error.domain(),
            Some(lattice_writer_lease::WriterLeaseError::ZeroEvidenceDigest)
        );
    }
}
