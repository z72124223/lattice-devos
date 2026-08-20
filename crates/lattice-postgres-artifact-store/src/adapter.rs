use lattice_artifact_store::{
    ArtifactRepository, ArtifactRepositoryError, ArtifactRepositoryErrorKind,
    ArtifactRepositorySnapshot, ArtifactStoreIdentity,
};
use lattice_contracts::ContentDigest;
use postgres::error::SqlState;
use postgres::{Client, IsolationLevel, Row, Transaction};

use crate::{ExtensionTarget, digest_bytes, sha256_bytes, verify_embedded_extension_manifest};

const MAX_SERIALIZATION_RETRIES: usize = 3;
const LOAD_FOR_UPDATE_SQL: &str =
    "SELECT * FROM artifact_store.artifact_store_load_for_update_v1($1,$2,$3,$4,$5,$6)";
const COMMIT_SQL: &str = "SELECT artifact_store.artifact_store_commit_snapshot_v1(\
     $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)";
const LOAD_CURRENT_SQL: &str =
    "SELECT * FROM artifact_store.artifact_store_load_current_v1($1,$2,$3,$4,$5,$6)";

/// Live `PostgreSQL` implementation of Artifact Store's metadata repository.
/// It supplies durability only and does not authorize live provenance or byte
/// effects on behalf of external owners.
pub struct PostgresArtifactStore {
    client: Client,
    target: ExtensionTarget,
}

impl PostgresArtifactStore {
    /// Constructs an adapter around an already provisioned runtime
    /// connection. DSN and credentials never enter repository inputs.
    ///
    /// # Errors
    ///
    /// Rejects an unavailable connection or database-name substitution.
    pub fn new(
        mut client: Client,
        target: ExtensionTarget,
    ) -> Result<Self, ArtifactRepositoryError> {
        let database: String = client
            .query_one("SELECT pg_catalog.current_database()::text", &[])
            .and_then(|row| row.try_get(0))
            .map_err(|_| repository_error(ArtifactRepositoryErrorKind::Unavailable))?;
        if database != target.database_name() {
            return Err(repository_error(
                ArtifactRepositoryErrorKind::AuthorityMismatch,
            ));
        }
        verify_embedded_extension_manifest()
            .map_err(|_| repository_error(ArtifactRepositoryErrorKind::Corrupt))?;
        Ok(Self { client, target })
    }

    fn load_once(
        &mut self,
        store_id: &ArtifactStoreIdentity,
    ) -> Result<Option<ArtifactRepositorySnapshot>, AdapterFailure> {
        let manifest = verify_embedded_extension_manifest().map_err(|_| corrupt_failure())?;
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(database_failure)?;
        enter_reader(&mut transaction)?;
        let store_value = store_id.as_str();
        let database_name = self.target.database_name();
        let database_identity = self.target.database_identity_digest().as_str();
        let global_manifest = self.target.global_manifest_digest().as_str();
        let memory_manifest = self.target.memory_manifest_digest().as_str();
        let extension_manifest = manifest.manifest_sha256().as_str();
        let rows = transaction
            .query(
                LOAD_CURRENT_SQL,
                &[
                    &store_value,
                    &database_name,
                    &database_identity,
                    &global_manifest,
                    &memory_manifest,
                    &extension_manifest,
                ],
            )
            .map_err(database_failure)?;
        let result = match rows.as_slice() {
            [] => None,
            [row] => Some(snapshot_from_row(row)?),
            _ => return Err(corrupt_failure()),
        };
        transaction.commit().map_err(database_failure)?;
        Ok(result)
    }

    fn compare_once(
        &mut self,
        expected: &ContentDigest,
        next: &ArtifactRepositorySnapshot,
    ) -> Result<ArtifactRepositorySnapshot, AdapterFailure> {
        next.replay().map_err(|_| domain_failure())?;
        let manifest = verify_embedded_extension_manifest().map_err(|_| corrupt_failure())?;
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(database_failure)?;
        enter_writer(&mut transaction)?;
        let store_value = next.store_id().as_str();
        let database_name = self.target.database_name();
        let database_identity = self.target.database_identity_digest().as_str();
        let global_manifest = self.target.global_manifest_digest().as_str();
        let memory_manifest = self.target.memory_manifest_digest().as_str();
        let extension_manifest = manifest.manifest_sha256().as_str();
        let rows = transaction
            .query(
                LOAD_FOR_UPDATE_SQL,
                &[
                    &store_value,
                    &database_name,
                    &database_identity,
                    &global_manifest,
                    &memory_manifest,
                    &extension_manifest,
                ],
            )
            .map_err(database_failure)?;
        match rows.as_slice() {
            [] if expected != next.checkpoint_digest() => return Err(stale_failure()),
            [] => next.verify_initial().map_err(|_| domain_failure())?,
            [row] => {
                let current = snapshot_from_row(row)?;
                if current.checkpoint_digest() == next.checkpoint_digest() {
                    transaction.commit().map_err(commit_failure)?;
                    return Ok(current);
                }
                if current.checkpoint_digest() != expected || current.store_id() != next.store_id()
                {
                    return Err(stale_failure());
                }
                current
                    .verify_successor(next)
                    .map_err(|_| domain_failure())?;
            }
            _ => return Err(corrupt_failure()),
        }
        let snapshot_sha = sha256_bytes(next.snapshot_bytes());
        let checkpoint_sha = sha256_bytes(next.checkpoint_bytes());
        let expected_bytes = digest_bytes(expected).map_err(|_| corrupt_failure())?;
        let next_digest = digest_bytes(next.checkpoint_digest()).map_err(|_| corrupt_failure())?;
        let status: String = transaction
            .query_one(
                COMMIT_SQL,
                &[
                    &store_value,
                    &expected_bytes,
                    &next.snapshot_bytes(),
                    &snapshot_sha,
                    &next.checkpoint_bytes(),
                    &checkpoint_sha,
                    &next_digest,
                    &database_name,
                    &database_identity,
                    &global_manifest,
                    &memory_manifest,
                    &extension_manifest,
                ],
            )
            .and_then(|row| row.try_get(0))
            .map_err(database_failure)?;
        if status != "COMMITTED" && status != "RETRY" {
            return Err(corrupt_failure());
        }
        transaction.commit().map_err(commit_failure)?;
        Ok(next.clone())
    }
}

