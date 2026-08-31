use std::error::Error;
use std::fmt;
use std::time::{Duration, Instant};

use lattice_contracts::ContentDigest;
use postgres::{Client, GenericClient, IsolationLevel};
use sha2::{Digest, Sha256};

use crate::{
    ExtensionManifestEvidence, FOREMAN_EXTENSION_ID, FOREMAN_EXTENSION_PATH,
    FOREMAN_EXTENSION_SCHEMA_VERSION, REQUIRED_GLOBAL_MANIFEST_SHA256,
    REQUIRED_GLOBAL_SCHEMA_VERSION, StoreV8RebindEvidence, verify_embedded_extension,
    verify_embedded_store_v8_rebind,
};

const DATABASE_IDENTITY_DOMAIN: &[u8] = b"LATTICE_POSTGRES_DATABASE_IDENTITY_V1\0";
const STORE_V8_GLOBAL_SCHEMA_VERSION: i32 = 8;
const STORE_V8_GLOBAL_MANIFEST_SHA256: &str =
    "2b1fcbbc81261c28ab06ac3180f75c2ee458e57a4adc7e49bc399209f421de60";
const GLOBAL_MIGRATION_ADVISORY_LOCK: i64 = 0x4c41_5454_4943_4501;
const EXTENSION_ADVISORY_LOCK: i64 = 7_212_400_260_826;
const GLOBAL_APPLY_GATE_TIMEOUT: Duration = Duration::from_secs(30);
const GLOBAL_APPLY_GATE_POLL_INTERVAL: Duration = Duration::from_millis(20);
const EXPECTED_TABLE_COUNT: i64 = 17;
const EXPECTED_FUNCTION_COUNT: i64 = 43;
const EXPECTED_RUNTIME_FUNCTION_COUNT: i64 = 39;
const EXPECTED_INTERNAL_TRIGGER_COUNT: i64 = 92;
const EXPECTED_CONTROL_INTERNAL_TRIGGER_COUNT: i64 = 12;
const EXPECTED_TYPE_COUNT: i64 = 34;
const FUNCTION_CATALOG_DOMAIN: &[u8] = b"LATTICE_POSTGRES_FOREMAN_FUNCTION_CATALOG_V1\0";
const TABLE_CATALOG_DOMAIN: &[u8] = b"LATTICE_POSTGRES_FOREMAN_TABLE_CATALOG_V2\0";
const EXPECTED_FUNCTION_CATALOG_SHA256: &str =
    "8d8dd263498cab48b1164bf456f5d3b314d575ee9a186460715beea02bc8bfec";
const EXPECTED_TABLE_CATALOG_SHA256: &str =
    "42f151dd9f52ba1e82a2aac392234f2b285c18e9bd71a00372f7c7b4a1237eb5";
const EXPECTED_STORE_V8_REBOUND_FUNCTION_CATALOG_SHA256: &str =
    "3874875a39369bd3e3e9238afbe5abd2cfc2cd4f29447d6013bcf59ffbb61bb0";
const EXPECTED_STORE_V8_REBOUND_TABLE_CATALOG_SHA256: &str =
    "28606d1ae0b3dce3f7f47f93dfde651fbe44c28d237ce9558bbbf6cad728078d";
const PREDECESSOR_EXTENSION_SQL_BYTES: i64 = 349_470;
const PREDECESSOR_EXTENSION_SQL_SHA256: &str =
    "32dd034191b9d87c8792f78c26b5d84533a95405ff4d1cc5be00da54a08d4b13";
const PREDECESSOR_EXTENSION_MANIFEST_SHA256: &str =
    "0b1855611b37da4ed8b17be3d85e6410598fb13a255ce307d0907e702afeea63";
const PREDECESSOR_FUNCTION_CATALOG_SHA256: &str =
    "7b249bf8416f734a34b6e1b9e7b407d17b00771139ac71a12294a3b0543e6120";
const PREDECESSOR_TABLE_CATALOG_SHA256: &str =
    "dcd206da5753e55a4499717a896fd1373165430edd6eadeaf6c1284c23fbde17";

/// Closed administrative/runtime database roles for the extension.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionDatabaseRole {
    Migrator,
    Runtime,
}

impl ExtensionDatabaseRole {
    #[must_use]
    pub const fn session_role(self) -> &'static str {
        match self {
            Self::Migrator => "lattice_migrator_login",
            Self::Runtime => "lattice_runtime_login",
        }
    }

    #[must_use]
    pub const fn current_role(self) -> &'static str {
        match self {
            Self::Migrator => "lattice_migrator",
            Self::Runtime => "lattice_runtime",
        }
    }
}

/// Explicit database name/run identity from which Store derives its UUID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionTarget {
    database_name: String,
    expected_database_uuid: String,
    expected_database_identity_digest: ContentDigest,
}

impl ExtensionTarget {
    /// Constructs a closed non-ambient target and derives its exact Store identity.
    ///
    /// # Errors
    ///
    /// Rejects default/system names and path-like, whitespace, or oversized values.
    pub fn new(
        database_name: impl Into<String>,
        installation_run_id: impl Into<String>,
    ) -> Result<Self, ExtensionSetupError> {
        let database_name = database_name.into();
        let installation_run_id = installation_run_id.into();
        if !valid_database_name(&database_name)
            || database_name == "postgres"
            || !valid_run_id(&installation_run_id)
        {
            return Err(error(
                ExtensionSetupErrorKind::InvalidTarget,
                "FOREMAN_EXTENSION_INVALID_TARGET",
            ));
        }
        let (expected_database_uuid, expected_database_identity_digest) =
            derive_database_identity(&database_name, &installation_run_id)?;
        Ok(Self {
            database_name,
            expected_database_uuid,
            expected_database_identity_digest,
        })
    }

    #[must_use]
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    #[must_use]
    pub fn expected_database_uuid(&self) -> &str {
        &self.expected_database_uuid
    }

    #[must_use]
    pub const fn expected_database_identity_digest(&self) -> &ContentDigest {
        &self.expected_database_identity_digest
    }
}

/// Closed setup/verification failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionSetupErrorKind {
    InvalidTarget,
    ManifestInvalid,
    TransactionFailed,
    RoleMismatch,
    ServerVersionMismatch,
    GlobalIdentityMismatch,
    PartialProfile,
    SchemaCollision,
    CatalogMismatch,
}

/// Secret-free setup error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExtensionSetupError {
    kind: ExtensionSetupErrorKind,
    code: &'static str,
}

impl ExtensionSetupError {
    #[must_use]
    pub const fn kind(self) -> ExtensionSetupErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ExtensionSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for ExtensionSetupError {}

/// Verified catalog evidence safe to retain in an adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionCatalogEvidence {
    database_uuid: String,
    role: ExtensionDatabaseRole,
    sql_sha256: ContentDigest,
    manifest_sha256: ContentDigest,
}

impl ExtensionCatalogEvidence {
    #[must_use]
    pub fn database_uuid(&self) -> &str {
        &self.database_uuid
    }

    #[must_use]
    pub const fn role(&self) -> ExtensionDatabaseRole {
        self.role
    }

    #[must_use]
    pub const fn sql_sha256(&self) -> &ContentDigest {
        &self.sql_sha256
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> &ContentDigest {
        &self.manifest_sha256
    }
}

/// Result of an explicit administrative application.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtensionApplyOutcome {
    Installed(ExtensionCatalogEvidence),
    Upgraded(ExtensionCatalogEvidence),
    Rebound(ExtensionCatalogEvidence),
    AlreadyCurrent(ExtensionCatalogEvidence),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoreProfile {
    V7,
    V8,
}

struct GlobalApplyGate<'client> {
    client: &'client mut Client,
    held: bool,
}

impl<'client> GlobalApplyGate<'client> {
    fn acquire(client: &'client mut Client) -> Result<Self, ExtensionSetupError> {
        let started_at = Instant::now();
        loop {
            let acquired: bool = client
                .query_one(
                    "SELECT pg_catalog.pg_try_advisory_lock($1::bigint)",
                    &[&GLOBAL_MIGRATION_ADVISORY_LOCK],
                )
                .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_GLOBAL_GATE_QUERY_FAILED"))?
                .get(0);
            if acquired {
                return Ok(Self { client, held: true });
            }
            let elapsed = started_at.elapsed();
            if elapsed >= GLOBAL_APPLY_GATE_TIMEOUT {
                return Err(transaction_stage_error(
                    "FOREMAN_EXTENSION_GLOBAL_GATE_TIMEOUT",
                ));
            }
            std::thread::sleep(std::cmp::min(
                GLOBAL_APPLY_GATE_POLL_INTERVAL,
                GLOBAL_APPLY_GATE_TIMEOUT.saturating_sub(elapsed),
            ));
        }
    }

    fn client(&mut self) -> &mut Client {
        self.client
    }

    fn release(mut self) -> Result<(), ExtensionSetupError> {
        let released: bool = self
            .client
            .query_one(
                "SELECT pg_catalog.pg_advisory_unlock($1::bigint)",
                &[&GLOBAL_MIGRATION_ADVISORY_LOCK],
            )
            .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_GLOBAL_GATE_RELEASE_FAILED"))?
            .get(0);
        self.held = false;
        if !released {
            return Err(transaction_stage_error(
                "FOREMAN_EXTENSION_GLOBAL_GATE_RELEASE_FAILED",
            ));
        }
        Ok(())
    }
}

