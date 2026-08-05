use std::error::Error;
use std::fmt;

use lattice_contracts::{CodebaseMemoryPersistenceIdentity, ContentDigest};
use postgres::{Client, GenericClient, IsolationLevel};
use sha2::{Digest, Sha256};

use crate::{
    CODEBASE_MEMORY_EXTENSION_ID, CODEBASE_MEMORY_EXTENSION_PATH,
    CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION, verify_embedded_extension_manifest,
};

const SUPPORTED_POSTGRES_MAJOR: u32 = 17;
const REQUIRED_GLOBAL_SCHEMA_VERSION: u16 = 3;
const REQUIRED_GLOBAL_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const DATABASE_IDENTITY_DOMAIN: &[u8] = b"LATTICE_POSTGRES_DATABASE_IDENTITY_V1\0";
const EXTENSION_ADVISORY_LOCK: i64 = 0x4c41_5443_4d45_4d31;

const EXPECTED_TABLES: [&str; 8] = [
    "codebase_memory_analyses",
    "codebase_memory_extension_identity",
    "codebase_memory_extension_ledger",
    "codebase_memory_receipts",
    "codebase_memory_records",
    "codebase_memory_reflections",
    "codebase_memory_retrieval_audits",
    "openclaw_gateway_commands",
];
const EXPECTED_FUNCTIONS: [&str; 7] = [
    "codebase_memory_load_receipt_v1",
    "codebase_memory_load_reflection_v2",
    "codebase_memory_persist_analysis_v1",
    "codebase_memory_persist_reflection_v2",
    "codebase_memory_persist_retrieval_v1",
    "openclaw_gateway_finalize_terminal_v1",
    "openclaw_gateway_reconcile_and_claim_v1",
];

/// Closed database roles admitted by the extension setup and verifier.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExtensionDatabaseRole {
    Migrator,
    Runtime,
}

impl ExtensionDatabaseRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Migrator => "lattice_migrator",
            Self::Runtime => "lattice_runtime",
        }
    }

    #[must_use]
    pub const fn login_role(self) -> &'static str {
        match self {
            Self::Migrator => "lattice_migrator_login",
            Self::Runtime => "lattice_runtime_login",
        }
    }
}

/// Exact marker-owned disposable database target accepted by the extension.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionTarget {
    database_name: String,
    run_id: String,
    expected_database_uuid: String,
    expected_database_identity_digest: ContentDigest,
}

impl ExtensionTarget {
    /// Constructs the same fixed TASK-019 database identity without depending
    /// on the Postgres Store crate.
    ///
    /// # Errors
    ///
    /// Rejects default, unmarked, malformed, or unbounded targets.
    pub fn new(
        database_name: impl Into<String>,
        run_id: impl Into<String>,
    ) -> Result<Self, ExtensionSetupError> {
        let database_name = database_name.into();
        let run_id = run_id.into();
        let suffix = database_name.strip_prefix("lattice_task019_");
        let valid_database = database_name.len() <= 63
            && suffix.is_some_and(|value| {
                !value.is_empty()
                    && value.len() <= 32
                    && value.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'
                    })
            });
        let valid_run = run_id.len() == 32
            && run_id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid_database || !valid_run {
            return Err(ExtensionSetupError::new(
                ExtensionSetupErrorKind::TargetMismatch,
                "MEMORY_EXTENSION_TARGET_MISMATCH",
            ));
        }
        let (expected_database_uuid, expected_database_identity_digest) =
            derive_database_identity(&database_name, &run_id)?;
        Ok(Self {
            database_name,
            run_id,
            expected_database_uuid,
            expected_database_identity_digest,
        })
    }

    #[must_use]
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    #[must_use]
    pub fn database_comment(&self) -> String {
        format!("LATTICE_DEVOS_DISPOSABLE_V1:{}", self.run_id)
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

/// Successful result of one explicit administrative extension attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtensionApplyOutcome {
    Installed,
    AlreadyCurrent,
}

/// Read-only exact database/extension identity evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionCatalogEvidence {
    database_uuid: String,
    server_version_num: u32,
    role: ExtensionDatabaseRole,
    identity: CodebaseMemoryPersistenceIdentity,
}

impl ExtensionCatalogEvidence {
    #[must_use]
    pub fn database_uuid(&self) -> &str {
        &self.database_uuid
    }

    #[must_use]
    pub const fn server_version_num(&self) -> u32 {
        self.server_version_num
    }

    #[must_use]
    pub const fn role(&self) -> ExtensionDatabaseRole {
        self.role
    }

    #[must_use]
    pub const fn identity(&self) -> &CodebaseMemoryPersistenceIdentity {
        &self.identity
    }
}

/// Stable setup failure categories for the independent extension.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ExtensionSetupErrorKind {
    ManifestInvalid,
    TargetMismatch,
    TargetUnowned,
    GlobalProfileMismatch,
    InstallationRequired,
    PartialProfile,
    SchemaCollision,
    PermissionDenied,
    ServerUnsupported,
    UnsafeSetting,
    TransactionFailed,
    CommitOutcomeUnknown,
    CatalogMismatch,
    PostApplyVerificationFailed,
}

/// Bounded extension setup failure without database or credential text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtensionSetupError {
    kind: ExtensionSetupErrorKind,
    code: &'static str,
}

