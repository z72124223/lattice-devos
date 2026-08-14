use std::error::Error;
use std::fmt;

use lattice_contracts::{CodebaseMemoryPersistenceIdentity, ContentDigest};
use postgres::{Client, GenericClient, IsolationLevel};
use sha2::{Digest, Sha256};

use crate::{
    CODEBASE_MEMORY_EXTENSION_ID, CODEBASE_MEMORY_EXTENSION_PATH,
    CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION, CODEBASE_MEMORY_V2_EXTENSION_PATH,
    verify_embedded_extension_manifest, verify_embedded_v2_extension_manifest,
};

const SUPPORTED_POSTGRES_MAJOR: u32 = 17;
const REQUIRED_GLOBAL_SCHEMA_VERSION: u16 = 5;
// Updated only after the exact six-entry global manifest is frozen.
const REQUIRED_GLOBAL_MANIFEST_SHA256: &str =
    "f92a51fa19c4fe0ffebfc40f20924bd1209bb2441b1bc69f787bc3c4a925425d";
const HISTORICAL_V2_GLOBAL_SCHEMA_VERSION: u16 = 3;
const HISTORICAL_V2_GLOBAL_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const DATABASE_IDENTITY_DOMAIN: &[u8] = b"LATTICE_POSTGRES_DATABASE_IDENTITY_V1\0";
const GLOBAL_MIGRATION_ADVISORY_LOCK: i64 = 0x4c41_5454_4943_4501;
const EXTENSION_ADVISORY_LOCK: i64 = 0x4c41_5443_4d45_4d31;
const CATALOG_SIGNATURE_DOMAIN: &[u8] = b"LATTICE_POSTGRES_CATALOG_SIGNATURE_V1\0";

// Frozen from the coordinated marker-owned disposable PostgreSQL 17 fixture.
const V2_EXPECTED_RELATION_SIGNATURE: &str =
    "5631c99dc7aa577e9a27fd0ed5fcf6c4f5f497bb912c66e2a3cb7ef1d58e44a9";
const V2_EXPECTED_COLUMN_SIGNATURE: &str =
    "b5a5532fdf430ac33fed991a1911e03a00796c1903913735fe7d21c1ccc9192c";
const V2_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "146b3a028be4a53594d1e5fb6f1467b47bcd7e8a51adfb917adcb37d2896aafa";
const V2_EXPECTED_INDEX_SIGNATURE: &str =
    "fa3057263dff10100258845861a0f186b699f8e687f50afcb2ab42af9c4fc9d4";
const V2_EXPECTED_FUNCTION_SIGNATURE: &str =
    "40a74ca1e6c7c51dc70d4ec87e02a07ed2de5650d7c87db3fc1b9d4b8298e573";
const V2_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "e7c870b9c6283f4878f3669f14269f0ee6f00e97819a3a484cd86a0314f69960";
const V2_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "0f30496f3b905e73fa907c716d77e6a8a6a2a72de91b07c35b8b5160af1f7a51";
const V2_EXPECTED_SCHEMA_ACL_SIGNATURE: &str =
    "9b049b7344630b703b9550c0ec7c7c917d47d3b3f24de170334f247447ebdb0b";
const V3_EXPECTED_RELATION_SIGNATURE: &str =
    "1a1da07041f8164ffa4b3a1ca0062019ffa9ef570d2be8313de8278222e7be33";
const V3_EXPECTED_COLUMN_SIGNATURE: &str =
    "8cbcc9e650d09f31982f20d12e0e85c756ddd03a7524392d429804c2b3cd1b9a";
const V3_EXPECTED_CONSTRAINT_SIGNATURE: &str =
    "e1d460c1e5aeceff8912301335b94926b959ec34d5e2e8fc98541e9a342a6456";
const V3_EXPECTED_INDEX_SIGNATURE: &str =
    "118108a135f2482ea6e0cba99de6fafbef5a1262184c042798cdde2ce2a462b5";
const V3_EXPECTED_FUNCTION_SIGNATURE: &str =
    "552617bd2f5ef441b6db07db94d87fadd62c8d1b1bccc1f01f2958e269ca34a9";
const V3_EXPECTED_TABLE_ACL_SIGNATURE: &str =
    "e7c870b9c6283f4878f3669f14269f0ee6f00e97819a3a484cd86a0314f69960";
const V3_EXPECTED_FUNCTION_ACL_SIGNATURE: &str =
    "501f47077e0b58b6694f7ff778a646de210252aa6f129083fa17b7d2cc7030fb";
const V3_EXPECTED_SCHEMA_ACL_SIGNATURE: &str =
    "9b049b7344630b703b9550c0ec7c7c917d47d3b3f24de170334f247447ebdb0b";