impl ArtifactRepository for PostgresArtifactStore {
    fn load(
        &mut self,
        store_id: &ArtifactStoreIdentity,
    ) -> Result<Option<ArtifactRepositorySnapshot>, ArtifactRepositoryError> {
        self.load_once(store_id).map_err(|failure| failure.error)
    }

    fn compare_and_swap(
        &mut self,
        expected_checkpoint_digest: &ContentDigest,
        next: &ArtifactRepositorySnapshot,
    ) -> Result<ArtifactRepositorySnapshot, ArtifactRepositoryError> {
        for attempt in 0..=MAX_SERIALIZATION_RETRIES {
            match self.compare_once(expected_checkpoint_digest, next) {
                Ok(snapshot) => return Ok(snapshot),
                Err(failure) if failure.retryable && attempt < MAX_SERIALIZATION_RETRIES => {}
                Err(failure) if failure.retryable => {
                    return Err(repository_error(
                        ArtifactRepositoryErrorKind::SerializationExhausted,
                    ));
                }
                Err(failure) => return Err(failure.error),
            }
        }
        Err(repository_error(
            ArtifactRepositoryErrorKind::SerializationExhausted,
        ))
    }
}

fn snapshot_from_row(row: &Row) -> Result<ArtifactRepositorySnapshot, AdapterFailure> {
    let snapshot_bytes: Vec<u8> = row.try_get(1).map_err(|_| corrupt_failure())?;
    let snapshot_sha: Vec<u8> = row.try_get(2).map_err(|_| corrupt_failure())?;
    let checkpoint_bytes: Vec<u8> = row.try_get(3).map_err(|_| corrupt_failure())?;
    let checkpoint_sha: Vec<u8> = row.try_get(4).map_err(|_| corrupt_failure())?;
    let checkpoint_digest: Vec<u8> = row.try_get(5).map_err(|_| corrupt_failure())?;
    if sha256_bytes(&snapshot_bytes) != snapshot_sha
        || sha256_bytes(&checkpoint_bytes) != checkpoint_sha
    {
        return Err(corrupt_failure());
    }
    let snapshot =
        ArtifactRepositorySnapshot::from_canonical_bytes(&snapshot_bytes, &checkpoint_bytes)
            .map_err(|_| corrupt_failure())?;
    if digest_bytes(snapshot.checkpoint_digest()).map_err(|_| corrupt_failure())?
        != checkpoint_digest
    {
        return Err(corrupt_failure());
    }
    Ok(snapshot)
}

fn enter_writer(transaction: &mut Transaction<'_>) -> Result<(), AdapterFailure> {
    transaction
        .batch_execute("SET LOCAL ROLE lattice_runtime; SET LOCAL synchronous_commit = on")
        .map_err(authority_failure)
}

fn enter_reader(transaction: &mut Transaction<'_>) -> Result<(), AdapterFailure> {
    transaction
        .batch_execute("SET LOCAL ROLE lattice_runtime")
        .map_err(authority_failure)
}

struct AdapterFailure {
    error: ArtifactRepositoryError,
    retryable: bool,
}

const fn repository_error(kind: ArtifactRepositoryErrorKind) -> ArtifactRepositoryError {
    ArtifactRepositoryError::new(kind)
}

const fn domain_failure() -> AdapterFailure {
    AdapterFailure {
        error: repository_error(ArtifactRepositoryErrorKind::Domain),
        retryable: false,
    }
}

const fn corrupt_failure() -> AdapterFailure {
    AdapterFailure {
        error: repository_error(ArtifactRepositoryErrorKind::Corrupt),
        retryable: false,
    }
}

const fn stale_failure() -> AdapterFailure {
    AdapterFailure {
        error: repository_error(ArtifactRepositoryErrorKind::StaleWrite),
        retryable: false,
    }
}

fn authority_failure(_error: postgres::Error) -> AdapterFailure {
    AdapterFailure {
        error: repository_error(ArtifactRepositoryErrorKind::AuthorityMismatch),
        retryable: false,
    }
}

#[allow(clippy::needless_pass_by_value)]
fn database_failure(error: postgres::Error) -> AdapterFailure {
    let retryable = error.code().is_some_and(|code| {
        code == &SqlState::T_R_SERIALIZATION_FAILURE || code == &SqlState::T_R_DEADLOCK_DETECTED
    });
    let kind =
        error.code().map_or(
            ArtifactRepositoryErrorKind::Unavailable,
            |code| match code.code() {
                "LAS04" => ArtifactRepositoryErrorKind::StaleWrite,
                "LAS05" => ArtifactRepositoryErrorKind::AuthorityMismatch,
                "LAS01" | "LAS03" => ArtifactRepositoryErrorKind::Corrupt,
                _ => ArtifactRepositoryErrorKind::Unavailable,
            },
        );
    AdapterFailure {
        error: repository_error(kind),
        retryable,
    }
}

fn commit_failure(_error: postgres::Error) -> AdapterFailure {
    AdapterFailure {
        error: repository_error(ArtifactRepositoryErrorKind::CommitOutcomeUnknown),
        retryable: false,
    }
}