impl ExtensionSetupError {
    const fn new(kind: ExtensionSetupErrorKind, code: &'static str) -> Self {
        Self { kind, code }
    }

    #[must_use]
    pub const fn kind(&self) -> ExtensionSetupErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ExtensionSetupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code)
    }
}

impl Error for ExtensionSetupError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExtensionPreState {
    Fresh,
    ExactV2,
    Partial,
    Collision,
}

/// Runs the fixed administrative extension transaction.
///
/// # Errors
///
/// Fails closed for every target, global-v3, role, setting, partial, collision,
/// transaction, or identity mismatch.
pub fn apply_extension(
    client: &mut Client,
    target: &ExtensionTarget,
) -> Result<ExtensionApplyOutcome, ExtensionSetupError> {
    let manifest = verify_embedded_extension_manifest().map_err(|_| {
        ExtensionSetupError::new(
            ExtensionSetupErrorKind::ManifestInvalid,
            "MEMORY_EXTENSION_MANIFEST_INVALID",
        )
    })?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::ReadCommitted)
        .start()
        .map_err(|_| stage_error("MEMORY_EXTENSION_TRANSACTION_START_FAILED"))?;
    harden_transaction(&mut transaction)?;
    transaction
        .query_one(
            "SELECT pg_catalog.pg_advisory_xact_lock($1)",
            &[&EXTENSION_ADVISORY_LOCK],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_ADVISORY_LOCK_FAILED"))?;
    let server_version_num = preflight(&mut transaction, target, ExtensionDatabaseRole::Migrator)?;
    match classify_pre_state(&mut transaction)? {
        ExtensionPreState::Fresh => {
            let sql = std::str::from_utf8(manifest.bytes()).map_err(|_| {
                ExtensionSetupError::new(
                    ExtensionSetupErrorKind::ManifestInvalid,
                    "MEMORY_EXTENSION_MANIFEST_INVALID",
                )
            })?;
            transaction
                .batch_execute(sql)
                .map_err(|error| map_extension_sql_error(&error))?;
            insert_identity(&mut transaction, target, &manifest)?;
            if classify_pre_state(&mut transaction)? != ExtensionPreState::ExactV2 {
                return Err(catalog_error());
            }
            verify_catalog_closure(&mut transaction)?;
            read_identity(
                &mut transaction,
                target,
                ExtensionDatabaseRole::Migrator,
                server_version_num,
                &manifest,
            )?;
            transaction.commit().map_err(|_| {
                ExtensionSetupError::new(
                    ExtensionSetupErrorKind::CommitOutcomeUnknown,
                    "MEMORY_EXTENSION_COMMIT_OUTCOME_UNKNOWN",
                )
            })?;
            verify_extension(client, target, ExtensionDatabaseRole::Migrator).map_err(|_| {
                ExtensionSetupError::new(
                    ExtensionSetupErrorKind::PostApplyVerificationFailed,
                    "MEMORY_EXTENSION_POST_APPLY_VERIFICATION_FAILED",
                )
            })?;
            Ok(ExtensionApplyOutcome::Installed)
        }
        ExtensionPreState::Partial => Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PartialProfile,
            "MEMORY_EXTENSION_PARTIAL_PROFILE",
        )),
        ExtensionPreState::Collision => Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::SchemaCollision,
            "MEMORY_EXTENSION_SCHEMA_COLLISION",
        )),
        ExtensionPreState::ExactV2 => {
            verify_catalog_closure(&mut transaction)?;
            read_identity(
                &mut transaction,
                target,
                ExtensionDatabaseRole::Migrator,
                server_version_num,
                &manifest,
            )?;
            transaction.commit().map_err(|_| {
                ExtensionSetupError::new(
                    ExtensionSetupErrorKind::CommitOutcomeUnknown,
                    "MEMORY_EXTENSION_COMMIT_OUTCOME_UNKNOWN",
                )
            })?;
            Ok(ExtensionApplyOutcome::AlreadyCurrent)
        }
    }
}

/// Verifies the exact catalog/ACL profile and typed identity without mutation.
///
/// # Errors
///
/// Fails closed for a fresh, partial, colliding, substituted, or unavailable
/// profile.
pub fn verify_extension(
    client: &mut Client,
    target: &ExtensionTarget,
    role: ExtensionDatabaseRole,
) -> Result<ExtensionCatalogEvidence, ExtensionSetupError> {
    if role != ExtensionDatabaseRole::Migrator {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PermissionDenied,
            "MEMORY_EXTENSION_ADMIN_VERIFIER_REQUIRED",
        ));
    }
    let manifest = verify_embedded_extension_manifest().map_err(|_| {
        ExtensionSetupError::new(
            ExtensionSetupErrorKind::ManifestInvalid,
            "MEMORY_EXTENSION_MANIFEST_INVALID",
        )
    })?;
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(|_| transaction_error())?;
    harden_transaction(&mut transaction)?;
    let server_version_num = preflight(&mut transaction, target, role)?;
    let state = classify_pre_state(&mut transaction)?;
    if state != ExtensionPreState::ExactV2 {
        return Err(match state {
            ExtensionPreState::Fresh => ExtensionSetupError::new(
                ExtensionSetupErrorKind::InstallationRequired,
                "MEMORY_EXTENSION_INSTALLATION_REQUIRED",
            ),
            ExtensionPreState::Partial => ExtensionSetupError::new(
                ExtensionSetupErrorKind::PartialProfile,
                "MEMORY_EXTENSION_PARTIAL_PROFILE",
            ),
            ExtensionPreState::Collision => ExtensionSetupError::new(
                ExtensionSetupErrorKind::SchemaCollision,
                "MEMORY_EXTENSION_SCHEMA_COLLISION",
            ),
            ExtensionPreState::ExactV2 => unreachable!(),
        });
    }
    verify_catalog_closure(&mut transaction)?;
    let evidence = read_identity(
        &mut transaction,
        target,
        role,
        server_version_num,
        &manifest,
    )?;
    transaction.commit().map_err(|_| transaction_error())?;
    Ok(evidence)
}