const RELATION_SIGNATURE_SQL: &str = r"
    SELECT pg_catalog.jsonb_build_array(
        n.nspname, c.relname, c.relkind::text, owner.rolname,
        c.relpersistence::text, c.relrowsecurity, c.relforcerowsecurity,
        c.relhassubclass, c.relispartition, c.relreplident::text,
        COALESCE(pg_catalog.array_to_string(c.reloptions, ','), '<NULL>'),
        COALESCE(pg_catalog.obj_description(c.oid, 'pg_class'), '<NULL>')
    )::text
    FROM pg_catalog.pg_class AS c
    JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
    JOIN pg_catalog.pg_roles AS owner ON owner.oid = c.relowner
    WHERE n.nspname = 'memory' AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f')
    ORDER BY c.relname
";

const COLUMN_SIGNATURE_SQL: &str = r"
    SELECT pg_catalog.jsonb_build_array(
        c.relname, a.attnum, a.attname,
        pg_catalog.format_type(a.atttypid, a.atttypmod),
        a.attnotnull, a.attisdropped,
        COALESCE(pg_catalog.pg_get_expr(ad.adbin, ad.adrelid, false), '<NULL>'),
        a.attidentity::text, a.attgenerated::text,
        CASE WHEN coll.oid IS NULL THEN '<NULL>'
             ELSE coll_ns.nspname || '.' || coll.collname END,
        a.attstorage::text, a.attcompression::text, a.attstattarget,
        COALESCE(a.attacl::text, '<NULL>')
    )::text
    FROM pg_catalog.pg_class AS c
    JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
    JOIN pg_catalog.pg_attribute AS a ON a.attrelid = c.oid
    LEFT JOIN pg_catalog.pg_attrdef AS ad ON ad.adrelid = c.oid AND ad.adnum = a.attnum
    LEFT JOIN pg_catalog.pg_collation AS coll ON coll.oid = a.attcollation
    LEFT JOIN pg_catalog.pg_namespace AS coll_ns ON coll_ns.oid = coll.collnamespace
    WHERE n.nspname = 'memory' AND c.relkind IN ('r', 'p') AND a.attnum > 0
    ORDER BY c.relname, a.attnum
";

const CONSTRAINT_SIGNATURE_SQL: &str = r"
    SELECT pg_catalog.jsonb_build_array(
        c.relname, con.conname, con.contype::text, con.convalidated,
        con.condeferrable, con.condeferred, con.connoinherit,
        con.conislocal, con.coninhcount, con.conkey,
        ref_ns.nspname, ref_class.relname, con.confkey,
        con.confupdtype::text, con.confdeltype::text, con.confmatchtype::text,
        pg_catalog.pg_get_constraintdef(con.oid, false)
    )::text
    FROM pg_catalog.pg_constraint AS con
    JOIN pg_catalog.pg_namespace AS n ON n.oid = con.connamespace
    JOIN pg_catalog.pg_class AS c ON c.oid = con.conrelid
    LEFT JOIN pg_catalog.pg_class AS ref_class ON ref_class.oid = con.confrelid
    LEFT JOIN pg_catalog.pg_namespace AS ref_ns ON ref_ns.oid = ref_class.relnamespace
    WHERE n.nspname = 'memory'
    ORDER BY c.relname, con.conname
";

const INDEX_SIGNATURE_SQL: &str = r"
    SELECT pg_catalog.jsonb_build_array(
        table_class.relname, index_class.relname, i.indisunique, i.indisprimary,
        i.indisvalid, i.indisready, i.indislive, i.indisclustered,
        i.indisreplident, i.indnullsnotdistinct,
        pg_catalog.pg_get_indexdef(i.indexrelid, 0, true)
    )::text
    FROM pg_catalog.pg_index AS i
    JOIN pg_catalog.pg_class AS table_class ON table_class.oid = i.indrelid
    JOIN pg_catalog.pg_class AS index_class ON index_class.oid = i.indexrelid
    JOIN pg_catalog.pg_namespace AS n ON n.oid = table_class.relnamespace
    WHERE n.nspname = 'memory'
    ORDER BY table_class.relname, index_class.relname
";

const FUNCTION_SIGNATURE_SQL: &str = r"
    SELECT pg_catalog.jsonb_build_array(
        p.proname, pg_catalog.pg_get_function_identity_arguments(p.oid),
        pg_catalog.pg_get_function_result(p.oid), owner.rolname, language.lanname,
        p.prokind::text, p.prosecdef, p.proleakproof, p.provolatile::text,
        p.proparallel::text, p.proisstrict, p.proretset, p.pronargs,
        p.pronargdefaults, p.prorettype::regtype::text, p.proargtypes::text,
        COALESCE(p.proallargtypes::text, '<NULL>'),
        COALESCE(p.proargmodes::text, '<NULL>'),
        COALESCE(p.proargnames::text, '<NULL>'),
        COALESCE(pg_catalog.array_to_string(p.proconfig, ','), '<NULL>'),
        COALESCE(p.probin, '<NULL>'), p.prosrc,
        pg_catalog.pg_get_functiondef(p.oid),
        COALESCE(pg_catalog.obj_description(p.oid, 'pg_proc'), '<NULL>')
    )::text
    FROM pg_catalog.pg_proc AS p
    JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
    JOIN pg_catalog.pg_roles AS owner ON owner.oid = p.proowner
    JOIN pg_catalog.pg_language AS language ON language.oid = p.prolang
    WHERE n.nspname = 'memory'
    ORDER BY p.proname, pg_catalog.pg_get_function_identity_arguments(p.oid)