impl Drop for GlobalApplyGate<'_> {
    fn drop(&mut self) {
        if self.held {
            let _ = self.client.query_one(
                "SELECT pg_catalog.pg_advisory_unlock($1::bigint)",
                &[&GLOBAL_MIGRATION_ADVISORY_LOCK],
            );
            self.held = false;
        }
    }
}

/// Installs fresh v1, atomically replaces the exact empty predecessor, or
/// verifies an exact existing profile.
///
/// # Errors
///
/// Any wrong role, Store identity, schema collision, partial state, predecessor
/// data/dependency, SQL failure, catalog drift, or ACL drift rolls back and
/// fails closed. No ordinary Runtime path calls this administrative operation.
pub fn apply_extension(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let mut gate = GlobalApplyGate::acquire(client)?;
    let result = apply_extension_under_gate(gate.client(), target);
    gate.release()?;
    result
}

fn apply_extension_under_gate(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let manifest = verify_embedded_extension().map_err(|_| {
        error(
            ExtensionSetupErrorKind::ManifestInvalid,
            "FOREMAN_EXTENSION_MANIFEST_INVALID",
        )
    })?;
    let store_v8_rebind = verify_embedded_store_v8_rebind().map_err(|_| {
        error(
            ExtensionSetupErrorKind::ManifestInvalid,
            "FOREMAN_EXTENSION_STORE_V8_REBIND_INVALID",
        )
    })?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_TRANSACTION_START_FAILED"))?;
    harden(&mut transaction)
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_HARDEN_FAILED"))?;
    transaction
        .query_one(
            "SELECT pg_catalog.pg_advisory_xact_lock($1::bigint)",
            &[&EXTENSION_ADVISORY_LOCK],
        )
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_LOCK_FAILED"))?;
    let store_profile =
        verify_global_preflight(&mut transaction, target, ExtensionDatabaseRole::Migrator)
            .map_err(|failure| {
                restage_transaction_failure(
                    failure,
                    "FOREMAN_EXTENSION_GLOBAL_PREFLIGHT_QUERY_FAILED",
                )
            })?;
    let state = classify_extension(&mut transaction).map_err(|failure| {
        restage_transaction_failure(failure, "FOREMAN_EXTENSION_CLASSIFY_FAILED")
    })?;
    if state == ExtensionPreState::Exact {
        let function_digest = measure_function_catalog_digest(&mut transaction)?;
        let table_digest = measure_table_catalog_digest(&mut transaction)?;
        if function_digest == EXPECTED_FUNCTION_CATALOG_SHA256
            && table_digest == EXPECTED_TABLE_CATALOG_SHA256
            && store_profile == StoreProfile::V7
        {
            let evidence = verify_catalog(
                &mut transaction,
                target,
                ExtensionDatabaseRole::Migrator,
                &manifest,
                StoreProfile::V7,
            )?;
            transaction
                .commit()
                .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_COMMIT_FAILED"))?;
            return Ok(ExtensionApplyOutcome::AlreadyCurrent(evidence));
        }
        if function_digest == EXPECTED_STORE_V8_REBOUND_FUNCTION_CATALOG_SHA256
            && table_digest == EXPECTED_STORE_V8_REBOUND_TABLE_CATALOG_SHA256
            && store_profile == StoreProfile::V8
        {
            let evidence = verify_catalog(
                &mut transaction,
                target,
                ExtensionDatabaseRole::Migrator,
                &manifest,
                StoreProfile::V8,
            )?;
            transaction
                .commit()
                .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_COMMIT_FAILED"))?;
            return Ok(ExtensionApplyOutcome::AlreadyCurrent(evidence));
        }
        if function_digest == EXPECTED_FUNCTION_CATALOG_SHA256
            && table_digest == EXPECTED_TABLE_CATALOG_SHA256
            && store_profile == StoreProfile::V8
        {
            verify_store_v8_rebind_source(&mut transaction, target, &manifest)?;
            apply_store_v8_rebind(&mut transaction, &store_v8_rebind)?;
            let evidence = verify_store_v8_rebound_catalog(
                &mut transaction,
                target,
                &manifest,
                ExtensionDatabaseRole::Migrator,
            )?;
            transaction
                .commit()
                .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_COMMIT_FAILED"))?;
            return Ok(ExtensionApplyOutcome::Rebound(evidence));
        }
        if function_digest != PREDECESSOR_FUNCTION_CATALOG_SHA256
            || table_digest != PREDECESSOR_TABLE_CATALOG_SHA256
        {
            return Err(catalog_profile_mismatch());
        }
        verify_exact_empty_predecessor(&mut transaction, target)?;
        verify_stopped_admission(&mut transaction)?;
        transaction
            .batch_execute("DROP SCHEMA foreman_execution CASCADE")
            .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_PREDECESSOR_DROP_FAILED"))?;
        let sql = std::str::from_utf8(manifest.bytes()).map_err(|_| transaction_error())?;
        transaction
            .batch_execute(sql)
            .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_SQL_FAILED"))?;
        insert_identity(&mut transaction, target, &manifest)?;
        let evidence = if store_profile == StoreProfile::V8 {
            verify_store_v8_rebind_source(&mut transaction, target, &manifest)?;
            apply_store_v8_rebind(&mut transaction, &store_v8_rebind)?;
            verify_store_v8_rebound_catalog(
                &mut transaction,
                target,
                &manifest,
                ExtensionDatabaseRole::Migrator,
            )?
        } else {
            verify_catalog(
                &mut transaction,
                target,
                ExtensionDatabaseRole::Migrator,
                &manifest,
                StoreProfile::V7,
            )?
        };
        verify_stopped_admission(&mut transaction)?;
        transaction
            .commit()
            .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_COMMIT_FAILED"))?;
        return Ok(ExtensionApplyOutcome::Upgraded(evidence));
    }
    match state {
        ExtensionPreState::Fresh => {}
        ExtensionPreState::Partial => {
            return Err(error(
                ExtensionSetupErrorKind::PartialProfile,
                "FOREMAN_EXTENSION_PARTIAL_PROFILE",
            ));
        }
        ExtensionPreState::Collision => {
            return Err(error(
                ExtensionSetupErrorKind::SchemaCollision,
                "FOREMAN_EXTENSION_SCHEMA_COLLISION",
            ));
        }
        ExtensionPreState::Exact => unreachable!("handled above"),
    }
    verify_stopped_admission(&mut transaction)?;
    let sql = std::str::from_utf8(manifest.bytes()).map_err(|_| transaction_error())?;
    transaction
        .batch_execute(sql)
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_SQL_FAILED"))?;
    insert_identity(&mut transaction, target, &manifest)?;
    let evidence = if store_profile == StoreProfile::V8 {
        verify_store_v8_rebind_source(&mut transaction, target, &manifest)?;
        apply_store_v8_rebind(&mut transaction, &store_v8_rebind)?;
        verify_store_v8_rebound_catalog(
            &mut transaction,
            target,
            &manifest,
            ExtensionDatabaseRole::Migrator,
        )?
    } else {
        verify_catalog(
            &mut transaction,
            target,
            ExtensionDatabaseRole::Migrator,
            &manifest,
            StoreProfile::V7,
        )?
    };
    verify_stopped_admission(&mut transaction)?;
    transaction
        .commit()
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_COMMIT_FAILED"))?;
    Ok(ExtensionApplyOutcome::Installed(evidence))
}