fn harden_transaction(client: &mut impl GenericClient) -> Result<(), ExtensionSetupError> {
    client
        .batch_execute(
            "SET LOCAL search_path = pg_catalog; \
             SET LOCAL lock_timeout = '5s'; \
             SET LOCAL statement_timeout = '30s'; \
             SET LOCAL idle_in_transaction_session_timeout = '30s'; \
             SET LOCAL synchronous_commit = on;",
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_TRANSACTION_HARDEN_FAILED"))
}

fn preflight(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    role: ExtensionDatabaseRole,
) -> Result<u32, ExtensionSetupError> {
    let row = client
        .query_one(
            "SELECT current_database()::text, session_user::text, current_user::text, \
                    current_setting('role')::text, \
                    current_setting('server_version_num')::integer, \
                    d.description::text \
               FROM pg_catalog.pg_database AS db \
               LEFT JOIN pg_catalog.pg_shdescription AS d \
                 ON d.objoid = db.oid AND d.classoid = 'pg_database'::regclass \
              WHERE db.datname = current_database()",
            &[],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_PREFLIGHT_QUERY_FAILED"))?;
    let database_name: String = row.get(0);
    let session_user: String = row.get(1);
    let current_user: String = row.get(2);
    let current_role: String = row.get(3);
    let server_version_num: i32 = row.get(4);
    let database_comment: Option<String> = row.get(5);
    if database_name != target.database_name() {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::TargetMismatch,
            "MEMORY_EXTENSION_TARGET_MISMATCH",
        ));
    }
    if database_comment.as_deref() != Some(target.database_comment().as_str()) {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::TargetUnowned,
            "MEMORY_EXTENSION_TARGET_UNOWNED",
        ));
    }
    if session_user != role.login_role()
        || current_user != role.as_str()
        || current_role != role.as_str()
    {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::PermissionDenied,
            "MEMORY_EXTENSION_ROLE_MISMATCH",
        ));
    }
    let server_version_num = u32::try_from(server_version_num).map_err(|_| {
        ExtensionSetupError::new(
            ExtensionSetupErrorKind::ServerUnsupported,
            "MEMORY_EXTENSION_SERVER_UNSUPPORTED",
        )
    })?;
    if server_version_num / 10_000 != SUPPORTED_POSTGRES_MAJOR {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::ServerUnsupported,
            "MEMORY_EXTENSION_SERVER_UNSUPPORTED",
        ));
    }

    let global = client
        .query_one(
            "SELECT d.database_uuid::text, c.current_schema_version::integer, \
                    btrim(c.manifest_sha256)::text \
               FROM ONLY control.database_identity AS d \
               CROSS JOIN ONLY control.schema_compatibility AS c \
              WHERE d.singleton AND c.singleton",
            &[],
        )
        .map_err(|_| {
            ExtensionSetupError::new(
                ExtensionSetupErrorKind::GlobalProfileMismatch,
                "MEMORY_EXTENSION_GLOBAL_PROFILE_MISMATCH",
            )
        })?;
    let database_uuid: String = global.get(0);
    let global_version: i32 = global.get(1);
    let global_manifest: String = global.get(2);
    if database_uuid != target.expected_database_uuid()
        || global_version != i32::from(REQUIRED_GLOBAL_SCHEMA_VERSION)
        || global_manifest != REQUIRED_GLOBAL_MANIFEST_SHA256
    {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::GlobalProfileMismatch,
            "MEMORY_EXTENSION_GLOBAL_PROFILE_MISMATCH",
        ));
    }
    Ok(server_version_num)
}