";

const TABLE_ACL_SIGNATURE_SQL: &str = r"
    SELECT pg_catalog.jsonb_build_array(
        c.relname, owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'),
        grantor.rolname, acl.privilege_type, acl.is_grantable
    )::text
    FROM pg_catalog.pg_class AS c
    JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
    JOIN pg_catalog.pg_roles AS owner ON owner.oid = c.relowner
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(c.relacl, pg_catalog.acldefault('r', c.relowner))
    ) AS acl
    LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee
    JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = acl.grantor
    WHERE n.nspname = 'memory' AND c.relkind IN ('r', 'p')
    ORDER BY c.relname, owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'),
             grantor.rolname, acl.privilege_type, acl.is_grantable
";

const FUNCTION_ACL_SIGNATURE_SQL: &str = r"
    SELECT pg_catalog.jsonb_build_array(
        p.proname, pg_catalog.pg_get_function_identity_arguments(p.oid),
        owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
        acl.privilege_type, acl.is_grantable
    )::text
    FROM pg_catalog.pg_proc AS p
    JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
    JOIN pg_catalog.pg_roles AS owner ON owner.oid = p.proowner
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(p.proacl, pg_catalog.acldefault('f', p.proowner))
    ) AS acl
    LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee
    JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = acl.grantor
    WHERE n.nspname = 'memory'
    ORDER BY p.proname, pg_catalog.pg_get_function_identity_arguments(p.oid),
             owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
             acl.privilege_type, acl.is_grantable
";

const SCHEMA_ACL_SIGNATURE_SQL: &str = r"
    SELECT pg_catalog.jsonb_build_array(
        n.nspname, owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'),
        grantor.rolname, acl.privilege_type, acl.is_grantable
    )::text
    FROM pg_catalog.pg_namespace AS n
    JOIN pg_catalog.pg_roles AS owner ON owner.oid = n.nspowner
    CROSS JOIN LATERAL pg_catalog.aclexplode(
        COALESCE(n.nspacl, pg_catalog.acldefault('n', n.nspowner))
    ) AS acl
    LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee
    JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = acl.grantor
    WHERE n.nspname = 'memory'
    ORDER BY owner.rolname, COALESCE(grantee.rolname, 'PUBLIC'), grantor.rolname,
             acl.privilege_type, acl.is_grantable