/// Verifies one exact installed profile without installing or repairing it.
///
/// # Errors
///
/// Fails closed on identity, role, catalog, function, or ACL drift.
pub fn verify_extension(
    client: &mut Client,
    target: &ExtensionTarget,
    role: ExtensionDatabaseRole,
) -> Result<ExtensionCatalogEvidence, ExtensionSetupError> {
    let manifest = verify_embedded_extension().map_err(|_| {
        error(
            ExtensionSetupErrorKind::ManifestInvalid,
            "FOREMAN_EXTENSION_MANIFEST_INVALID",
        )
    })?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_VERIFY_START_FAILED"))?;
    harden(&mut transaction)?;
    let store_profile = verify_global_preflight(&mut transaction, target, role)?;
    let evidence = verify_catalog(&mut transaction, target, role, &manifest, store_profile)?;
    transaction.commit().map_err(|_| transaction_error())?;
    Ok(evidence)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExtensionPreState {
    Fresh,
    Exact,
    Partial,
    Collision,
}

fn harden(client: &mut impl GenericClient) -> Result<(), ExtensionSetupError> {
    client
        .batch_execute(
            "SET LOCAL search_path = pg_catalog; \
             SET LOCAL row_security = on; \
             SET LOCAL lock_timeout = '5s'; \
             SET LOCAL statement_timeout = '30s'",
        )
        .map_err(|_| transaction_error())
}

fn verify_global_preflight(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    role: ExtensionDatabaseRole,
) -> Result<StoreProfile, ExtensionSetupError> {
    verify_session(client, target, role).map_err(|failure| {
        restage_transaction_failure(failure, "FOREMAN_EXTENSION_SESSION_QUERY_FAILED")
    })?;
    let row = client
        .query_one(
            "SELECT d.database_uuid::text, c.current_schema_version::integer, \
                    pg_catalog.btrim(c.manifest_sha256)::text \
               FROM ONLY control.database_identity AS d \
               CROSS JOIN ONLY control.schema_compatibility AS c \
              WHERE d.singleton AND c.singleton",
            &[],
        )
        .map_err(|_| global_error())?;
    let database_uuid: String = row.get(0);
    let schema_version: i32 = row.get(1);
    let manifest_sha256: String = row.get(2);
    let profile = classify_store_profile(schema_version, &manifest_sha256);
    if database_uuid != target.expected_database_uuid() || profile.is_none() {
        return Err(global_error());
    }
    profile.ok_or_else(global_error)
}

fn classify_store_profile(schema_version: i32, manifest_sha256: &str) -> Option<StoreProfile> {
    match (schema_version, manifest_sha256) {
        (version, REQUIRED_GLOBAL_MANIFEST_SHA256)
            if version == i32::from(REQUIRED_GLOBAL_SCHEMA_VERSION) =>
        {
            Some(StoreProfile::V7)
        }
        (STORE_V8_GLOBAL_SCHEMA_VERSION, STORE_V8_GLOBAL_MANIFEST_SHA256) => Some(StoreProfile::V8),
        _ => None,
    }
}

#[cfg(test)]
fn supported_store_profile(schema_version: i32, manifest_sha256: &str) -> bool {
    classify_store_profile(schema_version, manifest_sha256).is_some()
}

fn verify_session(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    role: ExtensionDatabaseRole,
) -> Result<(), ExtensionSetupError> {
    let row = client
        .query_one(
            "SELECT pg_catalog.current_database()::text, \
                    CURRENT_USER::text, SESSION_USER::text, \
                    pg_catalog.current_setting('server_version_num')::integer",
            &[],
        )
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_SESSION_QUERY_FAILED"))?;
    let database: String = row.get(0);
    let current_role: String = row.get(1);
    let session_role: String = row.get(2);
    let server_version: i32 = row.get(3);
    if database != target.database_name() {
        return Err(global_error());
    }
    if current_role != role.current_role() || session_role != role.session_role() {
        return Err(error(
            ExtensionSetupErrorKind::RoleMismatch,
            "FOREMAN_EXTENSION_ROLE_MISMATCH",
        ));
    }
    if !(170_000..180_000).contains(&server_version) {
        return Err(error(
            ExtensionSetupErrorKind::ServerVersionMismatch,
            "FOREMAN_EXTENSION_SERVER_VERSION_MISMATCH",
        ));
    }
    Ok(())
}

fn classify_extension(
    client: &mut impl GenericClient,
) -> Result<ExtensionPreState, ExtensionSetupError> {
    let row = client
        .query_one(
            "SELECT pg_catalog.to_regnamespace('foreman_execution') IS NOT NULL, \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                     WHERE n.nspname = 'foreman_execution'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                     WHERE n.nspname = 'foreman_execution'), \
                    (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                      JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                     WHERE n.nspname = 'foreman_execution' \
                       AND c.relname = 'extension_identity' AND c.relkind = 'r')",
            &[],
        )
        .map_err(|_| transaction_error())?;
    let schema_exists: bool = row.get(0);
    let relations: i64 = row.get(1);
    let functions: i64 = row.get(2);
    let identity_table: i64 = row.get(3);
    if !schema_exists && relations == 0 && functions == 0 {
        return Ok(ExtensionPreState::Fresh);
    }
    if schema_exists && identity_table == 1 {
        let identity_rows: i64 = client
            .query_one(
                "SELECT pg_catalog.count(*) FROM ONLY foreman_execution.extension_identity",
                &[],
            )
            .map_err(|_| transaction_error())?
            .get(0);
        return Ok(if identity_rows == 1 {
            ExtensionPreState::Exact
        } else {
            ExtensionPreState::Partial
        });
    }
    Ok(if schema_exists && relations == 0 && functions == 0 {
        ExtensionPreState::Collision
    } else {
        ExtensionPreState::Partial
    })
}

#[allow(clippy::too_many_lines)]
fn verify_exact_empty_predecessor(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
) -> Result<(), ExtensionSetupError> {
    verify_closed_catalog_shape(client)?;
    let identity = client
        .query_one(
            "SELECT extension_id, extension_schema_version, extension_path, \
                    extension_sql_bytes, extension_sql_sha256, \
                    extension_manifest_sha256, database_name, database_uuid, \
                    database_identity_sha256, global_schema_version, \
                    global_manifest_sha256 \
               FROM foreman_execution.read_extension_identity_v1()",
            &[],
        )
        .map_err(|_| catalog_profile_mismatch())?;
    if identity.get::<_, String>(0) != FOREMAN_EXTENSION_ID
        || identity.get::<_, i16>(1)
            != i16::try_from(FOREMAN_EXTENSION_SCHEMA_VERSION).expect("fixed")
        || identity.get::<_, String>(2) != FOREMAN_EXTENSION_PATH
        || identity.get::<_, i64>(3) != PREDECESSOR_EXTENSION_SQL_BYTES
        || identity.get::<_, String>(4) != PREDECESSOR_EXTENSION_SQL_SHA256
        || identity.get::<_, String>(5) != PREDECESSOR_EXTENSION_MANIFEST_SHA256
        || identity.get::<_, String>(6) != target.database_name()
        || identity.get::<_, String>(7) != target.expected_database_uuid()
        || identity.get::<_, String>(8) != target.expected_database_identity_digest().as_str()
        || identity.get::<_, i16>(9)
            != i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed")
        || identity.get::<_, String>(10) != REQUIRED_GLOBAL_MANIFEST_SHA256
    {
        return Err(catalog_profile_mismatch());
    }

    let boundary = client
        .query_one(
            "WITH foreman_relations(objid) AS ( \
                SELECT c.oid FROM pg_catalog.pg_class c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' \
            ), foreman_constraints(objid) AS ( \
                SELECT c.oid FROM pg_catalog.pg_constraint c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.connamespace \
                 WHERE n.nspname='foreman_execution' \
            ), foreman_toast_relations(objid) AS ( \
                SELECT c.reltoastrelid FROM pg_catalog.pg_class c \
                 WHERE c.oid IN (SELECT objid FROM foreman_relations) \
                   AND c.reltoastrelid<>0 \
                UNION \
                SELECT i.indexrelid FROM pg_catalog.pg_index i \
                 WHERE i.indrelid IN ( \
                    SELECT c.reltoastrelid FROM pg_catalog.pg_class c \
                     WHERE c.oid IN (SELECT objid FROM foreman_relations) \
                       AND c.reltoastrelid<>0) \
            ), managed(classid,objid) AS ( \
                SELECT 'pg_namespace'::pg_catalog.regclass::oid,n.oid \
                  FROM pg_catalog.pg_namespace n \
                 WHERE n.nspname='foreman_execution' \
                UNION \
                SELECT 'pg_class'::pg_catalog.regclass::oid,objid \
                  FROM foreman_relations \
                UNION \
                SELECT 'pg_class'::pg_catalog.regclass::oid,objid \
                  FROM foreman_toast_relations \
                UNION \
                SELECT 'pg_proc'::pg_catalog.regclass::oid,p.oid \
                  FROM pg_catalog.pg_proc p \
                  JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                 WHERE n.nspname='foreman_execution' \
                UNION \
                SELECT 'pg_type'::pg_catalog.regclass::oid,t.oid \
                  FROM pg_catalog.pg_type t \
                  JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                 WHERE n.nspname='foreman_execution' \
                UNION \
                SELECT 'pg_constraint'::pg_catalog.regclass::oid,objid \
                  FROM foreman_constraints \
                UNION \
                SELECT 'pg_attrdef'::pg_catalog.regclass::oid,a.oid \
                  FROM pg_catalog.pg_attrdef a \
                 WHERE a.adrelid IN (SELECT objid FROM foreman_relations) \
                UNION \
                SELECT 'pg_trigger'::pg_catalog.regclass::oid,t.oid \
                  FROM pg_catalog.pg_trigger t \
                 WHERE t.tgrelid IN (SELECT objid FROM foreman_relations) \
                    OR t.tgconstraint IN (SELECT objid FROM foreman_constraints) \
                UNION \
                SELECT 'pg_rewrite'::pg_catalog.regclass::oid,r.oid \
                  FROM pg_catalog.pg_rewrite r \
                 WHERE r.ev_class IN (SELECT objid FROM foreman_relations) \
                UNION \
                SELECT 'pg_policy'::pg_catalog.regclass::oid,p.oid \
                  FROM pg_catalog.pg_policy p \
                 WHERE p.polrelid IN (SELECT objid FROM foreman_relations) \
                UNION \
                SELECT 'pg_statistic_ext'::pg_catalog.regclass::oid,s.oid \
                  FROM pg_catalog.pg_statistic_ext s \
                  JOIN pg_catalog.pg_namespace n ON n.oid=s.stxnamespace \
                 WHERE n.nspname='foreman_execution' \
                UNION \
                SELECT 'pg_type'::pg_catalog.regclass::oid,t.oid \
                  FROM pg_catalog.pg_type t \
                 WHERE t.typrelid IN (SELECT objid FROM foreman_toast_relations) \
            ) \
            SELECT \
                (SELECT pg_catalog.count(*) \
                   FROM ONLY foreman_execution.extension_identity), \
                (SELECT pg_catalog.count(*) \
                   FROM ONLY foreman_execution.extension_ledger), \
                ((SELECT pg_catalog.count(*) FROM ONLY foreman_execution.child_events) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.preparation_observations) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.promotion_intents) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.task_promotions) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_attempts) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.pending_worker_claims) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.execution_environments) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.worker_observations) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.verification_records) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.artifact_references) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.staged_artifact_references) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.provider_dispatch_claims) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.attempt_closures) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.approval_owner_snapshots) + \
                 (SELECT pg_catalog.count(*) FROM ONLY foreman_execution.approval_evidence)), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_depend d \
                  JOIN managed referenced \
                    ON referenced.classid=d.refclassid AND referenced.objid=d.refobjid \
                  LEFT JOIN managed dependent \
                    ON dependent.classid=d.classid AND dependent.objid=d.objid \
                 WHERE dependent.objid IS NULL)",
            &[],
        )
        .map_err(|_| catalog_profile_mismatch())?;
    if boundary.get::<_, i64>(0) != 1
        || boundary.get::<_, i64>(1) != 1
        || boundary.get::<_, i64>(2) != 0
        || boundary.get::<_, i64>(3) != 0
    {
        return Err(error(
            ExtensionSetupErrorKind::PartialProfile,
            "FOREMAN_EXTENSION_PREDECESSOR_NOT_EMPTY",
        ));
    }
    Ok(())
}

fn insert_identity(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), ExtensionSetupError> {
    client
        .execute(
            "INSERT INTO foreman_execution.extension_identity ( \
                singleton, extension_id, extension_schema_version, extension_path, \
                extension_sql_bytes, extension_sql_sha256, extension_manifest_sha256, \
                database_name, database_uuid, database_identity_sha256, \
                global_schema_version, global_manifest_sha256 \
             ) VALUES (true,$1,$2,$3,$4,$5,$6,$7,$8::text::uuid,$9,$10,$11)",
            &[
                &FOREMAN_EXTENSION_ID,
                &i16::try_from(FOREMAN_EXTENSION_SCHEMA_VERSION).expect("fixed version"),
                &FOREMAN_EXTENSION_PATH,
                &i64::try_from(manifest.byte_length()).expect("bounded SQL"),
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &target.database_name(),
                &target.expected_database_uuid(),
                &target.expected_database_identity_digest().as_str(),
                &i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed global version"),
                &REQUIRED_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_IDENTITY_ROW_FAILED"))?;
    client
        .execute(
            "INSERT INTO foreman_execution.extension_ledger ( \
                ledger_ordinal, extension_id, extension_schema_version, \
                extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                database_identity_sha256, global_schema_version, \
                global_manifest_sha256, event_kind \
             ) VALUES (1,$1,$2,$3,$4,$5::text::uuid,$6,$7,$8,'INSTALLED')",
            &[
                &FOREMAN_EXTENSION_ID,
                &i16::try_from(FOREMAN_EXTENSION_SCHEMA_VERSION).expect("fixed version"),
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &target.expected_database_uuid(),
                &target.expected_database_identity_digest().as_str(),
                &i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed global version"),
                &REQUIRED_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_LEDGER_ROW_FAILED"))?;
    Ok(())
}

fn verify_stopped_admission(client: &mut impl GenericClient) -> Result<(), ExtensionSetupError> {
    client
        .batch_execute("LOCK TABLE control.runtime_admission IN ACCESS EXCLUSIVE MODE")
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_RUNTIME_ADMISSION_LOCK_FAILED"))?;
    let rows = client
        .query(
            "SELECT admission_mode::text, daemon_instance_id::text, daemon_epoch, \
                    authority_revision, observation_digest, authority_head_digest \
               FROM ONLY control.runtime_admission \
              WHERE singleton = true",
            &[],
        )
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_RUNTIME_ADMISSION_QUERY_FAILED"))?;
    if rows.len() != 1 {
        return Err(runtime_admission_error());
    }
    let row = &rows[0];
    let mode: String = row.try_get(0).map_err(|_| runtime_admission_error())?;
    let daemon_instance: Option<String> = row.try_get(1).map_err(|_| runtime_admission_error())?;
    let daemon_epoch: Option<i64> = row.try_get(2).map_err(|_| runtime_admission_error())?;
    let authority_revision: i64 = row.try_get(3).map_err(|_| runtime_admission_error())?;
    let observation_digest: Option<Vec<u8>> =
        row.try_get(4).map_err(|_| runtime_admission_error())?;
    let authority_head_digest: Option<Vec<u8>> =
        row.try_get(5).map_err(|_| runtime_admission_error())?;
    if mode != "STOPPED"
        || daemon_instance.is_some()
        || daemon_epoch.is_some()
        || authority_revision != 0
        || observation_digest.is_some()
        || authority_head_digest.is_some()
    {
        return Err(runtime_admission_error());
    }
    Ok(())
}

fn verify_exact_identity_and_history(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
    store_profile: StoreProfile,
) -> Result<(), ExtensionSetupError> {
    let identities = client
        .query(
            "SELECT extension_id, extension_schema_version, extension_path, \
                    extension_sql_bytes, extension_sql_sha256, \
                    extension_manifest_sha256, database_name, database_uuid::text, \
                    database_identity_sha256, global_schema_version, \
                    global_manifest_sha256 \
               FROM ONLY foreman_execution.extension_identity \
              WHERE singleton",
            &[],
        )
        .map_err(|_| catalog_profile_mismatch())?;
    if identities.len() != 1 {
        return Err(catalog_profile_mismatch());
    }
    let identity = &identities[0];
    let (global_version, global_manifest) = match store_profile {
        StoreProfile::V7 => (
            i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed"),
            REQUIRED_GLOBAL_MANIFEST_SHA256,
        ),
        StoreProfile::V8 => (
            i16::try_from(STORE_V8_GLOBAL_SCHEMA_VERSION).expect("fixed"),
            STORE_V8_GLOBAL_MANIFEST_SHA256,
        ),
    };
    if identity.get::<_, String>(0) != FOREMAN_EXTENSION_ID
        || identity.get::<_, i16>(1)
            != i16::try_from(FOREMAN_EXTENSION_SCHEMA_VERSION).expect("fixed")
        || identity.get::<_, String>(2) != FOREMAN_EXTENSION_PATH
        || identity.get::<_, i64>(3) != i64::try_from(manifest.byte_length()).expect("bounded SQL")
        || identity.get::<_, String>(4) != manifest.sql_sha256().as_str()
        || identity.get::<_, String>(5) != manifest.manifest_sha256().as_str()
        || identity.get::<_, String>(6) != target.database_name()
        || identity.get::<_, String>(7) != target.expected_database_uuid()
        || identity.get::<_, String>(8) != target.expected_database_identity_digest().as_str()
        || identity.get::<_, i16>(9) != global_version
        || identity.get::<_, String>(10) != global_manifest
    {
        return Err(catalog_profile_mismatch());
    }

    let ledger = client
        .query(
            "SELECT ledger_ordinal, extension_id, extension_schema_version, \
                    extension_sql_sha256, extension_manifest_sha256, database_uuid::text, \
                    database_identity_sha256, global_schema_version, \
                    global_manifest_sha256, event_kind \
               FROM ONLY foreman_execution.extension_ledger \
              ORDER BY ledger_ordinal",
            &[],
        )
        .map_err(|_| catalog_profile_mismatch())?;
    let expected_rows = match store_profile {
        StoreProfile::V7 => 1,
        StoreProfile::V8 => 2,
    };
    if ledger.len() != expected_rows {
        return Err(catalog_profile_mismatch());
    }
    for (index, row) in ledger.iter().enumerate() {
        let (expected_ordinal, expected_global_version, expected_global_manifest, expected_event) =
            if index == 0 {
                (
                    1_i16,
                    i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed"),
                    REQUIRED_GLOBAL_MANIFEST_SHA256,
                    "INSTALLED",
                )
            } else {
                (
                    2_i16,
                    i16::try_from(STORE_V8_GLOBAL_SCHEMA_VERSION).expect("fixed"),
                    STORE_V8_GLOBAL_MANIFEST_SHA256,
                    "REBOUND",
                )
            };
        if row.get::<_, i16>(0) != expected_ordinal
            || row.get::<_, String>(1) != FOREMAN_EXTENSION_ID
            || row.get::<_, i16>(2)
                != i16::try_from(FOREMAN_EXTENSION_SCHEMA_VERSION).expect("fixed")
            || row.get::<_, String>(3) != manifest.sql_sha256().as_str()
            || row.get::<_, String>(4) != manifest.manifest_sha256().as_str()
            || row.get::<_, String>(5) != target.expected_database_uuid()
            || row.get::<_, String>(6) != target.expected_database_identity_digest().as_str()
            || row.get::<_, i16>(7) != expected_global_version
            || row.get::<_, String>(8) != expected_global_manifest
            || row.get::<_, String>(9) != expected_event
        {
            return Err(catalog_profile_mismatch());
        }
    }
    Ok(())
}

fn verify_store_v8_rebind_source(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
) -> Result<(), ExtensionSetupError> {
    verify_stopped_admission(client)?;
    verify_exact_catalog_digests(client, StoreProfile::V7)?;
    verify_closed_catalog_shape(client)?;
    verify_exact_identity_and_history(client, target, manifest, StoreProfile::V7)
}

fn apply_store_v8_rebind(
    client: &mut impl GenericClient,
    rebind: &StoreV8RebindEvidence,
) -> Result<(), ExtensionSetupError> {
    let sql = std::str::from_utf8(rebind.bytes())
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_STORE_V8_REBIND_UTF8_INVALID"))?;
    client
        .batch_execute(sql)
        .map_err(|_| transaction_stage_error("FOREMAN_EXTENSION_STORE_V8_REBIND_SQL_FAILED"))
}

fn verify_store_v8_rebound_catalog(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    manifest: &ExtensionManifestEvidence,
    role: ExtensionDatabaseRole,
) -> Result<ExtensionCatalogEvidence, ExtensionSetupError> {
    verify_stopped_admission(client)?;
    verify_exact_identity_and_history(client, target, manifest, StoreProfile::V8)?;
    verify_catalog(client, target, role, manifest, StoreProfile::V8)
}

#[allow(clippy::too_many_lines)]
fn verify_catalog(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    role: ExtensionDatabaseRole,
    manifest: &ExtensionManifestEvidence,
    store_profile: StoreProfile,
) -> Result<ExtensionCatalogEvidence, ExtensionSetupError> {
    // Pin the SECURITY DEFINER implementation and its complete ACL before
    // invoking the extension-owned identity reader.
    verify_exact_catalog_digests(client, store_profile)?;
    verify_closed_catalog_shape(client)?;
    let identity = client
        .query_opt(
            "SELECT extension_id, extension_schema_version, extension_path, \
                    extension_sql_bytes, extension_sql_sha256, \
                    extension_manifest_sha256, database_name, database_uuid, \
                    database_identity_sha256, global_schema_version, \
                    global_manifest_sha256 \
               FROM foreman_execution.read_extension_identity_v1()",
            &[],
        )
        .map_err(|_| {
            error(
                ExtensionSetupErrorKind::CatalogMismatch,
                "FOREMAN_EXTENSION_IDENTITY_QUERY_FAILED",
            )
        })?
        .ok_or_else(|| {
            error(
                ExtensionSetupErrorKind::CatalogMismatch,
                "FOREMAN_EXTENSION_IDENTITY_MISMATCH",
            )
        })?;
    let extension_id: String = identity.get(0);
    let extension_version: i16 = identity.get(1);
    let extension_path: String = identity.get(2);
    let extension_bytes: i64 = identity.get(3);
    let sql_sha256: String = identity.get(4);
    let manifest_sha256: String = identity.get(5);
    let database_name: String = identity.get(6);
    let database_uuid: String = identity.get(7);
    let database_identity: String = identity.get(8);
    let global_version: i16 = identity.get(9);
    let global_manifest: String = identity.get(10);
    let (expected_global_version, expected_global_manifest) = match store_profile {
        StoreProfile::V7 => (
            i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed"),
            REQUIRED_GLOBAL_MANIFEST_SHA256,
        ),
        StoreProfile::V8 => (
            i16::try_from(STORE_V8_GLOBAL_SCHEMA_VERSION).expect("fixed"),
            STORE_V8_GLOBAL_MANIFEST_SHA256,
        ),
    };
    if extension_id != FOREMAN_EXTENSION_ID
        || extension_version != i16::try_from(FOREMAN_EXTENSION_SCHEMA_VERSION).expect("fixed")
        || extension_path != FOREMAN_EXTENSION_PATH
        || extension_bytes != i64::try_from(manifest.byte_length()).expect("bounded")
        || sql_sha256 != manifest.sql_sha256().as_str()
        || manifest_sha256 != manifest.manifest_sha256().as_str()
        || database_name != target.database_name()
        || database_uuid != target.expected_database_uuid()
        || database_identity != target.expected_database_identity_digest().as_str()
        || global_version != expected_global_version
        || global_manifest != expected_global_manifest
    {
        return Err(error(
            ExtensionSetupErrorKind::CatalogMismatch,
            "FOREMAN_EXTENSION_IDENTITY_MISMATCH",
        ));
    }
    let shape = client
        .query_one(
            "SELECT \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                 WHERE n.nspname = 'foreman_execution' AND c.relkind = 'r'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                 WHERE n.nspname = 'foreman_execution'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                  JOIN pg_catalog.pg_roles AS r ON r.oid = c.relowner \
                 WHERE n.nspname = 'foreman_execution' AND c.relkind = 'r' \
                   AND r.rolname = 'lattice_migrator'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                  JOIN pg_catalog.pg_roles AS r ON r.oid = p.proowner \
                 WHERE n.nspname = 'foreman_execution' \
                   AND r.rolname = 'lattice_migrator' AND p.prosecdef \
                   AND p.proconfig = ARRAY['search_path=pg_catalog']::text[]), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class AS c \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                 WHERE n.nspname = 'foreman_execution' AND c.relkind = 'r' \
                   AND pg_catalog.has_table_privilege( \
                       'lattice_runtime', c.oid, \
                       'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_proc AS p \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
                 WHERE n.nspname = 'foreman_execution' \
                   AND pg_catalog.has_function_privilege('lattice_runtime', p.oid, 'EXECUTE')), \
                (SELECT EXISTS ( \
                    SELECT 1 FROM pg_catalog.pg_namespace AS n \
                    CROSS JOIN LATERAL pg_catalog.aclexplode( \
                        COALESCE(n.nspacl, pg_catalog.acldefault('n',n.nspowner))) AS a \
                    WHERE n.nspname = 'foreman_execution' AND a.grantee = 0 \
                      AND a.privilege_type = 'USAGE')), \
                pg_catalog.has_schema_privilege('lattice_runtime','foreman_execution','USAGE'), \
                pg_catalog.has_schema_privilege('lattice_runtime','foreman_execution','CREATE')",
            &[],
        )
        .map_err(|_| {
            error(
                ExtensionSetupErrorKind::CatalogMismatch,
                "FOREMAN_EXTENSION_SHAPE_QUERY_FAILED",
            )
        })?;
    let tables: i64 = shape.get(0);
    let functions: i64 = shape.get(1);
    let owned_tables: i64 = shape.get(2);
    let hardened_functions: i64 = shape.get(3);
    let runtime_tables: i64 = shape.get(4);
    let runtime_functions: i64 = shape.get(5);
    let public_usage: bool = shape.get(6);
    let runtime_usage: bool = shape.get(7);
    let runtime_create: bool = shape.get(8);
    if tables != EXPECTED_TABLE_COUNT
        || functions != EXPECTED_FUNCTION_COUNT
        || owned_tables != EXPECTED_TABLE_COUNT
        || hardened_functions != EXPECTED_FUNCTION_COUNT
        || runtime_tables != 0
        || runtime_functions != EXPECTED_RUNTIME_FUNCTION_COUNT
        || public_usage
        || !runtime_usage
        || runtime_create
    {
        return Err(error(
            ExtensionSetupErrorKind::CatalogMismatch,
            "FOREMAN_EXTENSION_SHAPE_MISMATCH",
        ));
    }
    Ok(ExtensionCatalogEvidence {
        database_uuid,
        role,
        sql_sha256: manifest.sql_sha256().clone(),
        manifest_sha256: manifest.manifest_sha256().clone(),
    })
}

fn verify_exact_catalog_digests(
    client: &mut impl GenericClient,
    store_profile: StoreProfile,
) -> Result<(), ExtensionSetupError> {
    let function_digest = measure_function_catalog_digest(client)?;
    let table_digest = measure_table_catalog_digest(client)?;
    let (expected_function_digest, expected_table_digest) = match store_profile {
        StoreProfile::V7 => (
            EXPECTED_FUNCTION_CATALOG_SHA256,
            EXPECTED_TABLE_CATALOG_SHA256,
        ),
        StoreProfile::V8 => (
            EXPECTED_STORE_V8_REBOUND_FUNCTION_CATALOG_SHA256,
            EXPECTED_STORE_V8_REBOUND_TABLE_CATALOG_SHA256,
        ),
    };
    if function_digest != expected_function_digest || table_digest != expected_table_digest {
        return Err(catalog_profile_mismatch());
    }
    Ok(())
}

fn verify_closed_catalog_shape(client: &mut impl GenericClient) -> Result<(), ExtensionSetupError> {
    let shape = client
        .query_one(
            "SELECT \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_class c \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND c.relkind NOT IN ('r','i')), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND NOT t.tgisinternal), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND t.tgisinternal), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' AND t.tgisinternal \
                   AND t.tgenabled='O' AND t.tgconstraint<>0), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_trigger t \
                  JOIN pg_catalog.pg_class c ON c.oid=t.tgrelid \
                  JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                  JOIN pg_catalog.pg_constraint con ON con.oid=t.tgconstraint \
                  JOIN pg_catalog.pg_namespace cn ON cn.oid=con.connamespace \
                 WHERE n.nspname IN ('control','memory','readmodel') \
                   AND cn.nspname='foreman_execution' AND t.tgisinternal \
                   AND t.tgenabled='O' AND t.tgconstraint<>0), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_type t \
                  JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                 WHERE n.nspname='foreman_execution' \
                   AND (NOT t.typisdefined OR t.typtype IN ('d','e','p','r','m'))), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_type t \
                  JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                 WHERE n.nspname='foreman_execution'), \
                (SELECT pg_catalog.count(*) FROM ( \
                   SELECT t.oid \
                     FROM pg_catalog.pg_type t \
                     JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                     LEFT JOIN LATERAL pg_catalog.aclexplode( \
                       COALESCE(t.typacl,pg_catalog.acldefault('T',t.typowner))) acl ON TRUE \
                    WHERE n.nspname='foreman_execution' \
                    GROUP BY t.oid,t.typowner \
                   HAVING pg_catalog.count(acl.privilege_type)<>2 \
                       OR pg_catalog.count(*) FILTER (WHERE acl.grantee=0 \
                           AND acl.grantor=t.typowner \
                           AND acl.privilege_type='USAGE' AND NOT acl.is_grantable)<>1 \
                       OR pg_catalog.count(*) FILTER (WHERE acl.grantee=t.typowner \
                           AND acl.grantor=t.typowner \
                           AND acl.privilege_type='USAGE' AND NOT acl.is_grantable)<>1 \
                ) type_acl_drift), \
                ((SELECT pg_catalog.count(*) FROM pg_catalog.pg_collation x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.collnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_conversion x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.connamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_operator x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.oprnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opclass x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.opcnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_opfamily x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.opfnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_statistic_ext x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.stxnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_config x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.cfgnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_dict x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.dictnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_parser x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.prsnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_ts_template x \
                   JOIN pg_catalog.pg_namespace n ON n.oid=x.tmplnamespace \
                  WHERE n.nspname='foreman_execution')), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_default_acl d \
                  JOIN pg_catalog.pg_namespace n ON n.oid=d.defaclnamespace \
                 WHERE n.nspname='foreman_execution'), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_cast c \
                 WHERE c.castsource IN (SELECT t.oid FROM pg_catalog.pg_type t \
                       JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                      WHERE n.nspname='foreman_execution') \
                    OR c.casttarget IN (SELECT t.oid FROM pg_catalog.pg_type t \
                       JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                      WHERE n.nspname='foreman_execution') \
                    OR c.castfunc IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                       JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                      WHERE n.nspname='foreman_execution')), \
                (SELECT pg_catalog.count(*) FROM pg_catalog.pg_transform tr \
                 WHERE tr.trftype IN (SELECT t.oid FROM pg_catalog.pg_type t \
                       JOIN pg_catalog.pg_namespace n ON n.oid=t.typnamespace \
                      WHERE n.nspname='foreman_execution') \
                    OR tr.trffromsql IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                       JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                      WHERE n.nspname='foreman_execution') \
                    OR tr.trftosql IN (SELECT p.oid FROM pg_catalog.pg_proc p \
                       JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace \
                      WHERE n.nspname='foreman_execution')), \
                ((SELECT pg_catalog.count(*) FROM pg_catalog.pg_rewrite w \
                   JOIN pg_catalog.pg_class c ON c.oid=w.ev_class \
                   JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='foreman_execution' AND w.rulename<>'_RETURN') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_policy p \
                   JOIN pg_catalog.pg_class c ON c.oid=p.polrelid \
                   JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace \
                  WHERE n.nspname='foreman_execution') + \
                 (SELECT pg_catalog.count(*) FROM pg_catalog.pg_inherits i \
                   JOIN pg_catalog.pg_class child ON child.oid=i.inhrelid \
                   JOIN pg_catalog.pg_namespace child_ns ON child_ns.oid=child.relnamespace \
                   JOIN pg_catalog.pg_class parent ON parent.oid=i.inhparent \
                   JOIN pg_catalog.pg_namespace parent_ns ON parent_ns.oid=parent.relnamespace \
                  WHERE child_ns.nspname='foreman_execution' \
                     OR parent_ns.nspname='foreman_execution'))",
            &[],
        )
        .map_err(|_| catalog_profile_mismatch())?;
    let expected = [
        0_i64,
        0,
        EXPECTED_INTERNAL_TRIGGER_COUNT,
        EXPECTED_INTERNAL_TRIGGER_COUNT,
        EXPECTED_CONTROL_INTERNAL_TRIGGER_COUNT,
        0,
        EXPECTED_TYPE_COUNT,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    if expected
        .into_iter()
        .enumerate()
        .any(|(index, expected)| shape.get::<_, i64>(index) != expected)
    {
        return Err(catalog_profile_mismatch());
    }
    verify_managed_extension_dependency_closure(client)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_managed_extension_dependency_closure(
    client: &mut impl GenericClient,
) -> Result<(), ExtensionSetupError> {
    let forbidden = client
        .query_one(
            "WITH managed_namespaces(objid) AS ( \
                SELECT n.oid FROM pg_catalog.pg_namespace n \
                 WHERE n.nspname='foreman_execution' \
            ), managed_relations(objid) AS ( \
                SELECT c.oid FROM pg_catalog.pg_class c \
                 WHERE c.relnamespace IN (SELECT objid FROM managed_namespaces) \
            ), managed_constraints(objid) AS ( \
                SELECT c.oid FROM pg_catalog.pg_constraint c \
                 WHERE c.connamespace IN (SELECT objid FROM managed_namespaces) \
            ), managed_toast_relations(objid) AS ( \
                SELECT c.reltoastrelid FROM pg_catalog.pg_class c \
                 WHERE c.oid IN (SELECT objid FROM managed_relations) \
                   AND c.reltoastrelid<>0 \
                UNION \
                SELECT i.indexrelid FROM pg_catalog.pg_index i \
                 WHERE i.indrelid IN ( \
                    SELECT c.reltoastrelid FROM pg_catalog.pg_class c \
                     WHERE c.oid IN (SELECT objid FROM managed_relations) \
                       AND c.reltoastrelid<>0) \
            ), managed_functions(objid) AS ( \
                SELECT p.oid FROM pg_catalog.pg_proc p \
                 WHERE p.pronamespace IN (SELECT objid FROM managed_namespaces) \
            ), managed_types(objid) AS ( \
                SELECT t.oid FROM pg_catalog.pg_type t \
                 WHERE t.typnamespace IN (SELECT objid FROM managed_namespaces) \
                    OR t.typrelid IN (SELECT objid FROM managed_toast_relations) \
            ), managed_casts(objid) AS ( \
                SELECT c.oid FROM pg_catalog.pg_cast c \
                 WHERE c.castsource IN (SELECT objid FROM managed_types) \
                    OR c.casttarget IN (SELECT objid FROM managed_types) \
                    OR c.castfunc IN (SELECT objid FROM managed_functions) \
            ), managed_transforms(objid) AS ( \
                SELECT tr.oid FROM pg_catalog.pg_transform tr \
                 WHERE tr.trftype IN (SELECT objid FROM managed_types) \
                    OR tr.trffromsql IN (SELECT objid FROM managed_functions) \
                    OR tr.trftosql IN (SELECT objid FROM managed_functions) \
            ), managed(classid,objid) AS ( \
                SELECT 'pg_namespace'::pg_catalog.regclass::oid,objid \
                  FROM managed_namespaces \
                UNION \
                SELECT 'pg_class'::pg_catalog.regclass::oid,objid FROM managed_relations \
                UNION \
                SELECT 'pg_class'::pg_catalog.regclass::oid,objid FROM managed_toast_relations \
                UNION \
                SELECT 'pg_proc'::pg_catalog.regclass::oid,objid FROM managed_functions \
                UNION \
                SELECT 'pg_type'::pg_catalog.regclass::oid,objid FROM managed_types \
                UNION \
                SELECT 'pg_cast'::pg_catalog.regclass::oid,objid FROM managed_casts \
                UNION \
                SELECT 'pg_transform'::pg_catalog.regclass::oid,objid \
                  FROM managed_transforms \
                UNION \
                SELECT 'pg_constraint'::pg_catalog.regclass::oid,objid \
                  FROM managed_constraints \
                UNION \
                SELECT 'pg_attrdef'::pg_catalog.regclass::oid,a.oid \
                  FROM pg_catalog.pg_attrdef a \
                 WHERE a.adrelid IN (SELECT objid FROM managed_relations) \
                UNION \
                SELECT 'pg_trigger'::pg_catalog.regclass::oid,t.oid \
                  FROM pg_catalog.pg_trigger t \
                 WHERE t.tgrelid IN (SELECT objid FROM managed_relations) \
                    OR t.tgconstraint IN (SELECT objid FROM managed_constraints) \
                UNION \
                SELECT 'pg_rewrite'::pg_catalog.regclass::oid,r.oid \
                  FROM pg_catalog.pg_rewrite r \
                 WHERE r.ev_class IN (SELECT objid FROM managed_relations) \
                UNION \
                SELECT 'pg_policy'::pg_catalog.regclass::oid,p.oid \
                  FROM pg_catalog.pg_policy p \
                 WHERE p.polrelid IN (SELECT objid FROM managed_relations) \
                UNION \
                SELECT 'pg_statistic_ext'::pg_catalog.regclass::oid,s.oid \
                  FROM pg_catalog.pg_statistic_ext s \
                 WHERE s.stxnamespace IN (SELECT objid FROM managed_namespaces) \
            ) \
            SELECT pg_catalog.count(*) FROM pg_catalog.pg_depend d \
              JOIN managed dependent \
                ON dependent.classid=d.classid AND dependent.objid=d.objid \
             WHERE d.deptype IN ('e','x')",
            &[],
        )
        .map_err(|_| catalog_profile_mismatch())?;
    if forbidden.get::<_, i64>(0) != 0 {
        return Err(catalog_profile_mismatch());
    }
    Ok(())
}

fn measure_function_catalog_digest(
    client: &mut impl GenericClient,
) -> Result<String, ExtensionSetupError> {
    let function_rows = client
        .query(
            "WITH function_profile AS ( \
                SELECT 1 AS kind,p.proname::text AS function_name, \
                       pg_catalog.pg_get_function_identity_arguments(p.oid)::text \
                           AS identity_arguments,''::text AS item_key, \
                       pg_catalog.json_build_array( \
                           'FUNCTION_PROFILE',p.proname, \
                           pg_catalog.pg_get_function_identity_arguments(p.oid), \
                           pg_catalog.pg_get_function_result(p.oid), \
                           pg_catalog.pg_get_functiondef(p.oid),owner.rolname, \
                           pg_catalog.has_function_privilege( \
                               'lattice_runtime',p.oid,'EXECUTE'), \
                           EXISTS (SELECT 1 FROM pg_catalog.aclexplode( \
                               COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) \
                               AS public_acl WHERE public_acl.grantee=0 \
                               AND public_acl.privilege_type='EXECUTE') \
                       )::text AS value \
                  FROM pg_catalog.pg_proc AS p \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                  JOIN pg_catalog.pg_roles AS owner ON owner.oid=p.proowner \
                 WHERE n.nspname='foreman_execution' \
                UNION ALL \
                SELECT 2,p.proname::text, \
                       pg_catalog.pg_get_function_identity_arguments(p.oid)::text, \
                       pg_catalog.json_build_array( \
                           CASE WHEN acl.grantee=0 THEN 'PUBLIC' \
                                ELSE grantee.rolname END,grantor.rolname, \
                           acl.privilege_type,acl.is_grantable)::text, \
                       pg_catalog.json_build_array( \
                           'FUNCTION_ACL',p.proname, \
                           pg_catalog.pg_get_function_identity_arguments(p.oid), \
                           CASE WHEN acl.grantee=0 THEN 'PUBLIC' \
                                ELSE grantee.rolname END,grantor.rolname, \
                           acl.privilege_type,acl.is_grantable)::text \
                  FROM pg_catalog.pg_proc AS p \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid=p.pronamespace \
                  CROSS JOIN LATERAL pg_catalog.aclexplode( \
                       COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) AS acl \
                  LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid=acl.grantee \
                  JOIN pg_catalog.pg_roles AS grantor ON grantor.oid=acl.grantor \
                 WHERE n.nspname='foreman_execution' \
            ) SELECT value FROM function_profile \
               ORDER BY kind,function_name,identity_arguments,item_key",
            &[],
        )
        .map_err(|_| catalog_profile_query_error())?;
    Ok(catalog_rows_sha256(FUNCTION_CATALOG_DOMAIN, &function_rows))
}

#[allow(clippy::too_many_lines)]
fn measure_table_catalog_digest(
    client: &mut impl GenericClient,
) -> Result<String, ExtensionSetupError> {
    let table_rows = client
        .query(
            "WITH profile AS ( \
                SELECT 0 AS kind,n.nspname::text AS relation_name,''::text AS item_key, \
                       pg_catalog.json_build_array( \
                           'SCHEMA_PROFILE',n.nspname,schema_owner.rolname, \
                           pg_catalog.obj_description(n.oid,'pg_namespace'))::text AS value \
                  FROM pg_catalog.pg_namespace AS n \
                  JOIN pg_catalog.pg_roles AS schema_owner ON schema_owner.oid = n.nspowner \
                 WHERE n.nspname='foreman_execution' \
                UNION ALL \
                SELECT 1,n.nspname::text, \
                       pg_catalog.json_build_array( \
                           CASE WHEN acl.grantee=0 THEN 'PUBLIC' \
                                ELSE grantee.rolname END,grantor.rolname, \
                           acl.privilege_type,acl.is_grantable)::text, \
                       pg_catalog.json_build_array( \
                           'SCHEMA_ACL',n.nspname, \
                           CASE WHEN acl.grantee=0 THEN 'PUBLIC' \
                                ELSE grantee.rolname END,grantor.rolname, \
                           acl.privilege_type,acl.is_grantable)::text \
                  FROM pg_catalog.pg_namespace AS n \
                  CROSS JOIN LATERAL pg_catalog.aclexplode( \
                        COALESCE(n.nspacl, pg_catalog.acldefault('n',n.nspowner))) AS acl \
                  LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid=acl.grantee \
                  JOIN pg_catalog.pg_roles AS grantor ON grantor.oid=acl.grantor \
                 WHERE n.nspname='foreman_execution' \
                UNION ALL \
                SELECT 2,c.relname::text,''::text, \
                        pg_catalog.json_build_array( \
                            'TABLE',c.relname,owner.rolname,c.relrowsecurity, \
                            c.relforcerowsecurity,c.relreplident,c.relpersistence, \
                            COALESCE(pg_catalog.array_to_string(c.reloptions,','),'<NULL>'), \
                            COALESCE(pg_catalog.array_to_string(toast.reloptions,','),'<NULL>'), \
                            pg_catalog.obj_description(c.oid,'pg_class'))::text \
                  FROM pg_catalog.pg_class AS c \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                  JOIN pg_catalog.pg_roles AS owner ON owner.oid=c.relowner \
                  LEFT JOIN pg_catalog.pg_class AS toast ON toast.oid=c.reltoastrelid \
                 WHERE n.nspname='foreman_execution' AND c.relkind='r' \
                UNION ALL \
                SELECT 3,c.relname::text,pg_catalog.lpad(a.attnum::text,5,'0'), \
                       pg_catalog.json_build_array( \
                            'COLUMN',c.relname,a.attnum,a.attname, \
                            pg_catalog.format_type(a.atttypid,a.atttypmod), \
                            coll_ns.nspname,coll.collname,a.attnotnull,a.attisdropped,a.attidentity, \
                            a.attgenerated,a.attstorage,a.attcompression,a.attstattarget, \
                            COALESCE(pg_catalog.array_to_string(a.attoptions,','),'<NULL>'), \
                            pg_catalog.pg_get_expr(d.adbin,d.adrelid), \
                            pg_catalog.col_description(c.oid,a.attnum))::text \
                  FROM pg_catalog.pg_class AS c \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                  JOIN pg_catalog.pg_attribute AS a \
                    ON a.attrelid=c.oid AND a.attnum>0 \
                  LEFT JOIN pg_catalog.pg_attrdef AS d \
                     ON d.adrelid=c.oid AND d.adnum=a.attnum \
                  LEFT JOIN pg_catalog.pg_collation AS coll ON coll.oid=a.attcollation \
                  LEFT JOIN pg_catalog.pg_namespace AS coll_ns ON coll_ns.oid=coll.collnamespace \
                 WHERE n.nspname='foreman_execution' AND c.relkind='r' \
                UNION ALL \
                SELECT 4,c.relname::text,k.conname::text, \
                        pg_catalog.json_build_array( \
                            'CONSTRAINT',c.relname,k.conname,k.contype, \
                            pg_catalog.pg_get_constraintdef(k.oid,false), \
                            pg_catalog.obj_description(k.oid,'pg_constraint'))::text \
                  FROM pg_catalog.pg_constraint AS k \
                  JOIN pg_catalog.pg_class AS c ON c.oid=k.conrelid \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                 WHERE n.nspname='foreman_execution' \
                UNION ALL \
                SELECT 5,t.relname::text,i.relname::text, \
                       pg_catalog.json_build_array( \
                            'INDEX',t.relname,i.relname, \
                            pg_catalog.pg_get_indexdef(i.oid), \
                            COALESCE(pg_catalog.array_to_string(i.reloptions,','),'<NULL>'), \
                            pg_catalog.obj_description(i.oid,'pg_class'))::text \
                  FROM pg_catalog.pg_index AS x \
                  JOIN pg_catalog.pg_class AS i ON i.oid=x.indexrelid \
                  JOIN pg_catalog.pg_class AS t ON t.oid=x.indrelid \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid=t.relnamespace \
                 WHERE n.nspname='foreman_execution' \
                UNION ALL \
                SELECT 6,c.relname::text, \
                       pg_catalog.json_build_array( \
                           CASE WHEN acl.grantee=0 THEN 'PUBLIC' \
                                ELSE grantee.rolname END,grantor.rolname, \
                           acl.privilege_type,acl.is_grantable)::text, \
                       pg_catalog.json_build_array( \
                           'TABLE_ACL',c.relname, \
                           CASE WHEN acl.grantee=0 THEN 'PUBLIC' \
                                ELSE grantee.rolname END,grantor.rolname, \
                           acl.privilege_type,acl.is_grantable)::text \
                  FROM pg_catalog.pg_class AS c \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                  CROSS JOIN LATERAL pg_catalog.aclexplode( \
                       COALESCE(c.relacl,pg_catalog.acldefault('r',c.relowner))) AS acl \
                  LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid=acl.grantee \
                  JOIN pg_catalog.pg_roles AS grantor ON grantor.oid=acl.grantor \
                 WHERE n.nspname='foreman_execution' AND c.relkind='r' \
                UNION ALL \
                SELECT 7,c.relname::text, \
                       pg_catalog.lpad(a.attnum::text,5,'0') || ':' || \
                       pg_catalog.json_build_array( \
                           CASE WHEN acl.grantee=0 THEN 'PUBLIC' \
                                ELSE grantee.rolname END,grantor.rolname, \
                           acl.privilege_type,acl.is_grantable)::text, \
                       pg_catalog.json_build_array( \
                           'TABLE_COLUMN_ACL',c.relname,a.attnum,a.attname, \
                           CASE WHEN acl.grantee=0 THEN 'PUBLIC' \
                                ELSE grantee.rolname END,grantor.rolname, \
                           acl.privilege_type,acl.is_grantable)::text \
                  FROM pg_catalog.pg_class AS c \
                  JOIN pg_catalog.pg_namespace AS n ON n.oid=c.relnamespace \
                  JOIN pg_catalog.pg_attribute AS a \
                    ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped \
                  CROSS JOIN LATERAL pg_catalog.aclexplode(a.attacl) AS acl \
                  LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid=acl.grantee \
                  JOIN pg_catalog.pg_roles AS grantor ON grantor.oid=acl.grantor \
                 WHERE n.nspname='foreman_execution' AND c.relkind='r' \
            ) SELECT value FROM profile ORDER BY kind,relation_name,item_key",
            &[],
        )
        .map_err(|_| catalog_profile_query_error())?;
    Ok(catalog_rows_sha256(TABLE_CATALOG_DOMAIN, &table_rows))
}

fn catalog_rows_sha256(domain: &[u8], rows: &[postgres::Row]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for row in rows {
        let value: String = row.get(0);
        update_framed(&mut hasher, value.as_bytes());
    }
    bytes_to_hex(&hasher.finalize())
}

const fn catalog_profile_query_error() -> ExtensionSetupError {
    error(
        ExtensionSetupErrorKind::CatalogMismatch,
        "FOREMAN_EXTENSION_CATALOG_PROFILE_QUERY_FAILED",
    )
}

const fn catalog_profile_mismatch() -> ExtensionSetupError {
    error(
        ExtensionSetupErrorKind::CatalogMismatch,
        "FOREMAN_EXTENSION_CATALOG_PROFILE_MISMATCH",
    )
}

fn valid_database_name(value: &str) -> bool {
    (3..=63).contains(&value.len())
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_run_id(value: &str) -> bool {
    (3..=128).contains(&value.len())
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn derive_database_identity(
    database_name: &str,
    run_id: &str,
) -> Result<(String, ContentDigest), ExtensionSetupError> {
    let mut hasher = Sha256::new();
    hasher.update(DATABASE_IDENTITY_DOMAIN);
    update_framed(&mut hasher, database_name.as_bytes());
    update_framed(&mut hasher, run_id.as_bytes());
    let digest = hasher.finalize();
    let digest_hex = bytes_to_hex(&digest);
    let identity = ContentDigest::from_sha256(digest_hex).map_err(|_| global_error())?;
    let mut uuid_bytes = [0_u8; 16];
    uuid_bytes.copy_from_slice(&digest[..16]);
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x80;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    Ok((format_uuid(uuid_bytes), identity))
}

fn update_framed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u64::try_from(value.len())
            .expect("bounded target")
            .to_be_bytes(),
    );
    hasher.update(value);
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn format_uuid(bytes: [u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(36);
    for (index, byte) in bytes.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            output.push('-');
        }
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

const fn error(kind: ExtensionSetupErrorKind, code: &'static str) -> ExtensionSetupError {
    ExtensionSetupError { kind, code }
}

const fn transaction_error() -> ExtensionSetupError {
    error(
        ExtensionSetupErrorKind::TransactionFailed,
        "FOREMAN_EXTENSION_TRANSACTION_FAILED",
    )
}

const fn transaction_stage_error(code: &'static str) -> ExtensionSetupError {
    error(ExtensionSetupErrorKind::TransactionFailed, code)
}

fn restage_transaction_failure(
    failure: ExtensionSetupError,
    code: &'static str,
) -> ExtensionSetupError {
    if matches!(failure.kind, ExtensionSetupErrorKind::TransactionFailed)
        && failure.code == "FOREMAN_EXTENSION_TRANSACTION_FAILED"
    {
        transaction_stage_error(code)
    } else {
        failure
    }
}

const fn global_error() -> ExtensionSetupError {
    error(
        ExtensionSetupErrorKind::GlobalIdentityMismatch,
        "FOREMAN_EXTENSION_GLOBAL_IDENTITY_MISMATCH",
    )
}

const fn runtime_admission_error() -> ExtensionSetupError {
    error(
        ExtensionSetupErrorKind::GlobalIdentityMismatch,
        "FOREMAN_EXTENSION_RUNTIME_ADMISSION_NOT_STOPPED",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_preflight_allows_only_exact_v7_or_v8_store_profiles() {
        assert!(supported_store_profile(
            i32::from(REQUIRED_GLOBAL_SCHEMA_VERSION),
            REQUIRED_GLOBAL_MANIFEST_SHA256
        ));
        assert!(supported_store_profile(8, STORE_V8_GLOBAL_MANIFEST_SHA256));
        assert!(!supported_store_profile(8, REQUIRED_GLOBAL_MANIFEST_SHA256));
        assert!(!supported_store_profile(
            i32::from(REQUIRED_GLOBAL_SCHEMA_VERSION),
            STORE_V8_GLOBAL_MANIFEST_SHA256
        ));
        assert!(!supported_store_profile(
            8,
            "01373ed5092e90bf6a9e383955cd70d0fd4e0ed821667f1905b69e313005ea82"
        ));
    }

    #[test]
    #[ignore = "requires the coordinator-owned disposable Foreman extension fixture"]
    fn measure_catalog_digests() {
        let connection = std::env::var("LATTICE_FOREMAN_CATALOG_SIGNATURE_URL")
            .expect("coordinator supplies fixture URL");
        let mut client =
            Client::connect(&connection, postgres::NoTls).expect("connect to coordinated fixture");
        client
            .batch_execute("SET search_path = pg_catalog")
            .expect("harden measurement search path");

        if std::env::var("LATTICE_FOREMAN_MEASURE_STORE_V8_REBOUND").as_deref() == Ok("1") {
            client
                .batch_execute("SET ROLE lattice_migrator; BEGIN; SET LOCAL search_path=pg_catalog")
                .expect("start rebound measurement transaction");
            for sql in [
                include_str!("../../../db/extensions/writer-lease/v5-store-v8-rebind.sql"),
                include_str!("../../../db/migrations/0009_external_verified_result_adoption.sql"),
                include_str!("../../../db/migrations/0010_store_v8_runtime_successor.sql"),
                include_str!("../../../db/extensions/foreman-execution/v1-store-v8-rebind.sql"),
            ] {
                client
                    .batch_execute(sql)
                    .expect("apply rebound measurement asset");
            }
            let function_digest =
                measure_function_catalog_digest(&mut client).expect("rebound function digest");
            let table_digest =
                measure_table_catalog_digest(&mut client).expect("rebound table digest");
            client
                .batch_execute("ROLLBACK; RESET ROLE")
                .expect("rollback rebound measurement transaction");
            println!("FOREMAN_STORE_V8_REBOUND_FUNCTION_CATALOG_SHA256={function_digest}");
            println!("FOREMAN_STORE_V8_REBOUND_TABLE_CATALOG_SHA256={table_digest}");
            return;
        }

        let function_digest =
            measure_function_catalog_digest(&mut client).expect("function catalog digest");
        let table_digest = measure_table_catalog_digest(&mut client).expect("table catalog digest");
        client
            .batch_execute(
                "SET ROLE lattice_migrator; \
                 BEGIN; \
                 REVOKE USAGE ON SCHEMA foreman_execution \
                    FROM lattice_guardian, lattice_readonly",
            )
            .expect("stage predecessor schema ACL");
        let predecessor_table_digest =
            measure_table_catalog_digest(&mut client).expect("predecessor table catalog digest");
        client
            .batch_execute("ROLLBACK; RESET ROLE")
            .expect("restore current catalog");
        println!("FOREMAN_FUNCTION_CATALOG_SHA256={function_digest}");
        println!("FOREMAN_TABLE_CATALOG_SHA256={table_digest}");
        println!("FOREMAN_PREDECESSOR_TABLE_CATALOG_SHA256={predecessor_table_digest}");
    }
}