fn classify_pre_state(
    client: &mut impl GenericClient,
) -> Result<ExtensionPreState, ExtensionSetupError> {
    let row = client
        .query_one(
            "SELECT \
                count(*) FILTER (WHERE c.relname IN ( \
                    'codebase_memory_analyses', \
                    'codebase_memory_extension_identity', \
                    'codebase_memory_extension_ledger', \
                    'codebase_memory_receipts', \
                    'codebase_memory_records', \
                    'codebase_memory_reflections', \
                    'codebase_memory_retrieval_audits', \
                    'openclaw_gateway_commands' \
                ))::bigint, \
                count(*)::bigint \
               FROM pg_catalog.pg_class AS c \
               JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
              WHERE n.nspname = 'memory' AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')",
            &[],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_PRESTATE_TABLE_QUERY_FAILED"))?;
    let expected_tables: i64 = row.get(0);
    let all_tables: i64 = row.get(1);
    let row = client
        .query_one(
            "SELECT \
                count(*) FILTER (WHERE p.proname IN ( \
                    'codebase_memory_load_receipt_v1', \
                    'codebase_memory_load_reflection_v2', \
                    'codebase_memory_persist_analysis_v1', \
                    'codebase_memory_persist_reflection_v2', \
                    'codebase_memory_persist_retrieval_v1', \
                    'openclaw_gateway_finalize_terminal_v1', \
                    'openclaw_gateway_reconcile_and_claim_v1' \
                ))::bigint, \
                count(*)::bigint \
               FROM pg_catalog.pg_proc AS p \
               JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
              WHERE n.nspname = 'memory'",
            &[],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_PRESTATE_FUNCTION_QUERY_FAILED"))?;
    let expected_functions: i64 = row.get(0);
    let all_functions: i64 = row.get(1);
    if all_tables == 0 && all_functions == 0 {
        return Ok(ExtensionPreState::Fresh);
    }
    if all_tables != i64::try_from(EXPECTED_TABLES.len()).expect("fixed table count")
        || expected_tables != all_tables
        || all_functions != i64::try_from(EXPECTED_FUNCTIONS.len()).expect("fixed function count")
        || expected_functions != all_functions
    {
        let has_expected = expected_tables > 0 || expected_functions > 0;
        let has_unknown = expected_tables != all_tables || expected_functions != all_functions;
        return Ok(if has_unknown {
            ExtensionPreState::Collision
        } else if has_expected {
            ExtensionPreState::Partial
        } else {
            ExtensionPreState::Collision
        });
    }
    let identity: i64 = client
        .query_one(
            "SELECT (SELECT count(*) FROM ONLY memory.codebase_memory_extension_identity) \
                  + (SELECT count(*) FROM ONLY memory.codebase_memory_extension_ledger)",
            &[],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_PRESTATE_IDENTITY_QUERY_FAILED"))?
        .get(0);
    Ok(if identity == 2 {
        ExtensionPreState::ExactV2
    } else {
        ExtensionPreState::Partial
    })
}

fn insert_identity(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    manifest: &crate::ExtensionManifestEvidence,
) -> Result<(), ExtensionSetupError> {
    let changed = client
        .execute(
            "INSERT INTO memory.codebase_memory_extension_identity ( \
                 singleton, extension_id, extension_schema_version, extension_path, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256 \
             ) VALUES ( \
                 true, $1, $2, $3, $4, $5, $6::text::uuid, $7, $8, $9 \
             )",
            &[
                &CODEBASE_MEMORY_EXTENSION_ID,
                &i16::try_from(CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION)
                    .expect("fixed extension version"),
                &CODEBASE_MEMORY_EXTENSION_PATH,
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &target.expected_database_uuid(),
                &target.expected_database_identity_digest().as_str(),
                &i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed global version"),
                &REQUIRED_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_IDENTITY_WRITE_FAILED"))?;
    if changed != 1 {
        return Err(stage_error("MEMORY_EXTENSION_IDENTITY_WRITE_FAILED"));
    }
    let changed = client
        .execute(
            "INSERT INTO memory.codebase_memory_extension_ledger ( \
                 ledger_ordinal, singleton, extension_id, extension_schema_version, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 event_kind \
             ) VALUES (1, true, $1, $2, $3, $4, $5::text::uuid, $6, $7, $8, 'INSTALLED')",
            &[
                &CODEBASE_MEMORY_EXTENSION_ID,
                &i16::try_from(CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION)
                    .expect("fixed extension version"),
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &target.expected_database_uuid(),
                &target.expected_database_identity_digest().as_str(),
                &i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed global version"),
                &REQUIRED_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_LEDGER_WRITE_FAILED"))?;
    if changed != 1 {
        return Err(stage_error("MEMORY_EXTENSION_LEDGER_WRITE_FAILED"));
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn verify_catalog_closure(client: &mut impl GenericClient) -> Result<(), ExtensionSetupError> {
    let relations = client
        .query(
            "SELECT c.relname::text, r.rolname::text, c.relkind::text, \
                    c.relpersistence::text, c.relrowsecurity, \
                    coalesce(pg_catalog.obj_description(c.oid, 'pg_class'), '')::text \
               FROM pg_catalog.pg_class AS c \
               JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
               JOIN pg_catalog.pg_roles AS r ON r.oid = c.relowner \
              WHERE n.nspname = 'memory' AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f') \
              ORDER BY c.relname",
            &[],
        )
        .map_err(|_| catalog_stage("MEMORY_EXTENSION_RELATION_QUERY_FAILED"))?;
    if relations.len() != EXPECTED_TABLES.len() {
        return Err(catalog_stage("MEMORY_EXTENSION_RELATION_COUNT_MISMATCH"));
    }
    let expected_comments = [
        "LATTICE_CODEBASE_MEMORY_ANALYSES_V1",
        "LATTICE_CODEBASE_MEMORY_EXTENSION_IDENTITY_V2",
        "LATTICE_CODEBASE_MEMORY_EXTENSION_LEDGER_V2",
        "LATTICE_CODEBASE_MEMORY_RECEIPTS_V1",
        "LATTICE_CODEBASE_MEMORY_RECORDS_V1",
        "LATTICE_CODEBASE_MEMORY_REFLECTIONS_V2",
        "LATTICE_CODEBASE_MEMORY_RETRIEVAL_AUDITS_V1",
        "LATTICE_OPENCLAW_GATEWAY_COMMANDS_V1",
    ];
    for ((row, expected_name), expected_comment) in
        relations.iter().zip(EXPECTED_TABLES).zip(expected_comments)
    {
        let name: String = row.get(0);
        let owner: String = row.get(1);
        let kind: String = row.get(2);
        let persistence: String = row.get(3);
        let row_security: bool = row.get(4);
        let comment: String = row.get(5);
        if name != expected_name
            || owner != "lattice_migrator"
            || kind != "r"
            || persistence != "p"
            || row_security
            || comment != expected_comment
        {
            return Err(catalog_stage("MEMORY_EXTENSION_RELATION_PROFILE_MISMATCH"));
        }
        let public_privilege: bool = client
            .query_one(
                "SELECT EXISTS ( \
                     SELECT 1 \
                       FROM pg_catalog.pg_class AS c \
                       JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                       CROSS JOIN LATERAL pg_catalog.aclexplode( \
                           coalesce(c.relacl, pg_catalog.acldefault('r', c.relowner)) \
                       ) AS a \
                      WHERE n.nspname = 'memory' AND c.relname = $1::text::name \
                        AND a.grantee = 0 \
                 )",
                &[&name],
            )
            .map_err(|_| catalog_stage("MEMORY_EXTENSION_PUBLIC_TABLE_ACL_QUERY_FAILED"))?
            .get(0);
        if public_privilege {
            return Err(catalog_stage("MEMORY_EXTENSION_TABLE_PUBLIC_ACL_MISMATCH"));
        }
        for role in [
            "lattice_runtime",
            "lattice_guardian",
            "lattice_readonly",
            "lattice_migrator_login",
            "lattice_runtime_login",
            "lattice_guardian_login",
            "lattice_readonly_login",
        ] {
            let has_privilege: bool = client
                .query_one(
                    "SELECT pg_catalog.has_table_privilege( \
                         $1::text::name, pg_catalog.format('memory.%I', $2::text)::text, \
                         'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER,MAINTAIN' \
                     )",
                    &[&role, &name],
                )
                .map_err(|_| catalog_stage("MEMORY_EXTENSION_ROLE_TABLE_ACL_QUERY_FAILED"))?
                .get(0);
            if has_privilege {
                return Err(catalog_stage("MEMORY_EXTENSION_TABLE_ROLE_ACL_MISMATCH"));
            }
        }
        let acl_closure = client
            .query_one(
                "SELECT pg_catalog.count(*)::bigint, \
                        pg_catalog.count(*) FILTER (WHERE \
                            grantee.rolname = 'lattice_migrator' \
                            AND grantor.rolname = 'lattice_migrator' \
                            AND a.privilege_type IN ( \
                                'SELECT','INSERT','UPDATE','DELETE','TRUNCATE','REFERENCES','TRIGGER' \
                                ,'MAINTAIN' \
                            ) \
                            AND NOT a.is_grantable \
                        )::bigint \
                   FROM pg_catalog.pg_class AS c \
                   JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
                   CROSS JOIN LATERAL pg_catalog.aclexplode( \
                       coalesce(c.relacl, pg_catalog.acldefault('r', c.relowner)) \
                   ) AS a \
                   LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = a.grantee \
                   JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = a.grantor \
                  WHERE n.nspname = 'memory' AND c.relname = $1::text::name",
                &[&name],
            )
            .map_err(|_| catalog_stage("MEMORY_EXTENSION_TABLE_ACL_QUERY_FAILED"))?;
        let acl_count: i64 = acl_closure.get(0);
        let admitted_acl_count: i64 = acl_closure.get(1);
        if acl_count != 8 {
            return Err(catalog_stage(
                "MEMORY_EXTENSION_TABLE_OWNER_ACL_COUNT_MISMATCH",
            ));
        }
        if admitted_acl_count != acl_count {
            return Err(catalog_stage(
                "MEMORY_EXTENSION_TABLE_OWNER_ACL_ROLE_MISMATCH",
            ));
        }
    }

    let functions = client
        .query(
            "SELECT p.proname::text, r.rolname::text, l.lanname::text, \
                    p.prosecdef, p.provolatile::text, p.proparallel::text, \
                    pg_catalog.oidvectortypes(p.proargtypes)::text, \
                    coalesce(pg_catalog.array_to_string(p.proconfig, ','), '')::text, \
                    coalesce(pg_catalog.obj_description(p.oid, 'pg_proc'), '')::text, p.oid \
               FROM pg_catalog.pg_proc AS p \
               JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace \
               JOIN pg_catalog.pg_roles AS r ON r.oid = p.proowner \
               JOIN pg_catalog.pg_language AS l ON l.oid = p.prolang \
              WHERE n.nspname = 'memory' \
              ORDER BY p.proname",
            &[],
        )
        .map_err(|_| catalog_stage("MEMORY_EXTENSION_FUNCTION_QUERY_FAILED"))?;
    if functions.len() != EXPECTED_FUNCTIONS.len() {
        return Err(catalog_stage("MEMORY_EXTENSION_FUNCTION_COUNT_MISMATCH"));
    }
    let expected_function_profiles = [
        (
            "codebase_memory_load_receipt_v1",
            "s",
            "r",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint",
            "LATTICE_CODEBASE_MEMORY_LOAD_RECEIPT_V1",
        ),
        (
            "codebase_memory_load_reflection_v2",
            "s",
            "r",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint",
            "LATTICE_CODEBASE_MEMORY_LOAD_REFLECTION_V2",
        ),
        (
            "codebase_memory_persist_analysis_v1",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint, text, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, integer[], bytea[], text[], text[], text[], text[], text[], text[], bytea[], integer[], integer[], text[], bytea[]",
            "LATTICE_CODEBASE_MEMORY_PERSIST_ANALYSIS_V1",
        ),
        (
            "codebase_memory_persist_reflection_v2",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint, bytea, text, text, bytea, bytea, bytea, bytea, text, text[], bytea[], text[]",
            "LATTICE_CODEBASE_MEMORY_PERSIST_REFLECTION_V2",
        ),
        (
            "codebase_memory_persist_retrieval_v1",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, bytea, bytea, bytea, smallint, text, bytea[], bytea[], bigint[], bytea, bytea, bytea",
            "LATTICE_CODEBASE_MEMORY_PERSIST_RETRIEVAL_V1",
        ),
        (
            "openclaw_gateway_finalize_terminal_v1",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, text, text, bigint, text, bytea, bytea, bytea, bytea",
            "LATTICE_OPENCLAW_GATEWAY_FINALIZE_TERMINAL_V1",
        ),
        (
            "openclaw_gateway_reconcile_and_claim_v1",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, text, text, bigint, text, bytea",
            "LATTICE_OPENCLAW_GATEWAY_RECONCILE_AND_CLAIM_V1",
        ),
    ];
    for (row, (name, volatility, parallel, arguments, comment)) in
        functions.iter().zip(expected_function_profiles)
    {
        let observed_name: String = row.get(0);
        let owner: String = row.get(1);
        let language: String = row.get(2);
        let security_definer: bool = row.get(3);
        let observed_volatility: String = row.get(4);
        let observed_parallel: String = row.get(5);
        let observed_arguments: String = row.get(6);
        let settings: String = row.get(7);
        let observed_comment: String = row.get(8);
        let oid: u32 = row.get(9);
        if observed_name != name
            || owner != "lattice_migrator"
            || language != "plpgsql"
            || !security_definer
            || observed_volatility != volatility
            || observed_parallel != parallel
            || observed_arguments != arguments
            || settings
                != "search_path=pg_catalog,row_security=on,lock_timeout=5s,statement_timeout=30s"
            || observed_comment != comment
        {
            return Err(catalog_stage("MEMORY_EXTENSION_FUNCTION_PROFILE_MISMATCH"));
        }
        let public_execute: bool = client
            .query_one(
                "SELECT EXISTS ( \
                     SELECT 1 \
                       FROM pg_catalog.pg_proc AS p \
                       CROSS JOIN LATERAL pg_catalog.aclexplode( \
                           coalesce(p.proacl, pg_catalog.acldefault('f', p.proowner)) \
                       ) AS a \
                      WHERE p.oid = $1::oid AND a.grantee = 0 \
                 )",
                &[&oid],
            )
            .map_err(|_| catalog_stage("MEMORY_EXTENSION_FUNCTION_ACL_QUERY_FAILED"))?
            .get(0);
        if public_execute {
            return Err(catalog_stage("MEMORY_EXTENSION_FUNCTION_ACL_MISMATCH"));
        }
        for (role, expected_execute) in [
            ("lattice_runtime", true),
            ("lattice_guardian", false),
            ("lattice_readonly", false),
            ("lattice_migrator_login", false),
            ("lattice_runtime_login", false),
            ("lattice_guardian_login", false),
            ("lattice_readonly_login", false),
        ] {
            let execute: bool = client
                .query_one(
                    "SELECT pg_catalog.has_function_privilege( \
                         $1::text::name, $2::oid, 'EXECUTE' \
                     )",
                    &[&role, &oid],
                )
                .map_err(|_| catalog_stage("MEMORY_EXTENSION_FUNCTION_ACL_QUERY_FAILED"))?
                .get(0);
            if execute != expected_execute {
                return Err(catalog_stage("MEMORY_EXTENSION_FUNCTION_ACL_MISMATCH"));
            }
        }
        let acl_closure = client
            .query_one(
                "SELECT pg_catalog.count(*)::bigint, \
                        pg_catalog.count(*) FILTER (WHERE \
                            grantee.rolname IN ('lattice_migrator', 'lattice_runtime') \
                            AND grantor.rolname = 'lattice_migrator' \
                            AND a.privilege_type = 'EXECUTE' \
                            AND NOT a.is_grantable \
                        )::bigint, \
                        pg_catalog.count(*) FILTER (WHERE \
                            grantee.rolname = 'lattice_migrator' \
                        )::bigint, \
                        pg_catalog.count(*) FILTER (WHERE \
                            grantee.rolname = 'lattice_runtime' \
                        )::bigint \
                   FROM pg_catalog.pg_proc AS p \
                   CROSS JOIN LATERAL pg_catalog.aclexplode( \
                       coalesce(p.proacl, pg_catalog.acldefault('f', p.proowner)) \
                   ) AS a \
                   LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = a.grantee \
                   JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = a.grantor \
                  WHERE p.oid = $1::oid",
                &[&oid],
            )
            .map_err(|_| catalog_stage("MEMORY_EXTENSION_FUNCTION_ACL_QUERY_FAILED"))?;
        let acl_count: i64 = acl_closure.get(0);
        let admitted_acl_count: i64 = acl_closure.get(1);
        let owner_acl_count: i64 = acl_closure.get(2);
        let runtime_acl_count: i64 = acl_closure.get(3);
        if acl_count != 2
            || admitted_acl_count != acl_count
            || owner_acl_count != 1
            || runtime_acl_count != 1
        {
            return Err(catalog_stage("MEMORY_EXTENSION_FUNCTION_ACL_MISMATCH"));
        }
    }

    let schema_acl = client
        .query_one(
            "SELECT \
                EXISTS ( \
                    SELECT 1 \
                      FROM pg_catalog.pg_namespace AS n \
                      CROSS JOIN LATERAL pg_catalog.aclexplode( \
                          coalesce(n.nspacl, pg_catalog.acldefault('n', n.nspowner)) \
                      ) AS a \
                     WHERE n.nspname = 'memory' AND a.grantee = 0 \
                ), \
                pg_catalog.has_schema_privilege('lattice_runtime', 'memory', 'USAGE'), \
                pg_catalog.has_schema_privilege('lattice_runtime', 'memory', 'CREATE'), \
                pg_catalog.has_schema_privilege('lattice_guardian', 'memory', 'USAGE'), \
                pg_catalog.has_schema_privilege('lattice_readonly', 'memory', 'USAGE')",
            &[],
        )
        .map_err(|_| catalog_stage("MEMORY_EXTENSION_SCHEMA_ACL_QUERY_FAILED"))?;
    let public_usage: bool = schema_acl.get(0);
    let runtime_usage: bool = schema_acl.get(1);
    let runtime_create: bool = schema_acl.get(2);
    let guardian_usage: bool = schema_acl.get(3);
    let readonly_usage: bool = schema_acl.get(4);
    if public_usage || !runtime_usage || runtime_create || guardian_usage || readonly_usage {
        return Err(catalog_stage("MEMORY_EXTENSION_SCHEMA_ACL_MISMATCH"));
    }
    let schema_acl_closure = client
        .query_one(
            "SELECT pg_catalog.count(*)::bigint, \
                    pg_catalog.count(*) FILTER (WHERE \
                        grantor.rolname = 'lattice_migrator' \
                        AND NOT a.is_grantable \
                        AND ( \
                            (grantee.rolname = 'lattice_migrator' \
                                AND a.privilege_type IN ('USAGE', 'CREATE')) \
                            OR (grantee.rolname = 'lattice_runtime' \
                                AND a.privilege_type = 'USAGE') \
                        ) \
                    )::bigint, \
                    pg_catalog.count(*) FILTER (WHERE \
                        grantee.rolname = 'lattice_migrator' \
                    )::bigint, \
                    pg_catalog.count(*) FILTER (WHERE \
                        grantee.rolname = 'lattice_runtime' \
                    )::bigint \
               FROM pg_catalog.pg_namespace AS n \
               CROSS JOIN LATERAL pg_catalog.aclexplode( \
                   coalesce(n.nspacl, pg_catalog.acldefault('n', n.nspowner)) \
               ) AS a \
               LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = a.grantee \
               JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = a.grantor \
              WHERE n.nspname = 'memory'",
            &[],
        )
        .map_err(|_| catalog_stage("MEMORY_EXTENSION_SCHEMA_ACL_QUERY_FAILED"))?;
    let schema_acl_count: i64 = schema_acl_closure.get(0);
    let admitted_schema_acl_count: i64 = schema_acl_closure.get(1);
    let owner_schema_acl_count: i64 = schema_acl_closure.get(2);
    let runtime_schema_acl_count: i64 = schema_acl_closure.get(3);
    if schema_acl_count != 3
        || admitted_schema_acl_count != schema_acl_count
        || owner_schema_acl_count != 2
        || runtime_schema_acl_count != 1
    {
        return Err(catalog_stage("MEMORY_EXTENSION_SCHEMA_ACL_MISMATCH"));
    }

    let columns: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) \
               FROM pg_catalog.pg_attribute AS a \
               JOIN pg_catalog.pg_class AS c ON c.oid = a.attrelid \
               JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
              WHERE n.nspname = 'memory' AND c.relname IN ( \
                    'codebase_memory_analyses', \
                    'codebase_memory_extension_identity', \
                    'codebase_memory_extension_ledger', \
                    'codebase_memory_receipts', \
                    'codebase_memory_records', \
                    'codebase_memory_reflections', \
                    'codebase_memory_retrieval_audits', \
                    'openclaw_gateway_commands' \
                ) \
                AND a.attnum > 0 AND NOT a.attisdropped",
            &[],
        )
        .map_err(|_| catalog_stage("MEMORY_EXTENSION_COLUMN_COUNT_QUERY_FAILED"))?
        .get(0);
    let constraints: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*) \
               FROM pg_catalog.pg_constraint AS x \
               JOIN pg_catalog.pg_class AS c ON c.oid = x.conrelid \
               JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace \
              WHERE n.nspname = 'memory' AND c.relname IN ( \
                    'codebase_memory_analyses', \
                    'codebase_memory_extension_identity', \
                    'codebase_memory_extension_ledger', \
                    'codebase_memory_receipts', \
                    'codebase_memory_records', \
                    'codebase_memory_reflections', \
                    'codebase_memory_retrieval_audits', \
                    'openclaw_gateway_commands' \
                )",
            &[],
        )
        .map_err(|_| catalog_stage("MEMORY_EXTENSION_CONSTRAINT_COUNT_QUERY_FAILED"))?
        .get(0);
    if columns != 115 || constraints != 58 {
        return Err(catalog_stage(
            "MEMORY_EXTENSION_COLUMN_CONSTRAINT_COUNT_MISMATCH",
        ));
    }
    Ok(())
}

fn read_identity(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    role: ExtensionDatabaseRole,
    server_version_num: u32,
    manifest: &crate::ExtensionManifestEvidence,
) -> Result<ExtensionCatalogEvidence, ExtensionSetupError> {
    let row = client
        .query_one(
            "SELECT extension_id::text, extension_schema_version::integer, \
                    extension_path::text, btrim(extension_sql_sha256)::text, \
                    btrim(extension_manifest_sha256)::text, database_uuid::text, \
                    btrim(database_identity_sha256)::text, global_schema_version::integer, \
                    btrim(global_manifest_sha256)::text \
               FROM ONLY memory.codebase_memory_extension_identity \
              WHERE singleton",
            &[],
        )
        .map_err(|_| catalog_error())?;
    let extension_id: String = row.get(0);
    let extension_version: i32 = row.get(1);
    let extension_path: String = row.get(2);
    let sql_sha256: String = row.get(3);
    let manifest_sha256: String = row.get(4);
    let database_uuid: String = row.get(5);
    let database_identity_sha256: String = row.get(6);
    let global_version: i32 = row.get(7);
    let global_manifest_sha256: String = row.get(8);
    if extension_id != CODEBASE_MEMORY_EXTENSION_ID
        || extension_version != i32::from(CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION)
        || extension_path != CODEBASE_MEMORY_EXTENSION_PATH
        || sql_sha256 != manifest.sql_sha256().as_str()
        || manifest_sha256 != manifest.manifest_sha256().as_str()
        || database_uuid != target.expected_database_uuid()
        || database_identity_sha256 != target.expected_database_identity_digest().as_str()
        || global_version != i32::from(REQUIRED_GLOBAL_SCHEMA_VERSION)
        || global_manifest_sha256 != REQUIRED_GLOBAL_MANIFEST_SHA256
    {
        return Err(catalog_error());
    }
    let global_manifest_digest =
        ContentDigest::from_sha256(global_manifest_sha256).map_err(|_| catalog_error())?;
    let identity = CodebaseMemoryPersistenceIdentity::v2(
        target.expected_database_identity_digest().clone(),
        global_manifest_digest,
        manifest.sql_sha256().clone(),
        manifest.manifest_sha256().clone(),
    )
    .map_err(|_| catalog_error())?;
    Ok(ExtensionCatalogEvidence {
        database_uuid,
        server_version_num,
        role,
        identity,
    })
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
    let identity = ContentDigest::from_sha256(digest_hex).map_err(|_| catalog_error())?;
    let mut uuid_bytes = [0_u8; 16];
    uuid_bytes.copy_from_slice(&digest[..16]);
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x80;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    Ok((format_uuid(uuid_bytes), identity))
}

fn update_framed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(
        u64::try_from(value.len())
            .expect("validated target field length")
            .to_be_bytes(),
    );
    hasher.update(value);
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

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

const fn transaction_error() -> ExtensionSetupError {
    ExtensionSetupError::new(
        ExtensionSetupErrorKind::TransactionFailed,
        "MEMORY_EXTENSION_TRANSACTION_FAILED",
    )
}

const fn stage_error(code: &'static str) -> ExtensionSetupError {
    ExtensionSetupError::new(ExtensionSetupErrorKind::TransactionFailed, code)
}

const fn catalog_error() -> ExtensionSetupError {
    ExtensionSetupError::new(
        ExtensionSetupErrorKind::CatalogMismatch,
        "MEMORY_EXTENSION_CATALOG_MISMATCH",
    )
}

const fn catalog_stage(code: &'static str) -> ExtensionSetupError {
    ExtensionSetupError::new(ExtensionSetupErrorKind::CatalogMismatch, code)
}

fn map_extension_sql_error(error: &postgres::Error) -> ExtensionSetupError {
    let static_code = match error.code().map(postgres::error::SqlState::code) {
        Some("42601") => "MEMORY_EXTENSION_SQL_SYNTAX_ERROR",
        Some("42883") => "MEMORY_EXTENSION_SQL_UNDEFINED_FUNCTION",
        Some("42P13") => "MEMORY_EXTENSION_SQL_INVALID_FUNCTION_DEFINITION",
        Some("42804") => "MEMORY_EXTENSION_SQL_DATATYPE_MISMATCH",
        Some("42703") => "MEMORY_EXTENSION_SQL_UNDEFINED_COLUMN",
        Some("42704") => "MEMORY_EXTENSION_SQL_UNDEFINED_OBJECT",
        Some("0A000") => "MEMORY_EXTENSION_SQL_FEATURE_UNSUPPORTED",
        _ => "MEMORY_EXTENSION_SQL_EXECUTION_FAILED",
    };
    ExtensionSetupError::new(ExtensionSetupErrorKind::TransactionFailed, static_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_identity_matches_the_store_domain_vector() {
        let target = ExtensionTarget::new(
            "lattice_task019_12345678_base",
            "11111111111111111111111111111111",
        )
        .expect("target");
        assert_eq!(target.expected_database_uuid().len(), 36);
        assert_eq!(target.expected_database_uuid().as_bytes()[14], b'8');
        assert_eq!(
            target.expected_database_identity_digest().as_str().len(),
            64
        );
    }
}