";

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
const EXPECTED_FUNCTIONS: [&str; 14] = [
    "codebase_memory_load_receipt_v1",
    "codebase_memory_load_receipt_v3",
    "codebase_memory_load_reflection_v2",
    "codebase_memory_load_reflection_v3",
    "codebase_memory_persist_analysis_v1",
    "codebase_memory_persist_analysis_v3",
    "codebase_memory_persist_reflection_v2",
    "codebase_memory_persist_reflection_v3",
    "codebase_memory_persist_retrieval_v1",
    "codebase_memory_persist_retrieval_v3",
    "openclaw_gateway_finalize_terminal_v1",
    "openclaw_gateway_finalize_terminal_v3",
    "openclaw_gateway_reconcile_and_claim_v1",
    "openclaw_gateway_reconcile_and_claim_v3",
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
    ExactV3,
    Partial,
    Collision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExactCatalogProfile {
    V2,
    V3,
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
    let v2_manifest = verify_embedded_v2_extension_manifest().map_err(|_| {
        ExtensionSetupError::new(
            ExtensionSetupErrorKind::ManifestInvalid,
            "MEMORY_EXTENSION_V2_MANIFEST_INVALID",
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
            &[&GLOBAL_MIGRATION_ADVISORY_LOCK],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_GLOBAL_ADVISORY_LOCK_FAILED"))?;
    transaction
        .query_one(
            "SELECT pg_catalog.pg_advisory_xact_lock($1)",
            &[&EXTENSION_ADVISORY_LOCK],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_ADVISORY_LOCK_FAILED"))?;
    let server_version_num = preflight(&mut transaction, target, ExtensionDatabaseRole::Migrator)?;
    let pre_state = classify_pre_state(&mut transaction)?;
    match pre_state {
        ExtensionPreState::Fresh => {
            let v2_sql = std::str::from_utf8(v2_manifest.bytes()).map_err(|_| {
                ExtensionSetupError::new(
                    ExtensionSetupErrorKind::ManifestInvalid,
                    "MEMORY_EXTENSION_MANIFEST_INVALID",
                )
            })?;
            transaction
                .batch_execute(v2_sql)
                .map_err(|error| map_extension_sql_error(&error))?;
            insert_v2_identity(&mut transaction, target, &v2_manifest)?;
            if classify_pre_state(&mut transaction)? != ExtensionPreState::ExactV2 {
                return Err(catalog_error());
            }
            verify_v2_source(&mut transaction, target, &v2_manifest)?;
            verify_exact_catalog_profile(&mut transaction, ExactCatalogProfile::V2)?;
            apply_v3_successor(&mut transaction, target, &manifest)?;
        }
        ExtensionPreState::Partial => {
            return Err(ExtensionSetupError::new(
                ExtensionSetupErrorKind::PartialProfile,
                "MEMORY_EXTENSION_PARTIAL_PROFILE",
            ));
        }
        ExtensionPreState::Collision => {
            return Err(ExtensionSetupError::new(
                ExtensionSetupErrorKind::SchemaCollision,
                "MEMORY_EXTENSION_SCHEMA_COLLISION",
            ));
        }
        ExtensionPreState::ExactV2 => {
            verify_v2_source(&mut transaction, target, &v2_manifest)?;
            verify_exact_catalog_profile(&mut transaction, ExactCatalogProfile::V2)?;
            apply_v3_successor(&mut transaction, target, &manifest)?;
        }
        ExtensionPreState::ExactV3 => {}
    }
    if classify_pre_state(&mut transaction)? != ExtensionPreState::ExactV3 {
        return Err(catalog_error());
    }
    verify_exact_catalog_profile(&mut transaction, ExactCatalogProfile::V3)?;
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
    if pre_state == ExtensionPreState::ExactV3 {
        Ok(ExtensionApplyOutcome::AlreadyCurrent)
    } else {
        verify_extension(client, target, ExtensionDatabaseRole::Migrator).map_err(|_| {
            ExtensionSetupError::new(
                ExtensionSetupErrorKind::PostApplyVerificationFailed,
                "MEMORY_EXTENSION_POST_APPLY_VERIFICATION_FAILED",
            )
        })?;
        Ok(ExtensionApplyOutcome::Installed)
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
    if state != ExtensionPreState::ExactV3 {
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
            ExtensionPreState::ExactV2 => ExtensionSetupError::new(
                ExtensionSetupErrorKind::InstallationRequired,
                "MEMORY_EXTENSION_UPGRADE_REQUIRED",
            ),
            ExtensionPreState::ExactV3 => unreachable!(),
        });
    }
    verify_exact_catalog_profile(&mut transaction, ExactCatalogProfile::V3)?;
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
    let writer_lease_present: bool = client
        .query_one(
            "SELECT pg_catalog.to_regnamespace('writer_lease') IS NOT NULL",
            &[],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_WRITER_LEASE_PREFLIGHT_FAILED"))?
        .get(0);
    if writer_lease_present {
        return Err(ExtensionSetupError::new(
            ExtensionSetupErrorKind::SchemaCollision,
            "MEMORY_EXTENSION_WRITER_LEASE_PROFILE_UNSUPPORTED",
        ));
    }
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
                    'codebase_memory_load_receipt_v3', \
                    'codebase_memory_load_reflection_v2', \
                    'codebase_memory_load_reflection_v3', \
                    'codebase_memory_persist_analysis_v1', \
                    'codebase_memory_persist_analysis_v3', \
                    'codebase_memory_persist_reflection_v2', \
                    'codebase_memory_persist_reflection_v3', \
                    'codebase_memory_persist_retrieval_v1', \
                    'codebase_memory_persist_retrieval_v3', \
                    'openclaw_gateway_finalize_terminal_v1', \
                    'openclaw_gateway_finalize_terminal_v3', \
                    'openclaw_gateway_reconcile_and_claim_v1', \
                    'openclaw_gateway_reconcile_and_claim_v3' \
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
        || !matches!(all_functions, 7 | 14)
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
    let identity_rows: i64 = client
        .query_one(
            "SELECT (SELECT count(*) FROM ONLY memory.codebase_memory_extension_identity) \
                  + (SELECT count(*) FROM ONLY memory.codebase_memory_extension_ledger)",
            &[],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_PRESTATE_IDENTITY_QUERY_FAILED"))?
        .get(0);
    Ok(match (all_functions, identity_rows) {
        (7, 2) => ExtensionPreState::ExactV2,
        (14, 3) => ExtensionPreState::ExactV3,
        _ => ExtensionPreState::Partial,
    })
}

fn insert_v2_identity(
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
                &i16::try_from(HISTORICAL_V2_GLOBAL_SCHEMA_VERSION - 1)
                    .expect("fixed extension version"),
                &CODEBASE_MEMORY_V2_EXTENSION_PATH,
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &target.expected_database_uuid(),
                &target.expected_database_identity_digest().as_str(),
                &i16::try_from(HISTORICAL_V2_GLOBAL_SCHEMA_VERSION).expect("fixed global version"),
                &HISTORICAL_V2_GLOBAL_MANIFEST_SHA256,
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
                &i16::try_from(HISTORICAL_V2_GLOBAL_SCHEMA_VERSION - 1)
                    .expect("fixed extension version"),
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &target.expected_database_uuid(),
                &target.expected_database_identity_digest().as_str(),
                &i16::try_from(HISTORICAL_V2_GLOBAL_SCHEMA_VERSION).expect("fixed global version"),
                &HISTORICAL_V2_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_LEDGER_WRITE_FAILED"))?;
    if changed != 1 {
        return Err(stage_error("MEMORY_EXTENSION_LEDGER_WRITE_FAILED"));
    }
    Ok(())
}

fn verify_v2_source(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    manifest: &crate::ExtensionManifestEvidence,
) -> Result<(), ExtensionSetupError> {
    let count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*)::bigint \
               FROM ONLY memory.codebase_memory_extension_identity AS i \
               JOIN ONLY memory.codebase_memory_extension_ledger AS l USING (singleton) \
              WHERE i.singleton AND l.ledger_ordinal = 1 \
                AND i.extension_id = $1 AND i.extension_schema_version = 2 \
                AND i.extension_path = $2 \
                AND i.extension_sql_sha256 = $3 \
                AND i.extension_manifest_sha256 = $4 \
                AND i.database_uuid = $5::text::uuid \
                AND i.database_identity_sha256 = $6 \
                AND i.global_schema_version = $7 \
                AND i.global_manifest_sha256 = $8 \
                AND l.extension_id = i.extension_id \
                AND l.extension_schema_version = i.extension_schema_version \
                AND l.extension_sql_sha256 = i.extension_sql_sha256 \
                AND l.extension_manifest_sha256 = i.extension_manifest_sha256 \
                AND l.database_uuid = i.database_uuid \
                AND l.database_identity_sha256 = i.database_identity_sha256 \
                AND l.global_schema_version = i.global_schema_version \
                AND l.global_manifest_sha256 = i.global_manifest_sha256 \
                AND l.event_kind = 'INSTALLED'",
            &[
                &CODEBASE_MEMORY_EXTENSION_ID,
                &CODEBASE_MEMORY_V2_EXTENSION_PATH,
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &target.expected_database_uuid(),
                &target.expected_database_identity_digest().as_str(),
                &i16::try_from(HISTORICAL_V2_GLOBAL_SCHEMA_VERSION)
                    .expect("fixed historical global version"),
                &HISTORICAL_V2_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .map_err(|_| catalog_stage("MEMORY_EXTENSION_V2_IDENTITY_QUERY_FAILED"))?
        .get(0);
    if count != 1 {
        return Err(catalog_stage("MEMORY_EXTENSION_V2_IDENTITY_MISMATCH"));
    }
    Ok(())
}

fn apply_v3_successor(
    client: &mut impl GenericClient,
    target: &ExtensionTarget,
    manifest: &crate::ExtensionManifestEvidence,
) -> Result<(), ExtensionSetupError> {
    let sql = std::str::from_utf8(manifest.bytes()).map_err(|_| {
        ExtensionSetupError::new(
            ExtensionSetupErrorKind::ManifestInvalid,
            "MEMORY_EXTENSION_MANIFEST_INVALID",
        )
    })?;
    client
        .batch_execute(sql)
        .map_err(|error| map_extension_sql_error(&error))?;
    let changed = client
        .execute(
            "UPDATE ONLY memory.codebase_memory_extension_identity \
                SET extension_schema_version = $1, extension_path = $2, \
                    extension_sql_sha256 = $3, extension_manifest_sha256 = $4, \
                    global_schema_version = $5, global_manifest_sha256 = $6 \
              WHERE singleton AND extension_schema_version = 2 \
                AND extension_path = $7 \
                AND global_schema_version = $8 \
                AND global_manifest_sha256 = $9",
            &[
                &i16::try_from(CODEBASE_MEMORY_EXTENSION_SCHEMA_VERSION)
                    .expect("fixed extension version"),
                &CODEBASE_MEMORY_EXTENSION_PATH,
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &i16::try_from(REQUIRED_GLOBAL_SCHEMA_VERSION).expect("fixed global version"),
                &REQUIRED_GLOBAL_MANIFEST_SHA256,
                &CODEBASE_MEMORY_V2_EXTENSION_PATH,
                &i16::try_from(HISTORICAL_V2_GLOBAL_SCHEMA_VERSION)
                    .expect("fixed historical global version"),
                &HISTORICAL_V2_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .map_err(|_| stage_error("MEMORY_EXTENSION_V3_IDENTITY_WRITE_FAILED"))?;
    if changed != 1 {
        return Err(stage_error("MEMORY_EXTENSION_V3_IDENTITY_WRITE_FAILED"));
    }
    let changed = client
        .execute(
            "INSERT INTO memory.codebase_memory_extension_ledger ( \
                 ledger_ordinal, singleton, extension_id, extension_schema_version, \
                 extension_sql_sha256, extension_manifest_sha256, database_uuid, \
                 database_identity_sha256, global_schema_version, global_manifest_sha256, \
                 event_kind \
             ) VALUES (2, true, $1, $2, $3, $4, $5::text::uuid, $6, $7, $8, 'UPGRADED')",
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
        .map_err(|_| stage_error("MEMORY_EXTENSION_V3_LEDGER_WRITE_FAILED"))?;
    if changed != 1 {
        return Err(stage_error("MEMORY_EXTENSION_V3_LEDGER_WRITE_FAILED"));
    }
    Ok(())
}

fn verify_exact_catalog_profile(
    client: &mut impl GenericClient,
    profile: ExactCatalogProfile,
) -> Result<(), ExtensionSetupError> {
    let expected = match profile {
        ExactCatalogProfile::V2 => [
            V2_EXPECTED_RELATION_SIGNATURE,
            V2_EXPECTED_COLUMN_SIGNATURE,
            V2_EXPECTED_CONSTRAINT_SIGNATURE,
            V2_EXPECTED_INDEX_SIGNATURE,
            V2_EXPECTED_FUNCTION_SIGNATURE,
            V2_EXPECTED_TABLE_ACL_SIGNATURE,
            V2_EXPECTED_FUNCTION_ACL_SIGNATURE,
            V2_EXPECTED_SCHEMA_ACL_SIGNATURE,
        ],
        ExactCatalogProfile::V3 => [
            V3_EXPECTED_RELATION_SIGNATURE,
            V3_EXPECTED_COLUMN_SIGNATURE,
            V3_EXPECTED_CONSTRAINT_SIGNATURE,
            V3_EXPECTED_INDEX_SIGNATURE,
            V3_EXPECTED_FUNCTION_SIGNATURE,
            V3_EXPECTED_TABLE_ACL_SIGNATURE,
            V3_EXPECTED_FUNCTION_ACL_SIGNATURE,
            V3_EXPECTED_SCHEMA_ACL_SIGNATURE,
        ],
    };
    for (query, expected) in [
        RELATION_SIGNATURE_SQL,
        COLUMN_SIGNATURE_SQL,
        CONSTRAINT_SIGNATURE_SQL,
        INDEX_SIGNATURE_SQL,
        FUNCTION_SIGNATURE_SQL,
        TABLE_ACL_SIGNATURE_SQL,
        FUNCTION_ACL_SIGNATURE_SQL,
        SCHEMA_ACL_SIGNATURE_SQL,
    ]
    .into_iter()
    .zip(expected)
    {
        let actual = catalog_signature(client, query)?;
        if !catalog_signature_matches(&actual, expected) {
            return Err(catalog_stage("MEMORY_EXTENSION_CATALOG_SIGNATURE_MISMATCH"));
        }
    }
    Ok(())
}

fn catalog_signature(
    client: &mut impl GenericClient,
    query: &str,
) -> Result<String, ExtensionSetupError> {
    let rows = client
        .query(query, &[])
        .map_err(|_| catalog_stage("MEMORY_EXTENSION_CATALOG_SIGNATURE_QUERY_FAILED"))?;
    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        values.push(row.try_get::<_, String>(0).map_err(|_| catalog_error())?);
    }
    catalog_signature_digest(&values)
}

fn catalog_signature_digest(values: &[String]) -> Result<String, ExtensionSetupError> {
    let mut hasher = Sha256::new();
    hasher.update(CATALOG_SIGNATURE_DOMAIN);
    hasher.update(
        u64::try_from(values.len())
            .map_err(|_| catalog_error())?
            .to_be_bytes(),
    );
    for value in values {
        hasher.update(
            u64::try_from(value.len())
                .map_err(|_| catalog_error())?
                .to_be_bytes(),
        );
        hasher.update(value.as_bytes());
    }
    Ok(bytes_to_hex(&hasher.finalize()))
}

fn catalog_signature_matches(actual: &str, expected: &str) -> bool {
    expected.len() == 64
        && expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && actual == expected
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
        "LATTICE_CODEBASE_MEMORY_ANALYSES_V3",
        "LATTICE_CODEBASE_MEMORY_EXTENSION_IDENTITY_V3",
        "LATTICE_CODEBASE_MEMORY_EXTENSION_LEDGER_V3",
        "LATTICE_CODEBASE_MEMORY_RECEIPTS_V3",
        "LATTICE_CODEBASE_MEMORY_RECORDS_V3",
        "LATTICE_CODEBASE_MEMORY_REFLECTIONS_V3",
        "LATTICE_CODEBASE_MEMORY_RETRIEVAL_AUDITS_V3",
        "LATTICE_OPENCLAW_GATEWAY_COMMANDS_V3",
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
            false,
        ),
        (
            "codebase_memory_load_receipt_v3",
            "s",
            "r",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint",
            "LATTICE_CODEBASE_MEMORY_LOAD_RECEIPT_V3",
            true,
        ),
        (
            "codebase_memory_load_reflection_v2",
            "s",
            "r",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint",
            "LATTICE_CODEBASE_MEMORY_LOAD_REFLECTION_V2",
            false,
        ),
        (
            "codebase_memory_load_reflection_v3",
            "s",
            "r",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint",
            "LATTICE_CODEBASE_MEMORY_LOAD_REFLECTION_V3",
            true,
        ),
        (
            "codebase_memory_persist_analysis_v1",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint, text, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, integer[], bytea[], text[], text[], text[], text[], text[], text[], bytea[], integer[], integer[], text[], bytea[]",
            "LATTICE_CODEBASE_MEMORY_PERSIST_ANALYSIS_V1",
            false,
        ),
        (
            "codebase_memory_persist_analysis_v3",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint, text, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, bytea, integer[], bytea[], text[], text[], text[], text[], text[], text[], bytea[], integer[], integer[], text[], bytea[]",
            "LATTICE_CODEBASE_MEMORY_PERSIST_ANALYSIS_V3",
            true,
        ),
        (
            "codebase_memory_persist_reflection_v2",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint, bytea, text, text, bytea, bytea, bytea, bytea, text, text[], bytea[], text[]",
            "LATTICE_CODEBASE_MEMORY_PERSIST_REFLECTION_V2",
            false,
        ),
        (
            "codebase_memory_persist_reflection_v3",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, smallint, text, text, text, text, bytea, text, text, bytea, bytea, smallint, bytea, text, text, bytea, bytea, bytea, bytea, text, text[], bytea[], text[]",
            "LATTICE_CODEBASE_MEMORY_PERSIST_REFLECTION_V3",
            true,
        ),
        (
            "codebase_memory_persist_retrieval_v1",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, bytea, bytea, bytea, smallint, text, bytea[], bytea[], bigint[], bytea, bytea, bytea",
            "LATTICE_CODEBASE_MEMORY_PERSIST_RETRIEVAL_V1",
            false,
        ),
        (
            "codebase_memory_persist_retrieval_v3",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, bytea, bytea, bytea, smallint, text, bytea[], bytea[], bigint[], bytea, bytea, bytea",
            "LATTICE_CODEBASE_MEMORY_PERSIST_RETRIEVAL_V3",
            true,
        ),
        (
            "openclaw_gateway_finalize_terminal_v1",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, text, text, bigint, text, bytea, bytea, bytea, bytea",
            "LATTICE_OPENCLAW_GATEWAY_FINALIZE_TERMINAL_V1",
            false,
        ),
        (
            "openclaw_gateway_finalize_terminal_v3",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, text, text, bigint, text, bytea, bytea, bytea, bytea",
            "LATTICE_OPENCLAW_GATEWAY_FINALIZE_TERMINAL_V3",
            true,
        ),
        (
            "openclaw_gateway_reconcile_and_claim_v1",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, text, text, bigint, text, bytea",
            "LATTICE_OPENCLAW_GATEWAY_RECONCILE_AND_CLAIM_V1",
            false,
        ),
        (
            "openclaw_gateway_reconcile_and_claim_v3",
            "v",
            "u",
            "bytea, bytea, bytea, bytea, text, text, bigint, text, bytea",
            "LATTICE_OPENCLAW_GATEWAY_RECONCILE_AND_CLAIM_V3",
            true,
        ),
    ];
    for (row, (name, volatility, parallel, arguments, comment, runtime_execute)) in
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
            ("lattice_runtime", runtime_execute),
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
        let expected_acl_count = if runtime_execute { 2 } else { 1 };
        if acl_count != expected_acl_count
            || admitted_acl_count != acl_count
            || owner_acl_count != 1
            || runtime_acl_count != i64::from(runtime_execute)
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
    if columns != 143 || constraints != 59 {
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
    let ledger_count: i64 = client
        .query_one(
            "SELECT pg_catalog.count(*)::bigint \
               FROM ONLY memory.codebase_memory_extension_ledger AS l \
              WHERE (l.ledger_ordinal = 1 \
                     AND l.extension_id = $1 AND l.extension_schema_version = 2 \
                     AND l.extension_sql_sha256 = $2 \
                     AND l.extension_manifest_sha256 = $3 \
                     AND l.database_uuid = $4::text::uuid \
                     AND l.database_identity_sha256 = $5 \
                     AND l.global_schema_version = 3 \
                     AND l.global_manifest_sha256 = $6 \
                     AND l.event_kind = 'INSTALLED') \
                 OR (l.ledger_ordinal = 2 \
                     AND l.extension_id = $1 AND l.extension_schema_version = 3 \
                     AND l.extension_sql_sha256 = $7 \
                     AND l.extension_manifest_sha256 = $8 \
                     AND l.database_uuid = $4::text::uuid \
                     AND l.database_identity_sha256 = $5 \
                     AND l.global_schema_version = 5 \
                     AND l.global_manifest_sha256 = $9 \
                     AND l.event_kind = 'UPGRADED')",
            &[
                &CODEBASE_MEMORY_EXTENSION_ID,
                &"9db54342b88f554ca76054c7a33ae72f04b412d2dfe21fae6eb4d8faf3e854e2",
                &"0aedbd7d9ef7ca07fc2910d0da34c163cc83e3dd56f9b28292ae1f4f0c3c4d7e",
                &target.expected_database_uuid(),
                &target.expected_database_identity_digest().as_str(),
                &HISTORICAL_V2_GLOBAL_MANIFEST_SHA256,
                &manifest.sql_sha256().as_str(),
                &manifest.manifest_sha256().as_str(),
                &REQUIRED_GLOBAL_MANIFEST_SHA256,
            ],
        )
        .map_err(|_| catalog_error())?
        .get(0);
    if ledger_count != 2 {
        return Err(catalog_error());
    }
    let global_manifest_digest =
        ContentDigest::from_sha256(global_manifest_sha256).map_err(|_| catalog_error())?;
    let identity = CodebaseMemoryPersistenceIdentity::v3(
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

    #[test]
    fn frozen_catalog_signatures_are_exact_lowercase_digests() {
        for expected in [
            V2_EXPECTED_RELATION_SIGNATURE,
            V2_EXPECTED_COLUMN_SIGNATURE,
            V2_EXPECTED_CONSTRAINT_SIGNATURE,
            V2_EXPECTED_INDEX_SIGNATURE,
            V2_EXPECTED_FUNCTION_SIGNATURE,
            V2_EXPECTED_TABLE_ACL_SIGNATURE,
            V2_EXPECTED_FUNCTION_ACL_SIGNATURE,
            V2_EXPECTED_SCHEMA_ACL_SIGNATURE,
            V3_EXPECTED_RELATION_SIGNATURE,
            V3_EXPECTED_COLUMN_SIGNATURE,
            V3_EXPECTED_CONSTRAINT_SIGNATURE,
            V3_EXPECTED_INDEX_SIGNATURE,
            V3_EXPECTED_FUNCTION_SIGNATURE,
            V3_EXPECTED_TABLE_ACL_SIGNATURE,
            V3_EXPECTED_FUNCTION_ACL_SIGNATURE,
            V3_EXPECTED_SCHEMA_ACL_SIGNATURE,
        ] {
            assert!(catalog_signature_matches(expected, expected));
        }
        assert!(!catalog_signature_matches(
            &"1".repeat(64),
            "TASK075_MEMORY_SIGNATURE_PENDING"
        ));
    }

    #[test]
    fn same_count_catalog_definition_and_acl_substitution_changes_signature() {
        let frozen = vec![
            r#"["memory","codebase_memory_receipts","CHECK (octet_length(receipt_digest) = 32)"]"#
                .to_owned(),
            r#"["memory","codebase_memory_load_receipt_v3","lattice_runtime","EXECUTE"]"#
                .to_owned(),
        ];
        let mut definition_drift = frozen.clone();
        definition_drift[0] =
            r#"["memory","codebase_memory_receipts","CHECK (octet_length(receipt_digest) = 31)"]"#
                .to_owned();
        let mut acl_drift = frozen.clone();
        acl_drift[1] =
            r#"["memory","codebase_memory_load_receipt_v3","PUBLIC","EXECUTE"]"#.to_owned();
        let expected = catalog_signature_digest(&frozen).expect("signature");
        let definition_drift =
            catalog_signature_digest(&definition_drift).expect("definition drift signature");
        let acl_drift = catalog_signature_digest(&acl_drift).expect("ACL drift signature");
        assert!(!catalog_signature_matches(&definition_drift, &expected));
        assert!(!catalog_signature_matches(&acl_drift, &expected));
    }

    #[test]
    #[ignore = "requires the coordinated marker-owned disposable PostgreSQL fixture"]
    fn measure_catalog_signatures() {
        let connection = std::env::var("LATTICE_MEMORY_CATALOG_SIGNATURE_URL")
            .expect("coordinator supplies fixture URL");
        let mut client = postgres::Client::connect(&connection, postgres::NoTls)
            .expect("connect to coordinated fixture");
        client
            .batch_execute("SET search_path = pg_catalog")
            .expect("harden measurement search path");
        for (label, query) in [
            ("RELATION", RELATION_SIGNATURE_SQL),
            ("COLUMN", COLUMN_SIGNATURE_SQL),
            ("CONSTRAINT", CONSTRAINT_SIGNATURE_SQL),
            ("INDEX", INDEX_SIGNATURE_SQL),
            ("FUNCTION", FUNCTION_SIGNATURE_SQL),
            ("TABLE_ACL", TABLE_ACL_SIGNATURE_SQL),
            ("FUNCTION_ACL", FUNCTION_ACL_SIGNATURE_SQL),
            ("SCHEMA_ACL", SCHEMA_ACL_SIGNATURE_SQL),
        ] {
            let signature = catalog_signature(&mut client, query).expect("catalog signature");
            println!("MEMORY_CATALOG_{label}_SIGNATURE={signature}");
        }
    }
}
